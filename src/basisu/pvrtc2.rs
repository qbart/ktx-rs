// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of the ETC1S -> PVRTC2 4bpp converters from basisu_transcoder.cpp
//! (`convert_etc1s_to_pvrtc2_rgb/rgba`, `transcoder_init_pvrtc2`).
//!
//! Float math follows the C++ evaluation order (f32, no fused ops).

use std::sync::OnceLock;

use super::atc::{
    ATC_IDENTITY_SELECTOR_MAPPING_INDEX, ETC1S_TO_ATC_SELECTOR_MAPPINGS,
    NUM_ETC1S_TO_ATC_SELECTOR_MAPPINGS, atc_tables, table_base, xlat_selectors,
};
use super::etc::{Color32, DecoderEtcBlock, ETC1_INTEN_TABLES, Endpoint, Selector, clamp255};
use super::tables::{ETC1S_TO_ATC_55, ETC1S_TO_PVRTC2_45};

/// `pvrtc2_block`: 4 modulation bytes + 32 bits of color data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pvrtc2Block {
    pub modulation: [u8; 4],
    pub color_data_bits: u32,
}

impl Pvrtc2Block {
    pub fn to_bytes(self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&self.modulation);
        b[4..].copy_from_slice(&self.color_data_bits.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; 8]) -> Self {
        Self {
            modulation: [b[0], b[1], b[2], b[3]],
            color_data_bits: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        }
    }

    #[inline]
    fn set_bits(&mut self, ofs: u32, len: u32, v: u32) {
        let mask = ((1u32 << len) - 1) << ofs;
        self.color_data_bits = (self.color_data_bits & !mask) | ((v << ofs) & mask);
    }

    fn set_flags(&mut self, hard: bool, modf: bool, opaque: bool) {
        self.set_bits(15, 1, hard as u32);
        self.set_bits(0, 1, modf as u32);
        self.set_bits(31, 1, opaque as u32);
    }

    /// 554
    pub fn set_low_color(&mut self, r: u32, g: u32, b: u32) {
        self.set_bits(10, 5, r);
        self.set_bits(5, 5, g);
        self.set_bits(1, 4, b);
    }

    /// 555
    pub fn set_high_color(&mut self, r: u32, g: u32, b: u32) {
        self.set_bits(26, 5, r);
        self.set_bits(21, 5, g);
        self.set_bits(16, 5, b);
    }

    /// 4433
    pub fn set_trans_low_color(&mut self, r: u32, g: u32, b: u32, a: u32) {
        self.set_bits(8, 4, r);
        self.set_bits(4, 4, g);
        self.set_bits(1, 3, b);
        self.set_bits(12, 3, a);
    }

    /// 4443
    pub fn set_trans_high_color(&mut self, r: u32, g: u32, b: u32, a: u32) {
        self.set_bits(24, 4, r);
        self.set_bits(20, 4, g);
        self.set_bits(16, 4, b);
        self.set_bits(28, 3, a);
    }
}

/// `{m_l, m_h}` match tables built by `transcoder_init_pvrtc2`.
struct Pvrtc2Tables {
    trans_match34: [(u8, u8); 256],
    trans_match44: [(u8, u8); 256],
    alpha_match33: [(u8, u8); 256],
    alpha_match33_0: [(u8, u8); 256],
    alpha_match33_3: [(u8, u8); 256],
}

fn match_table<FL, FH>(nl: u32, nh: u32, le_of: FL, he_of: FH, mode: u8) -> [(u8, u8); 256]
where
    FL: Fn(u32) -> u32,
    FH: Fn(u32) -> u32,
{
    let mut t = [(0u8, 0u8); 256];
    for (v, slot) in t.iter_mut().enumerate() {
        let v = v as i32;
        let mut lowest_err = i32::MAX;
        for l in 0..nl {
            for h in 0..nh {
                let m = match mode {
                    0 => le_of(l),
                    3 => he_of(h),
                    _ => (le_of(l) * 5 + he_of(h) * 3) / 8,
                };
                let err = (v - m as i32).abs();
                if err < lowest_err {
                    lowest_err = err;
                    *slot = match mode {
                        0 => (l as u8, l as u8),
                        3 => (h as u8, h as u8),
                        _ => (l as u8, h as u8),
                    };
                }
            }
        }
    }
    t
}

fn pvrtc2_tables() -> &'static Pvrtc2Tables {
    static T: OnceLock<Pvrtc2Tables> = OnceLock::new();
    T.get_or_init(|| {
        let a_lo = |l: u32| {
            let le = l << 1;
            (le << 4) | le
        };
        let a_hi = |h: u32| {
            let he = (h << 1) | 1;
            (he << 4) | he
        };
        let c3 = |l: u32| {
            let le = (l << 2) | (l >> 1);
            (le << 3) | (le >> 2)
        };
        let c4 = |h: u32| {
            let he = (h << 1) | (h >> 3);
            (he << 3) | (he >> 2)
        };
        Pvrtc2Tables {
            alpha_match33: match_table(8, 8, a_lo, a_hi, 1),
            alpha_match33_0: match_table(8, 1, a_lo, a_hi, 0),
            alpha_match33_3: match_table(1, 8, a_lo, a_hi, 3),
            trans_match34: match_table(8, 16, c3, c4, 1),
            trans_match44: match_table(16, 16, c4, c4, 1),
        }
    })
}

#[inline]
fn sol(t: &[(u8, u8, u16)], idx: usize) -> (u8, u8, u16) {
    t.get(idx).copied().unwrap_or((0, 0, 0))
}

/// Port of `convert_etc1s_to_pvrtc2_rgb`.
pub fn convert_etc1s_to_pvrtc2_rgb(blk: &mut Pvrtc2Block, ep: &Endpoint, sel: &Selector) {
    let at = atc_tables();
    blk.set_flags(true, false, true);

    let low_selector = sel.lo_selector as u32;
    let high_selector = sel.hi_selector as u32;
    let base_color = &ep.color5;
    let inten_table = ep.inten5 as u32;

    if low_selector == high_selector {
        let (r, g, b) = DecoderEtcBlock::get_block_color5(base_color, inten_table, low_selector);
        let (r, g, b) = (r as usize, g as usize, b as usize);
        let m55 = &at.atc_match55_equals_1;
        let m45 = &at.pvrtc2_match45_equals_1;
        blk.set_low_color(m55[r].0 as u32, m55[g].0 as u32, m45[b].0 as u32);
        blk.set_high_color(m55[r].1 as u32, m55[g].1 as u32, m45[b].1 as u32);
        blk.modulation = [0x55; 4];
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
            at.pvrtc2_match4[c0.b() as usize].1 as u32,
        );
        blk.set_high_color(
            m5[c1.r() as usize].1 as u32,
            m5[c1.g() as usize].1 as u32,
            m5[c1.b() as usize].1 as u32,
        );
        blk.modulation = sel.selectors;
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
            + sol(&ETC1S_TO_ATC_55, bg + m).2 as u32
            + sol(&ETC1S_TO_PVRTC2_45, bb + m).2 as u32;
        if total < best_err {
            best_err = total;
            best_mapping = m;
        }
    }
    let r = sol(&ETC1S_TO_ATC_55, br + best_mapping);
    let g = sol(&ETC1S_TO_ATC_55, bg + best_mapping);
    let b = sol(&ETC1S_TO_PVRTC2_45, bb + best_mapping);
    blk.set_low_color(r.0 as u32, g.0 as u32, b.0 as u32);
    blk.set_high_color(r.1 as u32, g.1 as u32, b.1 as u32);

    blk.modulation = if best_mapping == ATC_IDENTITY_SELECTOR_MAPPING_INDEX {
        sel.selectors
    } else {
        xlat_selectors(
            &sel.selectors,
            &ETC1S_TO_ATC_SELECTOR_MAPPINGS[best_mapping],
        )
    };
}

type Vec4 = [f32; 4];

#[inline]
fn v_dot(a: &Vec4, b: &Vec4) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

#[inline]
fn v_mul(a: &Vec4, s: f32) -> Vec4 {
    [a[0] * s, a[1] * s, a[2] * s, a[3] * s]
}

#[inline]
fn v_add(a: &Vec4, b: &Vec4) -> Vec4 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

#[inline]
fn v_sub(a: &Vec4, b: &Vec4) -> Vec4 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}

#[inline]
fn saturate(v: f32) -> f32 {
    // Same as C clampf for all inputs, NaN included.
    v.clamp(0.0, 1.0)
}

fn v_normalize(v: &mut Vec4) {
    let mut s = v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3];
    if s != 0.0 {
        s = 1.0 / s.sqrt();
        for c in v.iter_mut() {
            *c *= s;
        }
    }
}

#[inline]
fn v_from(c: &Color32) -> Vec4 {
    [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32]
}

/// `convert_rgba_5554_to_8888`.
fn convert_rgba_5554_to_8888(c: [u32; 4]) -> [i32; 4] {
    let e5 = |v: u32| (((v << 3) | (v >> 2)) & 0xFF) as i32;
    [
        e5(c[0]),
        e5(c[1]),
        e5(c[2]),
        (((c[3] << 4) | c[3]) & 0xFF) as i32,
    ]
}

#[inline]
fn sq(x: i32) -> u32 {
    x.wrapping_mul(x) as u32
}

/// Port of `convert_etc1s_to_pvrtc2_rgba`. `alpha` is the alpha slice's
/// endpoint/selector (C reads their indices from the destination block).
pub fn convert_etc1s_to_pvrtc2_rgba(
    blk: &mut Pvrtc2Block,
    ep: &Endpoint,
    sel: &Selector,
    alpha_endpoint: &Endpoint,
    alpha_selectors: &Selector,
) {
    let pt = pvrtc2_tables();
    blk.set_flags(true, false, false);

    let num_unique_alpha_selectors = alpha_selectors.num_unique_selectors;
    let alpha_block_colors =
        DecoderEtcBlock::get_block_colors5_g(&alpha_endpoint.color5, alpha_endpoint.inten5 as u32);
    let alo = alpha_selectors.lo_selector as usize & 3;
    let ahi = alpha_selectors.hi_selector as usize & 3;

    let mut constant_alpha_val = alpha_block_colors[alo];
    if num_unique_alpha_selectors != 1 {
        for &c in &alpha_block_colors[alo + 1..=ahi.max(alo)] {
            if constant_alpha_val != c {
                constant_alpha_val = -1;
                break;
            }
        }
    }

    if constant_alpha_val >= 250 {
        convert_etc1s_to_pvrtc2_rgb(blk, ep, sel);
        return;
    }

    let base_color = &ep.color5;
    let inten_table = ep.inten5 as u32;
    let low_selector = sel.lo_selector as usize & 3;
    let high_selector = sel.hi_selector as usize & 3;
    let num_unique_color_selectors = sel.num_unique_selectors;

    let e5 = |v: u8| ((v as i32) << 3) | ((v as i32) >> 2);
    let (br, bg, bb) = (e5(base_color.r()), e5(base_color.g()), e5(base_color.b()));
    let it = &ETC1_INTEN_TABLES[(inten_table & 7) as usize];
    let block_cols: [Color32; 4] = core::array::from_fn(|i| {
        let ci = it[i];
        Color32 {
            c: [
                clamp255(br + ci),
                clamp255(bg + ci),
                clamp255(bb + ci),
                clamp255(alpha_block_colors[i]),
            ],
        }
    });

    let mut solid_color_block = true;
    if num_unique_color_selectors > 1 {
        let lc = &block_cols[low_selector];
        for c in &block_cols[low_selector + 1..=high_selector.max(low_selector)] {
            if lc.r() != c.r() || lc.g() != c.g() || lc.b() != c.b() {
                solid_color_block = false;
                break;
            }
        }
    }

    if solid_color_block && constant_alpha_val >= 0 {
        let (r, g, b) =
            DecoderEtcBlock::get_block_color5(base_color, inten_table, low_selector as u32);
        let cav = constant_alpha_val as usize & 255;
        let (ri, gi, bi, ai) = (r as i32, g as i32, b as i32, constant_alpha_val);

        // Mod 0
        let (lr0, lg0, lb0) = (
            (r * 15 + 128) / 255,
            (g * 15 + 128) / 255,
            (b * 7 + 128) / 255,
        );
        let la0 = pt.alpha_match33_0[cav].0 as u32;
        let mut cr0 = (lr0 << 1) | (lr0 >> 3);
        let mut cg0 = (lg0 << 1) | (lg0 >> 3);
        let mut cb0 = (lb0 << 2) | (lb0 >> 1);
        let mut ca0 = la0 << 1;
        cr0 = (cr0 << 3) | (cr0 >> 2);
        cg0 = (cg0 << 3) | (cg0 >> 2);
        cb0 = (cb0 << 3) | (cb0 >> 2);
        ca0 = (ca0 << 4) | ca0;
        let err0 = sq(cr0 as i32 - ri)
            .wrapping_add(sq(cg0 as i32 - gi))
            .wrapping_add(sq(cb0 as i32 - bi))
            .wrapping_add(sq(ca0 as i32 - ai).wrapping_mul(2));

        if err0 == 0 || constant_alpha_val < 3 {
            blk.set_trans_low_color(lr0, lg0, lb0, la0);
            blk.set_trans_high_color(0, 0, 0, 0);
            blk.modulation = [0; 4];
            return;
        }

        // Mod 3
        let (lr3, lg3, lb3) = (
            (r * 15 + 128) / 255,
            (g * 15 + 128) / 255,
            (b * 15 + 128) / 255,
        );
        let la3 = pt.alpha_match33_3[cav].0 as u32;
        let mut cr3 = (lr3 << 1) | (lr3 >> 3);
        let mut cg3 = (lg3 << 1) | (lg3 >> 3);
        let mut cb3 = (lb3 << 1) | (lb3 >> 3);
        let mut ca3 = (la3 << 1) | 1;
        cr3 = (cr3 << 3) | (cr3 >> 2);
        cg3 = (cg3 << 3) | (cg3 >> 2);
        cb3 = (cb3 << 3) | (cb3 >> 2);
        ca3 = (ca3 << 4) | ca3;
        let err3 = sq(cr3 as i32 - ri)
            .wrapping_add(sq(cg3 as i32 - gi))
            .wrapping_add(sq(cb3 as i32 - bi))
            .wrapping_add(sq(ca3 as i32 - ai).wrapping_mul(2));

        // Mod 1
        let m44 = &pt.trans_match44;
        let m34 = &pt.trans_match34;
        let (lr1, lg1, lb1) = (
            m44[r as usize & 255].0 as u32,
            m44[g as usize & 255].0 as u32,
            m34[b as usize & 255].0 as u32,
        );
        let (hr1, hg1, hb1) = (
            m44[r as usize & 255].1 as u32,
            m44[g as usize & 255].1 as u32,
            m34[b as usize & 255].1 as u32,
        );
        let (la1, ha1) = (
            pt.alpha_match33[cav].0 as u32,
            pt.alpha_match33[cav].1 as u32,
        );

        let x5 = |v: u32| (v << 3) | (v >> 2);
        let clr1 = x5((lr1 << 1) | (lr1 >> 3));
        let clg1 = x5((lg1 << 1) | (lg1 >> 3));
        let clb1 = x5((lb1 << 2) | (lb1 >> 1));
        let cla1 = {
            let v = la1 << 1;
            (v << 4) | v
        };
        let chr1 = x5((hr1 << 1) | (hr1 >> 3));
        let chg1 = x5((hg1 << 1) | (hg1 >> 3));
        let chb1 = x5((hb1 << 1) | (hb1 >> 3));
        let cha1 = {
            let v = (ha1 << 1) | 1;
            (v << 4) | v
        };
        let r1 = (clr1 * 5 + chr1 * 3) / 8;
        let g1 = (clg1 * 5 + chg1 * 3) / 8;
        let b1 = (clb1 * 5 + chb1 * 3) / 8;
        let a1 = (cla1 * 5 + cha1 * 3) / 8;
        let err1 = sq(r1 as i32 - ri)
            .wrapping_add(sq(g1 as i32 - gi))
            .wrapping_add(sq(b1 as i32 - bi))
            .wrapping_add(sq(a1 as i32 - ai).wrapping_mul(2));

        if err1 < err0 && err1 < err3 {
            blk.set_trans_low_color(lr1, lg1, lb1, la1);
            blk.set_trans_high_color(hr1, hg1, hb1, ha1);
            blk.modulation = [0x55; 4];
        } else if err0 < err3 {
            blk.set_trans_low_color(lr0, lg0, lb0, la0);
            blk.set_trans_high_color(0, 0, 0, 0);
            blk.modulation = [0; 4];
        } else {
            blk.set_trans_low_color(0, 0, 0, 0);
            blk.set_trans_high_color(lr3, lg3, lb3, la3);
            blk.modulation = [0xFF; 4];
        }
        return;
    }

    // Complex block with non-solid color and/or alpha pixels.
    const S: f32 = 1.0 / 255.0;
    let lc = &block_cols[low_selector];
    let hc = &block_cols[high_selector];
    let (mut min_color, mut max_color): (Vec4, Vec4);

    if solid_color_block {
        let low_a = block_cols[alo].a() as f32;
        let high_a = block_cols[ahi].a() as f32;
        min_color = [
            lc.r() as f32 * S,
            lc.g() as f32 * S,
            lc.b() as f32 * S,
            low_a * S,
        ];
        max_color = [
            lc.r() as f32 * S,
            lc.g() as f32 * S,
            lc.b() as f32 * S,
            high_a * S,
        ];
    } else if constant_alpha_val >= 0 {
        let a = constant_alpha_val as f32 * S;
        min_color = [lc.r() as f32 * S, lc.g() as f32 * S, lc.b() as f32 * S, a];
        max_color = [hc.r() as f32 * S, hc.g() as f32 * S, hc.b() as f32 * S, a];
    } else if lc.r() == 0
        || hc.r() == 255
        || lc.g() == 0
        || hc.g() == 255
        || lc.b() == 0
        || hc.b() == 255
        || block_cols[alo].a() == 0
        || block_cols[ahi].a() == 255
    {
        // Full 4D PCA.
        let mut pixels = [Color32::default(); 16];
        let (mut sum_r, mut sum_g, mut sum_b, mut sum_a) = (0u32, 0u32, 0u32, 0u32);
        for (i, px) in pixels.iter_mut().enumerate() {
            let (x, y) = ((i & 3) as u32, (i >> 2) as u32);
            let rgb = &block_cols[(sel.get_selector(x, y) & 3) as usize];
            let a = block_cols[(alpha_selectors.get_selector(x, y) & 3) as usize].a();
            *px = Color32 {
                c: [rgb.r(), rgb.g(), rgb.b(), a],
            };
            sum_r += rgb.r() as u32;
            sum_g += rgb.g() as u32;
            sum_b += rgb.b() as u32;
            sum_a += a as u32;
        }
        let mut mean_color: Vec4 = [sum_r as f32, sum_g as f32, sum_b as f32, sum_a as f32];
        let mean_color_scaled = v_mul(&mean_color, 1.0 / 16.0);
        mean_color = v_mul(&mean_color, 1.0 / (16.0f32 * 255.0f32));
        for c in mean_color.iter_mut() {
            *c = saturate(*c);
        }

        let mut axis: Vec4 = [0.0; 4];
        for (i, px) in pixels.iter().enumerate() {
            let color = v_sub(&v_from(px), &mean_color_scaled);
            let a = v_mul(&color, color[0]);
            let b = v_mul(&color, color[1]);
            let c = v_mul(&color, color[2]);
            let d = v_mul(&color, color[3]);
            let mut n = if i != 0 { axis } else { color };
            v_normalize(&mut n);
            axis[0] += v_dot(&a, &n);
            axis[1] += v_dot(&b, &n);
            axis[2] += v_dot(&c, &n);
            axis[3] += v_dot(&d, &n);
        }
        v_normalize(&mut axis);
        if v_dot(&axis, &axis) < 0.5 {
            axis = [0.5; 4];
        }

        let (mut l, mut h) = (1e9f32, -1e9f32);
        for px in &pixels {
            let q = v_sub(&v_from(px), &mean_color_scaled);
            let d = v_dot(&q, &axis);
            l = if l < d { l } else { d };
            h = if h > d { h } else { d };
        }
        l *= 1.0 / 255.0;
        h *= 1.0 / 255.0;

        let c0 = v_add(&mean_color, &v_mul(&axis, l));
        let c1 = v_add(&mean_color, &v_mul(&axis, h));
        min_color = c0.map(saturate);
        max_color = c1.map(saturate);
        if min_color[3] > max_color[3] {
            core::mem::swap(&mut min_color, &mut max_color);
        }
    } else {
        // RGB axis is luma: 2D PCA on (L, A).
        let block_cols_l: [i32; 4] = core::array::from_fn(|i| {
            block_cols[i].r() as i32 + block_cols[i].g() as i32 + block_cols[i].b() as i32
        });
        let block_cols_a: [i32; 4] = core::array::from_fn(|i| block_cols[i].a() as i32 * 3);
        let (mut p0_min, mut p0_max, mut p1_min, mut p1_max) =
            (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        for y in 0..4 {
            let cs = sel.selectors[y];
            let as_ = alpha_selectors.selectors[y];
            for x in 0..4 {
                let l = block_cols_l[((cs >> (x * 2)) & 3) as usize];
                let a = block_cols_a[((as_ >> (x * 2)) & 3) as usize];
                let p0 = l + a;
                p0_min = p0_min.min(p0);
                p0_max = p0_max.max(p0);
                let p1 = l - a;
                p1_min = p1_min.min(p1);
                p1_max = p1_max.max(p1);
            }
        }
        let dist0 = p0_max - p0_min;
        let dist1 = p1_max - p1_min;

        min_color = [
            lc.r() as f32 * S,
            lc.g() as f32 * S,
            lc.b() as f32 * S,
            block_cols[alo].a() as f32 * S,
        ];
        max_color = [
            hc.r() as f32 * S,
            hc.g() as f32 * S,
            hc.b() as f32 * S,
            block_cols[ahi].a() as f32 * S,
        ];
        if dist1 > dist0 {
            for c in 0..3 {
                core::mem::swap(&mut min_color[c], &mut max_color[c]);
            }
        }
    }

    // 4433 4443
    let q = |v: f32, s: f32| clamp255((v * s + 0.5) as i32) as u32;
    let tmin = [
        q(min_color[0], 15.0),
        q(min_color[1], 15.0),
        q(min_color[2], 7.0),
        q(min_color[3], 7.0),
    ];
    let tmax = [
        q(max_color[0], 15.0),
        q(max_color[1], 15.0),
        q(max_color[2], 15.0),
        q(max_color[3], 7.0),
    ];
    blk.set_trans_low_color(tmin[0], tmin[1], tmin[2], tmin[3]);
    blk.set_trans_high_color(tmax[0], tmax[1], tmax[2], tmax[3]);

    let color_a = [
        ((tmin[0] << 1) | (tmin[0] >> 3)) & 0xFF,
        ((tmin[1] << 1) | (tmin[1] >> 3)) & 0xFF,
        ((tmin[2] << 2) | (tmin[2] >> 1)) & 0xFF,
        (tmin[3] << 1) & 0xFF,
    ];
    let color_b = [
        ((tmax[0] << 1) | (tmax[0] >> 3)) & 0xFF,
        ((tmax[1] << 1) | (tmax[1] >> 3)) & 0xFF,
        ((tmax[2] << 1) | (tmax[2] >> 3)) & 0xFF,
        ((tmax[3] << 1) | 1) & 0xFF,
    ];
    let color0 = convert_rgba_5554_to_8888(color_a);
    let color3 = convert_rgba_5554_to_8888(color_b);
    let [lr, lg, lb, la] = color0;
    let axis_r = color3[0] - lr;
    let axis_g = color3[1] - lg;
    let axis_b = color3[2] - lb;
    let axis_a = color3[3] - la;
    let len_a = axis_r * axis_r + axis_g * axis_g + axis_b * axis_b + axis_a * axis_a;
    let thresh01 = (len_a * 3) / 16;
    let thresh12 = len_a >> 1;
    let thresh23 = (len_a * 13) / 16;
    let qsel = |d: i32| (d >= thresh23) as u32 + (d >= thresh12) as u32 + (d >= thresh01) as u32;

    if (axis_r | axis_g | axis_b) == 0 {
        let ca_sel: [u32; 4] =
            core::array::from_fn(|i| qsel((block_cols[i].a() as i32 - la) * axis_a));
        for y in 0..4 {
            let a = alpha_selectors.selectors[y] as u32;
            blk.modulation[y] = (ca_sel[(a & 3) as usize]
                | (ca_sel[((a >> 2) & 3) as usize] << 2)
                | (ca_sel[((a >> 4) & 3) as usize] << 4)
                | (ca_sel[((a >> 6) & 3) as usize] << 6)) as u8;
        }
    } else {
        let cy: [i32; 4] = core::array::from_fn(|i| {
            (block_cols[i].r() as i32 - lr) * axis_r
                + (block_cols[i].g() as i32 - lg) * axis_g
                + (block_cols[i].b() as i32 - lb) * axis_b
        });
        let ca: [i32; 4] = core::array::from_fn(|i| (block_cols[i].a() as i32 - la) * axis_a);
        for y in 0..4 {
            let c = sel.selectors[y] as u32;
            let a = alpha_selectors.selectors[y] as u32;
            let mut s = 0u32;
            for x in 0..4 {
                let sh = x * 2;
                let d = cy[((c >> sh) & 3) as usize] + ca[((a >> sh) & 3) as usize];
                s |= qsel(d) << sh;
            }
            blk.modulation[y] = s as u8;
        }
    }
}
