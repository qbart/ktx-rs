// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! basisu ASTC HDR 6x6 / UASTC HDR 6x6 intermediate encoder (`astc_6x6_hdr::compress_photo`) vs.
//! golden output of the C++ original (`scripts/golden/gen_basisu_hdr6x6_enc_golden.sh`).
#![allow(clippy::panic)]

use std::collections::BTreeMap;

use ktx::basisu::astc_hdr_6x6_enc::{AstcHdr6x6GlobalConfig, compress_photo};
use ktx::basisu::comp::ImageF;

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
}

fn make_image(seed: u32, w: u32, h: u32) -> ImageF {
    let mut l = Lcg(seed.wrapping_mul(977).wrapping_add(3));
    let mut img = ImageF::with_size(w, h);
    let kind = seed % 5;
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = match kind {
                0 => (x as f32 * 0.5, y as f32 * 0.25 + 0.01, (x + y) as f32 * 0.1),
                1 => {
                    let s = if ((x / 4) + (y / 3)) & 1 != 0 {
                        40.0
                    } else {
                        0.05
                    };
                    (s, s * 0.5, s * 0.25)
                }
                2 => {
                    let r = (l.next() % 1000) as f32 * 0.02;
                    let g = (l.next() % 1000) as f32 * 0.002;
                    let b = (l.next() % 1000) as f32 * 0.05;
                    (r, g, b)
                }
                3 => {
                    let v = ((x * 3 + y * 5) % 17) as f32 * 0.3 + (l.next() % 10) as f32 * 0.001;
                    (v, v, v)
                }
                _ => {
                    let r = if x < w / 2 { 1.5 } else { 300.0 };
                    let g = if y < h / 3 { 0.002 } else { 2.0 };
                    let b = (l.next() % 50) as f32 * 0.1;
                    (r, g, b)
                }
            };
            *img.get_mut(x, y) = [r, g, b, 1.0];
        }
    }
    img
}

const IMGS: [(u32, u32); 6] = [(6, 6), (13, 7), (24, 18), (31, 20), (12, 30), (40, 26)];
// (level, lambda, rec2020, threads)
const CFGS: [(i32, f32, bool, u32); 18] = [
    (0, 0.0, false, 1),
    (1, 0.0, false, 1),
    (2, 0.0, false, 1),
    (3, 0.0, true, 1),
    (4, 0.0, false, 2),
    (5, 0.0, false, 1),
    (6, 0.0, false, 1),
    (7, 0.0, false, 3),
    (8, 0.0, true, 1),
    (9, 0.0, false, 1),
    (10, 0.0, false, 1),
    (11, 0.0, false, 1),
    (12, 0.0, false, 1),
    (2, 0.5, false, 1),
    (2, 2.0, false, 4),
    (0, 4.0, false, 1),
    (5, 1.0, true, 1),
    (12, 8.0, false, 1),
];

#[test]
fn hdr6x6_compress_photo_matches_basisu() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/basisu_hdr6x6_enc.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut fails = Vec::new();
    let mut n = 0;
    for (i, &(w, h)) in IMGS.iter().enumerate() {
        for (c, &(level, lambda, rec2020, threads)) in CFGS.iter().enumerate() {
            if (i + c) % 2 != 0 && c != 2 && c != 13 {
                continue;
            }
            let img = make_image((i * 31 + c) as u32, w, h);
            let mut cfg = AstcHdr6x6GlobalConfig::default();
            cfg.set_user_level(level);
            cfg.lambda = lambda;
            cfg.rec2020_bt2100_color_gamut = rec2020;
            let r = compress_photo(&img, &cfg, threads);
            let mut hh = H(0xcbf29ce484222325);
            hh.u(r.is_some() as u32);
            if let Some((inter, astc)) = &r {
                hh.u(inter.len() as u32);
                hh.bytes(inter);
                hh.u(astc.len() as u32);
                hh.bytes(astc);
            }
            let key = format!("{i}_{c}");
            let got = format!("{:016x}", hh.0);
            n += 1;
            match golden.get(&key) {
                Some(want) if *want == got => {}
                Some(want) => fails.push(format!("{key}: want {want} got {got}")),
                None => fails.push(format!("{key}: missing")),
            }
        }
    }
    assert_eq!(n, golden.len());
    if !fails.is_empty() {
        panic!("{} of {n} mismatches:\n{}", fails.len(), fails.join("\n"));
    }
}
