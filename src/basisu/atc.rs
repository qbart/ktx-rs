//! ATC / PVRTC2 shared tables from basisu_transcoder.cpp (`transcoder_init_atc`) and the
//! ETC1S to ATC block converter (`convert_etc1s_to_atc`).

use std::sync::OnceLock;

use super::etc::{DecoderEtcBlock, Endpoint, Selector};
use super::tables::{ETC1S_TO_ATC_55, ETC1S_TO_ATC_56};

pub const ETC1S_TO_ATC_SELECTOR_RANGES: [(u32, u32); 6] =
    [(0, 3), (1, 3), (0, 2), (1, 2), (2, 3), (0, 1)];
pub const NUM_ETC1S_TO_ATC_SELECTOR_RANGES: usize = 6;
pub const NUM_ETC1S_TO_ATC_SELECTOR_MAPPINGS: usize = 10;
pub const ETC1S_TO_ATC_SELECTOR_MAPPINGS: [[u8; 4]; 10] = [
    [0, 0, 1, 1],
    [0, 0, 1, 2],
    [0, 0, 1, 3],
    [0, 0, 2, 3],
    [0, 1, 1, 1],
    [0, 1, 2, 2],
    [0, 1, 2, 3], // 6 - identity
    [0, 2, 3, 3],
    [1, 2, 2, 2],
    [1, 2, 3, 3],
];
pub const ATC_IDENTITY_SELECTOR_MAPPING_INDEX: usize = 6;

/// `atc_match_entry` as (lo, hi).
pub type MatchTable = [(u8, u8); 256];

/// Tables built by `transcoder_init_atc`.
pub struct AtcTables {
    pub pvrtc2_match45_equals_1: MatchTable,
    pub atc_match55_equals_1: MatchTable,
    pub atc_match56_equals_1: MatchTable,
    pub pvrtc2_match4: MatchTable,
    pub atc_match5: MatchTable,
    pub atc_match6: MatchTable,
    pub selector_range_index: [[u32; 4]; 4],
}

fn expand(v: i32, size: i32) -> i32 {
    match size {
        // PVRTC2 only - expand to 5 then 8.
        16 => {
            let v = (v << 1) | (v >> 3);
            (v << 3) | (v >> 2)
        }
        32 => (v << 3) | (v >> 2),
        _ => (v << 2) | (v >> 4),
    }
}

/// Port of `prepare_atc_single_color_table`.
fn prepare_atc_single_color_table(size0: i32, size1: i32, sel: i32) -> MatchTable {
    let mut t = [(0u8, 0u8); 256];
    for (i, slot) in t.iter_mut().enumerate() {
        let i = i as i32;
        let mut lowest_e = 256;
        for lo in 0..size0 {
            // size0 == 1 expands like 64 (lo is always 0).
            let lo_e = expand(lo, size0);
            for hi in 0..size1 {
                let hi_e = expand(hi, size1);
                let e = if sel == 1 {
                    (((lo_e * 5 + hi_e * 3) / 8) - i).abs()
                } else {
                    (hi_e - i).abs()
                };
                if e < lowest_e {
                    *slot = (lo as u8, hi as u8);
                    lowest_e = e;
                }
            }
        }
    }
    t
}

pub fn atc_tables() -> &'static AtcTables {
    static T: OnceLock<AtcTables> = OnceLock::new();
    T.get_or_init(|| {
        let mut selector_range_index = [[0u32; 4]; 4];
        for (i, &(l, h)) in ETC1S_TO_ATC_SELECTOR_RANGES.iter().enumerate() {
            selector_range_index[l as usize][h as usize] = i as u32;
        }
        AtcTables {
            pvrtc2_match45_equals_1: prepare_atc_single_color_table(16, 32, 1),
            atc_match55_equals_1: prepare_atc_single_color_table(32, 32, 1),
            atc_match56_equals_1: prepare_atc_single_color_table(32, 64, 1),
            pvrtc2_match4: prepare_atc_single_color_table(1, 16, 3),
            atc_match5: prepare_atc_single_color_table(1, 32, 3),
            atc_match6: prepare_atc_single_color_table(1, 64, 3),
            selector_range_index,
        }
    })
}

/// Start index of a `[32][8][RANGES][MAPPINGS]` solution table row.
#[inline]
pub fn table_base(inten: u32, base: u8, range: u32) -> usize {
    (inten as usize * 32 + base as usize)
        * (NUM_ETC1S_TO_ATC_SELECTOR_RANGES * NUM_ETC1S_TO_ATC_SELECTOR_MAPPINGS)
        + range as usize * NUM_ETC1S_TO_ATC_SELECTOR_MAPPINGS
}

/// Translates 4 rows of linear 2-bit selectors through `xlat`.
pub fn xlat_selectors(selectors: &[u8; 4], xlat: &[u8; 4]) -> [u8; 4] {
    selectors.map(|bits| {
        let mut out = 0u8;
        for x in 0..4 {
            out |= xlat[((bits >> (x * 2)) & 3) as usize] << (x * 2);
        }
        out
    })
}

#[inline]
fn sol(t: &[(u8, u8, u16)], idx: usize) -> (u8, u8, u16) {
    t.get(idx).copied().unwrap_or((0, 0, 0))
}

/// `atc_block`: 5:5:5 low color, 5:6:5 high color, 4 rows of 2-bit selectors.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct AtcBlock {
    pub lo: [u8; 2],
    pub hi: [u8; 2],
    pub sels: [u8; 4],
}

impl AtcBlock {
    pub fn set_low_color(&mut self, r: u32, g: u32, b: u32) {
        let x = (r << 10) | (g << 5) | b;
        self.lo = [x as u8, (x >> 8) as u8];
    }

    pub fn set_high_color(&mut self, r: u32, g: u32, b: u32) {
        let x = (r << 11) | (g << 5) | b;
        self.hi = [x as u8, (x >> 8) as u8];
    }

    pub fn to_bytes(&self) -> [u8; 8] {
        [
            self.lo[0],
            self.lo[1],
            self.hi[0],
            self.hi[1],
            self.sels[0],
            self.sels[1],
            self.sels[2],
            self.sels[3],
        ]
    }
}

/// Port of `convert_etc1s_to_atc`.
pub fn convert_etc1s_to_atc(blk: &mut AtcBlock, ep: &Endpoint, sel: &Selector) {
    let at = atc_tables();
    let low_selector = sel.lo_selector as u32;
    let high_selector = sel.hi_selector as u32;
    let base_color = &ep.color5;
    let inten_table = ep.inten5 as u32;

    if low_selector == high_selector {
        let (r, g, b) = DecoderEtcBlock::get_block_color5(base_color, inten_table, low_selector);
        let (r, g, b) = (r as usize, g as usize, b as usize);
        let m55 = &at.atc_match55_equals_1;
        let m56 = &at.atc_match56_equals_1;
        blk.set_low_color(m55[r].0 as u32, m56[g].0 as u32, m55[b].0 as u32);
        blk.set_high_color(m55[r].1 as u32, m56[g].1 as u32, m55[b].1 as u32);
        blk.sels = [0x55; 4];
        return;
    } else if inten_table >= 7
        && sel.num_unique_selectors == 2
        && sel.lo_selector == 0
        && sel.hi_selector == 3
    {
        let bc = DecoderEtcBlock::get_block_colors5(base_color, inten_table);
        let (c0, c1) = (&bc[0], &bc[3]);
        let m5 = &at.atc_match5;
        blk.set_low_color(
            m5[c0.r() as usize].1 as u32,
            m5[c0.g() as usize].1 as u32,
            m5[c0.b() as usize].1 as u32,
        );
        blk.set_high_color(
            m5[c1.r() as usize].1 as u32,
            at.atc_match6[c1.g() as usize].1 as u32,
            m5[c1.b() as usize].1 as u32,
        );
        blk.sels = sel.selectors;
        return;
    }

    let rt = at.selector_range_index[(low_selector & 3) as usize][(high_selector & 3) as usize];
    let br = table_base(inten_table, base_color.r(), rt);
    let bg = table_base(inten_table, base_color.g(), rt);
    let bb = table_base(inten_table, base_color.b(), rt);

    let mut best_err = u32::MAX;
    let mut best_mapping = 0;
    for m in 0..NUM_ETC1S_TO_ATC_SELECTOR_MAPPINGS {
        let total = sol(&ETC1S_TO_ATC_55, br + m).2 as u32
            + sol(&ETC1S_TO_ATC_56, bg + m).2 as u32
            + sol(&ETC1S_TO_ATC_55, bb + m).2 as u32;
        if total < best_err {
            best_err = total;
            best_mapping = m;
        }
    }
    let r = sol(&ETC1S_TO_ATC_55, br + best_mapping);
    let g = sol(&ETC1S_TO_ATC_56, bg + best_mapping);
    let b = sol(&ETC1S_TO_ATC_55, bb + best_mapping);
    blk.set_low_color(r.0 as u32, g.0 as u32, b.0 as u32);
    blk.set_high_color(r.1 as u32, g.1 as u32, b.1 as u32);

    blk.sels = if best_mapping == ATC_IDENTITY_SELECTOR_MAPPING_INDEX {
        sel.selectors
    } else {
        xlat_selectors(
            &sel.selectors,
            &ETC1S_TO_ATC_SELECTOR_MAPPINGS[best_mapping],
        )
    };
}
