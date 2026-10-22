//! Port of the PVRTC1 4bpp helpers from basisu_transcoder.cpp: `pvrtc4_block`
//! endpoint packing, Morton swizzling and the ETC1S modulation fix-up passes
//! (`fixup_pvrtc1_4_modulation_rgb` / `_rgba`).

use super::etc::{DecoderEtcBlock, ETC_5_TO_8, ETC1_INTEN_TABLES, Endpoint, Selector};

/// `g_pvrtc_5`.
pub const PVRTC_5: [u8; 32] = [
    0, 8, 16, 24, 33, 41, 49, 57, 66, 74, 82, 90, 99, 107, 115, 123, 132, 140, 148, 156, 165, 173,
    181, 189, 198, 206, 214, 222, 231, 239, 247, 255,
];

/// `g_pvrtc_4`.
pub const PVRTC_4: [u8; 16] = [
    0, 16, 33, 49, 66, 82, 99, 115, 140, 156, 173, 189, 206, 222, 239, 255,
];

/// `g_pvrtc_3`.
pub const PVRTC_3: [u8; 8] = [0, 33, 74, 107, 148, 181, 222, 255];

/// `g_pvrtc_alpha`.
pub const PVRTC_ALPHA: [u8; 9] = [0, 34, 68, 102, 136, 170, 204, 238, 255];

/// `g_pvrtc_5_floor`.
pub const PVRTC_5_FLOOR: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 14,
    14, 14, 14, 14, 14, 14, 14, 15, 15, 15, 15, 15, 15, 15, 15, 15, 16, 16, 16, 16, 16, 16, 16, 16,
    17, 17, 17, 17, 17, 17, 17, 17, 18, 18, 18, 18, 18, 18, 18, 18, 19, 19, 19, 19, 19, 19, 19, 19,
    19, 20, 20, 20, 20, 20, 20, 20, 20, 21, 21, 21, 21, 21, 21, 21, 21, 22, 22, 22, 22, 22, 22, 22,
    22, 23, 23, 23, 23, 23, 23, 23, 23, 23, 24, 24, 24, 24, 24, 24, 24, 24, 25, 25, 25, 25, 25, 25,
    25, 25, 26, 26, 26, 26, 26, 26, 26, 26, 27, 27, 27, 27, 27, 27, 27, 27, 27, 28, 28, 28, 28, 28,
    28, 28, 28, 29, 29, 29, 29, 29, 29, 29, 29, 30, 30, 30, 30, 30, 30, 30, 30, 31,
];

/// `g_pvrtc_5_ceil`.
pub const PVRTC_5_CEIL: [u8; 256] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8,
    8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 11,
    11, 12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14,
    14, 14, 15, 15, 15, 15, 15, 15, 15, 15, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17,
    17, 17, 17, 18, 18, 18, 18, 18, 18, 18, 18, 19, 19, 19, 19, 19, 19, 19, 19, 20, 20, 20, 20, 20,
    20, 20, 20, 20, 21, 21, 21, 21, 21, 21, 21, 21, 22, 22, 22, 22, 22, 22, 22, 22, 23, 23, 23, 23,
    23, 23, 23, 23, 24, 24, 24, 24, 24, 24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 25, 26, 26, 26,
    26, 26, 26, 26, 26, 27, 27, 27, 27, 27, 27, 27, 27, 28, 28, 28, 28, 28, 28, 28, 28, 28, 29, 29,
    29, 29, 29, 29, 29, 29, 30, 30, 30, 30, 30, 30, 30, 30, 31, 31, 31, 31, 31, 31, 31, 31,
];

/// `g_pvrtc_4_floor`.
pub const PVRTC_4_FLOOR: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9,
    9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10,
    10, 10, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 15,
];

/// `g_pvrtc_4_ceil`.
pub const PVRTC_4_CEIL: [u8; 256] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 10, 10,
    10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15,
];

/// `g_pvrtc_3_floor`.
pub const PVRTC_3_FLOOR: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7,
];

/// `g_pvrtc_3_ceil`.
pub const PVRTC_3_CEIL: [u8; 256] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
];

/// `g_pvrtc_alpha_floor`.
pub const PVRTC_ALPHA_FLOOR: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 8,
];

/// `g_pvrtc_alpha_ceil`.
pub const PVRTC_ALPHA_CEIL: [u8; 256] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
];

/// `g_pvrtc_swizzle_table[i]`: spreads the 8 bits of `i` to even positions.
#[inline]
fn swizzle8(i: u32) -> u32 {
    let mut v = 0;
    for b in 0..8 {
        v |= ((i >> b) & 1) << (2 * b);
    }
    v
}

/// `basisu::total_bits`.
#[inline]
fn total_bits(mut v: u32) -> u32 {
    let mut n = 0;
    while v != 0 {
        v >>= 1;
        n += 1;
    }
    n
}

#[inline]
fn tab(t: &[u8], i: u32) -> u32 {
    t.get(i as usize).copied().unwrap_or(0) as u32
}

#[inline]
fn store(endpoints: &mut u32, endpoint_index: u32, packed: u32) {
    if endpoint_index != 0 {
        *endpoints = (*endpoints & 0xFFFF) | (packed << 16);
    } else {
        *endpoints = (*endpoints & 0xFFFF_0000) | packed;
    }
}

/// `pvrtc4_block::set_opaque_endpoint_floor`.
pub fn set_opaque_endpoint_floor(endpoints: &mut u32, endpoint_index: u32, c: [u8; 4]) {
    let m = *endpoints & 1;
    let r = tab(&PVRTC_5_FLOOR, c[0] as u32);
    let g = tab(&PVRTC_5_FLOOR, c[1] as u32);
    let b = if endpoint_index == 0 {
        tab(&PVRTC_4_FLOOR, c[2] as u32) << 1
    } else {
        tab(&PVRTC_5_FLOOR, c[2] as u32)
    };
    let mut packed = 0x8000 | (r << 10) | (g << 5) | b;
    if endpoint_index == 0 {
        packed = (packed & !1) | m;
    }
    store(endpoints, endpoint_index, packed);
}

/// `pvrtc4_block::set_opaque_endpoint_ceil`.
pub fn set_opaque_endpoint_ceil(endpoints: &mut u32, endpoint_index: u32, c: [u8; 4]) {
    let m = *endpoints & 1;
    let r = tab(&PVRTC_5_CEIL, c[0] as u32);
    let g = tab(&PVRTC_5_CEIL, c[1] as u32);
    let b = if endpoint_index == 0 {
        tab(&PVRTC_4_CEIL, c[2] as u32) << 1
    } else {
        tab(&PVRTC_5_CEIL, c[2] as u32)
    };
    let mut packed = 0x8000 | (r << 10) | (g << 5) | b;
    if endpoint_index == 0 {
        packed |= m;
    }
    store(endpoints, endpoint_index, packed);
}

/// `pvrtc4_block::set_endpoint_raw` (opaque 554/555, translucent 3443/3444).
pub fn set_endpoint_raw(endpoints: &mut u32, endpoint_index: u32, c: [u32; 4], opaque: bool) {
    let m = *endpoints & 1;
    let [r, g, b, a] = c;
    let packed = match (opaque, endpoint_index == 0) {
        (true, true) => 0x8000 | (r << 10) | (g << 5) | (b << 1) | m,
        (true, false) => 0x8000 | (r << 10) | (g << 5) | b,
        (false, true) => (a << 12) | (r << 8) | (g << 4) | (b << 1) | m,
        (false, false) => (a << 12) | (r << 8) | (g << 4) | b,
    };
    store(endpoints, endpoint_index, packed);
}

fn set_endpoint_rounded(endpoints: &mut u32, endpoint_index: u32, c: [u8; 4], ceil: bool) {
    let (t5, t4, t3, ta) = if ceil {
        (
            &PVRTC_5_CEIL,
            &PVRTC_4_CEIL,
            &PVRTC_3_CEIL,
            &PVRTC_ALPHA_CEIL,
        )
    } else {
        (
            &PVRTC_5_FLOOR,
            &PVRTC_4_FLOOR,
            &PVRTC_3_FLOOR,
            &PVRTC_ALPHA_FLOOR,
        )
    };
    let a = tab(ta, c[3] as u32);
    let first = endpoint_index == 0;
    if a == 8 {
        let b = if first {
            tab(t4, c[2] as u32)
        } else {
            tab(t5, c[2] as u32)
        };
        let rgba = [tab(t5, c[0] as u32), tab(t5, c[1] as u32), b, a];
        set_endpoint_raw(endpoints, endpoint_index, rgba, true);
    } else {
        let b = if first {
            tab(t3, c[2] as u32)
        } else {
            tab(t4, c[2] as u32)
        };
        let rgba = [tab(t4, c[0] as u32), tab(t4, c[1] as u32), b, a];
        set_endpoint_raw(endpoints, endpoint_index, rgba, false);
    }
}

/// `pvrtc4_block::set_endpoint_floor`.
pub fn set_endpoint_floor(endpoints: &mut u32, endpoint_index: u32, c: [u8; 4]) {
    set_endpoint_rounded(endpoints, endpoint_index, c, false);
}

/// `pvrtc4_block::set_endpoint_ceil`.
pub fn set_endpoint_ceil(endpoints: &mut u32, endpoint_index: u32, c: [u8; 4]) {
    set_endpoint_rounded(endpoints, endpoint_index, c, true);
}

/// Free function `get_opaque_endpoint_l0`.
#[inline]
pub(crate) fn get_opaque_endpoint_l0(packed: u32) -> u32 {
    let r = (packed >> 10) & 31;
    let g = (packed >> 5) & 31;
    let mut b = packed & 30;
    b |= b >> 4;
    r + g + b
}

/// Free function `get_opaque_endpoint_l1`.
#[inline]
pub(crate) fn get_opaque_endpoint_l1(endpoints: u32) -> u32 {
    let packed = endpoints >> 16;
    let r = (packed >> 10) & 31;
    let g = (packed >> 5) & 31;
    let b = packed & 31;
    r + g + b
}

/// Free function `get_endpoint_8888`.
pub fn get_endpoint_8888(endpoints: u32, endpoint_index: u32) -> [u32; 4] {
    let first = endpoint_index == 0;
    let packed = if first {
        endpoints & 0xFFFE
    } else {
        (endpoints >> 16) & 0xFFFF
    };
    if packed & 0x8000 != 0 {
        let r = tab(&PVRTC_5, (packed >> 10) & 31);
        let g = tab(&PVRTC_5, (packed >> 5) & 31);
        let b5 = packed & 31;
        let b = if first {
            tab(&PVRTC_4, b5 >> 1)
        } else {
            tab(&PVRTC_5, b5)
        };
        [r, g, b, 255]
    } else {
        let r = tab(&PVRTC_4, (packed >> 8) & 0xF);
        let g = tab(&PVRTC_4, (packed >> 4) & 0xF);
        let b4 = packed & 0xF;
        let b = if first {
            tab(&PVRTC_3, b4 >> 1)
        } else {
            tab(&PVRTC_4, b4)
        };
        let a = tab(&PVRTC_ALPHA, (packed >> 12) & 7);
        [r, g, b, a]
    }
}

/// Free function `get_endpoint_l8`.
#[inline]
pub(crate) fn get_endpoint_l8(endpoints: u32, endpoint_index: u32) -> u32 {
    get_endpoint_8888(endpoints, endpoint_index).iter().sum()
}

/// Pixel (lx, ly) -> bilinear weights of the 4 surrounding endpoints, grouped by
/// quadrant like the C `DO_PIX` lists.
const PIX_WEIGHTS: [(u32, u32, [i32; 4]); 16] = [
    (0, 0, [4, 4, 4, 4]),
    (1, 0, [2, 6, 2, 6]),
    (0, 1, [2, 2, 6, 6]),
    (1, 1, [1, 3, 3, 9]),
    (2, 0, [8, 0, 8, 0]),
    (3, 0, [6, 2, 6, 2]),
    (2, 1, [4, 0, 12, 0]),
    (3, 1, [3, 1, 9, 3]),
    (0, 2, [8, 8, 0, 0]),
    (1, 2, [4, 12, 0, 0]),
    (0, 3, [6, 6, 2, 2]),
    (1, 3, [3, 9, 1, 3]),
    (2, 2, [16, 0, 0, 0]),
    (3, 2, [12, 4, 0, 0]),
    (2, 3, [12, 0, 4, 0]),
    (3, 3, [9, 3, 3, 1]),
];

/// Shared body of the two fix-up passes. `lum(e, idx)` is the endpoint
/// luminance and `pixel(block_index, lx, ly)` the source pixel's luminance on the
/// same scale (x16).
pub(crate) fn fixup_pvrtc1_4_modulation<L, P>(
    pvrtc_endpoints: &[u32],
    dst: &mut [u8],
    num_blocks_x: u32,
    num_blocks_y: u32,
    lum: L,
    mut pixel: P,
) -> bool
where
    L: Fn(u32, u32) -> i32,
    P: FnMut(usize, u32, u32) -> Option<i32>,
{
    let x_mask = num_blocks_x.wrapping_sub(1);
    let y_mask = num_blocks_y.wrapping_sub(1);
    let min_bits = total_bits(x_mask).min(total_bits(y_mask));
    let swizzle_mask = 1u32.wrapping_shl(min_bits * 2).wrapping_sub(1);
    let ep = |i: u32| pvrtc_endpoints.get(i as usize).copied().unwrap_or(0);

    let mut block_index = 0usize;
    // really 3x3, indexed [ex][ey]
    let mut e0 = [[0i32; 3]; 3];
    let mut e1 = [[0i32; 3]; 3];

    for y in 0..num_blocks_y {
        let mut rows = [0u32; 3];
        for ey in 0..3u32 {
            let by = (y as i32 + ey as i32 - 1) as u32 & y_mask;
            rows[ey as usize] = by * num_blocks_x;
            for ex in 0..3u32 {
                let bx = (ex as i32 - 1) as u32 & x_mask;
                let e = ep(rows[ey as usize] + bx);
                e0[ex as usize][ey as usize] = lum(e, 0);
                e1[ex as usize][ey as usize] = lum(e, 1);
            }
        }
        let y_swizzle = (swizzle8(y >> 8) << 16) | swizzle8(y & 0xFF);

        for x in 0..num_blocks_x {
            let x_swizzle = (swizzle8(x >> 8) << 17) | (swizzle8(x & 0xFF) << 1);
            let mut swizzled = x_swizzle | y_swizzle;
            if num_blocks_x != num_blocks_y {
                swizzled &= swizzle_mask;
                if num_blocks_x > num_blocks_y {
                    swizzled |= (x >> min_bits) << (min_bits * 2);
                } else {
                    swizzled |= (y >> min_bits) << (min_bits * 2);
                }
            }

            let bx = (x + 1) & x_mask;
            for ey in 0..3 {
                let e = ep(rows[ey] + bx);
                e0[2][ey] = lum(e, 0);
                e1[2][ey] = lum(e, 1);
            }

            let mut modulation = 0u32;
            for &(lx, ly, w) in &PIX_WEIGHTS {
                let (ex, ey) = ((lx >> 1) as usize, (ly >> 1) as usize);
                let a = [
                    e0[ex][ey],
                    e0[ex + 1][ey],
                    e0[ex][ey + 1],
                    e0[ex + 1][ey + 1],
                ];
                let b = [
                    e1[ex][ey],
                    e1[ex + 1][ey],
                    e1[ex][ey + 1],
                    e1[ex + 1][ey + 1],
                ];
                let ca_l = a[0] * w[0] + a[1] * w[1] + a[2] * w[2] + a[3] * w[3];
                let cb_l = b[0] * w[0] + b[1] * w[1] + b[2] * w[2] + b[3] * w[3];
                let Some(cl) = pixel(block_index, lx, ly) else {
                    return false;
                };
                let mut dl = cb_l - ca_l;
                let vl = cl - ca_l;
                let mut p = vl * 16;
                if ca_l > cb_l {
                    p = -p;
                    dl = -dl;
                }
                let shift = ly * 8 + lx * 2;
                let mut m = 0;
                if p > 3 * dl {
                    m = 1 << shift;
                }
                if p > 8 * dl {
                    m = 2 << shift;
                }
                if p > 13 * dl {
                    m = 3 << shift;
                }
                modulation |= m;
            }

            let ofs = swizzled as usize * 8;
            let Some(out) = dst.get_mut(ofs..ofs + 8) else {
                return false;
            };
            out[..4].copy_from_slice(&modulation.to_le_bytes());
            out[4..].copy_from_slice(&ep(block_index as u32).to_le_bytes());

            for ey in 0..3 {
                e0[0][ey] = e0[1][ey];
                e0[1][ey] = e0[2][ey];
                e1[0][ey] = e1[1][ey];
                e1[1][ey] = e1[2][ey];
            }
            block_index += 1;
        }
    }
    true
}

#[inline]
fn etc_base_y_x16(b: &DecoderEtcBlock) -> (i32, &'static [i32; 4]) {
    let r = ETC_5_TO_8[(b.bytes[0] >> 3) as usize] as i32;
    let g = ETC_5_TO_8[(b.bytes[1] >> 3) as usize] as i32;
    let bl = ETC_5_TO_8[(b.bytes[2] >> 3) as usize] as i32;
    (
        (r + g + bl) * 16,
        &ETC1_INTEN_TABLES[(b.bytes[3] >> 5) as usize],
    )
}

/// Port of `fixup_pvrtc1_4_modulation_rgb`. `etc_blocks` hold ETC1 blocks with
/// packed ETC selectors.
pub fn fixup_pvrtc1_4_modulation_rgb(
    etc_blocks: &[DecoderEtcBlock],
    pvrtc_endpoints: &[u32],
    dst: &mut [u8],
    num_blocks_x: u32,
    num_blocks_y: u32,
) -> bool {
    let lum = |e: u32, idx: u32| {
        let l = if idx == 0 {
            get_opaque_endpoint_l0(e)
        } else {
            get_opaque_endpoint_l1(e)
        };
        (l * 255 / 31) as i32
    };
    let mut cache: Option<(usize, [i32; 4], [u32; 4])> = None;
    let pixel = |bi: usize, lx: u32, ly: u32| -> Option<i32> {
        if cache.map(|c| c.0) != Some(bi) {
            let src = etc_blocks.get(bi)?;
            let (by, t) = etc_base_y_x16(src);
            // ETC selector order: +small, +large, -small, -large.
            let colors = [
                by + t[2] * 48,
                by + t[3] * 48,
                by + t[1] * 48,
                by + t[0] * 48,
            ];
            let lookup: [u32; 4] = core::array::from_fn(|lx| {
                let byte_ofs = 7 - ((lx * 4) >> 3);
                let sh = (lx & 1) * 4;
                let lsb = (src.bytes[byte_ofs] >> sh) as u32;
                let msb = (src.bytes[byte_ofs - 2] >> sh) as u32;
                (lsb & 0xF) | ((msb & 0xF) << 4)
            });
            cache = Some((bi, colors, lookup));
        }
        let (_, colors, lookup) = cache?;
        let v = lookup[lx as usize];
        // g_etc1_x_selector_unpack[ly][v]
        let s = ((v >> ly) & 1) | (((v >> (4 + ly)) & 1) << 1);
        Some(colors[s as usize])
    };
    fixup_pvrtc1_4_modulation(pvrtc_endpoints, dst, num_blocks_x, num_blocks_y, lum, pixel)
}

/// Per-block (index, color x16, alpha x16, color selectors, alpha selectors).
type RgbaBlockCache = (usize, [i32; 4], [i32; 4], [u8; 4], [u8; 4]);

/// Port of `fixup_pvrtc1_4_modulation_rgba`. `etc_blocks` hold linear selectors;
/// `alpha_blocks` holds per-block (endpoint, selector) u16 index pairs.
#[allow(clippy::too_many_arguments)]
pub fn fixup_pvrtc1_4_modulation_rgba(
    etc_blocks: &[DecoderEtcBlock],
    pvrtc_endpoints: &[u32],
    dst: &mut [u8],
    num_blocks_x: u32,
    num_blocks_y: u32,
    alpha_blocks: &[u8],
    endpoints: &[Endpoint],
    selectors: &[Selector],
) -> bool {
    let lum = |e: u32, idx: u32| get_endpoint_l8(e, idx) as i32;
    let mut cache: Option<RgbaBlockCache> = None;
    let pixel = |bi: usize, lx: u32, ly: u32| -> Option<i32> {
        if cache.map(|c| c.0) != Some(bi) {
            let src = etc_blocks.get(bi)?;
            let ai = alpha_blocks.get(bi * 4..bi * 4 + 4)?;
            let aep = endpoints.get(u16::from_le_bytes([ai[0], ai[1]]) as usize)?;
            let asel = selectors.get(u16::from_le_bytes([ai[2], ai[3]]) as usize)?;
            let (by, t) = etc_base_y_x16(src);
            let colors: [i32; 4] = core::array::from_fn(|i| (by + t[i] * 48).clamp(0, 48 * 255));
            let abase = ETC_5_TO_8[(aep.color5.g() & 31) as usize] as i32 * 16;
            let at = &ETC1_INTEN_TABLES[(aep.inten5 & 7) as usize];
            let acolors: [i32; 4] =
                core::array::from_fn(|i| (abase + at[i] * 16).clamp(0, 16 * 255));
            let mut sels = [0u8; 4];
            sels.copy_from_slice(&src.bytes[4..8]);
            cache = Some((bi, colors, acolors, sels, asel.selectors));
        }
        let (_, colors, acolors, sels, asels) = cache?;
        let sh = lx * 2;
        let c = colors[((sels[ly as usize] >> sh) & 3) as usize];
        let a = acolors[((asels[ly as usize] >> sh) & 3) as usize];
        Some(c + a)
    };
    fixup_pvrtc1_4_modulation(pvrtc_endpoints, dst, num_blocks_x, num_blocks_y, lum, pixel)
}
