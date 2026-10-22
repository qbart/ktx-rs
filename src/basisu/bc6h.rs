//! Port of the BC6H encoder helpers and `astc_hdr_transcode_to_bc6h` from
//! basisu_transcoder.cpp (UASTC HDR 4x4 to BC6H).
#![allow(clippy::needless_range_loop)]

use super::astc_helpers::{
    self as ah, BISE_256_LEVELS, HalfFloat, LogAstcBlock, is_half_inf_or_nan, qlog16_to_half,
};
use super::hdr_tables::{BC6H_2SUBSET_PATTERNS, BC6H_BIT_LAYOUTS, BC6H_MODE_SIG_BITS};
use super::uastc_tables::ASTC_BC7_COMMON_PARTITIONS2;
use std::sync::OnceLock;

pub const NUM_BC6H_MODES: u32 = 14;
pub const BC6H_LAST_MODE_INDEX: u32 = 13;
pub const BC6H_FIRST_1SUBSET_MODE_INDEX: u32 = 10;
pub const MAX_BC6H_HALF_FLOAT_AS_UINT: u32 = 0x7BFF;
pub const TOTAL_ASTC_BC6H_COMMON_PARTITIONS2: usize = 27;
pub const MAX_QLOG12: i32 = 3967;
pub const NUM_MODE11_ENDPOINTS: usize = 6;
pub const NUM_MODE7_ENDPOINTS: usize = 4;

/// Port of `bc6h_logical_block`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bc6hLogicalBlock {
    pub mode: u32,
    pub partition_pattern: u32,
    /// `[comp][subset*2+lh_index]`
    pub endpoints: [[u32; 4]; 3],
    pub weights: [u8; 16],
}

/// Port of `bc6h_half_to_blog`.
#[inline]
pub fn bc6h_half_to_blog(h: HalfFloat, num_bits: u32) -> u32 {
    (u32::from(h) * 64 + 30) / (31 * (1 << (16 - num_bits)))
}

#[inline]
fn sig_bits(mode: u32, i: usize) -> u32 {
    u32::from(BC6H_MODE_SIG_BITS[(mode as usize).min(13)][i])
}

#[inline]
fn pattern(pat: u32) -> &'static [u8; 16] {
    &BC6H_2SUBSET_PATTERNS[(pat as usize) & 31]
}

#[inline]
fn write_bits(val: u64, num_bits: u32, bit_pos: &mut u32, l: &mut u64, h: &mut u64) {
    if *bit_pos < 64 {
        *l |= val << *bit_pos;
        if *bit_pos + num_bits > 64 {
            *h |= val >> (64 - *bit_pos);
        }
    } else if *bit_pos < 128 {
        *h |= val << (*bit_pos - 64);
    }
    *bit_pos += num_bits;
}

#[inline]
fn write_rev_bits(val: u64, num_bits: u32, bit_pos: &mut u32, l: &mut u64, h: &mut u64) {
    for i in 0..num_bits {
        write_bits((val >> (num_bits - 1 - i)) & 1, 1, bit_pos, l, h);
    }
}

/// Port of `pack_bc6h_block`.
pub fn pack_bc6h_block(dst: &mut [u8; 16], log_blk: &Bc6hLogicalBlock) {
    const MODE_BITS: [u8; 14] = [
        0b00, 0b01, 0b00010, 0b00110, 0b01010, 0b01110, 0b10010, 0b10110, 0b11010, 0b11110,
        0b00011, 0b00111, 0b01011, 0b01111,
    ];
    let mode = (log_blk.mode as usize).min(13);
    let mut l = u64::from(MODE_BITS[mode]);
    let mut h = 0u64;
    let mut bit_pos = if mode >= 2 { 5 } else { 2 };
    let num_subsets = if mode as u32 >= BC6H_FIRST_1SUBSET_MODE_INDEX {
        1
    } else {
        2
    };

    for layout in BC6H_BIT_LAYOUTS[mode].iter() {
        let [comp, index, last_bit, first_bit] = *layout;
        if comp == -1 {
            break;
        }
        let mut v = if comp == 3 {
            log_blk.partition_pattern
        } else {
            log_blk
                .endpoints
                .get(comp as usize)
                .and_then(|e| e.get(index as usize))
                .copied()
                .unwrap_or(0)
        };
        if first_bit == -1 {
            write_bits(
                u64::from((v >> last_bit) & 1),
                1,
                &mut bit_pos,
                &mut l,
                &mut h,
            );
        } else {
            let total_bits = (i32::from(last_bit) - i32::from(first_bit)).unsigned_abs() + 1;
            v >>= first_bit.min(last_bit);
            v &= (1 << total_bits) - 1;
            if first_bit > last_bit {
                write_rev_bits(u64::from(v), total_bits, &mut bit_pos, &mut l, &mut h);
            } else {
                write_bits(u64::from(v), total_bits, &mut bit_pos, &mut l, &mut h);
            }
        }
    }

    let num_mode_sel_bits = if num_subsets == 1 { 4 } else { 3 };
    let pat = pattern(log_blk.partition_pattern);
    for i in 0..16 {
        let sel = u64::from(log_blk.weights[i]);
        let mut num_bits = num_mode_sel_bits;
        if num_subsets == 2 {
            num_bits -= u32::from(pat[i] >> 7);
        } else if i == 0 {
            num_bits -= 1;
        }
        write_bits(sel, num_bits, &mut bit_pos, &mut l, &mut h);
    }

    dst[..8].copy_from_slice(&l.to_le_bytes());
    dst[8..].copy_from_slice(&h.to_le_bytes());
}

/// Port of `bc6h_enc_block_mode10`.
pub fn bc6h_enc_block_mode10(
    dst: &mut [u8; 16],
    endpoints: &[[HalfFloat; 2]; 3],
    weights: &[u8; 16],
) {
    let mut log_blk = Bc6hLogicalBlock::default();
    for c in 0..3 {
        log_blk.endpoints[c][0] = bc6h_half_to_blog(endpoints[c][0], 10);
        log_blk.endpoints[c][1] = bc6h_half_to_blog(endpoints[c][1], 10);
    }
    log_blk.weights = *weights;
    if log_blk.weights[0] & 8 != 0 {
        for w in log_blk.weights.iter_mut() {
            *w = 15u8.wrapping_sub(*w);
        }
        for c in 0..3 {
            log_blk.endpoints[c].swap(0, 1);
        }
    }
    log_blk.mode = BC6H_FIRST_1SUBSET_MODE_INDEX;
    pack_bc6h_block(dst, &log_blk);
}

/// Port of `bc6h_enc_block_1subset_4bit_weights`.
pub fn bc6h_enc_block_1subset_4bit_weights(
    dst: &mut [u8; 16],
    endpoints: &[[HalfFloat; 2]; 3],
    weights: &[u8; 16],
) {
    let mut log_blk = Bc6hLogicalBlock::default();
    let mut mode = BC6H_LAST_MODE_INDEX;
    while mode > BC6H_FIRST_1SUBSET_MODE_INDEX {
        let num_base_bits = sig_bits(mode, 0);
        let num_delta_bits = sig_bits(mode, 1);
        let delta_bitmask = (1i32 << num_delta_bits) - 1;
        let mut blog = [[0u32; 2]; 3];
        for c in 0..3 {
            blog[c][0] = bc6h_half_to_blog(endpoints[c][0], num_base_bits);
            blog[c][1] = bc6h_half_to_blog(endpoints[c][1], num_base_bits);
        }
        log_blk.weights = *weights;
        if log_blk.weights[0] & 8 != 0 {
            for w in log_blk.weights.iter_mut() {
                *w = 15u8.wrapping_sub(*w);
            }
            for c in 0..3 {
                blog[c].swap(0, 1);
            }
        }
        let max_delta = (1i32 << (num_delta_bits - 1)) - 1;
        let min_delta = -(max_delta + 1);
        let mut failed = false;
        for c in 0..3 {
            log_blk.endpoints[c][0] = blog[c][0];
            let delta = blog[c][1] as i32 - blog[c][0] as i32;
            if delta < min_delta || delta > max_delta {
                failed = true;
                break;
            }
            log_blk.endpoints[c][1] = (delta & delta_bitmask) as u32;
        }
        if !failed {
            log_blk.mode = mode;
            pack_bc6h_block(dst, &log_blk);
            return;
        }
        mode -= 1;
    }
    bc6h_enc_block_mode10(dst, endpoints, weights);
}

/// Inverts the weights of `subset` (pattern subset index) in place.
fn invert_subset_weights(weights: &mut [u8; 16], pat: &[u8; 16], subset: u8) {
    for i in 0..16 {
        if (pat[i] & 0x7F) == subset {
            weights[i] = 7u8.wrapping_sub(weights[i]);
        }
    }
}

/// Port of `bc6h_enc_block_1subset_mode9_3bit_weights`.
pub fn bc6h_enc_block_1subset_mode9_3bit_weights(
    dst: &mut [u8; 16],
    endpoints: &[[HalfFloat; 2]; 3],
    weights: &[u8; 16],
) {
    let mut log_blk = Bc6hLogicalBlock::default();
    for c in 0..3 {
        log_blk.endpoints[c][0] = bc6h_half_to_blog(endpoints[c][0], 6);
        log_blk.endpoints[c][2] = log_blk.endpoints[c][0];
        log_blk.endpoints[c][1] = bc6h_half_to_blog(endpoints[c][1], 6);
        log_blk.endpoints[c][3] = log_blk.endpoints[c][1];
    }
    log_blk.weights = *weights;
    let pat_index = 0;
    let pat = pattern(pat_index);
    if log_blk.weights[0] & 4 != 0 {
        for c in 0..3 {
            log_blk.endpoints[c].swap(0, 1);
        }
        invert_subset_weights(&mut log_blk.weights, pat, 0);
    }
    if log_blk.weights[15] & 4 != 0 {
        for c in 0..3 {
            log_blk.endpoints[c].swap(2, 3);
        }
        invert_subset_weights(&mut log_blk.weights, pat, 1);
    }
    log_blk.mode = 9;
    log_blk.partition_pattern = pat_index;
    pack_bc6h_block(dst, &log_blk);
}

/// Delta-encodes `blog[c][0..4]` for `mode` into `log_blk`; false if out of range.
fn pack_deltas(log_blk: &mut Bc6hLogicalBlock, blog: &[[u32; 4]; 3], mode: u32) -> bool {
    for c in 0..3 {
        let num_delta_bits = sig_bits(mode, c + 1);
        let delta_bitmask = (1i32 << num_delta_bits) - 1;
        let max_delta = (1i32 << (num_delta_bits - 1)) - 1;
        let min_delta = -(max_delta + 1);
        log_blk.endpoints[c][0] = blog[c][0];
        let d0 = blog[c][1] as i32 - blog[c][0] as i32;
        let d1 = blog[c][2] as i32 - blog[c][0] as i32;
        let d2 = blog[c][3] as i32 - blog[c][0] as i32;
        let out = |d: i32| d < min_delta || d > max_delta;
        if out(d0) || out(d1) || out(d2) {
            return false;
        }
        log_blk.endpoints[c][1] = (d0 & delta_bitmask) as u32;
        log_blk.endpoints[c][2] = (d1 & delta_bitmask) as u32;
        log_blk.endpoints[c][3] = (d2 & delta_bitmask) as u32;
    }
    true
}

/// Modes ordered from largest base bits to least (`s_mode_order`).
const MODE_ORDER: [u32; 9] = [2, 3, 4, 0, 5, 6, 7, 8, 1];

/// Port of `bc6h_enc_block_1subset_3bit_weights`.
pub fn bc6h_enc_block_1subset_3bit_weights(
    dst: &mut [u8; 16],
    endpoints: &[[HalfFloat; 2]; 3],
    weights: &[u8; 16],
) {
    let mut log_blk = Bc6hLogicalBlock::default();
    for mode in MODE_ORDER {
        let num_base_bits = sig_bits(mode, 0);
        let mut blog = [[0u32; 4]; 3];
        for c in 0..3 {
            blog[c][0] = bc6h_half_to_blog(endpoints[c][0], num_base_bits);
            blog[c][2] = blog[c][0];
            blog[c][1] = bc6h_half_to_blog(endpoints[c][1], num_base_bits);
            blog[c][3] = blog[c][1];
        }
        let pat_index = 0;
        let pat = pattern(pat_index);
        log_blk.weights = *weights;
        if log_blk.weights[0] & 4 != 0 {
            for c in 0..3 {
                blog[c].swap(0, 1);
            }
            invert_subset_weights(&mut log_blk.weights, pat, 0);
        }
        if log_blk.weights[15] & 4 != 0 {
            for c in 0..3 {
                blog[c].swap(2, 3);
            }
            invert_subset_weights(&mut log_blk.weights, pat, 1);
        }
        if !pack_deltas(&mut log_blk, &blog, mode) {
            continue;
        }
        log_blk.mode = mode;
        log_blk.partition_pattern = pat_index;
        pack_bc6h_block(dst, &log_blk);
        return;
    }
    bc6h_enc_block_1subset_mode9_3bit_weights(dst, endpoints, weights);
}

/// Applies the common partition's invert flag and the anchor-driven endpoint
/// swaps (shared by the 2-subset encoders). Returns the BC6H pattern index.
fn apply_2subset_swaps(
    blog: &mut [[u32; 4]; 3],
    weights: &mut [u8; 16],
    common_part_index: u32,
) -> u32 {
    let (bc7_pattern, _, invert_flag) = ASTC_BC7_COMMON_PARTITIONS2
        .get(common_part_index as usize)
        .copied()
        .unwrap_or((0, 0, false));
    if invert_flag {
        for c in 0..3 {
            blog[c].swap(0, 2);
            blog[c].swap(1, 3);
        }
    }
    let pat_index = u32::from(bc7_pattern);
    let pat = pattern(pat_index);
    let mut swap_flags = [false; 2];
    for i in 0..16 {
        if (pat[i] & 0x80) == 0 {
            continue;
        }
        if weights[i] & 4 != 0 {
            swap_flags[(pat[i] & 1) as usize] = true;
        }
    }
    if swap_flags[0] {
        for c in 0..3 {
            blog[c].swap(0, 1);
        }
        invert_subset_weights(weights, pat, 0);
    }
    if swap_flags[1] {
        for c in 0..3 {
            blog[c].swap(2, 3);
        }
        invert_subset_weights(weights, pat, 1);
    }
    pat_index
}

/// Port of `bc6h_enc_block_2subset_mode9_3bit_weights`; `endpoints[subset][comp][lh]`.
pub fn bc6h_enc_block_2subset_mode9_3bit_weights(
    dst: &mut [u8; 16],
    common_part_index: u32,
    endpoints: &[[[HalfFloat; 2]; 3]; 2],
    weights: &[u8; 16],
) {
    let mut log_blk = Bc6hLogicalBlock::default();
    for s in 0..2 {
        for c in 0..3 {
            log_blk.endpoints[c][s * 2] = bc6h_half_to_blog(endpoints[s][c][0], 6);
            log_blk.endpoints[c][1 + s * 2] = bc6h_half_to_blog(endpoints[s][c][1], 6);
        }
    }
    log_blk.weights = *weights;
    let pat_index = apply_2subset_swaps(
        &mut log_blk.endpoints,
        &mut log_blk.weights,
        common_part_index,
    );
    log_blk.mode = 9;
    log_blk.partition_pattern = pat_index;
    pack_bc6h_block(dst, &log_blk);
}

/// Port of `bc6h_enc_block_2subset_3bit_weights`; `endpoints[subset][comp][lh]`.
pub fn bc6h_enc_block_2subset_3bit_weights(
    dst: &mut [u8; 16],
    common_part_index: u32,
    endpoints: &[[[HalfFloat; 2]; 3]; 2],
    weights: &[u8; 16],
) {
    let mut log_blk = Bc6hLogicalBlock::default();
    for mode in MODE_ORDER {
        let num_base_bits = sig_bits(mode, 0);
        let mut blog = [[0u32; 4]; 3];
        for s in 0..2 {
            for c in 0..3 {
                blog[c][s * 2] = bc6h_half_to_blog(endpoints[s][c][0], num_base_bits);
                blog[c][1 + s * 2] = bc6h_half_to_blog(endpoints[s][c][1], num_base_bits);
            }
        }
        log_blk.weights = *weights;
        let pat_index = apply_2subset_swaps(&mut blog, &mut log_blk.weights, common_part_index);
        if !pack_deltas(&mut log_blk, &blog, mode) {
            continue;
        }
        log_blk.mode = mode;
        log_blk.partition_pattern = pat_index;
        pack_bc6h_block(dst, &log_blk);
        return;
    }
    bc6h_enc_block_2subset_mode9_3bit_weights(dst, common_part_index, endpoints, weights);
}

/// Port of `bc6h_enc_block_solid_color`.
pub fn bc6h_enc_block_solid_color(dst: &mut [u8; 16], color: [HalfFloat; 3]) -> bool {
    if (color[0] | color[1] | color[2]) & 0x8000 != 0 {
        return false;
    }
    let endpoints = [
        [color[0], color[0]],
        [color[1], color[1]],
        [color[2], color[2]],
    ];
    bc6h_enc_block_1subset_4bit_weights(dst, &endpoints, &[0; 16]);
    true
}

/// `g_astc_partition_id_to_common_bc7_pat_index` (from `astc_hdr_core_init`).
fn astc_partition_id_to_common_bc7_pat_index() -> &'static [i8; 1024] {
    static T: OnceLock<[i8; 1024]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [-1i8; 1024];
        for (part_index, &(_, astc, _)) in ASTC_BC7_COMMON_PARTITIONS2
            .iter()
            .take(TOTAL_ASTC_BC6H_COMMON_PARTITIONS2)
            .enumerate()
        {
            if let Some(slot) = t.get_mut(astc as usize) {
                *slot = part_index as i8;
            }
        }
        t
    })
}

/// Port of `qlog_to_half`.
#[inline]
pub fn qlog_to_half(qlog: u32, bits: u32) -> HalfFloat {
    qlog16_to_half((qlog << (16 - bits)) as i32)
}

fn dequant_endpoints<const N: usize>(endpoints: &[u8], ise_range: u32) -> [i32; N] {
    let tab = ah::dequant_tables().endpoint_ise_to_val(ise_range);
    let mut out = [0i32; N];
    for (i, o) in out.iter_mut().enumerate() {
        let v = endpoints.get(i).copied().unwrap_or(0);
        *o = i32::from(if ise_range == BISE_256_LEVELS {
            v
        } else {
            tab.get(v as usize).copied().unwrap_or(0)
        });
    }
    out
}

fn qlog12_in_range(e: &[[i32; 3]; 2]) -> bool {
    e.iter().all(|x| x.iter().all(|&v| v <= MAX_QLOG12))
}

/// Port of `decode_mode7_to_qlog12`: `e[lh][comp]`.
pub fn decode_mode7_to_qlog12(endpoints: &[u8], e: &mut [[i32; 3]; 2], ise_range: u32) -> bool {
    let [v0, v1, v2, v3] = dequant_endpoints::<4>(endpoints, ise_range);
    let (lo, hi, _) = ah::decode_mode7_ise20(v0, v1, v2, v3);
    *e = [lo, hi];
    qlog12_in_range(e)
}

/// Port of `decode_mode11_to_qlog12`: `e[lh][comp]`.
pub fn decode_mode11_to_qlog12(endpoints: &[u8], e: &mut [[i32; 3]; 2], ise_range: u32) -> bool {
    let [v0, v1, v2, v3, v4, v5] = dequant_endpoints::<6>(endpoints, ise_range);
    let (lo, hi) = ah::decode_mode11_ise20(v0, v1, v2, v3, v4, v5);
    *e = [lo, hi];
    qlog12_in_range(e)
}

fn map_weights(src: &[u8; 64], table: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = table.get(src[i] as usize).copied().unwrap_or(0);
    }
    out
}

/// Port of `transcode_bc6h_1subset`.
fn transcode_bc6h_1subset(
    h_e: &[[HalfFloat; 2]; 3],
    best_blk: &LogAstcBlock,
    dst: &mut [u8; 16],
) -> bool {
    let w = &best_blk.weights;
    let weights16 = |src: &[u8; 64]| -> [u8; 16] {
        let mut o = [0u8; 16];
        o.copy_from_slice(&src[..16]);
        o
    };
    if best_blk.weight_ise_range == 5 {
        bc6h_enc_block_1subset_3bit_weights(dst, h_e, &weights16(w));
        return true;
    }
    let bc6h_weights = match best_blk.weight_ise_range {
        1 => map_weights(w, &[0, 8, 15]),
        2 => map_weights(w, &[0, 5, 10, 15]),
        3 => map_weights(w, &[0, 4, 7, 11, 15]),
        4 => map_weights(w, &[0, 15, 3, 12, 6, 9]),
        6 => map_weights(w, &[0, 15, 2, 13, 3, 12, 5, 10, 6, 9]),
        7 => map_weights(w, &[0, 15, 4, 11, 1, 14, 5, 10, 2, 13, 6, 9]),
        8 => weights16(w),
        _ => return false,
    };
    bc6h_enc_block_1subset_4bit_weights(dst, h_e, &bc6h_weights);
    true
}

/// Port of `transcode_bc6h_2subsets`.
fn transcode_bc6h_2subsets(
    common_part_index: u32,
    best_blk: &LogAstcBlock,
    dst: &mut [u8; 16],
) -> bool {
    let cem = best_blk.color_endpoint_modes;
    if cem[0] != cem[1] || (cem[0] != 7 && cem[0] != 11) {
        return false;
    }
    let (wr, er) = (best_blk.weight_ise_range, best_blk.endpoint_ise_range);
    let ok = if cem[0] == 7 {
        matches!((wr, er), (1, 20) | (2, 20) | (3, 19) | (4, 17) | (5, 15))
    } else {
        matches!((wr, er), (1, 14) | (2, 12))
    };
    if !ok {
        return false;
    }

    let mut bc6h_endpoints = [[[0u16; 2]; 3]; 2];
    for s in 0..2 {
        let mut e = [[0i32; 3]; 2];
        let success = if cem[0] == 7 {
            decode_mode7_to_qlog12(
                best_blk
                    .endpoints
                    .get(s * NUM_MODE7_ENDPOINTS..)
                    .unwrap_or(&[]),
                &mut e,
                u32::from(er),
            )
        } else {
            decode_mode11_to_qlog12(
                best_blk
                    .endpoints
                    .get(s * NUM_MODE11_ENDPOINTS..)
                    .unwrap_or(&[]),
                &mut e,
                u32::from(er),
            )
        };
        if !success {
            return false;
        }
        for c in 0..3 {
            for lh in 0..2 {
                let h = qlog_to_half(e[lh][c] as u32, 12);
                if is_half_inf_or_nan(h) {
                    return false;
                }
                bc6h_endpoints[s][c][lh] = h;
            }
        }
    }

    let w = &best_blk.weights;
    let bc6h_weights = match wr {
        1 => map_weights(w, &[0, 4, 7]),
        2 => map_weights(w, &[0, 2, 5, 7]),
        3 => map_weights(w, &[0, 2, 4, 5, 7]),
        4 => map_weights(w, &[0, 7, 1, 6, 3, 4]),
        5 => map_weights(w, &[0, 1, 2, 3, 4, 5, 6, 7]),
        _ => return false,
    };
    bc6h_enc_block_2subset_3bit_weights(dst, common_part_index, &bc6h_endpoints, &bc6h_weights);
    true
}

/// Port of `astc_hdr_transcode_to_bc6h(const astc_blk&, bc6h_block&)`.
pub fn astc_hdr_transcode_to_bc6h(src_blk: &[u8; 16], dst: &mut [u8; 16]) -> bool {
    let mut log_blk = LogAstcBlock::default();
    if !ah::unpack_block(src_blk, &mut log_blk, 4, 4) {
        return false;
    }
    astc_hdr_transcode_log_to_bc6h(&log_blk, dst)
}

/// Port of `astc_hdr_transcode_to_bc6h(const log_astc_block&, bc6h_block&)`.
pub fn astc_hdr_transcode_log_to_bc6h(log_blk: &LogAstcBlock, dst: &mut [u8; 16]) -> bool {
    if log_blk.solid_color_flag_ldr {
        return false;
    }
    if log_blk.solid_color_flag_hdr {
        return bc6h_enc_block_solid_color(
            dst,
            [
                log_blk.solid_color(0),
                log_blk.solid_color(1),
                log_blk.solid_color(2),
            ],
        );
    }
    if log_blk.grid_width != 4 || log_blk.grid_height != 4 || log_blk.dual_plane {
        return false;
    }
    match log_blk.num_partitions {
        1 => {
            if !(1..=8).contains(&log_blk.weight_ise_range) {
                return false;
            }
            let er = u32::from(log_blk.endpoint_ise_range);
            let mut e = [[0i32; 3]; 2];
            let success = match log_blk.color_endpoint_modes[0] {
                7 => {
                    if er != 20 {
                        return false;
                    }
                    decode_mode7_to_qlog12(&log_blk.endpoints, &mut e, er)
                }
                11 => {
                    if log_blk.weight_ise_range <= 7 {
                        if er != 20 {
                            return false;
                        }
                    } else if er != 19 {
                        return false;
                    }
                    decode_mode11_to_qlog12(&log_blk.endpoints, &mut e, er)
                }
                _ => return false,
            };
            if !success {
                return false;
            }
            let q = |v: i32| qlog_to_half(v as u32, 12);
            let h_e = [
                [q(e[0][0]), q(e[1][0])],
                [q(e[0][1]), q(e[1][1])],
                [q(e[0][2]), q(e[1][2])],
            ];
            for i in 0..2 {
                if is_half_inf_or_nan(h_e[0][i])
                    || is_half_inf_or_nan(h_e[1][i])
                    || is_half_inf_or_nan(h_e[2][i])
                {
                    return false;
                }
            }
            transcode_bc6h_1subset(&h_e, log_blk, dst)
        }
        2 => {
            let idx = astc_partition_id_to_common_bc7_pat_index()
                .get(log_blk.partition_id as usize)
                .copied()
                .unwrap_or(-1);
            if idx < 0 {
                return false;
            }
            transcode_bc6h_2subsets(idx as u32, log_blk, dst)
        }
        _ => false,
    }
}
