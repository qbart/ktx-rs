//! Port of the ETC1S -> BC1 (DXT1) and BC4 (DXT5A) block converters and their
//! init-time tables (basisu_transcoder.cpp).

use std::sync::OnceLock;

use super::etc::{DecoderEtcBlock, Endpoint, Selector};
use super::tables::{ETC1_G_TO_DXT5A, ETC1_TO_DXT_5, ETC1_TO_DXT_6};

/// `g_etc1_to_dxt1_selector_ranges`.
pub const ETC1_TO_DXT1_SELECTOR_RANGES: [(u32, u32); 6] =
    [(0, 3), (1, 3), (0, 2), (1, 2), (2, 3), (0, 1)];
pub const NUM_ETC1_TO_DXT1_SELECTOR_RANGES: usize = 6;
pub const NUM_ETC1_TO_DXT1_SELECTOR_MAPPINGS: usize = 10;

/// `g_etc1_to_dxt1_selector_mappings`.
pub const ETC1_TO_DXT1_SELECTOR_MAPPINGS: [[u8; 4]; 10] = [
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

/// `bc1_match_entry`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bc1MatchEntry {
    pub hi: u8,
    pub lo: u8,
}

/// Tables built by `basisu_transcoder_init` for DXT1.
pub struct DxtTables {
    pub bc1_match5_equals_1: [Bc1MatchEntry; 256],
    pub bc1_match6_equals_1: [Bc1MatchEntry; 256],
    pub bc1_match5_equals_0: [Bc1MatchEntry; 256],
    pub bc1_match6_equals_0: [Bc1MatchEntry; 256],
    pub selector_range_index: [[u32; 4]; 4],
    pub mappings_raw_dxt1_256: [[u8; 256]; 10],
    pub mappings_raw_dxt1_inv_256: [[u8; 256]; 10],
}

/// Port of `prepare_bc1_single_color_table`.
fn prepare_bc1_single_color_table(
    expand: &[u8],
    size0: usize,
    size1: usize,
    sel: u32,
) -> [Bc1MatchEntry; 256] {
    let mut table = [Bc1MatchEntry::default(); 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let i = i as i32;
        let mut lowest_e = 256;
        for lo in 0..size0 {
            for hi in 0..size1 {
                let lo_e = expand[lo] as i32;
                let hi_e = expand[hi] as i32;
                let e = if sel == 1 {
                    ((hi_e * 2 + lo_e) / 3 - i).abs() + ((hi_e - lo_e).abs() * 3) / 100
                } else {
                    (hi_e - i).abs()
                };
                if e < lowest_e {
                    entry.hi = hi as u8;
                    entry.lo = lo as u8;
                    lowest_e = e;
                }
            }
        }
    }
    table
}

/// Lazily built DXT tables (thread safe, built once).
pub fn dxt_tables() -> &'static DxtTables {
    static TABLES: OnceLock<DxtTables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let expand5: Vec<u8> = (0..32u32).map(|i| ((i << 3) | (i >> 2)) as u8).collect();
        let expand6: Vec<u8> = (0..64u32).map(|i| ((i << 2) | (i >> 4)) as u8).collect();
        let mut selector_range_index = [[0u32; 4]; 4];
        for (i, &(l, h)) in ETC1_TO_DXT1_SELECTOR_RANGES.iter().enumerate() {
            selector_range_index[l as usize][h as usize] = i as u32;
        }
        let mut raw = [[0u8; 256]; 10];
        let mut raw_inv = [[0u8; 256]; 10];
        const LINEAR_DXT1_TO_DXT1: [u8; 4] = [0, 2, 3, 1];
        const DXT1_INVERTED_XLAT: [u8; 4] = [1, 0, 3, 2];
        for sm in 0..NUM_ETC1_TO_DXT1_SELECTOR_MAPPINGS {
            let mut m = [0u8; 4];
            let mut m_inv = [0u8; 4];
            for j in 0..4 {
                m[j] = LINEAR_DXT1_TO_DXT1[ETC1_TO_DXT1_SELECTOR_MAPPINGS[sm][j] as usize];
                m_inv[j] = DXT1_INVERTED_XLAT[m[j] as usize];
            }
            for i in 0..256usize {
                let mut k = 0u32;
                let mut k_inv = 0u32;
                for s in 0..4 {
                    k |= (m[(i >> (s * 2)) & 3] as u32) << (s * 2);
                    k_inv |= (m_inv[(i >> (s * 2)) & 3] as u32) << (s * 2);
                }
                raw[sm][i] = k as u8;
                raw_inv[sm][i] = k_inv as u8;
            }
        }
        DxtTables {
            bc1_match5_equals_1: prepare_bc1_single_color_table(&expand5, 32, 32, 1),
            bc1_match5_equals_0: prepare_bc1_single_color_table(&expand5, 1, 32, 0),
            bc1_match6_equals_1: prepare_bc1_single_color_table(&expand6, 64, 64, 1),
            bc1_match6_equals_0: prepare_bc1_single_color_table(&expand6, 1, 64, 0),
            selector_range_index,
            mappings_raw_dxt1_256: raw,
            mappings_raw_dxt1_inv_256: raw_inv,
        }
    })
}

/// Port of `dxt1_block` (8 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dxt1Block {
    pub bytes: [u8; 8],
}

impl Dxt1Block {
    pub fn set_low_color(&mut self, c: u16) {
        self.bytes[0..2].copy_from_slice(&c.to_le_bytes());
    }
    pub fn set_high_color(&mut self, c: u16) {
        self.bytes[2..4].copy_from_slice(&c.to_le_bytes());
    }
    pub fn get_low_color(&self) -> u32 {
        u16::from_le_bytes([self.bytes[0], self.bytes[1]]) as u32
    }
    pub fn get_high_color(&self) -> u32 {
        u16::from_le_bytes([self.bytes[2], self.bytes[3]]) as u32
    }
    pub fn selectors_mut(&mut self) -> &mut [u8] {
        &mut self.bytes[4..8]
    }
    pub fn get_selector(&self, x: u32, y: u32) -> u32 {
        ((self.bytes[4 + (y & 3) as usize] >> ((x & 3) * 2)) & 3) as u32
    }
    pub fn set_selector(&mut self, x: u32, y: u32, val: u32) {
        let b = &mut self.bytes[4 + (y & 3) as usize];
        *b &= !(3u8 << ((x & 3) * 2));
        *b |= ((val & 3) << ((x & 3) * 2)) as u8;
    }
    /// Port of `pack_color`.
    pub fn pack_color(r: u32, g: u32, b: u32, scaled: bool, bias: u32) -> u16 {
        let (mut r, mut g, mut b) = (r, g, b);
        if scaled {
            r = (r * 31 + bias) / 255;
            g = (g * 63 + bias) / 255;
            b = (b * 31 + bias) / 255;
        }
        (b.min(31) | (g.min(63) << 5) | (r.min(31) << 11)) as u16
    }
    /// Port of `pack_unscaled_color`.
    pub fn pack_unscaled_color(r: u32, g: u32, b: u32) -> u32 {
        b | (g << 5) | (r << 11)
    }
}

/// Port of `convert_etc1s_to_dxt1`.
pub fn convert_etc1s_to_dxt1(
    dst: &mut Dxt1Block,
    ep: &Endpoint,
    sel: &Selector,
    use_threecolor_blocks: bool,
) {
    let t = dxt_tables();
    let low_selector = sel.lo_selector as u32;
    let high_selector = sel.hi_selector as u32;
    let base_color = &ep.color5;
    let inten_table = ep.inten5 as u32;

    if low_selector == high_selector {
        let (r, g, b) = DecoderEtcBlock::get_block_color5(base_color, inten_table, low_selector);
        let mut mask: u32 = 0xAA;
        let mut max16 = ((t.bc1_match5_equals_1[r as usize].hi as u32) << 11)
            | ((t.bc1_match6_equals_1[g as usize].hi as u32) << 5)
            | t.bc1_match5_equals_1[b as usize].hi as u32;
        let mut min16 = ((t.bc1_match5_equals_1[r as usize].lo as u32) << 11)
            | ((t.bc1_match6_equals_1[g as usize].lo as u32) << 5)
            | t.bc1_match5_equals_1[b as usize].lo as u32;
        if !use_threecolor_blocks && min16 == max16 {
            // Guarantee BC3 blocks never use punchthrough alpha (3 color) mode.
            mask = 0;
            if min16 > 0 {
                min16 -= 1;
            } else {
                max16 = 1;
                min16 = 0;
                mask = 0x55;
            }
        }
        if max16 < min16 {
            std::mem::swap(&mut max16, &mut min16);
            mask ^= 0x55;
        }
        dst.set_low_color(max16 as u16);
        dst.set_high_color(min16 as u16);
        dst.selectors_mut().fill(mask as u8);
        return;
    } else if inten_table >= 7
        && sel.num_unique_selectors == 2
        && sel.lo_selector == 0
        && sel.hi_selector == 3
    {
        let block_colors = DecoderEtcBlock::get_block_colors5(base_color, inten_table);
        let (r0, g0, b0) = (
            block_colors[0].r() as usize,
            block_colors[0].g() as usize,
            block_colors[0].b() as usize,
        );
        let (r1, g1, b1) = (
            block_colors[3].r() as usize,
            block_colors[3].g() as usize,
            block_colors[3].b() as usize,
        );
        let mut max16 = ((t.bc1_match5_equals_0[r0].hi as u32) << 11)
            | ((t.bc1_match6_equals_0[g0].hi as u32) << 5)
            | t.bc1_match5_equals_0[b0].hi as u32;
        let mut min16 = ((t.bc1_match5_equals_0[r1].hi as u32) << 11)
            | ((t.bc1_match6_equals_0[g1].hi as u32) << 5)
            | t.bc1_match5_equals_0[b1].hi as u32;
        let (mut l, mut h) = (0u32, 1u32);
        if min16 == max16 {
            if min16 > 0 {
                min16 -= 1;
                l = 0;
                h = 0;
            } else {
                max16 = 1;
                min16 = 0;
                l = 1;
                h = 1;
            }
        }
        if max16 < min16 {
            std::mem::swap(&mut max16, &mut min16);
            l = 1;
            h = 0;
        }
        dst.set_low_color(max16 as u16);
        dst.set_high_color(min16 as u16);
        for y in 0..4 {
            for x in 0..4 {
                let s = sel.get_selector(x, y);
                dst.set_selector(x, y, if s == 3 { h } else { l });
            }
        }
        return;
    }

    let selector_range_table =
        t.selector_range_index[(low_selector & 3) as usize][(high_selector & 3) as usize] as usize;
    let stride = NUM_ETC1_TO_DXT1_SELECTOR_RANGES * NUM_ETC1_TO_DXT1_SELECTOR_MAPPINGS;
    let base = |c: u8| {
        (inten_table as usize * 32 + c as usize) * stride
            + selector_range_table * NUM_ETC1_TO_DXT1_SELECTOR_MAPPINGS
    };
    let tr = base(base_color.r());
    let tg = base(base_color.g());
    let tb = base(base_color.b());
    let get = |tab: &[(u8, u8, u16)], i: usize| tab.get(i).copied().unwrap_or((0, 0, 0));

    let mut best_err = u32::MAX;
    let mut best_mapping = 0usize;
    for m in 0..NUM_ETC1_TO_DXT1_SELECTOR_MAPPINGS {
        let total_err = get(&ETC1_TO_DXT_5, tr + m).2 as u32
            + get(&ETC1_TO_DXT_6, tg + m).2 as u32
            + get(&ETC1_TO_DXT_5, tb + m).2 as u32;
        if total_err < best_err {
            best_err = total_err;
            best_mapping = m;
        }
    }
    let (rr, gg, bb) = (
        get(&ETC1_TO_DXT_5, tr + best_mapping),
        get(&ETC1_TO_DXT_6, tg + best_mapping),
        get(&ETC1_TO_DXT_5, tb + best_mapping),
    );
    let mut l = Dxt1Block::pack_unscaled_color(rr.0 as u32, gg.0 as u32, bb.0 as u32);
    let mut h = Dxt1Block::pack_unscaled_color(rr.1 as u32, gg.1 as u32, bb.1 as u32);
    let mut xlat = &t.mappings_raw_dxt1_256[best_mapping];
    if l < h {
        std::mem::swap(&mut l, &mut h);
        xlat = &t.mappings_raw_dxt1_inv_256[best_mapping];
    }
    dst.set_low_color(l as u16);
    dst.set_high_color(h as u16);
    if l == h {
        let mut mask = 0u8;
        if !use_threecolor_blocks {
            if h > 0 {
                h -= 1;
            } else {
                h = 0;
                l = 1;
                mask = 0x55;
            }
            dst.set_low_color(l as u16);
            dst.set_high_color(h as u16);
        }
        dst.selectors_mut().fill(mask);
        return;
    }
    for i in 0..4 {
        dst.bytes[4 + i] = xlat[sel.selectors[i] as usize];
    }
}

/// `s_dxt5a_selector_ranges`.
pub const DXT5A_SELECTOR_RANGES: [(u32, u32); 4] = [(0, 3), (1, 3), (0, 2), (1, 2)];

/// Port of `dxt5a_block` (8 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dxt5aBlock {
    pub bytes: [u8; 8],
}

impl Dxt5aBlock {
    pub fn set_low_alpha(&mut self, i: u32) {
        self.bytes[0] = i as u8;
    }
    pub fn set_high_alpha(&mut self, i: u32) {
        self.bytes[1] = i as u8;
    }
    /// Port of `set_selector` (3-bit selectors packed after the endpoints).
    pub fn set_selector(&mut self, x: u32, y: u32, val: u32) {
        let selector_index = (y & 3) * 4 + (x & 3);
        let bit_index = selector_index * 3;
        let byte_index = (bit_index >> 3) as usize;
        let bit_ofs = bit_index & 7;
        let sels = &mut self.bytes[2..8];
        let mut v = sels[byte_index] as u32;
        if byte_index < 5 {
            v |= (sels[byte_index + 1] as u32) << 8;
        }
        v &= !(7 << bit_ofs);
        v |= (val & 7) << bit_ofs;
        sels[byte_index] = v as u8;
        if byte_index < 5 {
            sels[byte_index + 1] = (v >> 8) as u8;
        }
    }

    /// Opaque block used by `write_opaque_alpha_blocks`.
    pub fn opaque() -> Dxt5aBlock {
        Dxt5aBlock {
            bytes: [255, 255, 0, 0, 0, 0, 0, 0],
        }
    }
}

/// Port of `convert_etc1s_to_dxt5a`.
pub fn convert_etc1s_to_dxt5a(dst: &mut Dxt5aBlock, ep: &Endpoint, sel: &Selector) {
    let low_selector = sel.lo_selector as u32;
    let high_selector = sel.hi_selector as u32;
    let base_color = &ep.color5;
    let inten_table = ep.inten5 as u32;

    if low_selector == high_selector {
        let r = {
            let br = (((base_color.r() as u32) << 3) | ((base_color.r() as u32) >> 2)) as i32;
            super::etc::clamp255(
                br + super::etc::ETC1_INTEN_TABLES[(inten_table & 7) as usize]
                    [(low_selector & 3) as usize],
            ) as u32
        };
        dst.set_low_alpha(r);
        dst.set_high_alpha(r);
        dst.bytes[2..8].fill(0);
        return;
    } else if sel.num_unique_selectors == 2 {
        let block_colors = DecoderEtcBlock::get_block_colors5(base_color, inten_table);
        let r0 = block_colors[(low_selector & 3) as usize].r() as u32;
        let r1 = block_colors[(high_selector & 3) as usize].r() as u32;
        dst.set_low_alpha(r0);
        dst.set_high_alpha(r1);
        for y in 0..4 {
            for x in 0..4 {
                let s = sel.get_selector(x, y);
                dst.set_selector(x, y, if s == high_selector { 1 } else { 0 });
            }
        }
        return;
    }

    let mut selector_range_table = DXT5A_SELECTOR_RANGES
        .iter()
        .position(|&(l, h)| l == low_selector && h == high_selector)
        .unwrap_or(0);
    if selector_range_table >= DXT5A_SELECTOR_RANGES.len() {
        selector_range_table = 0;
    }
    let idx = (base_color.r() as usize + inten_table as usize * 32) * DXT5A_SELECTOR_RANGES.len()
        + selector_range_table;
    let (lo, hi, trans) = ETC1_G_TO_DXT5A.get(idx).copied().unwrap_or((0, 0, 0));
    dst.set_low_alpha(lo as u32);
    dst.set_high_alpha(hi as u32);
    for y in 0..4 {
        for x in 0..4 {
            let s = sel.get_selector(x, y);
            let ds = ((trans as u32) >> (s * 3)) & 7;
            dst.set_selector(x, y, ds);
        }
    }
}
