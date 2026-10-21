//! ATC / PVRTC2 shared tables from basisu_transcoder.cpp (`transcoder_init_atc`).

use std::sync::OnceLock;

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
