// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of the ETC1S -> BC7 mode 5 converters, the adaptive chroma filter and
//! `bc7_mode_5_encoder` (basisu_transcoder.cpp).
//!
//! Float math follows the C++ evaluation order so output matches byte for byte.
#![allow(clippy::neg_multiply, clippy::identity_op, clippy::erasing_op)]

use std::sync::OnceLock;

use super::etc::{Color32, DecoderEtcBlock, Endpoint, Selector};
use super::tables::{BC7_M5_EQUALS_1, ETC1_G_TO_BC7_M5A, ETC1_TO_BC7_M5_COLOR};

/// `g_bc7_weights2`.
pub const BC7_WEIGHTS2: [u32; 4] = [0, 21, 43, 64];

pub const ETC1_TO_BC7_M5_SELECTOR_RANGES: [(u32, u32); 6] =
    [(0, 3), (1, 3), (0, 2), (1, 2), (2, 3), (0, 1)];
pub const NUM_ETC1_TO_BC7_M5_SELECTOR_MAPPINGS: usize = 10;
pub const ETC1_TO_BC7_M5_SELECTOR_MAPPINGS: [[u8; 4]; 10] = [
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
pub const ETC1_TO_BC7_M5A_SELECTOR_RANGES: [(u32, u32); 6] =
    [(0, 3), (1, 3), (0, 2), (1, 2), (2, 3), (0, 1)];

/// Port of `bc7_mode_5` as two little-endian 64-bit words.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bc7Mode5 {
    pub lo: u64,
    pub hi: u64,
}

macro_rules! field {
    ($get:ident, $set:ident, $word:ident, $shift:expr, $bits:expr) => {
        #[inline]
        pub fn $get(&self) -> u32 {
            ((self.$word >> $shift) & ((1u64 << $bits) - 1)) as u32
        }
        #[inline]
        pub fn $set(&mut self, v: u32) {
            let mask = ((1u64 << $bits) - 1) << $shift;
            self.$word = (self.$word & !mask) | (((v as u64) << $shift) & mask);
        }
    };
}

impl Bc7Mode5 {
    field!(mode, set_mode, lo, 0, 6);
    field!(r0, set_r0, lo, 8, 7);
    field!(r1, set_r1, lo, 15, 7);
    field!(g0, set_g0, lo, 22, 7);
    field!(g1, set_g1, lo, 29, 7);
    field!(b0, set_b0, lo, 36, 7);
    field!(b1, set_b1, lo, 43, 7);
    field!(a0, set_a0, lo, 50, 8);
    field!(a1_0, set_a1_0, lo, 58, 6);
    field!(a1_1, set_a1_1, hi, 0, 2);

    pub fn from_bytes(b: &[u8]) -> Bc7Mode5 {
        let mut w = [0u8; 16];
        let n = b.len().min(16);
        w[..n].copy_from_slice(&b[..n]);
        let mut lo = [0u8; 8];
        let mut hi = [0u8; 8];
        lo.copy_from_slice(&w[..8]);
        hi.copy_from_slice(&w[8..]);
        Bc7Mode5 {
            lo: u64::from_le_bytes(lo),
            hi: u64::from_le_bytes(hi),
        }
    }

    pub fn to_bytes(self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&self.lo.to_le_bytes());
        out[8..].copy_from_slice(&self.hi.to_le_bytes());
        out
    }

    /// Port of `set_block_bits(pBytes, val, num_bits, cur_ofs)` on the 128-bit block.
    pub fn set_block_bits(&mut self, val: u32, num_bits: u32, cur_ofs: u32) {
        let mut bytes = self.to_bytes();
        let mut val = val;
        let mut mask: u32 = ((1u64 << num_bits) - 1) as u32;
        let mut num_bits = num_bits;
        let mut cur_ofs = cur_ofs;
        while num_bits != 0 {
            let n = (8 - (cur_ofs & 7)).min(num_bits);
            let idx = (cur_ofs >> 3) as usize;
            if idx < 16 {
                bytes[idx] &= !((mask << (cur_ofs & 7)) as u8);
                bytes[idx] |= (val << (cur_ofs & 7)) as u8;
            }
            val >>= n;
            mask >>= n;
            num_bits -= n;
            cur_ofs += n;
        }
        *self = Bc7Mode5::from_bytes(&bytes);
    }
}

struct Bc7Tables {
    m5_range_index: [[u32; 4]; 4],
    m5a_range_index: [[u32; 4]; 4],
    mode5_rgba_midpoints: [f32; 128],
}

fn tables() -> &'static Bc7Tables {
    static T: OnceLock<Bc7Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut m5_range_index = [[0u32; 4]; 4];
        for (i, &(l, h)) in ETC1_TO_BC7_M5_SELECTOR_RANGES.iter().enumerate() {
            m5_range_index[l as usize][h as usize] = i as u32;
        }
        let mut m5a_range_index = [[0u32; 4]; 4];
        for (i, &(l, h)) in ETC1_TO_BC7_M5A_SELECTOR_RANGES.iter().enumerate() {
            m5a_range_index[l as usize][h as usize] = i as u32;
        }
        let mut mid = [0f32; 128];
        for (i, m) in mid.iter_mut().enumerate() {
            let mut vl = (i as u32) << 1;
            vl |= vl >> 7;
            let lo = vl as f32 / 255.0f32;
            let mut vh = ((i as u32 + 1).min(127)) << 1;
            vh |= vh >> 7;
            let hi = vh as f32 / 255.0f32;
            *m = if i == 127 {
                1e15f32
            } else {
                (lo + hi) / 2.0f32
            };
        }
        Bc7Tables {
            m5_range_index,
            m5a_range_index,
            mode5_rgba_midpoints: mid,
        }
    })
}

fn equals_1(i: u32) -> (u32, u32) {
    let (hi, lo) = BC7_M5_EQUALS_1.get(i as usize).copied().unwrap_or((0, 0));
    (hi as u32, lo as u32)
}

/// Port of `convert_etc1s_to_bc7_m5_color`.
pub fn convert_etc1s_to_bc7_m5_color(dst: &mut Bc7Mode5, ep: &Endpoint, sel: &Selector) {
    *dst = Bc7Mode5::default();
    // Set alpha to 255
    dst.set_mode(1 << 5);
    dst.set_a0(255);
    dst.set_a1_0(63);
    dst.set_a1_1(3);

    let low_selector = sel.lo_selector as u32;
    let high_selector = sel.hi_selector as u32;
    let (base_r, base_g, base_b) = (
        ep.color5.r() as u32,
        ep.color5.g() as u32,
        ep.color5.b() as u32,
    );
    let inten_table = ep.inten5 as u32;

    if sel.num_unique_selectors == 1 {
        // Solid color block - use precomputed tables and set selectors to 1.
        let (r, g, b) = DecoderEtcBlock::get_block_color5(&ep.color5, inten_table, low_selector);
        dst.set_r0(equals_1(r).1);
        dst.set_g0(equals_1(g).1);
        dst.set_b0(equals_1(b).1);
        dst.set_r1(equals_1(r).0);
        dst.set_g1(equals_1(g).0);
        dst.set_b1(equals_1(b).0);
        dst.set_block_bits(0x2aaa_aaab, 31, 66);
        return;
    } else if sel.num_unique_selectors == 2 {
        // Block truncation coding for extreme blocks.
        let block_colors = DecoderEtcBlock::get_block_colors5(
            &Color32::new(base_r, base_g, base_b, 255),
            inten_table,
        );
        let lc = block_colors[(low_selector & 3) as usize];
        let hc = block_colors[(high_selector & 3) as usize];
        let (r0, g0, b0) = (lc.r() as u32, lc.g() as u32, lc.b() as u32);
        let (r1, g1, b1) = (hc.r() as u32, hc.g() as u32, hc.b() as u32);
        dst.set_r0(r0 >> 1);
        dst.set_g0(g0 >> 1);
        dst.set_b0(b0 >> 1);
        dst.set_r1(r1 >> 1);
        dst.set_g1(g1 >> 1);
        dst.set_b1(b1 >> 1);
        let mut output_low_selector = 0u32;
        let mut output_bit_offset = 0u32;
        let mut output_bits = 0u32;
        for y in 0..4 {
            for x in 0..4 {
                let s = sel.get_selector(x, y);
                let mut os = if s == low_selector {
                    output_low_selector
                } else {
                    3 ^ output_low_selector
                };
                let mut num_bits = 2;
                if (x | y) == 0 {
                    if os & 2 != 0 {
                        dst.set_r0(r1 >> 1);
                        dst.set_g0(g1 >> 1);
                        dst.set_b0(b1 >> 1);
                        dst.set_r1(r0 >> 1);
                        dst.set_g1(g0 >> 1);
                        dst.set_b1(b0 >> 1);
                        output_low_selector = 3;
                        os = 0;
                    }
                    num_bits = 1;
                }
                output_bits |= os << output_bit_offset;
                output_bit_offset += num_bits;
            }
        }
        dst.set_block_bits(output_bits, 31, 66);
        return;
    }

    let t = tables();
    let selector_range_table =
        t.m5_range_index[(low_selector & 3) as usize][(high_selector & 3) as usize] as usize;
    let stride = ETC1_TO_BC7_M5_SELECTOR_RANGES.len() * NUM_ETC1_TO_BC7_M5_SELECTOR_MAPPINGS;
    let base = |c: u32| {
        (inten_table as usize * 32 + c as usize) * stride
            + selector_range_table * NUM_ETC1_TO_BC7_M5_SELECTOR_MAPPINGS
    };
    let get = |i: usize| ETC1_TO_BC7_M5_COLOR.get(i).copied().unwrap_or((0, 0, 0));
    let (tr, tg, tb) = (base(base_r), base(base_g), base(base_b));
    let mut best_err = u32::MAX;
    let mut best_mapping = 0usize;
    for m in 0..NUM_ETC1_TO_BC7_M5_SELECTOR_MAPPINGS {
        let total_err = get(tr + m).2 as u32 + get(tg + m).2 as u32 + get(tb + m).2 as u32;
        if total_err < best_err {
            best_err = total_err;
            best_mapping = m;
        }
    }
    let xlat = &ETC1_TO_BC7_M5_SELECTOR_MAPPINGS[best_mapping];
    let (pr, pg, pb) = (
        get(tr + best_mapping),
        get(tg + best_mapping),
        get(tb + best_mapping),
    );
    let mut s_inv = 0u32;
    if xlat[sel.get_selector(0, 0) as usize] & 2 != 0 {
        dst.set_r0(pr.1 as u32);
        dst.set_g0(pg.1 as u32);
        dst.set_b0(pb.1 as u32);
        dst.set_r1(pr.0 as u32);
        dst.set_g1(pg.0 as u32);
        dst.set_b1(pb.0 as u32);
        s_inv = 3;
    } else {
        dst.set_r0(pr.0 as u32);
        dst.set_g0(pg.0 as u32);
        dst.set_b0(pb.0 as u32);
        dst.set_r1(pr.1 as u32);
        dst.set_g1(pg.1 as u32);
        dst.set_b1(pb.1 as u32);
    }
    let mut output_bits = 0u32;
    let mut output_bit_ofs = 0u32;
    for y in 0..4 {
        for x in 0..4 {
            let s = sel.get_selector(x, y);
            let os = xlat[s as usize] as u32 ^ s_inv;
            output_bits |= os << output_bit_ofs;
            output_bit_ofs += if (x | y) == 0 { 1 } else { 2 };
        }
    }
    dst.set_block_bits(output_bits, 31, 66);
}

/// Port of `convert_etc1s_to_bc7_m5_alpha`.
pub fn convert_etc1s_to_bc7_m5_alpha(dst: &mut Bc7Mode5, ep: &Endpoint, sel: &Selector) {
    let low_selector = sel.lo_selector as u32;
    let high_selector = sel.hi_selector as u32;
    let base_r = ep.color5.r() as u32;
    let inten_table = ep.inten5 as u32;

    if sel.num_unique_selectors == 1 {
        let (r, _, _) = DecoderEtcBlock::get_block_color5(&ep.color5, inten_table, low_selector);
        dst.set_a0(r);
        dst.set_a1_0(r & 63);
        dst.set_a1_1(r >> 6);
        return;
    } else if sel.num_unique_selectors == 2 {
        let block_colors = DecoderEtcBlock::get_block_colors5_g(&ep.color5, inten_table);
        let bl = block_colors[(low_selector & 3) as usize] as u32;
        let bh = block_colors[(high_selector & 3) as usize] as u32;
        dst.set_a0(bl);
        dst.set_a1_0(bh & 63);
        dst.set_a1_1(bh >> 6);
        let mut output_low_selector = 0u32;
        let mut output_bit_offset = 0u32;
        let mut output_bits = 0u32;
        for y in 0..4 {
            for x in 0..4 {
                let s = sel.get_selector(x, y);
                let mut os = if s == low_selector {
                    output_low_selector
                } else {
                    3 ^ output_low_selector
                };
                let mut num_bits = 2;
                if (x | y) == 0 {
                    if os & 2 != 0 {
                        dst.set_a0(bh);
                        dst.set_a1_0(bl & 63);
                        dst.set_a1_1(bl >> 6);
                        output_low_selector = 3;
                        os = 0;
                    }
                    num_bits = 1;
                }
                output_bits |= os << output_bit_offset;
                output_bit_offset += num_bits;
            }
        }
        dst.set_block_bits(output_bits, 31, 97);
        return;
    }

    let t = tables();
    let selector_range_table =
        t.m5a_range_index[(low_selector & 3) as usize][(high_selector & 3) as usize] as usize;
    let n = ETC1_TO_BC7_M5A_SELECTOR_RANGES.len();
    let idx = inten_table as usize * (32 * n) + base_r as usize * n + selector_range_table;
    let (tlo, thi, ttrans) = ETC1_G_TO_BC7_M5A.get(idx).copied().unwrap_or((0, 0, 0));
    dst.set_a0(tlo as u32);
    dst.set_a1_0(thi as u32 & 63);
    dst.set_a1_1(thi as u32 >> 6);
    let mut output_bit_offset = 0u32;
    let mut output_bits = 0u32;
    let mut selector_trans = ttrans as u32;
    for y in 0..4 {
        for x in 0..4 {
            let s = sel.get_selector(x, y);
            let mut os = (selector_trans >> (s * 2)) & 3;
            let mut num_bits = 2;
            if (x | y) == 0 {
                if os & 2 != 0 {
                    dst.set_a0(thi as u32);
                    dst.set_a1_0(tlo as u32 & 63);
                    dst.set_a1_1(tlo as u32 >> 6);
                    selector_trans ^= 0xFF;
                    os ^= 3;
                }
                num_bits = 1;
            }
            output_bits |= os << output_bit_offset;
            output_bit_offset += num_bits;
        }
    }
    dst.set_block_bits(output_bits, 31, 97);
}

#[inline]
fn lerpf(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}

#[inline]
fn clampf(v: f32, l: f32, h: f32) -> f32 {
    if v < l {
        l
    } else if v > h {
        h
    } else {
        v
    }
}

#[inline]
fn bc7_7_to_8(v: u32) -> u32 {
    (v << 1) | (v >> 6)
}

#[inline]
fn bc7_interp2(l: u32, h: u32, w: u32) -> u32 {
    (l * (64 - BC7_WEIGHTS2[w as usize]) + h * BC7_WEIGHTS2[w as usize] + 32) >> 6
}

/// `color5_to_cocg`.
fn color5_to_cocg(e: &Endpoint) -> [f32; 2] {
    let ex = |c: u8| (((c as i32) << 3) | ((c as i32) >> 2)) as f32;
    let (r, g, b) = (ex(e.color5.r()), ex(e.color5.g()), ex(e.color5.b()));
    // rgb_to_cocg: dot with (0.5, 0, -0.5) and (-0.25, 0.5, -0.25)
    [
        (r * 0.5f32) + (g * 0.0f32) + (b * -0.5f32),
        (r * -0.25f32) + (g * 0.5f32) + (b * -0.25f32),
    ]
}

/// Port of `chroma_filter_bc7_mode5`. `decoded_endpoints[x + y * num_blocks_x]`.
pub fn chroma_filter_bc7_mode5(
    decoded_endpoints: &[u16],
    dst: &mut [u8],
    dst_offset: usize,
    num_blocks_x: u32,
    num_blocks_y: u32,
    output_row_pitch_in_blocks_or_pixels: u32,
    endpoints: &[Endpoint],
) {
    const CHROMA_THRESH: f32 = 10.0;
    let (nbx, nby) = (num_blocks_x as i32, num_blocks_y as i32);
    let ep_at = |x: i32, y: i32| -> Endpoint {
        let x = x.clamp(0, nbx - 1);
        let y = y.clamp(0, nby - 1);
        let idx = decoded_endpoints
            .get((x + y * nbx) as usize)
            .copied()
            .unwrap_or(0) as usize;
        endpoints.get(idx).copied().unwrap_or_default()
    };
    for by in 0..nby {
        for bx in 0..nbx {
            let center = color5_to_cocg(&ep_at(bx, by));
            let mut do_filter = false;
            'outer: for dy in -1..=1 {
                let oy = by + dy;
                if oy < 0 || oy >= nby {
                    continue;
                }
                for dx in -1..=1 {
                    if (dx | dy) == 0 {
                        continue;
                    }
                    let ox = bx + dx;
                    if ox < 0 || ox >= nbx {
                        continue;
                    }
                    let nearby = color5_to_cocg(&ep_at(ox, oy));
                    let delta_co = (nearby[0] - center[0]).abs();
                    let delta_cg = (nearby[1] - center[1]).abs();
                    if delta_co > CHROMA_THRESH || delta_cg > CHROMA_THRESH {
                        do_filter = true;
                        break 'outer;
                    }
                }
            }
            if !do_filter {
                continue;
            }

            let ofs = dst_offset
                + (bx as usize + by as usize * output_row_pitch_in_blocks_or_pixels as usize) * 16;
            let Some(block_bytes) = dst.get(ofs..ofs + 16) else {
                continue;
            };
            let blk = Bc7Mode5::from_bytes(block_bytes);
            let (lr, lg, lb) = (
                bc7_7_to_8(blk.r0()),
                bc7_7_to_8(blk.g0()),
                bc7_7_to_8(blk.b0()),
            );
            let (hr, hg, hb) = (
                bc7_7_to_8(blk.r1()),
                bc7_7_to_8(blk.g1()),
                bc7_7_to_8(blk.b1()),
            );
            let mut y_vals = [0f32; 4];
            for (i, yv) in y_vals.iter_mut().enumerate() {
                let i = i as u32;
                let cr = bc7_interp2(lr, hr, i);
                let cg = bc7_interp2(lg, hg, i);
                let cb = bc7_interp2(lb, hb, i);
                *yv = cr as f32 * 0.25f32 + cg as f32 * 0.5f32 + cb as f32 * 0.25f32;
            }
            let mut sel_bits = blk.hi >> 2;
            let mut block_y_vals = [0f32; 16];
            let mut y_sum = 0.0f32;
            let mut y_sum_sq = 0.0f32;
            for (i, byv) in block_y_vals.iter_mut().enumerate() {
                let sel = (sel_bits & if i != 0 { 3 } else { 1 }) as usize;
                sel_bits >>= if i != 0 { 2 } else { 1 };
                let y = y_vals[sel];
                *byv = y;
                y_sum += y;
                y_sum_sq += y * y;
            }
            let s = 1.0f32 / 16.0f32;
            let ys = y_sum * s;
            let y_var = (y_sum_sq * s) - ys * ys;
            if y_var < 3.0f32 {
                continue;
            }

            let mut block_to_pack = [Color32::default(); 16];
            for bpy in 0..4i32 {
                let uby = by + ((bpy - 2) >> 2);
                for bpx in 0..4i32 {
                    let fx = (((bpx + 2) & 3) as f32 + 0.5f32) * (1.0f32 / 4.0f32);
                    let fy = (((bpy + 2) & 3) as f32 + 0.5f32) * (1.0f32 / 4.0f32);
                    let ubx = bx + ((bpx - 2) >> 2);
                    let a = color5_to_cocg(&ep_at(ubx, uby));
                    let b = color5_to_cocg(&ep_at(ubx + 1, uby));
                    let c = color5_to_cocg(&ep_at(ubx, uby + 1));
                    let d = color5_to_cocg(&ep_at(ubx + 1, uby + 1));
                    let ab = [lerpf(a[0], b[0], fx), lerpf(a[1], b[1], fx)];
                    let cd = [lerpf(c[0], d[0], fx), lerpf(c[1], d[1], fx)];
                    let f = [lerpf(ab[0], cd[0], fy), lerpf(ab[1], cd[1], fy)];
                    let ycocg = [block_y_vals[(bpx + bpy * 4) as usize], f[0], f[1]];
                    // ycocg_to_rgb: dots with (1,1,-1), (1,0,1), (1,-1,-1)
                    let conv = [
                        (ycocg[0] * 1.0f32) + (ycocg[1] * 1.0f32) + (ycocg[2] * -1.0f32),
                        (ycocg[0] * 1.0f32) + (ycocg[1] * 0.0f32) + (ycocg[2] * 1.0f32),
                        (ycocg[0] * 1.0f32) + (ycocg[1] * -1.0f32) + (ycocg[2] * -1.0f32),
                    ];
                    let cl = |v: f32| (0.5f32 + clampf(v, 0.0, 255.0)) as i32 as u32;
                    block_to_pack[(bpx + bpy * 4) as usize] =
                        Color32::new(cl(conv[0]), cl(conv[1]), cl(conv[2]), 255);
                }
            }
            let mut out = Bc7Mode5::default();
            encode_bc7_mode_5_block(&mut out, &block_to_pack, false);
            if let Some(d) = dst.get_mut(ofs..ofs + 16) {
                d.copy_from_slice(&out.to_bytes());
            }
        }
    }
}

// ----------------------------------------------------------------- bc7_mode_5_encoder

#[inline]
fn from_7(v: u32) -> u32 {
    (v << 1) | (v >> 6)
}

fn to_7f(c: f32) -> i32 {
    let mid = &tables().mode5_rgba_midpoints;
    let mut vl = (c * 127.0f32) as i32;
    let m = mid.get(vl.clamp(0, 127) as usize).copied().unwrap_or(1e15);
    vl += (c > m) as i32;
    vl.clamp(0, 127)
}

fn to_7i(c8: i32) -> i32 {
    to_7f(c8 as f32 * (1.0f32 / 255.0f32))
}

#[inline]
fn mul_8bit(a: i32, b: i32) -> i32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

#[inline]
fn lerp_8bit(a: i32, b: i32, s: i32) -> i32 {
    a + mul_8bit(b - a, s)
}

#[allow(clippy::too_many_arguments)]
fn eval_weights(
    pixels: &[Color32; 16],
    weights: &mut [u8; 16],
    lr: i32,
    lg: i32,
    lb: i32,
    hr: i32,
    hg: i32,
    hb: i32,
) {
    let (lr, lg, lb) = (from_7(lr as u32), from_7(lg as u32), from_7(lb as u32));
    let (hr, hg, hb) = (from_7(hr as u32), from_7(hg as u32), from_7(hb as u32));
    let mut cr = [0i32; 4];
    let mut cg = [0i32; 4];
    let mut cb = [0i32; 4];
    for i in 0..4 {
        cr[i] = (bc7_interp2(lr, hr, i as u32) as u8) as i32;
        cg[i] = (bc7_interp2(lg, hg, i as u32) as u8) as i32;
        cb[i] = (bc7_interp2(lb, hb, i as u32) as u8) as i32;
    }
    let (mut ar, mut ag, mut ab) = (cr[3] - cr[0], cg[3] - cg[0], cb[3] - cb[0]);
    let dots: [i32; 4] = core::array::from_fn(|i| cr[i] * ar + cg[i] * ag + cb[i] * ab);
    let (t0, t1, t2) = (dots[0] + dots[1], dots[1] + dots[2], dots[2] + dots[3]);
    ar *= 2;
    ag *= 2;
    ab *= 2;
    for i in 0..16 {
        let p = &pixels[i];
        let d = p.r() as i32 * ar + p.g() as i32 * ag + p.b() as i32 * ab;
        weights[i] = (d > t0) as u8 + (d >= t1) as u8 + (d >= t2) as u8;
    }
}

#[allow(clippy::too_many_arguments)]
fn pack_bc7_mode5_rgb_block(
    dst: &mut Bc7Mode5,
    mut lr: i32,
    mut lg: i32,
    mut lb: i32,
    mut hr: i32,
    mut hg: i32,
    mut hb: i32,
    weights: &[u8; 16],
) {
    dst.lo = 0;
    let mut weight_inv = 0u8;
    if weights[0] & 2 != 0 {
        std::mem::swap(&mut lr, &mut hr);
        std::mem::swap(&mut lg, &mut hg);
        std::mem::swap(&mut lb, &mut hb);
        weight_inv = 3;
    }
    dst.set_mode(32);
    dst.set_r0(lr as u32);
    dst.set_r1(hr as u32);
    dst.set_g0(lg as u32);
    dst.set_g1(hg as u32);
    dst.set_b0(lb as u32);
    dst.set_b1(hb as u32);
    dst.set_a0(255);
    dst.set_a1_0(63);
    let mut sel_bits: u64 = 3;
    let mut cur_ofs = 2u32;
    for (i, &w) in weights.iter().enumerate() {
        sel_bits |= ((weight_inv ^ w) as u64) << cur_ofs;
        cur_ofs += if i != 0 { 2 } else { 1 };
    }
    dst.hi = sel_bits;
}

const WEIGHT_VALS4: [u32; 4] = [0x000009, 0x010204, 0x040201, 0x090000];

#[allow(clippy::too_many_arguments)]
fn compute_least_squares_endpoints4_rgb(
    colors: &[Color32; 16],
    selectors: &[u8; 16],
    lr: &mut i32,
    lg: &mut i32,
    lb: &mut i32,
    hr: &mut i32,
    hg: &mut i32,
    hb: &mut i32,
    total_r: i32,
    total_g: i32,
    total_b: i32,
) -> bool {
    let (mut uq00_r, mut uq00_g, mut uq00_b) = (0u32, 0u32, 0u32);
    let mut weight_accum = 0u32;
    for i in 0..16 {
        let (r, g, b) = (
            colors[i].r() as u32,
            colors[i].g() as u32,
            colors[i].b() as u32,
        );
        let sel = selectors[i] as u32;
        weight_accum = weight_accum.wrapping_add(WEIGHT_VALS4[(sel & 3) as usize]);
        uq00_r += sel * r;
        uq00_g += sel * g;
        uq00_b += sel * b;
    }
    let q10_r = total_r * 3 - uq00_r as i32;
    let q10_g = total_g * 3 - uq00_g as i32;
    let q10_b = total_b * 3 - uq00_b as i32;
    let z00 = ((weight_accum >> 16) & 0xFF) as f32;
    let z10 = ((weight_accum >> 8) & 0xFF) as f32;
    let z11 = (weight_accum & 0xFF) as f32;
    let z01 = z10;
    let mut det = z00 * z11 - z01 * z10;
    if (det.abs() as f64) < 1e-8f32 as f64 {
        return false;
    }
    det = (3.0f32 / 255.0f32) / det;
    let iz00 = z11 * det;
    let iz01 = -z01 * det;
    let iz10 = -z10 * det;
    let iz11 = z00 * det;
    let fhr = clampf(iz00 * uq00_r as f32 + iz01 * q10_r as f32, 0.0, 1.0);
    let flr = clampf(iz10 * uq00_r as f32 + iz11 * q10_r as f32, 0.0, 1.0);
    let fhg = clampf(iz00 * uq00_g as f32 + iz01 * q10_g as f32, 0.0, 1.0);
    let flg = clampf(iz10 * uq00_g as f32 + iz11 * q10_g as f32, 0.0, 1.0);
    let fhb = clampf(iz00 * uq00_b as f32 + iz01 * q10_b as f32, 0.0, 1.0);
    let flb = clampf(iz10 * uq00_b as f32 + iz11 * q10_b as f32, 0.0, 1.0);
    *lr = to_7f(flr);
    *lg = to_7f(flg);
    *lb = to_7f(flb);
    *hr = to_7f(fhr);
    *hg = to_7f(fhg);
    *hb = to_7f(fhb);
    true
}

/// Port of `bc7_mode_5_encoder::encode_bc7_mode_5_block`.
pub fn encode_bc7_mode_5_block(dst: &mut Bc7Mode5, pixels: &[Color32; 16], hq_mode: bool) {
    let (mut total_r, mut total_g, mut total_b) = (0i32, 0i32, 0i32);
    let (mut min_r, mut min_g, mut min_b) = (255i32, 255i32, 255i32);
    let (mut max_r, mut max_g, mut max_b) = (0i32, 0i32, 0i32);
    for p in pixels {
        let (r, g, b) = (p.r() as i32, p.g() as i32, p.b() as i32);
        total_r += r;
        total_g += g;
        total_b += b;
        min_r = min_r.min(r);
        min_g = min_g.min(g);
        min_b = min_b.min(b);
        max_r = max_r.max(r);
        max_g = max_g.max(g);
        max_b = max_b.max(b);
    }
    if min_r == max_r && min_g == max_g && min_b == max_b {
        let (hr, lr) = equals_1(min_r as u32);
        let (hg, lg) = equals_1(min_g as u32);
        let (hb, lb) = equals_1(min_b as u32);
        let solid_weights = [1u8; 16];
        pack_bc7_mode5_rgb_block(
            dst,
            lr as i32,
            lg as i32,
            lb as i32,
            hr as i32,
            hg as i32,
            hb as i32,
            &solid_weights,
        );
        return;
    }
    let (mean_r, mean_g, mean_b) = ((total_r + 8) >> 4, (total_g + 8) >> 4, (total_b + 8) >> 4);
    let mut icov = [0i32; 6];
    for p in pixels {
        let r = p.r() as i32 - mean_r;
        let g = p.g() as i32 - mean_g;
        let b = p.b() as i32 - mean_b;
        icov[0] += r * r;
        icov[1] += r * g;
        icov[2] += r * b;
        icov[3] += g * g;
        icov[4] += g * b;
        icov[5] += b * b;
    }
    let block_max_var = icov[0].max(icov[3]).max(icov[5]);
    const SIMPLE_BLOCK_THRESH: i32 = 10 * 16;
    if !hq_mode && block_max_var < SIMPLE_BLOCK_THRESH {
        const L: i32 = 16;
        const H: i32 = 239;
        let lr = to_7i(lerp_8bit(min_r, max_r, L));
        let lg = to_7i(lerp_8bit(min_g, max_g, L));
        let lb = to_7i(lerp_8bit(min_b, max_b, L));
        let hr = to_7i(lerp_8bit(min_r, max_r, H));
        let hg = to_7i(lerp_8bit(min_g, max_g, H));
        let hb = to_7i(lerp_8bit(min_b, max_b, H));
        let mut cur_weights = [0u8; 16];
        eval_weights(pixels, &mut cur_weights, lr, lg, lb, hr, hg, hb);
        pack_bc7_mode5_rgb_block(dst, lr, lg, lb, hr, hg, hb, &cur_weights);
        return;
    }
    let cov: [f32; 6] = core::array::from_fn(|i| icov[i] as f32);
    let sc = 1.0f32 / block_max_var as f32;
    let (wx, wy, wz) = (sc * cov[0], sc * cov[3], sc * cov[5]);
    let alt_xr = cov[0] * wx + cov[1] * wy + cov[2] * wz;
    let alt_xg = cov[1] * wx + cov[3] * wy + cov[4] * wz;
    let alt_xb = cov[2] * wx + cov[4] * wy + cov[5] * wz;
    let (mut saxis_r, mut saxis_g, mut saxis_b) = (306i32, 601i32, 117i32);
    let k = alt_xr.abs().max(alt_xg.abs()).max(alt_xb.abs());
    if k.abs() >= 0.0000125f32 {
        let m = 2048.0f32 / k;
        saxis_r = (alt_xr * m) as i32;
        saxis_g = (alt_xg * m) as i32;
        saxis_b = (alt_xb * m) as i32;
    }
    saxis_r = ((saxis_r as u32) << 4) as i32;
    saxis_g = ((saxis_g as u32) << 4) as i32;
    saxis_b = ((saxis_b as u32) << 4) as i32;
    let mut low_dot = i32::MAX;
    let mut high_dot = i32::MIN;
    for (i, p) in pixels.iter().enumerate() {
        let dot = ((p.r() as i32)
            .wrapping_mul(saxis_r)
            .wrapping_add((p.g() as i32).wrapping_mul(saxis_g))
            .wrapping_add((p.b() as i32).wrapping_mul(saxis_b))
            & !0xF)
            + i as i32;
        low_dot = low_dot.min(dot);
        high_dot = high_dot.max(dot);
    }
    let low_c = (low_dot & 15) as usize;
    let high_c = (high_dot & 15) as usize;
    let mut lr = to_7i(pixels[low_c].r() as i32);
    let mut lg = to_7i(pixels[low_c].g() as i32);
    let mut lb = to_7i(pixels[low_c].b() as i32);
    let mut hr = to_7i(pixels[high_c].r() as i32);
    let mut hg = to_7i(pixels[high_c].g() as i32);
    let mut hb = to_7i(pixels[high_c].b() as i32);
    let mut cur_weights = [0u8; 16];
    eval_weights(pixels, &mut cur_weights, lr, lg, lb, hr, hg, hb);
    if compute_least_squares_endpoints4_rgb(
        pixels,
        &cur_weights,
        &mut lr,
        &mut lg,
        &mut lb,
        &mut hr,
        &mut hg,
        &mut hb,
        total_r,
        total_g,
        total_b,
    ) {
        eval_weights(pixels, &mut cur_weights, lr, lg, lb, hr, hg, hb);
    }
    pack_bc7_mode5_rgb_block(dst, lr, lg, lb, hr, hg, hb, &cur_weights);
}
