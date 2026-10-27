// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! astcenc partitioning search, angular endpoints, endpoint format selection and color
//! endpoint packing vs. golden output of the C++ original
//! (`scripts/golden/gen_astcenc_formats_golden.sh`).
#![allow(clippy::panic, clippy::needless_range_loop)]

mod common;

use std::collections::BTreeMap;

use common::astcenc_gen::*;
use ktx::astcenc::color_quantize::pack_color_endpoints;
use ktx::astcenc::find_best_partitioning::find_best_partition_candidates;
use ktx::astcenc::ideal_endpoints_and_weights::*;
use ktx::astcenc::image::load_image_block;
use ktx::astcenc::internal::*;
use ktx::astcenc::pick_best_endpoint_format::compute_ideal_endpoint_formats;
use ktx::astcenc::vecmath::{Vfloat4, Vint4};
use ktx::astcenc::weight_align::*;
use ktx::astcenc::*;

const FREE_BITS: [i32; 4] = [
    115 - 4,
    111 - 4 - PARTITION_INDEX_BITS as i32,
    108 - 4 - PARTITION_INDEX_BITS as i32,
    105 - 4 - PARTITION_INDEX_BITS as i32,
];

/// `compute_ideal_endpoint_formats` over a block mode range; hashes the candidates.
fn formats(
    h: &mut H,
    pi: &PartitionInfo,
    blk: &ImageBlock,
    ep: &Endpoints,
    tmp: &mut CompressionWorkingBuffers,
    start: u32,
    end: u32,
) {
    for limit in [1u32, 4, 8] {
        let mut pfs = [[0u8; BLOCK_MAX_PARTITIONS]; TUNE_MAX_TRIAL_CANDIDATES];
        let mut bm = [0i32; TUNE_MAX_TRIAL_CANDIDATES];
        let mut ql = [0u32; TUNE_MAX_TRIAL_CANDIDATES];
        let mut qlm = [0u32; TUNE_MAX_TRIAL_CANDIDATES];
        let bitcounts = tmp.qwt_bitcounts.clone();
        let errors = tmp.qwt_errors.clone();
        let n = compute_ideal_endpoint_formats(
            pi, blk, ep, &bitcounts, &errors, limit, start, end, &mut pfs, &mut bm, &mut ql,
            &mut qlm, tmp,
        );
        h.u(n);
        for i in 0..n as usize {
            h.u(bm[i] as u32);
            h.u(ql[i]);
            h.u(qlm[i]);
            h.bytes(&pfs[i][..pi.partition_count as usize]);
        }
    }
}

fn block_hash(bsd: &BlockSizeDescriptor, blk: &ImageBlock, bx: usize, kind: u32) -> u64 {
    let mut tmp = CompressionWorkingBuffers::default();
    let mut h = H::new();
    // Partitioning search
    for pc in 2..=4u32 {
        let n = bsd.partitioning_count_selected[pc as usize - 1];
        if n == 0 {
            continue;
        }
        for v in 0..2 {
            let mut bp = [0u32; TUNE_MAX_PARTITIONING_CANDIDATES];
            let limit = if v == 1 { n.min(7) } else { n };
            let req = if v == 1 { 2 } else { 8 };
            let got = find_best_partition_candidates(bsd, blk, pc, limit, &mut bp, req);
            h.u(got);
            for i in 0..got as usize {
                h.u(bp[i]);
            }
        }
    }
    // 1 plane: angular endpoints, weight errors and endpoint formats
    for pc in 1..=4u32 {
        let n = bsd.partitioning_count_selected[pc as usize - 1];
        if n == 0 {
            continue;
        }
        let pi = bsd.get_raw_partition_info(pc, (bx as u32 * 17 + kind) % n);
        let mut ei = EndpointsAndWeights::default();
        compute_ideal_colors_and_weights_1plane(blk, pi, &mut ei);
        for only_always in [false, true] {
            if only_always && bsd.decimation_mode_count_always == 0 {
                continue;
            }
            let mwq = if only_always { QUANT_8 } else { QUANT_32 };
            let ndm = if only_always {
                bsd.decimation_mode_count_always
            } else {
                bsd.decimation_mode_count_selected
            } as usize;
            for d in 0..ndm {
                if !bsd.get_decimation_mode(d).is_ref_1plane(mwq) {
                    continue;
                }
                compute_ideal_weights_for_decimation(
                    &ei,
                    bsd.get_decimation_info(d),
                    &mut tmp.dec_weights_ideal[d * BLOCK_MAX_WEIGHTS..],
                );
            }
            compute_angular_endpoints_1plane(only_always, bsd, mwq, &mut tmp);
            let nbm = if only_always {
                bsd.block_mode_count_1plane_always
            } else {
                bsd.block_mode_count_1plane_selected
            } as usize;
            for i in 0..nbm {
                let bm = bsd.block_modes[i];
                h.f(tmp.weight_low_value1[i]);
                h.f(tmp.weight_high_value1[i]);
                let bitcount = FREE_BITS[pc as usize - 1] - bm.weight_bits as i32;
                if bm.quant_mode as u32 > mwq || bitcount <= 0 {
                    tmp.qwt_errors[i] = 1e38;
                    continue;
                }
                tmp.qwt_bitcounts[i] = bitcount as i8;
                let di = bsd.get_decimation_info(bm.decimation_mode as usize);
                let mut uqf = [0.0f32; BLOCK_MAX_WEIGHTS];
                let dm = bm.decimation_mode as usize;
                compute_quantized_weights_for_decimation(
                    di,
                    tmp.weight_low_value1[i],
                    tmp.weight_high_value1[i],
                    &tmp.dec_weights_ideal[BLOCK_MAX_WEIGHTS * dm..],
                    &mut uqf,
                    &mut tmp.dec_weights_uquant[BLOCK_MAX_WEIGHTS * i..],
                    bm.get_weight_quant_mode(),
                );
                tmp.qwt_errors[i] = compute_error_of_weight_set_1plane(&ei, di, &uqf);
            }
            formats(&mut h, pi, blk, &ei.ep, &mut tmp, 0, nbm as u32);
        }
    }
    // 2 planes
    let mut p2 = 0u32;
    while p2 < 4 {
        let step = 3 - kind % 3;
        if p2 == 3 && blk.is_constant_channel(3) {
            p2 += step;
            continue;
        }
        let mut ei1 = EndpointsAndWeights::default();
        let mut ei2 = EndpointsAndWeights::default();
        compute_ideal_colors_and_weights_2planes(bsd, blk, p2, &mut ei1, &mut ei2);
        let mwq = QUANT_32;
        for d in 0..bsd.decimation_mode_count_selected as usize {
            if !bsd.get_decimation_mode(d).is_ref_2plane(mwq) {
                continue;
            }
            let di = bsd.get_decimation_info(d);
            compute_ideal_weights_for_decimation(
                &ei1,
                di,
                &mut tmp.dec_weights_ideal[d * BLOCK_MAX_WEIGHTS..],
            );
            compute_ideal_weights_for_decimation(
                &ei2,
                di,
                &mut tmp.dec_weights_ideal[d * BLOCK_MAX_WEIGHTS + WEIGHTS_PLANE2_OFFSET..],
            );
        }
        compute_angular_endpoints_2planes(bsd, mwq, &mut tmp);
        let start = bsd.block_mode_count_1plane_selected;
        let end = bsd.block_mode_count_1plane_2plane_selected;
        for i in start as usize..end as usize {
            let bm = bsd.block_modes[i];
            h.f(tmp.weight_low_value1[i]);
            h.f(tmp.weight_high_value1[i]);
            h.f(tmp.weight_low_value2[i]);
            h.f(tmp.weight_high_value2[i]);
            if bm.quant_mode as u32 > mwq {
                tmp.qwt_errors[i] = 1e38;
                continue;
            }
            tmp.qwt_bitcounts[i] = (109 - bm.weight_bits as i32) as i8;
            let di = bsd.get_decimation_info(bm.decimation_mode as usize);
            let mut u1 = [0.0f32; BLOCK_MAX_WEIGHTS];
            let mut u2 = [0.0f32; BLOCK_MAX_WEIGHTS];
            let id = BLOCK_MAX_WEIGHTS * bm.decimation_mode as usize;
            let q = BLOCK_MAX_WEIGHTS * i;
            compute_quantized_weights_for_decimation(
                di,
                tmp.weight_low_value1[i],
                tmp.weight_high_value1[i],
                &tmp.dec_weights_ideal[id..],
                &mut u1,
                &mut tmp.dec_weights_uquant[q..],
                bm.get_weight_quant_mode(),
            );
            compute_quantized_weights_for_decimation(
                di,
                tmp.weight_low_value2[i],
                tmp.weight_high_value2[i],
                &tmp.dec_weights_ideal[id + WEIGHTS_PLANE2_OFFSET..],
                &mut u2,
                &mut tmp.dec_weights_uquant[q + WEIGHTS_PLANE2_OFFSET..],
                bm.get_weight_quant_mode(),
            );
            tmp.qwt_errors[i] = compute_error_of_weight_set_2planes(&ei1, &ei2, di, &u1, &u2);
        }
        let p2m = Vint4::lane_id().eq(Vint4::splat(p2 as i32));
        let mut epm = Endpoints {
            partition_count: 1,
            ..Endpoints::default()
        };
        epm.endpt0[0] = Vfloat4::select(ei1.ep.endpt0[0], ei2.ep.endpt0[0], p2m);
        epm.endpt1[0] = Vfloat4::select(ei1.ep.endpt1[0], ei2.ep.endpt1[0], p2m);
        formats(
            &mut h,
            bsd.get_partition_info(1, 0),
            blk,
            &epm,
            &mut tmp,
            start,
            end,
        );
        p2 += step;
    }
    h.0
}

fn pack_hash(t: u32) -> u64 {
    let mut l = Lcg(0x5eed0000 + t);
    let mut comp = |lo: f32, hi: f32| lo + l.unit() * (hi - lo);
    let (lo, hi) = if t % 4 == 3 {
        (-2000.0, 70000.0)
    } else {
        (0.0, 65535.0)
    };
    // Argument evaluation order of the C++ vfloat4 constructor calls is left to right (g++)
    let mut v4 = |a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| {
        let x = comp(a.0, a.1);
        let y = comp(b.0, b.1);
        let z = comp(c.0, c.1);
        let w = comp(d.0, d.1);
        Vfloat4::new(x, y, z, w)
    };
    let r = (lo, hi);
    let mut c0 = v4(r, r, r, r);
    let mut c1 = v4(r, r, r, r);
    if t % 8 == 1 {
        let o = v4((0.0, 900.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0));
        c1 = c0 + Vfloat4::splat(o.m[0]);
    }
    if t % 8 == 2 {
        c1 = c0;
    }
    if t % 8 == 5 {
        c0 = c0 * 0.05;
        let o = v4((0.0, 3000.0), (0.0, 3000.0), (0.0, 3000.0), (0.0, 0.0));
        c1 = c0 + Vfloat4::new(o.m[0], o.m[1], o.m[2], 0.0);
    }
    let rgbs = v4(r, r, r, (0.0, 1.1));
    let rgbo = v4(r, r, r, (0.0, 20000.0));
    let mut h = H::new();
    for fmt in 0..16u8 {
        for q in QUANT_6..=QUANT_256 {
            let mut out = [0u8; 8];
            let ret = pack_color_endpoints(c0, c1, rgbs, rgbo, fmt, &mut out, q);
            h.bytes(&[ret]);
            h.bytes(&out);
        }
    }
    h.0
}

#[test]
fn formats_match_astcenc() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/astcenc_formats.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let profiles = [AstcencProfile::Ldr, AstcencProfile::Hdr];
    let mut fails = Vec::new();
    let mut n = 0;
    let mut check = |key: String, got: u64| {
        let got = format!("{got:016x}");
        match golden.get(&key) {
            Some(w) if *w == got => {}
            Some(w) => fails.push(format!("{key}: want {w} got {got}")),
            None => fails.push(format!("{key}: missing")),
        }
    };
    for (si, &(sx, sy, sz)) in SIZES.iter().enumerate() {
        let ctx = context(sx, sy, sz);
        let Some(bsd) = ctx.compress_bsd() else {
            panic!("no bsd")
        };
        for dt in 0..3 {
            for kind in 0..4u32 {
                let img = make_image(si, (sx, sy, sz), dt, kind);
                for (pi_, &prof) in profiles.iter().enumerate() {
                    for bx in 0..2usize {
                        let mut blk = new_block(sx * sy * sz, kind);
                        load_image_block(
                            prof,
                            &img,
                            &mut blk,
                            bsd,
                            bx * sx as usize,
                            0,
                            0,
                            &AstcencSwizzle::RGBA,
                        );
                        n += 1;
                        check(
                            format!("s{si} t{dt} k{kind} p{pi_} b{bx}"),
                            block_hash(bsd, &blk, bx, kind),
                        );
                    }
                }
            }
        }
    }
    for t in 0..96 {
        n += 1;
        check(format!("pack t{t}"), pack_hash(t));
    }
    assert_eq!(n, golden.len());
    if !fails.is_empty() {
        panic!(
            "{} of {n} mismatches:\n{}",
            fails.len(),
            fails[..fails.len().min(30)].join("\n")
        );
    }
}
