//! basisu vector quantizer (`tree_vector_quant`, `generate_hierarchical_codebook_threaded` in
//! `basisu_enc.h`) vs. golden output of the C++ original (`scripts/golden/gen_basisu_vq_golden.sh`).
#![allow(clippy::panic)]

use std::collections::BTreeMap;

use ktx::basisu::vq::*;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

struct H(u64);

impl H {
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn u(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn cb(&mut self, c: &[Vec<u32>]) {
        self.u(c.len() as u32);
        for v in c {
            self.u(v.len() as u32);
            for &x in v {
                self.u(x);
            }
        }
    }
}

fn make_vec<const N: usize>(k: u32, i: u32, l: &mut Lcg) -> [f32; N] {
    let cluster = l.next() % (3 + k * 5);
    let mut v = [0.0f32; N];
    for c in 0..N as u32 {
        v[c as usize] = match k % 6 {
            0 => ((l.next() >> 8) & 255) as f32 / 255.0,
            1 => ((cluster * 37 + c * 11) & 255) as f32 / 255.0 + (l.next() % 9) as f32 / 255.0,
            2 => (l.next() % 4) as f32 / 3.0,
            3 => {
                if c == 0 && cluster == 0 {
                    0.5
                } else if i & 1 != 0 {
                    -0.0
                } else {
                    0.0
                }
            }
            4 => ((cluster * 13 + c) % 7) as f32 * 0.25,
            _ => 0.5 + (l.next() % 3) as f32 * 1e-7 * (c + 1) as f32,
        };
    }
    v
}

#[allow(clippy::too_many_arguments)]
fn run_case<const N: usize>(
    k: u32,
    n: u32,
    max_cb: u32,
    max_parent: u32,
    threads: u32,
    even_odd: bool,
    wmode: u32,
) -> u64 {
    let mut l = Lcg(k
        .wrapping_mul(7919)
        .wrapping_add(n.wrapping_mul(31))
        .wrapping_add(max_cb));
    let mut q = TreeVectorQuant::<N>::new();
    let mut i = 0;
    while i < n {
        let v = make_vec::<N>(k, if even_odd { i & !1 } else { i }, &mut l);
        let w: u64 = match wmode {
            0 => 1,
            1 => 1 + (l.next() % 1000) as u64,
            _ => (l.next() % 3) as u64 + 1,
        };
        q.add_training_vec(&v, w);
        if even_odd {
            q.add_training_vec(&v, w + (i & 3) as u64);
            i += 1;
        }
        i += 1;
    }
    let mut h = H(0xcbf29ce484222325);
    let (mut cb, mut pcb) = (Vec::new(), Vec::new());
    let ok = generate_hierarchical_codebook_threaded(
        &mut q, max_cb, max_parent, &mut cb, &mut pcb, threads, even_odd,
    );
    h.bytes(&[ok as u8]);
    h.cb(&cb);
    h.cb(&pcb);
    let mut q2 = TreeVectorQuant::<N>::new();
    for (v, w) in q.get_training_vecs() {
        q2.add_training_vec(v, *w);
    }
    let ok = q2.generate(max_cb);
    h.bytes(&[ok as u8]);
    let (mut c2, mut c3) = (Vec::new(), Vec::new());
    q2.retrieve(&mut c2);
    h.cb(&c2);
    q2.retrieve_max(if max_parent != 0 { max_parent } else { 5 }, &mut c3);
    h.cb(&c3);
    let mut org = Vec::new();
    q2.retrieve_origins(&mut org);
    h.u(org.len() as u32);
    for o in &org {
        for x in o {
            h.bytes(&x.to_le_bytes());
        }
    }
    h.0
}

fn golden() -> BTreeMap<String, String> {
    include_str!("resources/golden/basisu_vq.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn vq_matches_basisu() {
    let g = golden();
    const NS: [u32; 7] = [1, 2, 3, 17, 100, 600, 3000];
    const CBS: [u32; 5] = [1, 2, 7, 64, 256];
    let mut fails = Vec::new();
    let mut idx = 0;
    for k in 0..12u32 {
        for (ni, &n) in NS.iter().enumerate() {
            for (ci, &cbs) in CBS.iter().enumerate() {
                let eo = (k + ci as u32) % 3 == 2 && n >= 2;
                let parent = if ci & 1 != 0 { cbs / 2 + 1 } else { 0 };
                let wmode = (k + ni as u32) % 3;
                let h6 = run_case::<6>(k, n, cbs, parent, 1, eo, wmode);
                let h16 = run_case::<16>(k, n, cbs, parent, 1, eo, wmode);
                let got = format!("{h6:016x} {h16:016x}");
                match g.get(&idx.to_string()) {
                    Some(w) if *w == got => {}
                    w => fails.push(format!(
                        "{idx} (k {k} n {n} cb {cbs}): want {w:?} got {got}"
                    )),
                }
                idx += 1;
            }
        }
    }
    if !fails.is_empty() {
        panic!(
            "{} of {idx} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(30)].join("\n")
        );
    }
}

#[test]
fn vq_threaded_matches_basisu() {
    let g = golden();
    for t in 0..2u32 {
        let got = format!(
            "{:016x}",
            run_case::<6>(
                0,
                270000,
                600,
                if t != 0 { 100 } else { 0 },
                4 + t * 3,
                false,
                1
            )
        );
        assert_eq!(g.get(&format!("mt{t}")), Some(&got), "mt{t}");
    }
}
