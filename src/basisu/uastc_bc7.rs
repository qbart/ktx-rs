//! Port of `transcode_uastc_to_bc7`, `determine_shared/unique_pbits` and
//! `encode_bc7_block` from basisu_transcoder.cpp. Float math keeps the C++
//! evaluation order (f32).
#![allow(clippy::needless_range_loop)]

use super::uastc::{UASTC_MODE_INDEX_SOLID_COLOR, UnpackedUastcBlock, uastc_tables, unpack_uastc};
use super::uastc_tables::*;

/// `bc7_optimization_results`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bc7OptimizationResults {
    pub mode: u32,
    pub partition: u32,
    pub selectors: [u8; 16],
    pub alpha_selectors: [u8; 16],
    pub low: [[u8; 4]; 3],
    pub high: [[u8; 4]; 3],
    pub pbits: [[u32; 2]; 3],
    pub index_selector: u32,
    pub rotation: u32,
}

#[inline]
fn clampi(v: i32, l: i32, h: i32) -> i32 {
    if v < l {
        l
    } else if v > h {
        h
    } else {
        v
    }
}

#[inline]
fn sqf(x: f32) -> f32 {
    x * x
}

/// Quantizes xl/xh with pbit `p` (shared by both pbit searches).
fn quant_p(x: &[f32; 4], scalep: f32, iscalep: i32, p: i32) -> [u8; 4] {
    core::array::from_fn(|c| {
        let q = ((x[c] * scalep - p as f32) / 2.0f32 + 0.5f32) as i32;
        clampi(q * 2 + p, p, iscalep - 1 + p) as u8
    })
}

fn scale_up(x: &[u8; 4], total_bits: u32) -> [u8; 4] {
    x.map(|v| {
        // C shifts the promoted int, so `s >> 8` is 0 rather than an overflow.
        let s = ((v as u32) << (8 - total_bits)) as u8;
        s | ((s as u32) >> total_bits) as u8
    })
}

/// Port of `determine_shared_pbits`.
fn determine_shared_pbits(
    total_comps: usize,
    comp_bits: u32,
    xl: &[f32; 4],
    xh: &[f32; 4],
    best_min: &mut [u8; 4],
    best_max: &mut [u8; 4],
    best_pbits: &mut [u32; 2],
) {
    let total_bits = comp_bits + 1;
    let iscalep = (1i32 << total_bits) - 1;
    let scalep = iscalep as f32;
    let mut best_err = 1e9f32;
    for p in 0..2 {
        let xmin = quant_p(xl, scalep, iscalep, p);
        let xmax = quant_p(xh, scalep, iscalep, p);
        let sl = scale_up(&xmin, total_bits);
        let sh = scale_up(&xmax, total_bits);
        let mut err = 0f32;
        for i in 0..total_comps {
            err += sqf((sl[i] as f32 / 255.0f32) - xl[i]) + sqf((sh[i] as f32 / 255.0f32) - xh[i]);
        }
        if err < best_err {
            best_err = err;
            *best_pbits = [p as u32, p as u32];
            *best_min = xmin.map(|v| v >> 1);
            *best_max = xmax.map(|v| v >> 1);
        }
    }
}

/// Port of `determine_unique_pbits`.
fn determine_unique_pbits(
    total_comps: usize,
    comp_bits: u32,
    xl: &[f32; 4],
    xh: &[f32; 4],
    best_min: &mut [u8; 4],
    best_max: &mut [u8; 4],
    best_pbits: &mut [u32; 2],
) {
    let total_bits = comp_bits + 1;
    let iscalep = (1i32 << total_bits) - 1;
    let scalep = iscalep as f32;
    let (mut best_err0, mut best_err1) = (1e9f32, 1e9f32);
    for p in 0..2 {
        let xmin = quant_p(xl, scalep, iscalep, p);
        let xmax = quant_p(xh, scalep, iscalep, p);
        let sl = scale_up(&xmin, total_bits);
        let sh = scale_up(&xmax, total_bits);
        let (mut err0, mut err1) = (0f32, 0f32);
        for i in 0..total_comps {
            err0 += sqf(sl[i] as f32 - xl[i] * 255.0f32);
            err1 += sqf(sh[i] as f32 - xh[i] * 255.0f32);
        }
        if err0 < best_err0 {
            best_err0 = err0;
            best_pbits[0] = p as u32;
            *best_min = xmin.map(|v| v >> 1);
        }
        if err1 < best_err1 {
            best_err1 = err1;
            best_pbits[1] = p as u32;
            *best_max = xmax.map(|v| v >> 1);
        }
    }
}

/// `bc7_convert_partition_index_3_to_2`.
pub fn bc7_convert_partition_index_3_to_2(mut p: u32, k: u32) -> u32 {
    match k >> 1 {
        0 => p = u32::from(p > 1),
        1 => p = u32::from(p != 0),
        2 => p = u32::from(!(p == 0 || p == 2)),
        _ => {}
    }
    if k & 1 != 0 {
        p = 1 - p;
    }
    p
}

/// Port of `transcode_uastc_to_bc7(const unpacked_uastc_block&, bc7_optimization_results&)`.
pub fn transcode_uastc_to_bc7_unpacked(
    u: &UnpackedUastcBlock,
    dst: &mut Bc7OptimizationResults,
) -> bool {
    *dst = Bc7OptimizationResults::default();
    let tb = uastc_tables();
    let mode = u.mode;
    let endpoint_range = UASTC_MODE_ENDPOINT_RANGES
        .get(mode as usize)
        .copied()
        .unwrap_or(0) as u32;
    let total_comps = UASTC_MODE_COMPS.get(mode as usize).copied().unwrap_or(0) as usize;
    let e = &u.astc.endpoints;
    let w = &u.astc.weights;
    let uq = |i: usize| tb.unquant(endpoint_range, e.get(i).copied().unwrap_or(0));
    let uqf = |i: usize| uq(i) as f32 / 255.0f32;
    let cp = u.common_pattern as usize;

    match mode {
        0 | 5 | 10 | 12 | 14 | 15 | 18 => {
            dst.mode = 6;
            let (mut xl, mut xh) = ([0f32; 4], [0f32; 4]);
            if total_comps == 2 {
                xl[0] = uqf(0);
                xh[0] = uqf(1);
                xl[1] = xl[0];
                xh[1] = xh[0];
                xl[2] = xl[0];
                xh[2] = xh[0];
                xl[3] = uqf(2);
                xh[3] = uqf(3);
            } else {
                xl[0] = uqf(0);
                xl[1] = uqf(2);
                xl[2] = uqf(4);
                xh[0] = uqf(1);
                xh[1] = uqf(3);
                xh[2] = uqf(5);
                if total_comps == 4 {
                    xl[3] = uqf(6);
                    xh[3] = uqf(7);
                } else {
                    xl[3] = 1.0;
                    xh[3] = 1.0;
                }
            }
            let mut pb = [0u32; 2];
            let (mut mn, mut mx) = ([0u8; 4], [0u8; 4]);
            let tc = if total_comps == 2 { 4 } else { total_comps };
            determine_unique_pbits(tc, 7, &xl, &xh, &mut mn, &mut mx, &mut pb);
            dst.low[0] = mn;
            dst.high[0] = mx;
            if total_comps == 3 {
                dst.low[0][3] = 127;
                dst.high[0][3] = 127;
            }
            dst.pbits[0] = pb;
            const BC7_5_TO_4: [u8; 32] = [
                0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 6, 7, 8, 9, 9, 9, 10, 10, 11, 11, 12, 12,
                13, 13, 14, 14, 15, 15,
            ];
            const BC7_2_TO_4: [u8; 4] = [0, 5, 10, 15];
            const BC7_3_TO_4: [u8; 8] = [0, 2, 4, 6, 9, 11, 13, 15];
            for i in 0..16 {
                let wi = w[i] as usize;
                dst.selectors[i] = match mode {
                    18 => BC7_5_TO_4[wi & 31],
                    14 => BC7_2_TO_4[wi & 3],
                    5 | 12 => BC7_3_TO_4[wi & 7],
                    _ => w[i],
                };
            }
        }
        1 => {
            dst.mode = 3;
            let f = |i: usize| e[i] as f32 / 255.0f32;
            let xl = [f(0), f(2), f(4), 1.0];
            let xh = [f(1), f(3), f(5), 1.0];
            let mut pb = [0u32; 2];
            let (mut mn, mut mx) = ([0u8; 4], [0u8; 4]);
            determine_unique_pbits(3, 7, &xl, &xh, &mut mn, &mut mx, &mut pb);
            for s in 0..2 {
                dst.low[s][..3].copy_from_slice(&mn[..3]);
                dst.high[s][..3].copy_from_slice(&mx[..3]);
                dst.pbits[s] = pb;
            }
            dst.selectors.copy_from_slice(&w[..16]);
        }
        2 => {
            dst.mode = 1;
            let Some(&(bc7, _, invert)) = ASTC_BC7_COMMON_PARTITIONS2.get(cp) else {
                return false;
            };
            dst.partition = bc7 as u32;
            let (mut xl, mut xh) = ([0f32, 0.0, 0.0, 1.0], [0f32, 0.0, 0.0, 1.0]);
            for subset in 0..2 {
                for i in 0..3 {
                    let v = e[i * 2 + subset * 6] as u32;
                    xl[i] = ((v << 4) | v) as f32 / 255.0f32;
                    let v = e[i * 2 + subset * 6 + 1] as u32;
                    xh[i] = ((v << 4) | v) as f32 / 255.0f32;
                }
                let mut pb = [0u32; 2];
                let (mut mn, mut mx) = ([0u8; 4], [0u8; 4]);
                determine_shared_pbits(3, 6, &xl, &xh, &mut mn, &mut mx, &mut pb);
                let si = if invert { 1 - subset } else { subset };
                dst.low[si][..3].copy_from_slice(&mn[..3]);
                dst.high[si][..3].copy_from_slice(&mx[..3]);
                dst.pbits[si][0] = pb[0];
            }
            dst.selectors.copy_from_slice(&w[..16]);
        }
        3 => {
            dst.mode = 2;
            let Some(&(bc7, _, perm)) = ASTC_BC7_COMMON_PARTITIONS3.get(cp) else {
                return false;
            };
            dst.partition = bc7 as u32;
            let perm = ASTC_TO_BC7_PARTITION_INDEX_PERM_TABLES[(perm as usize).min(5)];
            for subset in 0..3 {
                for comp in 0..3 {
                    let lo = (uq(comp * 2 + subset * 6) * 31 + 127) / 255;
                    let hi = (uq(comp * 2 + 1 + subset * 6) * 31 + 127) / 255;
                    let si = (perm[subset] as usize).min(2);
                    dst.low[si][comp] = lo as u8;
                    dst.high[si][comp] = hi as u8;
                }
            }
            dst.selectors.copy_from_slice(&w[..16]);
        }
        4 => {
            dst.mode = 3;
            let Some(&(bc7, _, invert)) = ASTC_BC7_COMMON_PARTITIONS2.get(cp) else {
                return false;
            };
            dst.partition = bc7 as u32;
            let (mut xl, mut xh) = ([0f32, 0.0, 0.0, 1.0], [0f32, 0.0, 0.0, 1.0]);
            for subset in 0..2 {
                for i in 0..3 {
                    xl[i] = uqf(i * 2 + subset * 6);
                    xh[i] = uqf(i * 2 + subset * 6 + 1);
                }
                let mut pb = [0u32; 2];
                let (mut mn, mut mx) = ([0u8; 4], [0u8; 4]);
                determine_unique_pbits(3, 7, &xl, &xh, &mut mn, &mut mx, &mut pb);
                let si = if invert { 1 - subset } else { subset };
                dst.low[si][..3].copy_from_slice(&mn[..3]);
                dst.high[si][..3].copy_from_slice(&mx[..3]);
                dst.low[si][3] = 127;
                dst.high[si][3] = 127;
                dst.pbits[si] = pb;
            }
            dst.selectors.copy_from_slice(&w[..16]);
        }
        6 | 11 | 13 | 17 => {
            dst.mode = 5;
            let ccs = u.astc.ccs;
            dst.rotation = ((ccs + 1) & 3) as u32;
            if total_comps == 2 {
                let l0 = ((uq(0) * 127 + 127) / 255) as u8;
                let h0 = ((uq(1) * 127 + 127) / 255) as u8;
                dst.low[0] = [l0, l0, l0, uq(2) as u8];
                dst.high[0] = [h0, h0, h0, uq(3) as u8];
            } else {
                for astc_comp in 0..4usize {
                    let bc7_comp = if astc_comp as i32 == ccs {
                        3
                    } else if astc_comp == 3 {
                        ccs.clamp(0, 3) as usize
                    } else {
                        astc_comp
                    };
                    let (mut l, mut h) = (255u32, 255u32);
                    if astc_comp < total_comps {
                        l = uq(astc_comp * 2);
                        h = uq(astc_comp * 2 + 1);
                    }
                    if bc7_comp < 3 {
                        l = (l * 127 + 127) / 255;
                        h = (h * 127 + 127) / 255;
                    }
                    dst.low[0][bc7_comp] = l as u8;
                    dst.high[0][bc7_comp] = h as u8;
                }
            }
            for i in 0..16 {
                if mode == 13 {
                    dst.selectors[i] = if w[i * 2] != 0 { 3 } else { 0 };
                    dst.alpha_selectors[i] = if w[i * 2 + 1] != 0 { 3 } else { 0 };
                } else {
                    dst.selectors[i] = w[i * 2];
                    dst.alpha_selectors[i] = w[i * 2 + 1];
                }
            }
        }
        7 => {
            dst.mode = 2;
            let Some(&(bc73, _, k)) = BC7_3_ASTC2_COMMON_PARTITIONS.get(cp) else {
                return false;
            };
            dst.partition = bc73 as u32;
            for bc7_part in 0..3usize {
                let astc_part =
                    bc7_convert_partition_index_3_to_2(bc7_part as u32, k as u32) as usize;
                for c in 0..3 {
                    dst.low[bc7_part][c] = ((uq(c * 2 + astc_part * 6) * 31 + 127) / 255) as u8;
                    dst.high[bc7_part][c] =
                        ((uq(c * 2 + 1 + astc_part * 6) * 31 + 127) / 255) as u8;
                }
            }
            dst.selectors.copy_from_slice(&w[..16]);
        }
        UASTC_MODE_INDEX_SOLID_COLOR => {
            let sc = u.solid_color.c;
            let m6 = &tb.bc7_mode_6_optimal_endpoints;
            let err = |p: usize| {
                sc.iter()
                    .map(|&c| m6[c as usize][p].error as u32)
                    .sum::<u32>()
            };
            let (best_err0, best_err1) = (err(0), err(1));
            if best_err0 > 0 && best_err1 > 0 {
                dst.mode = 5;
                for c in 0..3 {
                    let m5 = &tb.bc7_mode_5_optimal_endpoints[sc[c] as usize];
                    dst.low[0][c] = m5.lo;
                    dst.high[0][c] = m5.hi;
                }
                dst.selectors = [1; 16]; // BC7ENC_MODE_5_OPTIMAL_INDEX
                dst.low[0][3] = sc[3];
                dst.high[0][3] = sc[3];
            } else {
                dst.mode = 6;
                let best_p = usize::from(best_err1 < best_err0);
                for c in 0..4 {
                    dst.low[0][c] = m6[sc[c] as usize][best_p].lo;
                    dst.high[0][c] = m6[sc[c] as usize][best_p].hi;
                }
                dst.pbits[0] = [best_p as u32; 2];
                dst.selectors = [5; 16]; // BC7ENC_MODE_6_OPTIMAL_INDEX
            }
        }
        9 | 16 => {
            dst.mode = 7;
            let Some(&(bc7, _, invert)) = ASTC_BC7_COMMON_PARTITIONS2.get(cp) else {
                return false;
            };
            dst.partition = bc7 as u32;
            for astc_subset in 0..2usize {
                let (xl, xh) = if total_comps == 2 {
                    let b = astc_subset * 4;
                    let (l0, h0) = (uqf(b), uqf(b + 1));
                    ([l0, l0, l0, uqf(b + 2)], [h0, h0, h0, uqf(b + 3)])
                } else {
                    let b = astc_subset * 8;
                    (
                        [uqf(b), uqf(b + 2), uqf(b + 4), uqf(b + 6)],
                        [uqf(b + 1), uqf(b + 3), uqf(b + 5), uqf(b + 7)],
                    )
                };
                let mut pb = [0u32; 2];
                let (mut mn, mut mx) = ([0u8; 4], [0u8; 4]);
                determine_unique_pbits(4, 5, &xl, &xh, &mut mn, &mut mx, &mut pb);
                let si = if invert { 1 - astc_subset } else { astc_subset };
                dst.low[si] = mn;
                dst.high[si] = mx;
                dst.pbits[si] = pb;
            }
            dst.selectors.copy_from_slice(&w[..16]);
        }
        _ => return false,
    }
    true
}

/// `bc7_set_block_bits`.
fn bc7_set_block_bits(bytes: &mut [u8; 16], mut val: u32, mut num_bits: u32, cur_ofs: &mut u32) {
    while num_bits != 0 {
        let n = (8 - (*cur_ofs & 7)).min(num_bits);
        if let Some(b) = bytes.get_mut((*cur_ofs >> 3) as usize) {
            *b |= (val << (*cur_ofs & 7)) as u8;
        }
        val = val.checked_shr(n).unwrap_or(0);
        num_bits -= n;
        *cur_ofs += n;
    }
}

#[inline]
fn color_index_size(mode: usize, isel: u32) -> u32 {
    BC7_COLOR_INDEX_BITCOUNT[mode] as u32 + isel
}

#[inline]
fn alpha_index_size(mode: usize, isel: u32) -> u32 {
    (BC7_ALPHA_INDEX_BITCOUNT[mode] as u32).wrapping_sub(isel)
}

/// Port of `encode_bc7_block`.
pub fn encode_bc7_block(out: &mut [u8; 16], r: &Bc7OptimizationResults) {
    let mode = (r.mode & 7) as usize;
    let total_subsets = BC7_NUM_SUBSETS[mode] as usize;
    let total_partitions = 1u32 << BC7_PARTITION_BITS[mode];
    let part = (r.partition & 63) as usize;
    let partition: &[u8] = match total_subsets {
        1 => &[0u8; 16],
        2 => &BC7_PARTITION2[part * 16..part * 16 + 16],
        _ => &BC7_PARTITION3[part * 16..part * 16 + 16],
    };
    let mut color_selectors = r.selectors;
    let mut alpha_selectors = r.alpha_selectors;
    let mut low = r.low;
    let mut high = r.high;
    let mut pbits = r.pbits;
    let mut anchor = [-1i32; 3];
    let separate_alpha = mode == 4 || mode == 5;

    for k in 0..total_subsets {
        let anchor_index = if k == 0 {
            0
        } else if total_subsets == 3 && k == 1 {
            BC7_TABLE_ANCHOR_INDEX_THIRD_SUBSET_1[part] as usize
        } else if total_subsets == 3 && k == 2 {
            BC7_TABLE_ANCHOR_INDEX_THIRD_SUBSET_2[part] as usize
        } else {
            BC7_TABLE_ANCHOR_INDEX_SECOND_SUBSET[part] as usize
        };
        anchor[k] = anchor_index as i32;

        let num_color_indices = 1u32 << color_index_size(mode, r.index_selector);
        if color_selectors[anchor_index] as u32 & (num_color_indices >> 1) != 0 {
            for i in 0..16 {
                if partition[i] as usize == k {
                    color_selectors[i] =
                        ((num_color_indices - 1) - color_selectors[i] as u32) as u8;
                }
            }
            if separate_alpha {
                for q in 0..3 {
                    core::mem::swap(&mut low[k][q], &mut high[k][q]);
                }
            } else {
                core::mem::swap(&mut low[k], &mut high[k]);
            }
            if BC7_MODE_HAS_SHARED_P_BITS[mode] == 0 {
                pbits[k].swap(0, 1);
            }
        }

        if separate_alpha {
            let num_alpha_indices = 1u32 << alpha_index_size(mode, r.index_selector);
            if alpha_selectors[anchor_index] as u32 & (num_alpha_indices >> 1) != 0 {
                for i in 0..16 {
                    if partition[i] as usize == k {
                        alpha_selectors[i] =
                            ((num_alpha_indices - 1) - alpha_selectors[i] as u32) as u8;
                    }
                }
                core::mem::swap(&mut low[k][3], &mut high[k][3]);
            }
        }
    }

    *out = [0; 16];
    let mut ofs = 0u32;
    bc7_set_block_bits(out, 1 << mode, mode as u32 + 1, &mut ofs);
    if mode == 4 || mode == 5 {
        bc7_set_block_bits(out, r.rotation, 2, &mut ofs);
    }
    if mode == 4 {
        bc7_set_block_bits(out, r.index_selector, 1, &mut ofs);
    }
    if total_partitions > 1 {
        bc7_set_block_bits(
            out,
            r.partition,
            if total_partitions == 64 { 6 } else { 4 },
            &mut ofs,
        );
    }
    let total_comps = if mode >= 4 { 4 } else { 3 };
    for comp in 0..total_comps {
        let bits = if comp == 3 {
            BC7_ALPHA_PRECISION_TABLE[mode] as u32
        } else {
            BC7_COLOR_PRECISION_TABLE[mode] as u32
        };
        for subset in 0..total_subsets {
            bc7_set_block_bits(out, low[subset][comp] as u32, bits, &mut ofs);
            bc7_set_block_bits(out, high[subset][comp] as u32, bits, &mut ofs);
        }
    }
    if BC7_MODE_HAS_P_BITS[mode] != 0 {
        for p in pbits.iter().take(total_subsets) {
            bc7_set_block_bits(out, p[0], 1, &mut ofs);
            if BC7_MODE_HAS_SHARED_P_BITS[mode] == 0 {
                bc7_set_block_bits(out, p[1], 1, &mut ofs);
            }
        }
    }
    let is_anchor = |idx: usize| anchor.contains(&(idx as i32));
    for idx in 0..16 {
        let mut n = if r.index_selector != 0 {
            alpha_index_size(mode, r.index_selector)
        } else {
            color_index_size(mode, r.index_selector)
        };
        if is_anchor(idx) {
            n -= 1;
        }
        let v = if r.index_selector != 0 {
            alpha_selectors[idx]
        } else {
            color_selectors[idx]
        };
        bc7_set_block_bits(out, v as u32, n, &mut ofs);
    }
    if separate_alpha {
        for idx in 0..16 {
            let mut n = if r.index_selector != 0 {
                color_index_size(mode, r.index_selector)
            } else {
                alpha_index_size(mode, r.index_selector)
            };
            if is_anchor(idx) {
                n -= 1;
            }
            let v = if r.index_selector != 0 {
                color_selectors[idx]
            } else {
                alpha_selectors[idx]
            };
            bc7_set_block_bits(out, v as u32, n, &mut ofs);
        }
    }
}

/// Port of `transcode_uastc_to_bc7(const uastc_block&, void*)`.
pub fn transcode_uastc_to_bc7(src: &[u8; 16], dst: &mut [u8; 16]) -> bool {
    let mut u = UnpackedUastcBlock::default();
    if !unpack_uastc(src, &mut u, false, false) {
        return false;
    }
    let mut r = Bc7OptimizationResults::default();
    if !transcode_uastc_to_bc7_unpacked(&u, &mut r) {
        return false;
    }
    encode_bc7_block(dst, &r);
    true
}
