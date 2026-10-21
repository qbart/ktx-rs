//! Port of the ETC1S -> ASTC LDR 4x4 converter and the ASTC block packing
//! helpers from basisu_transcoder.cpp (`BASISD_SUPPORT_ASTC`, with
//! `BASISD_SUPPORT_ASTC_HIGHER_OPAQUE_QUALITY` = 1 as in native builds).

use std::sync::OnceLock;

use super::etc::{DecoderEtcBlock, Endpoint, Selector};
use super::tables::{ETC1_TO_ASTC, ETC1_TO_ASTC_0_255};

const ETC1_TO_ASTC_SELECTOR_RANGES: [(u32, u32); 6] =
    [(0, 3), (1, 3), (0, 2), (1, 2), (2, 3), (0, 1)];
const NUM_RANGES: usize = 6;
const NUM_MAPPINGS: usize = 10;
const ETC1_TO_ASTC_SELECTOR_MAPPINGS: [[u8; 4]; NUM_MAPPINGS] = [
    [0, 0, 1, 1],
    [0, 0, 1, 2],
    [0, 0, 1, 3],
    [0, 0, 2, 3],
    [0, 1, 1, 1],
    [0, 1, 2, 2],
    [0, 1, 2, 3],
    [0, 2, 3, 3],
    [1, 2, 2, 2],
    [1, 2, 3, 3],
];

/// `g_astc_trit_encode`: 5 trits to 8 output bits (3^5 entries).
pub const ASTC_TRIT_ENCODE: [u8; 243] = [
    0, 1, 2, 4, 5, 6, 8, 9, 10, 16, 17, 18, 20, 21, 22, 24, 25, 26, 3, 7, 11, 19, 23, 27, 12, 13,
    14, 32, 33, 34, 36, 37, 38, 40, 41, 42, 48, 49, 50, 52, 53, 54, 56, 57, 58, 35, 39, 43, 51, 55,
    59, 44, 45, 46, 64, 65, 66, 68, 69, 70, 72, 73, 74, 80, 81, 82, 84, 85, 86, 88, 89, 90, 67, 71,
    75, 83, 87, 91, 76, 77, 78, 128, 129, 130, 132, 133, 134, 136, 137, 138, 144, 145, 146, 148,
    149, 150, 152, 153, 154, 131, 135, 139, 147, 151, 155, 140, 141, 142, 160, 161, 162, 164, 165,
    166, 168, 169, 170, 176, 177, 178, 180, 181, 182, 184, 185, 186, 163, 167, 171, 179, 183, 187,
    172, 173, 174, 192, 193, 194, 196, 197, 198, 200, 201, 202, 208, 209, 210, 212, 213, 214, 216,
    217, 218, 195, 199, 203, 211, 215, 219, 204, 205, 206, 96, 97, 98, 100, 101, 102, 104, 105,
    106, 112, 113, 114, 116, 117, 118, 120, 121, 122, 99, 103, 107, 115, 119, 123, 108, 109, 110,
    224, 225, 226, 228, 229, 230, 232, 233, 234, 240, 241, 242, 244, 245, 246, 248, 249, 250, 227,
    231, 235, 243, 247, 251, 236, 237, 238, 28, 29, 30, 60, 61, 62, 92, 93, 94, 156, 157, 158, 188,
    189, 190, 220, 221, 222, 31, 63, 95, 159, 191, 223, 124, 125, 126,
];

/// `astc_extract_bits`: bits [low, high].
#[inline]
pub fn astc_extract_bits(bits: u32, low: u32, high: u32) -> u32 {
    (bits >> low) & ((1u32 << (high - low + 1)) - 1)
}

/// `astc_set_bits`: ORs `value` into the block starting at `bit_pos`.
pub fn astc_set_bits(bytes: &mut [u8; 16], bit_pos: &mut u32, mut value: u32, mut total_bits: u32) {
    while total_bits != 0 {
        let bits_to_write = total_bits.min(8 - (*bit_pos & 7));
        if let Some(b) = bytes.get_mut((*bit_pos >> 3) as usize) {
            *b |= (value << (*bit_pos & 7)) as u8;
        }
        *bit_pos += bits_to_write;
        total_bits -= bits_to_write;
        value = value.checked_shr(bits_to_write).unwrap_or(0);
    }
}

/// `astc_encode_trits`: encodes 5 values of a trit+`n`-bit range.
pub fn astc_encode_trits(bytes: &mut [u8; 16], values: &[u8], bit_pos: &mut u32, n: u32) {
    const MULS: [u32; 5] = [1, 3, 9, 27, 81];
    let mut trits = 0u32;
    let mut bits = [0u32; 5];
    let bit_mask = (1u32 << n) - 1;
    for i in 0..5 {
        let v = values.get(i).copied().unwrap_or(0) as u32;
        trits += (v >> n) * MULS[i];
        bits[i] = v & bit_mask;
    }
    let t = ASTC_TRIT_ENCODE.get(trits as usize).copied().unwrap_or(0) as u32;
    astc_set_bits(
        bytes,
        bit_pos,
        bits[0] | (astc_extract_bits(t, 0, 1) << n) | (bits[1] << (2 + n)),
        n * 2 + 2,
    );
    astc_set_bits(
        bytes,
        bit_pos,
        astc_extract_bits(t, 2, 3)
            | (bits[2] << 2)
            | (astc_extract_bits(t, 4, 4) << (2 + n))
            | (bits[3] << (3 + n))
            | (astc_extract_bits(t, 5, 6) << (3 + n * 2))
            | (bits[4] << (5 + n * 2))
            | (astc_extract_bits(t, 7, 7) << (5 + n * 3)),
        n * 3 + 6,
    );
}

/// `astc_block_params`.
#[derive(Default)]
struct BlockParams {
    endpoints: [u8; 10],
    weights: [u8; 32],
}

const REVERSE_BITS: [u8; 4] = [0, 2, 1, 3];

fn pack_weights2(bytes: &mut [u8; 16], weights: &[u8]) {
    for (i, &w) in weights.iter().enumerate() {
        let ofs = 126 - (i as u32 * 2);
        bytes[(ofs >> 3) as usize] |= REVERSE_BITS[(w & 3) as usize] << (ofs & 7);
    }
}

/// `astc_pack_block_cem_12_weight_range2`.
fn pack_cem_12_weight_range2(bytes: &mut [u8; 16], blk: &BlockParams) {
    *bytes = [0; 16];
    bytes[..8].copy_from_slice(&[0x42, 0x84, 0x01, 0x00, 0x00, 0x00, 0x00, 0xc0]);
    let mut bit_pos = 17;
    astc_encode_trits(bytes, &blk.endpoints[..5], &mut bit_pos, 4);
    astc_encode_trits(bytes, &blk.endpoints[5..], &mut bit_pos, 4);
    pack_weights2(bytes, &blk.weights);
}

/// `astc_pack_block_cem_12_weight_range0`.
fn pack_cem_12_weight_range0(bytes: &mut [u8; 16], blk: &BlockParams) {
    *bytes = [0; 16];
    bytes[..4].copy_from_slice(&[0x41, 0x84, 0x01, 0x00]);
    bytes[11] = 0xc0;
    let mut bit_pos = 17;
    for &e in &blk.endpoints[..8] {
        astc_set_bits(bytes, &mut bit_pos, e as u32, 8);
    }
    for (i, &w) in blk.weights.iter().enumerate() {
        let ofs = 127 - i as u32;
        bytes[(ofs >> 3) as usize] |= w << (ofs & 7);
    }
}

/// `astc_pack_block_cem_4_weight_range2`.
fn pack_cem_4_weight_range2(bytes: &mut [u8; 16], blk: &BlockParams) {
    *bytes = [0; 16];
    bytes[..8].copy_from_slice(&[0x42, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0]);
    let mut bit_pos = 17;
    for &e in &blk.endpoints[..4] {
        astc_set_bits(bytes, &mut bit_pos, e as u32, 8);
    }
    pack_weights2(bytes, &blk.weights);
}

/// `astc_pack_block_cem_8_weight_range2`.
fn pack_cem_8_weight_range2(bytes: &mut [u8; 16], blk: &BlockParams) {
    *bytes = [0; 16];
    bytes[..4].copy_from_slice(&[0x42, 0x00, 0x01, 0x00]);
    let mut bit_pos = 17;
    for &e in &blk.endpoints[..6] {
        astc_set_bits(bytes, &mut bit_pos, e as u32, 8);
    }
    pack_weights2(bytes, &blk.weights[..16]);
}

/// Tables built by `transcoder_init_astc`.
pub struct AstcTables {
    best_grayscale_mapping: [[[u8; NUM_RANGES]; 8]; 32],
    best_grayscale_mapping_0_255: [[[u8; NUM_RANGES]; 8]; 32],
    selector_range_index: [[u32; 4]; 4],
    /// `g_ise_to_unquant`.
    pub ise_to_unquant: [u32; 48],
    single_color_encoding_0: [u8; 256],
    single_color_encoding_1: [(u8, u8); 256],
}

#[inline]
fn table_base(inten: u32, base: u8, range: u32) -> usize {
    (inten as usize * 32 + base as usize) * (NUM_RANGES * NUM_MAPPINGS)
        + range as usize * NUM_MAPPINGS
}

#[inline]
fn sol(t: &[(u8, u8, u16)], idx: usize) -> (u8, u8, u16) {
    t.get(idx).copied().unwrap_or((0, 0, 0))
}

fn best_mappings(t: &[(u8, u8, u16)]) -> [[[u8; NUM_RANGES]; 8]; 32] {
    let mut out = [[[0u8; NUM_RANGES]; 8]; 32];
    for (base_color, o8) in out.iter_mut().enumerate() {
        for (inten_table, o) in o8.iter_mut().enumerate() {
            for (range_index, slot) in o.iter_mut().enumerate() {
                let b = table_base(inten_table as u32, base_color as u8, range_index as u32);
                let mut best_mapping = 0;
                let mut best_err = u32::MAX;
                for m in 0..NUM_MAPPINGS {
                    let e = sol(t, b + m).2 as u32;
                    if e < best_err {
                        best_err = e;
                        best_mapping = m;
                    }
                }
                *slot = best_mapping as u8;
            }
        }
    }
    out
}

/// Port of `transcoder_init_astc`.
pub fn astc_tables() -> &'static AstcTables {
    static T: OnceLock<AstcTables> = OnceLock::new();
    T.get_or_init(|| {
        let mut selector_range_index = [[0u32; 4]; 4];
        for (i, &(l, h)) in ETC1_TO_ASTC_SELECTOR_RANGES.iter().enumerate() {
            selector_range_index[l as usize][h as usize] = i as u32;
        }
        let mut ise_to_unquant = [0u32; 48];
        for trit in 0..3u32 {
            for bit in 0..16u32 {
                let a = if bit & 1 != 0 { 511 } else { 0 };
                let b = (bit >> 1) | ((bit >> 1) << 6);
                let c = 22;
                let mut unq = trit * c + b;
                unq ^= a;
                unq = (a & 0x80) | (unq >> 2);
                ise_to_unquant[(bit | (trit << 4)) as usize] = unq;
            }
        }
        let mut single_color_encoding_1 = [(0u8, 0u8); 256];
        for (i, slot) in single_color_encoding_1.iter_mut().enumerate() {
            let mut lowest_e = i32::MAX;
            for lo in 0..48 {
                for hi in 0..48 {
                    let lo_v = ise_to_unquant[lo] as i32;
                    let hi_v = ise_to_unquant[hi] as i32;
                    let l = lo_v | (lo_v << 8);
                    let h = hi_v | (hi_v << 8);
                    let v = ((l * (64 - 21) + (h * 21) + 32) / 64) >> 8;
                    let e = (v - i as i32).abs();
                    if e < lowest_e {
                        *slot = (lo as u8, hi as u8);
                        lowest_e = e;
                    }
                }
            }
        }
        let mut single_color_encoding_0 = [0u8; 256];
        for (i, slot) in single_color_encoding_0.iter_mut().enumerate() {
            let mut lowest_e = i32::MAX;
            for (lo, &v) in ise_to_unquant.iter().enumerate() {
                let e = (v as i32 - i as i32).abs();
                if e < lowest_e {
                    *slot = lo as u8;
                    lowest_e = e;
                }
            }
        }
        AstcTables {
            best_grayscale_mapping: best_mappings(&ETC1_TO_ASTC),
            best_grayscale_mapping_0_255: best_mappings(&ETC1_TO_ASTC_0_255),
            selector_range_index,
            ise_to_unquant,
            single_color_encoding_0,
            single_color_encoding_1,
        }
    })
}

impl AstcTables {
    fn range(&self, lo: u8, hi: u8) -> u32 {
        self.selector_range_index[(lo & 3) as usize][(hi & 3) as usize]
    }
    fn gray(&self, hq: bool, base: u8, inten: u32, range: u32) -> usize {
        let t = if hq {
            &self.best_grayscale_mapping_0_255
        } else {
            &self.best_grayscale_mapping
        };
        t[(base & 31) as usize][(inten & 7) as usize][range as usize % NUM_RANGES] as usize
    }
    fn unq(&self, e: u8) -> i32 {
        self.ise_to_unquant.get(e as usize).copied().unwrap_or(0) as i32
    }
}

/// Picks the RGB mapping with the lowest summed error (`DO_ITER` loop).
fn best_rgb_mapping(t: &[(u8, u8, u16)], br: usize, bg: usize, bb: usize) -> usize {
    let mut best_err = u32::MAX;
    let mut best_mapping = 0;
    for m in 0..NUM_MAPPINGS {
        let total = sol(t, br + m).2 as u32 + sol(t, bg + m).2 as u32 + sol(t, bb + m).2 as u32;
        if total < best_err {
            best_err = total;
            best_mapping = m;
        }
    }
    best_mapping
}

fn swap_rgb_endpoints(e: &mut [u8; 10]) {
    e.swap(0, 1);
    e.swap(2, 3);
    e.swap(4, 5);
}

/// Port of `convert_etc1s_to_astc_4x4`. When `alpha` is `Some`, it holds the
/// alpha slice's endpoint and selector (C reads their indices from the
/// destination block, written there by a previous `cIndices` pass).
pub fn convert_etc1s_to_astc_4x4(
    out: &mut [u8; 16],
    ep: &Endpoint,
    sel: &Selector,
    alpha: Option<(&Endpoint, &Selector)>,
) {
    let tb = astc_tables();
    let mut blk = BlockParams::default();

    let mut constant_alpha_val: i32 = 255;
    let mut num_unique_alpha_selectors = 1;

    if let Some((aep, asel)) = alpha {
        num_unique_alpha_selectors = asel.num_unique_selectors;
        if num_unique_alpha_selectors == 1 {
            let abc = DecoderEtcBlock::get_block_colors5_g(&aep.color5, aep.inten5 as u32);
            constant_alpha_val = abc[(asel.lo_selector & 3) as usize];
        }
    }

    let base_color = &ep.color5;
    let inten_table = ep.inten5 as u32;
    let low_selector = sel.lo_selector;
    let high_selector = sel.hi_selector;

    if sel.num_unique_selectors == 1 && num_unique_alpha_selectors == 1 {
        // Void-extent (solid color) block.
        let (r, g, b) =
            DecoderEtcBlock::get_block_color5(base_color, inten_table, low_selector as u32);
        *out = [0; 16];
        out[..8].copy_from_slice(&[0xfc, 0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        let mut bit_pos = 64;
        let a = constant_alpha_val as u32;
        astc_set_bits(out, &mut bit_pos, r | (r << 8), 16);
        astc_set_bits(out, &mut bit_pos, g | (g << 8), 16);
        astc_set_bits(out, &mut bit_pos, b | (b << 8), 16);
        astc_set_bits(out, &mut bit_pos, a | (a << 8), 16);
        return;
    } else if sel.num_unique_selectors <= 2 && num_unique_alpha_selectors <= 2 {
        // Block truncation coding: lossless (8-bit endpoints, 1-bit weights).
        let bc = DecoderEtcBlock::get_block_colors5(base_color, inten_table);
        let lo = &bc[(low_selector & 3) as usize];
        let hi = &bc[(high_selector & 3) as usize];
        blk.endpoints[0] = lo.r();
        blk.endpoints[2] = lo.g();
        blk.endpoints[4] = lo.b();
        blk.endpoints[1] = hi.r();
        blk.endpoints[3] = hi.g();
        blk.endpoints[5] = hi.b();

        let s0 = blk.endpoints[0] as i32 + blk.endpoints[2] as i32 + blk.endpoints[4] as i32;
        let s1 = blk.endpoints[1] as i32 + blk.endpoints[3] as i32 + blk.endpoints[5] as i32;
        let mut invert = false;
        if s1 < s0 {
            swap_rgb_endpoints(&mut blk.endpoints);
            invert = true;
        }

        if let Some((aep, asel)) = alpha {
            let abc = DecoderEtcBlock::get_block_colors5_g(&aep.color5, aep.inten5 as u32);
            blk.endpoints[6] = abc[(asel.lo_selector & 3) as usize] as u8;
            blk.endpoints[7] = abc[(asel.hi_selector & 3) as usize] as u8;
            for y in 0..4 {
                for x in 0..4 {
                    let s = asel.get_selector(x, y);
                    let s = u8::from(s == asel.hi_selector as u32);
                    blk.weights[((x + y * 4) * 2 + 1) as usize] = s;
                }
            }
        } else {
            blk.endpoints[6] = 255;
            blk.endpoints[7] = 255;
            for i in 0..16 {
                blk.weights[i * 2 + 1] = 0;
            }
        }

        for y in 0..4 {
            for x in 0..4 {
                let s = sel.get_selector(x, y);
                let mut s = u8::from(s == high_selector as u32);
                if invert {
                    s = 1 - s;
                }
                blk.weights[((x + y * 4) * 2) as usize] = s;
            }
        }
        pack_cem_12_weight_range0(out, &blk);
        return;
    }

    // HIGHER_OPAQUE_QUALITY: grayscale blocks as Luminance+Alpha.
    if base_color.r() == base_color.g() && base_color.r() == base_color.b() {
        if let Some((aep, asel)) = alpha {
            let alpha_inten_table = aep.inten5 as u32;
            if num_unique_alpha_selectors <= 2 {
                let abc = DecoderEtcBlock::get_block_colors5_g(&aep.color5, alpha_inten_table);
                blk.endpoints[2] = abc[(asel.lo_selector & 3) as usize] as u8;
                blk.endpoints[3] = abc[(asel.hi_selector & 3) as usize] as u8;
                for i in 0..16u32 {
                    let s = asel.get_selector(i & 3, i >> 2);
                    blk.weights[(i * 2 + 1) as usize] =
                        if s == asel.hi_selector as u32 { 3 } else { 0 };
                }
            } else {
                let rt = tb.range(asel.lo_selector, asel.hi_selector);
                let b = table_base(alpha_inten_table, aep.color5.g(), rt);
                let best = tb.gray(true, aep.color5.g(), alpha_inten_table, rt);
                let (lo, hi, _) = sol(&ETC1_TO_ASTC_0_255, b + best);
                blk.endpoints[2] = lo;
                blk.endpoints[3] = hi;
                let xlat = &ETC1_TO_ASTC_SELECTOR_MAPPINGS[best];
                for y in 0..4 {
                    for x in 0..4 {
                        let s = asel.get_selector(x, y);
                        blk.weights[((x + y * 4) * 2 + 1) as usize] = xlat[(s & 3) as usize];
                    }
                }
            }
        } else {
            blk.endpoints[2] = 255;
            blk.endpoints[3] = 255;
            for i in 0..16 {
                blk.weights[i * 2 + 1] = 0;
            }
        }

        if sel.num_unique_selectors <= 2 {
            let bc = DecoderEtcBlock::get_block_colors5_g(base_color, inten_table);
            blk.endpoints[0] = bc[(low_selector & 3) as usize] as u8;
            blk.endpoints[1] = bc[(high_selector & 3) as usize] as u8;
            for i in 0..16u32 {
                let s = sel.get_selector(i & 3, i >> 2);
                blk.weights[(i * 2) as usize] = if s == high_selector as u32 { 3 } else { 0 };
            }
        } else {
            let rt = tb.range(low_selector, high_selector);
            let b = table_base(inten_table, base_color.g(), rt);
            let best = tb.gray(true, base_color.g(), inten_table, rt);
            let (lo, hi, _) = sol(&ETC1_TO_ASTC_0_255, b + best);
            blk.endpoints[0] = lo;
            blk.endpoints[1] = hi;
            let xlat = &ETC1_TO_ASTC_SELECTOR_MAPPINGS[best];
            for y in 0..4 {
                for x in 0..4 {
                    let s = sel.get_selector(x, y);
                    blk.weights[((x + y * 4) * 2) as usize] = xlat[(s & 3) as usize];
                }
            }
        }
        pack_cem_4_weight_range2(out, &blk);
        return;
    }

    // HIGHER_OPAQUE_QUALITY: fully opaque blocks with 8-bit RGB endpoints.
    if num_unique_alpha_selectors == 1 && constant_alpha_val == 255 {
        let rt = tb.range(low_selector, high_selector);
        let br = table_base(inten_table, base_color.r(), rt);
        let bg = table_base(inten_table, base_color.g(), rt);
        let bb = table_base(inten_table, base_color.b(), rt);
        let t = &ETC1_TO_ASTC_0_255;
        let best = best_rgb_mapping(t, br, bg, bb);
        blk.endpoints[0] = sol(t, br + best).0;
        blk.endpoints[1] = sol(t, br + best).1;
        blk.endpoints[2] = sol(t, bg + best).0;
        blk.endpoints[3] = sol(t, bg + best).1;
        blk.endpoints[4] = sol(t, bb + best).0;
        blk.endpoints[5] = sol(t, bb + best).1;

        let s0 = blk.endpoints[0] as i32 + blk.endpoints[2] as i32 + blk.endpoints[4] as i32;
        let s1 = blk.endpoints[1] as i32 + blk.endpoints[3] as i32 + blk.endpoints[5] as i32;
        let mut invert = false;
        if s1 < s0 {
            swap_rgb_endpoints(&mut blk.endpoints);
            invert = true;
        }
        let xlat = &ETC1_TO_ASTC_SELECTOR_MAPPINGS[best];
        for y in 0..4 {
            for x in 0..4 {
                let s = sel.get_selector(x, y);
                let mut a = xlat[(s & 3) as usize];
                if invert {
                    a = 3 - a;
                }
                blk.weights[(x + y * 4) as usize] = a;
            }
        }
        pack_cem_8_weight_range2(out, &blk);
        return;
    }

    // Fallback: CEM 12, [0,47] endpoints, 2-bit weights, dual plane.
    if let Some((aep, asel)) = alpha {
        let alpha_inten_table = aep.inten5 as u32;
        let alpha_low_selector = asel.lo_selector;
        let alpha_high_selector = asel.hi_selector;
        if alpha_low_selector == alpha_high_selector {
            let abc = DecoderEtcBlock::get_block_colors5_g(&aep.color5, alpha_inten_table);
            let g = (abc[(alpha_low_selector & 3) as usize] & 255) as usize;
            blk.endpoints[6] = tb.single_color_encoding_1[g].0;
            blk.endpoints[7] = tb.single_color_encoding_1[g].1;
            for i in 0..16 {
                blk.weights[i * 2 + 1] = 1;
            }
        } else if alpha_inten_table >= 7
            && asel.num_unique_selectors == 2
            && alpha_low_selector == 0
            && alpha_high_selector == 3
        {
            let abc = DecoderEtcBlock::get_block_colors5(&aep.color5, alpha_inten_table);
            blk.endpoints[6] = tb.single_color_encoding_0[abc[0].g() as usize];
            blk.endpoints[7] = tb.single_color_encoding_0[abc[3].g() as usize];
            for y in 0..4 {
                for x in 0..4 {
                    let s = asel.get_selector(x, y);
                    blk.weights[((x + y * 4) * 2 + 1) as usize] = if s == alpha_high_selector as u32
                    {
                        3
                    } else {
                        0
                    };
                }
            }
        } else {
            let rt = tb.range(alpha_low_selector, alpha_high_selector);
            let b = table_base(alpha_inten_table, aep.color5.g(), rt);
            let best = tb.gray(false, aep.color5.g(), alpha_inten_table, rt);
            let (lo, hi, _) = sol(&ETC1_TO_ASTC, b + best);
            blk.endpoints[6] = lo;
            blk.endpoints[7] = hi;
            let xlat = &ETC1_TO_ASTC_SELECTOR_MAPPINGS[best];
            for y in 0..4 {
                for x in 0..4 {
                    let s = asel.get_selector(x, y);
                    blk.weights[((x + y * 4) * 2 + 1) as usize] = xlat[(s & 3) as usize];
                }
            }
        }
    } else {
        // 1 is 255 when dequantized.
        blk.endpoints[6] = 1;
        blk.endpoints[7] = 1;
        for i in 0..16 {
            blk.weights[i * 2 + 1] = 0;
        }
    }

    let invert;
    if low_selector == high_selector {
        let bc = DecoderEtcBlock::get_block_colors5(base_color, inten_table);
        let c = &bc[(low_selector & 3) as usize];
        let e1 = &tb.single_color_encoding_1;
        (blk.endpoints[0], blk.endpoints[1]) = e1[c.r() as usize];
        (blk.endpoints[2], blk.endpoints[3]) = e1[c.g() as usize];
        (blk.endpoints[4], blk.endpoints[5]) = e1[c.b() as usize];
        invert = maybe_invert_unq(tb, &mut blk.endpoints);
        for i in 0..16 {
            blk.weights[i * 2] = if invert { 2 } else { 1 };
        }
    } else if inten_table >= 7
        && sel.num_unique_selectors == 2
        && sel.lo_selector == 0
        && sel.hi_selector == 3
    {
        let bc = DecoderEtcBlock::get_block_colors5(base_color, inten_table);
        let e0 = &tb.single_color_encoding_0;
        blk.endpoints[0] = e0[bc[0].r() as usize];
        blk.endpoints[1] = e0[bc[3].r() as usize];
        blk.endpoints[2] = e0[bc[0].g() as usize];
        blk.endpoints[3] = e0[bc[3].g() as usize];
        blk.endpoints[4] = e0[bc[0].b() as usize];
        blk.endpoints[5] = e0[bc[3].b() as usize];
        invert = maybe_invert_unq(tb, &mut blk.endpoints);
        for y in 0..4 {
            for x in 0..4 {
                let s = sel.get_selector(x, y);
                let mut a = if s == low_selector as u32 { 0 } else { 3 };
                if invert {
                    a = 3 - a;
                }
                blk.weights[((x + y * 4) * 2) as usize] = a;
            }
        }
    } else {
        let rt = tb.range(low_selector, high_selector);
        let br = table_base(inten_table, base_color.r(), rt);
        let bg = table_base(inten_table, base_color.g(), rt);
        let bb = table_base(inten_table, base_color.b(), rt);
        let t = &ETC1_TO_ASTC;
        let best = best_rgb_mapping(t, br, bg, bb);
        blk.endpoints[0] = sol(t, br + best).0;
        blk.endpoints[1] = sol(t, br + best).1;
        blk.endpoints[2] = sol(t, bg + best).0;
        blk.endpoints[3] = sol(t, bg + best).1;
        blk.endpoints[4] = sol(t, bb + best).0;
        blk.endpoints[5] = sol(t, bb + best).1;
        invert = maybe_invert_unq(tb, &mut blk.endpoints);
        let xlat = &ETC1_TO_ASTC_SELECTOR_MAPPINGS[best];
        for y in 0..4 {
            for x in 0..4 {
                let s = sel.get_selector(x, y);
                let mut a = xlat[(s & 3) as usize];
                if invert {
                    a = 3 - a;
                }
                blk.weights[((x + y * 4) * 2) as usize] = a;
            }
        }
    }
    pack_cem_12_weight_range2(out, &blk);
}

/// Swaps RGB endpoints when the unquantized high sum is below the low sum.
fn maybe_invert_unq(tb: &AstcTables, e: &mut [u8; 10]) -> bool {
    let s0 = tb.unq(e[0]) + tb.unq(e[2]) + tb.unq(e[4]);
    let s1 = tb.unq(e[1]) + tb.unq(e[3]) + tb.unq(e[5]);
    if s1 < s0 {
        swap_rgb_endpoints(e);
        true
    } else {
        false
    }
}
