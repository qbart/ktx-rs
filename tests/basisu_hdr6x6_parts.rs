// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! Building blocks of the basisu ASTC HDR 6x6 encoder (`basisu_astc_hdr_6x6_enc.cpp`) vs. golden
//! output of the C++ original (`scripts/golden/gen_basisu_hdr6x6_parts_golden.sh`).
#![allow(clippy::panic, clippy::needless_range_loop)]

use std::collections::BTreeMap;

use ktx::basisu::astc_6x6_hdr::block_mode_desc;
use ktx::basisu::astc_hdr_6x6_enc::*;
use ktx::basisu::astc_hdr_enc::{AstcHdrCodecBaseOptions, OptMode, Q_LOG_BIAS_6X6, half_to_qlog16};
use ktx::basisu::astc_helpers::{get_ise_levels, half_to_float, pack_astc_block};
use ktx::basisu::comp::ImageF;
use ktx::basisu::huffman_enc::BitwiseCoder;

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
    fn i(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    fn f(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }
    fn d(&mut self, v: f64) {
        self.bytes(&v.to_le_bytes());
    }
}

fn rand_half(l: &mut Lcg, lo: i32, hi: i32) -> u32 {
    let e = (lo + (l.next() % (hi - lo + 1) as u32) as i32 + 15).clamp(0, 30);
    ((e as u32) << 10) | (l.next() & 1023)
}

#[allow(clippy::type_complexity)]
fn make_block(b: u32) -> ([[u16; 3]; 36], [[f32; 4]; 36], [[f32; 4]; 36]) {
    let mut l = Lcg(b.wrapping_mul(7919).wrapping_add(29));
    let kind = b % 6;
    let base = [
        rand_half(&mut l, -4, 6),
        rand_half(&mut l, -4, 6),
        rand_half(&mut l, -4, 6),
    ];
    let base2 = [
        rand_half(&mut l, -4, 6),
        rand_half(&mut l, -4, 6),
        rand_half(&mut l, -4, 6),
    ];
    let base3 = [
        rand_half(&mut l, -4, 6),
        rand_half(&mut l, -4, 6),
        rand_half(&mut l, -4, 6),
    ];
    let mut h = [[0u16; 3]; 36];
    let mut q16 = [[0f32; 4]; 36];
    let mut f = [[0f32; 4]; 36];
    for i in 0..36u32 {
        let (x, y) = (i % 6, i / 6);
        for c in 0..3usize {
            let mut v = match kind {
                0 => rand_half(&mut l, -8, 12),
                1 => base[c] + (x * 30 + y * 20) * (c as u32 + 1),
                2 => {
                    if x + y < 5 {
                        base[c]
                    } else {
                        base2[c]
                    }
                }
                3 => {
                    if x < 2 {
                        base[c]
                    } else if y < 3 {
                        base2[c]
                    } else {
                        base3[c]
                    }
                }
                4 => base[0] + (l.next() % 64),
                _ => base[c] + (l.next() % 300),
            };
            if v > 0x7BFF {
                v = 0x7BFF;
            }
            let iu = i as usize;
            h[iu][c] = v as u16;
            f[iu][c] = half_to_float(v as u16);
            q16[iu][c] = half_to_qlog16(v as i32) as f32;
        }
        f[i as usize][3] = 1.0;
    }
    (h, q16, f)
}

fn make_image(seed: u32, w: u32, h: u32) -> ImageF {
    let mut l = Lcg(seed.wrapping_mul(131).wrapping_add(7));
    let mut img = ImageF::with_size(w, h);
    for y in 0..h {
        for x in 0..w {
            let base = ((x * 7 + y * 3) % 23) as f32 * 0.25;
            for c in 0..3 {
                let v = if seed & 1 != 0 {
                    base * (c + 1) as f32 + (l.next() % 100) as f32 * 0.01
                } else {
                    half_to_float(rand_half(&mut l, -6, 8) as u16)
                };
                img.get_mut(x, y)[c] = v;
            }
            img.get_mut(x, y)[3] = 1.0;
        }
    }
    img
}

fn tables_hash() -> u64 {
    let t = hdr6x6_tables();
    let mut hh = H(0xcbf29ce484222325);
    for p in &t.partitions2 {
        hh.bytes(p);
    }
    for &v in &t.part2_seed_to_unique_index {
        hh.i(v);
    }
    for p in &t.partitions3 {
        hh.bytes(p);
    }
    for &v in &t.part3_seed_to_unique_index {
        hh.i(v);
    }
    for w in 1..=6 {
        for i in 0..6 {
            let cl = &t.contrib_lists[w][i];
            hh.u(cl.n as u32);
            for c in cl.p.iter().take(cl.n as usize) {
                hh.u(c.pixel as u32);
                hh.f(c.weight);
            }
        }
    }
    hh.0
}

fn pq_hash(i: u32) -> u64 {
    let mut l = Lcg(i.wrapping_mul(17).wrapping_add(1));
    let mut hh = H(0xcbf29ce484222325);
    for k in 0..16u32 {
        let v = match l.next() % 4 {
            0 => (l.next() % 100000) as f32 * 0.0001,
            1 => half_to_float(rand_half(&mut l, -15, 15) as u16),
            2 => (l.next() % 1000) as f32 * 1e-7,
            _ => (l.next() % 60000) as f32,
        };
        hh.f(forward_pq_tab_pub(v));
        let g = half_to_float(rand_half(&mut l, -10, 10) as u16);
        let b = (l.next() % 500) as f32 * 0.1;
        let out = linear_rgb_to_ictcp([v, g, b], k & 1 != 0, k & 2 != 0);
        hh.f(out[0]);
        hh.f(out[1]);
        hh.f(out[2]);
    }
    hh.0
}

fn coder_hash(hh: &mut H, c: &mut BitwiseCoder) {
    c.flush();
    hh.u(c.get_total_bits_u32());
    let b = c.get_bytes().clone();
    hh.bytes(&b);
}

fn block_hash(b: u32) -> u64 {
    let (h, q16, f) = make_block(b);
    let mut l = Lcg(b.wrapping_mul(31).wrapping_add(11));
    let mut hh = H(0xcbf29ce484222325);
    let t = hdr6x6_tables();
    let nb = [1usize, 6, 24, 48];
    let mut parts = [0i32; 64];
    for &n in &nb {
        parts = [0; 64];
        let s2 = estimate_partition2_6x6(&h, &mut parts, n);
        hh.u(s2 as u32);
        for &p in &parts[..n] {
            hh.i(p);
        }
        parts = [0; 64];
        let s3 = estimate_partition3_6x6(&h, &mut parts, n);
        hh.u(s3 as u32);
        for &p in &parts[..n] {
            hh.i(p);
        }
    }
    let _ = parts;
    let mut hf = [[0f32; 3]; 36];
    for i in 0..36 {
        hf[i] = [h[i][0] as f32, h[i][1] as f32, h[i][2] as f32];
    }
    let mut cand = [0i32; 64];
    estimate_partition2_6x6(&h, &mut cand, 48);
    let mut coptions = AstcHdrCodecBaseOptions {
        q_log_bias: Q_LOG_BIAS_6X6,
        ..Default::default()
    };
    if b & 1 != 0 {
        coptions.r_err_scale = 1.0;
        coptions.g_err_scale = 1.0;
    }
    let mut m11 = [0i32; 64];
    let mut m7 = [0i32; 64];
    let cand_u: Vec<u32> = cand[..48].iter().map(|&c| c as u32).collect();
    estimate_partitions_mode7_and_11(2, &t.partitions2, &cand_u, &hf, 8, &mut m11, &mut m7);
    for i in 0..8 {
        hh.i(m11[i]);
    }
    for i in 0..8 {
        hh.i(m7[i]);
    }
    {
        let all2: Vec<u32> = (0..NUM_UNIQUE_PARTITIONS2).collect();
        estimate_partitions_mode7_and_11(2, &t.partitions2, &all2, &hf, 20, &mut m11, &mut m7);
        for i in 0..20 {
            hh.i(m11[i]);
        }
        for i in 0..20 {
            hh.i(m7[i]);
        }
    }
    let mut c3 = [0i32; 64];
    let mut p3 = [0i32; 64];
    let s3 = estimate_partition3_6x6(&h, &mut c3, 48);
    if s3 {
        let c3u: Vec<u32> = c3[..48].iter().map(|&c| c as u32).collect();
        estimate_partitions_mode7(3, &t.partitions3, &c3u, &hf, 8, &mut p3);
    }
    for i in 0..8 {
        hh.i(p3[i]);
    }
    for tt in 0..3usize {
        let (bm, md) = loop {
            let bm = l.next() % 75;
            let md = block_mode_desc(bm).unwrap_or_default();
            if md.num_partitions >= 2 {
                break (bm, md);
            }
        };
        let om = if l.next() & 1 != 0 {
            OptMode::WeightedLeastSquares
        } else {
            OptMode::OrdinaryLeastSquares
        };
        let uber = (l.next() & 1) != 0;
        let comp_level = l.next() % 5;
        if md.num_partitions == 2 {
            let mut res = [TrialResult::default(); 2];
            let refine = (l.next() & 1) != 0;
            let st = encode_block_2_subsets(
                &mut res,
                md.grid_x,
                md.grid_y,
                md.cem,
                md.weight_ise_range,
                md.endpoint_ise_range,
                &h,
                &q16,
                &coptions,
                uber,
                if md.cem == 11 { m11[tt] } else { m7[tt] },
                comp_level,
                om,
                refine,
            );
            hh.u(st as u32);
            for r in res.iter() {
                hh.u((st && r.valid) as u32);
                if st && r.valid {
                    let mut blk = [0u8; 16];
                    pack_astc_block(&mut blk, &r.log_blk);
                    hh.bytes(&blk);
                    let mut c = BitwiseCoder::new();
                    code_block(&mut c, &r.log_blk, bm, EM_RAW, None);
                    coder_hash(&mut hh, &mut c);
                }
            }
        } else if s3 {
            let mut res = TrialResult::default();
            let n = 1 + (l.next() % 4) as usize;
            let st = encode_block_3_subsets(
                &mut res,
                md.cem,
                md.grid_x,
                md.grid_y,
                md.weight_ise_range,
                md.endpoint_ise_range,
                &h,
                &q16,
                &coptions,
                uber,
                &p3,
                n,
                comp_level,
                om,
            );
            hh.u(st as u32);
            if st {
                hh.d(res.err);
                let mut blk = [0u8; 16];
                pack_astc_block(&mut blk, &res.log_blk);
                hh.bytes(&blk);
                let mut c = BitwiseCoder::new();
                code_block(&mut c, &res.log_blk, bm, EM_RAW, None);
                coder_hash(&mut hh, &mut c);
            }
        }
    }
    let mut xf = [[0f32; 4]; 36];
    let mut yf = [[0f32; 4]; 36];
    filter_block_v4(3, 6, &f, &mut xf);
    filter_block_v4(6, 3, &f, &mut yf);
    hh.f(diff_blocks(&f, &xf));
    hh.f(diff_blocks(&f, &yf));
    for i in 0..36 {
        hh.f(xf[i][0]);
        hh.f(xf[i][1]);
        hh.f(xf[i][2]);
        hh.f(yf[i][2]);
    }
    let mut lp = [[0f32; 3]; 36];
    filter_block_v3(3, 3, &hf, &mut lp);
    hh.f(sub_and_compute_std_dev(&lp, &hf));
    let g = 1 + (l.next() % 6);
    let g2 = 1 + (l.next() % 6);
    filter_block_v3(g, g2, &hf, &mut lp);
    for i in 0..36 {
        hh.f(lp[i][1]);
    }
    let chans: Vec<Vec<f32>> = (0..3).map(|c| hf.iter().map(|p| p[c]).collect()).collect();
    let st: Vec<StatsF> = chans
        .iter()
        .map(|v| StatsF::calc(v.iter().copied()))
        .collect();
    for s in &st {
        hh.f(s.std_dev);
        hh.f(s.range);
        hh.f(s.avg);
    }
    hh.f(calc_pearson(&chans[0], &chans[1], &st[0], &st[1]));
    let cfg = AstcHdr6x6GlobalConfig {
        rec2020_bt2100_color_gamut: (b & 2) != 0,
        ..Default::default()
    };
    let mut itp_a = [[0f32; 3]; 36];
    let mut itp_b = [[0f32; 3]; 36];
    for i in 0..36 {
        itp_a[i] = linear_rgb_to_itp([f[i][0], f[i][1], f[i][2]], &cfg);
        itp_b[i] = linear_rgb_to_itp([xf[i][0], xf[i][1], xf[i][2]], &cfg);
    }
    hh.f(compute_block_ssim_itp(&itp_a, &itp_b));
    hh.f(compute_block_mse_itp(&itp_a, &itp_b, true));
    hh.f(compute_block_mse_itp(&itp_a, &itp_b, false));
    hh.f(compute_pixel_delta_itp(
        itp_a[3],
        itp_b[3],
        itp_a[3],
        (b & 4) != 0,
    ));
    for r in 0..=20u32 {
        let levels = get_ise_levels(r);
        let n = 1 + (l.next() % 64) as usize;
        let mut vals = [0u8; 64];
        for v in vals.iter_mut().take(n) {
            *v = (l.next() % levels) as u8;
        }
        let mut c = BitwiseCoder::new();
        hh.u(encode_values(&mut c, n, &vals, r));
        c.flush();
        let bytes = c.get_bytes().clone();
        hh.bytes(&bytes);
    }
    hh.0
}

fn image_hash(i: u32) -> u64 {
    let mut hh = H(0xcbf29ce484222325);
    let w = 5 + (i * 3) % 31;
    let h = 4 + (i * 5) % 23;
    let img = make_image(i, w, h);
    let (tm, tst) = tonemap_image_compressive2(&img);
    hh.u(tst as u32);
    for p in tm.get_pixels() {
        hh.bytes(p);
    }
    let sm = create_smooth_maps2(&tm, &SmoothMapParams::default());
    for v in &sm {
        hh.f(*v);
    }
    let mut f1 = ImageF::with_size(w, h);
    let mut f2 = ImageF::with_size(w, h);
    image_resample_f(&img, &mut f1, "gaussian", 1.45);
    image_resample_f(&img, &mut f2, "gaussian", 1.83);
    for y in 0..h {
        for x in 0..w {
            for v in f1.get(x, y) {
                hh.f(v);
            }
            for v in f2.get(x, y) {
                hh.f(v);
            }
        }
    }
    let cfg = AstcHdr6x6GlobalConfig {
        rec2020_bt2100_color_gamut: (i & 1) != 0,
        ..Default::default()
    };
    let itp = convert_rgb_image_to_itp(&img, &cfg);
    for y in 0..h {
        for x in 0..w {
            let p = itp.get(x, y);
            hh.f(p[0]);
            hh.f(p[1]);
            hh.f(p[2]);
        }
    }
    let mut cand = [[0f32; 3]; 36];
    for by in 0..h.div_ceil(6) {
        for bx in 0..w.div_ceil(6) {
            for y in 0..6 {
                for x in 0..6 {
                    let p = itp.get_clamped((bx * 6 + x + 1) as i32, (by * 6 + y) as i32);
                    cand[(y * 6 + x) as usize] = [p[0], p[1], p[2]];
                }
            }
            hh.f(calc_deblocking_penalty_itp(bx, by, w, h, &itp, &cand));
        }
    }
    hh.0
}

fn strips_hash() -> u64 {
    let mut hh = H(0xcbf29ce484222325);
    let mut nby = 1;
    while nby <= 70 {
        let mut th = 1;
        while th <= 9 {
            for k in 0..4 {
                let r = calc_strip_size(if k & 1 != 0 { 0.0 } else { 1.5 }, nby, th, k & 2 != 0);
                let (ts, rps) = r.unwrap_or((0, 0));
                hh.u(r.is_some() as u32);
                hh.u(ts);
                hh.u(rps);
            }
            th += 2;
        }
        nby += 3;
    }
    hh.0
}

#[test]
fn hdr6x6_parts_match_basisu() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/basisu_hdr6x6_parts.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let mut fails = Vec::new();
    for (key, want) in &golden {
        let num = |p: &str| key[p.len()..].parse::<u32>().unwrap_or(0);
        let got = if key == "tables" {
            tables_hash()
        } else if key == "strips" {
            strips_hash()
        } else if key.starts_with("pq") {
            pq_hash(num("pq"))
        } else if key.starts_with("img") {
            image_hash(num("img"))
        } else {
            block_hash(num("b"))
        };
        let got = format!("{got:016x}");
        if *want != got {
            fails.push(format!("{key}: want {want} got {got}"));
        }
    }
    if !fails.is_empty() {
        panic!(
            "{} of {} mismatches:\n{}",
            fails.len(),
            golden.len(),
            fails[..fails.len().min(40)].join("\n")
        );
    }
}
