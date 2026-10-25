//! basisu UASTC HDR 4x4 encoder (`basisu_uastc_hdr_4x4_enc.cpp`, `basisu_astc_hdr_common.cpp`)
//! and `unpack_bc6h` vs. golden output of the C++ original
//! (`scripts/golden/gen_basisu_uastc_hdr_4x4_enc_golden.sh`).
#![allow(clippy::panic, clippy::manual_is_multiple_of)]

use std::collections::BTreeMap;

use ktx::basisu::astc_hdr_enc::compute_block_error;
use ktx::basisu::astc_helpers::half_to_float;
use ktx::basisu::gpu_texture::unpack_bc6h;
use ktx::basisu::uastc_hdr_4x4_enc::*;

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
    fn d(&mut self, v: f64) {
        self.bytes(&v.to_le_bytes());
    }
}

fn rand_half(l: &mut Lcg, lo: i32, hi: i32) -> u32 {
    let mut e = lo + (l.next() % (hi - lo + 1) as u32) as i32 + 15;
    e = e.clamp(0, 30);
    ((e as u32) << 10) | (l.next() & 1023)
}

fn make_block(b: u32) -> [u16; 48] {
    let mut l = Lcg(b.wrapping_mul(7919).wrapping_add(17));
    let kind = b % 9;
    let base = [
        rand_half(&mut l, -6, 6),
        rand_half(&mut l, -6, 6),
        rand_half(&mut l, -6, 6),
    ];
    let base2 = [
        rand_half(&mut l, -6, 6),
        rand_half(&mut l, -6, 6),
        rand_half(&mut l, -6, 6),
    ];
    let mut h = [0u16; 48];
    for i in 0..16u32 {
        let (x, y) = (i & 3, i >> 2);
        for c in 0..3u32 {
            let cu = c as usize;
            let mut v = match kind {
                0 => rand_half(&mut l, -10, 15),
                1 => base[0] + (l.next() % 64),
                2 => base[cu],
                3 => base[cu] + u32::from(i == 5),
                4 => base[cu] + (x * 40 + y * 25) * (c + 1),
                5 => {
                    if ((x >> 1) + (y >> 1)) & 1 != 0 {
                        base[cu]
                    } else {
                        base2[cu]
                    }
                }
                6 => {
                    if l.next() & 7 == 0 {
                        0
                    } else {
                        rand_half(&mut l, -14, -8)
                    }
                }
                7 => rand_half(&mut l, 8, 15),
                _ => base[cu] + (l.next() % 200),
            };
            if v > 0x7BFF {
                v = 0x7BFF;
            }
            h[(i * 3 + c) as usize] = v as u16;
        }
        if kind == 1 {
            let v = h[(i * 3) as usize];
            h[(i * 3 + 1) as usize] = v;
            h[(i * 3 + 2) as usize] = v;
        }
    }
    h
}

/// Wide dynamic range blocks: random texels or two regions with unrelated colors, which reach
/// the 4 weight level fallback of the 2 subset mode 7 search at level 2.
fn make_wide_block(b: u32) -> [u16; 48] {
    let mut l = Lcg(b.wrapping_mul(104729).wrapping_add(5));
    let kind = b % 3;
    let c0 = [
        rand_half(&mut l, -14, 15),
        rand_half(&mut l, -14, 15),
        rand_half(&mut l, -14, 15),
    ];
    let c1 = [
        rand_half(&mut l, -14, 15),
        rand_half(&mut l, -14, 15),
        rand_half(&mut l, -14, 15),
    ];
    let mut h = [0u16; 48];
    for i in 0..16usize {
        for c in 0..3usize {
            let v = match kind {
                0 => rand_half(&mut l, -14, 15),
                1 => {
                    if i < 8 {
                        c0[c]
                    } else {
                        c1[c]
                    }
                }
                _ => {
                    if (i & 3) < 2 {
                        c0[c]
                    } else {
                        c1[c]
                    }
                }
            };
            h[i * 3 + c] = v.min(0x7BFF) as u16;
        }
    }
    h
}

// (level, perceptual, uber, ultra, favor_astc)
const CFGS: [(i32, bool, bool, bool, bool); 11] = [
    (0, true, false, false, false),
    (1, true, false, false, false),
    (2, true, false, false, false),
    (3, true, false, false, false),
    (4, true, false, false, false),
    (0, false, false, false, false),
    (1, false, false, false, true),
    (2, false, true, false, false),
    (3, true, true, true, false),
    (4, false, true, true, true),
    (1, true, false, true, false),
];

/// Hashes every result of every config (`all_cfgs`) or of the per block subset of configs.
fn block_hash(b: u32, hb: [u16; 48], all_cfgs: bool) -> u64 {
    let mut fb = [0f32; 48];
    for (f, &h) in fb.iter_mut().zip(hb.iter()) {
        *f = half_to_float(h);
    }
    let mut src = [[0u16; 3]; 16];
    for (i, s) in src.iter_mut().enumerate() {
        s.copy_from_slice(&hb[i * 3..i * 3 + 3]);
    }
    let mut hh = H(0xcbf29ce484222325);
    for (c, &(level, perceptual, uber, ultra, favor_astc)) in CFGS.iter().enumerate() {
        if !all_cfgs && (b as usize + c) % 3 != 0 && c != 1 {
            continue;
        }
        let mut o = UastcHdr4x4CodecOptions::default();
        o.set_quality_level(level);
        o.base.allow_uber_mode = uber;
        o.base.ultra_quant = ultra;
        if !perceptual {
            o.base.r_err_scale = 1.0;
            o.base.g_err_scale = 1.0;
        }
        o.bc6h_err_weight = if favor_astc { 0.1 } else { 0.65 };
        let mut res = Vec::new();
        let ok = astc_hdr_4x4_enc_block(&fb, &hb, &o, &mut res);
        hh.u(c as u32);
        hh.u(ok as u32);
        hh.u(res.len() as u32);
        let mut best_err = 1e30f32 as f64;
        let mut best = -1i32;
        for (i, r) in res.iter().enumerate() {
            hh.d(r.best_block_error);
            hh.u(r.is_solid as u32);
            hh.u(r.best_submodes[0]);
            hh.u(r.best_submodes[1]);
            hh.u(r.best_pat_index);
            hh.u(r.constrained_weights as u32);
            hh.u(r.improved_via_refinement_flag as u32);
            let mut blk = [0u8; 16];
            let pk = astc_hdr_4x4_pack_results_to_block(&mut blk, r);
            hh.u(pk as u32);
            hh.bytes(&blk);
            hh.bytes(&r.bc6h_block);
            let mut un = [[0u16; 3]; 16];
            unpack_bc6h(&r.bc6h_block, &mut un, false);
            let e6 = compute_block_error(16, &src, &un, &o.base);
            let w = o.bc6h_err_weight as f64;
            let overall = e6 * w + r.best_block_error * (1.0f32 - o.bc6h_err_weight) as f64;
            if i == 0 || overall < best_err {
                best_err = overall;
                best = i as i32;
            }
            hh.d(e6);
        }
        hh.u(best as u32);
    }
    hh.0
}

fn unpack_hash(b: u32) -> u64 {
    let mut l = Lcg(b.wrapping_mul(31).wrapping_add(3));
    let mut blk = [0u8; 16];
    for x in blk.iter_mut() {
        *x = l.next() as u8;
    }
    let mut hh = H(0xcbf29ce484222325);
    for s in 0..2 {
        let mut out = [[0x5555u16; 3]; 16];
        let ok = unpack_bc6h(&blk, &mut out, s != 0);
        hh.u(ok as u32);
        for p in out.iter() {
            for v in p.iter() {
                hh.bytes(&v.to_le_bytes());
            }
        }
    }
    hh.0
}

#[test]
fn uastc_hdr_4x4_enc_matches_basisu() {
    let golden: BTreeMap<String, String> =
        include_str!("resources/golden/basisu_uastc_hdr_4x4_enc.txt")
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
    let mut fails = Vec::new();
    let mut n = 0;
    for (key, want) in &golden {
        let Ok(idx) = key[1..].parse::<u32>() else {
            panic!("bad key {key}");
        };
        let got = format!(
            "{:016x}",
            if key.starts_with('b') {
                block_hash(idx, make_block(idx), false)
            } else if key.starts_with('w') {
                block_hash(idx, make_wide_block(idx), true)
            } else {
                unpack_hash(idx)
            }
        );
        n += 1;
        if *want != got {
            fails.push(format!("{key}: want {want} got {got}"));
        }
    }
    if !fails.is_empty() {
        panic!(
            "{} of {n} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(40)].join("\n")
        );
    }
}
