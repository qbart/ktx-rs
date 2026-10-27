// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of the `astc_6x6_hdr` namespace (basisu_transcoder.cpp,
//! basisu_astc_hdr_core.h): the UASTC HDR 6x6 intermediate decoder
//! (`decode_6x6_hdr`) and the fast BC6H encoder (`fast_encode_bc6h`).
#![allow(clippy::needless_range_loop)]

use super::astc_helpers::{
    self as ah, BISE_6_LEVELS, BISE_8_LEVELS, BISE_192_LEVELS, BISE_256_LEVELS, HalfFloat,
    LogAstcBlock, WeightedSample,
};
use super::bc6h::{
    BC6H_FIRST_1SUBSET_MODE_INDEX, Bc6hLogicalBlock, MAX_BC6H_HALF_FLOAT_AS_UINT,
    NUM_MODE7_ENDPOINTS, NUM_MODE11_ENDPOINTS, bc6h_half_to_blog, bc6h_half_to_blog16,
    pack_bc6h_block,
};
use super::bitreader::BitwiseDecoder;
use super::hdr_tables::{
    BC6H_MODE_SIG_BITS, BLOCK_MODE_DESCS, PART2_UNIQUE_INDEX_TO_SEED, PART3_UNIQUE_INDEX_TO_SEED,
    REUSE_XY_DELTAS,
};
use super::uastc_tables::{BC7_PARTITION2, BC7_TABLE_ANCHOR_INDEX_SECOND_SUBSET, BC7_WEIGHTS3};
use std::sync::OnceLock;

pub const MAX_ASTC_HDR_6X6_DIM: u32 = 32768;
/// 1+-(-4), so 5 rows are buffered.
pub const REUSE_MAX_BUFFER_ROWS: i32 = 5;
pub const TOTAL_BLOCK_MODE_DECS: u32 = 75;
/// Original release (v1.6).
pub const UASTC_6X6_HDR_SIG0: u32 = 0xABCD;
/// 2x2->4x4 weight grid upsampling change.
pub const UASTC_6X6_HDR_SIG1: u32 = 0xABCE;
pub const REUSE_XY_DELTA_BITS: u32 = 5;
pub const NUM_ENDPOINT_DELTA_BITS: u32 = 5;
pub const NUM_UNIQUE_PARTITIONS2: u32 = 521;
pub const NUM_UNIQUE_PARTITIONS3: u32 = 333;
/// `endpoint_mode::cTotal`.
const ENDPOINT_MODE_TOTAL: u32 = 5;
const EM_RAW: u32 = 0;
const EM_USE_LEFT: u32 = 1;
const EM_USE_UPPER: u32 = 2;
const EM_USE_LEFT_DELTA: u32 = 3;
const EM_USE_UPPER_DELTA: u32 = 4;

const BLOCK_W: u32 = 6;
const BLOCK_H: u32 = 6;

/// Port of `block_mode_desc`.
#[derive(Debug, Clone, Copy, Default)]
pub struct BlockModeDesc {
    pub dp: bool,
    pub cem: u32,
    pub num_partitions: u32,
    pub grid_x: u32,
    pub grid_y: u32,
    /// The coding ISE ranges (may not be valid ASTC ranges for this configuration).
    pub endpoint_ise_range: u32,
    pub weight_ise_range: u32,
    /// The physical/output ASTC ISE ranges.
    pub transcode_endpoint_ise_range: u32,
    pub transcode_weight_ise_range: u32,
    pub flags: u32,
    pub dp_channel: u32,
}

/// `g_block_mode_descs[index]`.
pub fn block_mode_desc(index: u32) -> Option<BlockModeDesc> {
    BLOCK_MODE_DESCS.get(index as usize).map(|r| BlockModeDesc {
        dp: r[0] != 0,
        cem: u32::from(r[1]),
        num_partitions: u32::from(r[2]),
        grid_x: u32::from(r[3]),
        grid_y: u32::from(r[4]),
        endpoint_ise_range: u32::from(r[5]),
        weight_ise_range: u32::from(r[6]),
        transcode_endpoint_ise_range: u32::from(r[7]),
        transcode_weight_ise_range: u32::from(r[8]),
        flags: u32::from(r[9]),
        dp_channel: u32::from(r[10]),
    })
}

/// Port of `requantize_astc_weights`.
pub fn requantize_astc_weights(
    n: usize,
    src_ise_vals: &[u8],
    from_ise_range: u32,
    dst_ise_vals: &mut [u8],
    to_ise_range: u32,
) {
    if from_ise_range == to_ise_range {
        for (d, s) in dst_ise_vals.iter_mut().zip(src_ise_vals).take(n) {
            *d = *s;
        }
        return;
    }
    let t = ah::dequant_tables();
    let dequant_tab = &t.weight_tab(from_ise_range).ise_to_val;
    let quant_tab = &t.weight_tab(to_ise_range).val_to_ise;
    for (d, s) in dst_ise_vals.iter_mut().zip(src_ise_vals).take(n) {
        let v = dequant_tab.get(*s as usize).copied().unwrap_or(0);
        *d = quant_tab.get(v as usize).copied().unwrap_or(0);
    }
}

/// `g_quantize_tables_preserve2/3`: ISE quantization that keeps the upper 2
/// (or 3) bits of the value.
struct PreserveTables {
    preserve2: [[u8; 256]; 20],
    preserve3: [[u8; 256]; 20],
}

/// Port of `init_quantize_tables`.
fn preserve_tables() -> &'static PreserveTables {
    static T: OnceLock<PreserveTables> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = PreserveTables {
            preserve2: [[0; 256]; 20],
            preserve3: [[0; 256]; 20],
        };
        let best = |ise_to_val: &[u8], desired_val: u32, mask: u32| {
            let mut best_err = u32::MAX;
            let mut best_ise_val = -1i32;
            for (ise_val, &quant_val) in ise_to_val.iter().enumerate() {
                let quant_val = u32::from(quant_val);
                if (quant_val & mask) != (desired_val & mask) {
                    continue;
                }
                let d = quant_val as i32 - desired_val as i32;
                let err = (d * d) as u32;
                if err < best_err {
                    best_err = err;
                    best_ise_val = ise_val as i32;
                }
            }
            best_ise_val as u8
        };
        for ise_range in BISE_6_LEVELS..=BISE_192_LEVELS {
            let ise_to_val = &ah::dequant_tables().endpoint_tab(ise_range).ise_to_val;
            for desired_val in 0..256u32 {
                t.preserve2[ise_range as usize][desired_val as usize] =
                    best(ise_to_val, desired_val, 0b1100_0000);
                if ise_range >= BISE_8_LEVELS {
                    t.preserve3[ise_range as usize][desired_val as usize] =
                        best(ise_to_val, desired_val, 0b1110_0000);
                }
            }
        }
        t
    })
}

#[inline]
fn get_bit(src_val: i32, src_bit: i32) -> i32 {
    (src_val >> src_bit) & 1
}

/// Port of `requantize_ise_endpoints` (`cem` 7 or 11).
pub fn requantize_ise_endpoints(
    cem: u32,
    src_ise_endpoint_range: u32,
    src_endpoints: &[u8],
    dst_ise_endpoint_range: u32,
    dst_endpoints: &mut [u8],
) {
    let n = if cem == 11 {
        NUM_MODE11_ENDPOINTS
    } else {
        NUM_MODE7_ENDPOINTS
    };
    let mut src = [0u8; NUM_MODE11_ENDPOINTS];
    for (d, s) in src.iter_mut().zip(src_endpoints).take(n) {
        *d = *s;
    }
    if src_ise_endpoint_range == dst_ise_endpoint_range {
        for (d, s) in dst_endpoints.iter_mut().zip(src).take(n) {
            *d = s;
        }
        return;
    }
    let t = ah::dequant_tables();
    if src_ise_endpoint_range != BISE_256_LEVELS {
        let dequant = &t.endpoint_tab(src_ise_endpoint_range).ise_to_val;
        for v in src.iter_mut().take(n) {
            *v = dequant.get(*v as usize).copied().unwrap_or(0);
        }
    }
    if dst_ise_endpoint_range == BISE_256_LEVELS {
        for (d, s) in dst_endpoints.iter_mut().zip(src).take(n) {
            *d = s;
        }
        return;
    }
    let quant_tab = &t.endpoint_tab(dst_ise_endpoint_range).val_to_ise;
    let q = |v: u8| quant_tab.get(v as usize).copied().unwrap_or(0);
    let pt = preserve_tables();
    let empty = [0u8; 256];
    let p2 = pt
        .preserve2
        .get(dst_ise_endpoint_range as usize)
        .unwrap_or(&empty);
    let p3 = pt
        .preserve3
        .get(dst_ise_endpoint_range as usize)
        .unwrap_or(&empty);
    // A smarter value quantization that preserves the key upper bits.
    let out: [u8; NUM_MODE11_ENDPOINTS] = if cem == 11 {
        let maj_comp = get_bit(i32::from(src[4]), 7) | (get_bit(i32::from(src[5]), 7) << 1);
        if maj_comp == 3 {
            // Direct; the regular tables preserve the MSB.
            src.map(q)
        } else {
            [
                q(src[0]),
                p2[src[1] as usize],
                p2[src[2] as usize],
                p2[src[3] as usize],
                p3[src[4] as usize],
                p3[src[5] as usize],
            ]
        }
    } else if cem == 7 {
        [
            p2[src[0] as usize],
            p3[src[1] as usize],
            p3[src[2] as usize],
            p3[src[3] as usize],
            0,
            0,
        ]
    } else {
        return;
    };
    for (d, s) in dst_endpoints.iter_mut().zip(out).take(n) {
        *d = s;
    }
}

/// Port of `copy_weight_grid`.
pub fn copy_weight_grid(
    dual_plane: bool,
    grid_x: u32,
    grid_y: u32,
    transcode_weights: &[u8],
    decomp_blk: &mut LogAstcBlock,
    orig_behavior: bool,
) {
    // Special case for 2x2, which isn't valid ASTC without dual plane: upsample to 4x4.
    if !dual_plane && grid_x == 2 && grid_y == 2 {
        decomp_blk.grid_width = 4;
        decomp_blk.grid_height = 4;
        let t = ah::dequant_tables().weight_tab(u32::from(decomp_blk.weight_ise_range));
        let tw = |i: u32| {
            let ise = transcode_weights.get(i as usize).copied().unwrap_or(0);
            u32::from(t.ise_to_val.get(ise as usize).copied().unwrap_or(0))
        };
        let mut weights = [WeightedSample::default(); 16];
        ah::compute_upsample_weights(4, 4, 2, 2, &mut weights);
        for dy in 0..4u32 {
            for dx in 0..4u32 {
                let sample = &weights[(dx + dy * 4) as usize];
                let (sx, sy) = (u32::from(sample.src_x), u32::from(sample.src_y));
                let mut total_weight = 8u32;
                for yo in 0..2u32 {
                    for xo in 0..2u32 {
                        let w = u32::from(sample.weights[yo as usize][xo as usize]);
                        if w == 0 {
                            continue;
                        }
                        // 10/17/2025 bugfix: the original release sampled the 1st
                        // or 2nd weight only (is_in_bounds returns a bool).
                        let index = if orig_behavior {
                            let v = (dx + xo) + (dy + yo) * grid_x;
                            u32::from(v < grid_x * grid_y)
                        } else {
                            (sx + xo) + (sy + yo) * grid_x
                        };
                        total_weight += tw(index) * w;
                    }
                }
                total_weight >>= 4;
                decomp_blk.weights[(dx + dy * 4) as usize] = t
                    .val_to_ise
                    .get(total_weight as usize)
                    .copied()
                    .unwrap_or(0);
            }
        }
    } else {
        let num_planes = if dual_plane { 2 } else { 1 };
        decomp_blk.grid_width = grid_x as u8;
        decomp_blk.grid_height = grid_y as u8;
        let n = (grid_x * grid_y * num_planes) as usize;
        for (d, s) in decomp_blk.weights.iter_mut().zip(transcode_weights).take(n) {
            *d = *s;
        }
    }
}

/// Port of `calc_row_index`: ring buffer row of `prev_y` seen from `cur_y`.
#[inline]
fn calc_row_index(cur_y: i32, prev_y: i32, cur_row_index: i32) -> i32 {
    let mut r = cur_row_index + (prev_y - cur_y);
    if r < 0 {
        r += REUSE_MAX_BUFFER_ROWS;
    }
    r
}

/// Port of `decode_values`: `total_values` BISE values coded as
/// trits/quints first, then the low bits.
pub fn decode_values(
    decoder: &mut BitwiseDecoder,
    total_values: u32,
    ise_range: u32,
    values: &mut [u8],
) -> bool {
    let [ep_bits, ep_trits, ep_quints] = ah::ISE_RANGE_TABLE
        .get(ise_range as usize)
        .copied()
        .unwrap_or([0, 0, 0]);
    let ep_bits = u32::from(ep_bits);
    let (total_tqs, bundle_size, mul) = if ep_trits != 0 {
        (total_values.div_ceil(5), 5u32, 3u32)
    } else if ep_quints != 0 {
        (total_values.div_ceil(3), 3, 5)
    } else {
        (0, 0, 0)
    };
    const MAX_TQ_VALUES: usize = 32;
    if total_tqs as usize > MAX_TQ_VALUES {
        return false;
    }
    let mut tq_values = [0u32; MAX_TQ_VALUES];
    for i in 0..total_tqs {
        let mut num_bits = if ep_trits != 0 { 8 } else { 7 };
        if i == total_tqs - 1 {
            let num_remaining = total_values - (total_tqs - 1) * bundle_size;
            if ep_trits != 0 {
                num_bits = match num_remaining {
                    1 => 2,
                    2 => 4,
                    3 => 5,
                    4 => 7,
                    _ => num_bits,
                };
            } else if ep_quints != 0 {
                num_bits = match num_remaining {
                    1 => 3,
                    2 => 5,
                    _ => num_bits,
                };
            }
        }
        tq_values[i as usize] = decoder.get_bits(num_bits);
    }
    let mut accum = 0u32;
    let mut accum_remaining = 0u32;
    let mut next_tq_index = 0usize;
    for i in 0..total_values as usize {
        let mut value = decoder.get_bits(ep_bits);
        if total_tqs != 0 {
            if accum_remaining == 0 {
                accum = tq_values.get(next_tq_index).copied().unwrap_or(0);
                next_tq_index += 1;
                accum_remaining = bundle_size;
            }
            let v = accum % mul;
            accum /= mul;
            accum_remaining -= 1;
            value |= v << ep_bits;
        }
        if let Some(d) = values.get_mut(i) {
            *d = value as u8;
        }
    }
    true
}

#[inline]
fn get_num_endpoint_vals(cem: u32) -> usize {
    if cem == 11 {
        NUM_MODE11_ENDPOINTS
    } else {
        NUM_MODE7_ENDPOINTS
    }
}

/// Builds the output (physical) logical block for a coded 6x6 block and packs it.
#[allow(clippy::too_many_arguments)]
fn pack_transcoded(
    bmd: &BlockModeDesc,
    num_partitions: u32,
    partition_id: u16,
    endpoint_ise_range: u32,
    endpoints: &[u8],
    weight_ise_range: u32,
    weights: &[u8],
    orig_behavior: bool,
    phys_blk: &mut [u8; 16],
) -> bool {
    let num_endpoint_values = get_num_endpoint_vals(bmd.cem);
    let mut decomp_blk = LogAstcBlock {
        dual_plane: bmd.dp,
        color_component_selector: bmd.dp_channel as u8,
        partition_id,
        num_partitions: num_partitions as u8,
        endpoint_ise_range: bmd.transcode_endpoint_ise_range as u8,
        weight_ise_range: bmd.transcode_weight_ise_range as u8,
        ..LogAstcBlock::default()
    };
    for p in 0..num_partitions as usize {
        if let Some(c) = decomp_blk.color_endpoint_modes.get_mut(p) {
            *c = bmd.cem as u8;
        }
        let o = num_endpoint_values * p;
        let (Some(src), Some(dst)) = (
            endpoints.get(o..),
            decomp_blk.endpoints.get_mut(o..o + num_endpoint_values),
        ) else {
            return false;
        };
        requantize_ise_endpoints(
            bmd.cem,
            endpoint_ise_range,
            src,
            bmd.transcode_endpoint_ise_range,
            dst,
        );
    }
    let total_grid_weights = (bmd.grid_x * bmd.grid_y * if bmd.dp { 2 } else { 1 }) as usize;
    let mut transcode_weights = [0u8; (BLOCK_W * BLOCK_H * 2) as usize];
    requantize_astc_weights(
        total_grid_weights,
        weights,
        weight_ise_range,
        &mut transcode_weights,
        bmd.transcode_weight_ise_range,
    );
    copy_weight_grid(
        bmd.dp,
        bmd.grid_x,
        bmd.grid_y,
        &transcode_weights,
        &mut decomp_blk,
        orig_behavior,
    );
    ah::pack_astc_block(phys_blk, &decomp_blk)
}

/// Output of [`decode_6x6_hdr`]: physical ASTC 6x6 blocks, row major.
#[derive(Debug, Clone, Default)]
pub struct Decoded6x6Hdr {
    pub blocks: Vec<[u8; 16]>,
    pub num_blocks_x: u32,
    pub num_blocks_y: u32,
    pub width: u32,
    pub height: u32,
}

/// Port of `decode_6x6_hdr`: UASTC HDR 6x6 intermediate stream to ASTC 6x6
/// blocks. Returns `None` where C returns false.
pub fn decode_6x6_hdr(comp_data: &[u8]) -> Option<Decoded6x6Hdr> {
    if comp_data.len() <= 2 * 3 + 1 {
        return None;
    }
    let mut decoder = BitwiseDecoder::new(comp_data);
    let orig_behavior = match decoder.get_bits(16) {
        UASTC_6X6_HDR_SIG0 => true,
        UASTC_6X6_HDR_SIG1 => false,
        _ => return None,
    };
    let width = decoder.get_bits(16);
    let height = decoder.get_bits(16);
    if width == 0 || height == 0 || width > MAX_ASTC_HDR_6X6_DIM || height > MAX_ASTC_HDR_6X6_DIM {
        return None;
    }
    let num_blocks_x = width.div_ceil(BLOCK_W);
    let num_blocks_y = height.div_ceil(BLOCK_H);
    let total_blocks = num_blocks_x * num_blocks_y;
    let nbx = num_blocks_x as usize;
    let mut blocks = vec![[0u8; 16]; total_blocks as usize];

    // The decoded (coding) log blocks, NOT the output log blocks: a ring buffer
    // of REUSE_MAX_BUFFER_ROWS rows.
    let mut log_blocks = vec![LogAstcBlock::default(); nbx * REUSE_MAX_BUFFER_ROWS as usize];
    let lb_index = |bx: u32, row: i32| bx as usize + row as usize * nbx;

    let mut cur_bx = 0u32;
    let mut cur_by = 0u32;
    let mut cur_row_index = 0i32;
    let advance = |cur_bx: &mut u32, cur_by: &mut u32, cur_row_index: &mut i32| {
        *cur_bx += 1;
        if *cur_bx == num_blocks_x {
            *cur_bx = 0;
            *cur_by += 1;
            *cur_row_index = (*cur_row_index + 1) % REUSE_MAX_BUFFER_ROWS;
        }
    };

    while cur_by < num_blocks_y {
        if decoder.get_bits_remaining() < 1 {
            return None;
        }
        let cur = lb_index(cur_bx, cur_row_index);
        let phys_index = (cur_bx + cur_by * num_blocks_x) as usize;

        if decoder.get_bits(1) == 0 {
            if decoder.get_bits(1) != 0 {
                // cReuse
                if cur_bx == 0 && cur_by == 0 {
                    return None;
                }
                let reuse_delta_index = decoder.get_bits(REUSE_XY_DELTA_BITS);
                let [dx, dy] = REUSE_XY_DELTAS
                    .get(reuse_delta_index as usize)
                    .copied()
                    .unwrap_or([0, 0]);
                let prev_bx = cur_bx as i32 + i32::from(dx);
                let prev_by = cur_by as i32 + i32::from(dy);
                if prev_bx < 0 || prev_bx >= num_blocks_x as i32 || prev_by < 0 {
                    return None;
                }
                let prev = log_blocks
                    .get(lb_index(
                        prev_bx as u32,
                        calc_row_index(cur_by as i32, prev_by, cur_row_index),
                    ))
                    .copied()?;
                if prev.solid_color_flag_hdr {
                    return None;
                }
                let mut log_blk = prev;
                let total_grid_weights = u32::from(log_blk.grid_width)
                    * u32::from(log_blk.grid_height)
                    * if log_blk.dual_plane { 2 } else { 1 };
                if !decode_values(
                    &mut decoder,
                    total_grid_weights,
                    u32::from(log_blk.weight_ise_range),
                    &mut log_blk.weights,
                ) {
                    return None;
                }
                // bmd's ISE ranges may differ from the logical block's (deltas).
                let bmd = block_mode_desc(u32::from(log_blk.user_mode))?;
                let ok = pack_transcoded(
                    &bmd,
                    bmd.num_partitions,
                    log_blk.partition_id,
                    u32::from(log_blk.endpoint_ise_range),
                    &log_blk.endpoints,
                    u32::from(log_blk.weight_ise_range),
                    &log_blk.weights,
                    orig_behavior,
                    blocks.get_mut(phys_index)?,
                );
                *log_blocks.get_mut(cur)? = log_blk;
                if !ok {
                    return None;
                }
                advance(&mut cur_bx, &mut cur_by, &mut cur_row_index);
            } else if decoder.get_bits(1) != 0 {
                // cSolid
                let rh = decoder.get_bits(15) as HalfFloat;
                let gh = decoder.get_bits(15) as HalfFloat;
                let bh = decoder.get_bits(15) as HalfFloat;
                let log_blk = log_blocks.get_mut(cur)?;
                log_blk.clear();
                log_blk.user_mode = 255;
                log_blk.solid_color_flag_hdr = true;
                log_blk.set_solid_color(0, rh);
                log_blk.set_solid_color(1, gh);
                log_blk.set_solid_color(2, bh);
                log_blk.set_solid_color(3, 0x3C00); // float_to_half(1.0f)
                if !ah::pack_astc_block(blocks.get_mut(phys_index)?, log_blk) {
                    return None;
                }
                advance(&mut cur_bx, &mut cur_by, &mut cur_row_index);
            } else {
                // cRun
                if cur_bx == 0 && cur_by == 0 {
                    return None;
                }
                let run_len = decoder.decode_vlc(5) + 1;
                let num_blocks_remaining = total_blocks - (cur_bx + cur_by * num_blocks_x);
                if run_len > num_blocks_remaining {
                    return None;
                }
                let (prev_bx, prev_by) = if cur_bx != 0 {
                    (cur_bx - 1, cur_by)
                } else {
                    (num_blocks_x - 1, cur_by - 1)
                };
                let prev_log_blk = log_blocks
                    .get(lb_index(
                        prev_bx,
                        calc_row_index(cur_by as i32, prev_by as i32, cur_row_index),
                    ))
                    .copied()?;
                let prev_phys_blk = blocks
                    .get((prev_bx + prev_by * num_blocks_x) as usize)
                    .copied()?;
                for _ in 0..run_len {
                    *log_blocks.get_mut(lb_index(cur_bx, cur_row_index))? = prev_log_blk;
                    *blocks.get_mut((cur_bx + cur_by * num_blocks_x) as usize)? = prev_phys_blk;
                    advance(&mut cur_bx, &mut cur_by, &mut cur_row_index);
                }
            }
            continue;
        }

        // cBlock
        let bm = decoder.decode_truncated_binary(TOTAL_BLOCK_MODE_DECS);
        let em = decoder.decode_truncated_binary(ENDPOINT_MODE_TOTAL);
        let bmd = block_mode_desc(bm)?;
        let num_endpoint_values = get_num_endpoint_vals(bmd.cem);
        let total_grid_weights = bmd.grid_x * bmd.grid_y * if bmd.dp { 2 } else { 1 };
        let mut log_blk = LogAstcBlock {
            user_mode: bm as u8,
            color_component_selector: bmd.dp_channel as u8,
            dual_plane: bmd.dp,
            weight_ise_range: bmd.weight_ise_range as u8,
            grid_width: bmd.grid_x as u8,
            grid_height: bmd.grid_y as u8,
            ..LogAstcBlock::default()
        };
        let num_partitions;

        match em {
            EM_USE_LEFT | EM_USE_UPPER | EM_USE_LEFT_DELTA | EM_USE_UPPER_DELTA => {
                let (mut nbx_, mut nby_) = (cur_bx as i32, cur_by as i32);
                if em == EM_USE_LEFT || em == EM_USE_LEFT_DELTA {
                    nbx_ -= 1;
                } else {
                    nby_ -= 1;
                }
                if nbx_ < 0 || nby_ < 0 {
                    return None;
                }
                let neighbor_blk = log_blocks
                    .get(lb_index(
                        nbx_ as u32,
                        calc_row_index(cur_by as i32, nby_, cur_row_index),
                    ))
                    .copied()?;
                if neighbor_blk.color_endpoint_modes[0] == 0
                    || bmd.cem != u32::from(neighbor_blk.color_endpoint_modes[0])
                {
                    return None;
                }
                num_partitions = 1;
                log_blk.num_partitions = 1;
                log_blk.color_endpoint_modes[0] = bmd.cem as u8;
                if em == EM_USE_LEFT || em == EM_USE_UPPER {
                    // Copies the neighbor's endpoint ISE range (not the mode's) to
                    // avoid more quantization error.
                    log_blk.endpoint_ise_range = neighbor_blk.endpoint_ise_range;
                    log_blk.endpoints[..num_endpoint_values]
                        .copy_from_slice(&neighbor_blk.endpoints[..num_endpoint_values]);
                } else {
                    log_blk.endpoint_ise_range = bmd.endpoint_ise_range as u8;
                    requantize_ise_endpoints(
                        bmd.cem,
                        u32::from(neighbor_blk.endpoint_ise_range),
                        &neighbor_blk.endpoints,
                        bmd.endpoint_ise_range,
                        &mut log_blk.endpoints,
                    );
                    let total_endpoint_delta_vals = 1i32 << NUM_ENDPOINT_DELTA_BITS;
                    let low_delta_limit = -(total_endpoint_delta_vals / 2);
                    let tab = ah::dequant_tables().endpoint_tab(bmd.endpoint_ise_range);
                    let total_endpoint_levels = ah::get_ise_levels(bmd.endpoint_ise_range) as i32;
                    for i in 0..num_endpoint_values {
                        let mut cur_val = i32::from(
                            tab.ise_to_rank
                                .get(log_blk.endpoints[i] as usize)
                                .copied()
                                .unwrap_or(0),
                        );
                        let delta =
                            decoder.get_bits(NUM_ENDPOINT_DELTA_BITS) as i32 + low_delta_limit;
                        cur_val += delta;
                        if cur_val < 0 || cur_val >= total_endpoint_levels {
                            return None;
                        }
                        log_blk.endpoints[i] = tab.rank_to_ise.get(cur_val as usize).copied()?;
                    }
                }
            }
            EM_RAW => {
                num_partitions = bmd.num_partitions;
                log_blk.num_partitions = bmd.num_partitions as u8;
                for p in 0..bmd.num_partitions as usize {
                    *log_blk.color_endpoint_modes.get_mut(p)? = bmd.cem as u8;
                }
                log_blk.endpoint_ise_range = bmd.endpoint_ise_range as u8;
                if bmd.num_partitions == 2 {
                    let i = decoder.decode_truncated_binary(NUM_UNIQUE_PARTITIONS2);
                    log_blk.partition_id = *PART2_UNIQUE_INDEX_TO_SEED.get(i as usize)?;
                } else if bmd.num_partitions == 3 {
                    let i = decoder.decode_truncated_binary(NUM_UNIQUE_PARTITIONS3);
                    log_blk.partition_id = *PART3_UNIQUE_INDEX_TO_SEED.get(i as usize)?;
                }
                if !decode_values(
                    &mut decoder,
                    num_endpoint_values as u32 * bmd.num_partitions,
                    bmd.endpoint_ise_range,
                    &mut log_blk.endpoints,
                ) {
                    return None;
                }
            }
            _ => return None,
        }

        if !decode_values(
            &mut decoder,
            total_grid_weights,
            bmd.weight_ise_range,
            &mut log_blk.weights,
        ) {
            return None;
        }
        let ok = pack_transcoded(
            &bmd,
            num_partitions,
            log_blk.partition_id,
            u32::from(log_blk.endpoint_ise_range),
            &log_blk.endpoints,
            bmd.weight_ise_range,
            &log_blk.weights,
            orig_behavior,
            blocks.get_mut(phys_index)?,
        );
        *log_blocks.get_mut(cur)? = log_blk;
        if !ok {
            return None;
        }
        advance(&mut cur_bx, &mut cur_by, &mut cur_row_index);
    }

    if decoder.get_bits(16) != 0xA742 {
        return None;
    }
    Some(Decoded6x6Hdr {
        blocks,
        num_blocks_x,
        num_blocks_y,
        width,
        height,
    })
}

// ---------------------------------------------------------------------------
// Fast BC6H encoder

pub const BC6H_NUM_DIFF_ENDPOINT_MODES_TO_TRY_2: u32 = 2;
pub const BC6H_NUM_DIFF_ENDPOINT_MODES_TO_TRY_4: u32 = 4;
pub const BC6H_NUM_DIFF_ENDPOINT_MODES_TO_TRY_9: u32 = 9;

/// Port of `fast_bc6h_params`.
#[derive(Debug, Clone, Copy)]
pub struct FastBc6hParams {
    pub num_diff_endpoint_modes_to_try: u32,
    pub max_2subset_pats_to_try: u32,
    pub hq_ls: bool,
    pub brute_force_weight4_assignment: bool,
}

impl Default for FastBc6hParams {
    fn default() -> Self {
        Self {
            hq_ls: true,
            num_diff_endpoint_modes_to_try: BC6H_NUM_DIFF_ENDPOINT_MODES_TO_TRY_2,
            max_2subset_pats_to_try: 1,
            brute_force_weight4_assignment: false,
        }
    }
}

const BC6H_WEIGHTS4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
const MAX_HALF_FLOAT: f32 = 65504.0;
/// 2^-14, the smallest positive normal half.
const MIN_HALF_FLOAT: f32 = f32::from_bits(0x3880_0000);
const MAX_HALF_FLOAT_AS_INT_BITS: u32 = 0x7BFF;
const REALLY_SMALL_FLOAT_VAL: f32 = 0.000_000_125;
const SMALL_FLOAT_VAL: f32 = 0.000_012_5;
const BIG_FLOAT_VAL: f32 = 1e30;
const FAST_BC6H_STD_DEV_THRESH: i64 = 256;
const FAST_BC6H_COMPLEX_STD_DEV_THRESH: i64 = 512;
const FAST_BC6H_VERY_COMPLEX_STD_DEV_THRESH: i64 = 2048;
const BC6H_NUM_PATS: usize = 32;

#[inline]
fn squaref(f: f32) -> f32 {
    f * f
}

/// Port of `fast_float_to_half_no_clamp_neg_nan_or_inf`.
#[inline]
fn fast_float_to_half_no_clamp_neg_nan_or_inf(f: f32) -> u32 {
    // Subtract 112 from the exponent, to change the bias from 127 to 15.
    let fu = (f * f32::from_bits(0x0780_0000)).to_bits();
    let mut h = (fu >> (23 - 10)) & 0x7FFF;
    // round to even
    h += u32::from((fu & 8191) > 4096);
    h.min(MAX_HALF_FLOAT_AS_INT_BITS)
}

/// Port of `ftoh`.
#[inline]
fn ftoh(f: f32) -> f32 {
    fast_float_to_half_no_clamp_neg_nan_or_inf(f.abs()) as f32 * if f < 0.0 { -1.0 } else { 1.0 }
}

/// Port of `fast_half_to_float_pos_not_inf_or_nan`.
#[inline]
fn fast_half_to_float_pos_not_inf_or_nan(h: u32) -> f32 {
    // add 112 to the exponent (112+half float's exp bias of 15=float32's bias of 127)
    f32::from_bits(h << 13) * f32::from_bits(0x7780_0000)
}

/// Port of `inv_sqrt` (constants spelled as in C).
#[inline]
#[allow(clippy::excessive_precision)]
fn inv_sqrt(v: f32) -> f32 {
    let f = f32::from_bits(0x5F1F_FFF9u32.wrapping_sub(v.to_bits() >> 1));
    0.703952253 * f * (2.38924456 - v * (f * f))
}

/// The six covariance products of a centered pixel, with C's int wrap.
#[inline]
fn cov_terms(r: i32, g: i32, b: i32) -> [i32; 6] {
    [
        r.wrapping_mul(r),
        r.wrapping_mul(g),
        r.wrapping_mul(b),
        g.wrapping_mul(g),
        g.wrapping_mul(b),
        b.wrapping_mul(b),
    ]
}

struct Bc6hEncTables {
    /// `g_bc6h_ls_weights_3/4`: w*w, (1-w)*w, (1-w)*(1-w), w.
    ls_weights_3: [[f32; 4]; 8],
    ls_weights_4: [[f32; 4]; 16],
    pats2: [u32; BC6H_NUM_PATS],
}

/// Port of `fast_encode_bc6h_init`.
fn enc_tables() -> &'static Bc6hEncTables {
    static T: OnceLock<Bc6hEncTables> = OnceLock::new();
    T.get_or_init(|| {
        let ls = |w: f32| [w * w, (1.0 - w) * w, (1.0 - w) * (1.0 - w), w];
        let mut t = Bc6hEncTables {
            ls_weights_3: [[0.0; 4]; 8],
            ls_weights_4: [[0.0; 4]; 16],
            pats2: [0; BC6H_NUM_PATS],
        };
        for i in 0..8 {
            t.ls_weights_3[i] = ls(BC7_WEIGHTS3[i] as f32 * (1.0 / 64.0));
        }
        for i in 0..16 {
            t.ls_weights_4[i] = ls(BC6H_WEIGHTS4[i] as f32 * (1.0 / 64.0));
        }
        for (pat_index, bits) in t.pats2.iter_mut().enumerate() {
            for j in 0..16 {
                *bits |= u32::from(BC7_PARTITION2[pat_index * 16 + j]) << j;
            }
        }
        t
    })
}

#[inline]
fn interp_half(min: u32, max: u32, w: u32) -> f32 {
    fast_half_to_float_pos_not_inf_or_nan(((min * (64 - w) + max * w + 32) >> 6) & 0xFFFF)
}

/// Port of `assign_weights_4`.
#[allow(clippy::too_many_arguments)]
fn assign_weights_4(
    float_pixels: &[[f32; 3]; 16],
    pixel_scales: &[f32; 16],
    weights: &mut [u8; 16],
    min: [u32; 3],
    max: [u32; 3],
    block_max_var: i64,
    try_2subsets_flag: bool,
    params: &FastBc6hParams,
) -> f64 {
    let mut cr = [0f32; 16];
    let mut cg = [0f32; 16];
    let mut cb = [0f32; 16];
    for i in 0..16 {
        let w = BC6H_WEIGHTS4[i];
        cr[i] = interp_half(min[0], max[0], w);
        cg[i] = interp_half(min[1], max[1], w);
        cb[i] = interp_half(min[2], max[2], w);
    }
    let err_at = |q: &[f32; 3], j: usize| {
        squaref(q[0] - cr[j]) + squaref(q[1] - cg[j]) + squaref(q[2] - cb[j])
    };

    let mut total_err = 0.0f64;
    if params.brute_force_weight4_assignment {
        for i in 0..16 {
            let [qr, qg, qb] = float_pixels[i];
            let mut best_err = squaref(cr[0] - qr) + squaref(cg[0] - qg) + squaref(cb[0] - qb);
            let mut best_idx = 0;
            for j in 1..16 {
                let (rd, gd, bd) = (cr[j] - qr, cg[j] - qg, cb[j] - qb);
                let e = rd * rd + gd * gd + bd * bd;
                if e < best_err {
                    best_err = e;
                    best_idx = j;
                }
            }
            weights[i] = best_idx as u8;
            total_err += f64::from(best_err * pixel_scales[i]);
        }
        return total_err;
    }

    let (dir_r, dir_g, dir_b) = (cr[15] - cr[0], cg[15] - cg[0], cb[15] - cb[0]);
    let mut dots = [0f32; 16];
    for i in 0..16 {
        dots[i] = cr[i] * dir_r + cg[i] * dir_g + cb[i] * dir_b;
    }
    let mut mid_dots = [0f32; 15];
    let mut monotonically_increasing = true;
    for i in 0..15 {
        mid_dots[i] = (dots[i] + dots[i + 1]) * 0.5;
        if dots[i] > dots[i + 1] {
            monotonically_increasing = false;
        }
    }
    let check_more_colors = block_max_var
        > FAST_BC6H_VERY_COMPLEX_STD_DEV_THRESH * FAST_BC6H_VERY_COMPLEX_STD_DEV_THRESH * 16;
    let search = |d: f32| {
        let mut low = 0usize;
        for step in [7, 3, 1, 0] {
            let mid = low + step;
            if d >= mid_dots[mid] {
                low = mid + 1;
            }
        }
        low
    };

    if !monotonically_increasing {
        // Seems very rare, not worth optimizing the other cases
        for i in 0..16 {
            let q = &float_pixels[i];
            let d = q[0] * dir_r + q[1] * dir_g + q[2] * dir_b;
            let mut best_e = (d - dots[0]).abs();
            let mut best_idx = 0;
            for j in 1..16 {
                let e = (d - dots[j]).abs();
                if e < best_e {
                    best_e = e;
                    best_idx = j;
                }
            }
            weights[i] = best_idx as u8;
            total_err += f64::from(err_at(q, best_idx) * pixel_scales[i]);
        }
    } else if !try_2subsets_flag || !check_more_colors {
        for i in 0..16 {
            let q = &float_pixels[i];
            let best_idx = search(q[0] * dir_r + q[1] * dir_g + q[2] * dir_b);
            weights[i] = best_idx as u8;
            // Giesen's MRSSE (Mean Relative Sum of Squared Errors).
            total_err += f64::from(err_at(q, best_idx) * pixel_scales[i]);
        }
    } else {
        for i in 0..16 {
            let q = &float_pixels[i];
            let mut best_idx = search(q[0] * dir_r + q[1] * dir_g + q[2] * dir_b);
            let mut err = err_at(q, best_idx);
            let alt_idx = if best_idx + 1 > 15 { 13 } else { best_idx + 1 };
            let alt_err = err_at(q, alt_idx);
            if alt_err < err {
                err = alt_err;
                best_idx = alt_idx;
            }
            let alt_idx2 = if best_idx == 0 { 2 } else { best_idx - 1 };
            let alt_err2 = err_at(q, alt_idx2);
            if alt_err2 < err {
                err = alt_err2;
                best_idx = alt_idx2;
            }
            weights[i] = best_idx as u8;
            total_err += f64::from(err * pixel_scales[i]);
        }
    }
    total_err
}

fn float_pixels_and_scales(pixels: &[HalfFloat; 48]) -> ([[f32; 3]; 16], [f32; 16]) {
    let mut float_pixels = [[0f32; 3]; 16];
    let mut pixel_scales = [0f32; 16];
    for i in 0..16 {
        for c in 0..3 {
            float_pixels[i][c] =
                fast_half_to_float_pos_not_inf_or_nan(u32::from(pixels[i * 3 + c]));
        }
        let [r, g, b] = float_pixels[i];
        pixel_scales[i] = 1.0 / (squaref(r) + squaref(g) + squaref(b) + MIN_HALF_FLOAT);
    }
    (float_pixels, pixel_scales)
}

/// Port of `assign_weights_simple_4`.
fn assign_weights_simple_4(
    pixels: &[HalfFloat; 48],
    weights: &mut [u8; 16],
    min: [u32; 3],
    max: [u32; 3],
    block_max_var: i64,
    params: &FastBc6hParams,
) {
    let fmin = min.map(|v| fast_half_to_float_pos_not_inf_or_nan(v & 0xFFFF));
    let fmax = max.map(|v| fast_half_to_float_pos_not_inf_or_nan(v & 0xFFFF));
    let mut fdir_r = fmax[0] - fmin[0];
    let mut fdir_g = fmax[1] - fmin[1];
    let mut fdir_b = fmax[2] - fmin[2];
    let l = inv_sqrt(fdir_r * fdir_r + fdir_g * fdir_g + fdir_b * fdir_b);
    if l != 0.0 {
        fdir_r *= l;
        fdir_g *= l;
        fdir_b *= l;
    }
    let lf = fmin[0] * fdir_r + fmin[1] * fdir_g + fmin[2] * fdir_b;
    let hf = fmax[0] * fdir_r + fmax[1] * fdir_g + fmax[2] * fdir_b;

    if lf >= MAX_HALF_FLOAT || hf >= MAX_HALF_FLOAT {
        // v2.1: can't use the faster half float based tricks below.
        let (float_pixels, pixel_scales) = float_pixels_and_scales(pixels);
        assign_weights_4(
            &float_pixels,
            &pixel_scales,
            weights,
            min,
            max,
            block_max_var,
            false,
            params,
        );
        return;
    }

    let mut lr = ftoh(lf);
    let hr = ftoh(hf);
    let frr = if hr == lr { 0.0 } else { 14.93333 / (hr - lr) };
    lr = (-lr * frr) + 0.53333;
    for i in 0..16 {
        let r = fast_half_to_float_pos_not_inf_or_nan(u32::from(pixels[i * 3]));
        let g = fast_half_to_float_pos_not_inf_or_nan(u32::from(pixels[i * 3 + 1]));
        let b = fast_half_to_float_pos_not_inf_or_nan(u32::from(pixels[i * 3 + 2]));
        let d = r * fdir_r + g * fdir_g + b * fdir_b;
        let w = ftoh(if d < MAX_HALF_FLOAT {
            d
        } else {
            MAX_HALF_FLOAT
        });
        weights[i] = ((w * frr + lr) as i32).clamp(0, 15) as u8;
    }
}

/// Subset endpoint candidates, `[subset]` per channel.
#[derive(Clone, Copy, Default)]
struct SubsetEndpoints {
    min: [[u32; 3]; 2],
    max: [[u32; 3]; 2],
}

/// Shared body of `assign_weights3` and `assign_weights_error_3`.
fn assign_weights3_err(
    trial_weights: &mut [u8; 16],
    best_pat_bits: u32,
    e: &SubsetEndpoints,
    float_pixels: &[[f32; 3]; 16],
    pixel_scales: Option<&[f32; 16]>,
) -> f64 {
    let mut subset_c = [[[0f32; 8]; 3]; 2]; // [subset][comp][j]
    for subset in 0..2 {
        for j in 0..8 {
            let w = BC7_WEIGHTS3[j];
            for c in 0..3 {
                subset_c[subset][c][j] = interp_half(e.min[subset][c], e.max[subset][c], w);
            }
        }
    }
    let mut trial_error = 0.0f64;
    for i in 0..16 {
        let sc = &subset_c[((best_pat_bits >> i) & 1) as usize];
        let [qr, qg, qb] = float_pixels[i];
        let mut best_error =
            squaref(sc[0][0] - qr) + squaref(sc[1][0] - qg) + squaref(sc[2][0] - qb);
        let mut best_idx = 0;
        for j in 1..8 {
            let err = squaref(sc[0][j] - qr) + squaref(sc[1][j] - qg) + squaref(sc[2][j] - qb);
            if err < best_error {
                best_error = err;
                best_idx = j;
            }
        }
        trial_weights[i] = best_idx as u8;
        if let Some(scales) = pixel_scales {
            trial_error += f64::from(best_error * scales[i]);
        }
    }
    trial_error
}

/// Port of `bc6h_dequantize`.
fn bc6h_dequantize(val: i32, bits: i32) -> i32 {
    if bits >= 15 {
        val
    } else if val == 0 {
        0
    } else if val == (1 << bits) - 1 {
        0xFFFF
    } else {
        ((val << 16) + 0x8000) >> bits
    }
}

/// Port of `bc6h_convert_to_half` (scale by 31/64).
#[inline]
fn bc6h_convert_to_half(val: i32) -> u32 {
    (((val * 31) >> 6) as u32) & 0xFFFF
}

#[inline]
fn blog(h: u32, bits: u32) -> u32 {
    bc6h_half_to_blog((h & 0xFFFF) as HalfFloat, bits)
}

#[inline]
fn quant_dequant(v: &mut u32, bits: u32) {
    *v = bc6h_convert_to_half(bc6h_dequantize(blog(*v, bits) as i32, bits as i32));
}

/// Port of `bc6h_quant_dequant_endpoints`.
fn bc6h_quant_dequant_endpoints(min: &mut [u32; 3], max: &mut [u32; 3], bits: u32) {
    min.iter_mut().for_each(|v| quant_dequant(v, bits));
    max.iter_mut().for_each(|v| quant_dequant(v, bits));
}

#[inline]
fn fast_roundf_int(x: f32) -> i32 {
    if x >= 0.0 {
        (x + 0.5) as i32
    } else {
        (x - 0.5) as i32
    }
}

/// Port of `fast_encode_bc6h_2subsets_pattern`.
#[allow(clippy::too_many_arguments)]
fn fast_encode_bc6h_2subsets_pattern(
    best_pat_index: u32,
    best_pat_bits: u32,
    pixels: &[HalfFloat; 48],
    float_pixels: &[[f32; 3]; 16],
    pixel_scales: &[f32; 16],
    cur_error: &mut f64,
    log_blk: &mut Bc6hLogicalBlock,
    mean: [i32; 3],
    params: &FastBc6hParams,
) {
    let px = |i: usize, c: usize| u32::from(pixels[i * 3 + c]);
    let subset_of = |i: usize| ((best_pat_bits >> i) & 1) as usize;

    let mut subset_icov = [[0i64; 6]; 2];
    for i in 0..16 {
        let s = subset_of(i);
        let r = px(i, 0) as i32 - mean[0];
        let g = px(i, 1) as i32 - mean[1];
        let b = px(i, 2) as i32 - mean[2];
        for (k, v) in cov_terms(r, g, b).into_iter().enumerate() {
            subset_icov[s][k] += i64::from(v);
        }
    }

    let mut subset_axis = [[0f32; 3]; 2];
    for s in 0..2 {
        let cov = subset_icov[s].map(|v| v as f32);
        let sc = 1.0 / (cov[0].max_c(cov[3]).max_c(cov[5]) + REALLY_SMALL_FLOAT_VAL);
        let (wx, wy, wz) = (sc * cov[0], sc * cov[3], sc * cov[5]);
        let alt_xr = cov[0] * wx + cov[1] * wy + cov[2] * wz;
        let alt_xg = cov[1] * wx + cov[3] * wy + cov[4] * wz;
        let alt_xb = cov[2] * wx + cov[4] * wy + cov[5] * wz;
        let l = squaref(alt_xr) + squaref(alt_xg) + squaref(alt_xb);
        let mut axis = [0.577_350_27f32; 3];
        if l.abs() >= SMALL_FLOAT_VAL {
            let inv_l = inv_sqrt(l);
            axis = [alt_xr * inv_l, alt_xg * inv_l, alt_xb * inv_l];
        }
        subset_axis[s] = axis;
    }

    let mut subset_min_dot = [BIG_FLOAT_VAL; 2];
    let mut subset_max_dot = [-BIG_FLOAT_VAL; 2];
    let mut subset_min_idx = [0usize; 2];
    let mut subset_max_idx = [0usize; 2];
    for i in 0..16 {
        let s = subset_of(i);
        let a = subset_axis[s];
        let dot = px(i, 0) as f32 * a[0] + px(i, 1) as f32 * a[1] + px(i, 2) as f32 * a[2];
        if dot < subset_min_dot[s] {
            subset_min_dot[s] = dot;
            subset_min_idx[s] = i;
        }
        if dot > subset_max_dot[s] {
            subset_max_dot[s] = dot;
            subset_max_idx[s] = i;
        }
    }

    let mut e = SubsetEndpoints::default();
    for s in 0..2 {
        for c in 0..3 {
            e.min[s][c] = px(subset_min_idx[s], c);
            e.max[s][c] = px(subset_max_idx[s], c);
        }
    }

    // least squares with unquantized endpoints
    {
        let ls3 = &enc_tables().ls_weights_3;
        let mut trial_weights = [0u8; 16];
        assign_weights3_err(&mut trial_weights, best_pat_bits, &e, float_pixels, None);

        let mut z00 = [0f32; 2];
        let mut z10 = [0f32; 2];
        let mut z11 = [0f32; 2];
        let mut q00 = [[0f32; 3]; 2];
        let mut t = [[0f32; 3]; 2];
        for i in 0..16 {
            let s = subset_of(i);
            let sel = trial_weights[i] as usize;
            z00[s] += ls3[sel][0];
            z10[s] += ls3[sel][1];
            z11[s] += ls3[sel][2];
            let w = ls3[sel][3];
            for c in 0..3 {
                let v = px(i, c) as f32;
                q00[s][c] += w * v;
                t[s][c] += v;
            }
        }
        for s in 0..2 {
            let z01 = z10[s];
            let mut det = z00[s] * z11[s] - z01 * z10[s];
            if det.abs() >= SMALL_FLOAT_VAL {
                det = 1.0 / det;
                let iz00 = z11[s] * det;
                let iz01 = -z01 * det;
                let iz10 = -z10[s] * det;
                let iz11 = z00[s] * det;
                for c in 0..3 {
                    let q10 = t[s][c] - q00[s][c];
                    let clamp = |x: f32| {
                        fast_roundf_int(x).clamp(0, MAX_BC6H_HALF_FLOAT_AS_UINT as i32) as u32
                    };
                    e.max[s][c] = clamp(iz00 * q00[s][c] + iz01 * q10);
                    e.min[s][c] = clamp(iz10 * q00[s][c] + iz11 * q10);
                }
            }
        }
    }

    const BC6H_2SUBSET_ABS_ENDPOINT_MODE: u32 = 9;
    let mut bc6h_mode_index = BC6H_2SUBSET_ABS_ENDPOINT_MODE;
    let mut num_endpoint_bits = 6u32;
    let mut abs_blog_endpoints = [[0u32; 4]; 3];
    let quant = |abs: &mut [[u32; 4]; 3], e: &SubsetEndpoints, bits: u32| {
        for s in 0..2 {
            for c in 0..3 {
                abs[c][s * 2] = blog(e.min[s][c], bits);
                abs[c][s * 2 + 1] = blog(e.max[s][c], bits);
            }
        }
    };

    if params.num_diff_endpoint_modes_to_try != 0 {
        // ordered from largest base bits to least
        const ORDER2: [u32; 2] = [5, 1];
        const ORDER4: [u32; 4] = [0, 5, 7, 1];
        const ORDER9: [u32; 9] = [2, 3, 4, 0, 5, 6, 7, 8, 1];
        let order: &[u32] = if params.num_diff_endpoint_modes_to_try >= 9 {
            &ORDER9
        } else if params.num_diff_endpoint_modes_to_try >= 4 {
            &ORDER4
        } else {
            &ORDER2
        };
        // Find the BC6H mode that will conservatively encode our trial endpoints.
        for &mode in order {
            let sig = BC6H_MODE_SIG_BITS[mode as usize];
            let num_base_bits = u32::from(sig[0]);
            quant(&mut abs_blog_endpoints, &e, num_base_bits);
            let mut c = 0;
            while c < 3 {
                // Symmetric limit, so endpoints can be inverted later if needed.
                let max_delta = (1i32 << (sig[c + 1] - 1)) - 1;
                let min_delta = -max_delta;
                let a = abs_blog_endpoints[c].map(|v| v as i32);
                let deltas = [
                    a[1] - a[0],
                    a[2] - a[0],
                    a[3] - a[0],
                    a[2] - a[1],
                    a[3] - a[1],
                ];
                if deltas.iter().any(|&d| d < min_delta || d > max_delta) {
                    break;
                }
                c += 1;
            }
            if c == 3 {
                bc6h_mode_index = mode;
                num_endpoint_bits = num_base_bits;
                break;
            }
        }
    }

    if bc6h_mode_index == BC6H_2SUBSET_ABS_ENDPOINT_MODE {
        quant(&mut abs_blog_endpoints, &e, num_endpoint_bits);
    }

    for s in 0..2 {
        for c in 0..3 {
            let deq =
                |v: u32| bc6h_convert_to_half(bc6h_dequantize(v as i32, num_endpoint_bits as i32));
            e.min[s][c] = deq(abs_blog_endpoints[c][s * 2]);
            e.max[s][c] = deq(abs_blog_endpoints[c][s * 2 + 1]);
        }
    }

    let mut trial_weights = [0u8; 16];
    let trial_error = assign_weights3_err(
        &mut trial_weights,
        best_pat_bits,
        &e,
        float_pixels,
        Some(pixel_scales),
    );

    if trial_error < *cur_error {
        let mut trial_log_blk = Bc6hLogicalBlock {
            mode: bc6h_mode_index,
            partition_pattern: best_pat_index,
            endpoints: abs_blog_endpoints,
            weights: trial_weights,
        };
        if trial_log_blk.weights[0] & 4 != 0 {
            for c in 0..3 {
                trial_log_blk.endpoints[c].swap(0, 1);
            }
            for i in 0..16 {
                if subset_of(i) == 0 {
                    trial_log_blk.weights[i] = 7 - trial_log_blk.weights[i];
                }
            }
        }
        let anchor = BC7_TABLE_ANCHOR_INDEX_SECOND_SUBSET[best_pat_index as usize] as usize;
        if trial_log_blk.weights[anchor] & 4 != 0 {
            for c in 0..3 {
                trial_log_blk.endpoints[c].swap(2, 3);
            }
            for i in 0..16 {
                if subset_of(i) == 1 {
                    trial_log_blk.weights[i] = 7 - trial_log_blk.weights[i];
                }
            }
        }
        if bc6h_mode_index != BC6H_2SUBSET_ABS_ENDPOINT_MODE {
            let sig = BC6H_MODE_SIG_BITS[bc6h_mode_index as usize];
            for c in 0..3 {
                let mask = (1i32 << sig[c + 1]) - 1;
                let ep = trial_log_blk.endpoints[c].map(|v| v as i32);
                for k in 1..4 {
                    trial_log_blk.endpoints[c][k] = ((ep[k] - ep[0]) & mask) as u32;
                }
            }
        }
        *cur_error = trial_error;
        *log_blk = trial_log_blk;
    }
}

/// `basisu::maximum` (first argument wins ties, NaN-agnostic like C).
trait MaxC {
    fn max_c(self, b: Self) -> Self;
}

impl MaxC for f32 {
    #[inline]
    fn max_c(self, b: f32) -> f32 {
        if self > b { self } else { b }
    }
}

/// Port of `fast_encode_bc6h_2subsets`.
#[allow(clippy::too_many_arguments)]
fn fast_encode_bc6h_2subsets(
    pixels: &[HalfFloat; 48],
    float_pixels: &[[f32; 3]; 16],
    pixel_scales: &[f32; 16],
    cur_error: &mut f64,
    log_blk: &mut Bc6hLogicalBlock,
    mean: [i32; 3],
    block_axis: [f32; 3],
    params: &FastBc6hParams,
) {
    let pats2 = &enc_tables().pats2;
    let mut encode = |pat: usize| {
        fast_encode_bc6h_2subsets_pattern(
            pat as u32,
            pats2[pat],
            pixels,
            float_pixels,
            pixel_scales,
            cur_error,
            log_blk,
            mean,
            params,
        );
    };
    if params.max_2subset_pats_to_try as usize == BC6H_NUM_PATS {
        for i in 0..BC6H_NUM_PATS {
            encode(i);
        }
        return;
    }

    let mut desired_pat_bits = 0u32;
    for i in 0..16 {
        let f = (i32::from(pixels[i * 3]) - mean[0]) as f32 * block_axis[0]
            + (i32::from(pixels[i * 3 + 1]) - mean[1]) as f32 * block_axis[1]
            + (i32::from(pixels[i * 3 + 2]) - mean[2]) as f32 * block_axis[2];
        desired_pat_bits |= u32::from(f >= 0.0) << i;
    }
    let mut pat_diffs = [0u32; BC6H_NUM_PATS];
    for p in 0..BC6H_NUM_PATS {
        let diff = (pats2[p] ^ desired_pat_bits).count_ones();
        pat_diffs[p] = (diff.min(16 - diff) << 8) | p as u32;
    }
    if params.max_2subset_pats_to_try == 1 {
        let best_diff = pat_diffs.iter().copied().min().unwrap_or(0);
        encode((best_diff & 0xFF) as usize);
    } else {
        pat_diffs.sort_unstable();
        for d in pat_diffs
            .iter()
            .take(params.max_2subset_pats_to_try as usize)
        {
            encode((d & 0xFF) as usize);
        }
    }
}

/// Port of `fast_encode_bc6h`: 16 RGB half floats (raster order) to an
/// unsigned BC6H block.
pub fn fast_encode_bc6h(pixels: &[HalfFloat; 48], block: &mut [u8; 16], params: &FastBc6hParams) {
    let mut log_blk = Bc6hLogicalBlock {
        mode: BC6H_FIRST_1SUBSET_MODE_INDEX,
        ..Bc6hLogicalBlock::default()
    };
    let px = |i: usize, c: usize| u32::from(pixels[i * 3 + c]);

    let mut omin = [u32::MAX; 3];
    let mut omax = [0u32; 3];
    let mut total = [0u32; 3];
    for i in 0..16 {
        for c in 0..3 {
            let v = px(i, c);
            total[c] += v;
            omin[c] = omin[c].min(v);
            omax[c] = omax[c].max(v);
        }
    }

    if omin == omax {
        // Solid block
        for c in 0..3 {
            log_blk.endpoints[c][0] = bc6h_half_to_blog16(omin[c] as HalfFloat);
            log_blk.endpoints[c][1] = 0;
        }
        log_blk.mode = 13;
        pack_bc6h_block(block, &log_blk);
        return;
    }

    let mean = total.map(|t| ((t + 8) / 16) as i32);
    let mut icov = [0i64; 6];
    for i in 0..16 {
        let r = px(i, 0) as i32 - mean[0];
        let g = px(i, 1) as i32 - mean[1];
        let b = px(i, 2) as i32 - mean[2];
        for (k, v) in cov_terms(r, g, b).into_iter().enumerate() {
            icov[k] += i64::from(v);
        }
    }
    // not divided by 16, i.e. scaled by 16
    let block_max_var = icov[0].max(icov[3]).max(icov[5]);

    let set_endpoints = |log_blk: &mut Bc6hLogicalBlock, min: [u32; 3], max: [u32; 3]| {
        for c in 0..3 {
            log_blk.endpoints[c][0] = blog(min[c], 10);
            log_blk.endpoints[c][1] = blog(max[c], 10);
        }
        if log_blk.weights[0] & 8 != 0 {
            for w in log_blk.weights.iter_mut() {
                *w = 15 - *w;
            }
            for c in 0..3 {
                log_blk.endpoints[c].swap(0, 1);
            }
        }
    };

    if block_max_var < FAST_BC6H_STD_DEV_THRESH * FAST_BC6H_STD_DEV_THRESH * 16 {
        // Simple block
        let mut min = [0u32; 3];
        let mut max = [0u32; 3];
        for c in 0..3 {
            min[c] = (omax[c] - omin[c]) / 32 + omin[c];
            max[c] = ((omax[c] - omin[c]) * 31) / 32 + omin[c];
        }
        bc6h_quant_dequant_endpoints(&mut min, &mut max, 10);
        assign_weights_simple_4(
            pixels,
            &mut log_blk.weights,
            min,
            max,
            block_max_var,
            params,
        );
        set_endpoints(&mut log_blk, min, max);
        pack_bc6h_block(block, &log_blk);
        return;
    }

    // Complex block (edges/strong gradients)
    let cov = icov.map(|v| v as f32);
    let sc = 1.0 / block_max_var as f32;
    let (wx, wy, wz) = (sc * cov[0], sc * cov[3], sc * cov[5]);
    let alt_xr = cov[0] * wx + cov[1] * wy + cov[2] * wz;
    let alt_xg = cov[1] * wx + cov[3] * wy + cov[4] * wz;
    let alt_xb = cov[2] * wx + cov[4] * wy + cov[5] * wz;
    let l = squaref(alt_xr) + squaref(alt_xg) + squaref(alt_xb);
    let (mut axis_r, mut axis_g, mut axis_b) = (0.577_350_27f32, 0.577_350_27f32, 0.577_350_27f32);
    if l.abs() >= SMALL_FLOAT_VAL {
        let inv_l = inv_sqrt(l);
        axis_r = alt_xr * inv_l;
        axis_g = alt_xg * inv_l;
        axis_b = alt_xb * inv_l;
    }
    let tr = axis_r * cov[0] + axis_g * cov[1] + axis_b * cov[2];
    let tg = axis_r * cov[1] + axis_g * cov[3] + axis_b * cov[4];
    let tb = axis_r * cov[2] + axis_g * cov[4] + axis_b * cov[5];
    let principle_axis_var = tr * axis_r + tg * axis_g + tb * axis_b;
    let inv_principle_axis_var = 1.0 / (principle_axis_var + REALLY_SMALL_FLOAT_VAL);
    axis_r = tr * inv_principle_axis_var;
    axis_g = tg * inv_principle_axis_var;
    axis_b = tb * inv_principle_axis_var;
    let total_var = cov[0] + cov[3] + cov[5];
    // A "very complex" block (principle axis variance below this fraction of the
    // total) may benefit from 2 subsets.
    const COMPLEX_BLOCK_PRINCIPLE_AXIS_FRACT_THRESH: f32 = 0.995;
    let try_2subsets = principle_axis_var < total_var * COMPLEX_BLOCK_PRINCIPLE_AXIS_FRACT_THRESH;

    let (float_pixels, pixel_scales) = float_pixels_and_scales(pixels);
    let mut min_idx = 0;
    let mut max_idx = 0;
    let mut min_dot = BIG_FLOAT_VAL;
    let mut max_dot = -BIG_FLOAT_VAL;
    for i in 0..16 {
        let dot = px(i, 0) as f32 * axis_r + px(i, 1) as f32 * axis_g + px(i, 2) as f32 * axis_b;
        if dot < min_dot {
            min_dot = dot;
            min_idx = i;
        }
        if dot > max_dot {
            max_dot = dot;
            max_idx = i;
        }
    }
    let mut min = [px(min_idx, 0), px(min_idx, 1), px(min_idx, 2)];
    let mut max = [px(max_idx, 0), px(max_idx, 1), px(max_idx, 2)];
    bc6h_quant_dequant_endpoints(&mut min, &mut max, 10);

    let mut cur_err = assign_weights_4(
        &float_pixels,
        &pixel_scales,
        &mut log_blk.weights,
        min,
        max,
        block_max_var,
        try_2subsets,
        params,
    );

    let ls4 = &enc_tables().ls_weights_4;
    let max_ls_passes = if params.hq_ls { 2 } else { 1 };
    for _ in 0..max_ls_passes {
        let (mut z00, mut z10, mut z11) = (0f32, 0f32, 0f32);
        let mut q00 = [0f32; 3];
        let mut t = [0f32; 3];
        for i in 0..16 {
            let sel = log_blk.weights[i] as usize;
            z00 += ls4[sel][0];
            z10 += ls4[sel][1];
            z11 += ls4[sel][2];
            let w = ls4[sel][3];
            for c in 0..3 {
                let v = px(i, c) as f32;
                q00[c] += w * v;
                t[c] += v;
            }
        }
        let z01 = z10;
        let mut det = z00 * z11 - z01 * z10;
        if det.abs() < SMALL_FLOAT_VAL {
            break;
        }
        det = 1.0 / det;
        let iz00 = z11 * det;
        let iz01 = -z01 * det;
        let iz10 = -z10 * det;
        let iz11 = z00 * det;
        let mut trial_min = [0u32; 3];
        let mut trial_max = [0u32; 3];
        for c in 0..3 {
            let q10 = t[c] - q00[c];
            let clamp =
                |x: f32| x.round().clamp(0.0, MAX_BC6H_HALF_FLOAT_AS_UINT as f32) as i32 as u32;
            trial_max[c] = clamp(iz00 * q00[c] + iz01 * q10);
            trial_min[c] = clamp(iz10 * q00[c] + iz11 * q10);
        }
        bc6h_quant_dequant_endpoints(&mut trial_min, &mut trial_max, 10);
        let mut trial_weights = [0u8; 16];
        let trial_err = assign_weights_4(
            &float_pixels,
            &pixel_scales,
            &mut trial_weights,
            trial_min,
            trial_max,
            block_max_var,
            try_2subsets,
            params,
        );
        if trial_err < cur_err {
            cur_err = trial_err;
            min = trial_min;
            max = trial_max;
            log_blk.weights = trial_weights;
        } else {
            break;
        }
    }

    set_endpoints(&mut log_blk, min, max);

    if params.max_2subset_pats_to_try > 0
        && try_2subsets
        && block_max_var > FAST_BC6H_COMPLEX_STD_DEV_THRESH * FAST_BC6H_COMPLEX_STD_DEV_THRESH * 16
    {
        fast_encode_bc6h_2subsets(
            pixels,
            &float_pixels,
            &pixel_scales,
            &mut cur_err,
            &mut log_blk,
            mean,
            [axis_r, axis_g, axis_b],
            params,
        );
    }

    pack_bc6h_block(block, &log_blk);
}
