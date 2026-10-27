// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! basisu bc7enc (`basisu_bc7enc.cpp`) vs. golden output of the C++ original
//! (`scripts/golden/gen_basisu_bc7enc_golden.sh`).
#![allow(clippy::panic, clippy::needless_range_loop)]

use std::collections::BTreeMap;

use ktx::basisu::bc7enc::*;
use ktx::basisu::enc::{ColorRgba, color_rgba};
use ktx::basisu::uastc_tables::*;

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
    fn q(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
}

fn make_block(b: u32) -> [ColorRgba; 16] {
    let mut l = Lcg(b.wrapping_mul(7919).wrapping_add(13));
    let kind = b % 7;
    let (br, bg, bb) = (l.next() & 255, l.next() & 255, l.next() & 255);
    let (r2, g2, b2) = (l.next() & 255, l.next() & 255, l.next() & 255);
    let mut px = [[0u8; 4]; 16];
    for i in 0..16 {
        let (n0, n1, n2, n3) = (l.next(), l.next(), l.next(), l.next());
        let (r, g, bl): (i32, i32, i32) = match kind {
            0 => ((n0 & 255) as i32, (n1 & 255) as i32, (n2 & 255) as i32),
            1 => {
                let (x, y) = ((i & 3) as i32, (i >> 2) as i32);
                (
                    br as i32 + x * 20 - 30,
                    bg as i32 + y * 15 - 20,
                    bb as i32 + (x + y) * 9 - 25,
                )
            }
            2 => (br as i32, bg as i32, bb as i32),
            3 => {
                if n0 & 1 != 0 {
                    (br as i32, bg as i32, bb as i32)
                } else {
                    (r2 as i32, g2 as i32, b2 as i32)
                }
            }
            4 => {
                let v = br as i32 + (n0 % 9) as i32 - 4;
                (v, v + (n1 % 3) as i32 - 1, v)
            }
            5 => (
                br as i32 + (n0 % 3) as i32,
                bg as i32 + (n1 % 3) as i32,
                bb as i32 + (n2 % 3) as i32,
            ),
            _ => (
                if b & 8 != 0 {
                    255 - (n0 % 6) as i32
                } else {
                    (n0 % 6) as i32
                },
                if b & 16 != 0 {
                    255 - (n1 % 6) as i32
                } else {
                    (n1 % 6) as i32
                },
                (n2 & 255) as i32,
            ),
        };
        let a = match b % 3 {
            0 => 255,
            1 => (n3 & 255) as i32,
            _ => ((i as u32 * 17 + br) & 255) as i32,
        };
        px[i] = color_rgba(r, g, bl, a);
    }
    px
}

// (mode, range, comp_bits, pbits, share, table, alpha)
const CFGS: [(u32, u32, u32, bool, bool, u32, bool); 16] = [
    (255, 19, 0, false, false, 3, false),
    (255, 19, 0, false, false, 3, true),
    (255, 11, 0, false, false, 4, false),
    (255, 8, 0, false, false, 1, false),
    (255, 7, 0, false, false, 0, false),
    (255, 8, 0, false, false, 0, true),
    (255, 13, 0, false, false, 0, false),
    (255, 20, 0, false, false, 0, false),
    (255, 20, 0, false, false, 0, true),
    (255, 5, 0, false, false, 5, true),
    (255, 13, 0, false, false, 3, false),
    (1, 0, 6, true, true, 1, false),
    (6, 0, 7, true, false, 2, true),
    (3, 0, 7, true, false, 0, false),
    (255, 0, 5, false, false, 0, false),
    (255, 0, 5, false, false, 1, false),
];
const NPS: [u32; 6] = [16, 16, 8, 5, 2, 1];

fn table(t: u32) -> (u32, &'static [u32], &'static [Bc7encVec4F]) {
    match t {
        0 => (4, &BC7_WEIGHTS2, &BC7_WEIGHTS2X),
        1 => (8, &BC7_WEIGHTS3, &BC7_WEIGHTS3X),
        2 => (16, &BC7_WEIGHTS4, &BC7_WEIGHTS4X),
        3 => (16, &ASTC_WEIGHTS4, &ASTC_WEIGHTS4X),
        4 => (32, &ASTC_WEIGHTS5, &ASTC_WEIGHTS5X),
        _ => (3, &ASTC_WEIGHTS_3LEVELS, &ASTC_WEIGHTS_3LEVELSX),
    }
}

fn block_hash(b: u32) -> u64 {
    let px = make_block(b);
    let mut l = Lcg(b.wrapping_mul(31).wrapping_add(7));
    let mut h = H(0xcbf29ce484222325);
    for (ci, &(mode, range, comp_bits, pbits, share, tab, alpha)) in CFGS.iter().enumerate() {
        for v in 0..12u32 {
            let (n, w, wx) = table(tab);
            let np = NPS[((b + v + ci as u32) % 6) as usize];
            let ofs = ((b + v) % (17 - np)) as usize;
            let mut force = [0u8; 16];
            for f in force.iter_mut() {
                *f = (l.next() % n) as u8;
            }
            let perc = v & 1 != 0;
            let p = ColorCellCompressorParams {
                num_pixels: np,
                pixels: &px[ofs..],
                num_selector_weights: n,
                selector_weights: w,
                selector_weightsx: wx,
                astc_endpoint_range: range,
                comp_bits,
                has_pbits: pbits,
                endpoints_share_pbit: share,
                has_alpha: alpha,
                perceptual: perc,
                weights: if perc {
                    [128, 64, 16, 32]
                } else {
                    [1, 1, 1, 1]
                },
                force_selectors: if v >= 10 { Some(&force) } else { None },
            };
            let cp = Bc7encCompressBlockParams {
                uber_level: (v >> 1) % 5,
                least_squares_passes: 1 + ((v >> 2) & 1),
                ..Default::default()
            };
            let mut r = ColorCellCompressorResults::default();
            let err = color_cell_compression(mode, &p, &mut r, &cp);
            h.q(err);
            h.q(r.best_overall_err);
            h.bytes(&r.low_endpoint);
            h.bytes(&r.high_endpoint);
            h.u(r.pbits[0]);
            h.u(r.pbits[1]);
            h.bytes(&r.astc_low_endpoint);
            h.bytes(&r.astc_high_endpoint);
            h.bytes(&r.selectors);
        }
    }
    for t in 0..6 {
        for nc in 3..=4u32 {
            for wi in 0..2u32 {
                for bi in 0..2 {
                    let (n, w, _) = table(t);
                    let np = NPS[((b + t + nc + wi) % 6) as usize];
                    let ws = if wi != 0 {
                        [128, 64, 16, 32]
                    } else {
                        [1, 1, 1, 1]
                    };
                    h.q(color_cell_compression_est_astc(
                        n,
                        nc,
                        w,
                        np,
                        &px[(16 - np) as usize..],
                        if bi != 0 { 1500 } else { u64::MAX },
                        &ws,
                    ));
                }
            }
        }
    }
    h.0
}

#[test]
fn bc7enc_matches_basisu() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/basisu_bc7enc.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut th = H(0xcbf29ce484222325);
    let t = bc7enc_tables();
    for r in t.astc_sorted_order_unquant.iter() {
        for q in r.iter() {
            th.bytes(&[q.unquant, q.index]);
        }
    }
    let mut fails = Vec::new();
    let got = format!("{:016x}", th.0);
    if golden.get("tables") != Some(&got) {
        fails.push(format!("tables: got {got}"));
    }
    let n = golden.len() as u32 - 1;
    for b in 0..n {
        let got = format!("{:016x}", block_hash(b));
        match golden.get(&b.to_string()) {
            Some(w) if *w == got => {}
            Some(w) => fails.push(format!("{b}: want {w} got {got}")),
            None => fails.push(format!("{b}: missing")),
        }
    }
    if !fails.is_empty() {
        panic!(
            "{} of {n} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(30)].join("\n")
        );
    }
}
