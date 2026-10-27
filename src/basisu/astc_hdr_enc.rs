// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of the CEM 7 / CEM 11 single plane parts of `encoder/basisu_astc_hdr_common.cpp` used by
//! the UASTC HDR 4x4 encoder: codec base options, qlog tables, PCA / least squares endpoint
//! fitting, selector evaluation, mode 7 / mode 11 endpoint packing and the
//! `encode_astc_hdr_block_mode_7` / `_mode_11` block encoders, and the 6x6 only dual plane,
//! downsampled weight grid and endpoint refinement paths.
//!
//! Float evaluation follows the SSE build (no FMA): f32 vector math in `vec<N, float>` order,
//! f64 where the C++ uses `double`. The 8x6 downsample matrices and the upsample matrix /
//! gradient descent helpers are not ported (unused by the libktx encoders).

// The float literals keep the C++ spelling (they round to the same f32).
#![allow(clippy::needless_range_loop, clippy::excessive_precision)]

use std::sync::OnceLock;

use super::astc_6x6_hdr::requantize_ise_endpoints;
use super::astc_hdr_downsample_tables::DOWNSAMPLE_MATRICES_6X6;
use super::astc_helpers::{
    self as ah, BISE_3_LEVELS, BISE_12_LEVELS, BISE_16_LEVELS, BISE_256_LEVELS, HalfFloat,
    dequant_tables, get_ise_levels, half_to_float, is_half_inf_or_nan, qlog16_to_half,
};
use super::bc6h::{
    NUM_MODE7_ENDPOINTS, NUM_MODE11_ENDPOINTS, decode_mode7_to_qlog12_s, decode_mode11_to_qlog12,
};

pub const MAX_ASTC_HDR_ENC_BLOCK_PIXELS: usize = 6 * 6;
pub const MODE11_TOTAL_SUBMODES: u32 = 8;
pub const MODE7_TOTAL_SUBMODES: u32 = 6;
pub const MIN_SUPPORTED_ISE_WEIGHT_INDEX: u32 = ah::BISE_2_LEVELS;
pub const MAX_SUPPORTED_ISE_WEIGHT_INDEX: u32 = ah::BISE_32_LEVELS;
pub const MIN_SUPPORTED_WEIGHT_LEVELS: u32 = 2;
pub const MAX_SUPPORTED_WEIGHT_LEVELS: usize = 32;
/// The original UASTC HDR 4x4 log bias.
pub const Q_LOG_BIAS_4X4: f32 = 0.125;
/// The log bias both encoders use now.
pub const Q_LOG_BIAS_6X6: f32 = 1.0;
pub const LDR_TO_HDR_NITS: f32 = 100.0;
pub const BIG_FLOAT_VAL: f32 = 1e30;
pub const MAX_QLOG16_VAL: f32 = 65504.0;
pub const ASTC_HDR_MAX_VAL: f32 = 65216.0;
pub const MAX_HALF_FLOAT: f64 = 65504.0;
pub const FIRST_MODE7_SUBMODE_INDEX: i32 = 0;
pub const MAX_MODE7_SUBMODE_INDEX: i32 = 5;
pub const FIRST_MODE11_SUBMODE_INDEX: i32 = -1;
pub const MAX_MODE11_SUBMODE_INDEX: i32 = 7;

pub const MAX_QLOG7: u32 = 123;
pub const MAX_QLOG8: u32 = 247;
pub const MAX_QLOG9: u32 = 495;
pub const MAX_QLOG10: u32 = 991;
pub const MAX_QLOG11: u32 = 1983;
pub const MAX_QLOG12: u32 = 3967;
pub const MAX_QLOG16: u32 = 63487;

/// `g_ise_weight_lerps`: the first entry is the # of weight levels for that BISE range.
pub const G_ISE_WEIGHT_LERPS: [[u8; 33]; 12] = {
    const fn row(v: &[u8]) -> [u8; 33] {
        let mut r = [0u8; 33];
        let mut i = 0;
        while i < v.len() {
            r[i] = v[i];
            i += 1;
        }
        r
    }
    [
        row(&[2, 0, 64]),
        row(&[3, 0, 32, 64]),
        row(&[4, 0, 21, 43, 64]),
        row(&[5, 0, 16, 32, 48, 64]),
        row(&[6, 0, 64, 12, 52, 25, 39]),
        row(&[8, 0, 9, 18, 27, 37, 46, 55, 64]),
        row(&[10, 0, 64, 7, 57, 14, 50, 21, 43, 28, 36]),
        row(&[12, 0, 64, 17, 47, 5, 59, 23, 41, 11, 53, 28, 36]),
        row(&[
            16, 0, 4, 8, 12, 17, 21, 25, 29, 35, 39, 43, 47, 52, 56, 60, 64,
        ]),
        row(&[
            20, 0, 64, 16, 48, 3, 61, 19, 45, 6, 58, 23, 41, 9, 55, 26, 38, 13, 51, 29, 35,
        ]),
        row(&[
            24, 0, 64, 8, 56, 16, 48, 24, 40, 2, 62, 11, 53, 19, 45, 27, 37, 5, 59, 13, 51, 22, 42,
            30, 34,
        ]),
        row(&[
            32, 0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 34, 36, 38, 40, 42, 44,
            46, 48, 50, 52, 54, 56, 58, 60, 62, 64,
        ]),
    ]
};

const DEF_R_ERROR_SCALE: f32 = 2.0;
const DEF_G_ERROR_SCALE: f32 = 3.0;

/// `astc_hdr_codec_base_options`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AstcHdrCodecBaseOptions {
    pub r_err_scale: f32,
    pub g_err_scale: f32,
    pub q_log_bias: f32,
    pub ultra_quant: bool,
    pub allow_uber_mode: bool,
    pub mode7_full_s_optimization: bool,
    pub take_first_non_clamping_mode11_submode: bool,
    pub take_first_non_clamping_mode7_submode: bool,
    pub disable_weight_plane_optimization: bool,
}

impl Default for AstcHdrCodecBaseOptions {
    fn default() -> Self {
        let mut o = AstcHdrCodecBaseOptions {
            r_err_scale: 0.0,
            g_err_scale: 0.0,
            q_log_bias: 0.0,
            ultra_quant: false,
            allow_uber_mode: false,
            mode7_full_s_optimization: false,
            take_first_non_clamping_mode11_submode: false,
            take_first_non_clamping_mode7_submode: false,
            disable_weight_plane_optimization: false,
        };
        o.init();
        o
    }
}

impl AstcHdrCodecBaseOptions {
    /// `astc_hdr_codec_base_options::init`.
    pub fn init(&mut self) {
        self.r_err_scale = DEF_R_ERROR_SCALE;
        self.g_err_scale = DEF_G_ERROR_SCALE;
        self.q_log_bias = Q_LOG_BIAS_4X4;
        self.ultra_quant = false;
        // Disabling by default to avoid transcoding outliers.
        self.allow_uber_mode = false;
        self.mode7_full_s_optimization = true;
        self.take_first_non_clamping_mode11_submode = false;
        self.take_first_non_clamping_mode7_submode = false;
        self.disable_weight_plane_optimization = true;
    }
}

/// `opt_mode_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptMode {
    NoOpt,
    OrdinaryLeastSquares,
    WeightedLeastSquares,
    WeightedLeastSquaresHeavy,
    WeightedAverage,
}

// ---------------------------------------------------------------------------------------------
// vec3F helpers (`basisu::vec<3, float>` semantics)

pub type Vec3 = [f32; 3];

#[inline]
fn v_add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
fn v_sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn v_mul(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
fn v_dot(a: Vec3, b: Vec3) -> f32 {
    let mut r = a[0] * b[0];
    r += a[1] * b[1];
    r += a[2] * b[2];
    r
}

#[inline]
fn v_length(a: Vec3) -> f32 {
    v_dot(a, a).sqrt()
}

#[inline]
fn v_clamp(a: &mut Vec3, l: f32, h: f32) {
    for c in a.iter_mut() {
        *c = clampf(*c, l, h);
    }
}

/// `basisu::clamp` (`value < low ? low : (value > high ? high : value)`).
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
fn squared_distance_d(a: Vec3, b: Vec3) -> f64 {
    let mut d2 = 0.0f64;
    for i in 0..3 {
        let d = a[i] as f64 - b[i] as f64;
        d2 += d * d;
    }
    d2
}

#[inline]
fn lerpf(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}

/// `aabb3F` (`vec_interval<vec3F>`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb3 {
    pub lo: Vec3,
    pub hi: Vec3,
}

impl Aabb3 {
    /// `cInitExpand`.
    pub fn init_expand() -> Self {
        Aabb3 {
            lo: [1e30; 3],
            hi: [-1e30; 3],
        }
    }

    pub fn new(lo: Vec3, hi: Vec3) -> Self {
        Aabb3 { lo, hi }
    }

    pub fn expand(&mut self, p: Vec3) {
        for c in 0..3 {
            if p[c] < self.lo[c] {
                self.lo[c] = p[c];
            }
            if p[c] > self.hi[c] {
                self.hi[c] = p[c];
            }
        }
    }

    pub fn get_dim(&self, axis: usize) -> f32 {
        self.hi[axis] - self.lo[axis]
    }

    /// `contains` (written like the C++ so NaN components count as inside).
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub fn contains(&self, p: Vec3) -> bool {
        (0..3).all(|i| !(p[i] < self.lo[i]) && !(p[i] > self.hi[i]))
    }
}

/// `intersection::ray_aabb` (Graphics Gems 1); `None` on `cFailure`.
fn ray_aabb(origin: Vec3, dir: Vec3, b: &Aabb3) -> Option<Vec3> {
    const RIGHT: i32 = 0;
    const LEFT: i32 = 1;
    const MIDDLE: i32 = 2;
    let mut inside = true;
    let mut quadrant = [0i32; 3];
    let mut candidate_plane = [0f32; 3];
    for i in 0..3 {
        if origin[i] < b.lo[i] {
            quadrant[i] = LEFT;
            candidate_plane[i] = b.lo[i];
            inside = false;
        } else if origin[i] > b.hi[i] {
            quadrant[i] = RIGHT;
            candidate_plane[i] = b.hi[i];
            inside = false;
        } else {
            quadrant[i] = MIDDLE;
        }
    }
    if inside {
        return Some(origin);
    }
    let mut max_t = [0f32; 3];
    for i in 0..3 {
        max_t[i] = if quadrant[i] != MIDDLE && dir[i] != 0.0 {
            (candidate_plane[i] - origin[i]) / dir[i]
        } else {
            -1.0
        };
    }
    let mut which_plane = 0;
    for i in 1..3 {
        if max_t[which_plane] < max_t[i] {
            which_plane = i;
        }
    }
    if max_t[which_plane] < 0.0 {
        return None;
    }
    let mut coord = [0f32; 3];
    for i in 0..3 {
        if i != which_plane {
            coord[i] = origin[i] + max_t[which_plane] * dir[i];
            if coord[i] < b.lo[i] || coord[i] > b.hi[i] {
                return None;
            }
        } else {
            coord[i] = candidate_plane[i];
        }
    }
    Some(coord)
}

// ---------------------------------------------------------------------------------------------
// Tables

/// `quant_qlog16`.
#[inline]
fn quant_qlog16(q16: u32, desired_bits: u32) -> u32 {
    let shift = 16 - desired_bits;
    let e = (q16 + (1u32 << (shift - 1)) - 1) >> shift;
    let max_val = (1u32 << desired_bits) - 1;
    e.min(max_val)
}

/// Encoder tables built by `astc_hdr_enc_init`.
pub struct AstcHdrEncTables {
    /// `g_half_to_qlog7`, `g_half_to_qlog8`: nearest qlog of a positive half float.
    pub half_to_qlog7: Vec<u16>,
    pub half_to_qlog8: Vec<u16>,
    /// `g_astc_ls_weights_ise[ise_range][i]`: w * w, (1 - w) * w, (1 - w) * (1 - w), w.
    pub ls_weights_ise: [[[f32; 4]; MAX_SUPPORTED_WEIGHT_LEVELS]; 12],
    /// `g_map_astc_to_linear_order[ise_range][astc_index]`.
    pub map_astc_to_linear_order: [[u8; MAX_SUPPORTED_WEIGHT_LEVELS]; 12],
    /// `g_map_linear_to_astc_order[ise_range][linear_index]`.
    pub map_linear_to_astc_order: [[u8; MAX_SUPPORTED_WEIGHT_LEVELS]; 12],
}

/// `compute_half_to_qlog_table`.
fn compute_half_to_qlog_table(bits: u32, qlog16_to_float: &[f32]) -> Vec<u16> {
    let max_val = (1u32 << bits) - 1;
    const FIRST_INVALID_QLOG16_INDEX: u32 = 63488;
    let mut table = vec![0u16; 32768];
    for h in 0..32768u32 {
        if is_half_inf_or_nan(h as HalfFloat) {
            continue;
        }
        let desired_val = half_to_float(h as HalfFloat);
        let mut best_err = BIG_FLOAT_VAL;
        let mut best_qlog = 0u32;
        let mut prev_err = BIG_FLOAT_VAL as f64;
        for i in 0..=max_val {
            let idx = i << (16 - bits);
            if idx >= FIRST_INVALID_QLOG16_INDEX {
                break;
            }
            let v = qlog16_to_float[idx as usize];
            let err = (v - desired_val).abs();
            if err as f64 > prev_err {
                // Every remaining entry will have guaranteed higher error
                break;
            }
            prev_err = err as f64;
            if err < best_err {
                best_err = err;
                best_qlog = i;
                if best_err == 0.0 {
                    break;
                }
            }
        }
        table[h as usize] = best_qlog as u16;
    }
    table
}

/// `astc_hdr_enc_init` (initialized once).
pub fn astc_hdr_enc_tables() -> &'static AstcHdrEncTables {
    static T: OnceLock<AstcHdrEncTables> = OnceLock::new();
    T.get_or_init(|| {
        let qlog16_to_float: Vec<f32> = (0..=65535u32)
            .map(|i| half_to_float(qlog16_to_half(i as i32)))
            .collect();
        let mut t = AstcHdrEncTables {
            half_to_qlog7: compute_half_to_qlog_table(7, &qlog16_to_float),
            half_to_qlog8: compute_half_to_qlog_table(8, &qlog16_to_float),
            ls_weights_ise: [[[0.0; 4]; MAX_SUPPORTED_WEIGHT_LEVELS]; 12],
            map_astc_to_linear_order: [[0; MAX_SUPPORTED_WEIGHT_LEVELS]; 12],
            map_linear_to_astc_order: [[0; MAX_SUPPORTED_WEIGHT_LEVELS]; 12],
        };
        for range in 0..12usize {
            let num_levels = G_ISE_WEIGHT_LERPS[range][0] as usize;
            for i in 0..num_levels {
                let w = G_ISE_WEIGHT_LERPS[range][1 + i] as f32 * (1.0 / 64.0);
                t.ls_weights_ise[range][i] = [w * w, (1.0 - w) * w, (1.0 - w) * (1.0 - w), w];
            }
            let mut s: Vec<u32> = (0..num_levels)
                .map(|i| ((G_ISE_WEIGHT_LERPS[range][1 + i] as u32) << 8) + i as u32)
                .collect();
            s.sort_unstable();
            for i in 0..num_levels {
                t.map_linear_to_astc_order[range][i] = (s[i] & 0xFF) as u8;
            }
            for i in 0..num_levels {
                t.map_astc_to_linear_order[range][t.map_linear_to_astc_order[range][i] as usize] =
                    i as u8;
            }
        }
        t
    })
}

// ---------------------------------------------------------------------------------------------
// Half / qlog helpers

/// `half_float_mantissa_to_qlog11`.
#[inline]
pub fn half_float_mantissa_to_qlog11(hf: i32) -> i32 {
    let q0 = (hf * 8 + 2) / 3;
    let q1 = (hf * 8 + 2048 + 4) / 5;
    if q0 < 512 {
        return q0;
    } else if q1 >= 1536 {
        return q1;
    }
    (hf * 8 + 512 + 2) / 4
}

/// `half_to_qlog16` (positive, finite halves).
#[inline]
pub fn half_to_qlog16(hf: i32) -> i32 {
    let exp = (hf >> 10) & 0x1F;
    let mantissa = half_float_mantissa_to_qlog11(hf & 0x3FF);
    (exp << 11) | mantissa
}

/// `fast_half_to_float_pos_not_inf_or_nan`.
#[inline]
pub fn fast_half_to_float_pos_not_inf_or_nan(h: HalfFloat) -> f32 {
    f32::from_bits((h as u32) << 13) * f32::from_bits(0x7780_0000)
}

/// `float_to_half_non_neg_no_nan_inf`.
pub fn float_to_half_non_neg_no_nan_inf(val: f32) -> HalfFloat {
    let i = val.to_bits() as i32;
    let flt_m = i & 0x7FFFFF;
    let flt_e = (i >> 23) & 0xFF;
    let (mut e, mut m) = (0i32, 0i32);
    // not zero or denormal
    if flt_e != 0 {
        let new_exp = flt_e - 127;
        if new_exp > 15 {
            e = 31;
        } else if new_exp < -14 {
            m = ((1u32 << 24) as f32 * val.abs()).round_ties_even() as i32;
        } else {
            e = new_exp + 15;
            m = (flt_m as f32 * (1.0 / (1u32 << 13) as f32)).round_ties_even() as i32;
        }
    }
    if m == 1024 {
        e += 1;
        m = 0;
    }
    ((e << 10) | m) as HalfFloat
}

/// `q(x, log_bias)`: approx log2 of the half + bias via the float bits.
#[inline]
pub fn q(x: HalfFloat, log_bias: f32) -> f64 {
    (fast_half_to_float_pos_not_inf_or_nan(x) + log_bias).to_bits() as f64
}

/// `q2(x, log_bias)`.
#[inline]
pub fn q2(x: HalfFloat, log_bias: f32) -> u32 {
    (fast_half_to_float_pos_not_inf_or_nan(x) + log_bias).to_bits()
}

/// `get_max_qlog`.
#[inline]
pub fn get_max_qlog(bits: u32) -> u32 {
    match bits {
        7 => MAX_QLOG7,
        8 => MAX_QLOG8,
        9 => MAX_QLOG9,
        10 => MAX_QLOG10,
        11 => MAX_QLOG11,
        12 => MAX_QLOG12,
        16 => MAX_QLOG16,
        _ => 0,
    }
}

#[inline]
fn get_bit(src_val: i32, src_bit: i32) -> i32 {
    (src_val >> src_bit) & 1
}

#[inline]
fn pack_bit(dst: &mut i32, dst_bit: i32, src_val: i32, src_bit: i32) {
    *dst |= get_bit(src_val, src_bit) << dst_bit;
}

// ---------------------------------------------------------------------------------------------
// Statistics and endpoint fitting

/// `calc_mean`.
pub fn calc_mean(num_pixels: usize, pixels: &[[f32; 4]]) -> Vec3 {
    let mut mean = [0f32; 3];
    for p in pixels.iter().take(num_pixels) {
        mean[0] += p[0];
        mean[1] += p[1];
        mean[2] += p[2];
    }
    let n = num_pixels as f32;
    [mean[0] / n, mean[1] / n, mean[2] / n]
}

/// `calc_rgb_pca`.
pub fn calc_rgb_pca(num_pixels: usize, pixels: &[[f32; 4]], mean_color: Vec3) -> Vec3 {
    let mut cov = [0f32; 6];
    for v in pixels.iter().take(num_pixels) {
        let r = v[0] - mean_color[0];
        let g = v[1] - mean_color[1];
        let b = v[2] - mean_color[2];
        cov[0] += r * r;
        cov[1] += r * g;
        cov[2] += r * b;
        cov[3] += g * g;
        cov[4] += g * b;
        cov[5] += b * b;
    }
    let maximumf = |a: f32, b: f32| if a > b { a } else { b };
    let (mut xr, mut xg, mut xb) = (0.9f32, 1.0f32, 0.7f32);
    for _ in 0..3 {
        let mut r = xr * cov[0] + xg * cov[1] + xb * cov[2];
        let mut g = xr * cov[1] + xg * cov[3] + xb * cov[4];
        let mut b = xr * cov[2] + xg * cov[4] + xb * cov[5];
        let mut m = maximumf(maximumf(r.abs(), g.abs()), b.abs());
        if m > 1e-10 {
            m = 1.0 / m;
            r *= m;
            g *= m;
            b *= m;
        }
        xr = r;
        xg = g;
        xb = b;
    }
    let mut len = xr * xr + xg * xg + xb * xb;
    let mut axis = [0.5773502691f32; 3];
    if len >= 1e-10 {
        len = 1.0 / len.sqrt();
        xr *= len;
        xg *= len;
        xb *= len;
        axis = [xr, xg, xb];
    }
    axis
}

/// `encode_astc_block_stats`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EncodeAstcBlockStats {
    pub num_pixels: u32,
    pub mean_q16: Vec3,
    pub axis_q16: Vec3,
}

impl EncodeAstcBlockStats {
    pub fn init(&mut self, num_pixels: u32, pixels_q16: &[[f32; 4]]) {
        self.num_pixels = num_pixels;
        self.mean_q16 = calc_mean(num_pixels as usize, pixels_q16);
        self.axis_q16 = calc_rgb_pca(num_pixels as usize, pixels_q16, self.mean_q16);
    }
}

/// `interp_color`: returns (color, inside).
fn interp_color(
    mean: Vec3,
    dir: Vec3,
    df: f32,
    colorspace_box: &Aabb3,
    input_box: &Aabb3,
) -> (Vec3, bool) {
    let k = v_add(mean, v_mul(dir, df));
    if colorspace_box.contains(k) {
        return (k, true);
    }
    // starts inside, ends outside
    let s = mean;
    let e = v_add(mean, v_mul(dir, df));
    // a ray guaranteed to go from the outside to inside
    let mut d = v_sub(s, e);
    let len = v_length(d);
    if len != 0.0 {
        d = v_mul(d, 1.0 / len);
    }
    let c = ray_aabb(e, d, input_box).unwrap_or(k);
    (c, false)
}

/// Shared tail of the RGB least squares fits: box fallback, color space clipping, clamping.
fn finish_ls_rgb(xl: &mut Vec3, xh: &mut Vec3, input_box: &Aabb3) {
    for c in 0..3 {
        if input_box.get_dim(c) < 0.0000125 {
            xl[c] = input_box.lo[c];
            xh[c] = input_box.hi[c];
        }
    }
    let mean = v_mul(v_add(*xl, *xh), 0.5);
    let mut dir = v_sub(*xh, *xl);
    let ln = v_length(dir);
    if ln != 0.0 {
        dir = [dir[0] / ln, dir[1] / ln, dir[2] / ln];
        let ld = v_dot(v_sub(*xl, mean), dir);
        let hd = v_dot(v_sub(*xh, mean), dir);
        let colorspace_box = Aabb3::new([0.0; 3], [MAX_QLOG16_VAL; 3]);
        let (l, inside1) = interp_color(mean, dir, ld, &colorspace_box, input_box);
        if !inside1 {
            *xl = l;
        }
        let (h, inside2) = interp_color(mean, dir, hd, &colorspace_box, input_box);
        if !inside2 {
            *xh = h;
        }
    }
    v_clamp(xl, 0.0, MAX_QLOG16_VAL);
    v_clamp(xh, 0.0, MAX_QLOG16_VAL);
}

/// Normal equation solve shared by the RGB fits (`z*`, `q00_*`, `t_*` sums).
fn solve_ls_rgb(z00: f32, z10: f32, z11: f32, q00: Vec3, t: Vec3) -> Option<(Vec3, Vec3)> {
    let q10 = [t[0] - q00[0], t[1] - q00[1], t[2] - q00[2]];
    let z01 = z10;
    let mut det = z00 * z11 - z01 * z10;
    if det == 0.0 {
        return None;
    }
    det = 1.0 / det;
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
    Some((xl, xh))
}

/// `compute_least_squares_endpoints_rgb` (all in Q16 space).
pub fn compute_least_squares_endpoints_rgb(
    n: usize,
    selectors: &[u8],
    selector_weights: &[[f32; 4]],
    colors: &[[f32; 4]],
    input_box: &Aabb3,
) -> Option<(Vec3, Vec3)> {
    let (mut z00, mut z10, mut z11) = (0f32, 0f32, 0f32);
    let mut q00 = [0f32; 3];
    let mut t = [0f32; 3];
    for i in 0..n {
        let sw = selector_weights[selectors[i] as usize];
        z00 += sw[0];
        z10 += sw[1];
        z11 += sw[2];
        let w = sw[3];
        for c in 0..3 {
            q00[c] += w * colors[i][c];
            t[c] += colors[i][c];
        }
    }
    let (mut xl, mut xh) = solve_ls_rgb(z00, z10, z11, q00, t)?;
    finish_ls_rgb(&mut xl, &mut xh, input_box);
    Some((xl, xh))
}

/// `compute_least_squares_endpoints_rgb_raw_weights` (raw [0, 64] weights).
pub fn compute_least_squares_endpoints_rgb_raw_weights(
    n: usize,
    raw_weights: &[u8],
    colors: &[[f32; 4]],
    input_box: &Aabb3,
) -> Option<(Vec3, Vec3)> {
    let (mut z00, mut z10, mut z11) = (0f32, 0f32, 0f32);
    let mut q00 = [0f32; 3];
    let mut t = [0f32; 3];
    for i in 0..n {
        let wt = raw_weights[i] as f32 * (1.0 / 64.0);
        z00 += wt * wt;
        z10 += (1.0 - wt) * wt;
        z11 += (1.0 - wt) * (1.0 - wt);
        for c in 0..3 {
            q00[c] += wt * colors[i][c];
            t[c] += colors[i][c];
        }
    }
    let (mut xl, mut xh) = solve_ls_rgb(z00, z10, z11, q00, t)?;
    finish_ls_rgb(&mut xl, &mut xh, input_box);
    Some((xl, xh))
}

/// `compute_weighted_least_squares_endpoints_rgb`: `selectors` + `selector_weights` or
/// `raw_weights` (ti in [0, 1]), emphasis weights wi.
pub fn compute_weighted_least_squares_endpoints_rgb(
    n: usize,
    selectors: Option<(&[u8], &[[f32; 4]])>,
    raw_weights: &[f32],
    emphasis_weights: &[f32],
    colors: &[[f32; 4]],
) -> Option<(Vec3, Vec3)> {
    let ti_of = |i: usize| match selectors {
        Some((s, w)) => w[s[i] as usize][3],
        None => raw_weights[i],
    };
    let mut total_wi = 0f32;
    for &w in emphasis_weights.iter().take(n) {
        total_wi += w;
    }
    if total_wi == 0.0 {
        return None;
    }
    let mut weighted_mean_tw = 0f32;
    let mut weighted_mean_pw = [0f32; 3];
    for i in 0..n {
        let wi = emphasis_weights[i];
        let ti = ti_of(i);
        weighted_mean_tw += wi * ti;
        for c in 0..3 {
            weighted_mean_pw[c] += wi * colors[i][c];
        }
    }
    weighted_mean_tw /= total_wi;
    for c in weighted_mean_pw.iter_mut() {
        *c /= total_wi;
    }
    let mut spt = [0f32; 3];
    let mut stt = 0f32;
    for i in 0..n {
        let wi = emphasis_weights[i];
        let ti = ti_of(i);
        for c in 0..3 {
            spt[c] += wi * (colors[i][c] - weighted_mean_pw[c]) * (ti - weighted_mean_tw);
        }
        let d = ti - weighted_mean_tw;
        stt += wi * (d * d);
    }
    if stt == 0.0 {
        return None;
    }
    let mut xl = [0f32; 3];
    let mut xh = [0f32; 3];
    for i in 0..3 {
        xh[i] = weighted_mean_pw[i] + (spt[i] / stt) * (1.0 - weighted_mean_tw);
        xl[i] = weighted_mean_pw[i] - (spt[i] / stt) * weighted_mean_tw;
    }
    v_clamp(&mut xl, 0.0, MAX_QLOG16_VAL);
    v_clamp(&mut xh, 0.0, MAX_QLOG16_VAL);
    Some((xl, xh))
}

// ---------------------------------------------------------------------------------------------
// Block colors and selector evaluation

/// `interpolate_qlog12_colors`: decoded halves `[n][3]` in ASTC order.
pub fn interpolate_qlog12_colors(
    e: &[[i32; 3]; 2],
    decoded_half: &mut [[HalfFloat; 3]],
    n: usize,
    ise_weight_range: u32,
) {
    for (i, d) in decoded_half.iter_mut().enumerate().take(n) {
        let c = G_ISE_WEIGHT_LERPS[ise_weight_range as usize][1 + i] as u32;
        for j in 0..3 {
            let a0 = (e[0][j] as u32) << 4;
            let a1 = (e[1][j] as u32) << 4;
            let v = (a0 * (64 - c) + a1 * c + 32) / 64;
            d[j] = qlog16_to_half(v as i32);
        }
    }
}

/// `get_astc_hdr_mode_11_block_colors`.
pub fn get_astc_hdr_mode_11_block_colors(
    endpoints: &[u8],
    decoded_half: &mut [[HalfFloat; 3]],
    n: usize,
    ise_weight_range: u32,
    ise_endpoint_range: u32,
) -> bool {
    let mut e = [[0i32; 3]; 2];
    if !decode_mode11_to_qlog12(endpoints, &mut e, ise_endpoint_range) {
        return false;
    }
    interpolate_qlog12_colors(&e, decoded_half, n, ise_weight_range);
    true
}

/// `get_astc_hdr_mode_7_block_colors`.
pub fn get_astc_hdr_mode_7_block_colors(
    endpoints: &[u8],
    decoded_half: &mut [[HalfFloat; 3]],
    n: usize,
    ise_weight_range: u32,
    ise_endpoint_range: u32,
) -> bool {
    let mut e = [[0i32; 3]; 2];
    if !decode_mode7_to_qlog12_s(endpoints, &mut e, None, ise_endpoint_range) {
        return false;
    }
    interpolate_qlog12_colors(&e, decoded_half, n, ise_weight_range);
    true
}

/// `eval_selectors_f`.
fn eval_selectors_f(
    num_pixels: usize,
    weights: &mut [u8],
    block_pixels_half: &[[HalfFloat; 3]],
    num_weight_levels: usize,
    decoded_half: &[[HalfFloat; 3]],
    coptions: &AstcHdrCodecBaseOptions,
    usable_selector_bitmask: u32,
) -> f64 {
    let r_weight = coptions.r_err_scale as f64;
    let g_weight = coptions.g_err_scale as f64;
    let bias = coptions.q_log_bias;
    let mut total_error = 0f64;
    let mut decoded_half_q = [[0f64; 3]; MAX_SUPPORTED_WEIGHT_LEVELS];
    for i in 0..num_weight_levels {
        for c in 0..3 {
            decoded_half_q[i][c] = q(decoded_half[i][c], bias);
        }
    }
    for p in 0..num_pixels {
        let desired = &block_pixels_half[p];
        let dr = q(desired[0], bias);
        let dg = q(desired[1], bias);
        let db = q(desired[2], bias);
        let mut lowest_e = BIG_FLOAT_VAL as f64;
        for i in 0..num_weight_levels {
            if (1u32 << i) & usable_selector_bitmask == 0 {
                continue;
            }
            let rd = decoded_half_q[i][0] - dr;
            let gd = decoded_half_q[i][1] - dg;
            let bd = decoded_half_q[i][2] - db;
            let e = r_weight * (rd * rd) + g_weight * (gd * gd) + bd * bd;
            if e < lowest_e {
                lowest_e = e;
                weights[p] = i as u8;
            }
        }
        total_error += lowest_e;
    }
    total_error
}

/// `eval_selectors`: picks the nearest decoded color per pixel (in ASTC weight order) and
/// returns the total error.
#[allow(clippy::too_many_arguments)]
pub fn eval_selectors(
    num_pixels: usize,
    weights: &mut [u8],
    ise_weight_range: u32,
    block_pixels_half: &[[HalfFloat; 3]],
    num_weight_levels: usize,
    decoded_half: &[[HalfFloat; 3]],
    coptions: &AstcHdrCodecBaseOptions,
    usable_selector_bitmask: u32,
) -> f64 {
    if coptions.r_err_scale != 2.0 || coptions.g_err_scale != 3.0 {
        return eval_selectors_f(
            num_pixels,
            weights,
            block_pixels_half,
            num_weight_levels,
            decoded_half,
            coptions,
            usable_selector_bitmask,
        );
    }
    let bias = coptions.q_log_bias;
    let mut total_error: u64 = 0;
    let mut decoded_half_q = [[0i64; 3]; MAX_SUPPORTED_WEIGHT_LEVELS];
    for i in 0..num_weight_levels {
        for c in 0..3 {
            decoded_half_q[i][c] = q2(decoded_half[i][c], bias) as i64;
        }
    }
    let err5 = |i: usize, d: [i64; 3]| -> i64 {
        let rd = decoded_half_q[i][0] - d[0];
        let gd = decoded_half_q[i][1] - d[1];
        let bd = decoded_half_q[i][2] - d[2];
        ((2 * (rd * rd) + 3 * (gd * gd) + bd * bd) << 5) | i as i64
    };
    let desired_q = |p: usize| -> [i64; 3] {
        let d = &block_pixels_half[p];
        [
            q2(d[0], bias) as i64,
            q2(d[1], bias) as i64,
            q2(d[2], bias) as i64,
        ]
    };

    if usable_selector_bitmask != u32::MAX {
        for p in 0..num_pixels {
            let d = desired_q(p);
            let mut lowest_e = i64::MAX;
            for i in 0..num_weight_levels {
                if (1u32 << i) & usable_selector_bitmask == 0 {
                    continue;
                }
                let rd = decoded_half_q[i][0] - d[0];
                let gd = decoded_half_q[i][1] - d[1];
                let bd = decoded_half_q[i][2] - d[2];
                let e = 2 * (rd * rd) + 3 * (gd * gd) + bd * bd;
                if e < lowest_e {
                    lowest_e = e;
                    weights[p] = i as u8;
                }
            }
            total_error = total_error.wrapping_add(lowest_e as u64);
        }
    } else if num_weight_levels <= 4 || coptions.disable_weight_plane_optimization {
        for p in 0..num_pixels {
            let d = desired_q(p);
            let mut lowest_e = i64::MAX;
            for i in 0..num_weight_levels {
                lowest_e = lowest_e.min(err5(i, d));
            }
            weights[p] = (lowest_e & 31) as u8;
            total_error = total_error.wrapping_add((lowest_e >> 5) as u64);
        }
    } else {
        let tab = &dequant_tables().weight_tab(ise_weight_range).val_to_ise;
        let idx = |v: usize| tab.get(v).copied().unwrap_or(0) as usize;
        let (lo_index, hi_index, mid_index) = (idx(0), idx(64), idx(32));
        let col = |k: usize| -> Vec3 {
            [
                decoded_half[k][0] as f32,
                decoded_half[k][1] as f32,
                decoded_half[k][2] as f32,
            ]
        };
        let low_color = col(lo_index);
        let high_color = col(hi_index);
        let mid_color = col(mid_index);
        let block_dir = v_sub(high_color, low_color);
        for p in 0..num_pixels {
            let desired = &block_pixels_half[p];
            let dq = desired_q(p);
            // Determine which side of the middle plane the point is for a modest gain
            let c = [
                desired[0] as f32 - mid_color[0],
                desired[1] as f32 - mid_color[1],
                desired[2] as f32 - mid_color[2],
            ];
            let dd = v_dot(c, block_dir);
            let (first, high_index) = if dd >= 0.0 {
                (num_weight_levels / 2, num_weight_levels)
            } else {
                (0, num_weight_levels / 2 + 1)
            };
            let mut lowest_e = i64::MAX;
            for i in first..high_index {
                lowest_e = lowest_e.min(err5(i, dq));
            }
            weights[p] = (lowest_e & 31) as u8;
            total_error = total_error.wrapping_add((lowest_e >> 5) as u64);
        }
    }
    total_error as f64
}

/// `compute_block_error`.
pub fn compute_block_error(
    num_pixels: usize,
    orig_block: &[[HalfFloat; 3]],
    packed_block: &[[HalfFloat; 3]],
    coptions: &AstcHdrCodecBaseOptions,
) -> f64 {
    let r_weight = coptions.r_err_scale as f64;
    let g_weight = coptions.g_err_scale as f64;
    let bias = coptions.q_log_bias;
    let mut total_error = 0f64;
    for p in 0..num_pixels {
        let rd = q(orig_block[p][0], bias) - q(packed_block[p][0], bias);
        let gd = q(orig_block[p][1], bias) - q(packed_block[p][1], bias);
        let bd = q(orig_block[p][2], bias) - q(packed_block[p][2], bias);
        total_error += r_weight * (rd * rd) + g_weight * (gd * gd) + bd * bd;
    }
    total_error
}

// ---------------------------------------------------------------------------------------------
// CEM 11 packing

#[inline]
fn compute_clamped_val(
    v: i32,
    l: i32,
    h: i32,
    did_clamp: &mut bool,
    max_clamp_mag: &mut i32,
) -> i32 {
    if v < l {
        *max_clamp_mag = (*max_clamp_mag).max(l - v);
        *did_clamp = true;
        l
    } else if v > h {
        *max_clamp_mag = (*max_clamp_mag).max(v - h);
        *did_clamp = true;
        h
    } else {
        v
    }
}

const S_B_BITS: [u8; 8] = [7, 8, 6, 7, 8, 6, 7, 6];
const S_C_BITS: [u8; 8] = [6, 6, 7, 7, 6, 7, 7, 7];
const S_D_BITS: [u8; 8] = [7, 6, 7, 6, 5, 6, 5, 6];

/// `pack_astc_mode11_submode` with `val_q` already packed to qlog9-qlog12. Returns
/// (did_clamp, max_clamp_mag).
pub fn pack_astc_mode11_submode_q(
    submode: u32,
    endpoints: &mut [u8],
    val_q: &mut [[i32; 3]; 2],
    early_out_if_clamped: bool,
    max_clamp_mag_accept_thresh: i32,
) -> (bool, i32) {
    let s = submode as usize & 7;
    let a_bits = 9 + (submode >> 1);
    let b_bits = S_B_BITS[s] as u32;
    let c_bits = S_C_BITS[s] as u32;
    let d_bits = S_D_BITS[s] as u32;
    let max_a_val = (1i32 << a_bits) - 1;
    let max_b_val = (1i32 << b_bits) - 1;
    let max_c_val = (1i32 << c_bits) - 1;
    let min_d_val = -(1i32 << (d_bits - 1));
    let max_d_val = -min_d_val - 1;

    let (mut highest_q, mut highest_val, mut highest_comp) = (-1i32, 0usize, 0usize);
    for (v, row) in val_q.iter().enumerate() {
        for (c, &x) in row.iter().enumerate() {
            if x > highest_q {
                highest_q = x;
                highest_val = v;
                highest_comp = c;
            }
        }
    }
    let had_tie = val_q[highest_val ^ 1][highest_comp] == highest_q;
    if highest_val != 1 {
        val_q.swap(0, 1);
    }
    if highest_comp != 0 {
        val_q[0].swap(0, highest_comp);
        val_q[1].swap(0, highest_comp);
    }

    // val[1][0] is now guaranteed to be highest
    let (mut best_va, mut best_vb0, mut best_vb1, mut best_vc, mut best_vd0, mut best_vd1) =
        (0, 0, 0, 0, 0, 0);
    let mut best_max_clamp_mag = 0;
    let mut best_did_clamp = false;
    let mut best_dist = u32::MAX;

    for pass in 0..2 {
        let trial_va = val_q[1][0];
        let mut did_clamp = false;
        let mut trial_max_clamp_mag = 0;
        let trial_vb0 = compute_clamped_val(
            trial_va - val_q[1][1],
            0,
            max_b_val,
            &mut did_clamp,
            &mut trial_max_clamp_mag,
        );
        let trial_vb1 = compute_clamped_val(
            trial_va - val_q[1][2],
            0,
            max_b_val,
            &mut did_clamp,
            &mut trial_max_clamp_mag,
        );
        let trial_vc = compute_clamped_val(
            trial_va - val_q[0][0],
            0,
            max_c_val,
            &mut did_clamp,
            &mut trial_max_clamp_mag,
        );
        let trial_vd0 = compute_clamped_val(
            (trial_va - trial_vb0 - trial_vc) - val_q[0][1],
            min_d_val,
            max_d_val,
            &mut did_clamp,
            &mut trial_max_clamp_mag,
        );
        let trial_vd1 = compute_clamped_val(
            (trial_va - trial_vb1 - trial_vc) - val_q[0][2],
            min_d_val,
            max_d_val,
            &mut did_clamp,
            &mut trial_max_clamp_mag,
        );

        if early_out_if_clamped
            && did_clamp
            && trial_max_clamp_mag > max_clamp_mag_accept_thresh
            && (!had_tie || pass == 1)
        {
            return (true, trial_max_clamp_mag);
        }

        let r_e0 = trial_va.clamp(0, max_a_val);
        let r_e1 = (trial_va - trial_vb0).clamp(0, max_a_val);
        let r_e2 = (trial_va - trial_vb1).clamp(0, max_a_val);
        let r_f0 = (trial_va - trial_vc).clamp(0, max_a_val);
        let r_f1 = (trial_va - trial_vb0 - trial_vc - trial_vd0).clamp(0, max_a_val);
        let r_f2 = (trial_va - trial_vb1 - trial_vc - trial_vd1).clamp(0, max_a_val);

        if !did_clamp || !had_tie {
            best_va = trial_va;
            best_vb0 = trial_vb0;
            best_vb1 = trial_vb1;
            best_vc = trial_vc;
            best_vd0 = trial_vd0;
            best_vd1 = trial_vd1;
            best_max_clamp_mag = trial_max_clamp_mag;
            best_did_clamp = did_clamp;
            break;
        }

        // we had a tie and it did clamp, try swapping L/H for a potential slight gain
        let sq = |x: i32| (x * x) as u32;
        let r_dist1 = sq(r_e0 - val_q[1][0])
            .wrapping_add(sq(r_e1 - val_q[1][1]))
            .wrapping_add(sq(r_e2 - val_q[1][2]));
        let r_dist0 = sq(r_f0 - val_q[0][0])
            .wrapping_add(sq(r_f1 - val_q[0][1]))
            .wrapping_add(sq(r_f2 - val_q[0][2]));
        let total_dist = r_dist1.wrapping_add(r_dist0);
        if total_dist < best_dist {
            best_dist = total_dist;
            best_va = trial_va;
            best_vb0 = trial_vb0;
            best_vb1 = trial_vb1;
            best_vc = trial_vc;
            best_vd0 = trial_vd0;
            best_vd1 = trial_vd1;
            best_did_clamp = did_clamp;
        }
        val_q.swap(0, 1);
    }

    // pack bits now
    let (mut v1, mut v2, mut v3, mut v4, mut v5) = (0i32, 0i32, 0i32, 0i32, 0i32);
    let (x0, x1, x2, x3, x4, x5) = match submode {
        0 => (
            get_bit(best_vb0, 6),
            get_bit(best_vb1, 6),
            get_bit(best_vd0, 6),
            get_bit(best_vd1, 6),
            get_bit(best_vd0, 5),
            get_bit(best_vd1, 5),
        ),
        1 => (
            get_bit(best_vb0, 6),
            get_bit(best_vb1, 6),
            get_bit(best_vb0, 7),
            get_bit(best_vb1, 7),
            get_bit(best_vd0, 5),
            get_bit(best_vd1, 5),
        ),
        2 => (
            get_bit(best_va, 9),
            get_bit(best_vc, 6),
            get_bit(best_vd0, 6),
            get_bit(best_vd1, 6),
            get_bit(best_vd0, 5),
            get_bit(best_vd1, 5),
        ),
        3 => (
            get_bit(best_vb0, 6),
            get_bit(best_vb1, 6),
            get_bit(best_va, 9),
            get_bit(best_vc, 6),
            get_bit(best_vd0, 5),
            get_bit(best_vd1, 5),
        ),
        4 => (
            get_bit(best_vb0, 6),
            get_bit(best_vb1, 6),
            get_bit(best_vb0, 7),
            get_bit(best_vb1, 7),
            get_bit(best_va, 9),
            get_bit(best_va, 10),
        ),
        5 => (
            get_bit(best_va, 9),
            get_bit(best_va, 10),
            get_bit(best_vc, 7),
            get_bit(best_vc, 6),
            get_bit(best_vd0, 5),
            get_bit(best_vd1, 5),
        ),
        6 => (
            get_bit(best_vb0, 6),
            get_bit(best_vb1, 6),
            get_bit(best_va, 11),
            get_bit(best_vc, 6),
            get_bit(best_va, 9),
            get_bit(best_va, 10),
        ),
        7 => (
            get_bit(best_va, 9),
            get_bit(best_va, 10),
            get_bit(best_va, 11),
            get_bit(best_vc, 6),
            get_bit(best_vd0, 5),
            get_bit(best_vd1, 5),
        ),
        _ => (0, 0, 0, 0, 0, 0),
    };
    let sm = submode as i32;
    let hc = highest_comp as i32;
    // write mode
    pack_bit(&mut v1, 7, sm, 0);
    pack_bit(&mut v2, 7, sm, 1);
    pack_bit(&mut v3, 7, sm, 2);
    // highest component
    pack_bit(&mut v4, 7, hc, 0);
    pack_bit(&mut v5, 7, hc, 1);
    // write bit 8 of va
    pack_bit(&mut v1, 6, best_va, 8);
    // extra bits
    pack_bit(&mut v2, 6, x0, 0);
    pack_bit(&mut v3, 6, x1, 0);
    pack_bit(&mut v4, 6, x2, 0);
    pack_bit(&mut v5, 6, x3, 0);
    pack_bit(&mut v4, 5, x4, 0);
    pack_bit(&mut v5, 5, x5, 0);

    let v0 = best_va & 0xFF;
    v1 |= best_vc & 63;
    v2 |= best_vb0 & 63;
    v3 |= best_vb1 & 63;
    v4 |= best_vd0 & 31;
    v5 |= best_vd1 & 31;

    for (d, v) in endpoints.iter_mut().zip([v0, v1, v2, v3, v4, v5]) {
        *d = v as u8;
    }
    (best_did_clamp, best_max_clamp_mag)
}

/// `pack_astc_mode11_submode` from Q16 endpoints. Returns (did_clamp, max_clamp_mag).
pub fn pack_astc_mode11_submode(
    submode: u32,
    endpoints: &mut [u8],
    low_q16: Vec3,
    high_q16: Vec3,
    early_out_if_clamped: bool,
    max_clamp_mag_accept_thresh: i32,
) -> (bool, i32) {
    let a_bits = 9 + (submode >> 1);
    let max_a_val = (1u32 << a_bits) - 1;
    // The maximum usable value before it turns to NaN/Inf
    let max_a_qlog = get_max_qlog(a_bits);
    let mut val_q = [[0i32; 3]; 2];
    for c in 0..3 {
        let mut l = quant_qlog16(low_q16[c].round() as u32, a_bits);
        let mut h = quant_qlog16(high_q16[c].round() as u32, a_bits);
        if l == h {
            if low_q16[c] < high_q16[c] {
                l = l.saturating_sub(1);
                if h != max_a_val {
                    h += 1;
                }
            } else {
                if l != max_a_val {
                    l += 1;
                }
                h = h.saturating_sub(1);
            }
        }
        val_q[0][c] = l.min(max_a_qlog) as i32;
        val_q[1][c] = h.min(max_a_qlog) as i32;
    }
    pack_astc_mode11_submode_q(
        submode,
        endpoints,
        &mut val_q,
        early_out_if_clamped,
        max_clamp_mag_accept_thresh,
    )
}

/// `(int)std::round(v)` clamped to [0, 65535].
#[inline]
fn round_clamp_u16(v: f32) -> i32 {
    (v.round() as i32).clamp(0, 65535)
}

/// `pack_astc_mode11_direct`.
pub fn pack_astc_mode11_direct(endpoints: &mut [u8], mut l_q16: Vec3, mut h_q16: Vec3) {
    let t = astc_hdr_enc_tables();
    let lg = v_dot(l_q16, [1.0; 3]);
    let hg = v_dot(h_q16, [1.0; 3]);
    if lg > hg {
        // Ensure low endpoint is generally less bright than high in direct mode.
        std::mem::swap(&mut l_q16, &mut h_q16);
    }
    for i in 0..3 {
        // TODO: This goes from QLOG16->HALF->QLOG8/7
        let l_half = qlog16_to_half(round_clamp_u16(l_q16[i])) as usize;
        let h_half = qlog16_to_half(round_clamp_u16(h_q16[i])) as usize;
        let (tab, m) = if i == 2 {
            (&t.half_to_qlog7, MAX_QLOG7)
        } else {
            (&t.half_to_qlog8, MAX_QLOG8)
        };
        let mut l_q = (tab.get(l_half).copied().unwrap_or(0) as u32).min(m) as i32;
        let mut h_q = (tab.get(h_half).copied().unwrap_or(0) as u32).min(m) as i32;
        if l_q == h_q {
            let m = m as i32;
            if l_q16[i] <= h_q16[i] {
                if l_q != 0 {
                    l_q -= 1;
                }
                if h_q != m {
                    h_q += 1;
                }
            } else {
                if h_q != 0 {
                    h_q -= 1;
                }
                if l_q != m {
                    l_q += 1;
                }
            }
        }
        if i == 2 {
            l_q |= 128;
            h_q |= 128;
        }
        endpoints[2 * i] = l_q as u8;
        endpoints[2 * i + 1] = h_q as u8;
    }
}

// ---------------------------------------------------------------------------------------------
// CEM 7 packing

/// `pack_astc_mode7_submode`. Returns (did_clamp, max_clamp_mag).
pub fn pack_astc_mode7_submode(
    submode: u32,
    endpoints: &mut [u8],
    rgb_q16: Vec3,
    s_q16: f32,
    ise_weight_range: u32,
    early_out_if_clamped: bool,
    max_clamp_mag_accept_thresh: i32,
) -> (bool, i32) {
    const S_R_BITS: [u8; 6] = [11, 11, 10, 9, 8, 7];
    const S_G_B_BITS: [u8; 6] = [5, 6, 5, 6, 7, 7];
    const S_S_BITS: [u8; 6] = [7, 5, 8, 7, 6, 7];
    let sm = (submode as usize).min(5);
    let mut max_clamp_mag = 0;

    // The precision of the components
    let prec_bits = S_R_BITS[sm] as u32;
    let mut qlog = [0i32; 4];
    let mut pack_bits = [0i32; 4];
    for i in 0..4 {
        let f = if i == 3 { s_q16 } else { rgb_q16[i] };
        pack_bits[i] = if i == 0 {
            S_R_BITS[sm] as i32
        } else if i == 3 {
            S_S_BITS[sm] as i32
        } else {
            S_G_B_BITS[sm] as i32
        };
        let mut v = quant_qlog16(
            (f.round() as i32).clamp(0, MAX_QLOG16 as i32) as u32,
            prec_bits,
        );
        // Only bias if there are enough texel weights, 4=6 weights
        if ise_weight_range >= 4 {
            // Explictly bias the high color, and the scale up, to better exploit the weights.
            let max_val = (1u32 << prec_bits) - 1;
            const K: u32 = 3;
            v = if i == 3 {
                (v + K * 2).min(max_val)
            } else {
                (v + K).min(max_val)
            };
        }
        if i != 3 {
            v = v.min(get_max_qlog(prec_bits));
        }
        // If S=0, we lose freedom for the texel weights to add any value.
        if i == 3 && v == 0 {
            v = 1;
        }
        qlog[i] = v as i32;
    }

    let mut maj_index = 0usize;
    let mut did_clamp = false;
    if submode != 5 {
        let mut largest_qlog = 0;
        for (i, &v) in qlog.iter().enumerate().take(3) {
            if v > largest_qlog {
                largest_qlog = v;
                maj_index = i;
            }
        }
        if maj_index != 0 {
            qlog.swap(0, maj_index);
        }
        qlog[1] = qlog[0] - qlog[1];
        qlog[2] = qlog[0] - qlog[2];
        for i in 1..4 {
            let max_val = (1i32 << pack_bits[i]) - 1;
            if qlog[i] > max_val {
                max_clamp_mag = max_clamp_mag.max(qlog[i] - max_val);
                qlog[i] = max_val;
                did_clamp = true;
                if early_out_if_clamped && max_clamp_mag > max_clamp_mag_accept_thresh {
                    return (true, max_clamp_mag);
                }
            }
        }
    }

    let mi = maj_index as i32;
    let (mode, x0, x1, x2, x3, x4, x5, x6) = match submode {
        0 => (
            mi << 2,
            get_bit(qlog[0], 9),
            get_bit(qlog[0], 8),
            get_bit(qlog[0], 7),
            get_bit(qlog[0], 10),
            get_bit(qlog[0], 6),
            get_bit(qlog[3], 6),
            get_bit(qlog[3], 5),
        ),
        1 => (
            (mi << 2) | 1,
            get_bit(qlog[0], 8),
            get_bit(qlog[1], 5),
            get_bit(qlog[0], 7),
            get_bit(qlog[2], 5),
            get_bit(qlog[0], 6),
            get_bit(qlog[0], 10),
            get_bit(qlog[0], 9),
        ),
        2 => (
            (mi << 2) | 2,
            get_bit(qlog[0], 9),
            get_bit(qlog[0], 8),
            get_bit(qlog[0], 7),
            get_bit(qlog[0], 6),
            get_bit(qlog[3], 7),
            get_bit(qlog[3], 6),
            get_bit(qlog[3], 5),
        ),
        3 => (
            (mi << 2) | 3,
            get_bit(qlog[0], 8),
            get_bit(qlog[1], 5),
            get_bit(qlog[0], 7),
            get_bit(qlog[2], 5),
            get_bit(qlog[0], 6),
            get_bit(qlog[3], 6),
            get_bit(qlog[3], 5),
        ),
        4 => (
            mi | 0xC,
            get_bit(qlog[1], 6),
            get_bit(qlog[1], 5),
            get_bit(qlog[2], 6),
            get_bit(qlog[2], 5),
            get_bit(qlog[0], 6),
            get_bit(qlog[0], 7),
            get_bit(qlog[3], 5),
        ),
        5 => (
            0xF,
            get_bit(qlog[1], 6),
            get_bit(qlog[1], 5),
            get_bit(qlog[2], 6),
            get_bit(qlog[2], 5),
            get_bit(qlog[0], 6),
            get_bit(qlog[3], 6),
            get_bit(qlog[3], 5),
        ),
        _ => (0, 0, 0, 0, 0, 0, 0, 0),
    };
    let r = qlog[0] & 63;
    let g = qlog[1] & 31;
    let b = qlog[2] & 31;
    let s = qlog[3] & 31;
    endpoints[0] = ((get_bit(mode, 1) << 7) | (get_bit(mode, 0) << 6) | r) as u8;
    endpoints[1] = ((get_bit(mode, 2) << 7) | (x0 << 6) | (x1 << 5) | g) as u8;
    endpoints[2] = ((get_bit(mode, 3) << 7) | (x2 << 6) | (x3 << 5) | b) as u8;
    endpoints[3] = ((x4 << 7) | (x5 << 6) | (x6 << 5) | s) as u8;
    (did_clamp, max_clamp_mag)
}

// ---------------------------------------------------------------------------------------------
// try_mode11 / try_mode7

/// `ultra_quant` endpoint variation shared by the mode 7 / 11 searches: tries +/-1 ISE rank on
/// each endpoint value and keeps the variant with the lowest `dist`.
fn ultra_quant_vary<const N: usize>(
    best: &mut [u8; N],
    best_dist: &mut f64,
    ise_endpoint_range: u32,
    dist_of: impl Fn(&[u8; N]) -> Option<f64>,
) {
    let tab = dequant_tables().endpoint_tab(ise_endpoint_range);
    let levels = get_ise_levels(ise_endpoint_range) as i32;
    let orig = *best;
    for c in 0..N {
        for dt in 0..=1 {
            let d = if dt != 0 { 1 } else { -1 };
            let mut varied = orig;
            let ise = varied[c] as usize;
            let rank = tab.ise_to_rank.get(ise).copied().unwrap_or(0) as i32;
            let rank = (rank + d).clamp(0, levels - 1);
            varied[c] = tab.rank_to_ise.get(rank as usize).copied().unwrap_or(0);
            let Some(dist) = dist_of(&varied) else {
                continue;
            };
            if dist < *best_dist {
                *best_dist = dist;
                *best = varied;
            }
        }
    }
}

fn q16_of(e: &[i32; 3]) -> Vec3 {
    [(e[0] << 4) as f32, (e[1] << 4) as f32, (e[2] << 4) as f32]
}

fn mode11_dist(endpoints: &[u8], ise_endpoint_range: u32, low: Vec3, high: Vec3) -> Option<f64> {
    let mut e = [[0i32; 3]; 2];
    if !decode_mode11_to_qlog12(endpoints, &mut e, ise_endpoint_range) {
        return None;
    }
    let e0 = q16_of(&e[0]);
    let e1 = q16_of(&e[1]);
    let dist0 = squared_distance_d(e0, low) + squared_distance_d(e1, high);
    let dist1 = squared_distance_d(e1, low) + squared_distance_d(e0, high);
    Some(dist0.min(dist1))
}

/// Submode search shared by `pack_mode11` and `try_mode11`: (endpoints, dist, submode).
#[allow(clippy::too_many_arguments)]
fn search_mode11(
    low_color_q16: Vec3,
    high_color_q16: Vec3,
    ise_endpoint_range: u32,
    coptions: &AstcHdrCodecBaseOptions,
    direct_only: bool,
    mut first_submode: i32,
    mut last_submode: i32,
    ignore_clamping: bool,
) -> ([u8; NUM_MODE11_ENDPOINTS], f64, i32) {
    if direct_only {
        first_submode = -1;
        last_submode = -1;
    }
    let mut best_trial_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
    let mut best_trial_dist = BIG_FLOAT_VAL as f64;
    let mut best_submode = 0;
    let mut submode = last_submode;
    while submode >= first_submode {
        let mut orig_trial_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
        let mut did_clamp = false;
        if submode == -1 {
            // If it had to clamp with one of the submodes, try direct which can't clamp.
            pack_astc_mode11_direct(&mut orig_trial_endpoints, low_color_q16, high_color_q16);
        } else {
            const MAX_CLAMP_MAG_ACCEPT_THRESH: i32 = 32;
            let (dc, max_clamp_mag) = pack_astc_mode11_submode(
                submode as u32,
                &mut orig_trial_endpoints,
                low_color_q16,
                high_color_q16,
                !ignore_clamping,
                MAX_CLAMP_MAG_ACCEPT_THRESH,
            );
            did_clamp = dc;
            if !ignore_clamping && did_clamp && max_clamp_mag > MAX_CLAMP_MAG_ACCEPT_THRESH {
                submode -= 1;
                continue;
            }
        }
        let mut trial_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
        requantize_ise_endpoints(
            11,
            BISE_256_LEVELS,
            &orig_trial_endpoints,
            ise_endpoint_range,
            &mut trial_endpoints,
        );
        if let Some(dist) = mode11_dist(
            &trial_endpoints,
            ise_endpoint_range,
            low_color_q16,
            high_color_q16,
        ) {
            if dist < best_trial_dist {
                best_trial_dist = dist;
                best_submode = submode;
                best_trial_endpoints = trial_endpoints;
            }
            if coptions.take_first_non_clamping_mode11_submode && !did_clamp {
                break;
            }
        }
        submode -= 1;
    }
    if coptions.ultra_quant
        && ise_endpoint_range < BISE_256_LEVELS
        && best_trial_dist != BIG_FLOAT_VAL as f64
    {
        ultra_quant_vary(
            &mut best_trial_endpoints,
            &mut best_trial_dist,
            ise_endpoint_range,
            |v| mode11_dist(v, ise_endpoint_range, low_color_q16, high_color_q16),
        );
    }
    (best_trial_endpoints, best_trial_dist, best_submode)
}

/// `pack_mode11`: returns `Some(submode_used)` if any submode packed.
#[allow(clippy::too_many_arguments)]
pub fn pack_mode11(
    low_color_q16: Vec3,
    high_color_q16: Vec3,
    ise_endpoint_range: u32,
    endpoints: &mut [u8],
    coptions: &AstcHdrCodecBaseOptions,
    direct_only: bool,
    first_submode: i32,
    last_submode: i32,
    ignore_clamping: bool,
) -> Option<u32> {
    let (best, dist, best_submode) = search_mode11(
        low_color_q16,
        high_color_q16,
        ise_endpoint_range,
        coptions,
        direct_only,
        first_submode,
        last_submode,
        ignore_clamping,
    );
    endpoints[..NUM_MODE11_ENDPOINTS].copy_from_slice(&best);
    if dist != BIG_FLOAT_VAL as f64 {
        Some((best_submode + 1) as u32)
    } else {
        None
    }
}

/// `try_mode11`: packs the endpoints, evaluates the selectors and keeps the result if it
/// lowers `cur_block_error`.
#[allow(clippy::too_many_arguments)]
pub fn try_mode11(
    num_pixels: usize,
    endpoints: &mut [u8],
    weights: &mut [u8],
    cur_block_error: &mut f64,
    submode_used: &mut u32,
    low_color_q16: Vec3,
    high_color_q16: Vec3,
    block_pixels_half: &[[HalfFloat; 3]],
    num_weight_levels: usize,
    ise_weight_range: u32,
    coptions: &AstcHdrCodecBaseOptions,
    direct_only: bool,
    ise_endpoint_range: u32,
    constrain_ise_weight_selectors: bool,
    first_submode: i32,
    last_submode: i32,
    ignore_clamping: bool,
) -> bool {
    let (best_trial_endpoints, best_trial_dist, best_submode) = search_mode11(
        low_color_q16,
        high_color_q16,
        ise_endpoint_range,
        coptions,
        direct_only,
        first_submode,
        last_submode,
        ignore_clamping,
    );
    if best_trial_dist == BIG_FLOAT_VAL as f64 {
        return false;
    }
    let mut decoded_half = [[0 as HalfFloat; 3]; MAX_SUPPORTED_WEIGHT_LEVELS];
    if !get_astc_hdr_mode_11_block_colors(
        &best_trial_endpoints,
        &mut decoded_half,
        num_weight_levels,
        ise_weight_range,
        ise_endpoint_range,
    ) {
        return false;
    }
    let mut usable_selector_bitmask = u32::MAX;
    if constrain_ise_weight_selectors && ise_weight_range == BISE_16_LEVELS {
        usable_selector_bitmask = (1 << 0)
            | (1 << 1)
            | (1 << 4)
            | (1 << 5)
            | (1 << 10)
            | (1 << 11)
            | (1 << 14)
            | (1 << 15);
    } else if constrain_ise_weight_selectors && ise_weight_range == BISE_12_LEVELS {
        usable_selector_bitmask = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3);
    }
    let mut trial_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let trial_blk_error = eval_selectors(
        num_pixels,
        &mut trial_weights,
        ise_weight_range,
        block_pixels_half,
        num_weight_levels,
        &decoded_half,
        coptions,
        usable_selector_bitmask,
    );
    if trial_blk_error < *cur_block_error {
        *cur_block_error = trial_blk_error;
        endpoints[..NUM_MODE11_ENDPOINTS].copy_from_slice(&best_trial_endpoints);
        weights[..num_pixels].copy_from_slice(&trial_weights[..num_pixels]);
        *submode_used = (best_submode + 1) as u32;
        return true;
    }
    false
}

fn mode7_dist(endpoints: &[u8], ise_endpoint_range: u32, high: Vec3, s_q16: f32) -> Option<f64> {
    let mut e = [[0i32; 3]; 2];
    let mut decoded_s = 0;
    if !decode_mode7_to_qlog12_s(endpoints, &mut e, Some(&mut decoded_s), ise_endpoint_range) {
        return None;
    }
    // e1 is always the high color
    let e1 = q16_of(&e[1]);
    decoded_s <<= 4;
    let ds = decoded_s as f64 - s_q16 as f64;
    Some(squared_distance_d(e1, high) + (ds * ds) * 3.0)
}

/// Submode search shared by `pack_mode7` and `try_mode7`: (endpoints, dist, submode).
#[allow(clippy::too_many_arguments)]
fn search_mode7(
    high_color_q16: Vec3,
    s_q16: f32,
    ise_endpoint_range: u32,
    ise_weight_range: u32,
    coptions: &AstcHdrCodecBaseOptions,
    first_submode: i32,
    last_submode: i32,
    ignore_clamping: bool,
) -> ([u8; NUM_MODE7_ENDPOINTS], f64, i32) {
    let mut best_trial_endpoints = [0u8; NUM_MODE7_ENDPOINTS];
    let mut best_trial_dist = BIG_FLOAT_VAL as f64;
    let mut best_trial_submode = 0;
    for submode in first_submode..=last_submode {
        const MAX_CLAMP_MAG_ACCEPT_THRESH: i32 = 16;
        let mut unquant_trial_endpoints = [0u8; NUM_MODE7_ENDPOINTS];
        let (did_clamp, max_clamp_mag) = pack_astc_mode7_submode(
            submode as u32,
            &mut unquant_trial_endpoints,
            high_color_q16,
            s_q16,
            ise_weight_range,
            !ignore_clamping,
            MAX_CLAMP_MAG_ACCEPT_THRESH,
        );
        if submode < 5
            && !ignore_clamping
            && did_clamp
            && max_clamp_mag > MAX_CLAMP_MAG_ACCEPT_THRESH
        {
            continue;
        }
        let mut trial_endpoints = [0u8; NUM_MODE7_ENDPOINTS];
        requantize_ise_endpoints(
            7,
            BISE_256_LEVELS,
            &unquant_trial_endpoints,
            ise_endpoint_range,
            &mut trial_endpoints,
        );
        let Some(dist) = mode7_dist(&trial_endpoints, ise_endpoint_range, high_color_q16, s_q16)
        else {
            continue;
        };
        if dist < best_trial_dist {
            best_trial_dist = dist;
            best_trial_submode = submode;
            best_trial_endpoints = trial_endpoints;
        }
        if coptions.take_first_non_clamping_mode7_submode && !did_clamp {
            break;
        }
    }
    if coptions.ultra_quant
        && ise_endpoint_range < BISE_256_LEVELS
        && best_trial_dist != BIG_FLOAT_VAL as f64
    {
        ultra_quant_vary(
            &mut best_trial_endpoints,
            &mut best_trial_dist,
            ise_endpoint_range,
            |v| mode7_dist(v, ise_endpoint_range, high_color_q16, s_q16),
        );
    }
    (best_trial_endpoints, best_trial_dist, best_trial_submode)
}

/// `pack_mode7`: returns `Some(submode_used)` if any submode packed.
#[allow(clippy::too_many_arguments)]
pub fn pack_mode7(
    high_color_q16: Vec3,
    s_q16: f32,
    ise_endpoint_range: u32,
    endpoints: &mut [u8],
    ise_weight_range: u32,
    coptions: &AstcHdrCodecBaseOptions,
    first_submode: i32,
    last_submode: i32,
    ignore_clamping: bool,
) -> Option<u32> {
    let (best, dist, sm) = search_mode7(
        high_color_q16,
        s_q16,
        ise_endpoint_range,
        ise_weight_range,
        coptions,
        first_submode,
        last_submode,
        ignore_clamping,
    );
    endpoints[..NUM_MODE7_ENDPOINTS].copy_from_slice(&best);
    if dist != BIG_FLOAT_VAL as f64 {
        Some(sm as u32)
    } else {
        None
    }
}

/// `try_mode7` (always early outs on clamping).
#[allow(clippy::too_many_arguments)]
pub fn try_mode7(
    num_pixels: usize,
    endpoints: &mut [u8],
    weights: &mut [u8],
    cur_block_error: &mut f64,
    submode_used: &mut u32,
    high_color_q16: Vec3,
    s_q16: f32,
    block_pixels_half: &[[HalfFloat; 3]],
    num_weight_levels: usize,
    ise_weight_range: u32,
    coptions: &AstcHdrCodecBaseOptions,
    ise_endpoint_range: u32,
    first_submode: i32,
    last_submode: i32,
) -> bool {
    let (best_trial_endpoints, best_trial_dist, best_trial_submode) = search_mode7(
        high_color_q16,
        s_q16,
        ise_endpoint_range,
        ise_weight_range,
        coptions,
        first_submode,
        last_submode,
        false,
    );
    if best_trial_dist == BIG_FLOAT_VAL as f64 {
        return false;
    }
    let mut decoded_half = [[0 as HalfFloat; 3]; MAX_SUPPORTED_WEIGHT_LEVELS];
    let mut trial_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    if !get_astc_hdr_mode_7_block_colors(
        &best_trial_endpoints,
        &mut decoded_half,
        num_weight_levels,
        ise_weight_range,
        ise_endpoint_range,
    ) {
        return false;
    }
    let trial_blk_error = eval_selectors(
        num_pixels,
        &mut trial_weights,
        ise_weight_range,
        block_pixels_half,
        num_weight_levels,
        &decoded_half,
        coptions,
        u32::MAX,
    );
    if trial_blk_error < *cur_block_error {
        *cur_block_error = trial_blk_error;
        endpoints[..NUM_MODE7_ENDPOINTS].copy_from_slice(&best_trial_endpoints);
        weights[..num_pixels].copy_from_slice(&trial_weights[..num_pixels]);
        *submode_used = best_trial_submode as u32;
        return true;
    }
    false
}

// ---------------------------------------------------------------------------------------------
// Block encoders

const LOW_EMPHASIS_WEIGHT: f32 = 1.0;
const MIDDLE_EMPHASIS_WEIGHT: f32 = 1.25;
const HIGH_EMPHASIS_WEIGHT: f32 = 1.0;
const LOW_EMPHASIS_WEIGHT_HEAVY: f32 = 1.0;
const MIDDLE_EMPHASIS_WEIGHT_HEAVY: f32 = 4.0;
const HIGH_EMPHASIS_WEIGHT_HEAVY: f32 = 1.0;

#[inline]
fn xyz(p: &[f32; 4]) -> Vec3 {
    [p[0], p[1], p[2]]
}

/// `encode_astc_hdr_block_mode_11`. Returns the (possibly improved) block error; the endpoints
/// and weights are only written when it improves on `cur_block_error`.
#[allow(clippy::too_many_arguments)]
pub fn encode_astc_hdr_block_mode_11(
    num_pixels: usize,
    block_pixels_half: &[[HalfFloat; 3]],
    block_pixels_q16: &[[f32; 4]],
    ise_weight_range: u32,
    best_submode: &mut u32,
    mut cur_block_error: f64,
    blk_endpoints: &mut [u8],
    blk_weights: &mut [u8],
    coptions: &AstcHdrCodecBaseOptions,
    direct_only: bool,
    ise_endpoint_range: u32,
    uber_mode: bool,
    constrain_ise_weight_selectors: bool,
    first_submode: i32,
    last_submode: i32,
    ignore_clamping: bool,
    opt_mode: OptMode,
    block_stats: Option<&EncodeAstcBlockStats>,
) -> f64 {
    let t = astc_hdr_enc_tables();
    *best_submode = 0;
    let num_weight_levels = get_ise_levels(ise_weight_range) as usize;
    let wr = ise_weight_range as usize;

    let (block_mean_color_q16, block_axis_q16) = match block_stats {
        Some(s) => (s.mean_q16, s.axis_q16),
        None => {
            let m = calc_mean(num_pixels, block_pixels_q16);
            (m, calc_rgb_pca(num_pixels, block_pixels_q16, m))
        }
    };

    let mut color_box_q16 = Aabb3::init_expand();
    let mut l = BIG_FLOAT_VAL;
    let mut h = -BIG_FLOAT_VAL;
    let mut low_color_q16 = [0f32; 3];
    let mut high_color_q16 = [0f32; 3];
    for p in block_pixels_q16.iter().take(num_pixels) {
        color_box_q16.expand(xyz(p));
        let k = v_sub(xyz(p), block_mean_color_q16);
        let kd = v_dot(k, block_axis_q16);
        if kd < l {
            l = kd;
            low_color_q16 = xyz(p);
        }
        if kd > h {
            h = kd;
            high_color_q16 = xyz(p);
        }
    }
    let (old_low, old_high) = (low_color_q16, high_color_q16);
    for i in 0..3 {
        low_color_q16[i] = lerpf(old_low[i], old_high[i], 1.0 / 64.0);
        high_color_q16[i] = lerpf(old_low[i], old_high[i], 63.0 / 64.0);
    }

    let mut trial_blk_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
    let mut trial_blk_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let mut trial_best_submode = 0u32;
    let mut trial_blk_error = BIG_FLOAT_VAL as f64;

    let did_improve = try_mode11(
        num_pixels,
        &mut trial_blk_endpoints,
        &mut trial_blk_weights,
        &mut trial_blk_error,
        &mut trial_best_submode,
        low_color_q16,
        high_color_q16,
        block_pixels_half,
        num_weight_levels,
        ise_weight_range,
        coptions,
        direct_only,
        ise_endpoint_range,
        constrain_ise_weight_selectors,
        first_submode,
        last_submode,
        ignore_clamping,
    );
    // If we couldn't find ANY usable solution due to endpoint quantization, just return.
    if !did_improve {
        return cur_block_error;
    }
    if trial_blk_error < cur_block_error {
        cur_block_error = trial_blk_error;
        blk_endpoints[..NUM_MODE11_ENDPOINTS].copy_from_slice(&trial_blk_endpoints);
        blk_weights[..num_pixels].copy_from_slice(&trial_blk_weights[..num_pixels]);
        *best_submode = trial_best_submode;
    }
    if opt_mode == OptMode::NoOpt {
        return cur_block_error;
    }

    // least squares on the most promising trial weight indices found
    const NUM_LS_PASSES: u32 = 3;
    let mut emphasis_weights = [0f32; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let ls_weights = &t.ls_weights_ise[wr];

    let try11 = |blk_endpoints: &mut [u8],
                 blk_weights: &mut [u8],
                 cur: &mut f64,
                 bs: &mut u32,
                 lo: Vec3,
                 hi: Vec3| {
        try_mode11(
            num_pixels,
            blk_endpoints,
            blk_weights,
            cur,
            bs,
            lo,
            hi,
            block_pixels_half,
            num_weight_levels,
            ise_weight_range,
            coptions,
            direct_only,
            ise_endpoint_range,
            constrain_ise_weight_selectors,
            first_submode,
            last_submode,
            ignore_clamping,
        )
    };

    if opt_mode == OptMode::WeightedAverage {
        const NUM_OPT_PASSES: u32 = 3;
        for _ in 0..NUM_OPT_PASSES {
            let mut low_p = [0f32; 3];
            let mut total_low = 0f32;
            let mut high_p = [0f32; 3];
            let mut total_high = 0f32;
            for i in 0..num_pixels {
                let p = xyz(&block_pixels_q16[i]);
                let lerp =
                    G_ISE_WEIGHT_LERPS[wr][trial_blk_weights[i] as usize + 1] as f32 * (1.0 / 64.0);
                low_p = v_add(low_p, v_mul(p, 1.0 - lerp));
                total_low += 1.0 - lerp;
                high_p = v_add(high_p, v_mul(p, lerp));
                total_high += lerp;
            }
            if total_low != 0.0 {
                low_p = v_mul(low_p, 1.0 / total_low);
            }
            if total_high != 0.0 {
                high_p = v_mul(high_p, 1.0 / total_high);
            }
            if !try11(
                blk_endpoints,
                blk_weights,
                &mut cur_block_error,
                best_submode,
                low_p,
                high_p,
            ) {
                break;
            }
            trial_blk_weights[..num_pixels].copy_from_slice(&blk_weights[..num_pixels]);
        }
    } else if opt_mode == OptMode::OrdinaryLeastSquares {
        for _ in 0..NUM_LS_PASSES {
            let Some((l_q16, h_q16)) = compute_least_squares_endpoints_rgb(
                num_pixels,
                &trial_blk_weights,
                ls_weights,
                block_pixels_q16,
                &color_box_q16,
            ) else {
                break;
            };
            if !try11(
                blk_endpoints,
                blk_weights,
                &mut cur_block_error,
                best_submode,
                l_q16,
                h_q16,
            ) {
                break;
            }
            // It's improved, so let's take the new weight indices.
            trial_blk_weights[..num_pixels].copy_from_slice(&blk_weights[..num_pixels]);
        }
    } else {
        if h == l {
            for w in emphasis_weights.iter_mut().take(num_pixels) {
                *w = 1.0;
            }
        } else {
            let mut mid = (0.0 - l) / (h - l);
            mid = clampf(mid, 0.01, 0.99);
            let (lw, mw, hw) = if opt_mode == OptMode::WeightedLeastSquaresHeavy {
                (
                    LOW_EMPHASIS_WEIGHT_HEAVY,
                    MIDDLE_EMPHASIS_WEIGHT_HEAVY,
                    HIGH_EMPHASIS_WEIGHT_HEAVY,
                )
            } else {
                (
                    LOW_EMPHASIS_WEIGHT,
                    MIDDLE_EMPHASIS_WEIGHT,
                    HIGH_EMPHASIS_WEIGHT,
                )
            };
            for i in 0..num_pixels {
                let k = v_sub(xyz(&block_pixels_q16[i]), block_mean_color_q16);
                let kd = v_dot(k, block_axis_q16);
                let mut v = (kd - l) / (h - l);
                v = if v < mid {
                    lerpf(lw, mw, v / mid)
                } else {
                    lerpf(mw, hw, (v - mid) * (1.0 - mid))
                };
                emphasis_weights[i] = v;
            }
        }
        for _ in 0..NUM_LS_PASSES {
            let Some((l_q16, h_q16)) = compute_weighted_least_squares_endpoints_rgb(
                num_pixels,
                Some((&trial_blk_weights, ls_weights)),
                &[],
                &emphasis_weights,
                block_pixels_q16,
            ) else {
                break;
            };
            if !try11(
                blk_endpoints,
                blk_weights,
                &mut cur_block_error,
                best_submode,
                l_q16,
                h_q16,
            ) {
                break;
            }
            trial_blk_weights[..num_pixels].copy_from_slice(&blk_weights[..num_pixels]);
        }
    }

    if uber_mode
        && ise_weight_range >= BISE_3_LEVELS
        && matches!(
            opt_mode,
            OptMode::OrdinaryLeastSquares
                | OptMode::WeightedLeastSquares
                | OptMode::WeightedLeastSquaresHeavy
        )
    {
        // Try varying the current best weight indices.
        let temp_astc_weights = trial_blk_weights;
        let a2l = &t.map_astc_to_linear_order[wr];
        let l2a = &t.map_linear_to_astc_order[wr];
        let (mut min_lin_sel, mut max_lin_sel) = (256u32, 0u32);
        for &astc_sel in temp_astc_weights.iter().take(num_pixels) {
            let lin_sel = a2l[astc_sel as usize] as u32;
            min_lin_sel = min_lin_sel.min(lin_sel);
            max_lin_sel = max_lin_sel.max(lin_sel);
        }
        let nwl = num_weight_levels as u32;
        for variant in 0..3 {
            let mut weights_changed = false;
            let mut trial_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
            for i in 0..num_pixels {
                let mut lin_sel = a2l[temp_astc_weights[i] as usize] as u32;
                let up = lin_sel == min_lin_sel && lin_sel < nwl - 1;
                let down = lin_sel == max_lin_sel && lin_sel > 0;
                match variant {
                    0 if up => {
                        lin_sel += 1;
                        weights_changed = true;
                    }
                    1 if down => {
                        lin_sel -= 1;
                        weights_changed = true;
                    }
                    2 if down => {
                        lin_sel -= 1;
                        weights_changed = true;
                    }
                    2 if up => {
                        lin_sel += 1;
                        weights_changed = true;
                    }
                    _ => {}
                }
                trial_weights[i] = l2a[lin_sel as usize];
            }
            if !weights_changed {
                continue;
            }
            let fit = if opt_mode == OptMode::OrdinaryLeastSquares {
                compute_least_squares_endpoints_rgb(
                    num_pixels,
                    &trial_weights,
                    ls_weights,
                    block_pixels_q16,
                    &color_box_q16,
                )
            } else {
                compute_weighted_least_squares_endpoints_rgb(
                    num_pixels,
                    Some((&trial_weights, ls_weights)),
                    &[],
                    &emphasis_weights,
                    block_pixels_q16,
                )
            };
            if let Some((l_q16, h_q16)) = fit {
                try11(
                    blk_endpoints,
                    blk_weights,
                    &mut cur_block_error,
                    best_submode,
                    l_q16,
                    h_q16,
                );
            }
        }
    }
    cur_block_error
}

/// `encode_astc_hdr_block_mode_7`.
#[allow(clippy::too_many_arguments)]
pub fn encode_astc_hdr_block_mode_7(
    num_pixels: usize,
    block_pixels_half: &[[HalfFloat; 3]],
    block_pixels_q16: &[[f32; 4]],
    ise_weight_range: u32,
    best_submode: &mut u32,
    mut cur_block_error: f64,
    blk_endpoints: &mut [u8],
    blk_weights: &mut [u8],
    coptions: &AstcHdrCodecBaseOptions,
    ise_endpoint_range: u32,
    first_submode: i32,
    last_submode: i32,
    block_stats: Option<&EncodeAstcBlockStats>,
) -> f64 {
    let num_weight_levels = get_ise_levels(ise_weight_range) as usize;
    let wr = ise_weight_range as usize;
    *best_submode = 0;

    let block_mean_color_q16 = match block_stats {
        Some(s) => s.mean_q16,
        None => calc_mean(num_pixels, block_pixels_q16),
    };
    let block_axis_q16: Vec3 = [0.577350259; 3];

    let mut color_box_q16 = Aabb3::init_expand();
    let mut l = BIG_FLOAT_VAL;
    let mut h = -BIG_FLOAT_VAL;
    for p in block_pixels_q16.iter().take(num_pixels) {
        color_box_q16.expand(xyz(p));
        let k = v_sub(xyz(p), block_mean_color_q16);
        let kd = v_dot(k, block_axis_q16);
        l = if kd < l { kd } else { l };
        h = if h < kd { kd } else { h };
    }

    let (mut low_color_q16, _) = interp_color(
        block_mean_color_q16,
        block_axis_q16,
        l,
        &color_box_q16,
        &color_box_q16,
    );
    let (mut high_color_q16, _) = interp_color(
        block_mean_color_q16,
        block_axis_q16,
        h,
        &color_box_q16,
        &color_box_q16,
    );
    v_clamp(&mut low_color_q16, 0.0, MAX_QLOG16_VAL);
    v_clamp(&mut high_color_q16, 0.0, MAX_QLOG16_VAL);

    let diff = v_sub(high_color_q16, low_color_q16);
    // S is subtracted from the high color with a scale of 1.0; move along the grayscale axis.
    let s_q16 = v_dot(diff, block_axis_q16) * block_axis_q16[0];

    let mut trial_blk_endpoints = [0u8; NUM_MODE7_ENDPOINTS];
    let mut trial_blk_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let mut trial_best_submode = 0u32;
    let mut trial_blk_error = BIG_FLOAT_VAL as f64;

    let try7 = |endpoints: &mut [u8],
                weights: &mut [u8],
                cur: &mut f64,
                bs: &mut u32,
                hi: Vec3,
                s: f32| {
        try_mode7(
            num_pixels,
            endpoints,
            weights,
            cur,
            bs,
            hi,
            s,
            block_pixels_half,
            num_weight_levels,
            ise_weight_range,
            coptions,
            ise_endpoint_range,
            first_submode,
            last_submode,
        )
    };

    let did_improve = try7(
        &mut trial_blk_endpoints,
        &mut trial_blk_weights,
        &mut trial_blk_error,
        &mut trial_best_submode,
        high_color_q16,
        s_q16.ceil(),
    );
    // If we couldn't find ANY usable solution due to endpoint quantization, just return.
    if !did_improve {
        return cur_block_error;
    }
    if trial_blk_error < cur_block_error {
        cur_block_error = trial_blk_error;
        blk_endpoints[..NUM_MODE7_ENDPOINTS].copy_from_slice(&trial_blk_endpoints);
        blk_weights[..num_pixels].copy_from_slice(&trial_blk_weights[..num_pixels]);
        *best_submode = trial_best_submode;
    }

    {
        // Try a simpler technique for artifact reduction
        l = BIG_FLOAT_VAL;
        h = -BIG_FLOAT_VAL;
        let mut alt_low_color_q16 = [0f32; 3];
        let mut alt_high_color_q16 = [0f32; 3];
        for p in block_pixels_q16.iter().take(num_pixels) {
            color_box_q16.expand(xyz(p));
            let k = v_sub(xyz(p), block_mean_color_q16);
            let kd = v_dot(k, block_axis_q16);
            if kd < l {
                alt_low_color_q16 = xyz(p);
                l = kd;
            }
            if kd > h {
                alt_high_color_q16 = xyz(p);
                h = kd;
            }
        }
        let old_alt_low = alt_low_color_q16;
        for i in 0..3 {
            alt_low_color_q16[i] = lerpf(old_alt_low[i], alt_high_color_q16[i], 1.0 / 64.0);
        }
        let alt_diff = v_sub(alt_high_color_q16, alt_low_color_q16);
        let alt_s_q16 = v_dot(alt_diff, block_axis_q16) * block_axis_q16[0];
        try7(
            blk_endpoints,
            blk_weights,
            &mut cur_block_error,
            best_submode,
            alt_high_color_q16,
            alt_s_q16.ceil(),
        );
    }

    let one_over_num_pixels = 1.0f32 / num_pixels as f32;
    const NUM_TRIALS: u32 = 2;
    for _ in 0..NUM_TRIALS {
        // Given a set of selectors and S, try to compute a better high color
        let mut new_high_color_q16 = block_mean_color_q16;
        let mut e = [[0i32; 3]; 2];
        let mut cur_s = 0;
        if !decode_mode7_to_qlog12_s(
            &trial_blk_endpoints,
            &mut e,
            Some(&mut cur_s),
            ise_endpoint_range,
        ) {
            break;
        }
        cur_s <<= 4;
        for &astc_sel in trial_blk_weights.iter().take(num_pixels) {
            let lerp = G_ISE_WEIGHT_LERPS[wr][astc_sel as usize + 1] as f32 * (1.0 / 64.0);
            let k = cur_s as f32 * (1.0 - lerp) * one_over_num_pixels;
            new_high_color_q16[0] += k;
            new_high_color_q16[1] += k;
            new_high_color_q16[2] += k;
        }
        let mut improved = try7(
            blk_endpoints,
            blk_weights,
            &mut cur_block_error,
            best_submode,
            new_high_color_q16,
            cur_s as f32,
        );
        if improved {
            trial_blk_endpoints.copy_from_slice(&blk_endpoints[..NUM_MODE7_ENDPOINTS]);
            trial_blk_weights[..num_pixels].copy_from_slice(&blk_weights[..num_pixels]);
        }

        // Given a set of selectors and a high color, try to compute a better S.
        let mut t = 0f32;
        for &astc_sel in trial_blk_weights.iter().take(num_pixels) {
            let lerp = G_ISE_WEIGHT_LERPS[wr][astc_sel as usize + 1] as f32 * (1.0 / 64.0);
            t += 1.0 - lerp;
        }
        t *= one_over_num_pixels;

        if !decode_mode7_to_qlog12_s(&trial_blk_endpoints, &mut e, None, ise_endpoint_range) {
            break;
        }
        let cur_h_q16 = q16_of(&e[1]);

        if t.abs() > 0.0000125 {
            let s_r = (cur_h_q16[0] - block_mean_color_q16[0]) / t;
            let s_g = (cur_h_q16[1] - block_mean_color_q16[1]) / t;
            let s_b = (cur_h_q16[2] - block_mean_color_q16[2]) / t;
            if try7(
                blk_endpoints,
                blk_weights,
                &mut cur_block_error,
                best_submode,
                cur_h_q16,
                s_r.ceil(),
            ) {
                improved = true;
            }
            if coptions.mode7_full_s_optimization {
                let maxf = |a: f32, b: f32| if a < b { b } else { a };
                let s_max = maxf(maxf(s_r, s_g), s_b) * 1.1;
                let s_max = if s_max < 65535.0 { s_max } else { 65535.0 };
                for s in [
                    s_g.ceil(),
                    s_b.ceil(),
                    ((s_r + s_g + s_b) / 3.0).ceil(),
                    s_max,
                ] {
                    if try7(
                        blk_endpoints,
                        blk_weights,
                        &mut cur_block_error,
                        best_submode,
                        cur_h_q16,
                        s,
                    ) {
                        improved = true;
                    }
                }
            }
        }
        if !improved {
            break;
        }
        trial_blk_endpoints.copy_from_slice(&blk_endpoints[..NUM_MODE7_ENDPOINTS]);
        trial_blk_weights[..num_pixels].copy_from_slice(&blk_weights[..num_pixels]);
    }
    cur_block_error
}

// ---------------------------------------------------------------------------------------------
// 6x6 only: CEM config decoding, dual plane, downsampled weight grids, endpoint refinement

/// `decode_cem_11_config`: (submode_index, maj_index).
pub fn decode_cem_11_config(endpoints: &[u8]) -> (i32, i32) {
    let mut submode_index = 0;
    let mut maj_index = 0;
    pack_bit(&mut submode_index, 0, endpoints[1] as i32, 7);
    pack_bit(&mut submode_index, 1, endpoints[2] as i32, 7);
    pack_bit(&mut submode_index, 2, endpoints[3] as i32, 7);
    pack_bit(&mut maj_index, 0, endpoints[4] as i32, 7);
    pack_bit(&mut maj_index, 1, endpoints[5] as i32, 7);
    (submode_index, maj_index)
}

/// `decode_cem_7_config`: (submode_index, maj_index).
pub fn decode_cem_7_config(endpoints: &[u8]) -> (i32, i32) {
    let (v0, v1, v2) = (
        endpoints[0] as i32,
        endpoints[1] as i32,
        endpoints[2] as i32,
    );
    // Extract mode bits and unpack to major component and mode.
    let modeval = ((v0 & 0xC0) >> 6) | ((v1 & 0x80) >> 5) | ((v2 & 0x80) >> 4);
    if (modeval & 0xC) != 0xC {
        (modeval & 3, modeval >> 2)
    } else if modeval != 0xF {
        (4, modeval & 3)
    } else {
        (5, 0)
    }
}

/// `dequantize_astc_weights`: ISE weight symbols to raw [0, 64] weights.
pub fn dequantize_astc_weights(
    n: usize,
    src_ise_vals: &[u8],
    from_ise_range: u32,
    dst_raw_weights: &mut [u8],
) {
    let tab = &dequant_tables().weight_tab(from_ise_range).ise_to_val;
    for (d, &s) in dst_raw_weights.iter_mut().zip(src_ise_vals).take(n) {
        *d = tab.get(s as usize).copied().unwrap_or(0);
    }
}

/// `eval_selectors_dual_plane`: plane 0 holds the two channels other than `channel_index`,
/// plane 1 the `channel_index` channel.
#[allow(clippy::too_many_arguments)]
pub fn eval_selectors_dual_plane(
    channel_index: usize,
    num_pixels: usize,
    weights0: &mut [u8],
    weights1: &mut [u8],
    block_pixels_half: &[[HalfFloat; 3]],
    num_weight_levels: usize,
    decoded_half: &[[HalfFloat; 3]],
    coptions: &AstcHdrCodecBaseOptions,
    usable_selector_bitmask: u32,
) -> f64 {
    let bias = coptions.q_log_bias;
    let mut total_error = 0f64;
    let mut decoded_half_q = [[0f64; 3]; MAX_SUPPORTED_WEIGHT_LEVELS];
    for i in 0..num_weight_levels {
        for c in 0..3 {
            decoded_half_q[i][c] = q(decoded_half[i][c], bias);
        }
    }
    let channel_weights = [
        coptions.r_err_scale as f64,
        coptions.g_err_scale as f64,
        1.0f64,
    ];
    let first_channel = (channel_index + 1) % 3;
    let second_channel = (channel_index + 2) % 3;
    // First plane
    let first_channel_weight = channel_weights[first_channel];
    let second_channel_weight = channel_weights[second_channel];
    for p in 0..num_pixels {
        let desired = &block_pixels_half[p];
        let dx = q(desired[first_channel], bias);
        let dy = q(desired[second_channel], bias);
        let mut lowest_e = BIG_FLOAT_VAL as f64;
        for i in 0..num_weight_levels {
            if (1u32 << i) & usable_selector_bitmask == 0 {
                continue;
            }
            let xd = decoded_half_q[i][first_channel] - dx;
            let yd = decoded_half_q[i][second_channel] - dy;
            let e = first_channel_weight * (xd * xd) + second_channel_weight * (yd * yd);
            if e < lowest_e {
                lowest_e = e;
                weights0[p] = i as u8;
            }
        }
        total_error += lowest_e;
    }
    // Second plane
    let alt_channel_weight = channel_weights[channel_index];
    for p in 0..num_pixels {
        let da = q(block_pixels_half[p][channel_index], bias);
        let mut lowest_e = BIG_FLOAT_VAL as f64;
        for i in 0..num_weight_levels {
            if (1u32 << i) & usable_selector_bitmask == 0 {
                continue;
            }
            let ad = decoded_half_q[i][channel_index] - da;
            let e = alt_channel_weight * (ad * ad);
            if e < lowest_e {
                lowest_e = e;
                weights1[p] = i as u8;
            }
        }
        total_error += lowest_e;
    }
    total_error
}

/// `compute_least_squares_endpoints_2D` / `_1D`: per channel least squares fit with box
/// fallback, clamped to the qlog16 range.
fn compute_least_squares_endpoints_nd<const N: usize>(
    n: usize,
    selectors: &[u8],
    selector_weights: &[[f32; 4]],
    colors: &[[f32; N]],
    box_lo: [f32; N],
    box_hi: [f32; N],
) -> Option<([f32; N], [f32; N])> {
    let (mut z00, mut z10, mut z11) = (0f32, 0f32, 0f32);
    let mut q00 = [0f32; N];
    let mut t = [0f32; N];
    for i in 0..n {
        let sw = selector_weights[selectors[i] as usize];
        z00 += sw[0];
        z10 += sw[1];
        z11 += sw[2];
        let w = sw[3];
        for c in 0..N {
            q00[c] += w * colors[i][c];
            t[c] += colors[i][c];
        }
    }
    let z01 = z10;
    let mut det = z00 * z11 - z01 * z10;
    if det == 0.0 {
        return None;
    }
    det = 1.0 / det;
    let iz00 = z11 * det;
    let iz01 = -z01 * det;
    let iz10 = -z10 * det;
    let iz11 = z00 * det;
    let mut xl = [0f32; N];
    let mut xh = [0f32; N];
    for c in 0..N {
        let q10 = t[c] - q00[c];
        xl[c] = iz00 * q00[c] + iz01 * q10;
        xh[c] = iz10 * q00[c] + iz11 * q10;
        if box_hi[c] - box_lo[c] < 0.0000125 {
            xl[c] = box_lo[c];
            xh[c] = box_hi[c];
        }
    }
    for c in 0..N {
        xl[c] = clampf(xl[c], 0.0, MAX_QLOG16_VAL);
        xh[c] = clampf(xh[c], 0.0, MAX_QLOG16_VAL);
    }
    Some((xl, xh))
}

/// `try_mode11_dual_plane`.
#[allow(clippy::too_many_arguments)]
pub fn try_mode11_dual_plane(
    channel_index: usize,
    num_pixels: usize,
    endpoints: &mut [u8],
    weights0: &mut [u8],
    weights1: &mut [u8],
    cur_block_error: &mut f64,
    submode_used: &mut u32,
    low_color_q16: Vec3,
    high_color_q16: Vec3,
    block_pixels_half: &[[HalfFloat; 3]],
    num_weight_levels: usize,
    ise_weight_range: u32,
    coptions: &AstcHdrCodecBaseOptions,
    direct_only: bool,
    ise_endpoint_range: u32,
    constrain_ise_weight_selectors: bool,
    first_submode: i32,
    last_submode: i32,
    ignore_clamping: bool,
) -> bool {
    let (best_trial_endpoints, best_trial_dist, best_submode) = search_mode11(
        low_color_q16,
        high_color_q16,
        ise_endpoint_range,
        coptions,
        direct_only,
        first_submode,
        last_submode,
        ignore_clamping,
    );
    if best_trial_dist == BIG_FLOAT_VAL as f64 {
        return false;
    }
    let mut decoded_half = [[0 as HalfFloat; 3]; MAX_SUPPORTED_WEIGHT_LEVELS];
    if !get_astc_hdr_mode_11_block_colors(
        &best_trial_endpoints,
        &mut decoded_half,
        num_weight_levels,
        ise_weight_range,
        ise_endpoint_range,
    ) {
        return false;
    }
    let mut usable_selector_bitmask = u32::MAX;
    if constrain_ise_weight_selectors && ise_weight_range == BISE_16_LEVELS {
        usable_selector_bitmask = (1 << 0)
            | (1 << 1)
            | (1 << 4)
            | (1 << 5)
            | (1 << 10)
            | (1 << 11)
            | (1 << 14)
            | (1 << 15);
    } else if constrain_ise_weight_selectors && ise_weight_range == BISE_12_LEVELS {
        usable_selector_bitmask = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3);
    }
    let mut trial_weights0 = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let mut trial_weights1 = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let trial_blk_error = eval_selectors_dual_plane(
        channel_index,
        num_pixels,
        &mut trial_weights0,
        &mut trial_weights1,
        block_pixels_half,
        num_weight_levels,
        &decoded_half,
        coptions,
        usable_selector_bitmask,
    );
    if trial_blk_error < *cur_block_error {
        *cur_block_error = trial_blk_error;
        endpoints[..NUM_MODE11_ENDPOINTS].copy_from_slice(&best_trial_endpoints);
        weights0[..num_pixels].copy_from_slice(&trial_weights0[..num_pixels]);
        weights1[..num_pixels].copy_from_slice(&trial_weights1[..num_pixels]);
        *submode_used = (best_submode + 1) as u32;
        return true;
    }
    false
}

/// `encode_astc_hdr_block_mode_11_dual_plane` (`uber_mode` is unused, as in C++).
#[allow(clippy::too_many_arguments)]
pub fn encode_astc_hdr_block_mode_11_dual_plane(
    num_pixels: usize,
    block_pixels_half: &[[HalfFloat; 3]],
    block_pixels_q16: &[[f32; 4]],
    channel_index: usize,
    ise_weight_range: u32,
    best_submode: &mut u32,
    mut cur_block_error: f64,
    blk_endpoints: &mut [u8],
    blk_weights0: &mut [u8],
    blk_weights1: &mut [u8],
    coptions: &AstcHdrCodecBaseOptions,
    direct_only: bool,
    ise_endpoint_range: u32,
    _uber_mode: bool,
    constrain_ise_weight_selectors: bool,
    first_submode: i32,
    last_submode: i32,
    ignore_clamping: bool,
) -> f64 {
    let t = astc_hdr_enc_tables();
    *best_submode = 0;
    let num_weight_levels = get_ise_levels(ise_weight_range) as usize;

    let mut temp_block_pixels_q16 = [[0f32; 4]; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    for i in 0..num_pixels {
        temp_block_pixels_q16[i] = block_pixels_q16[i];
        temp_block_pixels_q16[i][channel_index] = 0.0;
    }
    let block_mean_color_q16 = calc_mean(num_pixels, &temp_block_pixels_q16);
    let block_axis_q16 = calc_rgb_pca(num_pixels, &temp_block_pixels_q16, block_mean_color_q16);

    let mut l = BIG_FLOAT_VAL;
    let mut h = -BIG_FLOAT_VAL;
    let mut low_color_q16 = [0f32; 3];
    let mut high_color_q16 = [0f32; 3];
    let mut color_box_q16 = Aabb3::init_expand();
    for i in 0..num_pixels {
        color_box_q16.expand(xyz(&block_pixels_q16[i]));
        let k = v_sub(xyz(&temp_block_pixels_q16[i]), block_mean_color_q16);
        let kd = v_dot(k, block_axis_q16);
        if kd < l {
            l = kd;
            low_color_q16 = xyz(&block_pixels_q16[i]);
        }
        if kd > h {
            h = kd;
            high_color_q16 = xyz(&block_pixels_q16[i]);
        }
    }
    low_color_q16[channel_index] = 0.0;
    high_color_q16[channel_index] = 0.0;
    let a = v_dot(low_color_q16, [1.0; 3]);
    let b = v_dot(high_color_q16, [1.0; 3]);
    if a <= b {
        low_color_q16[channel_index] = color_box_q16.lo[channel_index];
        high_color_q16[channel_index] = color_box_q16.hi[channel_index];
    } else {
        high_color_q16[channel_index] = color_box_q16.lo[channel_index];
        low_color_q16[channel_index] = color_box_q16.hi[channel_index];
    }
    let (old_low, old_high) = (low_color_q16, high_color_q16);
    for i in 0..3 {
        low_color_q16[i] = lerpf(old_low[i], old_high[i], 1.0 / 64.0);
        high_color_q16[i] = lerpf(old_low[i], old_high[i], 63.0 / 64.0);
    }

    let mut trial_blk_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
    let mut trial_blk_weights0 = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let mut trial_blk_weights1 = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let mut trial_best_submode = 0u32;
    let mut trial_blk_error = BIG_FLOAT_VAL as f64;

    let try_dp = |e: &mut [u8],
                  w0: &mut [u8],
                  w1: &mut [u8],
                  cur: &mut f64,
                  bs: &mut u32,
                  lo: Vec3,
                  hi: Vec3| {
        try_mode11_dual_plane(
            channel_index,
            num_pixels,
            e,
            w0,
            w1,
            cur,
            bs,
            lo,
            hi,
            block_pixels_half,
            num_weight_levels,
            ise_weight_range,
            coptions,
            direct_only,
            ise_endpoint_range,
            constrain_ise_weight_selectors,
            first_submode,
            last_submode,
            ignore_clamping,
        )
    };

    let did_improve = try_dp(
        &mut trial_blk_endpoints,
        &mut trial_blk_weights0,
        &mut trial_blk_weights1,
        &mut trial_blk_error,
        &mut trial_best_submode,
        low_color_q16,
        high_color_q16,
    );
    // If we couldn't find ANY usable solution due to endpoint quantization, just return.
    if !did_improve {
        return cur_block_error;
    }
    if trial_blk_error < cur_block_error {
        cur_block_error = trial_blk_error;
        blk_endpoints[..NUM_MODE11_ENDPOINTS].copy_from_slice(&trial_blk_endpoints);
        blk_weights0[..num_pixels].copy_from_slice(&trial_blk_weights0[..num_pixels]);
        blk_weights1[..num_pixels].copy_from_slice(&trial_blk_weights1[..num_pixels]);
        *best_submode = trial_best_submode;
    }

    let chan0 = (channel_index + 1) % 3;
    let chan1 = (channel_index + 2) % 3;
    let plane0_lo = [color_box_q16.lo[chan0], color_box_q16.lo[chan1]];
    let plane0_hi = [color_box_q16.hi[chan0], color_box_q16.hi[chan1]];
    let plane1_lo = [color_box_q16.lo[channel_index]];
    let plane1_hi = [color_box_q16.hi[channel_index]];
    let mut plane0_q16 = [[0f32; 2]; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let mut plane1_q16 = [[0f32; 1]; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    for i in 0..num_pixels {
        plane0_q16[i] = [block_pixels_q16[i][chan0], block_pixels_q16[i][chan1]];
        plane1_q16[i] = [block_pixels_q16[i][channel_index]];
    }
    let ls_weights = &t.ls_weights_ise[ise_weight_range as usize];
    const NUM_LS_PASSES: u32 = 3;
    for _ in 0..NUM_LS_PASSES {
        let Some((l0, h0)) = compute_least_squares_endpoints_nd(
            num_pixels,
            &trial_blk_weights0,
            ls_weights,
            &plane0_q16,
            plane0_lo,
            plane0_hi,
        ) else {
            break;
        };
        let Some((l1, h1)) = compute_least_squares_endpoints_nd(
            num_pixels,
            &trial_blk_weights1,
            ls_weights,
            &plane1_q16,
            plane1_lo,
            plane1_hi,
        ) else {
            break;
        };
        let mut l_q16 = [0f32; 3];
        let mut h_q16 = [0f32; 3];
        l_q16[channel_index] = l1[0];
        h_q16[channel_index] = h1[0];
        l_q16[chan0] = l0[0];
        h_q16[chan0] = h0[0];
        l_q16[chan1] = l0[1];
        h_q16[chan1] = h0[1];
        if !try_dp(
            blk_endpoints,
            blk_weights0,
            blk_weights1,
            &mut cur_block_error,
            best_submode,
            l_q16,
            h_q16,
        ) {
            break;
        }
        // It's improved, so let's take the new weight indices.
        trial_blk_weights0[..num_pixels].copy_from_slice(&blk_weights0[..num_pixels]);
        trial_blk_weights1[..num_pixels].copy_from_slice(&blk_weights1[..num_pixels]);
    }
    cur_block_error
}

/// Error of qlog16 endpoints (`e16`, already shifted) interpolated by raw [0, 64] weights.
fn raw_weights_error(
    num_pixels: usize,
    block_pixels_half: &[[HalfFloat; 3]],
    raw_weights: &[u8],
    e16: &[[i32; 3]; 2],
    coptions: &AstcHdrCodecBaseOptions,
) -> f64 {
    let r_weight = coptions.r_err_scale as f64;
    let g_weight = coptions.g_err_scale as f64;
    let bias = coptions.q_log_bias;
    let mut err = 0f64;
    for p in 0..num_pixels {
        let desired = &block_pixels_half[p];
        let c = raw_weights[p] as u32;
        let mut d = [0f64; 3];
        for j in 0..3 {
            let (a0, a1) = (e16[0][j] as u32, e16[1][j] as u32);
            let v = (a0 * (64 - c) + a1 * c + 32) / 64;
            d[j] = q(qlog16_to_half(v as i32), bias) - q(desired[j], bias);
        }
        err += r_weight * (d[0] * d[0]) + g_weight * (d[1] * d[1]) + d[2] * d[2];
    }
    err
}

/// `compute_block_error_from_raw_weights` (qlog12 endpoints).
pub fn compute_block_error_from_raw_weights(
    num_pixels: usize,
    block_pixels_half: &[[HalfFloat; 3]],
    raw_weights: &[u8],
    endpoints_qlog12: &[[i32; 3]; 2],
    coptions: &AstcHdrCodecBaseOptions,
) -> f64 {
    let mut e16 = [[0i32; 3]; 2];
    for i in 0..3 {
        e16[0][i] = endpoints_qlog12[0][i] << 4;
        e16[1][i] = endpoints_qlog12[1][i] << 4;
    }
    raw_weights_error(num_pixels, block_pixels_half, raw_weights, &e16, coptions)
}

/// `get_6x6_downsample_matrix`.
pub fn get_6x6_downsample_matrix(grid_width: u32, grid_height: u32) -> Option<&'static [f32]> {
    DOWNSAMPLE_MATRICES_6X6
        .iter()
        .find(|m| m.0 == grid_width && m.1 == grid_height)
        .map(|m| m.2)
}

/// `downsample_weight_grid`: dequantized [by][bx] weights to a [wy][wx] grid.
pub fn downsample_weight_grid(
    matrix_weights: &[f32],
    bx: u32,
    by: u32,
    wx: u32,
    wy: u32,
    src_weights: &[u8],
    dst_weights: &mut [u8],
) {
    let total_block_samples = (bx * by) as usize;
    let mut m = 0;
    for y in 0..wy {
        for x in 0..wx {
            let mut total = 0.5f32;
            for i in 0..total_block_samples {
                let w = matrix_weights.get(m + i).copied().unwrap_or(0.0);
                if w != 0.0 {
                    total += w * src_weights[i] as f32;
                }
            }
            dst_weights[(x + y * wx) as usize] = (total as i32).clamp(0, 64) as u8;
            m += total_block_samples;
        }
    }
}

/// `downsample_ise_weights`: block resolution ISE weights to grid resolution ISE weights.
#[allow(clippy::too_many_arguments)]
pub fn downsample_ise_weights(
    dequant_weight_ise_range: u32,
    quant_weight_ise_range: u32,
    block_w: u32,
    block_h: u32,
    grid_w: u32,
    grid_h: u32,
    src_weights: &[u8],
    dst_weights: &mut [u8],
) {
    if block_w == grid_w && block_h == grid_h {
        let n = (block_w * block_h) as usize;
        if dequant_weight_ise_range != quant_weight_ise_range {
            super::astc_6x6_hdr::requantize_astc_weights(
                n,
                src_weights,
                dequant_weight_ise_range,
                dst_weights,
                quant_weight_ise_range,
            );
        } else {
            dst_weights[..n].copy_from_slice(&src_weights[..n]);
        }
        return;
    }
    let mut desired_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let dequant_tab = &dequant_tables()
        .weight_tab(dequant_weight_ise_range)
        .ise_to_val;
    for i in 0..(block_w * block_h) as usize {
        desired_weights[i] = dequant_tab
            .get(src_weights[i] as usize)
            .copied()
            .unwrap_or(0);
    }
    let mut downsampled_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let matrix = get_6x6_downsample_matrix(grid_w, grid_h).unwrap_or(&[]);
    downsample_weight_grid(
        matrix,
        block_w,
        block_h,
        grid_w,
        grid_h,
        &desired_weights,
        &mut downsampled_weights,
    );
    let quant_tab = &dequant_tables()
        .weight_tab(quant_weight_ise_range)
        .val_to_ise;
    for i in 0..(grid_w * grid_h) as usize {
        dst_weights[i] = quant_tab
            .get(downsampled_weights[i] as usize)
            .copied()
            .unwrap_or(0);
    }
}

/// `downsample_ise_weights_dual_plane`: interleaves the two downsampled planes.
#[allow(clippy::too_many_arguments)]
pub fn downsample_ise_weights_dual_plane(
    dequant_weight_ise_range: u32,
    quant_weight_ise_range: u32,
    block_w: u32,
    block_h: u32,
    grid_w: u32,
    grid_h: u32,
    src_weights0: &[u8],
    src_weights1: &[u8],
    dst_weights: &mut [u8],
) {
    let mut d0 = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    let mut d1 = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    downsample_ise_weights(
        dequant_weight_ise_range,
        quant_weight_ise_range,
        block_w,
        block_h,
        grid_w,
        grid_h,
        src_weights0,
        &mut d0,
    );
    downsample_ise_weights(
        dequant_weight_ise_range,
        quant_weight_ise_range,
        block_w,
        block_h,
        grid_w,
        grid_h,
        src_weights1,
        &mut d1,
    );
    for i in 0..(grid_w * grid_h) as usize {
        dst_weights[i * 2] = d0[i];
        dst_weights[i * 2 + 1] = d1[i];
    }
}

/// Emphasis weights along the block's principal axis (weighted least squares variants).
#[allow(clippy::too_many_arguments)]
fn emphasis_weights_for(
    num_pixels: usize,
    block_pixels_q16: &[[f32; 4]],
    mean: Vec3,
    axis: Vec3,
    l: f32,
    h: f32,
    opt_mode: OptMode,
    out: &mut [f32],
) {
    if h == l {
        for w in out.iter_mut().take(num_pixels) {
            *w = 1.0;
        }
        return;
    }
    let mut mid = (0.0 - l) / (h - l);
    mid = clampf(mid, 0.01, 0.99);
    let (lw, mw, hw) = if opt_mode == OptMode::WeightedLeastSquaresHeavy {
        (
            LOW_EMPHASIS_WEIGHT_HEAVY,
            MIDDLE_EMPHASIS_WEIGHT_HEAVY,
            HIGH_EMPHASIS_WEIGHT_HEAVY,
        )
    } else {
        (
            LOW_EMPHASIS_WEIGHT,
            MIDDLE_EMPHASIS_WEIGHT,
            HIGH_EMPHASIS_WEIGHT,
        )
    };
    for i in 0..num_pixels {
        let k = v_sub(xyz(&block_pixels_q16[i]), mean);
        let kd = v_dot(k, axis);
        let mut v = (kd - l) / (h - l);
        v = if v < mid {
            lerpf(lw, mw, v / mid)
        } else {
            lerpf(mw, hw, (v - mid) * (1.0 - mid))
        };
        out[i] = v;
    }
}

/// `encode_astc_hdr_block_downsampled_mode_11`: CEM 11 with a `grid_x` x `grid_y` weight grid.
/// C++ quirk kept: a failed weighted least squares fit returns 0 (`return false` from a function
/// returning double).
#[allow(clippy::too_many_arguments)]
pub fn encode_astc_hdr_block_downsampled_mode_11(
    block_x: u32,
    block_y: u32,
    grid_x: u32,
    grid_y: u32,
    ise_weight_range: u32,
    ise_endpoint_range: u32,
    num_pixels: usize,
    block_pixels_half: &[[HalfFloat; 3]],
    block_pixels_q16: &[[f32; 4]],
    mut cur_block_error: f64,
    first_submode: i32,
    last_submode: i32,
    ignore_clamping: bool,
    opt_mode: OptMode,
    blk_endpoints: &mut [u8],
    blk_weights: &mut [u8],
    best_submode: &mut u32,
    coptions: &AstcHdrCodecBaseOptions,
    block_stats: Option<&EncodeAstcBlockStats>,
) -> f64 {
    *best_submode = 0;
    let num_weights = (grid_x * grid_y) as usize;
    let (block_mean_color_q16, block_axis_q16) = match block_stats {
        Some(s) => (s.mean_q16, s.axis_q16),
        None => {
            let m = calc_mean(num_pixels, block_pixels_q16);
            (m, calc_rgb_pca(num_pixels, block_pixels_q16, m))
        }
    };
    let mut color_box_q16 = Aabb3::init_expand();
    let mut l = BIG_FLOAT_VAL;
    let mut h = -BIG_FLOAT_VAL;
    let mut low_color_q16 = [0f32; 3];
    let mut high_color_q16 = [0f32; 3];
    for p in block_pixels_q16.iter().take(num_pixels) {
        color_box_q16.expand(xyz(p));
        let k = v_sub(xyz(p), block_mean_color_q16);
        let kd = v_dot(k, block_axis_q16);
        if kd < l {
            l = kd;
            low_color_q16 = xyz(p);
        }
        if kd > h {
            h = kd;
            high_color_q16 = xyz(p);
        }
    }
    let (old_low, old_high) = (low_color_q16, high_color_q16);
    for i in 0..3 {
        low_color_q16[i] = lerpf(old_low[i], old_high[i], 1.0 / 64.0);
        high_color_q16[i] = lerpf(old_low[i], old_high[i], 63.0 / 64.0);
    }

    const NUM_PASSES: u32 = 3;
    for pass in 0..NUM_PASSES {
        let mut trial_blk_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
        // at block resolution, not grid res
        let mut trial_blk_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
        let mut trial_best_submode = 0u32;
        let mut trial_blk_error = BIG_FLOAT_VAL as f64;
        let could_pack = try_mode11(
            num_pixels,
            &mut trial_blk_endpoints,
            &mut trial_blk_weights,
            &mut trial_blk_error,
            &mut trial_best_submode,
            low_color_q16,
            high_color_q16,
            block_pixels_half,
            32,
            ah::BISE_32_LEVELS,
            coptions,
            false,
            ise_endpoint_range,
            false,
            first_submode,
            last_submode,
            ignore_clamping,
        );
        if !could_pack {
            break;
        }
        let mut trial_downsampled_ise_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
        downsample_ise_weights(
            ah::BISE_32_LEVELS,
            ise_weight_range,
            block_x,
            block_y,
            grid_x,
            grid_y,
            &trial_blk_weights,
            &mut trial_downsampled_ise_weights,
        );
        let mut trial_downsampled_raw_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
        dequantize_astc_weights(
            num_weights,
            &trial_downsampled_ise_weights,
            ise_weight_range,
            &mut trial_downsampled_raw_weights,
        );
        // raw weights, NOT ISE
        let mut trial_upsampled_raw_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
        ah::upsample_weight_grid(
            block_x,
            block_y,
            grid_x,
            grid_y,
            &trial_downsampled_raw_weights,
            &mut trial_upsampled_raw_weights,
        );

        let mut trial_e = [[0i32; 3]; 2];
        if !decode_mode11_to_qlog12(&trial_blk_endpoints, &mut trial_e, ise_endpoint_range) {
            return cur_block_error;
        }
        let mut trial_error = compute_block_error_from_raw_weights(
            num_pixels,
            block_pixels_half,
            &trial_upsampled_raw_weights,
            &trial_e,
            coptions,
        );
        if trial_error < cur_block_error {
            cur_block_error = trial_error;
            blk_endpoints[..NUM_MODE11_ENDPOINTS].copy_from_slice(&trial_blk_endpoints);
            blk_weights[..num_weights]
                .copy_from_slice(&trial_downsampled_ise_weights[..num_weights]);
            *best_submode = trial_best_submode;
        } else if pass != 0 {
            break;
        }

        if matches!(
            opt_mode,
            OptMode::WeightedLeastSquares | OptMode::WeightedLeastSquaresHeavy
        ) {
            let mut emphasis_weights = [0f32; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
            emphasis_weights_for(
                num_pixels,
                block_pixels_q16,
                block_mean_color_q16,
                block_axis_q16,
                l,
                h,
                opt_mode,
                &mut emphasis_weights,
            );
            let mut wf = [0f32; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
            for i in 0..num_pixels {
                wf[i] = trial_upsampled_raw_weights[i] as f32 * (1.0 / 64.0);
            }
            match compute_weighted_least_squares_endpoints_rgb(
                num_pixels,
                None,
                &wf,
                &emphasis_weights,
                block_pixels_q16,
            ) {
                Some((lo, hi)) => {
                    low_color_q16 = lo;
                    high_color_q16 = hi;
                }
                None => return 0.0,
            }
        } else {
            match compute_least_squares_endpoints_rgb_raw_weights(
                num_pixels,
                &trial_upsampled_raw_weights,
                block_pixels_q16,
                &color_box_q16,
            ) {
                Some((lo, hi)) => {
                    low_color_q16 = lo;
                    high_color_q16 = hi;
                }
                None => break,
            }
        }

        let Some(sm) = pack_mode11(
            low_color_q16,
            high_color_q16,
            ise_endpoint_range,
            &mut trial_blk_endpoints,
            coptions,
            false,
            first_submode,
            last_submode,
            false,
        ) else {
            break;
        };
        trial_best_submode = sm;
        if !decode_mode11_to_qlog12(&trial_blk_endpoints, &mut trial_e, ise_endpoint_range) {
            break;
        }
        trial_error = compute_block_error_from_raw_weights(
            num_pixels,
            block_pixels_half,
            &trial_upsampled_raw_weights,
            &trial_e,
            coptions,
        );
        if trial_error < cur_block_error {
            cur_block_error = trial_error;
            blk_endpoints[..NUM_MODE11_ENDPOINTS].copy_from_slice(&trial_blk_endpoints);
            blk_weights[..num_weights]
                .copy_from_slice(&trial_downsampled_ise_weights[..num_weights]);
            *best_submode = trial_best_submode;
        } else {
            break;
        }
    }
    cur_block_error
}

/// Upsamples a subset's grid weights to raw per pixel weights via `pixel_block_ofs`.
#[allow(clippy::too_many_arguments)]
fn subset_raw_weights(
    block_w: u32,
    block_h: u32,
    grid_w: u32,
    grid_h: u32,
    weights: &[u8],
    weight_ise_range: u32,
    num_pixels: usize,
    pixel_block_ofs: Option<&[u8]>,
) -> [u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS] {
    let num_weights = (grid_w * grid_h) as usize;
    let mut dequantized_raw_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    dequantize_astc_weights(
        num_weights,
        weights,
        weight_ise_range,
        &mut dequantized_raw_weights,
    );
    let mut upsampled_weights = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    ah::upsample_weight_grid(
        block_w,
        block_h,
        grid_w,
        grid_h,
        &dequantized_raw_weights,
        &mut upsampled_weights,
    );
    let mut out = [0u8; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    for i in 0..num_pixels {
        let ofs = pixel_block_ofs.map_or(i, |o| o[i] as usize);
        out[i] = upsampled_weights[ofs];
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn refine_endpoints_mode11(
    endpoint_ise_range: u32,
    endpoint_vals: &mut [u8],
    block_w: u32,
    block_h: u32,
    grid_w: u32,
    grid_h: u32,
    weights: &[u8],
    weight_ise_range: u32,
    num_pixels: usize,
    block_pixels_half: &[[HalfFloat; 3]],
    block_pixels_q16: &[[f32; 4]],
    pixel_block_ofs: Option<&[u8]>,
    coptions: &AstcHdrCodecBaseOptions,
    direct_only: bool,
    first_submode: i32,
    last_submode: i32,
    opt_mode: OptMode,
) -> bool {
    if opt_mode == OptMode::NoOpt {
        return false;
    }
    let trial_blk_raw_weights = subset_raw_weights(
        block_w,
        block_h,
        grid_w,
        grid_h,
        weights,
        weight_ise_range,
        num_pixels,
        pixel_block_ofs,
    );
    let mut color_box_q16 = Aabb3::init_expand();
    let mut trial_blk_raw_weightsf = [0f32; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
    for i in 0..num_pixels {
        color_box_q16.expand(xyz(&block_pixels_q16[i]));
        trial_blk_raw_weightsf[i] = trial_blk_raw_weights[i] as f32 * (1.0 / 64.0);
    }
    let (l_q16, h_q16) = match opt_mode {
        OptMode::OrdinaryLeastSquares => {
            match compute_least_squares_endpoints_rgb_raw_weights(
                num_pixels,
                &trial_blk_raw_weights,
                block_pixels_q16,
                &color_box_q16,
            ) {
                Some(r) => r,
                None => return false,
            }
        }
        OptMode::WeightedLeastSquares | OptMode::WeightedLeastSquaresHeavy => {
            let mean = calc_mean(num_pixels, block_pixels_q16);
            let axis = calc_rgb_pca(num_pixels, block_pixels_q16, mean);
            let mut l = BIG_FLOAT_VAL;
            let mut h = -BIG_FLOAT_VAL;
            for p in block_pixels_q16.iter().take(num_pixels) {
                let kd = v_dot(v_sub(xyz(p), mean), axis);
                if kd < l {
                    l = kd;
                }
                if kd > h {
                    h = kd;
                }
            }
            let mut emphasis_weights = [0f32; MAX_ASTC_HDR_ENC_BLOCK_PIXELS];
            emphasis_weights_for(
                num_pixels,
                block_pixels_q16,
                mean,
                axis,
                l,
                h,
                opt_mode,
                &mut emphasis_weights,
            );
            match compute_weighted_least_squares_endpoints_rgb(
                num_pixels,
                None,
                &trial_blk_raw_weightsf,
                &emphasis_weights,
                block_pixels_q16,
            ) {
                Some(r) => r,
                None => return false,
            }
        }
        _ => {
            // cWeightedAverage
            let mut l_q16 = [0f32; 3];
            let mut total_low = 0f32;
            let mut h_q16 = [0f32; 3];
            let mut total_high = 0f32;
            for i in 0..num_pixels {
                let p = xyz(&block_pixels_q16[i]);
                let lerp = trial_blk_raw_weights[i] as f32 * (1.0 / 64.0);
                l_q16 = v_add(l_q16, v_mul(p, 1.0 - lerp));
                total_low += 1.0 - lerp;
                h_q16 = v_add(h_q16, v_mul(p, lerp));
                total_high += lerp;
            }
            if total_low == 0.0 || total_high == 0.0 {
                return false;
            }
            (
                v_mul(l_q16, 1.0 / total_low),
                v_mul(h_q16, 1.0 / total_high),
            )
        }
    };
    let mut trial_endpoints = [0u8; NUM_MODE11_ENDPOINTS];
    if pack_mode11(
        l_q16,
        h_q16,
        endpoint_ise_range,
        &mut trial_endpoints,
        coptions,
        direct_only,
        first_submode,
        last_submode,
        false,
    )
    .is_none()
    {
        return false;
    }
    let mut cur_e = [[0i32; 3]; 2];
    if !decode_mode11_to_qlog12(endpoint_vals, &mut cur_e, endpoint_ise_range) {
        return false;
    }
    let mut trial_e = [[0i32; 3]; 2];
    if !decode_mode11_to_qlog12(&trial_endpoints, &mut trial_e, endpoint_ise_range) {
        return false;
    }
    let cur_error = compute_block_error_from_raw_weights(
        num_pixels,
        block_pixels_half,
        &trial_blk_raw_weights,
        &cur_e,
        coptions,
    );
    let trial_error = compute_block_error_from_raw_weights(
        num_pixels,
        block_pixels_half,
        &trial_blk_raw_weights,
        &trial_e,
        coptions,
    );
    if trial_error < cur_error {
        endpoint_vals[..NUM_MODE11_ENDPOINTS].copy_from_slice(&trial_endpoints);
        return true;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn refine_endpoints_mode7(
    endpoint_ise_range: u32,
    endpoint_vals: &mut [u8],
    block_w: u32,
    block_h: u32,
    grid_w: u32,
    grid_h: u32,
    weights: &[u8],
    weight_ise_range: u32,
    num_pixels: usize,
    block_pixels_half: &[[HalfFloat; 3]],
    block_pixels_q16: &[[f32; 4]],
    pixel_block_ofs: Option<&[u8]>,
    coptions: &AstcHdrCodecBaseOptions,
    first_submode: i32,
    last_submode: i32,
) -> bool {
    let trial_blk_raw_weights = subset_raw_weights(
        block_w,
        block_h,
        grid_w,
        grid_h,
        weights,
        weight_ise_range,
        num_pixels,
        pixel_block_ofs,
    );
    let mut cur_e = [[0i32; 3]; 2];
    let mut cur_s = 0;
    if !decode_mode7_to_qlog12_s(
        endpoint_vals,
        &mut cur_e,
        Some(&mut cur_s),
        endpoint_ise_range,
    ) {
        return false;
    }
    cur_s <<= 4;
    let block_mean_color_q16 = calc_mean(num_pixels, block_pixels_q16);
    let mut new_high_color_q16 = block_mean_color_q16;
    let one_over_num_pixels = 1.0f32 / num_pixels as f32;
    for &w in trial_blk_raw_weights.iter().take(num_pixels) {
        let lerp = w as f32 * (1.0 / 64.0);
        let k = cur_s as f32 * (1.0 - lerp) * one_over_num_pixels;
        new_high_color_q16[0] += k;
        new_high_color_q16[1] += k;
        new_high_color_q16[2] += k;
    }
    // Given a set of selectors and a high color, try to compute a better S.
    let mut t = 0f32;
    for &w in trial_blk_raw_weights.iter().take(num_pixels) {
        let lerp = w as f32 * (1.0 / 64.0);
        t += 1.0 - lerp;
    }
    t *= one_over_num_pixels;
    if t.abs() < 0.0000125 {
        return false;
    }
    let mut trial_endpoints = [0u8; NUM_MODE7_ENDPOINTS];
    if pack_mode7(
        new_high_color_q16,
        cur_s as f32,
        endpoint_ise_range,
        &mut trial_endpoints,
        weight_ise_range,
        coptions,
        first_submode,
        last_submode,
        false,
    )
    .is_none()
    {
        return false;
    }
    let mut trial_e = [[0i32; 3]; 2];
    if !decode_mode7_to_qlog12_s(&trial_endpoints, &mut trial_e, None, endpoint_ise_range) {
        return false;
    }
    let cur_h_q16 = q16_of(&trial_e[1]);
    let s_r = (cur_h_q16[0] - block_mean_color_q16[0]) / t;
    let new_s_q16 = s_r.ceil();
    if pack_mode7(
        new_high_color_q16,
        new_s_q16,
        endpoint_ise_range,
        &mut trial_endpoints,
        weight_ise_range,
        coptions,
        first_submode,
        last_submode,
        false,
    )
    .is_none()
    {
        return false;
    }
    if !decode_mode7_to_qlog12_s(&trial_endpoints, &mut trial_e, None, endpoint_ise_range) {
        return false;
    }
    let cur_error = compute_block_error_from_raw_weights(
        num_pixels,
        block_pixels_half,
        &trial_blk_raw_weights,
        &cur_e,
        coptions,
    );
    let trial_error = compute_block_error_from_raw_weights(
        num_pixels,
        block_pixels_half,
        &trial_blk_raw_weights,
        &trial_e,
        coptions,
    );
    if trial_error < cur_error {
        endpoint_vals[..NUM_MODE7_ENDPOINTS].copy_from_slice(&trial_endpoints);
        return true;
    }
    false
}

/// `refine_endpoints`: refits CEM 7 / 11 endpoints given a (possibly downsampled) weight grid.
/// `pixel_block_ofs` maps the subset's pixels to block offsets (`None` = identity).
#[allow(clippy::too_many_arguments)]
pub fn refine_endpoints(
    cem: u32,
    endpoint_ise_range: u32,
    endpoint_vals: &mut [u8],
    block_w: u32,
    block_h: u32,
    grid_w: u32,
    grid_h: u32,
    weights: &[u8],
    weight_ise_range: u32,
    num_pixels: usize,
    block_pixels_half: &[[HalfFloat; 3]],
    block_pixels_q16: &[[f32; 4]],
    pixel_block_ofs: Option<&[u8]>,
    coptions: &AstcHdrCodecBaseOptions,
    opt_mode: OptMode,
) -> bool {
    match cem {
        7 => refine_endpoints_mode7(
            endpoint_ise_range,
            endpoint_vals,
            block_w,
            block_h,
            grid_w,
            grid_h,
            weights,
            weight_ise_range,
            num_pixels,
            block_pixels_half,
            block_pixels_q16,
            pixel_block_ofs,
            coptions,
            FIRST_MODE7_SUBMODE_INDEX,
            MAX_MODE7_SUBMODE_INDEX,
        ),
        11 => refine_endpoints_mode11(
            endpoint_ise_range,
            endpoint_vals,
            block_w,
            block_h,
            grid_w,
            grid_h,
            weights,
            weight_ise_range,
            num_pixels,
            block_pixels_half,
            block_pixels_q16,
            pixel_block_ofs,
            coptions,
            false,
            FIRST_MODE11_SUBMODE_INDEX,
            MAX_MODE11_SUBMODE_INDEX,
            opt_mode,
        ),
        _ => false,
    }
}
