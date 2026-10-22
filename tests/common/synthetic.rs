//! Deterministic inputs shared with the C golden generators.

pub fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

/// Must match `synthetic()` in scripts/golden/miniz_golden.cpp and
/// scripts/golden/zstd_golden.c.
pub fn synthetic_inputs() -> Vec<(String, Vec<u8>)> {
    let mut v = vec![
        ("empty".to_string(), vec![]),
        ("abc".to_string(), b"abc".to_vec()),
    ];
    let mut r = Lcg(1);
    v.push((
        "random".into(),
        (0..100000).map(|_| r.next() as u8).collect(),
    ));
    let mut r = Lcg(2);
    v.push((
        "lowentropy".into(),
        (0..300000).map(|_| (r.next() & 3) as u8).collect(),
    ));
    v.push((
        "quadratic".into(),
        (0..200000u32)
            .map(|i| (i.wrapping_mul(i) >> 7) as u8)
            .collect(),
    ));
    v.push(("zeros".into(), vec![0; 1000000]));
    let words = [
        "the ",
        "quick ",
        "brown ",
        "fox ",
        "jumps ",
        "over ",
        "lazy ",
        "dog ",
        "ktx ",
        "texture\n",
    ];
    let mut r = Lcg(3);
    let mut d = Vec::new();
    while d.len() < 150000 {
        d.extend_from_slice(words[(r.next() % 10) as usize].as_bytes());
    }
    v.push(("text".into(), d));
    let mut r = Lcg(4);
    let mut d: Vec<u8> = Vec::new();
    for _ in 0..70000 {
        let x = r.next();
        if x.is_multiple_of(7) && d.len() > 300 {
            let back = 1 + (x % 300) as usize;
            let len = 3 + ((x >> 9) % 40) as usize;
            for _ in 0..len {
                d.push(d[d.len() - back]);
            }
        } else {
            d.push((x >> 3) as u8);
        }
    }
    v.push(("backrefs".into(), d));
    v
}
