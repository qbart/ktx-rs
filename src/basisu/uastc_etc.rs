//! Port of the UASTC -> ETC1 / ETC2 RGBA (EAC A8) converters from
//! basisu_transcoder.cpp: `apply_etc1_bias`, `etc1_determine_selectors`,
//! `transcode_uastc_to_etc1` (RGB and single channel), `pack_etc1_y_estimate_flipped`,
//! `transcode_uastc_to_etc2_eac_a8`, `transcode_uastc_to_etc2_rgba`.
#![allow(clippy::needless_range_loop)]

use super::eac::{EAC_MODIFIER_TABLE, ETC2_EAC_A8_SEL4, EacBlock};
use super::etc::{Color32, ETC1_INTEN_TABLES, SELECTOR_INDEX_TO_ETC1, clamp255};
use super::uastc::{
    UASTC_MODE_INDEX_SOLID_COLOR, UnpackedUastcBlock, unpack_uastc, unpack_uastc_unpacked_pixels,
};
use super::uastc_tables::*;

/// Port of `apply_etc1_bias`.
pub fn apply_etc1_bias(block_color: &[u8; 4], bias: u32, limit: u32, subblock: u32) -> [u8; 4] {
    let mut result = [0u8; 4];
    const DIVS: [u32; 3] = [1, 3, 9];
    for c in 0..3 {
        let sb = subblock != 0;
        let delta: i32 = match bias {
            2 => {
                if sb {
                    0
                } else if c == 0 {
                    -1
                } else {
                    0
                }
            }
            5 => {
                if sb {
                    0
                } else if c == 1 {
                    -1
                } else {
                    0
                }
            }
            6 => {
                if sb {
                    0
                } else if c == 2 {
                    -1
                } else {
                    0
                }
            }
            7 => {
                if sb {
                    0
                } else if c == 0 {
                    1
                } else {
                    0
                }
            }
            11 => {
                if sb {
                    0
                } else if c == 1 {
                    1
                } else {
                    0
                }
            }
            15 => {
                if sb {
                    0
                } else if c == 2 {
                    1
                } else {
                    0
                }
            }
            18 => {
                if !sb {
                    0
                } else if c == 0 {
                    -1
                } else {
                    0
                }
            }
            19 => {
                if !sb {
                    0
                } else if c == 1 {
                    -1
                } else {
                    0
                }
            }
            20 => {
                if !sb {
                    0
                } else if c == 2 {
                    -1
                } else {
                    0
                }
            }
            21 => {
                if !sb {
                    0
                } else if c == 0 {
                    1
                } else {
                    0
                }
            }
            24 => {
                if !sb {
                    0
                } else if c == 1 {
                    1
                } else {
                    0
                }
            }
            8 => {
                if !sb {
                    0
                } else if c == 2 {
                    1
                } else {
                    0
                }
            }
            10 => -2,
            27 => {
                if sb {
                    0
                } else {
                    -1
                }
            }
            28 => {
                if sb {
                    -1
                } else {
                    1
                }
            }
            29 => {
                if sb {
                    1
                } else {
                    0
                }
            }
            30 => {
                if sb {
                    -1
                } else {
                    0
                }
            }
            31 => {
                if sb {
                    0
                } else {
                    1
                }
            }
            _ => ((bias / DIVS[c]) % 3) as i32 - 1,
        };
        let mut v = block_color[c] as i32;
        if v == 0 {
            if delta == -2 {
                v += 3;
            } else {
                v += delta + 1;
            }
        } else if v == limit as i32 {
            v += delta - 1;
        } else {
            v += delta;
            if v < 0 || v > limit as i32 {
                v = (v - delta) - delta;
            }
        }
        result[c] = v as u8;
    }
    result
}

/// `decoder_etc_block::get_block_colors` for a raw ETC1 block.
fn etc_get_block_colors(b: &[u8; 8], subblock: u32) -> [[u8; 4]; 4] {
    let base: [i32; 3] = if b[3] & 2 != 0 {
        let c5: [i32; 3] = [(b[0] >> 3) as i32, (b[1] >> 3) as i32, (b[2] >> 3) as i32];
        let c = if subblock != 0 {
            let d = |v: u8| {
                let d = (v & 7) as i32;
                if d >= 4 { d - 8 } else { d }
            };
            [
                (c5[0] + d(b[0])).clamp(0, 31),
                (c5[1] + d(b[1])).clamp(0, 31),
                (c5[2] + d(b[2])).clamp(0, 31),
            ]
        } else {
            c5
        };
        c.map(|v| (v << 3) | (v >> 2))
    } else {
        let c4: [i32; 3] = if subblock != 0 {
            [(b[0] & 15) as i32, (b[1] & 15) as i32, (b[2] & 15) as i32]
        } else {
            [(b[0] >> 4) as i32, (b[1] >> 4) as i32, (b[2] >> 4) as i32]
        };
        c4.map(|v| (v << 4) | v)
    };
    let table = if subblock != 0 {
        (b[3] >> 2) & 7
    } else {
        b[3] >> 5
    };
    let t = &ETC1_INTEN_TABLES[table as usize];
    core::array::from_fn(|i| {
        [
            clamp255(base[0] + t[i]),
            clamp255(base[1] + t[i]),
            clamp255(base[2] + t[i]),
            255,
        ]
    })
}

#[inline]
fn store_selector_masks(dst: &mut [u8; 8], l_bitmask: u16, h_bitmask: u16) {
    dst[7] = l_bitmask as u8;
    dst[6] = (l_bitmask >> 8) as u8;
    dst[5] = h_bitmask as u8;
    dst[4] = (h_bitmask >> 8) as u8;
}

/// Port of `etc1_determine_selectors`.
fn etc1_determine_selectors(
    dst: &mut [u8; 8],
    px: &[Color32; 16],
    first_subblock: u32,
    last_subblock: u32,
) {
    const TRAN: [u32; 4] = [1, 0, 2, 3];
    let (mut l_bitmask, mut h_bitmask) = (0u16, 0u16);
    let flip = dst[3] & 1 != 0;
    for subblock in first_subblock..last_subblock {
        let bc = etc_get_block_colors(dst, subblock);
        let by: [u32; 4] = core::array::from_fn(|i| {
            bc[i][0] as u32 * 54 + bc[i][1] as u32 * 183 + bc[i][2] as u32 * 19
        });
        let (y01, y12, y23) = (by[0] + by[1], by[1] + by[2], by[2] + by[3]);
        let sel = |c: &Color32| {
            let l = c.r() as u32 * 108 + c.g() as u32 * 366 + c.b() as u32 * 38;
            TRAN[(l < y01) as usize + (l < y12) as usize + (l < y23) as usize]
        };
        if flip {
            let mut ofs = subblock * 2;
            for y in 0..2 {
                for x in 0..4 {
                    let t = sel(&px[(x + (subblock * 2 + y) * 4) as usize]);
                    l_bitmask |= ((t & 1) << ofs) as u16;
                    h_bitmask |= ((t >> 1) << ofs) as u16;
                    ofs += 4;
                }
                ofs = ofs + 1 - 16;
            }
        } else {
            let mut ofs = subblock * 2 * 4;
            for x in 0..2 {
                for y in 0..4 {
                    let t = sel(&px[(subblock * 2 + x + y * 4) as usize]);
                    l_bitmask |= ((t & 1) << ofs) as u16;
                    h_bitmask |= ((t >> 1) << ofs) as u16;
                    ofs += 1;
                }
            }
        }
    }
    store_selector_masks(dst, l_bitmask, h_bitmask);
}

/// `g_etc1_pixel_coords[flip][subblock][j]` as (x, y).
fn etc1_pixel_coord(flip: bool, subblock: u32, j: u32) -> (u32, u32) {
    if flip {
        (j & 3, subblock * 2 + (j >> 2))
    } else {
        (subblock * 2 + (j >> 2), j & 3)
    }
}

/// Port of `transcode_uastc_to_etc1(unpacked_uastc_block&, color32[4][4], void*)`.
pub fn transcode_uastc_to_etc1_unpacked(
    u: &UnpackedUastcBlock,
    px: &[Color32; 16],
    dst: &mut [u8; 8],
) {
    if u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
        dst[3] = (((u.etc1_diff as u32) << 1) | (u.etc1_inten0 << 5) | (u.etc1_inten0 << 2)) as u8;
        if u.etc1_diff {
            dst[0] = (u.etc1_r << 3) as u8;
            dst[1] = (u.etc1_g << 3) as u8;
            dst[2] = (u.etc1_b << 3) as u8;
        } else {
            dst[0] = (u.etc1_r | (u.etc1_r << 4)) as u8;
            dst[1] = (u.etc1_g | (u.etc1_g << 4)) as u8;
            dst[2] = (u.etc1_b | (u.etc1_b << 4)) as u8;
        }
        dst[4..].copy_from_slice(&ETC1_SOLID_SELECTORS[(u.etc1_selector & 3) as usize]);
        return;
    }
    let flip = u.etc1_flip;
    let diff = u.etc1_diff;
    dst[3] =
        ((flip as u32) | ((diff as u32) << 1) | (u.etc1_inten0 << 5) | (u.etc1_inten1 << 2)) as u8;
    let limit = if diff { 31 } else { 15 };
    let mut block_colors = [[0u8; 4]; 2];
    for subset in 0..2u32 {
        let mut avg = [0u32; 3];
        for j in 0..8 {
            let (x, y) = etc1_pixel_coord(flip, subset, j);
            let c = &px[(y * 4 + x) as usize];
            avg[0] += c.r() as u32;
            avg[1] += c.g() as u32;
            avg[2] += c.b() as u32;
        }
        let bc = &mut block_colors[subset as usize];
        for c in 0..3 {
            bc[c] = ((avg[c] * limit + 1020) / (8 * 255)) as u8;
        }
        bc[3] = 0;
        if UASTC_MODE_HAS_ETC1_BIAS
            .get(u.mode as usize)
            .copied()
            .unwrap_or(0)
            != 0
        {
            *bc = apply_etc1_bias(bc, u.etc1_bias, limit, subset);
        }
    }
    if diff {
        for c in 0..3 {
            let mut d = (block_colors[1][c] as i32 - block_colors[0][c] as i32).clamp(-4, 3);
            if d < 0 {
                d += 8;
            }
            dst[c] = ((block_colors[0][c] << 3) as i32 | d) as u8;
        }
    } else {
        for c in 0..3 {
            dst[c] = block_colors[1][c] | (block_colors[0][c] << 4);
        }
    }
    etc1_determine_selectors(dst, px, 0, 2);
}

/// Port of `transcode_uastc_to_etc1(const uastc_block&, void*)`.
pub fn transcode_uastc_to_etc1(src: &[u8; 16], dst: &mut [u8; 8]) -> bool {
    let mut u = UnpackedUastcBlock::default();
    if !unpack_uastc(src, &mut u, false, true) {
        return false;
    }
    let mut px = [Color32::default(); 16];
    if u.mode != UASTC_MODE_INDEX_SOLID_COLOR && !unpack_uastc_unpacked_pixels(&u, &mut px, false) {
        return false;
    }
    transcode_uastc_to_etc1_unpacked(&u, &px, dst);
    true
}

/// Port of `pack_etc1_y_estimate_flipped`: returns (flip, upper, lower, left, right).
fn pack_etc1_y_estimate_flipped(p: &[u8; 16]) -> (bool, i32, i32, i32, i32) {
    let g = |x: usize, y: usize| p[x + y * 4] as i32;
    let s00 = g(0, 0) + g(0, 1) + g(1, 0) + g(1, 1);
    let s10 = g(2, 0) + g(2, 1) + g(3, 0) + g(3, 1);
    let s01 = g(0, 2) + g(0, 3) + g(1, 2) + g(1, 3);
    let s11 = g(2, 2) + g(2, 3) + g(3, 2) + g(3, 3);
    let upper = (s00 + s10 + 4) / 8;
    let lower = (s01 + s11 + 4) / 8;
    let left = (s00 + s01 + 4) / 8;
    let right = (s10 + s11 + 4) / 8;
    let d2 = |x: usize, y: usize, a: i32| (g(x, y) - a) * (g(x, y) - a);
    let (mut ud, mut ld, mut lfd, mut rd) = (0, 0, 0, 0);
    for i in 0..4 {
        for j in 0..2 {
            ud += d2(i, j, upper);
            ld += d2(i, 2 + j, lower);
            lfd += d2(j, i, left);
            rd += d2(2 + j, i, right);
        }
    }
    (ud + ld < lfd + rd, upper, lower, left, right)
}

fn y_solid_config(range: u32, low: u32) -> u32 {
    let l = (low & 255) as usize;
    (if range == 0 {
        ETC1_Y_SOLID_BLOCK_1I_CONFIGS[l]
    } else if range < 2 {
        ETC1_Y_SOLID_BLOCK_2I_CONFIGS[l]
    } else {
        ETC1_Y_SOLID_BLOCK_4I_CONFIGS[l]
    }) as u32
}

/// Writes 8 selectors of one subblock into the ETC bitmasks (`flip` layout
/// iterates rows of 4, otherwise columns of 4).
fn put_subblock_selectors<F: Fn(u32, u32) -> u32>(
    flip: bool,
    subblock: u32,
    l_bitmask: &mut u16,
    h_bitmask: &mut u16,
    sel: F,
) {
    if flip {
        let mut ofs = subblock * 2;
        for y in 0..2 {
            for x in 0..4 {
                let t = sel(x, y);
                *l_bitmask |= ((t & 1) << ofs) as u16;
                *h_bitmask |= ((t >> 1) << ofs) as u16;
                ofs += 4;
            }
            ofs = ofs + 1 - 16;
        }
    } else {
        let mut ofs = subblock * 2 * 4;
        for x in 0..2 {
            for y in 0..4 {
                let t = sel(x, y);
                *l_bitmask |= ((t & 1) << ofs) as u16;
                *h_bitmask |= ((t >> 1) << ofs) as u16;
                ofs += 1;
            }
        }
    }
}

/// Port of `transcode_uastc_to_etc1(const uastc_block&, void*, uint32_t channel)`.
pub fn transcode_uastc_to_etc1_channel(src: &[u8; 16], dst: &mut [u8; 8], channel: u32) -> bool {
    let mut u = UnpackedUastcBlock::default();
    if !unpack_uastc(src, &mut u, false, true) {
        return false;
    }
    let ch = (channel & 3) as usize;
    if u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
        let y = u.solid_color.c[ch] as usize;
        let enc = ETC1_Y_SOLID_BLOCK_CONFIGS[y] as u32;
        let base = enc & 31;
        let sel = (enc >> 5) & 3;
        let table = enc >> 7;
        dst[3] = (2 | (table << 5) | (table << 2)) as u8;
        dst[0] = (base << 3) as u8;
        dst[1] = (base << 3) as u8;
        dst[2] = (base << 3) as u8;
        dst[4..].copy_from_slice(&ETC1_SOLID_SELECTORS[sel as usize]);
        return true;
    }
    let mut px = [Color32::default(); 16];
    if !unpack_uastc_unpacked_pixels(&u, &mut px, false) {
        return false;
    }
    let block_y: [u8; 16] = core::array::from_fn(|i| px[i].c[ch]);
    let by = |x: u32, y: u32| block_y[(y * 4 + x) as usize] as u32;
    let (flip, upper_avg, lower_avg, left_avg, right_avg) = pack_etc1_y_estimate_flipped(&block_y);

    let (mut low, mut high) = ([255u32, 255], [0u32, 0]);
    for y in 0..4u32 {
        for x in 0..4u32 {
            let s = if flip {
                (y >= 2) as usize
            } else {
                (x >= 2) as usize
            };
            let v = by(x, y);
            low[s] = low[s].min(v);
            high[s] = high[s].max(v);
        }
    }
    let range = [high[0] - low[0], high[1] - low[1]];
    dst[3] = flip as u8;

    // Selector of pixel (x, y) within subblock coordinates.
    let sub_px = |subblock: u32, x: u32, y: u32| {
        if flip {
            by(x, y + subblock * 2)
        } else {
            by(x + subblock * 2, y)
        }
    };

    if range[0] <= 3 && range[1] <= 3 {
        dst[0] = 0;
        dst[1] = 0;
        dst[2] = 0;
        let (mut lb, mut hb) = (0u16, 0u16);
        for subblock in 0..2u32 {
            let s = subblock as usize;
            let enc = y_solid_config(range[s], low[s]);
            let table = enc & 7;
            let base = (enc >> 3) & 31;
            let sels = [
                (enc >> 8) & 3,
                (enc >> 10) & 3,
                (enc >> 12) & 3,
                (enc >> 14) & 3,
            ];
            dst[3] |= (table << if subblock != 0 { 2 } else { 5 }) as u8;
            let sv = (base << if subblock != 0 { 0 } else { 4 }) as u8;
            dst[0] |= sv;
            dst[1] |= sv;
            dst[2] |= sv;
            put_subblock_selectors(flip, subblock, &mut lb, &mut hb, |x, y| {
                let t = sub_px(subblock, x, y).wrapping_sub(low[s]) & 3;
                SELECTOR_INDEX_TO_ETC1[sels[t as usize] as usize] as u32
            });
        }
        store_selector_masks(dst, lb, hb);
        return true;
    }

    let mut y0 = ((if flip { upper_avg } else { left_avg }) as u32 * 31 + 127) / 255;
    let mut y1 = ((if flip { lower_avg } else { right_avg }) as u32 * 31 + 127) / 255;
    let mut diff = true;
    let mut dy = y1 as i32 - y0 as i32;
    if !(-4..=3).contains(&dy) {
        diff = false;
        y0 = ((if flip { upper_avg } else { left_avg }) as u32 * 15 + 127) / 255;
        y1 = ((if flip { lower_avg } else { right_avg }) as u32 * 15 + 127) / 255;
        let v = (y1 | (y0 << 4)) as u8;
        dst[0] = v;
        dst[1] = v;
        dst[2] = v;
    } else {
        y1 = (y0 as i32 + dy) as u32;
        if dy < 0 {
            dy += 8;
        }
        let v = ((y0 << 3) as i32 | dy) as u8;
        dst[0] = v;
        dst[1] = v;
        dst[2] = v;
        dst[3] |= 2;
    }

    let base_y = if diff {
        [(y0 << 3) | (y0 >> 2), (y1 << 3) | (y1 >> 2)]
    } else {
        [(y0 << 4) | y0, (y1 << 4) | y1]
    };
    let enc_range: [u32; 2] = core::array::from_fn(|s| {
        let pos = (high[s] as i32 - base_y[s] as i32).abs();
        let neg = (base_y[s] as i32 - low[s] as i32).abs();
        pos.max(neg) as u32
    });

    let (mut lb, mut hb) = (0u16, 0u16);
    for subblock in 0..2u32 {
        let s = subblock as usize;
        if !diff && range[s] <= 3 {
            let enc = y_solid_config(range[s], low[s]);
            let table = enc & 7;
            let base = (enc >> 3) & 31;
            let sels = [
                (enc >> 8) & 3,
                (enc >> 10) & 3,
                (enc >> 12) & 3,
                (enc >> 14) & 3,
            ];
            dst[3] |= (table << if subblock != 0 { 2 } else { 5 }) as u8;
            let mask = !(0xFu32 << if subblock != 0 { 0 } else { 4 }) as u8;
            dst[0] &= mask;
            dst[1] &= mask;
            dst[2] &= mask;
            let sv = (base << if subblock != 0 { 0 } else { 4 }) as u8;
            dst[0] |= sv;
            dst[1] |= sv;
            dst[2] |= sv;
            put_subblock_selectors(flip, subblock, &mut lb, &mut hb, |x, y| {
                let t = sub_px(subblock, x, y).wrapping_sub(low[s]) & 3;
                SELECTOR_INDEX_TO_ETC1[sels[t as usize] as usize] as u32
            });
            continue;
        }

        let mut best_err = u32::MAX;
        let mut best_sels = [0u8; 8];
        let mut best_inten = 0u32;
        let base = base_y[s] as i32;
        let low_limit = -base;
        let high_limit = 255 - base;

        let mut mask = 0xFFu32;
        let er = enc_range[s];
        if er <= 51 {
            mask = 0xF;
            if er > 22 {
                mask &= !(1 << 0);
            }
            if !(4..=39).contains(&er) {
                mask &= !(1 << 1);
            }
            if er < 9 {
                mask &= !(1 << 2);
            }
            if er < 12 {
                mask &= !(1 << 3);
            }
        } else {
            mask &= !((1 << 0) | (1 << 1));
            if er > 60 {
                mask &= !(1 << 2);
            }
            if er > 89 {
                mask &= !(1 << 3);
            }
            if er > 120 {
                mask &= !(1 << 4);
            }
            if er > 136 {
                mask &= !(1 << 5);
            }
            if er > 174 {
                mask &= !(1 << 6);
            }
        }

        const S_TABLE: [u8; 4] = [1, 0, 2, 3];
        // Pixel iteration order: flip = 2 rows of 4, else 4 rows of 2.
        let (rows, cols) = if flip { (2u32, 4u32) } else { (4u32, 2u32) };
        for inten in 0..8usize {
            if mask & (1 << inten) == 0 {
                continue;
            }
            let it = &ETC1_INTEN_TABLES[inten];
            let t0 = low_limit.max(it[0]);
            let t1 = low_limit.max(it[1]);
            let t2 = high_limit.min(it[2]);
            let t3 = high_limit.min(it[3]);
            let tv = [t2, t3, t1, t0];
            let (th01, th12, th23) = (t0 + t1, t1 + t2, t2 + t3);
            let mut total_err = 0u32;
            let mut sels = [0u8; 8];
            let mode = if (high[s] as i32 - base) * 2 < th01 {
                sels = [3; 8];
                Some(3usize)
            } else if (low[s] as i32 - base) * 2 >= th23 {
                sels = [1; 8];
                Some(1usize)
            } else {
                None
            };
            'rows: for y in 0..rows {
                for x in 0..cols {
                    let delta = sub_px(subblock, x, y) as i32 - base;
                    let c = match mode {
                        Some(c) => c,
                        None => {
                            let d2 = delta * 2;
                            let c = S_TABLE
                                [(d2 < th01) as usize + (d2 < th12) as usize + (d2 < th23) as usize]
                                as usize;
                            sels[(y * cols + x) as usize] = c as u8;
                            c
                        }
                    };
                    let e = (tv[c] - delta).unsigned_abs();
                    total_err = total_err.wrapping_add(e * e);
                }
                if total_err >= best_err {
                    break 'rows;
                }
            }
            if total_err < best_err {
                best_err = total_err;
                best_inten = inten as u32;
                best_sels = sels;
            }
        }

        dst[3] |= (best_inten << if subblock != 0 { 2 } else { 5 }) as u8;
        put_subblock_selectors(flip, subblock, &mut lb, &mut hb, |x, y| {
            best_sels[(y * cols_of(flip) + x) as usize] as u32
        });
    }
    store_selector_masks(dst, lb, hb);
    true
}

#[inline]
fn cols_of(flip: bool) -> u32 {
    if flip { 4 } else { 2 }
}

/// Port of `transcode_uastc_to_etc2_eac_a8`.
pub fn transcode_uastc_to_etc2_eac_a8(
    u: &UnpackedUastcBlock,
    px: &[Color32; 16],
    dst: &mut EacBlock,
) {
    let has_alpha = UASTC_MODE_HAS_ALPHA
        .get(u.mode as usize)
        .copied()
        .unwrap_or(0)
        != 0;
    if !has_alpha || u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
        let a = if u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
            u.solid_color.a() as u32
        } else {
            255
        };
        dst.set_base(a);
        dst.set_table(13);
        dst.set_multiplier(1);
        dst.bytes[2..].copy_from_slice(&ETC2_EAC_A8_SEL4);
        return;
    }
    let (mut min_a, mut max_a) = (255u32, 0u32);
    for p in px {
        min_a = min_a.min(p.a() as u32);
        max_a = max_a.max(p.a() as u32);
    }
    if min_a == max_a {
        dst.set_base(min_a);
        dst.set_table(13);
        dst.set_multiplier(1);
        dst.bytes[2..].copy_from_slice(&ETC2_EAC_A8_SEL4);
        return;
    }
    let table = (u.etc2_hints & 0xF) as usize;
    let multiplier = (u.etc2_hints >> 4) as i32;
    dst.set_multiplier(multiplier as u32);
    dst.set_table(table as u32);
    let t = &EAC_MODIFIER_TABLE[table];
    let range = (t[7] as i32 - t[3] as i32) as f32;
    let s = (0 - t[3] as i32) as f32 / range;
    let (fa, fb) = (min_a as f32, max_a as f32);
    let center = (fa + (fb - fa) * s).round() as i32;
    dst.set_base(center as u32);
    let vals: [i32; 8] =
        core::array::from_fn(|j| clamp255(center + t[j] as i32 * multiplier) as i32);
    let mut sels = 0u64;
    for i in 0..16usize {
        let a = px[(i & 3) * 4 + (i >> 2)].a() as i32;
        let mut min_err = u32::MAX;
        for (j, &v) in vals.iter().enumerate() {
            let e = ((v - a).unsigned_abs() << 3) | j as u32;
            min_err = min_err.min(e);
        }
        sels |= ((min_err & 7) as u64) << (45 - i * 3);
    }
    dst.set_selector_bits(sels);
}

/// Port of `transcode_uastc_to_etc2_rgba`.
pub fn transcode_uastc_to_etc2_rgba(src: &[u8; 16], dst: &mut [u8; 16]) -> bool {
    let mut u = UnpackedUastcBlock::default();
    if !unpack_uastc(src, &mut u, false, true) {
        return false;
    }
    let mut px = [Color32::default(); 16];
    if u.mode != UASTC_MODE_INDEX_SOLID_COLOR && !unpack_uastc_unpacked_pixels(&u, &mut px, false) {
        return false;
    }
    let mut eac = EacBlock::default();
    eac.bytes.copy_from_slice(&dst[..8]);
    transcode_uastc_to_etc2_eac_a8(&u, &px, &mut eac);
    let mut etc = [0u8; 8];
    etc.copy_from_slice(&dst[8..]);
    transcode_uastc_to_etc1_unpacked(&u, &px, &mut etc);
    dst[..8].copy_from_slice(&eac.bytes);
    dst[8..].copy_from_slice(&etc);
    true
}

const ETC2_EAC_BIT_OFS: [u32; 16] = [45, 33, 21, 9, 42, 30, 18, 6, 39, 27, 15, 3, 36, 24, 12, 0];

/// `pack_eac_solid_block`.
fn pack_eac_solid_block(blk: &mut EacBlock, a: u32) {
    blk.set_base(a);
    blk.set_table(13);
    blk.set_multiplier(0);
    blk.bytes[2..].copy_from_slice(&ETC2_EAC_A8_SEL4);
}

/// Shared prologue of `pack_eac` / `pack_eac_high_quality`: returns
/// (min, max) or `None` when the block was fully written (solid or <= 5 range).
fn pack_eac_trivial(blk: &mut EacBlock, p: &[u8; 16]) -> Option<(u32, u32)> {
    let min_alpha = p.iter().copied().min().unwrap_or(0) as u32;
    let max_alpha = p.iter().copied().max().unwrap_or(0) as u32;
    if min_alpha == max_alpha {
        pack_eac_solid_block(blk, min_alpha);
        return None;
    }
    if max_alpha - min_alpha <= 5 {
        // Table 13 is lossless for ranges <= 5.
        let mut base = clamp255(max_alpha as i32 - 2) as i32;
        blk.set_base(base as u32);
        blk.set_multiplier(1);
        blk.set_table(13);
        base -= 3;
        const SELS: [u8; 6] = [2, 1, 0, 4, 5, 6];
        let mut packed = 0u64;
        for i in 0..16 {
            let sel = (p[i] as i32 - base).clamp(0, 5) as usize;
            packed |= (SELS[sel] as u64) << ETC2_EAC_BIT_OFS[i];
        }
        blk.set_selector_bits(packed);
        return None;
    }
    Some((min_alpha, max_alpha))
}

/// Base and multiplier estimate for one EAC table.
fn eac_base_mul(table: usize, min_alpha: u32, max_alpha: u32) -> (i32, i32) {
    let t = &EAC_MODIFIER_TABLE[table];
    let range = (t[7] as i32 - t[3] as i32) as f32;
    let (fa, fb) = (min_alpha as f32, max_alpha as f32);
    let s = (0 - t[3] as i32) as f32 / range;
    let base = clamp255((fa + (fb - fa) * s).round() as i32) as i32;
    let mul = (((max_alpha - min_alpha) as f32 / range).round() as i32).clamp(1, 15);
    (base, mul)
}

fn write_eac(blk: &mut EacBlock, base: i32, mul: i32, table: u32, sels: &[u8; 16]) {
    blk.set_base(base as u32);
    blk.set_multiplier(mul as u32);
    blk.set_table(table);
    let mut packed = 0u64;
    for i in 0..16 {
        packed |= (sels[i] as u64) << ETC2_EAC_BIT_OFS[i];
    }
    blk.set_selector_bits(packed);
}

/// Port of `pack_eac` (checks 4 tables).
fn pack_eac(blk: &mut EacBlock, p: &[u8; 16]) {
    let Some((min_alpha, max_alpha)) = pack_eac_trivial(blk, p) else {
        return;
    };
    const TABLES: [usize; 4] = [2, 8, 11, 13];
    let mut base = [0i32; 4];
    let mut mul = [0i32; 4];
    let mut mul_or = 0i32;
    for i in 0..4 {
        (base[i], mul[i]) = eac_base_mul(TABLES[i], min_alpha, max_alpha);
        mul_or |= mul[i];
    }
    let mut total_err = [0u32; 4];
    let mut sels = [[0u8; 16]; 4];
    for i in 0..16 {
        let a = p[i] as i32;
        let mut l = [u32::MAX; 4];
        for s in 0..8 {
            for k in 0..4 {
                let m = EAC_MODIFIER_TABLE[TABLES[k]][s] as i32;
                let d = if !(7..=255 - 7).contains(&a) {
                    clamp255(mul[k] * m + base[k]) as i32 - a
                } else if mul_or == 1 {
                    m + (base[k] - a)
                } else {
                    mul[k] * m + (base[k] - a)
                };
                l[k] = l[k].min((d.unsigned_abs() << 3) | s as u32);
            }
        }
        for k in 0..4 {
            sels[k][i] = (l[k] & 7) as u8;
            total_err[k] = total_err[k].wrapping_add((l[k] >> 3).wrapping_mul(l[k] >> 3));
        }
    }
    let mut min_err = total_err[0];
    let mut min_index = 0;
    for i in 1..4 {
        if total_err[i] < min_err {
            min_err = total_err[i];
            min_index = i;
        }
    }
    write_eac(
        blk,
        base[min_index],
        mul[min_index],
        TABLES[min_index] as u32,
        &sels[min_index],
    );
}

/// Port of `pack_eac_high_quality` (checks all 16 tables).
fn pack_eac_high_quality(blk: &mut EacBlock, p: &[u8; 16]) {
    let Some((min_alpha, max_alpha)) = pack_eac_trivial(blk, p) else {
        return;
    };
    let mut base = [0i32; 16];
    let mut mul = [0i32; 16];
    for t in 0..16 {
        (base[t], mul[t]) = eac_base_mul(t, min_alpha, max_alpha);
    }
    let mut total_err = [0u32; 16];
    let mut sels = [[0u8; 16]; 16];
    for t in 0..16 {
        let tbl = &EAC_MODIFIER_TABLE[t];
        let (m, b) = (mul[t], base[t]);
        let (mut prev_l, mut prev_a) = (0u32, u32::MAX);
        for i in 0..16 {
            let a = p[i] as i32;
            if a as u32 == prev_a {
                sels[t][i] = (prev_l & 7) as u8;
                total_err[t] = total_err[t].wrapping_add((prev_l >> 3) * (prev_l >> 3));
            } else {
                let mut l = u32::MAX;
                for s in 0..8 {
                    let v = clamp255(m * tbl[s] as i32 + b) as i32;
                    let e = ((v - a).unsigned_abs() << 3) | s as u32;
                    l = l.min(e);
                }
                sels[t][i] = (l & 7) as u8;
                total_err[t] = total_err[t].wrapping_add((l >> 3) * (l >> 3));
                prev_l = l;
                prev_a = a as u32;
            }
        }
    }
    let mut min_err = total_err[0];
    let mut min_index = 0;
    for i in 1..16 {
        if total_err[i] < min_err {
            min_err = total_err[i];
            min_index = i;
        }
    }
    write_eac(
        blk,
        base[min_index],
        mul[min_index],
        min_index as u32,
        &sels[min_index],
    );
}

fn eac_channel(
    u: &UnpackedUastcBlock,
    px: &[Color32; 16],
    blk: &mut EacBlock,
    chan: u32,
    high_quality: bool,
) {
    if chan == 3 {
        transcode_uastc_to_etc2_eac_a8(u, px, blk);
    } else {
        let vals = px.map(|p| p.c[(chan & 3) as usize]);
        if high_quality {
            pack_eac_high_quality(blk, &vals);
        } else {
            pack_eac(blk, &vals);
        }
    }
}

/// Port of `transcode_uastc_to_etc2_eac_r11`.
pub fn transcode_uastc_to_etc2_eac_r11(
    src: &[u8; 16],
    dst: &mut [u8; 8],
    high_quality: bool,
    chan0: u32,
) -> bool {
    let mut u = UnpackedUastcBlock::default();
    if !unpack_uastc(src, &mut u, false, true) {
        return false;
    }
    let mut blk = EacBlock { bytes: *dst };
    if u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
        pack_eac_solid_block(&mut blk, u.solid_color.c[(chan0 & 3) as usize] as u32);
    } else {
        let mut px = [Color32::default(); 16];
        if !unpack_uastc_unpacked_pixels(&u, &mut px, false) {
            return false;
        }
        eac_channel(&u, &px, &mut blk, chan0, high_quality);
    }
    *dst = blk.bytes;
    true
}

/// Port of `transcode_uastc_to_etc2_eac_rg11`.
pub fn transcode_uastc_to_etc2_eac_rg11(
    src: &[u8; 16],
    dst: &mut [u8; 16],
    high_quality: bool,
    chan0: u32,
    chan1: u32,
) -> bool {
    let mut u = UnpackedUastcBlock::default();
    if !unpack_uastc(src, &mut u, false, true) {
        return false;
    }
    let mut b0 = EacBlock::default();
    let mut b1 = EacBlock::default();
    b0.bytes.copy_from_slice(&dst[..8]);
    b1.bytes.copy_from_slice(&dst[8..]);
    if u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
        pack_eac_solid_block(&mut b0, u.solid_color.c[(chan0 & 3) as usize] as u32);
        pack_eac_solid_block(&mut b1, u.solid_color.c[(chan1 & 3) as usize] as u32);
    } else {
        let mut px = [Color32::default(); 16];
        if !unpack_uastc_unpacked_pixels(&u, &mut px, false) {
            return false;
        }
        eac_channel(&u, &px, &mut b0, chan0, high_quality);
        eac_channel(&u, &px, &mut b1, chan1, high_quality);
    }
    dst[..8].copy_from_slice(&b0.bytes);
    dst[8..].copy_from_slice(&b1.bytes);
    true
}
