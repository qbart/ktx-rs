// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Subset of `encoder/basisu_gpu_texture.cpp` used by the encoders: `unpack_bc1` and the
//! `basist::bc7u::unpack_bc7` BC7 decoder behind `unpack_block(cBC1 / cBC7)`.

#![allow(clippy::needless_range_loop)]

use super::enc::ColorRgba;
use super::uastc_tables::{
    BC7_PARTITION2, BC7_PARTITION3, BC7_TABLE_ANCHOR_INDEX_SECOND_SUBSET,
    BC7_TABLE_ANCHOR_INDEX_THIRD_SUBSET_1, BC7_TABLE_ANCHOR_INDEX_THIRD_SUBSET_2, BC7_WEIGHTS2,
    BC7_WEIGHTS3, BC7_WEIGHTS4,
};

fn bc1_unpack_color(c: u32) -> (u32, u32, u32) {
    let r = (c >> 11) & 31;
    let g = (c >> 5) & 63;
    let b = c & 31;
    (
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    )
}

/// `unpack_bc1`: returns true if the block uses 3 color punchthrough alpha mode. With
/// `set_alpha` false only RGB is written.
pub fn unpack_bc1(block: &[u8; 8], pixels: &mut [ColorRgba; 16], set_alpha: bool) -> bool {
    let l = block[0] as u32 | (block[1] as u32) << 8;
    let h = block[2] as u32 | (block[3] as u32) << 8;
    let (r0, g0, b0) = bc1_unpack_color(l);
    let (r1, g1, b1) = bc1_unpack_color(h);
    let mut c = [[0u8; 4]; 4];
    c[0] = [r0 as u8, g0 as u8, b0 as u8, 255];
    c[1] = [r1 as u8, g1 as u8, b1 as u8, 255];
    let mut used_punchthrough = false;
    if l > h {
        c[2] = [
            ((r0 * 2 + r1) / 3) as u8,
            ((g0 * 2 + g1) / 3) as u8,
            ((b0 * 2 + b1) / 3) as u8,
            255,
        ];
        c[3] = [
            ((r1 * 2 + r0) / 3) as u8,
            ((g1 * 2 + g0) / 3) as u8,
            ((b1 * 2 + b0) / 3) as u8,
            255,
        ];
    } else {
        c[2] = [
            ((r0 + r1) / 2) as u8,
            ((g0 + g1) / 2) as u8,
            ((b0 + b1) / 2) as u8,
            255,
        ];
        used_punchthrough = true;
    }
    for y in 0..4 {
        for x in 0..4 {
            let s = (block[4 + y] >> (x * 2)) & 3;
            let p = &mut pixels[y * 4 + x];
            if set_alpha {
                *p = c[s as usize];
            } else {
                p[..3].copy_from_slice(&c[s as usize][..3]);
            }
        }
    }
    used_punchthrough
}

#[inline]
fn bc7_dequant_p(val: u32, pbit: u32, val_bits: u32) -> u32 {
    let total_bits = val_bits + 1;
    let mut val = (val << 1) | pbit;
    val <<= 8 - total_bits;
    val |= val >> total_bits;
    val
}

#[inline]
fn bc7_dequant(val: u32, val_bits: u32) -> u32 {
    let mut val = val << (8 - val_bits);
    val |= val >> val_bits;
    val
}

#[inline]
fn bc7_interp(l: u32, h: u32, w: usize, bits: u32) -> u32 {
    let wt = match bits {
        2 => BC7_WEIGHTS2[w & 3],
        3 => BC7_WEIGHTS3[w & 7],
        4 => BC7_WEIGHTS4[w & 15],
        _ => return 0,
    };
    (l * (64 - wt) + h * wt + 32) >> 6
}

fn read_bits32(buf: &[u8; 16], bit_offset: &mut u32, codesize: u32) -> u32 {
    let mut bits = 0u32;
    let mut total_bits = 0u32;
    while total_bits < codesize {
        let byte_bit_offset = *bit_offset & 7;
        let bits_to_read = (codesize - total_bits).min(8 - byte_bit_offset);
        let mut byte_bits = (buf[(*bit_offset >> 3) as usize & 15] as u32) >> byte_bit_offset;
        byte_bits &= (1 << bits_to_read) - 1;
        bits |= byte_bits << total_bits;
        total_bits += bits_to_read;
        *bit_offset += bits_to_read;
    }
    bits
}

fn unpack_bc7_mode0_2(mode: u32, buf: &[u8; 16], pixels: &mut [ColorRgba; 16]) -> bool {
    const ENDPOINTS: usize = 6;
    const COMPS: usize = 3;
    let weight_bits = if mode == 0 { 3 } else { 2 };
    let endpoint_bits = if mode == 0 { 4 } else { 5 };
    let pbits_n = if mode == 0 { 6 } else { 0 };
    let weight_vals = 1usize << weight_bits;

    let mut bo = 0u32;
    if read_bits32(buf, &mut bo, mode + 1) != (1 << mode) {
        return false;
    }
    let part = read_bits32(buf, &mut bo, if mode == 0 { 4 } else { 6 }) as usize;
    let mut endpoints = [[0u32; 4]; ENDPOINTS];
    for c in 0..COMPS {
        for e in 0..ENDPOINTS {
            endpoints[e][c] = read_bits32(buf, &mut bo, endpoint_bits);
        }
    }
    let mut pbits = [0u32; 6];
    for p in 0..pbits_n {
        pbits[p] = read_bits32(buf, &mut bo, 1);
    }
    let mut weights = [0usize; 16];
    for i in 0..16 {
        let anchor = i == 0
            || i == BC7_TABLE_ANCHOR_INDEX_THIRD_SUBSET_1[part] as usize
            || i == BC7_TABLE_ANCHOR_INDEX_THIRD_SUBSET_2[part] as usize;
        weights[i] = read_bits32(
            buf,
            &mut bo,
            if anchor { weight_bits - 1 } else { weight_bits },
        ) as usize;
    }
    for e in 0..ENDPOINTS {
        for c in 0..4 {
            endpoints[e][c] = if c == 3 {
                255
            } else if pbits_n != 0 {
                bc7_dequant_p(endpoints[e][c], pbits[e], endpoint_bits)
            } else {
                bc7_dequant(endpoints[e][c], endpoint_bits)
            };
        }
    }
    let mut block_colors = [[[0u8; 4]; 8]; 3];
    for s in 0..3 {
        for i in 0..weight_vals {
            for c in 0..3 {
                block_colors[s][i][c] =
                    bc7_interp(endpoints[s * 2][c], endpoints[s * 2 + 1][c], i, weight_bits) as u8;
            }
            block_colors[s][i][3] = 255;
        }
    }
    for i in 0..16 {
        pixels[i] = block_colors[BC7_PARTITION3[part * 16 + i] as usize][weights[i]];
    }
    true
}

fn unpack_bc7_mode1_3_7(mode: u32, buf: &[u8; 16], pixels: &mut [ColorRgba; 16]) -> bool {
    const ENDPOINTS: usize = 4;
    let comps = if mode == 7 { 4 } else { 3 };
    let weight_bits = if mode == 1 { 3 } else { 2 };
    let endpoint_bits = if mode == 7 {
        5
    } else if mode == 1 {
        6
    } else {
        7
    };
    let pbits_n = if mode == 1 { 2 } else { 4 };
    let shared_pbits = mode == 1;
    let weight_vals = 1usize << weight_bits;

    let mut bo = 0u32;
    if read_bits32(buf, &mut bo, mode + 1) != (1 << mode) {
        return false;
    }
    let part = read_bits32(buf, &mut bo, 6) as usize;
    let mut endpoints = [[0u32; 4]; ENDPOINTS];
    for c in 0..comps {
        for e in 0..ENDPOINTS {
            endpoints[e][c] = read_bits32(buf, &mut bo, endpoint_bits);
        }
    }
    let mut pbits = [0u32; 4];
    for p in 0..pbits_n {
        pbits[p] = read_bits32(buf, &mut bo, 1);
    }
    let mut weights = [0usize; 16];
    for i in 0..16 {
        let anchor = i == 0 || i == BC7_TABLE_ANCHOR_INDEX_SECOND_SUBSET[part] as usize;
        weights[i] = read_bits32(
            buf,
            &mut bo,
            if anchor { weight_bits - 1 } else { weight_bits },
        ) as usize;
    }
    let alpha_c = if mode == 7 { 4 } else { 3 };
    for e in 0..ENDPOINTS {
        for c in 0..4 {
            endpoints[e][c] = if c == alpha_c {
                255
            } else {
                let pb = pbits[if shared_pbits { e >> 1 } else { e }];
                bc7_dequant_p(endpoints[e][c], pb, endpoint_bits) & 0xFF
            };
        }
    }
    let mut block_colors = [[[0u8; 4]; 8]; 2];
    for s in 0..2 {
        for i in 0..weight_vals {
            for c in 0..comps {
                block_colors[s][i][c] =
                    bc7_interp(endpoints[s * 2][c], endpoints[s * 2 + 1][c], i, weight_bits) as u8;
            }
            if comps == 3 {
                block_colors[s][i][3] = 255;
            }
        }
    }
    for i in 0..16 {
        pixels[i] = block_colors[BC7_PARTITION2[part * 16 + i] as usize][weights[i]];
    }
    true
}

fn unpack_bc7_mode4_5(mode: u32, buf: &[u8; 16], pixels: &mut [ColorRgba; 16]) -> bool {
    let weight_bits_c = 2u32;
    let a_weight_bits = if mode == 4 { 3 } else { 2 };
    let endpoint_bits = if mode == 4 { 5 } else { 7 };
    let a_endpoint_bits = if mode == 4 { 6 } else { 8 };

    let mut bo = 0u32;
    if read_bits32(buf, &mut bo, mode + 1) != (1 << mode) {
        return false;
    }
    let comp_rot = read_bits32(buf, &mut bo, 2) as usize;
    let index_mode = if mode == 4 {
        read_bits32(buf, &mut bo, 1) as usize
    } else {
        0
    };
    let mut endpoints = [[0u32; 4]; 2];
    for c in 0..4 {
        for e in 0..2 {
            endpoints[e][c] = read_bits32(
                buf,
                &mut bo,
                if c == 3 {
                    a_endpoint_bits
                } else {
                    endpoint_bits
                },
            );
        }
    }
    let weight_bits = if index_mode != 0 {
        [a_weight_bits, weight_bits_c]
    } else {
        [weight_bits_c, a_weight_bits]
    };
    let mut weights = [0usize; 16];
    let mut a_weights = [0usize; 16];
    for i in 0..16 {
        let v = read_bits32(
            buf,
            &mut bo,
            weight_bits[index_mode] - if i == 0 { 1 } else { 0 },
        ) as usize;
        if index_mode != 0 {
            a_weights[i] = v;
        } else {
            weights[i] = v;
        }
    }
    for i in 0..16 {
        let v = read_bits32(
            buf,
            &mut bo,
            weight_bits[1 - index_mode] - if i == 0 { 1 } else { 0 },
        ) as usize;
        if index_mode != 0 {
            weights[i] = v;
        } else {
            a_weights[i] = v;
        }
    }
    for e in 0..2 {
        for c in 0..4 {
            endpoints[e][c] = bc7_dequant(
                endpoints[e][c],
                if c == 3 {
                    a_endpoint_bits
                } else {
                    endpoint_bits
                },
            ) & 0xFF;
        }
    }
    let mut block_colors = [[0u8; 4]; 8];
    for i in 0..(1usize << weight_bits[0]) {
        for c in 0..3 {
            block_colors[i][c] =
                bc7_interp(endpoints[0][c], endpoints[1][c], i, weight_bits[0]) as u8;
        }
    }
    for i in 0..(1usize << weight_bits[1]) {
        block_colors[i][3] = bc7_interp(endpoints[0][3], endpoints[1][3], i, weight_bits[1]) as u8;
    }
    for i in 0..16 {
        pixels[i] = block_colors[weights[i]];
        pixels[i][3] = block_colors[a_weights[i]][3];
        if comp_rot >= 1 {
            pixels[i].swap(3, comp_rot - 1);
        }
    }
    true
}

fn unpack_bc7_mode6(buf: &[u8; 16], pixels: &mut [ColorRgba; 16]) -> bool {
    let lo = u64::from_le_bytes([
        buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
    ]);
    let hi = u64::from_le_bytes([
        buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
    ]);
    if lo & 0x7F != 1 << 6 {
        return false;
    }
    let f = |shift: u32| ((lo >> shift) & 0x7F) as u32;
    let p0 = ((lo >> 63) & 1) as u32;
    let p1 = (hi & 1) as u32;
    let e0 = [
        f(7) << 1 | p0,
        f(21) << 1 | p0,
        f(35) << 1 | p0,
        f(49) << 1 | p0,
    ];
    let e1 = [
        f(14) << 1 | p1,
        f(28) << 1 | p1,
        f(42) << 1 | p1,
        f(56) << 1 | p1,
    ];
    let mut vals = [[0u8; 4]; 16];
    for i in 0..16 {
        let w = BC7_WEIGHTS4[i];
        let iw = 64 - w;
        for c in 0..4 {
            vals[i][c] = ((e0[c] * iw + e1[c] * w + 32) >> 6) as u8;
        }
    }
    // selectors: s00 is 3 bits at bit 1 of hi, the rest 4 bits each
    let mut shift = 1u32;
    for i in 0..16 {
        let bits = if i == 0 { 3 } else { 4 };
        let s = ((hi >> shift) & ((1 << bits) - 1)) as usize;
        shift += bits;
        pixels[i] = vals[s];
    }
    true
}

/// `basist::bc7u::unpack_bc7`.
pub fn unpack_bc7(block: &[u8; 16], pixels: &mut [ColorRgba; 16]) -> bool {
    let first_byte = block[0] as u32;
    for mode in 0..=7u32 {
        if first_byte & (1 << mode) != 0 {
            return match mode {
                0 | 2 => unpack_bc7_mode0_2(mode, block, pixels),
                1 | 3 | 7 => unpack_bc7_mode1_3_7(mode, block, pixels),
                4 | 5 => unpack_bc7_mode4_5(mode, block, pixels),
                _ => unpack_bc7_mode6(block, pixels),
            };
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// BC6H decoding (`basisu_gpu_texture.cpp` `unpack_bc6h`)

const BC6H_MODE_LOOKUP: [i8; 32] = [
    0, 1, 2, 10, 0, 1, 3, 11, 0, 1, 4, 12, 0, 1, 5, 13, 0, 1, 6, -1, 0, 1, 7, -1, 0, 1, 8, -1, 0,
    1, 9, -1,
];
const BC6H_WEIGHT3: [u8; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const BC6H_WEIGHT4: [u8; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

#[inline]
fn bc6h_sign_extend(val: i32, bits: i32) -> i32 {
    (val << (32 - bits)) >> (32 - bits)
}

#[inline]
fn bc6h_apply_delta(base: i32, delta: i32, num_bits: i32, is_signed: bool) -> i32 {
    let bitmask = (1 << num_bits) - 1;
    let v = (base + delta) & bitmask;
    if is_signed {
        bc6h_sign_extend(v, num_bits)
    } else {
        v
    }
}

fn bc6h_dequantize(mut val: i32, bits: i32, is_signed: bool) -> i32 {
    if is_signed {
        if bits >= 16 {
            return val;
        }
        let s_flag = val < 0;
        if s_flag {
            val = -val;
        }
        let mut result = if val == 0 {
            0
        } else if val >= (1 << (bits - 1)) - 1 {
            0x7FFF
        } else {
            ((val << 15) + 0x4000) >> (bits - 1)
        };
        if s_flag {
            result = -result;
        }
        result
    } else if bits >= 15 {
        val
    } else if val == 0 {
        0
    } else if val == (1 << bits) - 1 {
        0xFFFF
    } else {
        ((val << 16) + 0x8000) >> bits
    }
}

#[inline]
fn bc6h_interpolate(a: i32, b: i32, weights: &[u8], index: usize) -> i32 {
    let w = weights.get(index).copied().unwrap_or(0) as i32;
    (a * (64 - w) + b * w + 32) >> 6
}

#[inline]
fn bc6h_convert_to_half(mut val: i32, is_signed: bool) -> u16 {
    if !is_signed {
        // scale by 31/64
        return ((val * 31) >> 6) as u16;
    }
    // scale by 31/32
    val = if val < 0 {
        -(((-val) * 31) >> 5)
    } else {
        (val * 31) >> 5
    };
    let mut s = 0;
    if val < 0 {
        s = 0x8000;
        val = -val;
    }
    (s | val) as u16
}

#[inline]
fn bc6h_get_bits(num_bits: u32, l: &mut u64, h: &mut u64) -> u32 {
    let v = (*l & ((1u64 << num_bits) - 1)) as u32;
    *l >>= num_bits;
    *l |= *h << (64 - num_bits);
    *h >>= num_bits;
    v
}

#[inline]
fn bc6h_reverse_bits(v: u32, num_bits: u32) -> u32 {
    let mut res = 0;
    for i in 0..num_bits {
        let bit = u32::from(v & (1 << i) != 0);
        res |= bit << (num_bits - 1 - i);
    }
    res
}

/// Port of `unpack_bc6h`: decodes a BC6H block to 16 RGB halves (row major). On an invalid
/// mode false is returned and, like the C++ (which clears 4 halves per 12 half row), only the
/// first 4 halves of each row are zeroed.
pub fn unpack_bc6h(block: &[u8; 16], dst: &mut [[u16; 3]; 16], is_signed: bool) -> bool {
    use super::hdr_tables::{BC6H_2SUBSET_PATTERNS, BC6H_BIT_LAYOUTS, BC6H_MODE_SIG_BITS};
    let mut lo = [0u8; 8];
    let mut hi = [0u8; 8];
    lo.copy_from_slice(&block[..8]);
    hi.copy_from_slice(&block[8..]);
    let mut blo = u64::from_le_bytes(lo);
    let mut bhi = u64::from_le_bytes(hi);

    // Unpack mode
    let mode = BC6H_MODE_LOOKUP[(blo & 31) as usize];
    if mode < 0 {
        for y in 0..4 {
            dst[y * 4] = [0; 3];
            dst[y * 4 + 1][0] = 0;
        }
        return false;
    }
    let mode = mode as usize;
    // Skip mode bits
    bc6h_get_bits(if mode < 2 { 2 } else { 5 }, &mut blo, &mut bhi);

    let num_subsets = if mode >= 10 { 1 } else { 2 };
    let is_mode_9_or_10 = mode == 9 || mode == 10;

    // Unpack endpoint components [subset][comp][l/h]
    let mut comps = [[[0i32; 2]; 3]; 2];
    let mut part_index = 0i32;
    for layout in BC6H_BIT_LAYOUTS[mode].iter() {
        let (comp, index, last_bit, first_bit) =
            (layout[0], layout[1], layout[2] as i32, layout[3] as i32);
        if comp < 0 {
            break;
        }
        let (subset, lh_index) = ((index >> 1).max(0) as usize, (index & 1) as usize);
        let res = if comp == 3 {
            &mut part_index
        } else {
            &mut comps[subset & 1][comp as usize % 3][lh_index]
        };
        if first_bit < 0 {
            *res |= (bc6h_get_bits(1, &mut blo, &mut bhi) << last_bit) as i32;
        } else {
            let total_bits = (last_bit - first_bit).unsigned_abs() + 1;
            let bit_shift = first_bit.min(last_bit);
            let mut b = bc6h_get_bits(total_bits, &mut blo, &mut bhi);
            if last_bit < first_bit {
                b = bc6h_reverse_bits(b, total_bits);
            }
            *res |= (b << bit_shift) as i32;
        }
    }

    // Sign extend/dequantize endpoints
    let sig = |i: usize| BC6H_MODE_SIG_BITS[mode][i] as i32;
    let num_sig_bits = sig(0);
    if is_signed {
        for comp in 0..3 {
            comps[0][comp][0] = bc6h_sign_extend(comps[0][comp][0], num_sig_bits);
        }
    }
    if is_signed || !is_mode_9_or_10 {
        for subset in 0..num_subsets {
            for comp in 0..3 {
                for lh in (if subset != 0 { 0 } else { 1 })..2 {
                    comps[subset][comp][lh] =
                        bc6h_sign_extend(comps[subset][comp][lh], sig(1 + comp));
                }
            }
        }
    }
    if !is_mode_9_or_10 {
        for subset in 0..num_subsets {
            for comp in 0..3 {
                for lh in (if subset != 0 { 0 } else { 1 })..2 {
                    comps[subset][comp][lh] = bc6h_apply_delta(
                        comps[0][comp][0],
                        comps[subset][comp][lh],
                        num_sig_bits,
                        is_signed,
                    );
                }
            }
        }
    }
    for subset in comps.iter_mut().take(num_subsets) {
        for comp in subset.iter_mut() {
            for v in comp.iter_mut() {
                *v = bc6h_dequantize(*v, num_sig_bits, is_signed);
            }
        }
    }

    // Now unpack weights and output texels
    let weight_bits: i32 = if mode >= 10 { 4 } else { 3 };
    let weights: &[u8] = if mode >= 10 {
        &BC6H_WEIGHT4
    } else {
        &BC6H_WEIGHT3
    };
    let pat = &BC6H_2SUBSET_PATTERNS[(part_index & 31) as usize];
    for (i, d) in dst.iter_mut().enumerate() {
        let mut subset = if num_subsets == 1 {
            if i != 0 { 0 } else { 0x80 }
        } else {
            pat[i] as i32
        };
        let num_bits = weight_bits + if subset & 0x80 != 0 { -1 } else { 0 };
        subset &= 1;
        let weight_index = bc6h_get_bits(num_bits as u32, &mut blo, &mut bhi) as usize;
        let s = subset as usize;
        for c in 0..3 {
            d[c] = bc6h_convert_to_half(
                bc6h_interpolate(comps[s][c][0], comps[s][c][1], weights, weight_index),
                is_signed,
            );
        }
    }
    true
}
