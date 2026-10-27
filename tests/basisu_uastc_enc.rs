// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! basisu UASTC LDR 4x4 encoder (`basisu_uastc_enc.cpp`) and BC1 / BC7 block decoders vs.
//! golden output of the C++ original (`scripts/golden/gen_basisu_uastc_enc_golden.sh`).
#![allow(clippy::panic, clippy::needless_range_loop)]

use std::collections::BTreeMap;

use ktx::basisu::gpu_texture::{unpack_bc1, unpack_bc7};
use ktx::basisu::uastc_enc::*;

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
}

fn make_block(b: u32) -> [u8; 64] {
    let mut l = Lcg(b.wrapping_mul(7919).wrapping_add(13));
    let kind = b % 9;
    let (br, bg, bb) = (
        (l.next() & 255) as i32,
        (l.next() & 255) as i32,
        (l.next() & 255) as i32,
    );
    let (r2, g2, b2) = (
        (l.next() & 255) as i32,
        (l.next() & 255) as i32,
        (l.next() & 255) as i32,
    );
    let mut px = [0u8; 64];
    for i in 0..16 {
        let (n0, n1, n2, n3) = (l.next(), l.next(), l.next(), l.next());
        let (r, g, bl): (i32, i32, i32) = match kind {
            0 => ((n0 & 255) as i32, (n1 & 255) as i32, (n2 & 255) as i32),
            1 => {
                let (x, y) = ((i & 3) as i32, (i >> 2) as i32);
                (br + x * 20 - 30, bg + y * 15 - 20, bb + (x + y) * 9 - 25)
            }
            2 => (br, bg, bb),
            3 => {
                if n0 & 1 != 0 {
                    (br, bg, bb)
                } else {
                    (r2, g2, b2)
                }
            }
            4 => {
                let v = br + (n0 % 9) as i32 - 4;
                (v, v + (n1 % 3) as i32 - 1, v)
            }
            5 => (
                br + (n0 % 3) as i32,
                bg + (n1 % 3) as i32,
                bb + (n2 % 3) as i32,
            ),
            6 => (
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
            7 => {
                let v = br + (i & 3) as i32 * 17 - (i >> 2) as i32 * 9;
                (v, v, v)
            }
            _ => {
                let v = if n0 & 1 != 0 { br } else { r2 };
                (v, v, v)
            }
        };
        let mut a = match b % 3 {
            0 => 255,
            1 => (n3 & 255) as i32,
            _ => (i as i32 * 17 + br) & 255,
        };
        if b % 11 == 5 {
            a = if n3 & 1 != 0 { 255 } else { 254 };
        }
        px[i * 4] = r.clamp(0, 255) as u8;
        px[i * 4 + 1] = g.clamp(0, 255) as u8;
        px[i * 4 + 2] = bl.clamp(0, 255) as u8;
        px[i * 4 + 3] = a.clamp(0, 255) as u8;
    }
    px
}

const EXTRA_FLAGS: [u32; 6] = [
    C_PACK_UASTC_FAVOR_UASTC_ERROR,
    C_PACK_UASTC_FAVOR_BC7_ERROR,
    C_PACK_UASTC_ETC1_FASTER_HINTS,
    C_PACK_UASTC_ETC1_FASTEST_HINTS,
    C_PACK_UASTC_ETC1_DISABLE_FLIP_AND_INDIVIDUAL,
    C_PACK_UASTC_FAVOR_SIMPLER_MODES,
];

fn block_hash(b: u32) -> u64 {
    let px = make_block(b);
    let mut h = H(0xcbf29ce484222325);
    for v in 0..11u32 {
        let flags = if v < 5 {
            if v == 4 && !b.is_multiple_of(5) {
                continue;
            }
            v
        } else {
            (if v == 10 { 3 } else { b % 3 }) | EXTRA_FLAGS[(v - 5) as usize]
        };
        let mut blk = [0u8; 16];
        encode_uastc(&px, &mut blk, flags);
        h.bytes(&blk);
    }
    let mut l = Lcg(b.wrapping_mul(101).wrapping_add(3));
    let mut raw = [0u8; 16];
    for r in raw.iter_mut() {
        *r = l.next() as u8;
    }
    let mut out = [[0u8; 4]; 16];
    let mut bc1 = [0u8; 8];
    bc1.copy_from_slice(&raw[..8]);
    unpack_bc1(&bc1, &mut out, true);
    h.bytes(out.as_flattened());
    let mut out = [[0u8; 4]; 16];
    let ok = unpack_bc7(&raw, &mut out);
    h.bytes(&[ok as u8]);
    h.bytes(out.as_flattened());
    h.0
}

#[test]
fn uastc_enc_matches_basisu() {
    let golden: BTreeMap<u32, String> = include_str!("resources/golden/basisu_uastc_enc.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter_map(|(k, v)| Some((k.parse().ok()?, v.to_string())))
        .collect();
    let mut fails = Vec::new();
    for (&b, want) in &golden {
        let got = format!("{:016x}", block_hash(b));
        if *want != got {
            fails.push(format!("{b}: want {want} got {got}"));
        }
    }
    assert_eq!(golden.len(), 3000);
    if !fails.is_empty() {
        panic!(
            "{} of {} mismatches:\n{}",
            fails.len(),
            golden.len(),
            fails[..fails.len().min(30)].join("\n")
        );
    }
}
