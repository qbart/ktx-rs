// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! basisu `basis_compressor` (`basisu_comp.cpp`, `basisu_basis_file.cpp`) vs. golden output of
//! the C++ original (`scripts/golden/gen_basisu_comp_golden.sh`).
#![allow(clippy::panic)]

use std::collections::BTreeMap;

use ktx::basisu::comp::*;
use ktx::basisu::enc::color_rgba;

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

fn make_image(seed: u32, w: u32, h: u32, kind: u32, alpha: u32) -> Image {
    let mut l = Lcg(seed.wrapping_mul(7919).wrapping_add(11));
    let mut img = Image::with_size(w, h);
    for y in 0..h {
        for x in 0..w {
            let (xi, yi) = (x as i32, y as i32);
            let (mut r, mut g, mut b) = match kind {
                0 => (
                    (x * 255 / if w > 1 { w - 1 } else { 1 }) as i32,
                    (y * 255 / if h > 1 { h - 1 } else { 1 }) as i32,
                    128 + xi - yi,
                ),
                1 => {
                    let c = if ((x >> 2) + (y >> 2)) & 1 != 0 {
                        220
                    } else {
                        30
                    };
                    (c, (c + xi * 4) & 255, 255 - c)
                }
                2 => (
                    (l.next() & 255) as i32,
                    ((l.next() >> 4) & 255) as i32,
                    ((l.next() >> 8) & 255) as i32,
                ),
                _ => {
                    let c = (((x / 6) + (y / 5)) % 3) as i32 * 100 + ((x >> 1) & 1) as i32 * 9;
                    (c, 255 - c, (c * 2) & 255)
                }
            };
            if kind != 2 {
                r += (l.next() % 7) as i32 - 3;
                g += (l.next() % 7) as i32 - 3;
                b += (l.next() % 7) as i32 - 3;
            }
            let mut a = 255;
            if alpha == 1 {
                a = ((x * 37 + y * 11 + (l.next() & 15)) & 255) as i32;
            } else if alpha == 2 && x == w / 2 && y == h / 2 {
                a = 254;
            }
            *img.get_mut(x, y) = color_rgba(r, g, b, a);
        }
    }
    img
}

// (tex_type, n, w, h, kind, alpha_mask, halve)
const INPUTS: [(u32, u32, u32, u32, u32, u32, bool); 13] = [
    (0, 1, 16, 16, 0, 0, false),
    (0, 1, 13, 7, 2, 1, false),
    (0, 1, 37, 29, 1, 0, false),
    (0, 3, 40, 24, 0, 0, true),
    (0, 3, 22, 18, 3, 2, true),
    (3, 3, 24, 16, 1, 0, false),
    (3, 3, 20, 12, 0, 7, false),
    (0, 1, 64, 64, 3, 0, false),
    (0, 1, 1, 1, 2, 0, false),
    (0, 1, 5, 3, 0, 4, false),
    (1, 2, 12, 12, 2, 0, false),
    (0, 1, 128, 96, 2, 0, false),
    (0, 1, 96, 80, 0, 1, false),
];

fn make_input(i: u32, p: &mut BasisCompressorParams) {
    let (tex_type, n, mut w, mut h, kind, alpha_mask, halve) = INPUTS[i as usize];
    p.tex_type = tex_type;
    for k in 0..n {
        let mut alpha = (alpha_mask >> k) & 1;
        if alpha_mask == 4 {
            alpha = 2;
        }
        p.source_images
            .push(make_image(i * 17 + k, w, h, kind, alpha));
        if halve {
            w = if w > 1 { w / 2 } else { 1 };
            h = if h > 1 { h / 2 } else { 1 };
        }
    }
}

#[derive(Clone, Copy)]
struct Cfg {
    uastc: bool,
    quality: i32,
    max_ep: u32,
    max_sel: u32,
    level: i32,
    perceptual: bool,
    no_ep_rdo: bool,
    no_sel_rdo: bool,
    ep_thresh: f32,
    sel_thresh: f32,
    swizzle: usize,
    force_alpha: bool,
    check_alpha: bool,
    flags: u32,
    rdo: bool,
    lambda: f32,
    dict: i32,
    smooth_scale: f32,
    std_dev: f32,
    favor_simpler: bool,
    mt: bool,
    threads: u32,
}

const SWIZZLES: [[u8; 4]; 3] = [[0, 1, 2, 3], [0, 0, 0, 1], [2, 1, 0, 3]];

#[allow(clippy::too_many_arguments)]
const fn e(
    quality: i32,
    max_ep: u32,
    max_sel: u32,
    level: i32,
    perceptual: bool,
    no_ep_rdo: bool,
    no_sel_rdo: bool,
    ep_thresh: f32,
    sel_thresh: f32,
    swizzle: usize,
    force_alpha: bool,
    check_alpha: bool,
) -> Cfg {
    Cfg {
        uastc: false,
        quality,
        max_ep,
        max_sel,
        level,
        perceptual,
        no_ep_rdo,
        no_sel_rdo,
        ep_thresh,
        sel_thresh,
        swizzle,
        force_alpha,
        check_alpha,
        flags: 0,
        rdo: false,
        lambda: 0.0,
        dict: 0,
        smooth_scale: 0.0,
        std_dev: 0.0,
        favor_simpler: false,
        mt: false,
        threads: 1,
    }
}

#[allow(clippy::too_many_arguments)]
const fn u(
    flags: u32,
    rdo: bool,
    lambda: f32,
    dict: i32,
    smooth_scale: f32,
    std_dev: f32,
    favor_simpler: bool,
    mt: bool,
    threads: u32,
    swizzle: usize,
    force_alpha: bool,
    check_alpha: bool,
) -> Cfg {
    Cfg {
        uastc: true,
        quality: -1,
        max_ep: 0,
        max_sel: 0,
        level: 2,
        perceptual: true,
        no_ep_rdo: false,
        no_sel_rdo: false,
        ep_thresh: -1.0,
        sel_thresh: -1.0,
        swizzle,
        force_alpha,
        check_alpha,
        flags,
        rdo,
        lambda,
        dict,
        smooth_scale,
        std_dev,
        favor_simpler,
        mt,
        threads,
    }
}

const CFGS: [Cfg; 28] = [
    e(128, 0, 0, 2, true, false, false, -1.0, -1.0, 0, false, true),
    e(1, 0, 0, 1, true, false, false, -1.0, -1.0, 0, false, true),
    e(64, 0, 0, 0, false, false, false, -1.0, -1.0, 0, false, true),
    e(140, 0, 0, 3, true, false, false, -1.0, -1.0, 0, false, true),
    e(170, 0, 0, 2, true, false, false, 2.0, -1.0, 0, false, true),
    e(200, 0, 0, 4, true, true, false, -1.0, -1.0, 0, false, true),
    e(230, 0, 0, 5, false, false, true, -1.0, 0.5, 0, false, true),
    e(255, 0, 0, 6, true, false, false, -1.0, -1.0, 0, false, true),
    e(
        -1, 300, 200, 2, true, false, false, -1.0, -1.0, 0, false, true,
    ),
    e(-1, 16, 16, 1, false, true, true, -1.0, -1.0, 0, false, true),
    e(128, 0, 0, 2, true, false, false, -1.0, -1.0, 1, false, true),
    e(128, 0, 0, 2, true, false, false, -1.0, -1.0, 2, true, true),
    e(
        128, 0, 0, 2, true, false, false, -1.0, -1.0, 0, false, false,
    ),
    e(-1, 0, 0, 2, true, false, false, -1.0, -1.0, 0, false, true),
    u(2, false, 0.0, 0, 0.0, 0.0, true, true, 1, 0, false, true),
    u(0, false, 0.0, 0, 0.0, 0.0, true, true, 1, 0, false, true),
    u(
        1 | 64,
        false,
        0.0,
        0,
        0.0,
        0.0,
        true,
        true,
        1,
        1,
        false,
        true,
    ),
    u(
        3 | 16,
        false,
        0.0,
        0,
        0.0,
        0.0,
        true,
        true,
        1,
        0,
        true,
        true,
    ),
    u(
        4 | 8 | 128,
        false,
        0.0,
        0,
        0.0,
        0.0,
        true,
        true,
        1,
        2,
        false,
        false,
    ),
    u(
        2, true, 1.0, 4096, 10.0, 18.0, true, true, 1, 0, false, true,
    ),
    u(
        2, true, 3.0, 1024, 20.0, 9.0, false, true, 1, 0, false, true,
    ),
    u(
        1, true, 0.5, 8192, 10.0, 18.0, true, true, 4, 0, false, true,
    ),
    u(2, true, 5.0, 64, 1.0, 1.0, true, false, 4, 0, false, true),
    e(223, 0, 0, 2, true, false, false, -1.0, -1.0, 0, false, true),
    e(129, 0, 0, 2, true, false, false, -1.0, -1.0, 0, false, true),
    e(100, 0, 0, 1, true, false, false, -1.0, -1.0, 0, false, true),
    e(30, 0, 0, 2, false, false, false, -1.0, -1.0, 0, false, true),
    e(
        223, 0, 0, 6, false, false, false, -1.0, -1.0, 0, false, true,
    ),
];

fn case_hash(i: u32, cfg: &Cfg) -> u64 {
    let mut p = BasisCompressorParams {
        status_output: false,
        job_pool_threads: cfg.threads,
        ..Default::default()
    };
    make_input(i, &mut p);
    p.uastc = cfg.uastc;
    p.swizzle = SWIZZLES[cfg.swizzle];
    if cfg.force_alpha {
        p.force_alpha = true;
    }
    if !cfg.check_alpha {
        p.check_for_alpha = false;
    }
    if !cfg.uastc {
        p.quality_level = cfg.quality;
        p.etc1s_max_endpoint_clusters = cfg.max_ep;
        p.etc1s_max_selector_clusters = cfg.max_sel;
        p.etc1s_compression_level.set(cfg.level);
        p.set_srgb_options(cfg.perceptual);
        p.no_endpoint_rdo = cfg.no_ep_rdo;
        p.no_selector_rdo = cfg.no_sel_rdo;
        if cfg.ep_thresh >= 0.0 {
            p.endpoint_rdo_thresh.set(cfg.ep_thresh);
        }
        if cfg.sel_thresh >= 0.0 {
            p.selector_rdo_thresh.set(cfg.sel_thresh);
        }
    } else {
        p.pack_uastc_ldr_4x4_flags = cfg.flags;
        if cfg.rdo {
            p.rdo_uastc_ldr_4x4 = true;
            p.rdo_uastc_ldr_4x4_quality_scalar.set(cfg.lambda);
            p.rdo_uastc_ldr_4x4_dict_size.set(cfg.dict);
            p.rdo_uastc_ldr_4x4_max_smooth_block_error_scale
                .set(cfg.smooth_scale);
            p.rdo_uastc_ldr_4x4_smooth_block_max_std_dev
                .set(cfg.std_dev);
            p.rdo_uastc_ldr_4x4_favor_simpler_modes_in_rdo_mode = cfg.favor_simpler;
            p.rdo_uastc_ldr_4x4_multithreading = cfg.mt;
        }
    }
    let mut comp = BasisCompressor::new();
    let mut h = H(0xcbf29ce484222325);
    if !comp.init(&p) {
        h.u(0xFFFF_FFFF);
    } else {
        let ec = comp.process();
        h.u(ec as u32);
        if ec == BasisCompressorErrorCode::Success {
            let f = comp.get_output_basis_file();
            h.u(f.len() as u32);
            h.bytes(f);
        }
    }
    h.0
}

#[test]
fn basis_compressor_matches_basisu() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/basisu_comp.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut fails = Vec::new();
    let mut n = 0;
    for i in 0..INPUTS.len() as u32 {
        for (c, cfg) in CFGS.iter().enumerate() {
            let key = format!("{i}_{c}");
            let got = format!("{:016x}", case_hash(i, cfg));
            n += 1;
            match golden.get(&key) {
                Some(w) if *w == got => {}
                Some(w) => fails.push(format!("{key}: want {w} got {got}")),
                None => fails.push(format!("{key}: missing")),
            }
        }
    }
    assert_eq!(n, golden.len());
    if !fails.is_empty() {
        panic!(
            "{} of {n} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(40)].join("\n")
        );
    }
}
