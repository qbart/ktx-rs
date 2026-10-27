// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Part of the Rust port of KTX-Software; see NOTICE.md.
//! astcenc ideal endpoints / weights kernels vs. golden output of the C++ original
//! (`scripts/golden/gen_astcenc_weights_golden.sh`).
#![allow(clippy::panic, clippy::needless_range_loop)]

mod common;

use std::collections::BTreeMap;

use common::astcenc_gen::*;
use ktx::astcenc::ideal_endpoints_and_weights::*;
use ktx::astcenc::image::load_image_block;
use ktx::astcenc::internal::*;
use ktx::astcenc::vecmath::Vfloat4;
use ktx::astcenc::*;

fn hash_ei(h: &mut H, ei: &EndpointsAndWeights, texel_count: usize) {
    h.u(ei.is_constant_weight_error_scale as u32);
    h.u(ei.ep.partition_count);
    for q in 0..ei.ep.partition_count as usize {
        h.v4(ei.ep.endpt0[q]);
        h.v4(ei.ep.endpt1[q]);
    }
    for i in 0..texel_count {
        h.f(ei.weights[i]);
        h.f(ei.weight_error_scale[i]);
    }
}

#[test]
fn weights_match_astcenc() {
    let golden: BTreeMap<String, String> = include_str!("resources/golden/astcenc_weights.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let profiles = [AstcencProfile::Ldr, AstcencProfile::Hdr];
    let mut fails = Vec::new();
    let mut n = 0;
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
                        let mut h = H::new();
                        let key = format!("s{si} t{dt} k{kind} p{pi_} b{bx}");
                        let tc = blk.texel_count as usize;
                        let ndm = bsd.decimation_mode_count_selected as usize;
                        for pc in 1..=4u32 {
                            let np = bsd.partitioning_count_selected[pc as usize - 1];
                            for k in 0..2u32.min(np) {
                                let pi = bsd.get_raw_partition_info(pc, (k * 53 + bx as u32) % np);
                                let mut ei = EndpointsAndWeights::default();
                                compute_ideal_colors_and_weights_1plane(&blk, pi, &mut ei);
                                hash_ei(&mut h, &ei, tc);
                                let mut d = 0;
                                while d < ndm {
                                    let di = bsd.get_decimation_info(d);
                                    let mut ideal = [0.0f32; 72];
                                    compute_ideal_weights_for_decimation(&ei, di, &mut ideal);
                                    for w in 0..di.weight_count as usize {
                                        h.f(ideal[w]);
                                    }
                                    let mut q = 0;
                                    while q < 12 {
                                        let mut uval = [0.0f32; 72];
                                        let mut uq = [0u8; 72];
                                        let (lo, hi) =
                                            if q % 2 == 1 { (0.1, 0.85) } else { (0.0, 1.0) };
                                        compute_quantized_weights_for_decimation(
                                            di, lo, hi, &ideal, &mut uval, &mut uq, q,
                                        );
                                        for w in 0..di.weight_count as usize {
                                            h.f(uval[w]);
                                            h.bytes(&[uq[w]]);
                                        }
                                        h.f(compute_error_of_weight_set_1plane(&ei, di, &uval));
                                        let mut ep = ei.ep;
                                        let mut rgbs = [Vfloat4::zero(); 4];
                                        let mut rgbo = [Vfloat4::zero(); 4];
                                        recompute_ideal_colors_1plane(
                                            &blk, pi, di, &uq, &mut ep, &mut rgbs, &mut rgbo,
                                        );
                                        for z in 0..pc as usize {
                                            h.v4(ep.endpt0[z]);
                                            h.v4(ep.endpt1[z]);
                                            h.v4(rgbs[z]);
                                            h.v4(rgbo[z]);
                                        }
                                        q += if pc == 1 { 1 } else { 5 };
                                    }
                                    d += if d < 5 { 1 } else { 7 };
                                }
                            }
                        }
                        for p2 in 0..4u32 {
                            if p2 == 3 && blk.is_constant_channel(3) {
                                continue;
                            }
                            let mut ei1 = EndpointsAndWeights::default();
                            let mut ei2 = EndpointsAndWeights::default();
                            compute_ideal_colors_and_weights_2planes(
                                bsd, &blk, p2, &mut ei1, &mut ei2,
                            );
                            hash_ei(&mut h, &ei1, tc);
                            hash_ei(&mut h, &ei2, tc);
                            for d in 0..ndm {
                                let di = bsd.get_decimation_info(d);
                                if di.weight_count > 32 || d % 3 != 0 {
                                    continue;
                                }
                                let mut id1 = [0.0f32; 72];
                                let mut id2 = [0.0f32; 72];
                                compute_ideal_weights_for_decimation(&ei1, di, &mut id1);
                                compute_ideal_weights_for_decimation(&ei2, di, &mut id2);
                                for q in (0..12).step_by(4) {
                                    let mut u1 = [0.0f32; 72];
                                    let mut u2 = [0.0f32; 72];
                                    let mut q1 = [0u8; 72];
                                    let mut q2 = [0u8; 72];
                                    compute_quantized_weights_for_decimation(
                                        di, 0.0, 1.0, &id1, &mut u1, &mut q1, q,
                                    );
                                    compute_quantized_weights_for_decimation(
                                        di, 0.0, 1.0, &id2, &mut u2, &mut q2, q,
                                    );
                                    h.f(compute_error_of_weight_set_2planes(
                                        &ei1, &ei2, di, &u1, &u2,
                                    ));
                                    let mut ep = ei1.ep;
                                    let mut rgbs = Vfloat4::zero();
                                    let mut rgbo = Vfloat4::zero();
                                    recompute_ideal_colors_2planes(
                                        &blk, bsd, di, &q1, &q2, &mut ep, &mut rgbs, &mut rgbo,
                                        p2 as i32,
                                    );
                                    h.v4(ep.endpt0[0]);
                                    h.v4(ep.endpt1[0]);
                                    h.v4(rgbs);
                                    h.v4(rgbo);
                                }
                            }
                        }
                        n += 1;
                        let got = format!("{:016x}", h.0);
                        match golden.get(&key) {
                            Some(w) if *w == got => {}
                            Some(w) => fails.push(format!("{key}: want {w} got {got}")),
                            None => fails.push(format!("{key}: missing")),
                        }
                    }
                }
            }
        }
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
