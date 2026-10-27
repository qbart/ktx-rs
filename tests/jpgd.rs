// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! jpgd port vs basis_universal's jpgd.cpp.
//!
//! Not a C test port: `scripts/golden/gen_jpgd_golden.sh` decodes the
//! synthetic JPEGs of `scripts/golden/make_jpegs.py` (baseline, progressive,
//! every sampling factor, restarts, 16-bit quantization tables) and two
//! progressive sample images, plus byte flips and truncations of the small
//! files, with and without linear chroma filtering, recording the status
//! codes, sizes, bytes read and a hash of every decoded scan line
//! (`tests/resources/golden/jpgd.txt`). "CRASH" cases only need to not panic.
#![allow(clippy::panic)]

mod common;

use common::synthetic::fnv1a;
use ktx::jpgd::*;
use std::collections::HashMap;

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
}

fn fnv_more(data: &[u8], mut h: u64) -> u64 {
    for &b in data {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    h
}

fn run_case(file: &[u8], flags: u32) -> String {
    let mut stream = JpegDecoderMemStream::new(file.to_vec());
    if file.is_empty() {
        stream.close();
    }
    let mut d = JpegDecoder::new(Box::new(stream), flags);
    let mut o = format!(
        "C{} {}x{}x{}",
        d.get_error_code(),
        d.get_width(),
        d.get_height(),
        d.get_num_components()
    );
    if d.get_error_code() != JPGD_SUCCESS {
        return o;
    }
    let b = d.begin_decoding();
    o += &format!(" B{b},{}", d.get_error_code());
    if b != JPGD_SUCCESS {
        return o;
    }
    o += &format!(" bpp{}", d.get_bytes_per_pixel());
    let mut h = 0xcbf29ce484222325u64;
    let mut lines = 0;
    let mut status = 0;
    for _ in 0..d.get_height() + 2 {
        let (s, line) = d.decode();
        status = s;
        if s != JPGD_SUCCESS {
            break;
        }
        h = fnv_more(line, h);
        lines += 1;
    }
    o + &format!(
        " L{lines} S{status} E{} T{} {h:016x}",
        d.get_error_code(),
        d.get_total_bytes_read()
    )
}

fn files() -> Vec<String> {
    let mut v = Vec::new();
    for dir in ["tests/resources/imageio/jpg", "tests/resources/input/jpg"] {
        for e in std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{dir}: {e}"))
            .flatten()
        {
            let n = format!("{dir}/{}", e.file_name().to_string_lossy());
            if n.ends_with(".jpg") {
                v.push(n);
            }
        }
    }
    v.sort();
    v
}

#[test]
fn jpgd_matches_basisu() {
    let golden = std::fs::read_to_string("tests/resources/golden/jpgd.txt")
        .unwrap_or_else(|e| panic!("golden: {e}"));
    let mut expected = HashMap::new();
    for line in golden.lines() {
        let mut it = line.splitn(3, '\t');
        let (Some(n), Some(f), Some(r)) = (it.next(), it.next(), it.next()) else {
            panic!("bad line {line}");
        };
        expected.insert((n.to_string(), f.to_string()), r.to_string());
    }
    let mut cases: Vec<(String, u32, Vec<u8>)> = Vec::new();
    for name in files() {
        let d = std::fs::read(&name).unwrap_or_else(|e| panic!("{name}: {e}"));
        for flags in 0..2 {
            cases.push((name.clone(), flags, d.clone()));
        }
        if d.is_empty() || d.len() > 4096 {
            continue;
        }
        let mut rng = Lcg(fnv1a(name.as_bytes()) as u32);
        for k in 0..40u32 {
            let mut c = d.clone();
            let pos = rng.next() as usize % c.len();
            c[pos] = rng.next() as u8;
            cases.push((format!("{name}#f{k}"), k & 1, c));
        }
        for k in 0..6u32 {
            let len = rng.next() as usize % d.len();
            cases.push((format!("{name}#t{k}"), k & 1, d[..len].to_vec()));
        }
    }
    assert_eq!(cases.len(), expected.len(), "case count");
    let mut failures = Vec::new();
    for (name, flags, data) in &cases {
        let got = run_case(data, *flags);
        let Some(want) = expected.get(&(name.clone(), flags.to_string())) else {
            panic!("{name} {flags}: no golden line");
        };
        if want != "CRASH" && &got != want {
            failures.push(format!("{name} {flags}\n  want {want}\n  got  {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        cases.len(),
        failures[..failures.len().min(8)].join("\n")
    );
}

#[test]
fn decompress_from_memory() {
    let d = std::fs::read("tests/resources/imageio/jpg/h2v2_33x31.jpg").unwrap_or_default();
    for req in [1, 3, 4] {
        let Some((img, w, h, comps)) = decompress_jpeg_image_from_memory(&d, req, 1) else {
            panic!("decode failed for {req}");
        };
        assert_eq!((w, h, comps), (33, 31, 3));
        assert_eq!(img.len(), (33 * 31 * req) as usize);
    }
    assert!(decompress_jpeg_image_from_memory(&d, 2, 0).is_none());
    assert!(decompress_jpeg_image_from_memory(&d[..10], 4, 0).is_none());
}

#[test]
fn random_corruptions_never_panic() {
    let mut rng = Lcg(4242);
    for name in files() {
        let d = std::fs::read(&name).unwrap_or_default();
        if d.len() > 4096 {
            continue;
        }
        for _ in 0..300 {
            let mut c = d.clone();
            for _ in 0..1 + rng.next() % 6 {
                let pos = rng.next() as usize % c.len();
                c[pos] = rng.next() as u8;
            }
            if rng.next().is_multiple_of(4) {
                let len = rng.next() as usize % c.len();
                c.truncate(len);
            }
            let _ = run_case(&c, rng.next() & 1);
        }
    }
}
