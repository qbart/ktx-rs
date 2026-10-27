// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of the UASTC -> BC1/BC3/BC4/BC5 converters and the real-time BC1/BC4
//! encoders from basisu_transcoder.cpp (`encode_bc4`, `encode_bc1`,
//! `encode_bc1_solid_block`, `transcode_uastc_to_bc1_hint0/hint1`, ...).
//! Float math keeps the C++ evaluation order (f32).
#![allow(clippy::needless_range_loop)]

use super::dxt::{Dxt1Block, dxt_tables};
use super::etc::Color32;
use super::uastc::{
    UASTC_MODE_INDEX_SOLID_COLOR, UnpackedUastcBlock, uastc_tables, unpack_uastc,
    unpack_uastc_unpacked_pixels,
};
use super::uastc_tables::*;

pub const ENCODE_BC1_HIGH_QUALITY: u32 = 1;
pub const ENCODE_BC1_HIGHER_QUALITY: u32 = 2;
pub const ENCODE_BC1_USE_SELECTORS: u32 = 4;

const UASTC5_TO_BC1: [u8; 32] = [
    0, 0, 0, 0, 0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 1, 1, 1, 1,
];
const UASTC4_TO_BC1: [u8; 16] = [0, 0, 0, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 1, 1, 1];
const UASTC3_TO_BC1: [u8; 8] = [0, 0, 2, 2, 3, 3, 1, 1];
const UASTC2_TO_BC1: [u8; 4] = [0, 2, 3, 1];
const UASTC1_TO_BC1: [u8; 2] = [0, 1];

/// `s_uastc_to_bc1_weights[bits]`.
fn uastc_to_bc1_weights(bits: u32) -> &'static [u8] {
    match bits {
        1 => &UASTC1_TO_BC1,
        2 => &UASTC2_TO_BC1,
        3 => &UASTC3_TO_BC1,
        4 => &UASTC4_TO_BC1,
        5 => &UASTC5_TO_BC1,
        _ => &[],
    }
}

/// Port of `encode_bc4` over 16 scalar values.
pub fn encode_bc4(dst: &mut [u8; 8], p: &[u8; 16]) {
    let min_v = p.iter().copied().min().unwrap_or(0) as i32;
    let max_v = p.iter().copied().max().unwrap_or(0) as i32;
    dst[0] = max_v as u8;
    dst[1] = min_v as u8;
    if max_v == min_v {
        dst[2..].fill(0);
        return;
    }
    let delta = max_v - min_v;
    let t = [
        delta * 13,
        delta * 11,
        delta * 9,
        delta * 7,
        delta * 5,
        delta * 3,
        delta,
    ];
    let bias = 4 - min_v * 14;
    const TRAN: [u64; 8] = [1, 7, 6, 5, 4, 3, 2, 0];
    let mut f = 0u64;
    for (i, &pv) in p.iter().enumerate() {
        let v = pv as i32 * 14 + bias;
        let idx = t.iter().filter(|&&ti| v >= ti).count();
        f |= TRAN[idx] << (i * 3);
    }
    for k in 0..6 {
        dst[2 + k] = (f >> (8 * k)) as u8;
    }
}

/// `write_bc4_solid_block`.
fn write_bc4_solid_block(dst: &mut [u8; 8], a: u32) {
    dst[0] = a as u8;
    dst[1] = a as u8;
    dst[2..].fill(0);
}

/// Port of `encode_bc1_solid_block`.
pub fn encode_bc1_solid_block(dst: &mut Dxt1Block, fr: u32, fg: u32, fb: u32) {
    let t = dxt_tables();
    let (r, g, b) = (
        (fr & 255) as usize,
        (fg & 255) as usize,
        (fb & 255) as usize,
    );
    let mut mask = 0xAAu32;
    let mut max16 = ((t.bc1_match5_equals_1[r].hi as u32) << 11)
        | ((t.bc1_match6_equals_1[g].hi as u32) << 5)
        | t.bc1_match5_equals_1[b].hi as u32;
    let mut min16 = ((t.bc1_match5_equals_1[r].lo as u32) << 11)
        | ((t.bc1_match6_equals_1[g].lo as u32) << 5)
        | t.bc1_match5_equals_1[b].lo as u32;
    if min16 == max16 {
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
        core::mem::swap(&mut max16, &mut min16);
        mask ^= 0x55;
    }
    dst.set_low_color(max16 as u16);
    dst.set_high_color(min16 as u16);
    dst.bytes[4..].fill(mask as u8);
}

#[inline]
fn to_5(v: u32) -> i32 {
    let v = v * 31 + 128;
    (((v + (v >> 8)) >> 8) & 0xFF) as i32
}

#[inline]
fn to_6(v: u32) -> i32 {
    let v = v * 63 + 128;
    (((v + (v >> 8)) >> 8) & 0xFF) as i32
}

/// Port of `bc1_find_sels`.
#[allow(clippy::too_many_arguments)]
fn bc1_find_sels(
    px: &[Color32; 16],
    lr: i32,
    lg: i32,
    lb: i32,
    hr: i32,
    hg: i32,
    hb: i32,
    sels: &mut [u8; 16],
) {
    let mut br = [0i32; 4];
    let mut bg = [0i32; 4];
    let mut bb = [0i32; 4];
    br[0] = (lr << 3) | (lr >> 2);
    bg[0] = (lg << 2) | (lg >> 4);
    bb[0] = (lb << 3) | (lb >> 2);
    br[3] = (hr << 3) | (hr >> 2);
    bg[3] = (hg << 2) | (hg >> 4);
    bb[3] = (hb << 3) | (hb >> 2);
    br[1] = (br[0] * 2 + br[3]) / 3;
    bg[1] = (bg[0] * 2 + bg[3]) / 3;
    bb[1] = (bb[0] * 2 + bb[3]) / 3;
    br[2] = (br[3] * 2 + br[0]) / 3;
    bg[2] = (bg[3] * 2 + bg[0]) / 3;
    bb[2] = (bb[3] * 2 + bb[0]) / 3;
    let (mut ar, mut ag, mut ab) = (br[3] - br[0], bg[3] - bg[0], bb[3] - bb[0]);
    let dots: [i32; 4] = core::array::from_fn(|i| br[i] * ar + bg[i] * ag + bb[i] * ab);
    let (t0, t1, t2) = (dots[0] + dots[1], dots[1] + dots[2], dots[2] + dots[3]);
    ar *= 2;
    ag *= 2;
    ab *= 2;
    const S_SELS: [u8; 4] = [3, 2, 1, 0];
    for i in 0..16 {
        let d = px[i].r() as i32 * ar + px[i].g() as i32 * ag + px[i].b() as i32 * ab;
        sels[i] = S_SELS[(d <= t0) as usize + (d < t1) as usize + (d < t2) as usize];
    }
}

/// Port of `compute_least_squares_endpoints_rgb`.
fn compute_least_squares_endpoints_rgb(
    px: &[Color32; 16],
    sels: &[u8; 16],
) -> Option<([f32; 3], [f32; 3])> {
    const WEIGHT_VALS: [u32; 4] = [0x000009, 0x010204, 0x040201, 0x090000];
    let (mut uq00, mut ut) = ([0u32; 3], [0u32; 3]);
    let mut weight_accum = 0u32;
    for i in 0..16 {
        let sel = (sels[i] & 3) as u32;
        for c in 0..3 {
            ut[c] += px[i].c[c] as u32;
            uq00[c] += sel * px[i].c[c] as u32;
        }
        weight_accum += WEIGHT_VALS[sel as usize];
    }
    let q00: [f32; 3] = uq00.map(|v| v as f32);
    let t: [f32; 3] = ut.map(|v| v as f32);
    let q10: [f32; 3] = core::array::from_fn(|c| t[c] * 3.0f32 - q00[c]);
    let z00 = ((weight_accum >> 16) & 0xFF) as f32;
    let z10 = ((weight_accum >> 8) & 0xFF) as f32;
    let z11 = (weight_accum & 0xFF) as f32;
    let z01 = z10;
    let mut det = z00 * z11 - z01 * z10;
    if det.abs() < 1e-8f32 {
        return None;
    }
    det = 3.0f32 / det;
    let iz00 = z11 * det;
    let iz01 = -z01 * det;
    let iz10 = -z10 * det;
    let iz11 = z00 * det;
    let mut xl = [0f32; 3];
    let mut xh = [0f32; 3];
    for c in 0..3 {
        xl[c] = iz00 * q00[c] + iz01 * q10[c];
        xh[c] = iz10 * q00[c] + iz11 * q10[c];
    }
    for c in 0..3 {
        if xl[c] < 0.0 || xh[c] > 255.0 {
            let lo_v = px.iter().map(|p| p.c[c]).min().unwrap_or(0);
            let hi_v = px.iter().map(|p| p.c[c]).max().unwrap_or(0);
            if lo_v == hi_v {
                xl[c] = lo_v as f32;
                xh[c] = hi_v as f32;
            }
        }
    }
    Some((xl, xh))
}

/// Port of `encode_bc1` (the PCA + least squares encoder).
pub fn encode_bc1(dst: &mut Dxt1Block, px: &[Color32; 16], flags: u32) {
    let (mut avg_r, mut avg_g, mut avg_b) = (-1i32, 0i32, 0i32);
    let (mut lr, mut lg, mut lb, mut hr, mut hg, mut hb) = (0i32, 0, 0, 0, 0, 0);
    let mut sels = [0u8; 16];

    if flags & ENCODE_BC1_USE_SELECTORS != 0 {
        let s = u32::from_le_bytes([dst.bytes[4], dst.bytes[5], dst.bytes[6], dst.bytes[7]]);
        const SEL_TRAN: [u8; 4] = [0, 3, 1, 2];
        for i in 0..16 {
            sels[i] = SEL_TRAN[((s >> (i * 2)) & 3) as usize];
        }
    } else {
        let (fr, fg, fb) = (px[0].r() as i32, px[0].g() as i32, px[0].b() as i32);
        if px[1..]
            .iter()
            .all(|p| p.r() as i32 == fr && p.g() as i32 == fg && p.b() as i32 == fb)
        {
            encode_bc1_solid_block(dst, fr as u32, fg as u32, fb as u32);
            return;
        }
        let (mut tr, mut tg, mut tb) = (fr, fg, fb);
        let (mut max_r, mut max_g, mut max_b) = (fr, fg, fb);
        let (mut min_r, mut min_g, mut min_b) = (fr, fg, fb);
        for p in &px[1..] {
            let (r, g, b) = (p.r() as i32, p.g() as i32, p.b() as i32);
            max_r = max_r.max(r);
            max_g = max_g.max(g);
            max_b = max_b.max(b);
            min_r = min_r.min(r);
            min_g = min_g.min(g);
            min_b = min_b.min(b);
            tr += r;
            tg += g;
            tb += b;
        }
        avg_r = (tr + 8) >> 4;
        avg_g = (tg + 8) >> 4;
        avg_b = (tb + 8) >> 4;
        let mut icov = [0i32; 6];
        for p in px {
            let r = p.r() as i32 - avg_r;
            let g = p.g() as i32 - avg_g;
            let b = p.b() as i32 - avg_b;
            icov[0] += r * r;
            icov[1] += r * g;
            icov[2] += r * b;
            icov[3] += g * g;
            icov[4] += g * b;
            icov[5] += b * b;
        }
        let cov: [f32; 6] = icov.map(|v| v as f32 * (1.0f32 / 255.0f32));
        let mut xr = (max_r - min_r) as f32;
        let mut xg = (max_g - min_g) as f32;
        let mut xb = (max_b - min_b) as f32;
        for _ in 0..4 {
            let r = xr * cov[0] + xg * cov[1] + xb * cov[2];
            let g = xr * cov[1] + xg * cov[3] + xb * cov[4];
            let b = xr * cov[2] + xg * cov[4] + xb * cov[5];
            xr = r;
            xg = g;
            xb = b;
        }
        let fmax = |a: f32, b: f32| if a > b { a } else { b };
        let k = fmax(fmax(xr.abs(), xg.abs()), xb.abs());
        let (mut sr, mut sg, mut sb) = (306i32, 601i32, 117i32);
        if k >= 2.0 {
            let m = 1024.0f32 / k;
            sr = (xr * m) as i32;
            sg = (xg * m) as i32;
            sb = (xb * m) as i32;
        }
        let (mut low_dot, mut high_dot, mut low_c, mut high_c) =
            (i32::MAX, i32::MIN, 0usize, 0usize);
        for (i, p) in px.iter().enumerate() {
            let dot = p.r() as i32 * sr + p.g() as i32 * sg + p.b() as i32 * sb;
            if dot < low_dot {
                low_dot = dot;
                low_c = i;
            }
            if dot > high_dot {
                high_dot = dot;
                high_c = i;
            }
        }
        lr = to_5(px[low_c].r() as u32);
        lg = to_6(px[low_c].g() as u32);
        lb = to_5(px[low_c].b() as u32);
        hr = to_5(px[high_c].r() as u32);
        hg = to_6(px[high_c].g() as u32);
        hb = to_5(px[high_c].b() as u32);
        bc1_find_sels(px, lr, lg, lb, hr, hg, hb, &mut sels);
    }

    let passes = if flags & ENCODE_BC1_HIGHER_QUALITY != 0 {
        3
    } else if flags & ENCODE_BC1_HIGH_QUALITY != 0 {
        2
    } else {
        1
    };
    let t = dxt_tables();
    for _ in 0..passes {
        match compute_least_squares_endpoints_rgb(px, &sels) {
            None => {
                if avg_r < 0 {
                    let (mut tr, mut tg, mut tb) = (0i32, 0i32, 0i32);
                    for p in px {
                        tr += p.r() as i32;
                        tg += p.g() as i32;
                        tb += p.b() as i32;
                    }
                    avg_r = (tr + 8) >> 4;
                    avg_g = (tg + 8) >> 4;
                    avg_b = (tb + 8) >> 4;
                }
                let (ar, ag, ab) = (
                    (avg_r & 255) as usize,
                    (avg_g & 255) as usize,
                    (avg_b & 255) as usize,
                );
                lr = t.bc1_match5_equals_1[ar].hi as i32;
                lg = t.bc1_match6_equals_1[ag].hi as i32;
                lb = t.bc1_match5_equals_1[ab].hi as i32;
                hr = t.bc1_match5_equals_1[ar].lo as i32;
                hg = t.bc1_match6_equals_1[ag].lo as i32;
                hb = t.bc1_match5_equals_1[ab].lo as i32;
            }
            Some((xl, xh)) => {
                let q = |v: f32, s: f32, m: i32| ((v * s + 0.5f32) as i32).clamp(0, m);
                const S31: f32 = 31.0f32 / 255.0f32;
                const S63: f32 = 63.0f32 / 255.0f32;
                lr = q(xl[0], S31, 31);
                lg = q(xl[1], S63, 63);
                lb = q(xl[2], S31, 31);
                hr = q(xh[0], S31, 31);
                hg = q(xh[1], S63, 63);
                hb = q(xh[2], S31, 31);
            }
        }
        bc1_find_sels(px, lr, lg, lb, hr, hg, hb, &mut sels);
    }

    let mut lc16 = Dxt1Block::pack_unscaled_color(lr as u32, lg as u32, lb as u32) & 0xFFFF;
    let mut hc16 = Dxt1Block::pack_unscaled_color(hr as u32, hg as u32, hb as u32) & 0xFFFF;
    if lc16 == hc16 {
        let mut mask = 0u8;
        if hc16 > 0 {
            hc16 -= 1;
        } else {
            hc16 = 0;
            lc16 = 1;
            mask = 0x55;
        }
        dst.set_low_color(lc16 as u16);
        dst.set_high_color(hc16 as u16);
        dst.bytes[4..].fill(mask);
    } else {
        let mut invert_mask = 0u8;
        if lc16 < hc16 {
            core::mem::swap(&mut lc16, &mut hc16);
            invert_mask = 0x55;
        }
        dst.set_low_color(lc16 as u16);
        dst.set_high_color(hc16 as u16);
        const SEL_TRANS: [u32; 4] = [0, 2, 3, 1];
        let mut packed = 0u32;
        for i in 0..16 {
            packed |= SEL_TRANS[(sels[i] & 3) as usize] << (i * 2);
        }
        for k in 0..4 {
            dst.bytes[4 + k] = (packed >> (8 * k)) as u8 ^ invert_mask;
        }
    }
}

/// Port of `transcode_uastc_to_bc1_hint0`.
pub fn transcode_uastc_to_bc1_hint0(u: &UnpackedUastcBlock, b: &mut Dxt1Block) {
    let tb = uastc_tables();
    let mode = u.mode as usize;
    let a = &u.astc;
    let er = UASTC_MODE_ENDPOINT_RANGES.get(mode).copied().unwrap_or(0) as u32;
    let total_comps = UASTC_MODE_COMPS.get(mode).copied().unwrap_or(0);
    let uq = |i: usize| tb.unquant(er, a.endpoints[i]);
    if total_comps == 2 {
        let (l, h) = (uq(0), uq(1));
        b.set_low_color(Dxt1Block::pack_color(l, l, l, true, 127));
        b.set_high_color(Dxt1Block::pack_color(h, h, h, true, 127));
    } else {
        b.set_low_color(Dxt1Block::pack_color(uq(0), uq(2), uq(4), true, 127));
        b.set_high_color(Dxt1Block::pack_color(uq(1), uq(3), uq(5), true, 127));
    }
    if b.get_low_color() == b.get_high_color() {
        let mut lc16 = b.get_low_color();
        let mut hc16 = b.get_high_color();
        let mut mask = 0u8;
        if hc16 > 0 {
            hc16 -= 1;
        } else {
            hc16 = 0;
            lc16 = 1;
            mask = 0x55;
        }
        b.set_low_color(lc16 as u16);
        b.set_high_color(hc16 as u16);
        b.bytes[4..].fill(mask);
    } else {
        let mut invert = false;
        if b.get_low_color() < b.get_high_color() {
            b.bytes.swap(0, 2);
            b.bytes.swap(1, 3);
            invert = true;
        }
        let tran =
            uastc_to_bc1_weights(UASTC_MODE_WEIGHT_BITS.get(mode).copied().unwrap_or(0) as u32);
        let plane_shift = UASTC_MODE_PLANES
            .get(mode)
            .copied()
            .unwrap_or(1)
            .saturating_sub(1) as usize;
        let mut sels = 0u32;
        for i in (0..16usize).rev() {
            let mut s = tran
                .get(a.weights[i << plane_shift] as usize)
                .copied()
                .unwrap_or(0) as u32;
            if invert {
                s ^= 1;
            }
            sels = (sels << 2) | s;
        }
        b.bytes[4..].copy_from_slice(&sels.to_le_bytes());
    }
}

/// Port of `transcode_uastc_to_bc1_hint1`.
pub fn transcode_uastc_to_bc1_hint1(
    u: &UnpackedUastcBlock,
    px: &[Color32; 16],
    b: &mut Dxt1Block,
    high_quality: bool,
) {
    let mode = u.mode as usize;
    b.set_low_color(1);
    b.set_high_color(0);
    let tran = uastc_to_bc1_weights(UASTC_MODE_WEIGHT_BITS.get(mode).copied().unwrap_or(0) as u32);
    let plane_shift = UASTC_MODE_PLANES
        .get(mode)
        .copied()
        .unwrap_or(1)
        .saturating_sub(1) as usize;
    let mut sels = 0u32;
    for i in (0..16usize).rev() {
        sels <<= 2;
        sels |= tran
            .get(u.astc.weights[i << plane_shift] as usize)
            .copied()
            .unwrap_or(0) as u32;
    }
    b.bytes[4..].copy_from_slice(&sels.to_le_bytes());
    let hq = if high_quality {
        ENCODE_BC1_HIGH_QUALITY
    } else {
        0
    };
    encode_bc1(b, px, hq | ENCODE_BC1_USE_SELECTORS);
}

fn encode_bc1_from(
    u: &UnpackedUastcBlock,
    px: &[Color32; 16],
    b: &mut Dxt1Block,
    high_quality: bool,
) {
    if !high_quality && u.bc1_hint0 {
        transcode_uastc_to_bc1_hint0(u, b);
    } else if u.bc1_hint1 {
        transcode_uastc_to_bc1_hint1(u, px, b, high_quality);
    } else {
        encode_bc1(
            b,
            px,
            if high_quality {
                ENCODE_BC1_HIGH_QUALITY
            } else {
                0
            },
        );
    }
}

fn unpack_with_pixels(src: &[u8; 16]) -> Option<UnpackedUastcBlock> {
    let mut u = UnpackedUastcBlock::default();
    unpack_uastc(src, &mut u, false, true).then_some(u)
}

/// Port of `transcode_uastc_to_bc1`.
pub fn transcode_uastc_to_bc1(src: &[u8; 16], dst: &mut [u8; 8], high_quality: bool) -> bool {
    let Some(u) = unpack_with_pixels(src) else {
        return false;
    };
    let mut b = Dxt1Block::default();
    if u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
        let c = &u.solid_color;
        encode_bc1_solid_block(&mut b, c.r() as u32, c.g() as u32, c.b() as u32);
    } else if !high_quality && u.bc1_hint0 {
        transcode_uastc_to_bc1_hint0(&u, &mut b);
    } else {
        let mut px = [Color32::default(); 16];
        if !unpack_uastc_unpacked_pixels(&u, &mut px, false) {
            return false;
        }
        encode_bc1_from(&u, &px, &mut b, high_quality);
    }
    *dst = b.bytes;
    true
}

/// Port of `transcode_uastc_to_bc3`.
pub fn transcode_uastc_to_bc3(src: &[u8; 16], dst: &mut [u8; 16], high_quality: bool) -> bool {
    let Some(u) = unpack_with_pixels(src) else {
        return false;
    };
    let mut bc4 = [0u8; 8];
    let mut b = Dxt1Block::default();
    if u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
        let c = &u.solid_color;
        write_bc4_solid_block(&mut bc4, c.a() as u32);
        encode_bc1_solid_block(&mut b, c.r() as u32, c.g() as u32, c.b() as u32);
    } else {
        let mut px = [Color32::default(); 16];
        if !unpack_uastc_unpacked_pixels(&u, &mut px, false) {
            return false;
        }
        encode_bc4(&mut bc4, &px.map(|p| p.a()));
        encode_bc1_from(&u, &px, &mut b, high_quality);
    }
    dst[..8].copy_from_slice(&bc4);
    dst[8..].copy_from_slice(&b.bytes);
    true
}

/// Port of `transcode_uastc_to_bc4`.
pub fn transcode_uastc_to_bc4(src: &[u8; 16], dst: &mut [u8; 8], chan0: u32) -> bool {
    let Some(u) = unpack_with_pixels(src) else {
        return false;
    };
    let c0 = (chan0 & 3) as usize;
    if u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
        write_bc4_solid_block(dst, u.solid_color.c[c0] as u32);
        return true;
    }
    let mut px = [Color32::default(); 16];
    if !unpack_uastc_unpacked_pixels(&u, &mut px, false) {
        return false;
    }
    encode_bc4(dst, &px.map(|p| p.c[c0]));
    true
}

/// Port of `transcode_uastc_to_bc5`.
pub fn transcode_uastc_to_bc5(src: &[u8; 16], dst: &mut [u8; 16], chan0: u32, chan1: u32) -> bool {
    let Some(u) = unpack_with_pixels(src) else {
        return false;
    };
    let (c0, c1) = ((chan0 & 3) as usize, (chan1 & 3) as usize);
    let (mut b0, mut b1) = ([0u8; 8], [0u8; 8]);
    if u.mode == UASTC_MODE_INDEX_SOLID_COLOR {
        write_bc4_solid_block(&mut b0, u.solid_color.c[c0] as u32);
        write_bc4_solid_block(&mut b1, u.solid_color.c[c1] as u32);
    } else {
        let mut px = [Color32::default(); 16];
        if !unpack_uastc_unpacked_pixels(&u, &mut px, false) {
            return false;
        }
        encode_bc4(&mut b0, &px.map(|p| p.c[c0]));
        encode_bc4(&mut b1, &px.map(|p| p.c[c1]));
    }
    dst[..8].copy_from_slice(&b0);
    dst[8..].copy_from_slice(&b1);
    true
}
