//! Port of the ETC1S -> ETC2 EAC A8 / EAC R11 block converters (basisu_transcoder.cpp).

use super::etc::{ETC1_INTEN_TABLES, Endpoint, Selector, clamp255};
use super::tables::{ETC1_G_TO_ETC2_A8, ETC1_G_TO_ETC2_R11};

/// `g_eac_modifier_table`.
pub const EAC_MODIFIER_TABLE: [[i8; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

/// `g_etc2_eac_a8_sel4`: all selectors = 4.
pub const ETC2_EAC_A8_SEL4: [u8; 6] = [0x92, 0x49, 0x24, 0x92, 0x49, 0x24];

/// `s_etc2_eac_selector_ranges`.
pub const ETC2_EAC_SELECTOR_RANGES: [(u32, u32); 4] = [(0, 3), (1, 3), (0, 2), (1, 2)];

/// Port of `eac_block` (8 bytes: base, table|multiplier<<4, 48 selector bits).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EacBlock {
    pub bytes: [u8; 8],
}

impl EacBlock {
    pub fn set_base(&mut self, b: u32) {
        self.bytes[0] = b as u8;
    }
    pub fn set_table(&mut self, t: u32) {
        self.bytes[1] = (self.bytes[1] & 0xF0) | (t as u8 & 0xF);
    }
    pub fn set_multiplier(&mut self, m: u32) {
        self.bytes[1] = (self.bytes[1] & 0x0F) | ((m as u8 & 0xF) << 4);
    }
    pub fn set_selector_bits(&mut self, pixels: u64) {
        for i in 0..6 {
            self.bytes[2 + i] = (pixels >> (40 - 8 * i)) as u8;
        }
    }
    pub fn get_selector_bits(&self) -> u64 {
        (0..6).fold(0u64, |acc, i| {
            acc | ((self.bytes[2 + i] as u64) << (40 - 8 * i))
        })
    }
    pub fn set_selector(&mut self, x: u32, y: u32, s: u32) {
        let ofs = 45 - ((y & 3) + (x & 3) * 4) * 3;
        let mut p = self.get_selector_bits();
        p &= !(7u64 << ofs);
        p |= ((s & 7) as u64) << ofs;
        self.set_selector_bits(p);
    }
    /// Opaque block used by `write_opaque_alpha_blocks` (base 255, table 13, mul 1, sel 4).
    pub fn opaque() -> EacBlock {
        let mut b = EacBlock::default();
        b.set_base(255);
        b.set_multiplier(1);
        b.set_table(13);
        b.bytes[2..8].copy_from_slice(&ETC2_EAC_A8_SEL4);
        b
    }
}

fn convert_with_table(dst: &mut EacBlock, ep: &Endpoint, sel: &Selector, table: &[(u8, u8, u16)]) {
    let low_selector = sel.lo_selector as u32;
    let high_selector = sel.hi_selector as u32;
    let base_color = &ep.color5;
    let inten_table = ep.inten5 as u32;

    if low_selector == high_selector {
        let br = (((base_color.r() as u32) << 3) | ((base_color.r() as u32) >> 2)) as i32;
        let r = clamp255(
            br + ETC1_INTEN_TABLES[(inten_table & 7) as usize][(low_selector & 3) as usize],
        ) as u32;
        // Constant block: table 13, multiplier 1, all selectors 4.
        dst.set_base(r);
        dst.set_table(13);
        dst.set_multiplier(1);
        dst.bytes[2..8].copy_from_slice(&ETC2_EAC_A8_SEL4);
        return;
    }
    let selector_range_table = ETC2_EAC_SELECTOR_RANGES
        .iter()
        .position(|&(l, h)| l == low_selector && h == high_selector)
        .unwrap_or(0);
    let idx = (base_color.r() as usize + inten_table as usize * 32)
        * ETC2_EAC_SELECTOR_RANGES.len()
        + selector_range_table;
    let (base, table_mul, trans) = table.get(idx).copied().unwrap_or((0, 0, 0));
    dst.set_base(base as u32);
    dst.set_table((table_mul >> 4) as u32);
    dst.set_multiplier((table_mul & 15) as u32);
    let mut selector_bits = 0u64;
    for y in 0..4 {
        for x in 0..4 {
            let s = sel.get_selector(x, y);
            let ds = ((trans as u32) >> (s * 3)) & 7;
            let dst_ofs = 45 - (y + x * 4) * 3;
            selector_bits |= (ds as u64) << dst_ofs;
        }
    }
    dst.set_selector_bits(selector_bits);
}

/// Port of `convert_etc1s_to_etc2_eac_a8`.
pub fn convert_etc1s_to_etc2_eac_a8(dst: &mut EacBlock, ep: &Endpoint, sel: &Selector) {
    convert_with_table(dst, ep, sel, &ETC1_G_TO_ETC2_A8);
}

/// Port of `convert_etc1s_to_etc2_eac_r11`.
pub fn convert_etc1s_to_etc2_eac_r11(dst: &mut EacBlock, ep: &Endpoint, sel: &Selector) {
    convert_with_table(dst, ep, sel, &ETC1_G_TO_ETC2_R11);
}
