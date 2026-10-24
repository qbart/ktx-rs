//! Port of `encoder/basisu_etc.h` / `basisu_etc.cpp`: the ETC1 block encoder used by the
//! ETC1S frontend/backend and the UASTC encoder (`etc_block`, `etc1_optimizer`,
//! `pack_etc1_block_solid_color`, `pack_eac_a8`).
//!
//! KTX builds basisu with SSE 4.1; the SSE kernels used by `etc1_optimizer` compute the
//! same integer distances as the scalar code, so the scalar path is ported.

#![allow(clippy::needless_range_loop)]

use std::sync::OnceLock;

use super::eac::EAC_MODIFIER_TABLE;
use super::enc::{
    ColorRgba, color_distance, color_rgba, color_rgba_noclamp, hash_hsieh, indirect_sort,
};
use super::etc_enc_tables::*;

pub const C_ETC1_BYTES_PER_BLOCK: u32 = 8;
pub const C_ETC1_SELECTOR_BITS: u32 = 2;
pub const C_ETC1_SELECTOR_VALUES: u32 = 4;
pub const C_ETC1_SELECTOR_MASK: u32 = 3;
pub const C_ETC1_BLOCK_SHIFT: u32 = 2;
pub const C_ETC1_BLOCK_SIZE: u32 = 4;
pub const C_ETC1_LSB_SELECTOR_INDICES_BIT_OFFSET: u32 = 0;
pub const C_ETC1_MSB_SELECTOR_INDICES_BIT_OFFSET: u32 = 16;
pub const C_ETC1_FLIP_BIT_OFFSET: u32 = 32;
pub const C_ETC1_DIFF_BIT_OFFSET: u32 = 33;
pub const C_ETC1_INTEN_MODIFIER_NUM_BITS: u32 = 3;
pub const C_ETC1_INTEN_MODIFIER_VALUES: u32 = 8;
pub const C_ETC1_RIGHT_INTEN_MODIFIER_TABLE_BIT_OFFSET: u32 = 34;
pub const C_ETC1_LEFT_INTEN_MODIFIER_TABLE_BIT_OFFSET: u32 = 37;
pub const C_ETC1_BASE_COLOR_COMP_NUM_BITS: u32 = 5;
pub const C_ETC1_BASE_COLOR_COMP_MAX: u32 = 32;
pub const C_ETC1_DELTA_COLOR_COMP_NUM_BITS: u32 = 3;
pub const C_ETC1_DELTA_COLOR_COMP: u32 = 8;
pub const C_ETC1_DELTA_COLOR_COMP_MAX: u32 = 8;
pub const C_ETC1_BASE_COLOR5_R_BIT_OFFSET: u32 = 59;
pub const C_ETC1_BASE_COLOR5_G_BIT_OFFSET: u32 = 51;
pub const C_ETC1_BASE_COLOR5_B_BIT_OFFSET: u32 = 43;
pub const C_ETC1_DELTA_COLOR3_R_BIT_OFFSET: u32 = 56;
pub const C_ETC1_DELTA_COLOR3_G_BIT_OFFSET: u32 = 48;
pub const C_ETC1_DELTA_COLOR3_B_BIT_OFFSET: u32 = 40;
pub const C_ETC1_ABS_COLOR_COMP_NUM_BITS: u32 = 4;
pub const C_ETC1_ABS_COLOR_COMP_MAX: u32 = 16;
pub const C_ETC1_ABS_COLOR4_R1_BIT_OFFSET: u32 = 60;
pub const C_ETC1_ABS_COLOR4_G1_BIT_OFFSET: u32 = 52;
pub const C_ETC1_ABS_COLOR4_B1_BIT_OFFSET: u32 = 44;
pub const C_ETC1_ABS_COLOR4_R2_BIT_OFFSET: u32 = 56;
pub const C_ETC1_ABS_COLOR4_G2_BIT_OFFSET: u32 = 48;
pub const C_ETC1_ABS_COLOR4_B2_BIT_OFFSET: u32 = 40;
pub const C_ETC1_COLOR_DELTA_MIN: i32 = -4;
pub const C_ETC1_COLOR_DELTA_MAX: i32 = 3;

/// `g_etc1_inten_tables`.
pub const G_ETC1_INTEN_TABLES: [[i32; 4]; 8] = [
    [-8, -2, 2, 8],
    [-17, -5, 5, 17],
    [-29, -9, 9, 29],
    [-42, -13, 13, 42],
    [-60, -18, 18, 60],
    [-80, -24, 24, 80],
    [-106, -33, 33, 106],
    [-183, -47, 47, 183],
];

/// `g_etc1_to_selector_index`.
pub const G_ETC1_TO_SELECTOR_INDEX: [u8; 4] = [2, 3, 1, 0];
/// `g_selector_index_to_etc1`.
pub const G_SELECTOR_INDEX_TO_ETC1: [u8; 4] = [3, 2, 0, 1];

/// `g_etc1_pixel_coords[flipped][subblock][subblock_pixel]` as (x, y).
pub const G_ETC1_PIXEL_COORDS: [[[(u8, u8); 8]; 2]; 2] = [
    [
        [
            (0, 0),
            (0, 1),
            (0, 2),
            (0, 3),
            (1, 0),
            (1, 1),
            (1, 2),
            (1, 3),
        ],
        [
            (2, 0),
            (2, 1),
            (2, 2),
            (2, 3),
            (3, 0),
            (3, 1),
            (3, 2),
            (3, 3),
        ],
    ],
    [
        [
            (0, 0),
            (1, 0),
            (2, 0),
            (3, 0),
            (0, 1),
            (1, 1),
            (2, 1),
            (3, 1),
        ],
        [
            (0, 2),
            (1, 2),
            (2, 2),
            (3, 2),
            (0, 3),
            (1, 3),
            (2, 3),
            (3, 3),
        ],
    ],
];

/// `g_etc1_pixel_indices[flipped][subblock][subblock_pixel]`.
pub const G_ETC1_PIXEL_INDICES: [[[u32; 8]; 2]; 2] = [
    [[0, 4, 8, 12, 1, 5, 9, 13], [2, 6, 10, 14, 3, 7, 11, 15]],
    [[0, 1, 2, 3, 4, 5, 6, 7], [8, 9, 10, 11, 12, 13, 14, 15]],
];

/// `g_etc2_eac_tables`.
pub const G_ETC2_EAC_TABLES: [[i8; 8]; 16] = EAC_MODIFIER_TABLE;

/// `g_etc2_eac_tables8`.
pub const G_ETC2_EAC_TABLES8: [[i8; 8]; 16] = [
    [-24, -48, -72, -120, 16, 40, 64, 112],
    [-24, -56, -80, -104, 16, 48, 72, 96],
    [-16, -40, -64, -104, 8, 32, 56, 96],
    [-16, -32, -48, -104, 8, 24, 40, 96],
    [-24, -48, -64, -96, 16, 40, 56, 88],
    [-24, -56, -72, -88, 16, 48, 64, 80],
    [-32, -56, -64, -88, 24, 48, 56, 80],
    [-24, -40, -64, -88, 16, 32, 56, 80],
    [-16, -48, -64, -80, 8, 40, 56, 72],
    [-16, -40, -64, -80, 8, 32, 56, 72],
    [-16, -32, -64, -80, 8, 24, 56, 72],
    [-16, -40, -56, -80, 8, 32, 48, 72],
    [-24, -32, -56, -80, 16, 24, 48, 72],
    [-8, -16, -24, -80, 0, 8, 16, 72],
    [-32, -48, -64, -72, 24, 40, 56, 64],
    [-24, -40, -56, -72, 16, 32, 48, 64],
];

pub const ETC2_EAC_MIN_VALUE_SELECTOR: usize = 3;
pub const ETC2_EAC_MAX_VALUE_SELECTOR: usize = 7;

/// `basisu::clamp255`.
#[inline]
fn clamp255(i: i32) -> u8 {
    if (i as u32) & 0xFFFF_FF00 != 0 {
        (!(i >> 31)) as u8
    } else {
        i as u8
    }
}

/// `etc_block::clamp255(x, did_clamp)`.
#[inline]
fn clamp255_dc(x: i32, did_clamp: &mut bool) -> u8 {
    if x < 0 {
        *did_clamp = true;
        0
    } else if x > 255 {
        *did_clamp = true;
        255
    } else {
        x as u8
    }
}

/// `basisu::color_rgba_i16` (r, g, b, a).
pub type ColorRgbaI16 = [i16; 4];

/// Port of `etc_block`: 8 bytes, big endian bit numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EtcBlock {
    pub bytes: [u8; 8],
}

impl EtcBlock {
    #[inline]
    pub fn clear(&mut self) {
        self.bytes = [0; 8];
    }

    #[inline]
    pub fn get_all_bits(&self) -> u64 {
        u64::from_be_bytes(self.bytes)
    }

    #[inline]
    pub fn get_general_bits(&self, ofs: u32, num: u32) -> u32 {
        ((self.get_all_bits() >> ofs) as u32) & ((1u32 << num) - 1)
    }

    #[inline]
    pub fn set_general_bits(&mut self, ofs: u32, num: u32, bits: u32) {
        let mut x = self.get_all_bits();
        let msk = ((1u64 << num) - 1) << ofs;
        x &= !msk;
        x |= (bits as u64) << ofs;
        self.bytes = x.to_be_bytes();
    }

    #[inline]
    pub fn get_byte_bits(&self, ofs: u32, num: u32) -> u32 {
        let byte_ofs = 7 - (ofs >> 3) as usize;
        let byte_bit_ofs = ofs & 7;
        (self.bytes[byte_ofs] as u32 >> byte_bit_ofs) & ((1 << num) - 1)
    }

    #[inline]
    pub fn set_byte_bits(&mut self, ofs: u32, num: u32, bits: u32) {
        let byte_ofs = 7 - (ofs >> 3) as usize;
        let byte_bit_ofs = ofs & 7;
        let mask = (1u32 << num) - 1;
        self.bytes[byte_ofs] &= !((mask << byte_bit_ofs) as u8);
        self.bytes[byte_ofs] |= (bits << byte_bit_ofs) as u8;
    }

    #[inline]
    pub fn get_flip_bit(&self) -> bool {
        (self.bytes[3] & 1) != 0
    }

    #[inline]
    pub fn set_flip_bit(&mut self, flip: bool) {
        self.bytes[3] &= !1;
        self.bytes[3] |= flip as u8;
    }

    #[inline]
    pub fn get_diff_bit(&self) -> bool {
        (self.bytes[3] & 2) != 0
    }

    #[inline]
    pub fn set_diff_bit(&mut self, diff: bool) {
        self.bytes[3] &= !2;
        self.bytes[3] |= (diff as u8) << 1;
    }

    #[inline]
    pub fn get_inten_table(&self, subblock_id: u32) -> u32 {
        let ofs = if subblock_id != 0 { 2 } else { 5 };
        (self.bytes[3] as u32 >> ofs) & 7
    }

    #[inline]
    pub fn set_inten_table(&mut self, subblock_id: u32, t: u32) {
        let ofs = if subblock_id != 0 { 2 } else { 5 };
        self.bytes[3] &= !((7u32 << ofs) as u8);
        self.bytes[3] |= (t << ofs) as u8;
    }

    #[inline]
    pub fn set_inten_tables_etc1s(&mut self, t: u32) {
        self.set_inten_table(0, t);
        self.set_inten_table(1, t);
    }

    pub fn is_etc1s(&self) -> bool {
        if self.get_inten_table(0) != self.get_inten_table(1) {
            return false;
        }
        if self.get_diff_bit() {
            if self.get_delta3_color() != 0 {
                return false;
            }
        } else if self.get_base4_color(0) != self.get_base4_color(1) {
            return false;
        }
        true
    }

    #[inline]
    pub fn get_raw_selector(&self, x: u32, y: u32) -> u32 {
        let bit_index = x * 4 + y;
        let byte_bit_ofs = bit_index & 7;
        let p = 7 - (bit_index >> 3) as usize;
        let lsb = (self.bytes[p] as u32 >> byte_bit_ofs) & 1;
        let msb = (self.bytes[p - 2] as u32 >> byte_bit_ofs) & 1;
        lsb | (msb << 1)
    }

    #[inline]
    pub fn get_selector(&self, x: u32, y: u32) -> u32 {
        G_ETC1_TO_SELECTOR_INDEX[self.get_raw_selector(x, y) as usize] as u32
    }

    #[inline]
    pub fn set_selector(&mut self, x: u32, y: u32, val: u32) {
        self.set_raw_selector(x, y, G_SELECTOR_INDEX_TO_ETC1[val as usize] as u32);
    }

    #[inline]
    pub fn set_raw_selector(&mut self, x: u32, y: u32, etc1_val: u32) {
        let bit_index = x * 4 + y;
        let p = 7 - (bit_index >> 3) as usize;
        let byte_bit_ofs = bit_index & 7;
        let mask = (1u32 << byte_bit_ofs) as u8;
        let lsb = (etc1_val & 1) as u8;
        let msb = (etc1_val >> 1) as u8;
        self.bytes[p] &= !mask;
        self.bytes[p] |= lsb << byte_bit_ofs;
        self.bytes[p - 2] &= !mask;
        self.bytes[p - 2] |= msb << byte_bit_ofs;
    }

    #[inline]
    pub fn get_raw_selector_bits(&self) -> u32 {
        u32::from_le_bytes([self.bytes[4], self.bytes[5], self.bytes[6], self.bytes[7]])
    }

    #[inline]
    pub fn set_raw_selector_bits(&mut self, bits: u32) {
        self.bytes[4..8].copy_from_slice(&bits.to_le_bytes());
    }

    #[inline]
    pub fn set_raw_selector_bytes(&mut self, b0: u8, b1: u8, b2: u8, b3: u8) {
        self.bytes[4] = b0;
        self.bytes[5] = b1;
        self.bytes[6] = b2;
        self.bytes[7] = b3;
    }

    pub fn set_base4_color(&mut self, idx: u32, c: u16) {
        let c = c as u32;
        if idx != 0 {
            self.set_byte_bits(C_ETC1_ABS_COLOR4_R2_BIT_OFFSET, 4, (c >> 8) & 15);
            self.set_byte_bits(C_ETC1_ABS_COLOR4_G2_BIT_OFFSET, 4, (c >> 4) & 15);
            self.set_byte_bits(C_ETC1_ABS_COLOR4_B2_BIT_OFFSET, 4, c & 15);
        } else {
            self.set_byte_bits(C_ETC1_ABS_COLOR4_R1_BIT_OFFSET, 4, (c >> 8) & 15);
            self.set_byte_bits(C_ETC1_ABS_COLOR4_G1_BIT_OFFSET, 4, (c >> 4) & 15);
            self.set_byte_bits(C_ETC1_ABS_COLOR4_B1_BIT_OFFSET, 4, c & 15);
        }
    }

    pub fn get_base4_color(&self, idx: u32) -> u16 {
        let (r, g, b) = if idx != 0 {
            (
                self.get_byte_bits(C_ETC1_ABS_COLOR4_R2_BIT_OFFSET, 4),
                self.get_byte_bits(C_ETC1_ABS_COLOR4_G2_BIT_OFFSET, 4),
                self.get_byte_bits(C_ETC1_ABS_COLOR4_B2_BIT_OFFSET, 4),
            )
        } else {
            (
                self.get_byte_bits(C_ETC1_ABS_COLOR4_R1_BIT_OFFSET, 4),
                self.get_byte_bits(C_ETC1_ABS_COLOR4_G1_BIT_OFFSET, 4),
                self.get_byte_bits(C_ETC1_ABS_COLOR4_B1_BIT_OFFSET, 4),
            )
        };
        (b | (g << 4) | (r << 8)) as u16
    }

    pub fn set_base5_color(&mut self, c: u16) {
        let c = c as u32;
        self.set_byte_bits(C_ETC1_BASE_COLOR5_R_BIT_OFFSET, 5, (c >> 10) & 31);
        self.set_byte_bits(C_ETC1_BASE_COLOR5_G_BIT_OFFSET, 5, (c >> 5) & 31);
        self.set_byte_bits(C_ETC1_BASE_COLOR5_B_BIT_OFFSET, 5, c & 31);
    }

    pub fn get_base5_color(&self) -> u16 {
        let r = self.get_byte_bits(C_ETC1_BASE_COLOR5_R_BIT_OFFSET, 5);
        let g = self.get_byte_bits(C_ETC1_BASE_COLOR5_G_BIT_OFFSET, 5);
        let b = self.get_byte_bits(C_ETC1_BASE_COLOR5_B_BIT_OFFSET, 5);
        (b | (g << 5) | (r << 10)) as u16
    }

    pub fn set_delta3_color(&mut self, c: u16) {
        let c = c as u32;
        self.set_byte_bits(C_ETC1_DELTA_COLOR3_R_BIT_OFFSET, 3, (c >> 6) & 7);
        self.set_byte_bits(C_ETC1_DELTA_COLOR3_G_BIT_OFFSET, 3, (c >> 3) & 7);
        self.set_byte_bits(C_ETC1_DELTA_COLOR3_B_BIT_OFFSET, 3, c & 7);
    }

    pub fn get_delta3_color(&self) -> u16 {
        let r = self.get_byte_bits(C_ETC1_DELTA_COLOR3_R_BIT_OFFSET, 3);
        let g = self.get_byte_bits(C_ETC1_DELTA_COLOR3_G_BIT_OFFSET, 3);
        let b = self.get_byte_bits(C_ETC1_DELTA_COLOR3_B_BIT_OFFSET, 3);
        (b | (g << 3) | (r << 6)) as u16
    }

    /// `determine_selectors(pSource_pixels, perceptual, begin_subblock, end_subblock)`.
    pub fn determine_selectors(
        &mut self,
        src: &[ColorRgba],
        perceptual: bool,
        begin_subblock: u32,
        end_subblock: u32,
    ) -> u64 {
        let mut total_error = 0u64;
        for subblock in begin_subblock..end_subblock {
            let mut block_colors = [[0u8; 4]; 4];
            self.get_block_colors(&mut block_colors, subblock);
            if self.get_flip_bit() {
                for y in 0..2 {
                    for x in 0..4 {
                        let px = &src[(x + (subblock * 2 + y) * 4) as usize];
                        let (s, e) = best_selector(perceptual, &block_colors, px);
                        self.set_selector(x, subblock * 2 + y, s);
                        total_error += e;
                    }
                }
            } else {
                for y in 0..4 {
                    for x in 0..2 {
                        let px = &src[((subblock * 2) + x + y * 4) as usize];
                        let (s, e) = best_selector(perceptual, &block_colors, px);
                        self.set_selector(subblock * 2 + x, y, s);
                        total_error += e;
                    }
                }
            }
        }
        total_error
    }

    fn subblock_base_color(&self, subblock_index: u32, scaled: bool) -> ColorRgba {
        if self.get_diff_bit() {
            if subblock_index != 0 {
                let mut b = [0u8; 4];
                Self::unpack_color5_delta(
                    &mut b,
                    self.get_base5_color(),
                    self.get_delta3_color(),
                    scaled,
                    255,
                );
                b
            } else {
                Self::unpack_color5(self.get_base5_color(), scaled, 255)
            }
        } else {
            Self::unpack_color4(self.get_base4_color(subblock_index), scaled, 255)
        }
    }

    pub fn get_block_color(&self, subblock_index: u32, scaled: bool) -> ColorRgba {
        self.subblock_base_color(subblock_index, scaled)
    }

    #[inline]
    pub fn get_subblock_index(&self, x: u32, y: u32) -> u32 {
        if self.get_flip_bit() {
            (y >= 2) as u32
        } else {
            (x >= 2) as u32
        }
    }

    /// Returns true if any component was clamped.
    pub fn get_block_colors(&self, block_colors: &mut [ColorRgba; 4], subblock_index: u32) -> bool {
        let b = self.subblock_base_color(subblock_index, true);
        let t = &G_ETC1_INTEN_TABLES[self.get_inten_table(subblock_index) as usize];
        let mut dc = false;
        for s in 0..4 {
            block_colors[s] = [
                clamp255_dc(b[0] as i32 + t[s], &mut dc),
                clamp255_dc(b[1] as i32 + t[s], &mut dc),
                clamp255_dc(b[2] as i32 + t[s], &mut dc),
                255,
            ];
        }
        dc
    }

    pub fn get_block_colors_etc1s(&self, block_colors: &mut [ColorRgba; 4]) {
        let b = Self::unpack_color5(self.get_base5_color(), true, 255);
        let t = &G_ETC1_INTEN_TABLES[self.get_inten_table(0) as usize];
        for s in 0..4 {
            block_colors[s] = color_rgba(
                b[0] as i32 + t[s],
                b[1] as i32 + t[s],
                b[2] as i32 + t[s],
                255,
            );
        }
    }

    /// Static `get_block_colors_etc1s(pBlock_colors, base5_color, inten_table)`.
    pub fn get_block_colors_etc1s_from(
        block_colors: &mut [ColorRgba; 4],
        base5_color: &ColorRgba,
        inten_table: u32,
    ) {
        let r = ((base5_color[0] as u32) << 3 | (base5_color[0] as u32) >> 2) as u8 as i32;
        let g = ((base5_color[1] as u32) << 3 | (base5_color[1] as u32) >> 2) as u8 as i32;
        let b = ((base5_color[2] as u32) << 3 | (base5_color[2] as u32) >> 2) as u8 as i32;
        let t = &G_ETC1_INTEN_TABLES[inten_table as usize];
        for s in 0..4 {
            block_colors[s] = color_rgba(r + t[s], g + t[s], b + t[s], 255);
        }
    }

    /// `get_block_color(color, subblock_index, selector_index)`.
    pub fn get_block_color_sel(&self, subblock_index: u32, selector_index: u32) -> ColorRgba {
        let b = self.subblock_base_color(subblock_index, true);
        let y = G_ETC1_INTEN_TABLES[self.get_inten_table(subblock_index) as usize]
            [selector_index as usize];
        color_rgba(b[0] as i32 + y, b[1] as i32 + y, b[2] as i32 + y, 255)
    }

    pub fn get_block_low_high_colors(
        &self,
        block_colors: &mut [ColorRgba; 2],
        subblock_index: u32,
    ) -> bool {
        let b = self.subblock_base_color(subblock_index, true);
        let t = &G_ETC1_INTEN_TABLES[self.get_inten_table(subblock_index) as usize];
        let mut dc = false;
        for (i, s) in [0usize, 3].into_iter().enumerate() {
            block_colors[i] = [
                clamp255_dc(b[0] as i32 + t[s], &mut dc),
                clamp255_dc(b[1] as i32 + t[s], &mut dc),
                clamp255_dc(b[2] as i32 + t[s], &mut dc),
                255,
            ];
        }
        dc
    }

    pub fn get_block_colors5(
        block_colors: &mut [ColorRgba; 4],
        base_color5: &ColorRgba,
        inten_table: u32,
        scaled: bool,
    ) {
        let mut b = *base_color5;
        if !scaled {
            for c in 0..3 {
                b[c] = (b[c] << 3) | (b[c] >> 2);
            }
        }
        let t = &G_ETC1_INTEN_TABLES[inten_table as usize];
        for s in 0..4 {
            block_colors[s] = color_rgba(
                b[0] as i32 + t[s],
                b[1] as i32 + t[s],
                b[2] as i32 + t[s],
                255,
            );
        }
    }

    pub fn get_block_colors4(
        block_colors: &mut [ColorRgba; 4],
        base_color4: &ColorRgba,
        inten_table: u32,
        scaled: bool,
    ) {
        let mut b = *base_color4;
        if !scaled {
            for c in 0..3 {
                b[c] = (b[c] << 4) | b[c];
            }
        }
        let t = &G_ETC1_INTEN_TABLES[inten_table as usize];
        for s in 0..4 {
            block_colors[s] = color_rgba(
                b[0] as i32 + t[s],
                b[1] as i32 + t[s],
                b[2] as i32 + t[s],
                255,
            );
        }
    }

    /// `evaluate_etc1_error(pBlock_pixels, perceptual, subblock_index)`; -1 = whole block.
    pub fn evaluate_etc1_error(
        &self,
        block_pixels: &[ColorRgba],
        perceptual: bool,
        subblock_index: i32,
    ) -> u64 {
        let mut unpacked = [[0u8; 4]; 16];
        unpack_etc1(self, &mut unpacked, false);
        let mut total_error = 0u64;
        if subblock_index < 0 {
            for i in 0..16 {
                total_error +=
                    color_distance(perceptual, &block_pixels[i], &unpacked[i], false) as u64;
            }
        } else {
            let flip = self.get_flip_bit() as usize;
            for i in 0..8 {
                let idx = G_ETC1_PIXEL_INDICES[flip][subblock_index as usize][i] as usize;
                total_error +=
                    color_distance(perceptual, &block_pixels[idx], &unpacked[idx], false) as u64;
            }
        }
        total_error
    }

    /// `get_subblock_pixels(pPixels, subblock_index)`; -1 = whole block (16 pixels).
    pub fn get_subblock_pixels(&self, pixels: &mut [ColorRgba], subblock_index: i32) {
        if subblock_index < 0 {
            unpack_etc1(self, pixels, false);
        } else {
            let mut unpacked = [[0u8; 4]; 16];
            unpack_etc1(self, &mut unpacked, false);
            let flip = self.get_flip_bit() as usize;
            for i in 0..8 {
                pixels[i] =
                    unpacked[G_ETC1_PIXEL_INDICES[flip][subblock_index as usize][i] as usize];
            }
        }
    }

    /// Returns (low, high).
    pub fn get_selector_range(&self) -> (u32, u32) {
        let (mut low, mut high) = (3, 0);
        for y in 0..4 {
            for x in 0..4 {
                let s = self.get_selector(x, y);
                low = low.min(s);
                high = high.max(s);
            }
        }
        (low, high)
    }

    pub fn set_block_color4(&mut self, c0_unscaled: &ColorRgba, c1_unscaled: &ColorRgba) {
        self.set_diff_bit(false);
        self.set_base4_color(0, Self::pack_color4(c0_unscaled, false, 127));
        self.set_base4_color(1, Self::pack_color4(c1_unscaled, false, 127));
    }

    pub fn set_block_color5(&mut self, c0_unscaled: &ColorRgba, c1_unscaled: &ColorRgba) {
        self.set_diff_bit(true);
        self.set_base5_color(Self::pack_color5(c0_unscaled, false, 127));
        let dr = c1_unscaled[0] as i32 - c0_unscaled[0] as i32;
        let dg = c1_unscaled[1] as i32 - c0_unscaled[1] as i32;
        let db = c1_unscaled[2] as i32 - c0_unscaled[2] as i32;
        self.set_delta3_color(Self::pack_delta3(dr, dg, db));
    }

    pub fn set_block_color5_etc1s(&mut self, c_unscaled: &ColorRgba) {
        self.set_diff_bit(true);
        self.set_base5_color(Self::pack_color5(c_unscaled, false, 127));
        self.set_delta3_color(Self::pack_delta3(0, 0, 0));
    }

    pub fn set_block_color5_check(
        &mut self,
        c0_unscaled: &ColorRgba,
        c1_unscaled: &ColorRgba,
    ) -> bool {
        self.set_diff_bit(true);
        self.set_base5_color(Self::pack_color5(c0_unscaled, false, 127));
        let dr = c1_unscaled[0] as i32 - c0_unscaled[0] as i32;
        let dg = c1_unscaled[1] as i32 - c0_unscaled[1] as i32;
        let db = c1_unscaled[2] as i32 - c0_unscaled[2] as i32;
        let r = C_ETC1_COLOR_DELTA_MIN..=C_ETC1_COLOR_DELTA_MAX;
        if !r.contains(&dr) || !r.contains(&dg) || !r.contains(&db) {
            return false;
        }
        self.set_delta3_color(Self::pack_delta3(dr, dg, db));
        true
    }

    pub fn set_block_color5_clamp(
        &mut self,
        c0_unscaled: &ColorRgba,
        c1_unscaled: &ColorRgba,
    ) -> bool {
        self.set_diff_bit(true);
        self.set_base5_color(Self::pack_color5(c0_unscaled, false, 127));
        let d = |c: usize| {
            (c1_unscaled[c] as i32 - c0_unscaled[c] as i32)
                .clamp(C_ETC1_COLOR_DELTA_MIN, C_ETC1_COLOR_DELTA_MAX)
        };
        self.set_delta3_color(Self::pack_delta3(d(0), d(1), d(2)));
        true
    }

    pub fn get_selector_color(&self, x: u32, y: u32, s: u32) -> ColorRgba {
        let mut block_colors = [[0u8; 4]; 4];
        self.get_block_colors(&mut block_colors, self.get_subblock_index(x, y));
        block_colors[s as usize]
    }

    pub fn pack_color5(color: &ColorRgba, scaled: bool, bias: u32) -> u16 {
        Self::pack_color5_rgb(
            color[0] as u32,
            color[1] as u32,
            color[2] as u32,
            scaled,
            bias,
        )
    }

    pub fn pack_color5_rgb(mut r: u32, mut g: u32, mut b: u32, scaled: bool, bias: u32) -> u16 {
        if scaled {
            r = (r * 31 + bias) / 255;
            g = (g * 31 + bias) / 255;
            b = (b * 31 + bias) / 255;
        }
        r = r.min(31);
        g = g.min(31);
        b = b.min(31);
        (b | (g << 5) | (r << 10)) as u16
    }

    pub fn unpack_color5(packed_color5: u16, scaled: bool, alpha: u32) -> ColorRgba {
        let mut b = packed_color5 as u32 & 31;
        let mut g = (packed_color5 as u32 >> 5) & 31;
        let mut r = (packed_color5 as u32 >> 10) & 31;
        if scaled {
            b = (b << 3) | (b >> 2);
            g = (g << 3) | (g >> 2);
            r = (r << 3) | (r >> 2);
        }
        color_rgba_noclamp(r as i32, g as i32, b as i32, alpha.min(255) as i32)
    }

    /// `unpack_color5(r, g, b, packed_color5, scaled)`.
    pub fn unpack_color5_rgb(packed_color5: u16, scaled: bool) -> (u32, u32, u32) {
        let c = Self::unpack_color5(packed_color5, scaled, 0);
        (c[0] as u32, c[1] as u32, c[2] as u32)
    }

    /// `unpack_color5(result, packed_color5, packed_delta3, scaled, alpha)`.
    pub fn unpack_color5_delta(
        result: &mut ColorRgba,
        packed_color5: u16,
        packed_delta3: u16,
        scaled: bool,
        alpha: u32,
    ) -> bool {
        let dc = Self::unpack_delta3(packed_delta3);
        let mut b = (packed_color5 as i32 & 31) + dc[2] as i32;
        let mut g = ((packed_color5 as i32 >> 5) & 31) + dc[1] as i32;
        let mut r = ((packed_color5 as i32 >> 10) & 31) + dc[0] as i32;
        let mut success = true;
        if ((r | g | b) as u32) > 31 {
            success = false;
            r = r.clamp(0, 31);
            g = g.clamp(0, 31);
            b = b.clamp(0, 31);
        }
        if scaled {
            b = (b << 3) | (b >> 2);
            g = (g << 3) | (g >> 2);
            r = (r << 3) | (r >> 2);
        }
        *result = color_rgba_noclamp(r, g, b, alpha.min(255) as i32);
        success
    }

    /// `unpack_color5(r, g, b, packed_color5, packed_delta3, scaled, alpha)`.
    pub fn unpack_color5_delta_rgb(
        packed_color5: u16,
        packed_delta3: u16,
        scaled: bool,
        alpha: u32,
    ) -> (bool, u32, u32, u32) {
        let mut c = [0u8; 4];
        let ok = Self::unpack_color5_delta(&mut c, packed_color5, packed_delta3, scaled, alpha);
        (ok, c[0] as u32, c[1] as u32, c[2] as u32)
    }

    pub fn pack_delta3(mut r: i32, mut g: i32, mut b: i32) -> u16 {
        if r < 0 {
            r += 8;
        }
        if g < 0 {
            g += 8;
        }
        if b < 0 {
            b += 8;
        }
        (b | (g << 3) | (r << 6)) as u16
    }

    pub fn unpack_delta3(packed_delta3: u16) -> ColorRgbaI16 {
        let (r, g, b) = Self::unpack_delta3_rgb(packed_delta3);
        [r as i16, g as i16, b as i16, 255]
    }

    pub fn unpack_delta3_rgb(packed_delta3: u16) -> (i32, i32, i32) {
        let mut r = (packed_delta3 as i32 >> 6) & 7;
        let mut g = (packed_delta3 as i32 >> 3) & 7;
        let mut b = packed_delta3 as i32 & 7;
        if r >= 4 {
            r -= 8;
        }
        if g >= 4 {
            g -= 8;
        }
        if b >= 4 {
            b -= 8;
        }
        (r, g, b)
    }

    pub fn try_pack_color5_delta3(color5_unscaled: &[ColorRgba; 2]) -> bool {
        let dr = color5_unscaled[1][0] as i32 - color5_unscaled[0][0] as i32;
        let dg = color5_unscaled[1][1] as i32 - color5_unscaled[0][1] as i32;
        let db = color5_unscaled[1][2] as i32 - color5_unscaled[0][2] as i32;
        !(dr.min(dg).min(db) < C_ETC1_COLOR_DELTA_MIN
            || dr.max(dg).max(db) > C_ETC1_COLOR_DELTA_MAX)
    }

    pub fn pack_color4(color: &ColorRgba, scaled: bool, bias: u32) -> u16 {
        Self::pack_color4_rgb(
            color[0] as u32,
            color[1] as u32,
            color[2] as u32,
            scaled,
            bias,
        )
    }

    pub fn pack_color4_rgb(mut r: u32, mut g: u32, mut b: u32, scaled: bool, bias: u32) -> u16 {
        if scaled {
            r = (r * 15 + bias) / 255;
            g = (g * 15 + bias) / 255;
            b = (b * 15 + bias) / 255;
        }
        r = r.min(15);
        g = g.min(15);
        b = b.min(15);
        (b | (g << 4) | (r << 8)) as u16
    }

    pub fn unpack_color4(packed_color4: u16, scaled: bool, alpha: u32) -> ColorRgba {
        let mut b = packed_color4 as u32 & 15;
        let mut g = (packed_color4 as u32 >> 4) & 15;
        let mut r = (packed_color4 as u32 >> 8) & 15;
        if scaled {
            b = (b << 4) | b;
            g = (g << 4) | g;
            r = (r << 4) | r;
        }
        color_rgba_noclamp(r as i32, g as i32, b as i32, alpha.min(255) as i32)
    }

    pub fn unpack_color4_rgb(packed_color4: u16, scaled: bool) -> (u32, u32, u32) {
        let c = Self::unpack_color4(packed_color4, scaled, 0);
        (c[0] as u32, c[1] as u32, c[2] as u32)
    }

    fn subblock_colors(dst: &mut [ColorRgba; 4], r: u32, g: u32, b: u32, table_idx: u32) {
        let t = &G_ETC1_INTEN_TABLES[table_idx as usize];
        for s in 0..4 {
            dst[s] = color_rgba(r as i32 + t[s], g as i32 + t[s], b as i32 + t[s], 255);
        }
    }

    pub fn get_diff_subblock_colors(dst: &mut [ColorRgba; 4], packed_color5: u16, table_idx: u32) {
        let (r, g, b) = Self::unpack_color5_rgb(packed_color5, true);
        Self::subblock_colors(dst, r, g, b, table_idx);
    }

    pub fn get_diff_subblock_colors_delta(
        dst: &mut [ColorRgba; 4],
        packed_color5: u16,
        packed_delta3: u16,
        table_idx: u32,
    ) -> bool {
        let (success, r, g, b) =
            Self::unpack_color5_delta_rgb(packed_color5, packed_delta3, true, 255);
        Self::subblock_colors(dst, r, g, b, table_idx);
        success
    }

    pub fn get_abs_subblock_colors(dst: &mut [ColorRgba; 4], packed_color4: u16, table_idx: u32) {
        let (r, g, b) = Self::unpack_color4_rgb(packed_color4, true);
        Self::subblock_colors(dst, r, g, b, table_idx);
    }

    pub fn unscaled_to_scaled_color(dst: &mut ColorRgba, src: &ColorRgba, color4: bool) {
        for c in 0..3 {
            dst[c] = if color4 {
                src[c] | (src[c] << 4)
            } else {
                (src[c] >> 2) | (src[c] << 3)
            };
        }
        dst[3] = src[3];
    }
}

#[inline]
fn best_selector(perceptual: bool, block_colors: &[ColorRgba; 4], px: &ColorRgba) -> (u32, u64) {
    let mut best_selector = 0;
    let mut best_error = u64::MAX;
    for s in 0..4 {
        let err = color_distance(perceptual, &block_colors[s], px, false) as u64;
        if err < best_error {
            best_error = err;
            best_selector = s as u32;
        }
    }
    (best_selector, best_error)
}

/// `unpack_etc1(block, pDst, preserve_alpha)`; returns false for an invalid delta (ETC2).
pub fn unpack_etc1(block: &EtcBlock, dst: &mut [ColorRgba], preserve_alpha: bool) -> bool {
    let diff_flag = block.get_diff_bit();
    let flip_flag = block.get_flip_bit();
    let table_index0 = block.get_inten_table(0);
    let table_index1 = block.get_inten_table(1);
    let mut sc0 = [[0u8; 4]; 4];
    let mut sc1 = [[0u8; 4]; 4];
    if diff_flag {
        let base_color5 = block.get_base5_color();
        let delta_color3 = block.get_delta3_color();
        EtcBlock::get_diff_subblock_colors(&mut sc0, base_color5, table_index0);
        if !EtcBlock::get_diff_subblock_colors_delta(
            &mut sc1,
            base_color5,
            delta_color3,
            table_index1,
        ) {
            return false;
        }
    } else {
        EtcBlock::get_abs_subblock_colors(&mut sc0, block.get_base4_color(0), table_index0);
        EtcBlock::get_abs_subblock_colors(&mut sc1, block.get_base4_color(1), table_index1);
    }
    for y in 0..4u32 {
        for x in 0..4u32 {
            let second = if flip_flag { y >= 2 } else { x >= 2 };
            let sc = if second { &sc1 } else { &sc0 };
            let c = sc[block.get_selector(x, y) as usize];
            let d = &mut dst[(x + y * 4) as usize];
            if preserve_alpha {
                d[0] = c[0];
                d[1] = c[1];
                d[2] = c[2];
            } else {
                *d = c;
            }
        }
    }
    true
}

/// `basis_etc_quality`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum BasisEtcQuality {
    Fast,
    Medium,
    #[default]
    Slow,
    Uber,
}

/// `basis_etc1_pack_params`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasisEtc1PackParams {
    pub quality: BasisEtcQuality,
    pub perceptual: bool,
    pub cluster_fit: bool,
    pub force_etc1s: bool,
    pub use_color4: bool,
    pub flip_bias: f32,
}

impl Default for BasisEtc1PackParams {
    fn default() -> Self {
        BasisEtc1PackParams {
            quality: BasisEtcQuality::Slow,
            perceptual: true,
            cluster_fit: true,
            force_etc1s: false,
            use_color4: true,
            flip_bias: 0.0,
        }
    }
}

/// `etc1_solution_coordinates`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Etc1SolutionCoordinates {
    pub unscaled_color: ColorRgba,
    pub inten_table: u32,
    pub color4: bool,
}

impl Etc1SolutionCoordinates {
    pub fn new(r: u32, g: u32, b: u32, inten_table: u32, color4: bool) -> Self {
        Etc1SolutionCoordinates {
            unscaled_color: [r as u8, g as u8, b as u8, 255],
            inten_table: inten_table as u8 as u32,
            color4,
        }
    }

    pub fn from_color(c: &ColorRgba, inten_table: u32, color4: bool) -> Self {
        Etc1SolutionCoordinates {
            unscaled_color: *c,
            inten_table,
            color4,
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn init(&mut self, c: &ColorRgba, inten_table: u32, color4: bool) {
        *self = Self::from_color(c, inten_table, color4);
    }

    pub fn get_scaled_color(&self) -> ColorRgba {
        let u = &self.unscaled_color;
        let (br, bg, bb) = if self.color4 {
            (u[0] | (u[0] << 4), u[1] | (u[1] << 4), u[2] | (u[2] << 4))
        } else {
            (
                (u[0] >> 2) | (u[0] << 3),
                (u[1] >> 2) | (u[1] << 3),
                (u[2] >> 2) | (u[2] << 3),
            )
        };
        [br, bg, bb, 255]
    }

    pub fn get_block_colors(&self, block_colors: &mut [ColorRgba; 4]) {
        let s = self.get_scaled_color();
        let t = &G_ETC1_INTEN_TABLES[self.inten_table as usize];
        for i in 0..4 {
            block_colors[i] = color_rgba(
                s[0] as i32 + t[i],
                s[1] as i32 + t[i],
                s[2] as i32 + t[i],
                255,
            );
        }
    }
}

static DEFAULT_SCAN_DELTA: [i32; 1] = [0];

/// `etc1_optimizer::params`. Note: like the C++ struct, `use_color4` here shadows
/// `pack.use_color4` and defaults to false.
#[derive(Debug, Clone, Copy)]
pub struct Etc1OptimizerParams<'a> {
    pub pack: BasisEtc1PackParams,
    pub num_src_pixels: u32,
    pub src_pixels: &'a [ColorRgba],
    pub use_color4: bool,
    pub scan_deltas: &'a [i32],
    pub base_color5: ColorRgba,
    pub constrain_against_base_color5: bool,
    pub refinement: bool,
    pub force_selectors: Option<&'a [u8]>,
}

impl Default for Etc1OptimizerParams<'_> {
    fn default() -> Self {
        Etc1OptimizerParams {
            pack: BasisEtc1PackParams::default(),
            num_src_pixels: 0,
            src_pixels: &[],
            use_color4: false,
            scan_deltas: &DEFAULT_SCAN_DELTA,
            base_color5: [0; 4],
            constrain_against_base_color5: false,
            refinement: true,
            force_selectors: None,
        }
    }
}

impl<'a> Etc1OptimizerParams<'a> {
    /// `params(const basis_etc1_pack_params&)`.
    pub fn from_pack_params(base_params: &BasisEtc1PackParams) -> Self {
        Etc1OptimizerParams {
            pack: *base_params,
            ..Default::default()
        }
    }
}

/// `etc1_optimizer::results`; `selectors` replaces the caller provided `m_pSelectors` buffer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Etc1OptimizerResults {
    pub error: u64,
    pub block_color_unscaled: ColorRgba,
    pub block_inten_table: u32,
    pub n: u32,
    pub selectors: Vec<u8>,
    pub block_color4: bool,
}

impl Etc1OptimizerResults {
    /// `results::operator=`: copies min(rhs.n, n) selectors.
    pub fn assign(&mut self, rhs: &Etc1OptimizerResults) {
        self.block_color_unscaled = rhs.block_color_unscaled;
        self.block_color4 = rhs.block_color4;
        self.block_inten_table = rhs.block_inten_table;
        self.error = rhs.error;
        let n = (rhs.n.min(self.n) as usize)
            .min(self.selectors.len())
            .min(rhs.selectors.len());
        self.selectors[..n].copy_from_slice(&rhs.selectors[..n]);
    }
}

#[derive(Debug, Clone)]
struct PotentialSolution {
    coords: Etc1SolutionCoordinates,
    selectors: Vec<u8>,
    error: u64,
    valid: bool,
}

impl Default for PotentialSolution {
    fn default() -> Self {
        PotentialSolution {
            coords: Default::default(),
            selectors: Vec::new(),
            error: u64::MAX,
            valid: false,
        }
    }
}

impl PotentialSolution {
    /// `*pBest_solution = trial_solution`.
    fn assign_from(&mut self, other: &PotentialSolution) {
        self.coords = other.coords;
        self.selectors.clone_from(&other.selectors);
        self.error = other.error;
        self.valid = other.valid;
    }
}

const C_SOLUTIONS_TRIED_HASH_BITS: u32 = 10;
const C_TOTAL_SOLUTIONS_TRIED_HASH_SIZE: u32 = 1 << C_SOLUTIONS_TRIED_HASH_BITS;
const C_SOLUTIONS_TRIED_HASH_MASK: u32 = C_TOTAL_SOLUTIONS_TRIED_HASH_SIZE - 1;

/// `BASISU_ETC1_CLUSTER_FIT_ORDER_TABLE_SIZE`.
pub const BASISU_ETC1_CLUSTER_FIT_ORDER_TABLE_SIZE: u32 = 165;

/// Port of `etc1_optimizer`. `init` + `compute` take the params explicitly instead of
/// keeping the C++ `m_pParams` / `m_pResult` pointers.
#[derive(Debug, Clone)]
pub struct Etc1Optimizer {
    limit: i32,
    avg_color: [f32; 3],
    br: i32,
    bg: i32,
    bb: i32,
    max_comp_spread: i32,
    luma: Vec<u16>,
    sorted_luma: Vec<u32>,
    sorted_luma_indices: Vec<u32>,
    selectors: Vec<u8>,
    best_selectors: Vec<u8>,
    best_solution: PotentialSolution,
    trial_solution: PotentialSolution,
    temp_selectors: Vec<u8>,
    solutions_tried: [u8; (C_TOTAL_SOLUTIONS_TRIED_HASH_SIZE / 8) as usize],
}

impl Default for Etc1Optimizer {
    fn default() -> Self {
        Etc1Optimizer {
            limit: 0,
            avg_color: [0.0; 3],
            br: 0,
            bg: 0,
            bb: 0,
            max_comp_spread: 0,
            luma: Vec::new(),
            sorted_luma: Vec::new(),
            sorted_luma_indices: Vec::new(),
            selectors: Vec::new(),
            best_selectors: Vec::new(),
            best_solution: PotentialSolution::default(),
            trial_solution: PotentialSolution::default(),
            temp_selectors: Vec::new(),
            solutions_tried: [0; (C_TOTAL_SOLUTIONS_TRIED_HASH_SIZE / 8) as usize],
        }
    }
}

impl Etc1Optimizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// `etc1_optimizer::init`.
    pub fn init(&mut self, p: &Etc1OptimizerParams) {
        let n = p.num_src_pixels as usize;
        self.selectors.resize(n, 0);
        self.best_selectors.resize(n, 0);
        self.temp_selectors.resize(n, 0);
        self.trial_solution.selectors.resize(n, 0);
        self.best_solution.selectors.resize(n, 0);

        self.limit = if p.use_color4 { 15 } else { 31 };

        let mut avg_color = [0.0f32; 3];
        self.luma.resize(n, 0);
        self.sorted_luma_indices.resize(n, 0);
        self.sorted_luma.resize(n, 0);

        let (mut min_r, mut min_g, mut min_b) = (255i32, 255i32, 255i32);
        let (mut max_r, mut max_g, mut max_b) = (0i32, 0i32, 0i32);
        for i in 0..n {
            let c = &p.src_pixels[i];
            min_r = min_r.min(c[0] as i32);
            min_g = min_g.min(c[1] as i32);
            min_b = min_b.min(c[2] as i32);
            max_r = max_r.max(c[0] as i32);
            max_g = max_g.max(c[1] as i32);
            max_b = max_b.max(c[2] as i32);
            avg_color[0] += c[0] as f32;
            avg_color[1] += c[1] as f32;
            avg_color[2] += c[2] as f32;
            self.luma[i] = (c[0] as u16) + (c[1] as u16) + (c[2] as u16);
            self.sorted_luma_indices[i] = i as u32;
        }
        for v in avg_color.iter_mut() {
            *v /= n as f32;
        }
        self.avg_color = avg_color;
        self.max_comp_spread = (max_r - min_r).max(max_g - min_g).max(max_b - min_b);

        let lim = self.limit as f32;
        self.br = ((self.avg_color[0] * lim / 255.0 + 0.5) as u32 as i32).clamp(0, self.limit);
        self.bg = ((self.avg_color[1] * lim / 255.0 + 0.5) as u32 as i32).clamp(0, self.limit);
        self.bb = ((self.avg_color[2] * lim / 255.0 + 0.5) as u32 as i32).clamp(0, self.limit);

        if p.pack.quality == BasisEtcQuality::Fast {
            indirect_sort(&mut self.sorted_luma_indices, &self.luma);
            for i in 0..n {
                self.sorted_luma[i] = self.luma[self.sorted_luma_indices[i] as usize] as u32;
            }
        }

        self.best_solution.coords.clear();
        self.best_solution.valid = false;
        self.best_solution.error = u64::MAX;

        self.solutions_tried = [0; (C_TOTAL_SOLUTIONS_TRIED_HASH_SIZE / 8) as usize];
    }

    /// `etc1_optimizer::compute`.
    pub fn compute(&mut self, p: &Etc1OptimizerParams, result: &mut Etc1OptimizerResults) -> bool {
        if p.force_selectors.is_some() && p.pack.quality < BasisEtcQuality::Slow {
            return false;
        }
        let n = p.num_src_pixels as usize;

        if p.pack.cluster_fit {
            let perms = match p.pack.quality {
                BasisEtcQuality::Fast => 4,
                BasisEtcQuality::Medium => 16,
                BasisEtcQuality::Slow => 64,
                BasisEtcQuality::Uber => BASISU_ETC1_CLUSTER_FIT_ORDER_TABLE_SIZE,
            };
            self.compute_internal_cluster_fit(p, perms);
        } else {
            self.compute_internal_neighborhood(p, self.br, self.bg, self.bb);
        }

        if !self.best_solution.valid {
            result.error = u32::MAX as u64;
            return false;
        }

        result.error = self.best_solution.error;
        result.block_color_unscaled = self.best_solution.coords.unscaled_color;
        result.block_color4 = self.best_solution.coords.color4;
        result.block_inten_table = self.best_solution.coords.inten_table;
        if result.selectors.len() < n {
            result.selectors.resize(n, 0);
        }
        match p.force_selectors {
            Some(fs) => result.selectors[..n].copy_from_slice(&fs[..n]),
            None => result.selectors[..n].copy_from_slice(&self.best_solution.selectors[..n]),
        }
        result.n = n as u32;
        true
    }

    fn refine_solution(&mut self, p: &Etc1OptimizerParams, max_refinement_trials: u32) {
        let n = p.num_src_pixels as usize;
        for _ in 0..max_refinement_trials {
            let t = &G_ETC1_INTEN_TABLES[self.best_solution.coords.inten_table as usize];
            let (mut dr, mut dg, mut db) = (0i32, 0i32, 0i32);
            let base = self.best_solution.coords.get_scaled_color();
            for &s in &self.best_solution.selectors[..n] {
                let yd = t[s as usize];
                dr += (base[0] as i32 + yd).clamp(0, 255) - base[0] as i32;
                dg += (base[1] as i32 + yd).clamp(0, 255) - base[1] as i32;
                db += (base[2] as i32 + yd).clamp(0, 255) - base[2] as i32;
            }
            if dr == 0 && dg == 0 && db == 0 {
                break;
            }
            let (br1, bg1, bb1) = self.refined_color(
                dr as f32 / n as f32,
                dg as f32 / n as f32,
                db as f32 / n as f32,
            );
            if !self.evaluate_solution(
                p,
                &Etc1SolutionCoordinates::new(br1 as u32, bg1 as u32, bb1 as u32, 0, p.use_color4),
            ) {
                break;
            }
        }
    }

    #[inline]
    fn refined_color(
        &self,
        avg_delta_r_f: f32,
        avg_delta_g_f: f32,
        avg_delta_b_f: f32,
    ) -> (i32, i32, i32) {
        let lim = self.limit as f32;
        let f = |avg: f32, d: f32| (((avg - d) * lim / 255.0 + 0.5) as i32).clamp(0, self.limit);
        (
            f(self.avg_color[0], avg_delta_r_f),
            f(self.avg_color[1], avg_delta_g_f),
            f(self.avg_color[2], avg_delta_b_f),
        )
    }

    fn compute_internal_neighborhood(
        &mut self,
        p: &Etc1OptimizerParams,
        scan_r: i32,
        scan_g: i32,
        scan_b: i32,
    ) {
        if self.best_solution.error == 0 {
            return;
        }
        let deltas = p.scan_deltas;
        for &zd in deltas {
            let mbb = scan_b + zd;
            if mbb < 0 {
                continue;
            } else if mbb > self.limit {
                break;
            }
            for &yd in deltas {
                let mbg = scan_g + yd;
                if mbg < 0 {
                    continue;
                } else if mbg > self.limit {
                    break;
                }
                for &xd in deltas {
                    let mbr = scan_r + xd;
                    if mbr < 0 {
                        continue;
                    } else if mbr > self.limit {
                        break;
                    }
                    let coords = Etc1SolutionCoordinates::new(
                        mbr as u32,
                        mbg as u32,
                        mbb as u32,
                        0,
                        p.use_color4,
                    );
                    if !self.evaluate_solution(p, &coords) {
                        continue;
                    }
                    if p.refinement {
                        let trials = if p.pack.quality == BasisEtcQuality::Fast {
                            2
                        } else if (xd | yd | zd) == 0 {
                            4
                        } else {
                            2
                        };
                        self.refine_solution(p, trials);
                    }
                }
            }
        }
    }

    fn compute_internal_cluster_fit(&mut self, p: &Etc1OptimizerParams, total_perms_to_try: u32) {
        let u = self.best_solution.coords.unscaled_color;
        if !self.best_solution.valid
            || self.br != u[0] as i32
            || self.bg != u[1] as i32
            || self.bb != u[2] as i32
        {
            let c = Etc1SolutionCoordinates::new(
                self.br as u32,
                self.bg as u32,
                self.bb as u32,
                0,
                p.use_color4,
            );
            self.evaluate_solution(p, &c);
        }
        if self.best_solution.error == 0 || !self.best_solution.valid {
            return;
        }
        for i in 0..total_perms_to_try as usize {
            let t = &G_ETC1_INTEN_TABLES[self.best_solution.coords.inten_table as usize];
            let base = self.best_solution.coords.get_scaled_color();
            let num_selectors = &CLUSTER_FIT_ORDER_TAB[i];
            let (mut dr, mut dg, mut db) = (0i32, 0i32, 0i32);
            for q in 0..4 {
                let yd = t[q];
                let k = num_selectors[q] as i32;
                dr += k * ((base[0] as i32 + yd).clamp(0, 255) - base[0] as i32);
                dg += k * ((base[1] as i32 + yd).clamp(0, 255) - base[1] as i32);
                db += k * ((base[2] as i32 + yd).clamp(0, 255) - base[2] as i32);
            }
            if dr == 0 && dg == 0 && db == 0 {
                continue;
            }
            let (br1, bg1, bb1) =
                self.refined_color(dr as f32 / 8.0, dg as f32 / 8.0, db as f32 / 8.0);
            let c =
                Etc1SolutionCoordinates::new(br1 as u32, bg1 as u32, bb1 as u32, 0, p.use_color4);
            self.evaluate_solution(p, &c);
            if self.best_solution.error == 0 {
                break;
            }
        }
    }

    /// Returns false if this solution was probably already tried.
    fn check_for_redundant_solution(&mut self, coords: &Etc1SolutionCoordinates) -> bool {
        let kh = hash_hsieh(&coords.unscaled_color[..3]);
        let h0 = kh & C_SOLUTIONS_TRIED_HASH_MASK;
        let h1 = (kh >> C_SOLUTIONS_TRIED_HASH_BITS) & C_SOLUTIONS_TRIED_HASH_MASK;
        let t = &mut self.solutions_tried;
        if (t[(h0 >> 3) as usize] & (1 << (h0 & 7))) != 0
            && (t[(h1 >> 3) as usize] & (1 << (h1 & 7))) != 0
        {
            return false;
        }
        t[(h0 >> 3) as usize] |= 1 << (h0 & 7);
        t[(h1 >> 3) as usize] |= 1 << (h1 & 7);
        true
    }

    #[inline]
    fn evaluate_solution(
        &mut self,
        p: &Etc1OptimizerParams,
        coords: &Etc1SolutionCoordinates,
    ) -> bool {
        if p.pack.quality >= BasisEtcQuality::Medium {
            self.evaluate_solution_slow(p, coords)
        } else {
            self.evaluate_solution_fast(p, coords)
        }
    }

    fn violates_base_color5(p: &Etc1OptimizerParams, coords: &Etc1SolutionCoordinates) -> bool {
        if !p.constrain_against_base_color5 {
            return false;
        }
        let dr = coords.unscaled_color[0] as i32 - p.base_color5[0] as i32;
        let dg = coords.unscaled_color[1] as i32 - p.base_color5[1] as i32;
        let db = coords.unscaled_color[2] as i32 - p.base_color5[2] as i32;
        dr.min(dg).min(db) < C_ETC1_COLOR_DELTA_MIN || dr.max(dg).max(db) > C_ETC1_COLOR_DELTA_MAX
    }

    /// Accepts the trial solution if it beats the best one (`*pBest_solution = trial_solution`).
    fn finish_trial(&mut self, p: &Etc1OptimizerParams, coords: &Etc1SolutionCoordinates) -> bool {
        self.trial_solution.coords.unscaled_color = coords.unscaled_color;
        self.trial_solution.coords.color4 = p.use_color4;
        if self.trial_solution.error < self.best_solution.error {
            self.best_solution.assign_from(&self.trial_solution);
            true
        } else {
            false
        }
    }

    fn evaluate_solution_slow(
        &mut self,
        p: &Etc1OptimizerParams,
        coords: &Etc1SolutionCoordinates,
    ) -> bool {
        if !self.check_for_redundant_solution(coords) {
            return false;
        }
        self.trial_solution.valid = false;
        if Self::violates_base_color5(p, coords) {
            return false;
        }
        let base_color = coords.get_scaled_color();
        let n = p.num_src_pixels as usize;
        self.trial_solution.error = i64::MAX as u64;
        let force = p.force_selectors;

        for inten_table in 0..C_ETC1_INTEN_MODIFIER_VALUES as usize {
            if p.pack.quality <= BasisEtcQuality::Medium
                && EVAL_DIST_TABLES[inten_table][self.max_comp_spread as usize] == 0
            {
                continue;
            }
            let t = &G_ETC1_INTEN_TABLES[inten_table];
            let mut block_colors = [[0u8; 4]; 4];
            for s in 0..4 {
                let yd = t[s];
                block_colors[s] = color_rgba(
                    base_color[0] as i32 + yd,
                    base_color[1] as i32 + yd,
                    base_color[2] as i32 + yd,
                    255,
                );
            }
            let mut total_error = 0u64;
            for c in 0..n {
                let src_pixel = &p.src_pixels[c];
                let (best_selector_index, best_error) = match force {
                    Some(fs) => {
                        let s = fs[c] as usize;
                        (
                            s,
                            color_distance(p.pack.perceptual, src_pixel, &block_colors[s], false),
                        )
                    }
                    None => {
                        let mut bsi = 0;
                        let mut be =
                            color_distance(p.pack.perceptual, src_pixel, &block_colors[0], false);
                        for s in 1..4 {
                            let e = color_distance(
                                p.pack.perceptual,
                                src_pixel,
                                &block_colors[s],
                                false,
                            );
                            if e < be {
                                be = e;
                                bsi = s;
                            }
                        }
                        (bsi, be)
                    }
                };
                self.temp_selectors[c] = best_selector_index as u8;
                total_error += best_error as u64;
                if total_error >= self.trial_solution.error {
                    break;
                }
            }
            if total_error < self.trial_solution.error {
                self.trial_solution.error = total_error;
                self.trial_solution.coords.inten_table = inten_table as u32;
                std::mem::swap(&mut self.trial_solution.selectors, &mut self.temp_selectors);
                self.trial_solution.valid = true;
            }
        }
        self.finish_trial(p, coords)
    }

    fn evaluate_solution_fast(
        &mut self,
        p: &Etc1OptimizerParams,
        coords: &Etc1SolutionCoordinates,
    ) -> bool {
        if !self.check_for_redundant_solution(coords) {
            return false;
        }
        if Self::violates_base_color5(p, coords) {
            self.trial_solution.valid = false;
            return false;
        }
        let base_color = coords.get_scaled_color();
        let n = p.num_src_pixels as usize;
        self.trial_solution.error = u64::MAX;
        let perceptual = if p.pack.quality == BasisEtcQuality::Fast {
            false
        } else {
            p.pack.perceptual
        };

        for inten_table in (0..C_ETC1_INTEN_MODIFIER_VALUES as usize).rev() {
            let t = &G_ETC1_INTEN_TABLES[inten_table];
            let mut block_inten = [0u32; 4];
            let mut block_colors = [[0u8; 4]; 4];
            for s in 0..4 {
                let yd = t[s];
                let bc = color_rgba(
                    base_color[0] as i32 + yd,
                    base_color[1] as i32 + yd,
                    base_color[2] as i32 + yd,
                    255,
                );
                block_colors[s] = bc;
                block_inten[s] = bc[0] as u32 + bc[1] as u32 + bc[2] as u32;
            }
            let mid = [
                block_inten[0] + block_inten[1],
                block_inten[1] + block_inten[2],
                block_inten[2] + block_inten[3],
            ];
            let mut total_error = 0u64;
            let src = p.src_pixels;
            let dist = |a: &ColorRgba, b: &ColorRgba| -> u64 {
                if perceptual {
                    color_distance(true, a, b, false) as u64
                } else {
                    super::enc::color_distance_linear(a, b, false) as u64
                }
            };
            let sl = &self.sorted_luma;
            let sli = &self.sorted_luma_indices;
            if sl[n - 1] * 2 < mid[0] {
                if block_inten[0] > sl[n - 1] {
                    let min_error = (block_inten[0] as i32 - sl[n - 1] as i32).unsigned_abs();
                    if min_error as u64 >= self.trial_solution.error {
                        continue;
                    }
                }
                self.temp_selectors[..n].fill(0);
                for c in 0..n {
                    total_error += dist(&block_colors[0], &src[c]);
                }
            } else if sl[0] * 2 >= mid[2] {
                if sl[0] > block_inten[3] {
                    let min_error = (sl[0] as i32 - block_inten[3] as i32).unsigned_abs();
                    if min_error as u64 >= self.trial_solution.error {
                        continue;
                    }
                }
                self.temp_selectors[..n].fill(3);
                for c in 0..n {
                    total_error += dist(&block_colors[3], &src[c]);
                }
            } else {
                let mut cur_selector = 0usize;
                let mut c = 0usize;
                'outer: while c < n {
                    let y = sl[c];
                    while y * 2 >= mid[cur_selector] {
                        cur_selector += 1;
                        if cur_selector > 2 {
                            break 'outer;
                        }
                    }
                    let spi = sli[c] as usize;
                    self.temp_selectors[spi] = cur_selector as u8;
                    total_error += dist(&block_colors[cur_selector], &src[spi]);
                    c += 1;
                }
                while c < n {
                    let spi = sli[c] as usize;
                    self.temp_selectors[spi] = 3;
                    total_error += dist(&block_colors[3], &src[spi]);
                    c += 1;
                }
            }
            if total_error < self.trial_solution.error {
                self.trial_solution.error = total_error;
                self.trial_solution.coords.inten_table = inten_table as u32;
                std::mem::swap(&mut self.trial_solution.selectors, &mut self.temp_selectors);
                self.trial_solution.valid = true;
                if total_error == 0 {
                    break;
                }
            }
        }
        self.finish_trial(p, coords)
    }
}

/// `pack_etc1_block_context`.
#[derive(Debug, Clone, Default)]
pub struct PackEtc1BlockContext {
    pub optimizer: Etc1Optimizer,
}

fn etc1_decode_value(diff: u32, inten: u32, selector: u32, packed_c: u32) -> u32 {
    let mut c = if diff != 0 {
        (packed_c >> 2) | (packed_c << 3)
    } else {
        packed_c | (packed_c << 4)
    } as i32;
    c += G_ETC1_INTEN_TABLES[inten as usize][selector as usize];
    c.clamp(0, 255) as u32
}

/// `g_etc1_inverse_lookup`, built by `pack_etc1_solid_color_init`.
fn etc1_inverse_lookup() -> &'static [[u16; 256]; 64] {
    static TABLE: OnceLock<Box<[[u16; 256]; 64]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = Box::new([[0u16; 256]; 64]);
        for diff in 0..2u32 {
            let limit = if diff != 0 { 32 } else { 16 };
            for inten in 0..8u32 {
                for selector in 0..4u32 {
                    let idx = (diff + (inten << 1) + (selector << 4)) as usize;
                    for color in 0..256u32 {
                        let mut best_error = u32::MAX;
                        let mut best_packed_c = 0;
                        for packed_c in 0..limit {
                            let v = etc1_decode_value(diff, inten, selector, packed_c) as i32;
                            let err = (v - color as i32).unsigned_abs();
                            if err < best_error {
                                best_error = err;
                                best_packed_c = packed_c;
                                if best_error == 0 {
                                    break;
                                }
                            }
                        }
                        t[idx][color as usize] = (best_packed_c | (best_error << 8)) as u16;
                    }
                }
            }
        }
        t
    })
}

/// `pack_etc1_solid_color_init` (the table is also built lazily on first use).
pub fn pack_etc1_solid_color_init() {
    let _ = etc1_inverse_lookup();
}

/// `pack_etc1_block_solid_color(block, pColor)`: returns the squared error.
pub fn pack_etc1_block_solid_color(block: &mut EtcBlock, color: &[u8]) -> u64 {
    const S_NEXT_COMP: [usize; 4] = [1, 2, 0, 1];
    let inv = etc1_inverse_lookup();
    let mut best_error = u32::MAX;
    let mut best_i = 0usize;
    let (mut best_x, mut best_packed_c1, mut best_packed_c2) = (0u32, 0u32, 0u32);

    'search: for i in 0..3 {
        let c1 = color[S_NEXT_COMP[i]] as usize;
        let c2 = color[S_NEXT_COMP[i + 1]] as usize;
        for delta in -1i32..=1 {
            let c_plus_delta = (color[i] as i32 + delta).clamp(0, 255);
            let table: &[u16] = if c_plus_delta == 0 {
                &ETC1_COLOR8_TO_ETC_BLOCK_CONFIG_0_255[0]
            } else if c_plus_delta == 255 {
                &ETC1_COLOR8_TO_ETC_BLOCK_CONFIG_0_255[1]
            } else {
                &ETC1_COLOR8_TO_ETC_BLOCK_CONFIG_1_TO_254[(c_plus_delta - 1) as usize]
            };
            let mut k = 0;
            loop {
                let x = table[k] as u32;
                k += 1;
                let inverse = &inv[(x & 0xFF) as usize];
                let p1 = inverse[c1] as u32;
                let p2 = inverse[c2] as u32;
                let d0 = c_plus_delta - color[i] as i32;
                let trial_error = (d0 * d0) as u32 + (p1 >> 8) * (p1 >> 8) + (p2 >> 8) * (p2 >> 8);
                if trial_error < best_error {
                    best_error = trial_error;
                    best_x = x;
                    best_packed_c1 = p1 & 0xFF;
                    best_packed_c2 = p2 & 0xFF;
                    best_i = i;
                    if best_error == 0 {
                        break 'search;
                    }
                }
                if table[k] == 0xFFFF {
                    break;
                }
            }
        }
    }

    let diff = best_x & 1;
    let inten = (best_x >> 1) & 7;
    block.bytes[3] = ((((inten | (inten << 3)) << 2) | (diff << 1)) & 0xFF) as u8;
    let etc1_selector = G_SELECTOR_INDEX_TO_ETC1[((best_x >> 4) & 3) as usize];
    let s0: u8 = if etc1_selector & 2 != 0 { 0xFF } else { 0 };
    let s1: u8 = if etc1_selector & 1 != 0 { 0xFF } else { 0 };
    block.bytes[4] = s0;
    block.bytes[5] = s0;
    block.bytes[6] = s1;
    block.bytes[7] = s1;
    let best_packed_c0 = (best_x >> 8) & 255;
    let (a, b, c) = (best_i, S_NEXT_COMP[best_i], S_NEXT_COMP[best_i + 1]);
    if diff != 0 {
        block.bytes[a] = (best_packed_c0 << 3) as u8;
        block.bytes[b] = (best_packed_c1 << 3) as u8;
        block.bytes[c] = (best_packed_c2 << 3) as u8;
    } else {
        block.bytes[a] = (best_packed_c0 | (best_packed_c0 << 4)) as u8;
        block.bytes[b] = (best_packed_c1 | (best_packed_c1 << 4)) as u8;
        block.bytes[c] = (best_packed_c2 | (best_packed_c2 << 4)) as u8;
    }
    best_error as u64
}

/// `eac_a8_block`: byte 0 = base, byte 1 = table | multiplier << 4, then 6 selector bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EacA8Block {
    pub base: u8,
    pub table: u8,
    pub multiplier: u8,
    pub selectors: [u8; 6],
}

impl EacA8Block {
    #[inline]
    pub fn get_selector(&self, x: u32, y: u32, selector_bits: u64) -> u32 {
        ((selector_bits >> (45 - (y + x * 4) * 3)) & 7) as u32
    }

    #[inline]
    pub fn get_selector_bits(&self) -> u64 {
        let s = &self.selectors;
        ((s[0] as u64) << 40)
            | ((s[1] as u64) << 32)
            | ((s[2] as u64) << 24)
            | ((s[3] as u64) << 16)
            | ((s[4] as u64) << 8)
            | s[5] as u64
    }

    #[inline]
    pub fn set_selector_bits(&mut self, pixels: u64) {
        for i in 0..6 {
            self.selectors[i] = (pixels >> (40 - 8 * i)) as u8;
        }
    }

    pub fn set_selector(&mut self, x: u32, y: u32, s: u32) {
        let ofs = 45 - (y + x * 4) * 3;
        let mut pixels = self.get_selector_bits();
        pixels &= !(7u64 << ofs);
        pixels |= (s as u64) << ofs;
        self.set_selector_bits(pixels);
    }

    pub fn to_bytes(&self) -> [u8; 8] {
        let s = &self.selectors;
        [
            self.base,
            (self.table & 15) | (self.multiplier << 4),
            s[0],
            s[1],
            s[2],
            s[3],
            s[4],
            s[5],
        ]
    }
}

/// `etc2_rgba_block`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Etc2RgbaBlock {
    pub alpha: EacA8Block,
    pub rgb: EtcBlock,
}

/// `pack_eac_a8_results`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackEacA8Results {
    pub base: u32,
    pub table: u32,
    pub multiplier: u32,
    pub selectors: Vec<u8>,
    pub selectors_temp: Vec<u8>,
}

/// `pack_eac_a8(results, pPixels, num_pixels, base_search_rad, mul_search_rad, table_mask)`.
pub fn pack_eac_a8_results(
    results: &mut PackEacA8Results,
    pixels: &[u8],
    num_pixels: u32,
    base_search_rad: u32,
    mul_search_rad: u32,
    table_mask: u32,
) -> u64 {
    let n = num_pixels as usize;
    results.selectors.resize(n, 0);
    results.selectors_temp.resize(n, 0);

    let (mut min_alpha, mut max_alpha) = (255u32, 0u32);
    for &a in &pixels[..n] {
        min_alpha = min_alpha.min(a as u32);
        max_alpha = max_alpha.max(a as u32);
    }
    if min_alpha == max_alpha {
        results.base = min_alpha;
        results.table = 13;
        results.multiplier = 1;
        results.selectors[..n].fill(4);
        return 0;
    }
    let alpha_range = max_alpha - min_alpha;
    let mut best_err = u64::MAX;

    for table in 0..16usize {
        if table_mask & (1u32 << table) == 0 {
            continue;
        }
        let tbl = &G_ETC2_EAC_TABLES[table];
        let range = (tbl[ETC2_EAC_MAX_VALUE_SELECTOR] as i32
            - tbl[ETC2_EAC_MIN_VALUE_SELECTOR] as i32) as f32;
        let (lo, hi) = (min_alpha as f32, max_alpha as f32);
        let center = (lo
            + (hi - lo) * ((0 - tbl[ETC2_EAC_MIN_VALUE_SELECTOR] as i32) as f32 / range))
            .round() as i32;
        let base_min = clamp255(center.wrapping_sub(base_search_rad as i32)) as i32;
        let base_max = clamp255(center.wrapping_add(base_search_rad as i32)) as i32;
        let mul = (alpha_range as f32 / range).round() as i32;
        let mul_low = mul.wrapping_sub(mul_search_rad as i32).clamp(1, 15);
        let mul_high = mul.wrapping_add(mul_search_rad as i32).clamp(1, 15);

        for base in base_min..=base_max {
            for multiplier in mul_low..=mul_high {
                let mut total_err = 0u64;
                for i in 0..n {
                    let a = pixels[i] as i32;
                    let mut best_s_err = u32::MAX;
                    let mut best_s = 0u32;
                    for s in 0..8 {
                        let v = clamp255(multiplier * tbl[s] as i32 + base) as i32;
                        let err = (a - v).unsigned_abs();
                        if err < best_s_err {
                            best_s_err = err;
                            best_s = s as u32;
                        }
                    }
                    results.selectors_temp[i] = best_s as u8;
                    total_err += best_s_err.wrapping_mul(best_s_err) as u64;
                    if total_err >= best_err {
                        break;
                    }
                }
                if total_err < best_err {
                    best_err = total_err;
                    results.base = base as u32;
                    results.multiplier = multiplier as u32;
                    results.table = table as u32;
                    std::mem::swap(&mut results.selectors, &mut results.selectors_temp);
                    if best_err == 0 {
                        return best_err;
                    }
                }
            }
        }
    }
    best_err
}

/// `pack_eac_a8(pBlock, pPixels, base_search_rad, mul_search_rad, table_mask)` (16 pixels).
pub fn pack_eac_a8(
    block: &mut EacA8Block,
    pixels: &[u8],
    base_search_rad: u32,
    mul_search_rad: u32,
    table_mask: u32,
) {
    let mut results = PackEacA8Results::default();
    pack_eac_a8_results(
        &mut results,
        pixels,
        16,
        base_search_rad,
        mul_search_rad,
        table_mask,
    );
    block.base = results.base as u8;
    block.multiplier = results.multiplier as u8;
    block.table = results.table as u8;
    for y in 0..4 {
        for x in 0..4 {
            block.set_selector(x, y, results.selectors[(x + y * 4) as usize] as u32);
        }
    }
}
