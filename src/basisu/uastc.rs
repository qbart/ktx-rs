// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of the UASTC LDR 4x4 core from basisu_transcoder.cpp: ASTC endpoint
//! unquantization tables (`uastc_init`), `unpack_uastc` (block -> ASTC
//! description and -> pixels), `pack_astc_block` and `transcode_uastc_to_astc`.

use std::sync::OnceLock;

use super::astc::{astc_encode_trits, astc_extract_bits, astc_set_bits};
use super::etc::Color32;
use super::uastc_tables::*;

pub const TOTAL_UASTC_MODES: u32 = 19;
pub const UASTC_MODE_INDEX_SOLID_COLOR: u32 = 8;
pub const TOTAL_ASTC_RANGES: usize = 21;

const ZERO_PATTERN: [u8; 16] = [0; 16];
const ZERO_ANCHORS: [u8; 3] = [0; 3];

/// `astc_block_desc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AstcBlockDesc {
    pub weight_range: i32,
    pub subsets: i32,
    pub partition_seed: i32,
    pub cem: i32,
    pub ccs: i32,
    pub dual_plane: bool,
    pub endpoints: [u8; 18],
    pub weights: [u8; 64],
}

impl Default for AstcBlockDesc {
    fn default() -> Self {
        Self {
            weight_range: 0,
            subsets: 0,
            partition_seed: 0,
            cem: 0,
            ccs: 0,
            dual_plane: false,
            endpoints: [0; 18],
            weights: [0; 64],
        }
    }
}

/// `unpacked_uastc_block`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnpackedUastcBlock {
    pub astc: AstcBlockDesc,
    pub mode: u32,
    pub common_pattern: u32,
    pub solid_color: Color32,
    pub bc1_hint0: bool,
    pub bc1_hint1: bool,
    pub etc1_flip: bool,
    pub etc1_diff: bool,
    pub etc1_inten0: u32,
    pub etc1_inten1: u32,
    pub etc1_bias: u32,
    pub etc2_hints: u32,
    pub etc1_selector: u32,
    pub etc1_r: u32,
    pub etc1_g: u32,
    pub etc1_b: u32,
}

/// `g_astc_endpoint_unquant_params` (B bit string, C).
const ASTC_ENDPOINT_UNQUANT_PARAMS: [(&[u8; 9], u32); TOTAL_ASTC_RANGES] = [
    (b"000000000", 0),
    (b"000000000", 0),
    (b"000000000", 0),
    (b"000000000", 0),
    (b"000000000", 204),
    (b"000000000", 0),
    (b"000000000", 113),
    (b"b000b0bb0", 93),
    (b"000000000", 0),
    (b"b0000bb00", 54),
    (b"cb000cbcb", 44),
    (b"000000000", 0),
    (b"cb0000cbc", 26),
    (b"dcb000dcb", 22),
    (b"000000000", 0),
    (b"dcb0000dc", 13),
    (b"edcb000ed", 11),
    (b"000000000", 0),
    (b"edcb0000e", 6),
    (b"fedcb000f", 5),
    (b"000000000", 0),
];

#[inline]
fn bise(range: u32) -> [u32; 3] {
    ASTC_BISE_RANGE_TABLE
        .get(range as usize)
        .copied()
        .unwrap_or([0, 0, 0])
}

/// `astc_get_levels`.
pub fn astc_get_levels(range: u32) -> u32 {
    let [b, t, q] = bise(range);
    (1 + 2 * t + 4 * q) << b
}

/// `astc_is_valid_endpoint_range`.
pub fn astc_is_valid_endpoint_range(range: u32) -> bool {
    let [_, t, q] = bise(range);
    if t == 0 && q == 0 {
        return true;
    }
    ASTC_ENDPOINT_UNQUANT_PARAMS
        .get(range as usize)
        .is_some_and(|p| p.1 != 0)
}

/// `unquant_astc_endpoint`.
pub fn unquant_astc_endpoint(
    packed_bits: u32,
    packed_trits: u32,
    packed_quints: u32,
    range: u32,
) -> u32 {
    let [bits, trits, quints] = bise(range);
    if trits == 0 && quints == 0 {
        let mut val = 0u32;
        let mut bits_left: i32 = 8;
        while bits_left > 0 {
            let mut v = packed_bits;
            let n = bits_left.min(bits as i32);
            if n <= 0 {
                break;
            }
            if n < bits as i32 {
                v >>= bits as i32 - n;
            }
            val |= v << (bits_left - n);
            bits_left -= n;
        }
        val
    } else {
        let (b_str, c) = ASTC_ENDPOINT_UNQUANT_PARAMS
            .get(range as usize)
            .copied()
            .unwrap_or((b"000000000", 0));
        let a = if packed_bits & 1 != 0 { 511 } else { 0 };
        let d = if trits != 0 {
            packed_trits
        } else {
            packed_quints
        };
        let mut b = 0u32;
        for &ch in b_str.iter() {
            b <<= 1;
            if ch != b'0' {
                b |= (packed_bits >> (ch - b'a')) & 1;
            }
        }
        let mut val = d * c + b;
        val ^= a;
        (a & 0x80) | (val >> 2)
    }
}

/// `unquant_astc_endpoint_val`.
pub fn unquant_astc_endpoint_val(packed_val: u32, range: u32) -> u32 {
    let [bits, trits, quints] = bise(range);
    let mask = (1u32 << bits) - 1;
    if trits == 0 && quints == 0 {
        unquant_astc_endpoint(packed_val, 0, 0, range)
    } else if trits != 0 {
        unquant_astc_endpoint(packed_val & mask, packed_val >> bits, 0, range)
    } else {
        unquant_astc_endpoint(packed_val & mask, 0, packed_val >> bits, range)
    }
}

/// `endpoint_err`.
#[derive(Debug, Clone, Copy, Default)]
pub struct EndpointErr {
    pub error: u16,
    pub lo: u8,
    pub hi: u8,
}

/// Tables built by `uastc_init`.
pub struct UastcTables {
    /// `g_astc_unquant[range][index]` as (unquant, sorted index).
    pub astc_unquant: [[(u8, u8); 256]; TOTAL_ASTC_RANGES],
    pub bc7_mode_6_optimal_endpoints: [[EndpointErr; 2]; 256],
    pub bc7_mode_5_optimal_endpoints: [EndpointErr; 256],
}

impl UastcTables {
    #[inline]
    pub fn unquant(&self, range: u32, v: u8) -> u32 {
        self.astc_unquant
            .get(range as usize)
            .map_or(0, |t| t[v as usize].0 as u32)
    }
}

/// Port of `uastc_init`.
pub fn uastc_tables() -> &'static UastcTables {
    static T: OnceLock<Box<UastcTables>> = OnceLock::new();
    T.get_or_init(|| {
        let mut astc_unquant = [[(0u8, 0u8); 256]; TOTAL_ASTC_RANGES];
        for (range, table) in astc_unquant.iter_mut().enumerate() {
            let range = range as u32;
            if !astc_is_valid_endpoint_range(range) {
                continue;
            }
            let levels = astc_get_levels(range).min(256);
            let mut vals: Vec<u32> = (0..levels)
                .map(|i| (unquant_astc_endpoint_val(i, range) << 8) | i)
                .collect();
            vals.sort_unstable();
            for (i, &v) in vals.iter().enumerate() {
                let order = (v & 0xFF) as usize;
                table[order] = ((v >> 8) as u8, i as u8);
            }
        }

        let mut m6 = [[EndpointErr::default(); 2]; 256];
        let w6 = BC7_WEIGHTS4[5] as i32;
        for (c, slot) in m6.iter_mut().enumerate() {
            for (lp, e) in slot.iter_mut().enumerate() {
                let mut best = EndpointErr {
                    error: u16::MAX,
                    lo: 0,
                    hi: 0,
                };
                for l in 0..128i32 {
                    let low = (l << 1) | lp as i32;
                    for h in 0..128i32 {
                        let high = (h << 1) | lp as i32;
                        let k = (low * (64 - w6) + high * w6 + 32) >> 6;
                        let err = (k - c as i32) * (k - c as i32);
                        if err < best.error as i32 {
                            best = EndpointErr {
                                error: err as u16,
                                lo: l as u8,
                                hi: h as u8,
                            };
                        }
                    }
                }
                *e = best;
            }
        }

        let mut m5 = [EndpointErr::default(); 256];
        let w5 = BC7_WEIGHTS2[1] as i32;
        for (c, e) in m5.iter_mut().enumerate() {
            let mut best = EndpointErr {
                error: u16::MAX,
                lo: 0,
                hi: 0,
            };
            for l in 0..128i32 {
                let low = (l << 1) | (l >> 6);
                for h in 0..128i32 {
                    let high = (h << 1) | (h >> 6);
                    let k = (low * (64 - w5) + high * w5 + 32) >> 6;
                    let err = (k - c as i32) * (k - c as i32);
                    if err < best.error as i32 {
                        best = EndpointErr {
                            error: err as u16,
                            lo: l as u8,
                            hi: h as u8,
                        };
                    }
                }
            }
            *e = best;
        }

        Box::new(UastcTables {
            astc_unquant,
            bc7_mode_6_optimal_endpoints: m6,
            bc7_mode_5_optimal_endpoints: m5,
        })
    })
}

/// `astc_interpolate`.
#[inline]
pub fn astc_interpolate(mut l: u32, mut h: u32, w: u32, srgb: bool) -> u32 {
    if srgb {
        l = (l << 8) | 0x80;
        h = (h << 8) | 0x80;
    } else {
        l = (l << 8) | l;
        h = (h << 8) | h;
    }
    let k = (l * (64 - w) + h * w + 32) >> 6;
    k >> 8
}

/// `get_anchor_indices`: returns (anchor indices, partition pattern).
pub fn get_anchor_indices(
    subsets: u32,
    mode: u32,
    common_pattern: u32,
) -> (&'static [u8; 3], &'static [u8; 16]) {
    let cp = common_pattern as usize;
    if subsets >= 2 {
        if subsets == 3 {
            if let (Some(p), Some(a)) = (
                ASTC_BC7_PATTERNS3.get(cp),
                ASTC_BC7_PATTERN3_ANCHORS.get(cp),
            ) {
                return (a, p);
            }
        } else if mode == 7 {
            if let (Some(p), Some(a)) = (
                BC7_3_ASTC2_PATTERNS2.get(cp),
                BC7_3_ASTC2_PATTERNS2_ANCHORS.get(cp),
            ) {
                return (a, p);
            }
        } else if let (Some(p), Some(a)) = (
            ASTC_BC7_PATTERNS2.get(cp),
            ASTC_BC7_PATTERN2_ANCHORS.get(cp),
        ) {
            return (a, p);
        }
    }
    (&ZERO_ANCHORS, &ZERO_PATTERN)
}

#[inline]
fn byte(buf: &[u8; 16], i: u32) -> u32 {
    buf.get(i as usize).copied().unwrap_or(0) as u32
}

#[inline]
fn read_bit(buf: &[u8; 16], bit_offset: &mut u32) -> u32 {
    let v = byte(buf, *bit_offset >> 3) >> (*bit_offset & 7);
    *bit_offset += 1;
    v & 1
}

/// `read_bits1_to_9` / `read_bits1_to_9_fst` (identical results).
#[inline]
fn read_bits1_to_9(buf: &[u8; 16], bit_offset: &mut u32, codesize: u32) -> u32 {
    if codesize == 0 {
        return 0;
    }
    let ofs = *bit_offset;
    let w = byte(buf, ofs >> 3) | (byte(buf, (ofs >> 3) + 1) << 8);
    *bit_offset += codesize;
    (w >> (ofs & 7)) & ((1u32 << codesize) - 1)
}

#[inline]
fn md(t: &[u8; 19], mode: u32) -> u32 {
    t.get(mode as usize).copied().unwrap_or(0) as u32
}

/// Port of `unpack_uastc(const uastc_block&, unpacked_uastc_block&, ...)`.
pub fn unpack_uastc(
    blk: &[u8; 16],
    unpacked: &mut UnpackedUastcBlock,
    blue_contract_check: bool,
    read_hints: bool,
) -> bool {
    let mode = UASTC_HUFF_MODES[(blk[0] & 127) as usize] as u32;
    if mode >= TOTAL_UASTC_MODES {
        return false;
    }
    unpacked.mode = mode;
    unpacked.common_pattern = 0;

    let mut bit_ofs = UASTC_MODE_HUFF_CODES[mode as usize][1];

    if mode == UASTC_MODE_INDEX_SOLID_COLOR {
        let r = read_bits1_to_9(blk, &mut bit_ofs, 8);
        let g = read_bits1_to_9(blk, &mut bit_ofs, 8);
        let b = read_bits1_to_9(blk, &mut bit_ofs, 8);
        let a = read_bits1_to_9(blk, &mut bit_ofs, 8);
        unpacked.solid_color = Color32::new(r, g, b, a);
        if read_hints {
            unpacked.etc1_flip = false;
            unpacked.etc1_diff = read_bit(blk, &mut bit_ofs) != 0;
            unpacked.etc1_inten0 = read_bits1_to_9(blk, &mut bit_ofs, 3);
            unpacked.etc1_inten1 = 0;
            unpacked.etc1_selector = read_bits1_to_9(blk, &mut bit_ofs, 2);
            unpacked.etc1_r = read_bits1_to_9(blk, &mut bit_ofs, 5);
            unpacked.etc1_g = read_bits1_to_9(blk, &mut bit_ofs, 5);
            unpacked.etc1_b = read_bits1_to_9(blk, &mut bit_ofs, 5);
            unpacked.etc1_bias = 0;
            unpacked.etc2_hints = 0;
        }
        return true;
    }

    if read_hints {
        unpacked.bc1_hint0 =
            md(&UASTC_MODE_HAS_BC1_HINT0, mode) != 0 && read_bit(blk, &mut bit_ofs) != 0;
        unpacked.bc1_hint1 =
            md(&UASTC_MODE_HAS_BC1_HINT1, mode) != 0 && read_bit(blk, &mut bit_ofs) != 0;
        unpacked.etc1_flip = read_bit(blk, &mut bit_ofs) != 0;
        unpacked.etc1_diff = read_bit(blk, &mut bit_ofs) != 0;
        unpacked.etc1_inten0 = read_bits1_to_9(blk, &mut bit_ofs, 3);
        unpacked.etc1_inten1 = read_bits1_to_9(blk, &mut bit_ofs, 3);
        unpacked.etc1_bias = if md(&UASTC_MODE_HAS_ETC1_BIAS, mode) != 0 {
            read_bits1_to_9(blk, &mut bit_ofs, 5)
        } else {
            0
        };
        unpacked.etc2_hints = if md(&UASTC_MODE_HAS_ALPHA, mode) != 0 {
            read_bits1_to_9(blk, &mut bit_ofs, 8)
        } else {
            0
        };
    } else {
        bit_ofs += md(&UASTC_MODE_TOTAL_HINT_BITS, mode);
    }

    let mut subsets = 1u32;
    match mode {
        2 | 4 | 7 | 9 | 16 => {
            unpacked.common_pattern = read_bits1_to_9(blk, &mut bit_ofs, 5);
            subsets = 2;
        }
        3 => {
            unpacked.common_pattern = read_bits1_to_9(blk, &mut bit_ofs, 4);
            subsets = 3;
        }
        _ => {}
    }

    let cp = unpacked.common_pattern as usize;
    let part_seed = match mode {
        2 | 4 | 9 | 16 => match ASTC_BC7_COMMON_PARTITIONS2.get(cp) {
            Some(p) => p.1 as u32,
            None => return false,
        },
        3 => match ASTC_BC7_COMMON_PARTITIONS3.get(cp) {
            Some(p) => p.1 as u32,
            None => return false,
        },
        7 => match BC7_3_ASTC2_COMMON_PARTITIONS.get(cp) {
            Some(p) => p.1 as u32,
            None => return false,
        },
        _ => 0,
    };

    let mut total_planes = 1u32;
    match mode {
        6 | 11 | 13 => {
            unpacked.astc.ccs = read_bits1_to_9(blk, &mut bit_ofs, 2) as i32;
            total_planes = 2;
        }
        17 => {
            unpacked.astc.ccs = 3;
            total_planes = 2;
        }
        _ => {}
    }

    unpacked.astc.dual_plane = total_planes == 2;
    unpacked.astc.subsets = subsets as i32;
    unpacked.astc.partition_seed = part_seed as i32;

    let total_comps = md(&UASTC_MODE_COMPS, mode);
    let weight_bits = md(&UASTC_MODE_WEIGHT_BITS, mode);
    unpacked.astc.weight_range = md(&UASTC_MODE_WEIGHT_RANGES, mode) as i32;

    let total_values = total_comps * 2 * subsets;
    let endpoint_range = md(&UASTC_MODE_ENDPOINT_RANGES, mode);
    unpacked.astc.cem = md(&UASTC_MODE_CEM, mode) as i32;

    let [ep_bits, ep_trits, ep_quints] = bise(endpoint_range);

    let (mut total_tqs, mut bundle_size, mut mul) = (0u32, 0u32, 0u32);
    if ep_trits != 0 {
        total_tqs = total_values.div_ceil(5);
        bundle_size = 5;
        mul = 3;
    } else if ep_quints != 0 {
        total_tqs = total_values.div_ceil(3);
        bundle_size = 3;
        mul = 5;
    }

    let mut tq_values = [0u32; 8];
    for i in 0..total_tqs.min(8) {
        let mut num_bits = if ep_trits != 0 { 8 } else { 7 };
        if i == total_tqs - 1 {
            let num_remaining = total_values - (total_tqs - 1) * bundle_size;
            if ep_trits != 0 {
                match num_remaining {
                    1 => num_bits = 2,
                    2 => num_bits = 4,
                    3 => num_bits = 5,
                    4 => num_bits = 7,
                    _ => {}
                }
            } else if ep_quints != 0 {
                match num_remaining {
                    1 => num_bits = 3,
                    2 => num_bits = 5,
                    _ => {}
                }
            }
        }
        tq_values[i as usize] = read_bits1_to_9(blk, &mut bit_ofs, num_bits);
    }

    let (mut accum, mut accum_remaining, mut next_tq_index) = (0u32, 0u32, 0usize);
    for i in 0..total_values.min(18) {
        let mut value = read_bits1_to_9(blk, &mut bit_ofs, ep_bits);
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
        unpacked.astc.endpoints[i as usize] = value as u8;
    }

    let (anchors, pattern) = get_anchor_indices(subsets, mode, unpacked.common_pattern);

    let w = &mut unpacked.astc.weights;
    if mode == 18 {
        // Mode 18 is the only mode with more than 64 weight bits.
        for (i, wi) in w.iter_mut().take(16).enumerate() {
            let n = if i != 0 { weight_bits } else { weight_bits - 1 };
            *wi = read_bits1_to_9(blk, &mut bit_ofs, n) as u8;
        }
    } else {
        // All other modes have <= 64 weight bits.
        let all = u128::from_le_bytes(*blk);
        let bits = all.checked_shr(bit_ofs).unwrap_or(0) as u64;
        let mut ofs = 0u32;
        let mask = (1u64 << weight_bits) - 1;
        let anchor_mask = (1u64 << (weight_bits.max(1) - 1)) - 1;
        let mut take = |m: u64, n: u32| {
            let v = (bits.checked_shr(ofs).unwrap_or(0) & m) as u8;
            ofs += n;
            v
        };
        if total_planes == 2 {
            w[0] = take(anchor_mask, weight_bits - 1);
            w[1] = take(anchor_mask, weight_bits - 1);
            for wi in w.iter_mut().take(32).skip(2) {
                *wi = take(mask, weight_bits);
            }
        } else if subsets == 1 {
            w[0] = take(anchor_mask, weight_bits - 1);
            for wi in w.iter_mut().take(16).skip(1) {
                *wi = take(mask, weight_bits);
            }
        } else {
            for (i, wi) in w.iter_mut().take(16).enumerate() {
                if anchors.iter().any(|&a| a as usize == i) {
                    *wi = take(anchor_mask, weight_bits - 1);
                } else {
                    *wi = take(mask, weight_bits);
                }
            }
        }
    }

    if blue_contract_check && total_comps >= 3 {
        let tb = uastc_tables();
        let mut invert_subset = [false; 3];
        let mut any_flag = false;
        for (s, inv) in invert_subset.iter_mut().enumerate().take(subsets as usize) {
            let base = s * total_comps as usize * 2;
            let e = &unpacked.astc.endpoints;
            let uq =
                |k: usize| tb.unquant(endpoint_range, e.get(base + k).copied().unwrap_or(0)) as i32;
            let s0 = uq(0) + uq(2) + uq(4);
            let s1 = uq(1) + uq(3) + uq(5);
            if s1 < s0 {
                for c in 0..total_comps as usize {
                    unpacked.astc.endpoints.swap(base + c * 2, base + c * 2 + 1);
                }
                *inv = true;
                any_flag = true;
            }
        }
        if any_flag {
            let weight_mask = ((1u32 << weight_bits) - 1) as u8;
            let tp = total_planes as usize;
            for i in 0..16 {
                if invert_subset[(pattern[i] as usize).min(2)] {
                    let w = &mut unpacked.astc.weights;
                    w[i * tp] = weight_mask.wrapping_sub(w[i * tp]);
                    if tp == 2 {
                        w[i * tp + 1] = weight_mask.wrapping_sub(w[i * tp + 1]);
                    }
                }
            }
        }
    }
    true
}

fn astc_weight_table(weight_bits: u32) -> &'static [u32] {
    match weight_bits {
        1 => &BC7_WEIGHTS1,
        2 => &BC7_WEIGHTS2,
        3 => &BC7_WEIGHTS3,
        4 => &ASTC_WEIGHTS4,
        5 => &ASTC_WEIGHTS5,
        _ => &[],
    }
}

/// Port of `unpack_uastc(mode, common_pattern, solid_color, astc, pPixels, srgb)`.
pub fn unpack_uastc_pixels_desc(
    mode: u32,
    common_pattern: u32,
    solid_color: &Color32,
    astc: &AstcBlockDesc,
    pixels: &mut [Color32; 16],
    srgb: bool,
) -> bool {
    if mode == UASTC_MODE_INDEX_SOLID_COLOR {
        *pixels = [*solid_color; 16];
        return true;
    }
    let tb = uastc_tables();
    let total_subsets = md(&UASTC_MODE_SUBSETS, mode) as usize;
    let total_comps = md(&UASTC_MODE_COMPS, mode).min(4) as usize;
    let endpoint_range = md(&UASTC_MODE_ENDPOINT_RANGES, mode);
    let total_planes = md(&UASTC_MODE_PLANES, mode);
    let weight_bits = md(&UASTC_MODE_WEIGHT_BITS, mode);
    let weight_levels = 1usize << weight_bits;

    let ep =
        |k: usize| tb.unquant(endpoint_range, astc.endpoints.get(k).copied().unwrap_or(0)) as u8;
    let mut endpoints = [[Color32::default(); 2]; 3];
    for (s, e) in endpoints.iter_mut().enumerate().take(total_subsets) {
        let base = s * total_comps * 2;
        if total_comps == 2 {
            let (ll, lh, al, ah) = (ep(base), ep(base + 1), ep(base + 2), ep(base + 3));
            e[0].c = [ll, ll, ll, al];
            e[1].c = [lh, lh, lh, ah];
        } else {
            for c in 0..4 {
                if c < total_comps {
                    e[0].c[c] = ep(base + c * 2);
                    e[1].c[c] = ep(base + c * 2 + 1);
                } else {
                    e[0].c[c] = 255;
                    e[1].c[c] = 255;
                }
            }
        }
    }

    let weights = astc_weight_table(weight_bits);
    let mut block_colors = [[Color32::default(); 32]; 3];
    for s in 0..total_subsets {
        for (l, &wl) in weights.iter().enumerate().take(weight_levels.min(32)) {
            let (e0, e1) = (&endpoints[s][0], &endpoints[s][1]);
            let bc = &mut block_colors[s][l];
            if total_comps == 2 {
                let lc = astc_interpolate(e0.c[0] as u32, e1.c[0] as u32, wl, srgb) as u8;
                let ac = astc_interpolate(e0.c[3] as u32, e1.c[3] as u32, wl, srgb) as u8;
                bc.c = [lc, lc, lc, ac];
            } else {
                for c in 0..4 {
                    bc.c[c] = if c < total_comps {
                        astc_interpolate(e0.c[c] as u32, e1.c[c] as u32, wl, srgb) as u8
                    } else {
                        255
                    };
                }
            }
        }
    }

    let (_, pattern) = if total_subsets >= 2 {
        get_anchor_indices(total_subsets as u32, mode, common_pattern)
    } else {
        (&ZERO_ANCHORS, &ZERO_PATTERN)
    };

    let wv = |i: usize| (astc.weights[i] as usize).min(31);
    if total_planes == 1 {
        for (i, px) in pixels.iter_mut().enumerate() {
            *px = block_colors[(pattern[i] as usize).min(2)][wv(i)];
        }
    } else {
        for (i, px) in pixels.iter_mut().enumerate() {
            let w0 = wv(i * 2);
            let w1 = wv(i * 2 + 1);
            for c in 0..4 {
                px.c[c] = if c as i32 == astc.ccs {
                    block_colors[0][w1].c[c]
                } else {
                    block_colors[0][w0].c[c]
                };
            }
        }
    }
    true
}

/// Port of `unpack_uastc(const unpacked_uastc_block&, color32*, bool)`.
pub fn unpack_uastc_unpacked_pixels(
    u: &UnpackedUastcBlock,
    pixels: &mut [Color32; 16],
    srgb: bool,
) -> bool {
    unpack_uastc_pixels_desc(
        u.mode,
        u.common_pattern,
        &u.solid_color,
        &u.astc,
        pixels,
        srgb,
    )
}

/// Port of `unpack_uastc(const uastc_block&, color32*, bool)`.
pub fn unpack_uastc_pixels(blk: &[u8; 16], pixels: &mut [Color32; 16], srgb: bool) -> bool {
    let mut u = UnpackedUastcBlock::default();
    if !unpack_uastc(blk, &mut u, false, false) {
        return false;
    }
    unpack_uastc_unpacked_pixels(&u, pixels, srgb)
}

/// `astc_set_bits_1_to_9`.
fn astc_set_bits_1_to_9(buf: &mut [u8; 16], bit_offset: &mut u32, code: u32, codesize: u32) {
    if codesize != 0 {
        let byte_bit_offset = *bit_offset & 7;
        let val = code << byte_bit_offset;
        let index = (*bit_offset >> 3) as usize;
        if let Some(b) = buf.get_mut(index) {
            *b |= val as u8;
        }
        if codesize > 8 - byte_bit_offset
            && let Some(b) = buf.get_mut(index + 1)
        {
            *b |= (val >> 8) as u8;
        }
        *bit_offset += codesize;
    }
}

/// `astc_encode_quints`.
fn astc_encode_quints(out: &mut [u8; 16], values: &[u8; 5], bit_pos: &mut u32, n: u32) {
    const MULS: [u32; 3] = [1, 5, 25];
    let mut quints = 0u32;
    let mut bits = [0u32; 3];
    let bit_mask = (1u32 << n) - 1;
    for i in 0..3 {
        let v = values[i] as u32;
        quints += (v >> n) * MULS[i];
        bits[i] = v & bit_mask;
    }
    let t = ASTC_QUINT_ENCODE.get(quints as usize).copied().unwrap_or(0) as u32;
    astc_set_bits(
        out,
        bit_pos,
        bits[0]
            | (astc_extract_bits(t, 0, 2) << n)
            | (bits[1] << (3 + n))
            | (astc_extract_bits(t, 3, 4) << (3 + n * 2))
            | (bits[2] << (5 + n * 2))
            | (astc_extract_bits(t, 5, 6) << (5 + n * 3)),
        7 + n * 3,
    );
}

/// `astc_pack_bise`.
fn astc_pack_bise(
    dst: &mut [u8; 16],
    src_vals: &[u8],
    mut bit_pos: u32,
    num_vals: usize,
    range: u32,
) {
    let mut temp = [0u8; 16];
    let [num_bits, trits, quints] = bise(range);
    let group_size = if trits != 0 {
        5
    } else if quints != 0 {
        3
    } else {
        0
    };
    if group_size != 0 {
        let total_groups = num_vals.div_ceil(group_size);
        for g in 0..total_groups {
            let mut vals = [0u8; 5];
            let limit = group_size.min(num_vals - g * group_size);
            for (i, v) in vals.iter_mut().enumerate().take(limit) {
                *v = src_vals.get(g * group_size + i).copied().unwrap_or(0);
            }
            if group_size == 5 {
                astc_encode_trits(&mut temp, &vals, &mut bit_pos, num_bits);
            } else {
                astc_encode_quints(&mut temp, &vals, &mut bit_pos, num_bits);
            }
        }
    } else {
        for &v in src_vals.iter().take(num_vals) {
            astc_set_bits_1_to_9(&mut temp, &mut bit_pos, v as u32, num_bits);
        }
    }
    for (d, t) in dst.iter_mut().zip(temp.iter()) {
        *d |= *t;
    }
}

/// Port of `pack_astc_solid_block`.
pub fn pack_astc_solid_block(dst: &mut [u8; 16], color: &Color32) {
    let (r, g, b, a) = (
        color.r() as u32,
        color.g() as u32,
        color.b() as u32,
        color.a() as u32,
    );
    *dst = [0; 16];
    dst[..8].copy_from_slice(&[0xfc, 0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    let mut bit_pos = 64;
    astc_set_bits(dst, &mut bit_pos, r | (r << 8), 16);
    astc_set_bits(dst, &mut bit_pos, g | (g << 8), 16);
    astc_set_bits(dst, &mut bit_pos, b | (b << 8), 16);
    astc_set_bits(dst, &mut bit_pos, a | (a << 8), 16);
}

const REVERSE_BITS2: [u8; 4] = [0, 2, 1, 3];
const REVERSE_BITS3: [u8; 8] = [0, 4, 2, 6, 1, 5, 3, 7];
const REVERSE_BITS4: [u8; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];
const REVERSE_BITS5: [u8; 32] = [
    0, 16, 8, 24, 4, 20, 12, 28, 2, 18, 10, 26, 6, 22, 14, 30, 1, 17, 9, 25, 5, 21, 13, 29, 3, 19,
    11, 27, 7, 23, 15, 31,
];

/// Port of `pack_astc_block`.
pub fn pack_astc_block(dst: &mut [u8; 16], blk: &AstcBlockDesc, uastc_mode: u32) -> bool {
    let total_weights = if blk.dual_plane { 32 } else { 16 };
    let mode = UASTC_MODE_ASTC_BLOCK_MODE
        .get(uastc_mode as usize)
        .copied()
        .unwrap_or(0);
    *dst = [0; 16];
    dst[0] = mode as u8;
    dst[1] = (mode >> 8) as u8;

    let mut bit_pos = 11u32;
    let bits_per_weight = bise(blk.weight_range as u32)[0];

    astc_set_bits_1_to_9(dst, &mut bit_pos, (blk.subsets - 1) as u32, 2);
    if blk.subsets == 1 {
        astc_set_bits_1_to_9(dst, &mut bit_pos, blk.cem as u32, 4);
    } else {
        astc_set_bits(dst, &mut bit_pos, blk.partition_seed as u32, 10);
        astc_set_bits_1_to_9(dst, &mut bit_pos, ((blk.cem << 2) & 63) as u32, 6);
    }

    if blk.dual_plane {
        let total_weight_bits = total_weights * bits_per_weight;
        let mut ccs_bit_pos = 128u32.wrapping_sub(total_weight_bits + 2);
        astc_set_bits_1_to_9(dst, &mut ccs_bit_pos, blk.ccs as u32, 2);
    }

    let num_cem_pairs = ((1 + (blk.cem >> 2)) * blk.subsets) as usize;
    astc_pack_bise(
        dst,
        &blk.endpoints,
        bit_pos,
        num_cem_pairs * 2,
        md(&UASTC_MODE_ENDPOINT_RANGES, uastc_mode),
    );

    // Weight bits in reverse bit order, from the top of the block down.
    let n = bits_per_weight;
    for i in 0..total_weights {
        let w = blk.weights[i as usize] as usize;
        let rev = match n {
            1 => w as u32 & 1,
            2 => REVERSE_BITS2[w & 3] as u32,
            3 => REVERSE_BITS3[w & 7] as u32,
            4 => REVERSE_BITS4[w & 15] as u32,
            5 => REVERSE_BITS5[w & 31] as u32,
            _ => return true,
        };
        let ofs = 128 - n - i * n;
        let v = rev << (ofs & 7);
        let index = (ofs >> 3) as usize;
        if let Some(b) = dst.get_mut(index) {
            *b |= v as u8;
        }
        if (n == 3 || n == 5)
            && let Some(b) = dst.get_mut(index + 1)
        {
            *b |= (v >> 8) as u8;
        }
    }
    true
}

/// Port of `transcode_uastc_to_astc`.
pub fn transcode_uastc_to_astc(src: &[u8; 16], dst: &mut [u8; 16]) -> bool {
    let mut u = UnpackedUastcBlock::default();
    if !unpack_uastc(src, &mut u, true, false) {
        return false;
    }
    if u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
        pack_astc_solid_block(dst, &u.solid_color);
        true
    } else {
        pack_astc_block(dst, &u.astc, u.mode)
    }
}
