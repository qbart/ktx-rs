// SPDX-FileCopyrightText: 2019-2020 The Khronos Group Inc.
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software dfdutils and modified; see NOTICE.md.
//! Port of external/dfdutils/colourspaces.c.

use super::khr_df::*;

/// Port of `Primaries`: CIE 1931 x,y chromaticities of R, G, B and white.
#[allow(non_snake_case)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Primaries {
    pub Rx: f32,
    pub Ry: f32,
    pub Gx: f32,
    pub Gy: f32,
    pub Bx: f32,
    pub By: f32,
    pub Wx: f32,
    pub Wy: f32,
}

const fn p(v: [f32; 8]) -> Primaries {
    Primaries {
        Rx: v[0],
        Ry: v[1],
        Gx: v[2],
        Gy: v[3],
        Bx: v[4],
        By: v[5],
        Wx: v[6],
        Wy: v[7],
    }
}

const PRIMARY_MAP: [(u32, Primaries); 11] = [
    (
        KHR_DF_PRIMARIES_BT709,
        p([0.640, 0.330, 0.300, 0.600, 0.150, 0.060, 0.3127, 0.3290]),
    ),
    (
        KHR_DF_PRIMARIES_BT601_EBU,
        p([0.640, 0.330, 0.290, 0.600, 0.150, 0.060, 0.3127, 0.3290]),
    ),
    (
        KHR_DF_PRIMARIES_BT601_SMPTE,
        p([0.630, 0.340, 0.310, 0.595, 0.155, 0.070, 0.3127, 0.3290]),
    ),
    (
        KHR_DF_PRIMARIES_BT2020,
        p([0.708, 0.292, 0.170, 0.797, 0.131, 0.046, 0.3127, 0.3290]),
    ),
    (
        KHR_DF_PRIMARIES_CIEXYZ,
        p([1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
    ),
    (
        KHR_DF_PRIMARIES_ACES,
        p([0.7347, 0.2653, 0.0, 1.0, 0.0001, -0.077, 0.32168, 0.33767]),
    ),
    (
        KHR_DF_PRIMARIES_ACESCC,
        p([0.713, 0.293, 0.165, 0.830, 0.128, 0.044, 0.32168, 0.33767]),
    ),
    (
        KHR_DF_PRIMARIES_NTSC1953,
        p([0.67, 0.33, 0.21, 0.71, 0.14, 0.08, 0.310, 0.316]),
    ),
    (
        KHR_DF_PRIMARIES_PAL525,
        p([0.630, 0.340, 0.310, 0.595, 0.155, 0.070, 0.3101, 0.3162]),
    ),
    (
        KHR_DF_PRIMARIES_DISPLAYP3,
        p([0.6800, 0.3200, 0.2650, 0.69, 0.1500, 0.0600, 0.3127, 0.3290]),
    ),
    (
        KHR_DF_PRIMARIES_ADOBERGB,
        p([0.6400, 0.3300, 0.2100, 0.71, 0.1500, 0.0600, 0.3127, 0.3290]),
    ),
];

/// Port of `findMapping`. Only the x chromaticities are compared, as in C.
pub fn find_mapping(p: &Primaries, latitude: f32) -> u32 {
    for (e, m) in PRIMARY_MAP.iter() {
        if m.Rx - p.Rx <= latitude
            && p.Rx - m.Rx <= latitude
            && m.Gx - p.Gx <= latitude
            && p.Gx - m.Gx <= latitude
            && m.Bx - p.Bx <= latitude
            && p.Bx - m.Bx <= latitude
            && m.Wx - p.Wx <= latitude
            && p.Wx - m.Wx <= latitude
        {
            return *e;
        }
    }
    KHR_DF_PRIMARIES_UNSPECIFIED
}

/// Port of `getPrimaries`.
pub fn get_primaries(primaries: u32) -> Option<Primaries> {
    PRIMARY_MAP
        .iter()
        .find(|(e, _)| *e == primaries)
        .map(|(_, p)| *p)
}
