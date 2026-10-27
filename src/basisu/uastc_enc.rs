// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of `encoder/basisu_uastc_enc.h` / `basisu_uastc_enc.cpp`: the UASTC LDR 4x4 block
//! encoder (`encode_uastc`, `pack_uastc`) with its BC1, ETC1 and ETC2 EAC A8 hint search.

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use super::bc7enc::*;
use super::dxt::Dxt1Block;
use super::enc::{ColorRgba, color_distance, color_distance_linear};
use super::etc::Color32;
use super::etc_enc::{
    EacA8Block, EtcBlock, G_ETC1_PIXEL_COORDS, G_ETC2_EAC_TABLES, pack_etc1_block_solid_color,
};
use super::gpu_texture::{unpack_bc1, unpack_bc7};
use super::uastc::{
    AstcBlockDesc, UASTC_MODE_INDEX_SOLID_COLOR, UnpackedUastcBlock, astc_interpolate,
    get_anchor_indices, uastc_tables, unpack_uastc, unpack_uastc_pixels_desc,
};
use super::uastc_bc1::{encode_bc1, transcode_uastc_to_bc1_hint0, transcode_uastc_to_bc1_hint1};
use super::uastc_bc7::{
    Bc7OptimizationResults, bc7_convert_partition_index_3_to_2, encode_bc7_block,
    transcode_uastc_to_bc7_unpacked,
};
use super::uastc_etc::apply_etc1_bias;
use super::uastc_tables::*;

pub const TOTAL_PACK_UASTC_LEVELS: u32 = 5;

pub const C_PACK_UASTC_LEVEL_FASTEST: u32 = 0;
pub const C_PACK_UASTC_LEVEL_FASTER: u32 = 1;
pub const C_PACK_UASTC_LEVEL_DEFAULT: u32 = 2;
pub const C_PACK_UASTC_LEVEL_SLOWER: u32 = 3;
pub const C_PACK_UASTC_LEVEL_VERY_SLOW: u32 = 4;
pub const C_PACK_UASTC_LEVEL_MASK: u32 = 0xF;
pub const C_PACK_UASTC_FAVOR_UASTC_ERROR: u32 = 8;
pub const C_PACK_UASTC_FAVOR_BC7_ERROR: u32 = 16;
pub const C_PACK_UASTC_ETC1_FASTER_HINTS: u32 = 64;
pub const C_PACK_UASTC_ETC1_FASTEST_HINTS: u32 = 128;
pub const C_PACK_UASTC_ETC1_DISABLE_FLIP_AND_INDIVIDUAL: u32 = 256;
pub const C_PACK_UASTC_FAVOR_SIMPLER_MODES: u32 = 512;

const MAX_ENCODE_RESULTS: usize = 512;

const ETC2_EAC_MIN_VALUE_SELECTOR: usize = 3;
const ETC2_EAC_MAX_VALUE_SELECTOR: usize = 7;

/// `uastc_encode_results`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UastcEncodeResults {
    pub uastc_mode: u32,
    pub common_pattern: u32,
    pub astc: AstcBlockDesc,
    pub solid_color: ColorRgba,
    pub astc_err: u64,
}

#[inline]
fn mt(t: &[u8; 19], mode: u32) -> u32 {
    t.get(mode as usize).copied().unwrap_or(0) as u32
}

#[inline]
fn uastc_write_bits(buf: &mut [u8; 32], bit_offset: &mut u32, mut code: u64, mut codesize: u32) {
    while codesize != 0 {
        let byte_bit_offset = *bit_offset & 7;
        let bits_to_write = codesize.min(8 - byte_bit_offset);
        buf[(*bit_offset >> 3) as usize & 31] |= (code << byte_bit_offset) as u8;
        code >>= bits_to_write;
        codesize -= bits_to_write;
        *bit_offset += bits_to_write;
    }
}

/// `pack_uastc`.
pub fn pack_uastc(
    blk: &mut [u8; 16],
    result: &UastcEncodeResults,
    etc1_blk: &EtcBlock,
    etc1_bias: u32,
    etc_eac_a8_blk: &EacA8Block,
    bc1_hint0: bool,
    bc1_hint1: bool,
) {
    let mode = result.uastc_mode;
    let mut buf = [0u8; 32];
    let mut bo = 0u32;
    let huff = UASTC_MODE_HUFF_CODES[(mode as usize).min(19)];
    uastc_write_bits(&mut buf, &mut bo, huff[0] as u64, huff[1]);

    if mode == UASTC_MODE_INDEX_SOLID_COLOR {
        for c in 0..4 {
            uastc_write_bits(&mut buf, &mut bo, result.solid_color[c] as u64, 8);
        }
        uastc_write_bits(&mut buf, &mut bo, etc1_blk.get_diff_bit() as u64, 1);
        uastc_write_bits(&mut buf, &mut bo, etc1_blk.get_inten_table(0) as u64, 3);
        uastc_write_bits(&mut buf, &mut bo, etc1_blk.get_selector(0, 0) as u64, 2);
        let c = if etc1_blk.get_diff_bit() {
            EtcBlock::unpack_color5(etc1_blk.get_base5_color(), false, 255)
        } else {
            EtcBlock::unpack_color4(etc1_blk.get_base4_color(0), false, 255)
        };
        for i in 0..3 {
            uastc_write_bits(&mut buf, &mut bo, c[i] as u64, 5);
        }
        blk.copy_from_slice(&buf[..16]);
        return;
    }

    if mt(&UASTC_MODE_HAS_BC1_HINT0, mode) != 0 {
        uastc_write_bits(&mut buf, &mut bo, bc1_hint0 as u64, 1);
    }
    if mt(&UASTC_MODE_HAS_BC1_HINT1, mode) != 0 {
        uastc_write_bits(&mut buf, &mut bo, bc1_hint1 as u64, 1);
    }
    uastc_write_bits(&mut buf, &mut bo, etc1_blk.get_flip_bit() as u64, 1);
    uastc_write_bits(&mut buf, &mut bo, etc1_blk.get_diff_bit() as u64, 1);
    uastc_write_bits(&mut buf, &mut bo, etc1_blk.get_inten_table(0) as u64, 3);
    uastc_write_bits(&mut buf, &mut bo, etc1_blk.get_inten_table(1) as u64, 3);
    if mt(&UASTC_MODE_HAS_ETC1_BIAS, mode) != 0 {
        uastc_write_bits(&mut buf, &mut bo, etc1_bias as u64, 5);
    }
    if mt(&UASTC_MODE_HAS_ALPHA, mode) != 0 {
        let etc2_hints = etc_eac_a8_blk.table as u32 | ((etc_eac_a8_blk.multiplier as u32) << 4);
        uastc_write_bits(&mut buf, &mut bo, etc2_hints as u64, 8);
    }

    let mut subsets = 1u32;
    match mode {
        2 | 4 | 7 | 9 | 16 => {
            uastc_write_bits(&mut buf, &mut bo, result.common_pattern as u64, 5);
            subsets = 2;
        }
        3 => {
            uastc_write_bits(&mut buf, &mut bo, result.common_pattern as u64, 4);
            subsets = 3;
        }
        _ => {}
    }

    let mut total_planes = 1usize;
    match mode {
        6 | 11 | 13 => {
            uastc_write_bits(&mut buf, &mut bo, result.astc.ccs as u64, 2);
            total_planes = 2;
        }
        17 => total_planes = 2,
        _ => {}
    }

    let mut weights = [0u8; 32];
    weights[..16 * total_planes].copy_from_slice(&result.astc.weights[..16 * total_planes]);
    let mut endpoints = result.astc.endpoints;
    let total_comps = mt(&UASTC_MODE_COMPS, mode) as usize;
    let weight_bits = mt(&UASTC_MODE_WEIGHT_BITS, mode);
    let (anchors, pattern) = get_anchor_indices(subsets, mode, result.common_pattern);

    for plane_index in 0..total_planes {
        for subset_index in 0..subsets as usize {
            let anchor_index = anchors[subset_index] as usize;
            // Check anchor weight's MSB - if it's set then invert this subset's weights and
            // swap the endpoints
            if weights[anchor_index * total_planes + plane_index] & (1 << (weight_bits - 1)) != 0 {
                for i in 0..16 {
                    if pattern[i] as usize == subset_index {
                        let w = &mut weights[i * total_planes + plane_index];
                        *w = (((1u32 << weight_bits) - 1) - *w as u32) as u8;
                    }
                }
                if total_planes == 2 {
                    for c in 0..total_comps {
                        let comp_plane = if total_comps == 2 {
                            c
                        } else if c as i32 == result.astc.ccs {
                            1
                        } else {
                            0
                        };
                        if comp_plane == plane_index && c * 2 + 1 < endpoints.len() {
                            endpoints.swap(c * 2, c * 2 + 1);
                        }
                    }
                } else {
                    for c in 0..total_comps {
                        let b = subset_index * total_comps * 2 + c * 2;
                        endpoints.swap(b, b + 1);
                    }
                }
            }
        }
    }

    let total_values = total_comps * 2 * subsets as usize;
    let endpoint_range = mt(&UASTC_MODE_ENDPOINT_RANGES, mode) as usize;
    let [ep_bits, ep_trits, ep_quints] = ASTC_BISE_RANGE_TABLE[endpoint_range.min(20)];

    let mut bit_values = [0u32; 18];
    let mut tq_values = [0u32; 8];
    let mut total_tq_values = 0usize;
    let mut tq_accum = 0u32;
    let mut tq_mul = 1u32;
    for i in 0..total_values {
        let val = endpoints[i] as u32;
        bit_values[i] = val & ((1 << ep_bits) - 1);
        let tq = val >> ep_bits;
        if ep_trits != 0 {
            tq_accum += tq * tq_mul;
            tq_mul *= 3;
            if tq_mul == 243 {
                tq_values[total_tq_values] = tq_accum;
                total_tq_values += 1;
                tq_accum = 0;
                tq_mul = 1;
            }
        } else if ep_quints != 0 {
            tq_accum += tq * tq_mul;
            tq_mul *= 5;
            if tq_mul == 125 {
                tq_values[total_tq_values] = tq_accum;
                total_tq_values += 1;
                tq_accum = 0;
                tq_mul = 1;
            }
        }
    }
    for i in 0..total_tq_values {
        let num_bits = if ep_trits != 0 { 8 } else { 7 };
        uastc_write_bits(&mut buf, &mut bo, tq_values[i] as u64, num_bits);
    }
    if tq_mul > 1 {
        let num_bits = if ep_trits != 0 {
            match tq_mul {
                3 => 2,
                9 => 4,
                27 => 5,
                _ => 7,
            }
        } else if tq_mul == 5 {
            3
        } else {
            5
        };
        uastc_write_bits(&mut buf, &mut bo, tq_accum as u64, num_bits);
    }
    for i in 0..total_values {
        uastc_write_bits(&mut buf, &mut bo, bit_values[i] as u64, ep_bits);
    }

    let plane_shift = if total_planes == 2 { 1 } else { 0 };
    for i in 0..16 * total_planes {
        let mut numbits = weight_bits;
        for s in 0..subsets as usize {
            if anchors[s] as usize == (i >> plane_shift) {
                numbits -= 1;
                break;
            }
        }
        uastc_write_bits(&mut buf, &mut bo, weights[i] as u64, numbits);
    }
    blk.copy_from_slice(&buf[..16]);
}

type Block = [ColorRgba; 16];

/// `color_cell_compression(255, ...)` with the `memset(0)` params / results the mode
/// encoders use and unit channel weights.
fn ccell(
    pixels: &[ColorRgba],
    num_pixels: u32,
    weights: &[u32],
    weightsx: &[Bc7encVec4F],
    range: u32,
    has_alpha: bool,
    comp: &Bc7encCompressBlockParams,
) -> (u64, ColorCellCompressorResults) {
    let p = ColorCellCompressorParams {
        num_pixels,
        pixels,
        num_selector_weights: weights.len() as u32,
        selector_weights: weights,
        selector_weightsx: weightsx,
        astc_endpoint_range: range,
        weights: [1, 1, 1, 1],
        has_alpha,
        ..Default::default()
    };
    let mut r = ColorCellCompressorResults::default();
    let err = color_cell_compression(255, &p, &mut r, comp);
    (err, r)
}

#[inline]
fn uq(range: u32, v: u8) -> i32 {
    uastc_tables().unquant(range, v) as i32
}

/// Sets `ep[0..2*ncomps]` from the ASTC endpoints of `r` (low/high interleaved per channel).
fn set_endpoints(ep: &mut [u8], r: &ColorCellCompressorResults, ncomps: usize) {
    for c in 0..ncomps {
        ep[c * 2] = r.astc_low_endpoint[c];
        ep[c * 2 + 1] = r.astc_high_endpoint[c];
    }
}

/// Swaps the `npairs` endpoint pairs when the high RGB endpoint is darker; returns true if so.
fn invert_if_needed(ep: &mut [u8], range: u32, npairs: usize) -> bool {
    let s0 = uq(range, ep[0]) + uq(range, ep[2]) + uq(range, ep[4]);
    let s1 = uq(range, ep[1]) + uq(range, ep[3]) + uq(range, ep[5]);
    if s1 < s0 {
        for k in 0..npairs {
            ep.swap(k * 2, k * 2 + 1);
        }
        true
    } else {
        false
    }
}

fn push(
    results: &mut Vec<UastcEncodeResults>,
    mode: u32,
    common_pattern: u32,
    astc: AstcBlockDesc,
    err: u64,
) {
    if results.len() < MAX_ENCODE_RESULTS {
        results.push(UastcEncodeResults {
            uastc_mode: mode,
            common_pattern,
            astc,
            solid_color: [0; 4],
            astc_err: err,
        });
    }
}

fn desc(
    weight_range: i32,
    subsets: i32,
    seed: i32,
    cem: i32,
    ccs: i32,
    dual: bool,
) -> AstcBlockDesc {
    AstcBlockDesc {
        weight_range,
        subsets,
        partition_seed: seed,
        cem,
        ccs,
        dual_plane: dual,
        ..Default::default()
    }
}

/// Single subset modes 0, 1, 5, 10, 12, 14, 18: one color cell over the whole block.
fn astc_mode_1subset(
    mode: u32,
    block: &Block,
    results: &mut Vec<UastcEncodeResults>,
    comp: &Bc7encCompressBlockParams,
    force_selectors: Option<&[u8]>,
) {
    let (range, weight_range, w, wx, alpha): (u32, i32, &[u32], &[Bc7encVec4F], bool) = match mode {
        0 => (19, 8, &ASTC_WEIGHTS4, &ASTC_WEIGHTS4X, false),
        18 => (11, 11, &ASTC_WEIGHTS5, &ASTC_WEIGHTS5X, false),
        1 => (20, 2, &BC7_WEIGHTS2, &BC7_WEIGHTS2X, false),
        5 => (20, 5, &BC7_WEIGHTS3, &BC7_WEIGHTS3X, false),
        10 => (13, 8, &ASTC_WEIGHTS4, &ASTC_WEIGHTS4X, true),
        12 => (19, 5, &BC7_WEIGHTS3, &BC7_WEIGHTS3X, true),
        _ => (20, 2, &BC7_WEIGHTS2, &BC7_WEIGHTS2X, true),
    };
    let p = ColorCellCompressorParams {
        num_pixels: 16,
        pixels: block,
        num_selector_weights: w.len() as u32,
        selector_weights: w,
        selector_weightsx: wx,
        astc_endpoint_range: range,
        weights: [1, 1, 1, 1],
        has_alpha: alpha,
        force_selectors,
        ..Default::default()
    };
    let mut r = ColorCellCompressorResults::default();
    let part_err = color_cell_compression(255, &p, &mut r, comp);

    let mut a = desc(weight_range, 1, 0, if alpha { 12 } else { 8 }, 0, false);
    let nc = if alpha { 4 } else { 3 };
    set_endpoints(&mut a.endpoints, &r, nc);
    let invert = if force_selectors.is_none() || mode != 0 && mode != 18 {
        invert_if_needed(&mut a.endpoints, range, nc)
    } else {
        false
    };
    let max_w = w.len() as u8 - 1;
    for i in 0..16 {
        a.weights[i] = r.selectors[i];
        if invert {
            a.weights[i] = max_w - a.weights[i];
        }
    }
    push(results, mode, 0, a, part_err);
}

fn estimate_partition2(
    num_weights: u32,
    num_comps: u32,
    w: &[u32],
    block: &Block,
    weights: &[u32; 4],
) -> u32 {
    let mut best_err = u64::MAX;
    let mut best_common_pattern = 0;
    for common_pattern in 0..ASTC_BC7_COMMON_PARTITIONS2.len() {
        let bc7_pattern = ASTC_BC7_COMMON_PARTITIONS2[common_pattern].0 as usize;
        let part = &BC7_PARTITION2[bc7_pattern * 16..bc7_pattern * 16 + 16];
        let (colors, counts) = split::<2>(block, part);
        let mut total_subset_err = 0u64;
        let mut subset = 0;
        while subset < 2 && total_subset_err < best_err {
            total_subset_err += color_cell_compression_est_astc(
                num_weights,
                num_comps,
                w,
                counts[subset],
                &colors[subset],
                best_err,
                weights,
            );
            subset += 1;
        }
        if total_subset_err < best_err {
            best_err = total_subset_err;
            best_common_pattern = common_pattern as u32;
        }
    }
    best_common_pattern
}

fn estimate_partition2_list(
    num_weights: u32,
    num_comps: u32,
    w: &[u32],
    block: &Block,
    parts: &mut [u32; 8],
    max_parts: usize,
    weights: &[u32; 4],
) {
    let mut part_error = [u64::MAX; 8];
    parts[..max_parts].fill(0);
    for common_pattern in 0..ASTC_BC7_COMMON_PARTITIONS2.len() {
        let bc7_pattern = ASTC_BC7_COMMON_PARTITIONS2[common_pattern].0 as usize;
        let part = &BC7_PARTITION2[bc7_pattern * 16..bc7_pattern * 16 + 16];
        let (colors, counts) = split::<2>(block, part);
        let mut total_subset_err = 0u64;
        for subset in 0..2 {
            total_subset_err += color_cell_compression_est_astc(
                num_weights,
                num_comps,
                w,
                counts[subset],
                &colors[subset],
                u64::MAX,
                weights,
            );
        }
        for i in 0..max_parts {
            if total_subset_err < part_error[i] {
                for j in (i + 1..max_parts).rev() {
                    parts[j] = parts[j - 1];
                    part_error[j] = part_error[j - 1];
                }
                parts[i] = common_pattern as u32;
                part_error[i] = total_subset_err;
                break;
            }
        }
    }
}

/// Splits the block's pixels by subset (`part[i]` per pixel), keeping pixel order.
fn split<const N: usize>(block: &Block, part: &[u8]) -> ([[ColorRgba; 16]; N], [u32; N]) {
    let mut colors = [[[0u8; 4]; 16]; N];
    let mut counts = [0u32; N];
    for i in 0..16 {
        let s = part[i] as usize;
        colors[s][counts[s] as usize] = block[i];
        counts[s] += 1;
    }
    (colors, counts)
}

/// Like `split` but also returns each pixel's index within its subset.
fn split_indexed<const N: usize>(
    block: &Block,
    part: impl Fn(usize) -> usize,
) -> ([[ColorRgba; 16]; N], [u32; N], [u32; 16]) {
    let mut colors = [[[0u8; 4]; 16]; N];
    let mut counts = [0u32; N];
    let mut index = [0u32; 16];
    for i in 0..16 {
        let s = part(i);
        index[i] = counts[s];
        colors[s][counts[s] as usize] = block[i];
        counts[s] += 1;
    }
    (colors, counts, index)
}

/// Modes 2 and 4: 2 subsets of the common BC7/ASTC partitions, RGB.
fn astc_mode2_or_4(
    mode: u32,
    block: &Block,
    results: &mut Vec<UastcEncodeResults>,
    comp: &Bc7encCompressBlockParams,
    estimate_partition: bool,
) {
    let (range, weight_range, w, wx): (u32, i32, &[u32], &[Bc7encVec4F]) = if mode == 2 {
        (8, 5, &BC7_WEIGHTS3, &BC7_WEIGHTS3X)
    } else {
        (12, 2, &BC7_WEIGHTS2, &BC7_WEIGHTS2X)
    };
    let mut first = 0u32;
    let mut last = ASTC_BC7_COMMON_PARTITIONS2.len() as u32;
    if estimate_partition {
        first = estimate_partition2(w.len() as u32, 3, w, block, &[1, 1, 1, 1]);
        last = first + 1;
    }
    for common_pattern in first..last {
        let (bc7_pattern, seed, pinvert) = ASTC_BC7_COMMON_PARTITIONS2[common_pattern as usize];
        let bp = bc7_pattern as usize * 16;
        let (pixels, counts, index) =
            split_indexed::<2>(block, |i| BC7_PARTITION2[bp + i] as usize);
        let mut total_part_err = 0u64;
        let mut res = [ColorCellCompressorResults::default(); 2];
        for part in 0..2 {
            let (err, r) = ccell(&pixels[part], counts[part], w, wx, range, false, comp);
            total_part_err += err;
            res[part] = r;
        }
        let mut a = desc(weight_range, 2, seed as i32, 8, 0, false);
        let (p0, p1) = if pinvert { (1, 0) } else { (0, 1) };
        let mut invert = [false; 2];
        set_endpoints(&mut a.endpoints[0..6], &res[p0], 3);
        invert[0] = invert_if_needed(&mut a.endpoints[0..6], range, 3);
        set_endpoints(&mut a.endpoints[6..12], &res[p1], 3);
        invert[1] = invert_if_needed(&mut a.endpoints[6..12], range, 3);
        let max_w = w.len() as u8 - 1;
        for i in 0..16 {
            let bc7_part = BC7_PARTITION2[bp + i] as usize;
            a.weights[i] = res[bc7_part].selectors[index[i] as usize];
            let astc_part = if pinvert { 1 - bc7_part } else { bc7_part };
            if invert[astc_part] {
                a.weights[i] = max_w - a.weights[i];
            }
        }
        push(results, mode, common_pattern, a, total_part_err);
    }
}

/// Mode 3: 3 subsets, 2-bit indices, [0,11] endpoints, BC7 mode 2.
fn astc_mode3(
    block: &Block,
    results: &mut Vec<UastcEncodeResults>,
    comp: &Bc7encCompressBlockParams,
    estimate_partition: bool,
) {
    let mut first = 0u32;
    let mut last = ASTC_BC7_COMMON_PARTITIONS3.len() as u32;
    if estimate_partition {
        let mut best_err = u64::MAX;
        let mut best_common_pattern = 0u32;
        for common_pattern in 0..ASTC_BC7_COMMON_PARTITIONS3.len() {
            let bp = ASTC_BC7_COMMON_PARTITIONS3[common_pattern].0 as usize * 16;
            let (colors, counts) = split::<3>(block, &BC7_PARTITION3[bp..bp + 16]);
            let mut total_subset_err = 0u64;
            let mut subset = 0;
            while subset < 3 && total_subset_err < best_err {
                total_subset_err += color_cell_compression_est_astc(
                    4,
                    3,
                    &BC7_WEIGHTS2,
                    counts[subset],
                    &colors[subset],
                    best_err,
                    &[1, 1, 1, 1],
                );
                subset += 1;
            }
            if total_subset_err < best_err {
                best_err = total_subset_err;
                best_common_pattern = common_pattern as u32;
            }
        }
        first = best_common_pattern;
        last = best_common_pattern + 1;
    }
    let range = 7;
    for common_pattern in first..last {
        let (bc7_pattern, seed, perm) = ASTC_BC7_COMMON_PARTITIONS3[common_pattern as usize];
        let bp = bc7_pattern as usize * 16;
        let (pixels, counts, index) =
            split_indexed::<3>(block, |i| BC7_PARTITION3[bp + i] as usize);
        let mut total_part_err = 0u64;
        let mut res = [ColorCellCompressorResults::default(); 3];
        for part in 0..3 {
            let (err, r) = ccell(
                &pixels[part],
                counts[part],
                &BC7_WEIGHTS2,
                &BC7_WEIGHTS2X,
                range,
                false,
                comp,
            );
            total_part_err += err;
            res[part] = r;
        }
        let mut a = desc(2, 3, seed as i32, 8, 0, false);
        let astc_to_bc7_part = ASTC_TO_BC7_PARTITION_INDEX_PERM_TABLES[perm as usize];
        let mut invert_astc_part = [false; 3];
        for astc_part in 0..3 {
            let ep = &mut a.endpoints[6 * astc_part..6 * astc_part + 6];
            set_endpoints(ep, &res[astc_to_bc7_part[astc_part] as usize], 3);
            invert_astc_part[astc_part] = invert_if_needed(ep, range, 3);
        }
        for i in 0..16 {
            let bc7_part = BC7_PARTITION3[bp + i] as usize;
            a.weights[i] = res[bc7_part].selectors[index[i] as usize];
            let astc_part = (0..3)
                .find(|&k| astc_to_bc7_part[k] as usize == bc7_part)
                .unwrap_or(0);
            if invert_astc_part[astc_part] {
                a.weights[i] = 3 - a.weights[i];
            }
        }
        push(results, 3, common_pattern, a, total_part_err);
    }
}

/// Mode 7: 2 subset ASTC, 3 subset BC7.
fn astc_mode7(
    block: &Block,
    results: &mut Vec<UastcEncodeResults>,
    comp: &Bc7encCompressBlockParams,
    estimate_partition: bool,
) {
    let mut first = 0u32;
    let mut last = BC7_3_ASTC2_COMMON_PARTITIONS.len() as u32;
    if estimate_partition {
        let mut best_err = u64::MAX;
        let mut best_common_pattern = 0u32;
        for common_pattern in 0..BC7_3_ASTC2_COMMON_PARTITIONS.len() {
            let (colors, counts) = split::<2>(block, &BC7_3_ASTC2_PATTERNS2[common_pattern]);
            let mut total_subset_err = 0u64;
            let mut subset = 0;
            while subset < 2 && total_subset_err < best_err {
                total_subset_err += color_cell_compression_est_astc(
                    4,
                    3,
                    &BC7_WEIGHTS2,
                    counts[subset],
                    &colors[subset],
                    best_err,
                    &[1, 1, 1, 1],
                );
                subset += 1;
            }
            if total_subset_err < best_err {
                best_err = total_subset_err;
                best_common_pattern = common_pattern as u32;
            }
        }
        first = best_common_pattern;
        last = best_common_pattern + 1;
    }
    let range = 12;
    for common_pattern in first..last {
        let (bc7_pattern, astc_pattern, k) = BC7_3_ASTC2_COMMON_PARTITIONS[common_pattern as usize];
        let bp = bc7_pattern as usize * 16;
        let part_of = |i: usize| {
            bc7_convert_partition_index_3_to_2(BC7_PARTITION3[bp + i] as u32, k as u32) as usize
        };
        let (pixels, counts, index) = split_indexed::<2>(block, part_of);
        let mut total_part_err = 0u64;
        let mut res = [ColorCellCompressorResults::default(); 2];
        for part in 0..2 {
            let (err, r) = ccell(
                &pixels[part],
                counts[part],
                &BC7_WEIGHTS2,
                &BC7_WEIGHTS2X,
                range,
                false,
                comp,
            );
            total_part_err += err;
            res[part] = r;
        }
        let mut a = desc(2, 2, astc_pattern as i32, 8, 0, false);
        let mut invert = [false; 2];
        set_endpoints(&mut a.endpoints[0..6], &res[0], 3);
        invert[0] = invert_if_needed(&mut a.endpoints[0..6], range, 3);
        set_endpoints(&mut a.endpoints[6..12], &res[1], 3);
        invert[1] = invert_if_needed(&mut a.endpoints[6..12], range, 3);
        for i in 0..16 {
            let astc_part = part_of(i);
            a.weights[i] = res[astc_part].selectors[index[i] as usize];
            if invert[astc_part] {
                a.weights[i] = 3 - a.weights[i];
            }
        }
        push(results, 7, common_pattern, a, total_part_err);
    }
}

/// `color_distance_la`.
#[inline]
fn color_distance_la(a: &ColorRgba, b: &ColorRgba) -> u32 {
    let dl = a[0] as i32 - b[0] as i32;
    let da = a[3] as i32 - b[3] as i32;
    (dl * dl + da * da) as u32
}

/// Mode 9 (RGBA, BC7 mode 7) or 16 (LA): 2 subsets.
fn astc_mode9_or_16(
    mode: u32,
    source_block: &Block,
    results: &mut Vec<UastcEncodeResults>,
    comp: &Bc7encCompressBlockParams,
    mut estimate_partition_list_size: u32,
) {
    let mut temp_block = *source_block;
    if mode == 16 {
        for p in temp_block.iter_mut() {
            // Use (l,0,0,a) not (l,l,l,a) so both components are treated equally.
            *p = [p[0], 0, 0, p[3]];
        }
    }
    let block = &temp_block;
    let weights = [1u32, 1, 1, 1];
    let range = if mode == 16 { 20 } else { 8 };

    let mut first = 0u32;
    let mut last = ASTC_BC7_COMMON_PARTITIONS2.len() as u32;
    let mut use_part_list = false;
    let mut parts = [0u32; 8];
    if estimate_partition_list_size == 1 {
        first = estimate_partition2(4, 4, &BC7_WEIGHTS2, block, &weights);
        last = first + 1;
    } else if estimate_partition_list_size > 0 {
        estimate_partition_list_size = estimate_partition_list_size.min(8);
        estimate_partition2_list(
            4,
            4,
            &BC7_WEIGHTS2,
            block,
            &mut parts,
            estimate_partition_list_size as usize,
            &weights,
        );
        first = 0;
        last = estimate_partition_list_size;
        use_part_list = true;
    }

    for iter in first..last {
        let common_pattern = if use_part_list {
            parts[iter as usize]
        } else {
            iter
        };
        let (bc7_pattern, seed, pinvert) = ASTC_BC7_COMMON_PARTITIONS2[common_pattern as usize];
        let bp = bc7_pattern as usize * 16;
        let (pixels, counts, index) =
            split_indexed::<2>(block, |i| BC7_PARTITION2[bp + i] as usize);
        let mut total_err = 0u64;
        let mut res = [ColorCellCompressorResults::default(); 2];
        for subset in 0..2 {
            let (subset_err, r) = ccell(
                &pixels[subset],
                counts[subset],
                &BC7_WEIGHTS2,
                &BC7_WEIGHTS2X,
                range,
                true,
                comp,
            );
            res[subset] = r;
            if mode == 16 {
                let mut colors = [[0u8; 4]; 4];
                for c in 0..4 {
                    let s = if c < 3 { 0 } else { 3 };
                    colors[0][c] = uq(range, r.astc_low_endpoint[s]) as u8;
                    colors[3][c] = uq(range, r.astc_high_endpoint[s]) as u8;
                }
                for i in 1..3 {
                    for c in 0..4 {
                        colors[i][c] = astc_interpolate(
                            colors[0][c] as u32,
                            colors[3][c] as u32,
                            BC7_WEIGHTS2[i],
                            false,
                        ) as u8;
                    }
                }
                for p in 0..counts[subset] as usize {
                    let mut orig = pixels[subset][p];
                    orig[1] = orig[0];
                    orig[2] = orig[0];
                    total_err += color_distance_la(&orig, &colors[r.selectors[p] as usize]) as u64;
                }
            } else {
                total_err += subset_err;
            }
        }

        let mut a = desc(2, 2, seed as i32, if mode == 16 { 4 } else { 12 }, 0, false);
        let part = if pinvert { [1, 0] } else { [0, 1] };
        let mut invert = [false; 2];
        for p in 0..2 {
            let r = &res[part[p]];
            if mode == 16 {
                a.endpoints[p * 4] = r.astc_low_endpoint[0];
                a.endpoints[p * 4 + 1] = r.astc_high_endpoint[0];
                a.endpoints[p * 4 + 2] = r.astc_low_endpoint[3];
                a.endpoints[p * 4 + 3] = r.astc_high_endpoint[3];
            } else {
                let ep = &mut a.endpoints[p * 8..p * 8 + 8];
                set_endpoints(ep, r, 4);
                invert[p] = invert_if_needed(ep, range, 4);
            }
        }
        for i in 0..16 {
            let bc7_part = BC7_PARTITION2[bp + i] as usize;
            a.weights[i] = res[bc7_part].selectors[index[i] as usize];
            let astc_part = if pinvert { 1 - bc7_part } else { bc7_part };
            if invert[astc_part] {
                a.weights[i] = 3 - a.weights[i];
            }
        }
        push(results, mode, common_pattern, a, total_err);
    }
}

/// Dual plane modes 6 (RGB, rot 0..2), 11 (RGBA, rot 0..3), 13 (RGBA 1-bit, rot 0..3) and
/// 17 (LA).
fn astc_mode_dual_plane(
    mode: u32,
    block: &Block,
    results: &mut Vec<UastcEncodeResults>,
    comp_params: &Bc7encCompressBlockParams,
) {
    let (weight_range, range, w, wx): (i32, u32, &[u32], &[Bc7encVec4F]) = match mode {
        6 => (2, 18, &BC7_WEIGHTS2, &BC7_WEIGHTS2X),
        11 => (2, 13, &BC7_WEIGHTS2, &BC7_WEIGHTS2X),
        13 => (0, 20, &BC7_WEIGHTS1, &BC7_WEIGHTS1X),
        _ => (2, 20, &BC7_WEIGHTS2, &BC7_WEIGHTS2X),
    };
    let mut local = *comp_params;
    if mode != 6 {
        local.perceptual = false;
        local.weights = [1, 1, 1, 1];
    }
    let comp = &local;
    let last_rot_comp = match mode {
        6 => 3,
        17 => 1,
        _ => 4,
    };
    for rot_comp in 0..last_rot_comp {
        let mut block_rgb = *block;
        let mut block_a = *block;
        for i in 0..16 {
            if mode == 17 {
                let a = block_rgb[i][3];
                block_a[i] = [a, a, a, 255];
                let l = block_rgb[i][0];
                block_rgb[i] = [l, l, l, 255];
            } else {
                let c = block_a[i][rot_comp];
                block_a[i] = [c, c, c, 255];
                if mode == 6 {
                    block_rgb[i][rot_comp] = 255;
                } else {
                    block_rgb[i][rot_comp] = block_rgb[i][3];
                    block_rgb[i][3] = 255;
                }
            }
        }
        let (part_err_rgb, rrgb) = ccell(&block_rgb, 16, w, wx, range, false, comp);
        let (err_a, ra) = ccell(&block_a, 16, w, wx, range, false, comp);
        let part_err_a = err_a / 3;
        let total_err = if mode == 17 {
            part_err_rgb / 3 + part_err_a
        } else {
            part_err_rgb + part_err_a
        };

        let mut a = desc(
            weight_range,
            1,
            0,
            match mode {
                6 => 8,
                17 => 4,
                _ => 12,
            },
            if mode == 17 { 3 } else { rot_comp as i32 },
            true,
        );
        let mut invert = false;
        if mode == 17 {
            a.endpoints[0] = rrgb.astc_low_endpoint[0];
            a.endpoints[1] = rrgb.astc_high_endpoint[0];
            a.endpoints[2] = ra.astc_low_endpoint[0];
            a.endpoints[3] = ra.astc_high_endpoint[0];
        } else {
            for c in 0..3 {
                let r = if rot_comp == c { &ra } else { &rrgb };
                a.endpoints[c * 2] = r.astc_low_endpoint[c];
                a.endpoints[c * 2 + 1] = r.astc_high_endpoint[c];
            }
            let npairs = if mode == 6 {
                3
            } else {
                if rot_comp == 3 {
                    a.endpoints[6] = ra.astc_low_endpoint[0];
                    a.endpoints[7] = ra.astc_high_endpoint[0];
                } else {
                    a.endpoints[6] = rrgb.astc_low_endpoint[rot_comp];
                    a.endpoints[7] = rrgb.astc_high_endpoint[rot_comp];
                }
                4
            };
            invert = invert_if_needed(&mut a.endpoints, range, npairs);
        }
        let max_w = w.len() as u32 - 1;
        for i in 0..16 {
            let mut rgb_index = rrgb.selectors[i] as u32;
            let mut a_index = ra.selectors[i] as u32;
            if invert {
                rgb_index = max_w - rgb_index;
                a_index = max_w - a_index;
            }
            a.weights[i * 2] = rgb_index as u8;
            a.weights[i * 2 + 1] = a_index as u8;
        }
        push(results, mode, 0, a, total_err);
    }
}

/// Mode 15: LA, 16 weights, BC7 mode 6.
fn astc_mode15(
    block: &Block,
    results: &mut Vec<UastcEncodeResults>,
    comp: &Bc7encCompressBlockParams,
) {
    let range = 20;
    let mut temp_block = [[0u8; 4]; 16];
    for i in 0..16 {
        // Use (l,0,0,a) not (l,l,l,a) so both components are treated equally.
        temp_block[i] = [block[i][0], 0, 0, block[i][3]];
    }
    let (_, r) = ccell(
        &temp_block,
        16,
        &ASTC_WEIGHTS4,
        &ASTC_WEIGHTS4X,
        range,
        true,
        comp,
    );
    let mut a = desc(8, 1, 0, 4, 0, false);
    a.endpoints[0] = r.astc_low_endpoint[0];
    a.endpoints[1] = r.astc_high_endpoint[0];
    a.endpoints[2] = r.astc_low_endpoint[3];
    a.endpoints[3] = r.astc_high_endpoint[3];
    a.weights[..16].copy_from_slice(&r.selectors);

    let mut colors = [[0u8; 4]; 16];
    for c in 0..4 {
        let s = if c < 3 { 0 } else { 3 };
        colors[0][c] = uq(range, r.astc_low_endpoint[s]) as u8;
        colors[15][c] = uq(range, r.astc_high_endpoint[s]) as u8;
    }
    for i in 1..15 {
        for c in 0..4 {
            colors[i][c] = astc_interpolate(
                colors[0][c] as u32,
                colors[15][c] as u32,
                ASTC_WEIGHTS4[i],
                false,
            ) as u8;
        }
    }
    let mut total_err = 0u64;
    for p in 0..16 {
        total_err += color_distance_la(&block[p], &colors[r.selectors[p] as usize]) as u64;
    }
    push(results, 15, 0, a, total_err);
}

/// `compute_block_error`: (rgb, rgba, la).
fn compute_block_error(block: &Block, decoded: &Block) -> (u64, u64, u64) {
    let mut e = [0u64; 4];
    for i in 0..16 {
        for c in 0..4 {
            let d = block[i][c] as i32 - decoded[i][c] as i32;
            e[c] += (d * d) as u64;
        }
    }
    let rgb = e[0] + e[1] + e[2];
    (rgb, rgb + e[3], e[0] + e[3])
}

#[inline]
fn to_c32(b: &Block) -> [Color32; 16] {
    b.map(|c| Color32 { c })
}

#[inline]
fn from_c32(b: &[Color32; 16]) -> Block {
    b.map(|c| c.c)
}

fn unpack_results(r: &UastcEncodeResults, out: &mut Block) -> bool {
    let mut px = [Color32::default(); 16];
    let ok = unpack_uastc_pixels_desc(
        r.uastc_mode,
        r.common_pattern,
        &Color32 { c: r.solid_color },
        &r.astc,
        &mut px,
        false,
    );
    *out = from_c32(&px);
    ok
}

fn compute_bc1_hints(best: &UastcEncodeResults, block: &Block, decoded: &Block) -> (bool, bool) {
    let best_mode = best.uastc_mode;
    let perceptual = false;
    if best_mode == UASTC_MODE_INDEX_SOLID_COLOR {
        return (false, false);
    }
    let has_h0 = mt(&UASTC_MODE_HAS_BC1_HINT0, best_mode) != 0;
    let has_h1 = mt(&UASTC_MODE_HAS_BC1_HINT1, best_mode) != 0;
    if !has_h0 && !has_h1 {
        return (false, false);
    }

    let decoded32 = to_c32(decoded);
    let mut tbc1 = Dxt1Block::default();
    encode_bc1(&mut tbc1, &decoded32, 0);
    let mut tblock_bc1 = [[0u8; 4]; 16];
    unpack_bc1(&tbc1.bytes, &mut tblock_bc1, true);

    let mut tblock_hint0 = [[0u8; 4]; 16];
    let mut tblock_hint1 = [[0u8; 4]; 16];

    // Pack to UASTC, then unpack, because the endpoints may be swapped.
    let etc1_blk = EtcBlock::default();
    let etc2_blk = EacA8Block {
        multiplier: 1,
        ..Default::default()
    };
    let mut temp = [0u8; 16];
    pack_uastc(&mut temp, best, &etc1_blk, 0, &etc2_blk, false, false);
    let mut temp_unpacked = UnpackedUastcBlock::default();
    unpack_uastc(&temp, &mut temp_unpacked, false, true);
    let ublock = UnpackedUastcBlock {
        mode: best.uastc_mode,
        common_pattern: best.common_pattern,
        astc: temp_unpacked.astc,
        ..Default::default()
    };

    let mut b = Dxt1Block::default();
    if has_h1 {
        transcode_uastc_to_bc1_hint1(&ublock, &decoded32, &mut b, false);
        unpack_bc1(&b.bytes, &mut tblock_hint1, true);
    }
    if has_h0 {
        transcode_uastc_to_bc1_hint0(&ublock, &mut b);
        unpack_bc1(&b.bytes, &mut tblock_hint0, true);
    }

    let (mut t, mut h0, mut h1) = (0u64, 0u64, 0u64);
    for i in 0..16 {
        t += color_distance(perceptual, &block[i], &tblock_bc1[i], false) as u64;
        h0 += color_distance(perceptual, &block[i], &tblock_hint0[i], false) as u64;
        h1 += color_distance(perceptual, &block[i], &tblock_hint1[i], false) as u64;
    }
    let t_err = (t as f32).sqrt();
    let t_err_hint0 = (h0 as f32).sqrt();
    let t_err_hint1 = (h1 as f32).sqrt();
    let err_thresh0 = 1.075f32;
    let err_thresh1 = 1.075f32;
    (
        has_h0 && t_err_hint0 <= t_err * err_thresh0,
        has_h1 && t_err_hint1 <= t_err * err_thresh1,
    )
}

#[derive(Clone, Copy, Default)]
struct Ycbcr {
    y: i32,
    cb: i32,
    cr: i32,
}

#[inline]
fn rgb_to_y_cb_cr(c: &ColorRgba) -> Ycbcr {
    let y = c[0] as i32 * 54 + c[1] as i32 * 183 + c[2] as i32 * 19;
    Ycbcr {
        y,
        cb: ((c[2] as i32) << 8) - y,
        cr: ((c[0] as i32) << 8) - y,
    }
}

#[inline]
fn color_diff(a: &Ycbcr, b: &Ycbcr) -> u64 {
    let yd = (a.y - b.y) as i64;
    let cbd = (a.cb - b.cb) as i64;
    let crd = (a.cr - b.cr) as i64;
    (yd * yd * 4 + crd * crd + cbd * cbd) as u64
}

#[inline]
fn gray_distance2(c: &ColorRgba, r: i32, g: i32, b: i32) -> i32 {
    let gray_dist = ((c[0] as i32 - r) + (c[1] as i32 - g) + (c[2] as i32 - b) + 1) / 3;
    let gr = (r + gray_dist).clamp(0, 255);
    let gg = (g + gray_dist).clamp(0, 255);
    let gb = (b + gray_dist).clamp(0, 255);
    let dr = c[0] as i32 - gr;
    let dg = c[1] as i32 - gg;
    let db = c[2] as i32 - gb;
    dr * dr + dg * dg + db * db
}

fn pack_etc1_estimate_flipped(src: &Block) -> bool {
    let px = |x: usize, y: usize, c: usize| src[x + y * 4][c] as i32;
    let mut sums = [[[0i32; 2]; 2]; 3];
    for c in 0..3 {
        sums[c][0][0] = px(0, 0, c) + px(0, 1, c) + px(1, 0, c) + px(1, 1, c);
        sums[c][1][0] = px(2, 0, c) + px(2, 1, c) + px(3, 0, c) + px(3, 1, c);
        sums[c][0][1] = px(0, 2, c) + px(0, 3, c) + px(1, 2, c) + px(1, 3, c);
        sums[c][1][1] = px(2, 2, c) + px(2, 3, c) + px(3, 2, c) + px(3, 3, c);
    }
    let mut upper = [0i32; 3];
    let mut lower = [0i32; 3];
    let mut left = [0i32; 3];
    let mut right = [0i32; 3];
    for c in 0..3 {
        upper[c] = (sums[c][0][0] + sums[c][1][0] + 4) / 8;
        lower[c] = (sums[c][0][1] + sums[c][1][1] + 4) / 8;
        left[c] = (sums[c][0][0] + sums[c][0][1] + 4) / 8;
        right[c] = (sums[c][1][0] + sums[c][1][1] + 4) / 8;
    }
    let gd = |x: usize, y: usize, a: &[i32; 3]| gray_distance2(&src[x + y * 4], a[0], a[1], a[2]);
    let (mut ug, mut lg, mut leg, mut rg) = (0i32, 0i32, 0i32, 0i32);
    for i in 0..4 {
        for j in 0..2 {
            ug += gd(i, j, &upper);
            lg += gd(i, 2 + j, &lower);
            leg += gd(j, i, &left);
            rg += gd(2 + j, i, &right);
        }
    }
    ug + lg < leg + rg
}

#[inline]
fn min4(a: u64, b: u64, c: u64, d: u64) -> u64 {
    a.min(b).min(c).min(d)
}

fn compute_etc1_hints(
    best_etc1_blk: &mut EtcBlock,
    best_results: &UastcEncodeResults,
    block: &Block,
    decoded: &Block,
    level: u32,
    flags: u32,
) -> u32 {
    let mut best_etc1_bias = 0u32;
    if best_results.uastc_mode == UASTC_MODE_INDEX_SOLID_COLOR {
        pack_etc1_block_solid_color(best_etc1_blk, &best_results.solid_color);
        return 0;
    }
    let faster_etc1 = flags & C_PACK_UASTC_ETC1_FASTER_HINTS != 0;
    let fastest_etc1 = flags & C_PACK_UASTC_ETC1_FASTEST_HINTS != 0;
    let has_bias = mt(&UASTC_MODE_HAS_ETC1_BIAS, best_results.uastc_mode) != 0;

    // 0 should be at the top, but we need 13 first because it represents bias (0,0,0).
    const S_SORTED_BIAS_MODES: [u8; 32] = [
        13, 0, 22, 29, 27, 12, 26, 9, 30, 31, 8, 10, 25, 2, 23, 5, 15, 7, 3, 11, 6, 17, 28, 18, 1,
        19, 20, 21, 24, 4, 14, 16,
    ];

    let mut last_bias = 1u32;
    let mut use_faster_bias_mode_table = false;
    let flip_estimate = level <= C_PACK_UASTC_LEVEL_FASTER || faster_etc1 || fastest_etc1;
    if has_bias {
        let pick = |a: u32, b: u32| {
            if fastest_etc1 {
                1
            } else if faster_etc1 {
                a
            } else {
                b
            }
        };
        match level {
            C_PACK_UASTC_LEVEL_FASTEST => {
                last_bias = pick(1, 2);
                use_faster_bias_mode_table = true;
            }
            C_PACK_UASTC_LEVEL_FASTER => {
                last_bias = pick(3, 5);
                use_faster_bias_mode_table = true;
            }
            C_PACK_UASTC_LEVEL_DEFAULT => {
                last_bias = pick(10, 20);
                use_faster_bias_mode_table = true;
            }
            C_PACK_UASTC_LEVEL_SLOWER => {
                last_bias = pick(16, 32);
                use_faster_bias_mode_table = true;
            }
            _ => last_bias = 32,
        }
    }

    *best_etc1_blk = EtcBlock::default();
    let mut best_err = u64::MAX;
    let mut trial_block = EtcBlock::default();

    let mut block_ycbcr = [Ycbcr::default(); 16];
    let mut dec_ycbcr = [Ycbcr::default(); 16];
    for i in 0..16 {
        block_ycbcr[i] = rgb_to_y_cb_cr(&block[i]);
        dec_ycbcr[i] = rgb_to_y_cb_cr(&decoded[i]);
    }
    let byc = |x: usize, y: usize| &block_ycbcr[y * 4 + x];
    let dyc = |x: usize, y: usize| &dec_ycbcr[y * 4 + x];

    let (mut first_flip, mut last_flip) = (0u32, 2u32);
    let (first_individ, mut last_individ) = (0u32, 2u32);
    if flags & C_PACK_UASTC_ETC1_DISABLE_FLIP_AND_INDIVIDUAL != 0 {
        last_flip = 1;
        last_individ = 1;
    } else if flip_estimate {
        if pack_etc1_estimate_flipped(decoded) {
            first_flip = 1;
        }
        last_flip = first_flip + 1;
    }

    for flip in first_flip..last_flip {
        trial_block.set_flip_bit(flip != 0);
        for individ in first_individ..last_individ {
            let mul = if individ != 0 { 15 } else { 31 };
            trial_block.set_diff_bit(individ == 0);

            let mut unbiased = [[0u8; 4]; 2];
            let mut min_r = [255i32; 2];
            let mut min_g = [255i32; 2];
            let mut min_b = [255i32; 2];
            let mut max_r = [0i32; 2];
            let mut max_g = [0i32; 2];
            let mut max_b = [0i32; 2];
            for subset in 0..2 {
                let mut avg = [0u32; 3];
                for j in 0..8 {
                    let (cx, cy) = G_ETC1_PIXEL_COORDS[flip as usize][subset][j];
                    let p = &decoded[cy as usize * 4 + cx as usize];
                    avg[0] += p[0] as u32;
                    avg[1] += p[1] as u32;
                    avg[2] += p[2] as u32;
                    min_r[subset] = min_r[subset].min(p[0] as i32);
                    min_g[subset] = min_g[subset].min(p[1] as i32);
                    min_b[subset] = min_b[subset].min(p[2] as i32);
                    max_r[subset] = max_r[subset].max(p[0] as i32);
                    max_g[subset] = max_g[subset].max(p[1] as i32);
                    max_b[subset] = max_b[subset].max(p[2] as i32);
                }
                for c in 0..3 {
                    unbiased[subset][c] = ((avg[c] * mul + 1020) / (8 * 255)) as u8;
                }
                unbiased[subset][3] = 0;
            }

            for bias_iter in 0..last_bias {
                let bias = if use_faster_bias_mode_table {
                    S_SORTED_BIAS_MODES[bias_iter as usize] as u32
                } else {
                    bias_iter
                };
                let mut block_colors = [[0u8; 4]; 2];
                for subset in 0..2 {
                    block_colors[subset] = if has_bias {
                        apply_etc1_bias(&unbiased[subset], bias, mul, subset as u32)
                    } else {
                        unbiased[subset]
                    };
                }
                if individ != 0 {
                    trial_block.set_block_color4(&block_colors[0], &block_colors[1]);
                } else {
                    trial_block.set_block_color5_clamp(&block_colors[0], &block_colors[1]);
                }

                let mut range = [0u32; 2];
                for subset in 0..2 {
                    let base_c = trial_block.get_block_color(subset as u32, true);
                    let v = [
                        (max_r[subset] - base_c[0] as i32).abs(),
                        (base_c[0] as i32 - min_r[subset]).abs(),
                        (max_g[subset] - base_c[1] as i32).abs(),
                        (base_c[1] as i32 - min_g[subset]).abs(),
                        (max_b[subset] - base_c[2] as i32).abs(),
                        (base_c[2] as i32 - min_b[subset]).abs(),
                    ];
                    range[subset] = v.iter().copied().max().unwrap_or(0) as u32;
                }

                let mut best_inten_table = [0u32; 2];
                for subset in 0..2 {
                    let mut best_subset_err = u64::MAX;
                    let inten_table_limit =
                        if level == C_PACK_UASTC_LEVEL_VERY_SLOW || range[subset] > 51 {
                            8
                        } else if range[subset] >= 7 {
                            4
                        } else {
                            2
                        };
                    for inten_table in 0..inten_table_limit {
                        trial_block.set_inten_table(subset as u32, inten_table);
                        let mut color_table = [[0u8; 4]; 4];
                        trial_block.get_block_colors(&mut color_table, subset as u32);
                        let ct = color_table.map(|c| rgb_to_y_cb_cr(&c));
                        let md = |c: &Ycbcr| {
                            min4(
                                color_diff(&ct[0], c),
                                color_diff(&ct[1], c),
                                color_diff(&ct[2], c),
                                color_diff(&ct[3], c),
                            )
                        };
                        let mut total_error = 0u64;
                        if flip != 0 {
                            for y in 0..2 {
                                for x in 0..4 {
                                    total_error += md(dyc(x, subset * 2 + y));
                                }
                                if total_error >= best_subset_err {
                                    break;
                                }
                            }
                        } else {
                            for y in 0..4 {
                                total_error += md(dyc(subset * 2, y));
                                total_error += md(dyc(subset * 2 + 1, y));
                            }
                            if total_error >= best_subset_err {
                                break;
                            }
                        }
                        if total_error < best_subset_err {
                            best_subset_err = total_error;
                            best_inten_table[subset] = inten_table;
                        }
                    }
                }
                trial_block.set_inten_table(0, best_inten_table[0]);
                trial_block.set_inten_table(1, best_inten_table[1]);

                // Compute error against the ORIGINAL block.
                let mut err = 0u64;
                for subset in 0..2 {
                    let mut color_table = [[0u8; 4]; 4];
                    trial_block.get_block_colors(&mut color_table, subset as u32);
                    let ct = color_table.map(|c| rgb_to_y_cb_cr(&c));
                    let add = |x: usize, y: usize| -> u64 {
                        let c = dyc(x, y);
                        let best_index_err = min4(
                            color_diff(&ct[0], c) << 2,
                            (color_diff(&ct[1], c) << 2) + 1,
                            (color_diff(&ct[2], c) << 2) + 2,
                            (color_diff(&ct[3], c) << 2) + 3,
                        );
                        let best_index = (best_index_err & 3) as usize;
                        color_diff(byc(x, y), &ct[best_index])
                    };
                    if flip != 0 {
                        for y in 0..2 {
                            for x in 0..4 {
                                err += add(x, subset * 2 + y);
                            }
                            if err >= best_err {
                                break;
                            }
                        }
                    } else {
                        for y in 0..4 {
                            for x in 0..2 {
                                err += add(subset * 2 + x, y);
                            }
                            if err >= best_err {
                                break;
                            }
                        }
                    }
                }
                if err < best_err {
                    best_err = err;
                    *best_etc1_blk = trial_block;
                    best_etc1_bias = bias;
                }
            }
        }
    }
    best_etc1_bias
}

/// `uastc_pack_eac_a8`: returns (error, table, multiplier); the base is only needed by
/// callers that pack full EAC blocks, which UASTC hinting does not.
fn uastc_pack_eac_a8(
    pixels: &[u8; 16],
    base_search_rad: u32,
    mul_search_rad: u32,
    table_mask: u32,
) -> (u64, u32, u32) {
    let (mut table_r, mut mul_r) = (0u32, 0u32);
    let min_alpha = pixels.iter().copied().min().unwrap_or(0) as u32;
    let max_alpha = pixels.iter().copied().max().unwrap_or(0) as u32;
    if min_alpha == max_alpha {
        return (0, 13, 1);
    }
    let alpha_range = max_alpha - min_alpha;
    let mut best_err = u64::MAX;
    for table in 0..16usize {
        if table_mask & (1 << table) == 0 {
            continue;
        }
        let t = &G_ETC2_EAC_TABLES[table];
        let range =
            (t[ETC2_EAC_MAX_VALUE_SELECTOR] as i32 - t[ETC2_EAC_MIN_VALUE_SELECTOR] as i32) as f32;
        let s = (0 - t[ETC2_EAC_MIN_VALUE_SELECTOR] as i32) as f32 / range;
        let center = (min_alpha as f32 + (max_alpha as f32 - min_alpha as f32) * s).round() as i32;
        let base_min = (center - base_search_rad as i32).clamp(0, 255);
        let base_max = (center + base_search_rad as i32).clamp(0, 255);
        let mul = (alpha_range as f32 / range).round() as i32;
        let mul_low = (mul - mul_search_rad as i32).clamp(1, 15);
        let mul_high = (mul + mul_search_rad as i32).clamp(1, 15);
        for base in base_min..=base_max {
            for multiplier in mul_low..=mul_high {
                let mut total_err = 0u64;
                for &a in pixels.iter() {
                    let mut best_s_err = u32::MAX;
                    for s in 0..8 {
                        let v = (multiplier * t[s] as i32 + base).clamp(0, 255);
                        let err = (a as i32 - v).unsigned_abs();
                        if err < best_s_err {
                            best_s_err = err;
                        }
                    }
                    total_err += best_s_err.wrapping_mul(best_s_err) as u64;
                    if total_err >= best_err {
                        break;
                    }
                }
                if total_err < best_err {
                    best_err = total_err;
                    mul_r = multiplier as u32;
                    table_r = table as u32;
                    if best_err == 0 {
                        return (best_err, table_r, mul_r);
                    }
                }
            }
        }
    }
    (best_err, table_r, mul_r)
}

const DEFAULT_BC7_ERROR_WEIGHT: u64 = 50;
const UASTC_ERROR_THRESH: f32 = 1.3;

// TODO: This is a quick hack to favor certain modes when we know we'll be followed up with
// an RDO postprocess.
#[inline]
fn get_uastc_mode_weight(mode: u32) -> f32 {
    const FAVORED_MODE_WEIGHT: f32 = 0.8;
    match mode {
        0 | 10 => FAVORED_MODE_WEIGHT,
        _ => 1.0,
    }
}

/// `encode_uastc`: encodes 16 RGBA pixels (row major) into a UASTC block.
pub fn encode_uastc(rgba_pixels: &[u8; 64], output_block: &mut [u8; 16], flags: u32) {
    let mut block: Block = [[0u8; 4]; 16];
    for i in 0..16 {
        block[i].copy_from_slice(&rgba_pixels[i * 4..i * 4 + 4]);
    }

    let mut solid_color = true;
    let mut has_alpha = false;
    let mut is_la = true;
    let first_color = block[0];
    for c in &block {
        if c[3] < 255 {
            has_alpha = true;
        }
        if *c != first_color {
            solid_color = false;
        }
        if c[0] != c[1] || c[0] != c[2] {
            is_la = false;
        }
    }

    if solid_color {
        // Solid color blocks are so common that we handle them specially and as quickly as we can.
        let solid_results = UastcEncodeResults {
            uastc_mode: UASTC_MODE_INDEX_SOLID_COLOR,
            solid_color: first_color,
            ..Default::default()
        };
        let mut etc1_blk = EtcBlock::default();
        pack_etc1_block_solid_color(&mut etc1_blk, &first_color);
        let eac_a8_blk = EacA8Block {
            table: 0,
            multiplier: 1,
            ..Default::default()
        };
        pack_uastc(
            output_block,
            &solid_results,
            &etc1_blk,
            0,
            &eac_a8_blk,
            false,
            false,
        );
        return;
    }

    let level = (flags & 7).min(C_PACK_UASTC_LEVEL_VERY_SLOW);
    let favor_uastc_error = flags & C_PACK_UASTC_FAVOR_UASTC_ERROR != 0;
    let favor_bc7_error = !favor_uastc_error && flags & C_PACK_UASTC_FAVOR_BC7_ERROR != 0;

    // Set all options to slowest, then configure from there depending on the selected level.
    let mut mode_mask = u32::MAX;
    let mut uber_level = 6u32;
    let mut estimate_partition = false;
    let mut always_try_alpha_modes = true;
    let mut eac_a8_mul_search_rad = 3u32;
    let mut eac_a8_table_mask = u32::MAX;
    let mut least_squares_passes = 2u32;
    let mut bc1_hints = true;
    let mut only_use_la_on_transparent_blocks = false;
    match level {
        C_PACK_UASTC_LEVEL_FASTEST => {
            mode_mask = (1 << 0) | (1 << 8) | (1 << 11) | (1 << 12) | (1 << 15);
            always_try_alpha_modes = false;
            eac_a8_mul_search_rad = 0;
            eac_a8_table_mask = (1 << 2) | (1 << 8) | (1 << 11) | (1 << 13);
            uber_level = 0;
            least_squares_passes = 1;
            bc1_hints = false;
            estimate_partition = true;
            only_use_la_on_transparent_blocks = true;
        }
        C_PACK_UASTC_LEVEL_FASTER => {
            mode_mask = (1 << 0)
                | (1 << 4)
                | (1 << 6)
                | (1 << 8)
                | (1 << 9)
                | (1 << 11)
                | (1 << 12)
                | (1 << 15)
                | (1 << 17);
            always_try_alpha_modes = false;
            eac_a8_mul_search_rad = 0;
            eac_a8_table_mask = (1 << 2) | (1 << 8) | (1 << 11) | (1 << 13);
            uber_level = 0;
            least_squares_passes = 1;
            estimate_partition = true;
        }
        C_PACK_UASTC_LEVEL_DEFAULT => {
            mode_mask = (1 << 0)
                | (1 << 1)
                | (1 << 4)
                | (1 << 5)
                | (1 << 6)
                | (1 << 8)
                | (1 << 9)
                | (1 << 10)
                | (1 << 11)
                | (1 << 12)
                | (1 << 13)
                | (1 << 15)
                | (1 << 16)
                | (1 << 17);
            always_try_alpha_modes = false;
            eac_a8_mul_search_rad = 1;
            eac_a8_table_mask = (1 << 0)
                | (1 << 2)
                | (1 << 6)
                | (1 << 7)
                | (1 << 8)
                | (1 << 10)
                | (1 << 11)
                | (1 << 13);
            uber_level = 1;
            least_squares_passes = 1;
            estimate_partition = true;
        }
        C_PACK_UASTC_LEVEL_SLOWER => {
            always_try_alpha_modes = false;
            eac_a8_mul_search_rad = 2;
            uber_level = 3;
            estimate_partition = true;
        }
        _ => {}
    }

    if only_use_la_on_transparent_blocks && is_la && !has_alpha {
        is_la = false;
    }
    let try_alpha_modes = has_alpha || always_try_alpha_modes;

    let comp = Bc7encCompressBlockParams {
        max_partitions_mode1: 64,
        weights: [1, 1, 1, 1],
        uber_level,
        perceptual: false,
        least_squares_passes,
    };

    let mut results: Vec<UastcEncodeResults> = Vec::with_capacity(64);
    let on = |m: u32| mode_mask & (1 << m) != 0;
    let est4 = if estimate_partition { 4 } else { 0 };

    if is_la {
        if on(15) {
            astc_mode15(&block, &mut results, &comp);
        }
        if on(16) {
            astc_mode9_or_16(16, &block, &mut results, &comp, est4);
        }
        if on(17) {
            astc_mode_dual_plane(17, &block, &mut results, &comp);
        }
    }
    if !has_alpha {
        if on(0) {
            astc_mode_1subset(0, &block, &mut results, &comp, None);
        }
        if on(1) {
            astc_mode_1subset(1, &block, &mut results, &comp, None);
        }
        if on(2) {
            astc_mode2_or_4(2, &block, &mut results, &comp, estimate_partition);
        }
        if on(3) {
            astc_mode3(&block, &mut results, &comp, estimate_partition);
        }
        if on(4) {
            astc_mode2_or_4(4, &block, &mut results, &comp, estimate_partition);
        }
        if on(5) {
            astc_mode_1subset(5, &block, &mut results, &comp, None);
        }
        if on(6) {
            astc_mode_dual_plane(6, &block, &mut results, &comp);
        }
        if on(7) {
            astc_mode7(&block, &mut results, &comp, estimate_partition);
        }
        if on(18) {
            astc_mode_1subset(18, &block, &mut results, &comp, None);
        }
    }
    if try_alpha_modes {
        if on(9) {
            astc_mode9_or_16(9, &block, &mut results, &comp, est4);
        }
        if on(10) {
            astc_mode_1subset(10, &block, &mut results, &comp, None);
        }
        if on(11) {
            astc_mode_dual_plane(11, &block, &mut results, &comp);
        }
        if on(12) {
            astc_mode_1subset(12, &block, &mut results, &comp, None);
        }
        if on(13) {
            astc_mode_dual_plane(13, &block, &mut results, &comp);
        }
        if on(14) {
            astc_mode_1subset(14, &block, &mut results, &comp, None);
        }
    }
    if results.is_empty() {
        return;
    }

    // Fix up the errors so we consistently have LA, RGB, or RGBA error.
    for r in results.iter_mut() {
        let mode_is_la = mt(&UASTC_MODE_IS_LA, r.uastc_mode) != 0;
        if is_la != mode_is_la {
            let mut unpacked = [[0u8; 4]; 16];
            unpack_results(r, &mut unpacked);
            let mut total_err = 0u64;
            for j in 0..16 {
                total_err += if is_la {
                    color_distance_la(&unpacked[j], &block[j]) as u64
                } else {
                    color_distance_linear(&unpacked[j], &block[j], true) as u64
                };
            }
            r.astc_err = total_err;
        }
    }

    let total_results = results.len();
    let mut total_overall_err = vec![0u64; total_results];
    let mut uastc_err_f = vec![0f32; total_results];
    let mut best_uastc_err_f = 1e20f32 as f64;
    let mut best_index: Option<usize> = None;

    if total_results == 1 {
        best_index = Some(0);
    } else {
        let bc7_err_weight = if favor_bc7_error {
            100
        } else if favor_uastc_error {
            0
        } else {
            DEFAULT_BC7_ERROR_WEIGHT
        };
        let uastc_err_weight = if favor_bc7_error { 0 } else { 100 };

        // Find best overall results, balancing UASTC and UASTC->BC7 error.
        // We purposely allow UASTC error to increase a little, if doing so lowers the BC7 error.
        for i in 0..total_results {
            let r = &results[i];
            let unpacked_ublock = UnpackedUastcBlock {
                mode: r.uastc_mode,
                astc: r.astc,
                common_pattern: r.common_pattern,
                solid_color: Color32 { c: r.solid_color },
                ..Default::default()
            };
            let mut decoded = [[0u8; 4]; 16];
            unpack_results(r, &mut decoded);
            let (u_rgb, u_rgba, u_la) = compute_block_error(&block, &decoded);

            // Transcode to BC7
            let mut bc7_results = Bc7OptimizationResults::default();
            transcode_uastc_to_bc7_unpacked(&unpacked_ublock, &mut bc7_results);
            let mut bc7_data = [0u8; 16];
            encode_bc7_block(&mut bc7_data, &bc7_results);
            let mut decoded_bc7 = [[0u8; 4]; 16];
            unpack_bc7(&bc7_data, &mut decoded_bc7);
            let (b_rgb, b_rgba, b_la) = compute_block_error(&block, &decoded_bc7);

            if r.uastc_mode == UASTC_MODE_INDEX_SOLID_COLOR {
                best_index = Some(i);
                break;
            }
            let (total_uastc_err, total_bc7_err) = if is_la {
                (u_la, b_la)
            } else if has_alpha {
                (u_rgba, b_rgba)
            } else {
                (u_rgb, b_rgb)
            };
            total_overall_err[i] =
                (total_bc7_err * bc7_err_weight) / 100 + (total_uastc_err * uastc_err_weight) / 100;
            if total_overall_err[i] == 0 {
                best_index = Some(i);
                break;
            }
            uastc_err_f[i] = (total_uastc_err as f32).sqrt();
            if (uastc_err_f[i] as f64) < best_uastc_err_f {
                best_uastc_err_f = uastc_err_f[i] as f64;
            }
        }

        if best_index.is_none() {
            let mut best_err = u64::MAX;
            let simpler = flags & C_PACK_UASTC_FAVOR_SIMPLER_MODES != 0;
            let all = best_uastc_err_f == 0.0 || favor_bc7_error;
            for i in 0..total_results {
                if !all {
                    // Scan the UASTC results, and consider all results within a window that
                    // has the best UASTC+BC7 error.
                    let err_delta = uastc_err_f[i] as f64 / best_uastc_err_f;
                    if err_delta > UASTC_ERROR_THRESH as f64 {
                        continue;
                    }
                }
                // TODO: This is a quick hack to favor modes 0 or 10 for better RDO compression.
                let err_weight = if simpler {
                    get_uastc_mode_weight(results[i].uastc_mode)
                } else {
                    1.0
                };
                let w = (total_overall_err[i] as f32 * err_weight) as u64;
                if w < best_err {
                    best_err = w;
                    best_index = Some(i);
                    if best_err == 0 {
                        break;
                    }
                }
            }
        }
    }

    let Some(best_index) = best_index else {
        return;
    };
    let best_results = results[best_index];
    let best_mode = best_results.uastc_mode;
    let mut decoded = [[0u8; 4]; 16];
    unpack_results(&best_results, &mut decoded);

    // Compute BC1 hints
    let (bc1_hint0, bc1_hint1) = if bc1_hints {
        compute_bc1_hints(&best_results, &block, &decoded)
    } else {
        (false, false)
    };

    let mut eac_a8_blk = EacA8Block::default();
    if mt(&UASTC_MODE_HAS_ALPHA, best_mode) != 0 && best_mode != UASTC_MODE_INDEX_SOLID_COLOR {
        // Compute ETC2 hints
        let mut alpha = [0u8; 16];
        for i in 0..16 {
            alpha[i] = decoded[i][3];
        }
        let (_, table, multiplier) =
            uastc_pack_eac_a8(&alpha, 0, eac_a8_mul_search_rad, eac_a8_table_mask);
        // All we care about for hinting is the table and multiplier.
        eac_a8_blk.table = table as u8;
        eac_a8_blk.multiplier = multiplier as u8;
    }

    // Compute ETC1 hints
    let mut etc1_blk = EtcBlock::default();
    let etc1_bias =
        compute_etc1_hints(&mut etc1_blk, &best_results, &block, &decoded, level, flags);

    // Finally, pack the UASTC block with its hints and we're done.
    pack_uastc(
        output_block,
        &best_results,
        &etc1_blk,
        etc1_bias,
        &eac_a8_blk,
        bc1_hint0,
        bc1_hint1,
    );
}

/// `uastc_recompute_hints`: repacks `blk` from `unpacked` (or its own unpacked form) with
/// freshly computed BC1, ETC1 and ETC2 EAC A8 hints.
fn uastc_recompute_hints(
    blk: &mut [u8; 16],
    pixels: &Block,
    flags: u32,
    unpacked: Option<&UnpackedUastcBlock>,
) -> bool {
    let mut unpacked_blk = UnpackedUastcBlock::default();
    match unpacked {
        Some(u) => unpacked_blk = *u,
        None => {
            if !unpack_uastc(blk, &mut unpacked_blk, false, true) {
                return false;
            }
        }
    }
    let results = UastcEncodeResults {
        uastc_mode: unpacked_blk.mode,
        common_pattern: unpacked_blk.common_pattern,
        astc: unpacked_blk.astc,
        solid_color: unpacked_blk.solid_color.c,
        astc_err: 0,
    };
    let mut decoded: Block = [[0u8; 4]; 16];
    if !unpack_results(&results, &mut decoded) {
        return false;
    }
    let mut bc1_hints = true;
    let mut eac_a8_mul_search_rad = 3u32;
    let mut eac_a8_table_mask = u32::MAX;
    let level = flags & C_PACK_UASTC_LEVEL_MASK;
    match level {
        C_PACK_UASTC_LEVEL_FASTEST => {
            eac_a8_mul_search_rad = 0;
            eac_a8_table_mask = (1 << 2) | (1 << 8) | (1 << 11) | (1 << 13);
            bc1_hints = false;
        }
        C_PACK_UASTC_LEVEL_FASTER => {
            eac_a8_mul_search_rad = 0;
            eac_a8_table_mask = (1 << 2) | (1 << 8) | (1 << 11) | (1 << 13);
        }
        C_PACK_UASTC_LEVEL_DEFAULT => {
            eac_a8_mul_search_rad = 1;
            eac_a8_table_mask = (1 << 0)
                | (1 << 2)
                | (1 << 6)
                | (1 << 7)
                | (1 << 8)
                | (1 << 10)
                | (1 << 11)
                | (1 << 13);
        }
        C_PACK_UASTC_LEVEL_SLOWER => eac_a8_mul_search_rad = 2,
        _ => {}
    }
    let (bc1_hint0, bc1_hint1) = if bc1_hints {
        compute_bc1_hints(&results, pixels, &decoded)
    } else {
        (false, false)
    };
    let best_mode = unpacked_blk.mode;
    let mut eac_a8_blk = EacA8Block::default();
    if mt(&UASTC_MODE_HAS_ALPHA, best_mode) != 0 && best_mode != UASTC_MODE_INDEX_SOLID_COLOR {
        let mut alpha = [0u8; 16];
        for i in 0..16 {
            alpha[i] = decoded[i][3];
        }
        let (_, table, multiplier) =
            uastc_pack_eac_a8(&alpha, 0, eac_a8_mul_search_rad, eac_a8_table_mask);
        eac_a8_blk.table = table as u8;
        eac_a8_blk.multiplier = multiplier as u8;
    }
    let mut etc1_blk = EtcBlock::default();
    let etc1_bias = compute_etc1_hints(&mut etc1_blk, &results, pixels, &decoded, level, flags);
    pack_uastc(
        blk,
        &results,
        &etc1_blk,
        etc1_bias,
        &eac_a8_blk,
        bc1_hint0,
        bc1_hint1,
    );
    true
}

pub const UASCT_RDO_DEFAULT_LZ_DICT_SIZE: u32 = 4096;
pub const UASTC_RDO_DEFAULT_MAX_ALLOWED_RMS_INCREASE_RATIO: f32 = 10.0;
pub const UASTC_RDO_DEFAULT_SKIP_BLOCK_RMS_THRESH: f32 = 8.0;
pub const UASTC_RDO_DEFAULT_MAX_SMOOTH_BLOCK_STD_DEV: f32 = 18.0;
pub const UASTC_RDO_DEFAULT_SMOOTH_BLOCK_MAX_ERROR_SCALE: f32 = 10.0;

/// `uastc_rdo_params`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UastcRdoParams {
    /// Size of LZ dictionary to simulate in bytes.
    pub lz_dict_size: u32,
    /// The post-processor minimizes distortion + rate * lambda.
    pub lambda: f32,
    /// How much the RMS error of a block may increase before a trial is rejected.
    pub max_allowed_rms_increase_ratio: f32,
    /// Blocks with this much RMS error or more are skipped.
    pub skip_block_rms_thresh: f32,
    /// Refine the endpoints of mode 0 blocks with modified selectors.
    pub endpoint_refinement: bool,
    pub max_smooth_block_std_dev: f32,
    pub smooth_block_max_error_scale: f32,
    pub lz_literal_cost: u32,
}

impl Default for UastcRdoParams {
    /// `uastc_rdo_params::clear`.
    fn default() -> Self {
        UastcRdoParams {
            lz_dict_size: UASCT_RDO_DEFAULT_LZ_DICT_SIZE,
            lambda: 0.5,
            max_allowed_rms_increase_ratio: UASTC_RDO_DEFAULT_MAX_ALLOWED_RMS_INCREASE_RATIO,
            skip_block_rms_thresh: UASTC_RDO_DEFAULT_SKIP_BLOCK_RMS_THRESH,
            endpoint_refinement: true,
            max_smooth_block_std_dev: UASTC_RDO_DEFAULT_MAX_SMOOTH_BLOCK_STD_DEV,
            smooth_block_max_error_scale: UASTC_RDO_DEFAULT_SMOOTH_BLOCK_MAX_ERROR_SCALE,
            lz_literal_cost: 100,
        }
    }
}

const UASTC_MODE_SELECTOR_BITS: [[u8; 2]; 19] = [
    [65, 63],
    [69, 31],
    [73, 46],
    [89, 29],
    [89, 30],
    [68, 47],
    [66, 62],
    [89, 30],
    [0, 0],
    [97, 30],
    [65, 63],
    [66, 62],
    [81, 47],
    [94, 30],
    [92, 31],
    [62, 63],
    [98, 30],
    [61, 62],
    [49, 79],
];

/// `basisu::read_bits` (LSB first, at most 64 bits).
fn read_bits(buf: &[u8; 16], bit_offset: &mut u32, codesize: u32) -> u64 {
    let mut bits = 0u64;
    let mut total_bits = 0u32;
    while total_bits < codesize {
        let byte_bit_offset = *bit_offset & 7;
        let bits_to_read = (codesize - total_bits).min(8 - byte_bit_offset);
        let mut byte_bits = (buf[(*bit_offset >> 3) as usize] as u32) >> byte_bit_offset;
        byte_bits &= (1 << bits_to_read) - 1;
        bits |= (byte_bits as u64) << total_bits;
        total_bits += bits_to_read;
        *bit_offset += bits_to_read;
    }
    bits
}

fn set_block_bits(bytes: &mut [u8; 16], mut val: u64, mut num_bits: u32, mut cur_ofs: u32) -> u32 {
    let mut mask = if num_bits == 64 {
        u64::MAX
    } else {
        (1u64 << num_bits) - 1
    };
    while num_bits != 0 {
        let n = (8 - (cur_ofs & 7)).min(num_bits);
        let b = &mut bytes[(cur_ofs >> 3) as usize];
        *b &= !((mask << (cur_ofs & 7)) as u8);
        *b |= (val << (cur_ofs & 7)) as u8;
        val >>= n;
        mask >>= n;
        num_bits -= n;
        cur_ofs += n;
    }
    cur_ofs
}

const TDEFL_SMALL_DIST_EXTRA: [u8; 512] = {
    let mut t = [0u8; 512];
    let mut i = 0;
    while i < 512 {
        // floor(log2(i)) - 1 for i >= 4, 0 below (the tdefl distance extra bits).
        t[i] = if i < 4 {
            0
        } else {
            (31 - (i as u32).leading_zeros()) as u8 - 1
        };
        i += 1;
    }
    t
};

const TDEFL_LARGE_DIST_EXTRA: [u8; 128] = {
    let mut t = [0u8; 128];
    let mut i = 2;
    while i < 128 {
        t[i] = (31 - (i as u32).leading_zeros()) as u8 + 7;
        i += 1;
    }
    t
};

fn compute_match_cost_estimate(mut dist: u32) -> u32 {
    let len_cost = 7;
    let mut dist_cost = 5;
    if dist < 512 {
        dist_cost += TDEFL_SMALL_DIST_EXTRA[(dist & 511) as usize] as u32;
    } else {
        dist_cost += TDEFL_LARGE_DIST_EXTRA[(dist.min(32767) >> 8) as usize] as u32;
        while dist >= 32768 {
            dist_cost += 1;
            dist >>= 1;
        }
    }
    len_cost + dist_cost
}

/// `tracked_stat::get_std_dev` over one channel of 16 pixels.
fn std_dev(px: &Block, c: usize) -> f32 {
    let (mut total, mut total2) = (0i64, 0i64);
    for p in px {
        let v = p[c] as i32;
        total += v as i64;
        total2 += (v * v) as i64;
    }
    ((16 * total2 - total * total) as f32).sqrt() / 16.0
}

/// UASTC error averaged with the error of its BC7 transcoding (`(uastc_err + bc7_err) / 2`).
fn rdo_block_error(pixels: &Block, unpacked: &UnpackedUastcBlock) -> Option<u64> {
    let mut px = [Color32::default(); 16];
    if !unpack_uastc_pixels_desc(
        unpacked.mode,
        unpacked.common_pattern,
        &unpacked.solid_color,
        &unpacked.astc,
        &mut px,
        false,
    ) {
        return None;
    }
    let decoded = from_c32(&px);
    let mut uastc_err = 0u64;
    for i in 0..16 {
        uastc_err += color_distance(false, &pixels[i], &decoded[i], true) as u64;
    }
    // Transcode to BC7
    let mut b7_results = Bc7OptimizationResults::default();
    if !transcode_uastc_to_bc7_unpacked(unpacked, &mut b7_results) {
        return None;
    }
    let mut b7_block = [0u8; 16];
    encode_bc7_block(&mut b7_block, &b7_results);
    let mut decoded_b7: Block = [[0u8; 4]; 16];
    unpack_bc7(&b7_block, &mut decoded_b7);
    let mut bc7_err = 0u64;
    for i in 0..16 {
        bc7_err += color_distance(false, &pixels[i], &decoded_b7[i], true) as u64;
    }
    Some((uastc_err + bc7_err) / 2)
}

/// Decode failures of the trial blocks are a `continue` in C++, everything else aborts.
enum Trial {
    Skip,
    Fail,
    Err(u64),
}

fn rdo_trial_error(pixels: &Block, trial_blk: &[u8; 16]) -> Trial {
    let mut u = UnpackedUastcBlock::default();
    if !unpack_uastc(trial_blk, &mut u, false, true) {
        return Trial::Skip;
    }
    let mut px = [Color32::default(); 16];
    if !unpack_uastc_pixels_desc(
        u.mode,
        u.common_pattern,
        &u.solid_color,
        &u.astc,
        &mut px,
        false,
    ) {
        return Trial::Skip;
    }
    match rdo_block_error(pixels, &u) {
        Some(e) => Trial::Err(e),
        None => Trial::Fail,
    }
}

/// `uastc_rdo_blocks` statistics: (skipped, refined, modified, smooth).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UastcRdoStats {
    pub total_skipped: u32,
    pub total_refined: u32,
    pub total_modified: u32,
    pub total_smooth: u32,
}

/// `uastc_rdo_blocks` over `blocks` (one job range); `pixels` holds 16 pixels per block.
fn uastc_rdo_blocks(
    blocks: &mut [[u8; 16]],
    pixels: &[ColorRgba],
    params: &UastcRdoParams,
    flags: u32,
    stats: &mut UastcRdoStats,
) -> bool {
    let total_blocks_to_check = (params.lz_dict_size / 16).max(1) as i64;
    let perceptual = false;
    let mut selector_history: std::collections::HashMap<(u32, u64), u32> =
        std::collections::HashMap::new();

    for block_index in 0..blocks.len() {
        let blk = blocks[block_index];
        let mut px: Block = [[0u8; 4]; 16];
        match pixels.get(block_index * 16..block_index * 16 + 16) {
            Some(s) => px.copy_from_slice(s),
            None => return false,
        }

        let mut unpacked_blk = UnpackedUastcBlock::default();
        if !unpack_uastc(&blk, &mut unpacked_blk, false, true) {
            return false;
        }
        let block_mode = unpacked_blk.mode;
        if block_mode == UASTC_MODE_INDEX_SOLID_COLOR {
            continue;
        }

        let max_std_dev = std_dev(&px, 0)
            .max(std_dev(&px, 1))
            .max(std_dev(&px, 2))
            .max(std_dev(&px, 3));
        let mut yl = (max_std_dev / params.max_smooth_block_std_dev).clamp(0.0, 1.0);
        yl *= yl;
        let smooth_block_error_scale =
            params.smooth_block_max_error_scale + (1.0 - params.smooth_block_max_error_scale) * yl;
        if smooth_block_error_scale > 1.0 {
            stats.total_smooth += 1;
        }

        let Some(cur_err) = rdo_block_error(&px, &unpacked_blk) else {
            return false;
        };
        // Divide by 16*4 to compute RMS error
        let cur_ms_err = cur_err as f32 * (1.0 / 64.0);
        let cur_rms_err = cur_ms_err.sqrt();

        let first_sel_bit = UASTC_MODE_SELECTOR_BITS[block_mode as usize][0] as u32;
        let total_sel_bits = UASTC_MODE_SELECTOR_BITS[block_mode as usize][1] as u32;

        let mut cur_bit_offset = first_sel_bit;
        let cur_sel_bits = read_bits(&blk, &mut cur_bit_offset, total_sel_bits.min(64));

        if cur_rms_err >= params.skip_block_rms_thresh {
            // Block already has too much error, so don't mess with it.
            selector_history.insert((first_sel_bit, cur_sel_bits), block_index as u32);
            stats.total_skipped += 1;
            continue;
        }

        let cur_bits: i32 = match selector_history.get(&(first_sel_bit, cur_sel_bits)) {
            // Wasn't found - wildly estimate literal cost
            None => ((total_sel_bits * params.lz_literal_cost) / 100) as i32,
            // Was found - wildly estimate match cost
            Some(&m) => {
                let block_dist_in_bytes = (block_index as u32).wrapping_sub(m).wrapping_mul(16);
                compute_match_cost_estimate(block_dist_in_bytes) as i32
            }
        };

        let first_block_to_check = (block_index as i64 - total_blocks_to_check).max(0);
        let last_block_to_check = block_index as i64 - 1;

        let mut best_block = blk;
        let mut best_block_index = block_index;
        let mut best_t = cur_ms_err * smooth_block_error_scale + cur_bits as f32 * params.lambda;

        // Now scan through previous blocks, insert their selector bit patterns into the current
        // block, and find selector bit patterns which don't increase the overall block error too
        // much.
        let mut prev_block_index = last_block_to_check;
        while prev_block_index >= first_block_to_check {
            let pbi = prev_block_index as usize;
            prev_block_index -= 1;
            let prev_blk = blocks[pbi];

            let mut bit_offset = first_sel_bit;
            let mut sel_bits = read_bits(&prev_blk, &mut bit_offset, total_sel_bits.min(64));

            let match_block_index = match selector_history.get(&(first_sel_bit, sel_bits)) {
                Some(&m) => m as usize,
                None => pbi,
            };
            // Have we already checked this bit pattern? If so then skip this block.
            if match_block_index > pbi {
                continue;
            }

            let mut unpacked_prev_blk = UnpackedUastcBlock::default();
            if !unpack_uastc(&prev_blk, &mut unpacked_prev_blk, false, true) {
                return false;
            }

            let mut trial_blk = blk;
            set_block_bits(
                &mut trial_blk,
                sel_bits,
                total_sel_bits.min(64),
                first_sel_bit,
            );
            if total_sel_bits > 64 {
                sel_bits = read_bits(&prev_blk, &mut bit_offset, total_sel_bits - 64);
                set_block_bits(
                    &mut trial_blk,
                    sel_bits,
                    total_sel_bits - 64,
                    first_sel_bit + total_sel_bits.min(64),
                );
            }

            let trial_err = match rdo_trial_error(&px, &trial_blk) {
                Trial::Skip => continue,
                Trial::Fail => return false,
                Trial::Err(e) => e,
            };
            let trial_ms_err = trial_err as f32 * (1.0 / 64.0);
            let trial_rms_err = trial_ms_err.sqrt();
            if trial_rms_err > cur_rms_err * params.max_allowed_rms_increase_ratio {
                continue;
            }

            let block_dist_in_bytes = ((block_index - match_block_index) * 16) as u32;
            let match_bits = compute_match_cost_estimate(block_dist_in_bytes) as i32;
            let t = trial_ms_err * smooth_block_error_scale + match_bits as f32 * params.lambda;
            if t < best_t {
                best_t = t;
                best_block_index = pbi;
                best_block = trial_blk;
            }
        }

        if best_block_index != block_index {
            stats.total_modified += 1;

            let mut unpacked_best_blk = UnpackedUastcBlock::default();
            if !unpack_uastc(&best_block, &mut unpacked_best_blk, false, false) {
                return false;
            }

            if params.endpoint_refinement && block_mode == 0 {
                // Attempt to refine mode 0 block's endpoints, using the new selectors.
                let mut best_px = [Color32::default(); 16];
                if !unpack_uastc_pixels_desc(
                    unpacked_best_blk.mode,
                    unpacked_best_blk.common_pattern,
                    &unpacked_best_blk.solid_color,
                    &unpacked_best_blk.astc,
                    &mut best_px,
                    false,
                ) {
                    return false;
                }
                let decoded_best = from_c32(&best_px);
                // Compute the block's current error (with the modified selectors).
                let mut best_uastc_err = 0u64;
                for i in 0..16 {
                    best_uastc_err +=
                        color_distance(perceptual, &px[i], &decoded_best[i], true) as u64;
                }

                let comp_params = Bc7encCompressBlockParams {
                    max_partitions_mode1: 64,
                    weights: [1, 1, 1, 1],
                    uber_level: 0,
                    perceptual: false,
                    least_squares_passes: 1,
                };
                let mut results = Vec::with_capacity(1);
                let force = unpacked_best_blk.astc.weights;
                astc_mode_1subset(0, &px, &mut results, &comp_params, Some(&force[..16]));
                let Some(res) = results.first() else {
                    return false;
                };

                // See if the overall error has actually gone done.
                let mut decoded_trial: Block = [[0u8; 4]; 16];
                unpack_results(res, &mut decoded_trial);
                let mut trial_uastc_err = 0u64;
                for i in 0..16 {
                    trial_uastc_err +=
                        color_distance(perceptual, &px[i], &decoded_trial[i], true) as u64;
                }
                if trial_uastc_err < best_uastc_err {
                    // The error went down, so accept the new endpoints.
                    unpacked_best_blk.astc = res.astc;
                    stats.total_refined += 1;
                }
            }

            // The selectors have changed, so go recompute the block hints.
            if !uastc_recompute_hints(&mut best_block, &px, flags, Some(&unpacked_best_blk)) {
                return false;
            }
            // Write the modified block
            blocks[block_index] = best_block;
        }

        let mut bit_offset = first_sel_bit;
        let sel_bits = read_bits(&best_block, &mut bit_offset, total_sel_bits.min(64));
        selector_history.insert((first_sel_bit, sel_bits), block_index as u32);
    }
    true
}

/// `uastc_rdo`: a basic rate distortion optimization post-pass for UASTC. It only changes
/// selectors (copying selector bit patterns of previous blocks) and then updates the hints.
///
/// `pixels` holds 16 source pixels per block. With `total_jobs > 1` the blocks are split
/// into `total_jobs` independent ranges (as the C++ job pool does) processed on scoped threads;
/// the output depends only on the split, not on scheduling.
pub fn uastc_rdo(
    num_blocks: u32,
    blocks: &mut [[u8; 16]],
    pixels: &[ColorRgba],
    params: &UastcRdoParams,
    flags: u32,
    total_jobs: u32,
) -> (bool, UastcRdoStats) {
    let mut stats = UastcRdoStats::default();
    let n = num_blocks as usize;
    if blocks.len() < n || pixels.len() < n * 16 {
        return (false, stats);
    }
    let blocks = &mut blocks[..n];
    let blocks_per_job = num_blocks.checked_div(total_jobs).unwrap_or(0);
    if total_jobs <= 1 || blocks_per_job <= 8 {
        let ok = uastc_rdo_blocks(blocks, pixels, params, flags, &mut stats);
        return (ok, stats);
    }
    let per = blocks_per_job as usize;
    let all = std::thread::scope(|s| {
        let handles: Vec<_> = blocks
            .chunks_mut(per)
            .enumerate()
            .map(|(j, chunk)| {
                let px = &pixels[j * per * 16..];
                s.spawn(move || {
                    let mut st = UastcRdoStats::default();
                    let ok = uastc_rdo_blocks(chunk, px, params, flags, &mut st);
                    (ok, st)
                })
            })
            .collect();
        let mut all = true;
        for h in handles {
            match h.join() {
                Ok((ok, st)) => {
                    all &= ok;
                    stats.total_skipped += st.total_skipped;
                    stats.total_refined += st.total_refined;
                    stats.total_modified += st.total_modified;
                    stats.total_smooth += st.total_smooth;
                }
                Err(_) => all = false,
            }
        }
        all
    });
    (all, stats)
}
