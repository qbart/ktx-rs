//! basisu miniz deflate port vs the original basisu_miniz.h.
//!
//! Not a C test port: reference sizes and FNV-1a hashes come from
//! `scripts/golden/gen_miniz_golden.sh` (tdefl at levels 0..10 with every
//! strategy, and mz_compress2 at levels 1..9, on synthetic inputs and test
//! files), recorded in `tests/resources/golden/miniz.txt`.
#![allow(clippy::panic)]

mod common;

use ktx::basisu::miniz::{
    MZ_OK, TdeflCompressor, TdeflFlush, mz_compress_bound, mz_compress2,
    tdefl_create_comp_flags_from_zip_params,
};
use std::collections::HashMap;

fn fnv1a(data: &[u8]) -> u64 {
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

/// Must match `synthetic()` in scripts/golden/miniz_golden.cpp.
fn synthetic_inputs() -> Vec<(String, Vec<u8>)> {
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

fn golden() -> HashMap<String, (i32, usize, String)> {
    let path = common::resources_path().join("golden").join("miniz.txt");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => panic!("{}: {e}", path.display()),
    };
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((
                f.first()?.to_string(),
                (
                    f.get(1)?.parse().ok()?,
                    f.get(2)?.parse().ok()?,
                    f.get(3)?.to_string(),
                ),
            ))
        })
        .collect()
}

#[test]
fn deflate_matches_basisu_miniz() {
    let golden = golden();
    let r = common::resources_path();
    let mut inputs = synthetic_inputs();
    for f in [
        "ktx2/r8g8b8a8_srgb.ktx2",
        "ktx2/bc3_unorm_array_7.ktx2",
        "ktx2/astc_8x8_unorm_array_7.ktx2",
        "ktx/orient_up_metadata.ktx",
    ] {
        let data = match std::fs::read(r.join(f)) {
            Ok(d) => d,
            Err(e) => panic!("{f}: {e}"),
        };
        let name = f.rsplit('/').next().unwrap_or(f).to_string();
        inputs.push((name, data));
    }

    let mut checked = 0;
    for (name, data) in &inputs {
        for level in 0..=10 {
            for strategy in 0..=4 {
                let window_bits = if level % 2 == 1 { 15 } else { -15 };
                let flags = tdefl_create_comp_flags_from_zip_params(level, window_bits, strategy);
                let mut c = TdeflCompressor::new(flags);
                let status = c.compress(data, TdeflFlush::Finish) as i32;
                let out = c.into_output();
                let key = format!("{name}_tdefl_L{level}_S{strategy}");
                let Some((st, size, hash)) = golden.get(&key) else {
                    panic!("no golden for {key}")
                };
                assert_eq!(status, *st, "{key}");
                assert_eq!(out.len(), *size, "{key}");
                assert_eq!(&format!("{:016x}", fnv1a(&out)), hash, "{key}");
                checked += 1;
            }
        }
        for level in 1..=9 {
            let key = format!("{name}_compress2_L{level}");
            let Some((st, size, hash)) = golden.get(&key) else {
                panic!("no golden for {key}")
            };
            let out = match mz_compress2(data, mz_compress_bound(data.len() as u64), level) {
                Ok(o) => o,
                Err(e) => panic!("{key}: {e}"),
            };
            assert_eq!(*st, MZ_OK, "{key}");
            assert_eq!(out.len(), *size, "{key}");
            assert_eq!(&format!("{:016x}", fnv1a(&out)), hash, "{key}");
            checked += 1;
        }
    }
    assert_eq!(checked, golden.len());
}

/// Compressed output inflates back to the input.
#[test]
fn deflate_round_trips() {
    for (name, data) in synthetic_inputs() {
        for level in [1, 6, 10] {
            let out = match mz_compress2(&data, mz_compress_bound(data.len() as u64), level) {
                Ok(o) => o,
                Err(e) => panic!("{name} L{level}: {e}"),
            };
            let mut back = vec![0u8; data.len()];
            assert_eq!(
                ktx::zlib::ktx_uncompress_zlib_int(&mut back, &out),
                Ok(data.len()),
                "{name} L{level}"
            );
            assert!(back == data, "{name} L{level}");
        }
    }
}

/// A too small destination fails with MZ_BUF_ERROR like miniz.
#[test]
fn compress2_small_destination() {
    let data = vec![7u8; 1000];
    assert!(mz_compress2(&data, 4, 6).is_err());
    assert!(mz_compress2(&data, 0, 6).is_err());
}
