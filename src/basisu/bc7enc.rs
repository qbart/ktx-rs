// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of `encoder/basisu_bc7enc.h` / `basisu_bc7enc.cpp`: the BC7/ASTC color cell
//! compressor used by the UASTC LDR 4x4 encoder (`color_cell_compression`,
//! `color_cell_compression_est_astc`) and its lookup tables (`bc7enc_compress_block_init`).

#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

use std::sync::OnceLock;

use super::enc::ColorRgba;
use super::uastc::{
    EndpointErr, TOTAL_ASTC_RANGES, astc_get_levels, astc_interpolate,
    astc_is_valid_endpoint_range, unquant_astc_endpoint_val,
};
use super::uastc_tables::{ASTC_WEIGHTS4, ASTC_WEIGHTS5, BC7_WEIGHTS2, BC7_WEIGHTS3};

pub const BC7ENC_MAX_PARTITIONS1: u32 = 64;
pub const BC7ENC_MAX_UBER_LEVEL: u32 = 4;

/// `bc7enc_vec4F`.
pub type Bc7encVec4F = [f32; 4];

/// `g_bc7_weights1x`: per weight w*w, (1-w)*w, (1-w)*(1-w), w.
pub const BC7_WEIGHTS1X: [[f32; 4]; 2] = [
    [0.000000f32, 0.000000f32, 1.000000f32, 0.000000f32],
    [1.000000f32, 0.000000f32, 0.000000f32, 1.000000f32],
];
/// `g_bc7_weights2x`: per weight w*w, (1-w)*w, (1-w)*(1-w), w.
pub const BC7_WEIGHTS2X: [[f32; 4]; 4] = [
    [0.000000f32, 0.000000f32, 1.000000f32, 0.000000f32],
    [0.107666f32, 0.220459f32, 0.451416f32, 0.328125f32],
    [0.451416f32, 0.220459f32, 0.107666f32, 0.671875f32],
    [1.000000f32, 0.000000f32, 0.000000f32, 1.000000f32],
];
/// `g_bc7_weights3x`: per weight w*w, (1-w)*w, (1-w)*(1-w), w.
pub const BC7_WEIGHTS3X: [[f32; 4]; 8] = [
    [0.000000f32, 0.000000f32, 1.000000f32, 0.000000f32],
    [0.019775f32, 0.120850f32, 0.738525f32, 0.140625f32],
    [0.079102f32, 0.202148f32, 0.516602f32, 0.281250f32],
    [0.177979f32, 0.243896f32, 0.334229f32, 0.421875f32],
    [0.334229f32, 0.243896f32, 0.177979f32, 0.578125f32],
    [0.516602f32, 0.202148f32, 0.079102f32, 0.718750f32],
    [0.738525f32, 0.120850f32, 0.019775f32, 0.859375f32],
    [1.000000f32, 0.000000f32, 0.000000f32, 1.000000f32],
];
/// `g_bc7_weights4x`: per weight w*w, (1-w)*w, (1-w)*(1-w), w.
pub const BC7_WEIGHTS4X: [[f32; 4]; 16] = [
    [0.000000f32, 0.000000f32, 1.000000f32, 0.000000f32],
    [0.003906f32, 0.058594f32, 0.878906f32, 0.062500f32],
    [0.019775f32, 0.120850f32, 0.738525f32, 0.140625f32],
    [0.041260f32, 0.161865f32, 0.635010f32, 0.203125f32],
    [0.070557f32, 0.195068f32, 0.539307f32, 0.265625f32],
    [0.107666f32, 0.220459f32, 0.451416f32, 0.328125f32],
    [0.165039f32, 0.241211f32, 0.352539f32, 0.406250f32],
    [0.219727f32, 0.249023f32, 0.282227f32, 0.468750f32],
    [0.282227f32, 0.249023f32, 0.219727f32, 0.531250f32],
    [0.352539f32, 0.241211f32, 0.165039f32, 0.593750f32],
    [0.451416f32, 0.220459f32, 0.107666f32, 0.671875f32],
    [0.539307f32, 0.195068f32, 0.070557f32, 0.734375f32],
    [0.635010f32, 0.161865f32, 0.041260f32, 0.796875f32],
    [0.738525f32, 0.120850f32, 0.019775f32, 0.859375f32],
    [0.878906f32, 0.058594f32, 0.003906f32, 0.937500f32],
    [1.000000f32, 0.000000f32, 0.000000f32, 1.000000f32],
];
/// `g_astc_weights4x`: per weight w*w, (1-w)*w, (1-w)*(1-w), w.
pub const ASTC_WEIGHTS4X: [[f32; 4]; 16] = [
    [0.000000f32, 0.000000f32, 1.000000f32, 0.000000f32],
    [0.003906f32, 0.058594f32, 0.878906f32, 0.062500f32],
    [0.015625f32, 0.109375f32, 0.765625f32, 0.125000f32],
    [0.035156f32, 0.152344f32, 0.660156f32, 0.187500f32],
    [0.070557f32, 0.195068f32, 0.539307f32, 0.265625f32],
    [0.107666f32, 0.220459f32, 0.451416f32, 0.328125f32],
    [0.152588f32, 0.238037f32, 0.371338f32, 0.390625f32],
    [0.205322f32, 0.247803f32, 0.299072f32, 0.453125f32],
    [0.299072f32, 0.247803f32, 0.205322f32, 0.546875f32],
    [0.371338f32, 0.238037f32, 0.152588f32, 0.609375f32],
    [0.451416f32, 0.220459f32, 0.107666f32, 0.671875f32],
    [0.539307f32, 0.195068f32, 0.070557f32, 0.734375f32],
    [0.660156f32, 0.152344f32, 0.035156f32, 0.812500f32],
    [0.765625f32, 0.109375f32, 0.015625f32, 0.875000f32],
    [0.878906f32, 0.058594f32, 0.003906f32, 0.937500f32],
    [1.000000f32, 0.000000f32, 0.000000f32, 1.000000f32],
];
/// `g_astc_weights5x`: per weight w*w, (1-w)*w, (1-w)*(1-w), w.
pub const ASTC_WEIGHTS5X: [[f32; 4]; 32] = [
    [0.000000f32, 0.000000f32, 1.000000f32, 0.000000f32],
    [0.000977f32, 0.030273f32, 0.938477f32, 0.031250f32],
    [0.003906f32, 0.058594f32, 0.878906f32, 0.062500f32],
    [0.008789f32, 0.084961f32, 0.821289f32, 0.093750f32],
    [0.015625f32, 0.109375f32, 0.765625f32, 0.125000f32],
    [0.024414f32, 0.131836f32, 0.711914f32, 0.156250f32],
    [0.035156f32, 0.152344f32, 0.660156f32, 0.187500f32],
    [0.047852f32, 0.170898f32, 0.610352f32, 0.218750f32],
    [0.062500f32, 0.187500f32, 0.562500f32, 0.250000f32],
    [0.079102f32, 0.202148f32, 0.516602f32, 0.281250f32],
    [0.097656f32, 0.214844f32, 0.472656f32, 0.312500f32],
    [0.118164f32, 0.225586f32, 0.430664f32, 0.343750f32],
    [0.140625f32, 0.234375f32, 0.390625f32, 0.375000f32],
    [0.165039f32, 0.241211f32, 0.352539f32, 0.406250f32],
    [0.191406f32, 0.246094f32, 0.316406f32, 0.437500f32],
    [0.219727f32, 0.249023f32, 0.282227f32, 0.468750f32],
    [0.282227f32, 0.249023f32, 0.219727f32, 0.531250f32],
    [0.316406f32, 0.246094f32, 0.191406f32, 0.562500f32],
    [0.352539f32, 0.241211f32, 0.165039f32, 0.593750f32],
    [0.390625f32, 0.234375f32, 0.140625f32, 0.625000f32],
    [0.430664f32, 0.225586f32, 0.118164f32, 0.656250f32],
    [0.472656f32, 0.214844f32, 0.097656f32, 0.687500f32],
    [0.516602f32, 0.202148f32, 0.079102f32, 0.718750f32],
    [0.562500f32, 0.187500f32, 0.062500f32, 0.750000f32],
    [0.610352f32, 0.170898f32, 0.047852f32, 0.781250f32],
    [0.660156f32, 0.152344f32, 0.035156f32, 0.812500f32],
    [0.711914f32, 0.131836f32, 0.024414f32, 0.843750f32],
    [0.765625f32, 0.109375f32, 0.015625f32, 0.875000f32],
    [0.821289f32, 0.084961f32, 0.008789f32, 0.906250f32],
    [0.878906f32, 0.058594f32, 0.003906f32, 0.937500f32],
    [0.938477f32, 0.030273f32, 0.000977f32, 0.968750f32],
    [1.000000f32, 0.000000f32, 0.000000f32, 1.000000f32],
];
/// `g_astc_weights_3levelsx`.
pub const ASTC_WEIGHTS_3LEVELSX: [[f32; 4]; 3] = [
    [0.0, 0.0, 1.0, 0.0],
    [0.5 * 0.5, (1.0 - 0.5) * 0.5, (1.0 - 0.5) * (1.0 - 0.5), 0.5],
    [1.0, 0.0, 0.0, 1.0],
];

const BC7ENC_MODE_1_OPTIMAL_INDEX: u32 = 2;
const BC7ENC_ASTC_4BIT_3BIT_OPTIMAL_INDEX: u32 = 2;
const BC7ENC_ASTC_4BIT_2BIT_OPTIMAL_INDEX: u32 = 1;
const BC7ENC_ASTC_RANGE7_2BIT_OPTIMAL_INDEX: u32 = 1;
const BC7ENC_ASTC_RANGE13_4BIT_OPTIMAL_INDEX: u32 = 2;
const BC7ENC_ASTC_RANGE13_2BIT_OPTIMAL_INDEX: u32 = 1;
// not 1, which is optimal, because 26 losslessly maps to BC7 4-bit weights
const BC7ENC_ASTC_RANGE11_5BIT_OPTIMAL_INDEX: u32 = 13;

/// `astc_quant_bin`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AstcQuantBin {
    pub unquant: u8,
    pub index: u8,
}

/// Tables built by `bc7enc_compress_block_init` (and `astc_init`).
pub struct Bc7encTables {
    /// `g_astc_sorted_order_unquant[range][sorted index]`.
    pub astc_sorted_order_unquant: [[AstcQuantBin; 256]; TOTAL_ASTC_RANGES],
    /// `g_astc_nearest_sorted_index[range][v]`.
    pub astc_nearest_sorted_index: [[u8; 256]; TOTAL_ASTC_RANGES],
    pub bc7_mode_1_optimal_endpoints: [[EndpointErr; 2]; 256],
    pub astc_4bit_3bit_optimal_endpoints: [EndpointErr; 256],
    pub astc_4bit_2bit_optimal_endpoints: [EndpointErr; 256],
    pub astc_range7_2bit_optimal_endpoints: [EndpointErr; 256],
    pub astc_range13_4bit_optimal_endpoints: [EndpointErr; 256],
    pub astc_range13_2bit_optimal_endpoints: [EndpointErr; 256],
    pub astc_range11_5bit_optimal_endpoints: [EndpointErr; 256],
}

#[inline]
fn astc_interpolate_linear(l: u32, h: u32, w: u32) -> u32 {
    astc_interpolate(l, h, w, false)
}

fn optimal_endpoints(levels: u32, unq: impl Fn(u32) -> u32, w: u32) -> [EndpointErr; 256] {
    let mut out = [EndpointErr::default(); 256];
    for (c, e) in out.iter_mut().enumerate() {
        let mut best = EndpointErr {
            error: u16::MAX,
            lo: 0,
            hi: 0,
        };
        for l in 0..levels {
            let low = unq(l);
            for h in 0..levels {
                let high = unq(h);
                let k = astc_interpolate_linear(low, high, w) as i32;
                let err = (k - c as i32) * (k - c as i32);
                if err < best.error as i32 {
                    best = EndpointErr {
                        error: err as u16,
                        lo: l as u8,
                        hi: h as u8,
                    };
                }
            }
        }
        *e = best;
    }
    out
}

/// `bc7enc_compress_block_init`: the tables are built once on first use.
pub fn bc7enc_tables() -> &'static Bc7encTables {
    static T: OnceLock<Box<Bc7encTables>> = OnceLock::new();
    T.get_or_init(|| {
        // astc_init
        let mut sorted = [[AstcQuantBin::default(); 256]; TOTAL_ASTC_RANGES];
        let mut nearest = [[0u8; 256]; TOTAL_ASTC_RANGES];
        for range in 0..TOTAL_ASTC_RANGES {
            let r = range as u32;
            if !astc_is_valid_endpoint_range(r) {
                continue;
            }
            let levels = astc_get_levels(r).min(256) as usize;
            let mut vals: Vec<u32> = (0..levels as u32)
                .map(|i| (unquant_astc_endpoint_val(i, r) << 8) | i)
                .collect();
            vals.sort_unstable();
            for (i, &v) in vals.iter().enumerate() {
                sorted[range][i] = AstcQuantBin {
                    unquant: (v >> 8) as u8,
                    index: (v & 0xFF) as u8,
                };
            }
            for i in 0..256usize {
                let mut best_index = 0usize;
                let mut best_err = i32::MAX;
                for j in 0..levels {
                    let err = (sorted[range][j].unquant as i32 - i as i32).abs();
                    if err < best_err {
                        best_err = err;
                        best_index = j;
                    }
                }
                nearest[range][i] = best_index as u8;
            }
        }

        // BC7 666.1
        let mut m1 = [[EndpointErr::default(); 2]; 256];
        let w1 = BC7_WEIGHTS3[BC7ENC_MODE_1_OPTIMAL_INDEX as usize] as i32;
        for (c, slot) in m1.iter_mut().enumerate() {
            for (lp, e) in slot.iter_mut().enumerate() {
                let mut best = EndpointErr {
                    error: u16::MAX,
                    lo: 0,
                    hi: 0,
                };
                for l in 0..64i32 {
                    let mut low = ((l << 1) | lp as i32) << 1;
                    low |= low >> 7;
                    for h in 0..64i32 {
                        let mut high = ((h << 1) | lp as i32) << 1;
                        high |= high >> 7;
                        let k = (low * (64 - w1) + high * w1 + 32) >> 6;
                        let err = (k - c as i32) * (k - c as i32);
                        if err < best.error as i32 {
                            best = EndpointErr {
                                error: err as u16,
                                lo: l as u8,
                                hi: h as u8,
                            };
                        }
                    }
                }
                *e = best;
            }
        }

        let s7 = sorted[7];
        let s13 = sorted[13];
        let s11 = sorted[11];
        Box::new(Bc7encTables {
            astc_4bit_3bit_optimal_endpoints: optimal_endpoints(
                16,
                |l| (l << 4) | l,
                BC7_WEIGHTS3[BC7ENC_ASTC_4BIT_3BIT_OPTIMAL_INDEX as usize],
            ),
            astc_4bit_2bit_optimal_endpoints: optimal_endpoints(
                16,
                |l| (l << 4) | l,
                BC7_WEIGHTS2[BC7ENC_ASTC_4BIT_2BIT_OPTIMAL_INDEX as usize],
            ),
            astc_range7_2bit_optimal_endpoints: optimal_endpoints(
                12,
                |l| s7[l as usize].unquant as u32,
                BC7_WEIGHTS2[BC7ENC_ASTC_RANGE7_2BIT_OPTIMAL_INDEX as usize],
            ),
            astc_range13_4bit_optimal_endpoints: optimal_endpoints(
                48,
                |l| s13[l as usize].unquant as u32,
                ASTC_WEIGHTS4[BC7ENC_ASTC_RANGE13_4BIT_OPTIMAL_INDEX as usize],
            ),
            astc_range13_2bit_optimal_endpoints: optimal_endpoints(
                48,
                |l| s13[l as usize].unquant as u32,
                BC7_WEIGHTS2[BC7ENC_ASTC_RANGE13_2BIT_OPTIMAL_INDEX as usize],
            ),
            astc_range11_5bit_optimal_endpoints: optimal_endpoints(
                32,
                |l| s11[l as usize].unquant as u32,
                ASTC_WEIGHTS5[BC7ENC_ASTC_RANGE11_5BIT_OPTIMAL_INDEX as usize],
            ),
            astc_sorted_order_unquant: sorted,
            astc_nearest_sorted_index: nearest,
            bc7_mode_1_optimal_endpoints: m1,
        })
    })
}

/// `color_cell_compressor_params`. `selector_weightsx` is the matching `g_*_weights*x` table.
#[derive(Clone, Copy)]
pub struct ColorCellCompressorParams<'a> {
    pub num_pixels: u32,
    pub pixels: &'a [ColorRgba],
    pub num_selector_weights: u32,
    pub selector_weights: &'a [u32],
    pub selector_weightsx: &'a [Bc7encVec4F],
    pub comp_bits: u32,
    pub force_selectors: Option<&'a [u8]>,
    pub astc_endpoint_range: u32,
    pub weights: [u32; 4],
    pub has_alpha: bool,
    pub has_pbits: bool,
    pub endpoints_share_pbit: bool,
    pub perceptual: bool,
}

impl Default for ColorCellCompressorParams<'_> {
    /// The `memset(&ccell_params, 0, ...)` state.
    fn default() -> Self {
        ColorCellCompressorParams {
            num_pixels: 0,
            pixels: &[],
            num_selector_weights: 0,
            selector_weights: &[],
            selector_weightsx: &[],
            comp_bits: 0,
            force_selectors: None,
            astc_endpoint_range: 0,
            weights: [0; 4],
            has_alpha: false,
            has_pbits: false,
            endpoints_share_pbit: false,
            perceptual: false,
        }
    }
}

/// `color_cell_compressor_results`; the C selector pointers are owned 16 entry arrays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColorCellCompressorResults {
    pub best_overall_err: u64,
    pub low_endpoint: ColorRgba,
    pub high_endpoint: ColorRgba,
    pub pbits: [u32; 2],
    pub selectors: [u8; 16],
    pub selectors_temp: [u8; 16],
    pub astc_low_endpoint: ColorRgba,
    pub astc_high_endpoint: ColorRgba,
}

/// `bc7enc_compress_block_params`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bc7encCompressBlockParams {
    pub max_partitions_mode1: u32,
    pub weights: [u32; 4],
    pub uber_level: u32,
    pub perceptual: bool,
    pub least_squares_passes: u32,
}

impl Default for Bc7encCompressBlockParams {
    /// `bc7enc_compress_block_params_init`.
    fn default() -> Self {
        let mut p = Bc7encCompressBlockParams {
            max_partitions_mode1: BC7ENC_MAX_PARTITIONS1,
            weights: [0; 4],
            uber_level: 0,
            perceptual: false,
            least_squares_passes: 1,
        };
        bc7enc_compress_block_params_init_perceptual_weights(&mut p);
        p
    }
}

/// `bc7enc_compress_block_params_init_linear_weights`.
pub fn bc7enc_compress_block_params_init_linear_weights(p: &mut Bc7encCompressBlockParams) {
    p.perceptual = false;
    p.weights = [1, 1, 1, 1];
}

/// `bc7enc_compress_block_params_init_perceptual_weights`.
pub fn bc7enc_compress_block_params_init_perceptual_weights(p: &mut Bc7encCompressBlockParams) {
    p.perceptual = true;
    p.weights = [128, 64, 16, 32];
}

/// `bc7enc_compress_block_params_init`.
pub fn bc7enc_compress_block_params_init(p: &mut Bc7encCompressBlockParams) {
    *p = Bc7encCompressBlockParams::default();
}

#[inline]
fn clampi(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

#[inline]
fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

#[inline]
fn saturate(v: f32) -> f32 {
    clampf(v, 0.0, 1.0)
}

#[inline]
fn minimumf(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

#[inline]
fn maximumf(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

#[inline]
fn set_clamped(r: i32, g: i32, b: i32, a: i32) -> ColorRgba {
    [
        clampi(r, 0, 255) as u8,
        clampi(g, 0, 255) as u8,
        clampi(b, 0, 255) as u8,
        clampi(a, 0, 255) as u8,
    ]
}

#[inline]
fn vec4f_from_color(c: &ColorRgba) -> Bc7encVec4F {
    [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32]
}

#[inline]
fn vec4f_add(a: &Bc7encVec4F, b: &Bc7encVec4F) -> Bc7encVec4F {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

#[inline]
fn vec4f_sub(a: &Bc7encVec4F, b: &Bc7encVec4F) -> Bc7encVec4F {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}

#[inline]
fn vec4f_dot(a: &Bc7encVec4F, b: &Bc7encVec4F) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

#[inline]
fn vec4f_mul(a: &Bc7encVec4F, s: f32) -> Bc7encVec4F {
    [a[0] * s, a[1] * s, a[2] * s, a[3] * s]
}

#[inline]
fn vec4f_saturate(v: &Bc7encVec4F) -> Bc7encVec4F {
    [
        saturate(v[0]),
        saturate(v[1]),
        saturate(v[2]),
        saturate(v[3]),
    ]
}

#[inline]
fn vec4f_normalize_in_place(v: &mut Bc7encVec4F) {
    let mut s = v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3];
    if s != 0.0 {
        s = 1.0 / s.sqrt();
        v[0] *= s;
        v[1] *= s;
        v[2] *= s;
        v[3] *= s;
    }
}

/// `compute_least_squares_endpoints_rgba` / `_rgb` (`num_comps` 4 or 3).
fn compute_least_squares_endpoints(
    num_comps: usize,
    n: u32,
    selectors: &[u8],
    selector_weights: &[Bc7encVec4F],
    colors: &[ColorRgba],
) -> (Bc7encVec4F, Bc7encVec4F) {
    // Least squares using normal equations: http://www.cs.cornell.edu/~bindel/class/cs3220-s12/notes/lec10.pdf
    let (mut z00, mut z10, mut z11) = (0.0f64, 0.0f64, 0.0f64);
    let mut q00 = [0.0f64; 4];
    let mut t = [0.0f64; 4];
    for i in 0..n as usize {
        let sel = selectors[i] as usize;
        let sw = selector_weights[sel];
        z00 += sw[0] as f64;
        z10 += sw[1] as f64;
        z11 += sw[2] as f64;
        let w = sw[3];
        for c in 0..num_comps {
            q00[c] += (w * colors[i][c] as f32) as f64;
            t[c] += colors[i][c] as f64;
        }
    }
    let z01 = z10;
    let mut det = z00 * z11 - z01 * z10;
    if det != 0.0 {
        det = 1.0 / det;
    }
    let iz00 = z11 * det;
    let iz01 = -z01 * det;
    let iz10 = -z10 * det;
    let iz11 = z00 * det;

    let mut xl = [255.0f32; 4];
    let mut xh = [255.0f32; 4];
    for c in 0..num_comps {
        let q10 = t[c] - q00[c];
        xl[c] = (iz00 * q00[c] + iz01 * q10) as f32;
        xh[c] = (iz10 * q00[c] + iz11 * q10) as f32;
    }

    for c in 0..num_comps {
        if xl[c] < 0.0 || xh[c] > 255.0 {
            let mut lo_v = u32::MAX;
            let mut hi_v = 0u32;
            for i in 0..n as usize {
                lo_v = lo_v.min(colors[i][c] as u32);
                hi_v = hi_v.max(colors[i][c] as u32);
            }
            if lo_v == hi_v {
                xl[c] = lo_v as f32;
                xh[c] = hi_v as f32;
            }
        }
    }
    (xl, xh)
}

#[inline]
fn least_squares(p: &ColorCellCompressorParams, selectors: &[u8]) -> (Bc7encVec4F, Bc7encVec4F) {
    let (xl, xh) = compute_least_squares_endpoints(
        if p.has_alpha { 4 } else { 3 },
        p.num_pixels,
        selectors,
        p.selector_weightsx,
        p.pixels,
    );
    (vec4f_mul(&xl, 1.0 / 255.0), vec4f_mul(&xh, 1.0 / 255.0))
}

#[inline]
fn scale_color(c: &ColorRgba, p: &ColorCellCompressorParams) -> ColorRgba {
    let mut r = [0u8; 4];
    if p.astc_endpoint_range != 0 {
        let t = &bc7enc_tables().astc_sorted_order_unquant[p.astc_endpoint_range as usize];
        for i in 0..4 {
            r[i] = t[c[i] as usize].unquant;
        }
    } else {
        let n = p.comp_bits + if p.has_pbits { 1 } else { 0 };
        for i in 0..4 {
            let mut v = (c[i] as u32) << (8 - n);
            v |= v >> n;
            r[i] = v as u8;
        }
    }
    r
}

#[inline]
fn compute_color_distance_rgb(
    e1: &ColorRgba,
    e2: &ColorRgba,
    perceptual: bool,
    weights: &[u32; 4],
) -> u64 {
    let (dr, dg, db);
    if perceptual {
        let l1 = e1[0] as i32 * 109 + e1[1] as i32 * 366 + e1[2] as i32 * 37;
        let cr1 = ((e1[0] as i32) << 9) - l1;
        let cb1 = ((e1[2] as i32) << 9) - l1;
        let l2 = e2[0] as i32 * 109 + e2[1] as i32 * 366 + e2[2] as i32 * 37;
        let cr2 = ((e2[0] as i32) << 9) - l2;
        let cb2 = ((e2[2] as i32) << 9) - l2;
        dr = (l1 - l2) >> 8;
        dg = (cr1 - cr2) >> 8;
        db = (cb1 - cb2) >> 8;
    } else {
        dr = e1[0] as i32 - e2[0] as i32;
        dg = e1[1] as i32 - e2[1] as i32;
        db = e1[2] as i32 - e2[2] as i32;
    }
    weights[0]
        .wrapping_mul((dr * dr) as u32)
        .wrapping_add(weights[1].wrapping_mul((dg * dg) as u32))
        .wrapping_add(weights[2].wrapping_mul((db * db) as u32)) as u64
}

#[inline]
fn compute_color_distance_rgba(
    e1: &ColorRgba,
    e2: &ColorRgba,
    perceptual: bool,
    weights: &[u32; 4],
) -> u64 {
    let da = e1[3] as i32 - e2[3] as i32;
    compute_color_distance_rgb(e1, e2, perceptual, weights)
        + weights[3].wrapping_mul((da * da) as u32) as u64
}

/// Single color packers (`pack_mode1_to_one_color`, `pack_astc_*_to_one_color*`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum OneColor {
    Mode1,
    Astc4Bit3Bit,
    Astc4Bit2BitRgba,
    AstcRange7_2Bit,
    AstcRange13_2Bit,
    AstcRange11_5Bit,
}

/// Picks the single color packer `color_cell_compression` uses for these params.
fn one_color_kind(mode: u32, p: &ColorCellCompressorParams) -> Option<OneColor> {
    let (r, n, a) = (p.astc_endpoint_range, p.num_selector_weights, p.has_alpha);
    if mode == 1 {
        Some(OneColor::Mode1)
    } else if r == 8 && n == 8 && !a {
        Some(OneColor::Astc4Bit3Bit)
    } else if r == 7 && n == 4 && !a {
        Some(OneColor::AstcRange7_2Bit)
    } else if r == 8 && n == 4 && a {
        Some(OneColor::Astc4Bit2BitRgba)
    } else if r == 13 && n == 4 && !a {
        Some(OneColor::AstcRange13_2Bit)
    } else if r == 11 && n == 32 && !a {
        Some(OneColor::AstcRange11_5Bit)
    } else {
        None
    }
}

/// Packs the partition as the single color `c`; the selectors (all set to the returned
/// index) are left to the caller.
fn pack_to_one_color(
    kind: OneColor,
    p: &ColorCellCompressorParams,
    res: &mut ColorCellCompressorResults,
    c: [u32; 4],
) -> (u64, u8) {
    let t = bc7enc_tables();
    let mut px = [0u8; 4];
    let sel;
    if kind == OneColor::Mode1 {
        let tab = &t.bc7_mode_1_optimal_endpoints;
        let mut best_err = u32::MAX;
        let mut best_p = 0usize;
        for pb in 0..2 {
            let err = tab[c[0] as usize][pb].error as u32
                + tab[c[1] as usize][pb].error as u32
                + tab[c[2] as usize][pb].error as u32;
            if err < best_err {
                best_err = err;
                best_p = pb;
            }
        }
        let e = [
            tab[c[0] as usize][best_p],
            tab[c[1] as usize][best_p],
            tab[c[2] as usize][best_p],
        ];
        res.low_endpoint = [e[0].lo, e[1].lo, e[2].lo, 0];
        res.high_endpoint = [e[0].hi, e[1].hi, e[2].hi, 0];
        res.pbits = [best_p as u32, 0];
        sel = BC7ENC_MODE_1_OPTIMAL_INDEX;
        let w = BC7_WEIGHTS3[sel as usize];
        for i in 0..3 {
            let mut low = (((res.low_endpoint[i] as u32) << 1) | res.pbits[0]) << 1;
            low |= low >> 7;
            let mut high = (((res.high_endpoint[i] as u32) << 1) | res.pbits[0]) << 1;
            high |= high >> 7;
            px[i] = ((low * (64 - w) + high * w + 32) >> 6) as u8;
        }
        px[3] = 255;
    } else {
        let (tab, s, w, alpha_ep, nc) = match kind {
            OneColor::Astc4Bit3Bit => (
                &t.astc_4bit_3bit_optimal_endpoints,
                BC7ENC_ASTC_4BIT_3BIT_OPTIMAL_INDEX,
                BC7_WEIGHTS3[BC7ENC_ASTC_4BIT_3BIT_OPTIMAL_INDEX as usize],
                None,
                3,
            ),
            OneColor::Astc4Bit2BitRgba => (
                &t.astc_4bit_2bit_optimal_endpoints,
                BC7ENC_ASTC_4BIT_2BIT_OPTIMAL_INDEX,
                BC7_WEIGHTS2[BC7ENC_ASTC_4BIT_2BIT_OPTIMAL_INDEX as usize],
                None,
                4,
            ),
            OneColor::AstcRange7_2Bit => (
                &t.astc_range7_2bit_optimal_endpoints,
                BC7ENC_ASTC_RANGE7_2BIT_OPTIMAL_INDEX,
                BC7_WEIGHTS2[BC7ENC_ASTC_RANGE7_2BIT_OPTIMAL_INDEX as usize],
                None,
                3,
            ),
            OneColor::AstcRange13_2Bit => (
                &t.astc_range13_2bit_optimal_endpoints,
                BC7ENC_ASTC_RANGE13_2BIT_OPTIMAL_INDEX,
                BC7_WEIGHTS2[BC7ENC_ASTC_RANGE13_2BIT_OPTIMAL_INDEX as usize],
                Some(47u8),
                4,
            ),
            _ => (
                &t.astc_range11_5bit_optimal_endpoints,
                BC7ENC_ASTC_RANGE11_5BIT_OPTIMAL_INDEX,
                ASTC_WEIGHTS5[BC7ENC_ASTC_RANGE11_5BIT_OPTIMAL_INDEX as usize],
                Some(31u8),
                4,
            ),
        };
        let er = tab[c[0] as usize];
        let eg = tab[c[1] as usize];
        let eb = tab[c[2] as usize];
        let (la, ha) = match (kind, alpha_ep) {
            (OneColor::Astc4Bit2BitRgba, _) => {
                let ea = tab[c[3] as usize];
                (ea.lo, ea.hi)
            }
            (_, Some(v)) => (v, v),
            _ => (0, 0),
        };
        res.low_endpoint = [er.lo, eg.lo, eb.lo, la];
        res.high_endpoint = [er.hi, eg.hi, eb.hi, ha];
        res.pbits = [0, 0];
        let so = &t.astc_sorted_order_unquant[p.astc_endpoint_range as usize];
        for i in 0..4 {
            res.astc_low_endpoint[i] = so[res.low_endpoint[i] as usize].index;
            res.astc_high_endpoint[i] = so[res.high_endpoint[i] as usize].index;
        }
        sel = s;
        // The 4-bit packers expand by bit replication, the others use their range's table.
        let unq = |range: usize, v: u8| -> u32 {
            match kind {
                OneColor::Astc4Bit3Bit | OneColor::Astc4Bit2BitRgba => ((v as u32) << 4) | v as u32,
                _ => t.astc_sorted_order_unquant[range][v as usize].unquant as u32,
            }
        };
        let range = match kind {
            OneColor::AstcRange7_2Bit => 7,
            OneColor::AstcRange13_2Bit => 13,
            _ => 11,
        };
        for i in 0..nc {
            let low = unq(range, res.low_endpoint[i]);
            let high = unq(range, res.high_endpoint[i]);
            px[i] = astc_interpolate_linear(low, high, w) as u8;
        }
        if nc == 3 {
            px[3] = 255;
        }
    }

    let rgba = kind == OneColor::Astc4Bit2BitRgba;
    let mut total_err = 0u64;
    for i in 0..p.num_pixels as usize {
        total_err += if rgba {
            compute_color_distance_rgba(&px, &p.pixels[i], p.perceptual, &p.weights)
        } else {
            compute_color_distance_rgb(&px, &p.pixels[i], p.perceptual, &p.weights)
        };
    }
    res.best_overall_err = total_err;
    (total_err, sel as u8)
}

fn evaluate_solution(
    low: &ColorRgba,
    high: &ColorRgba,
    pbits: [u32; 2],
    p: &ColorCellCompressorParams,
    res: &mut ColorCellCompressorResults,
) -> u64 {
    let mut quant_min = *low;
    let mut quant_max = *high;
    if p.has_pbits {
        let (min_pbit, max_pbit) = if p.endpoints_share_pbit {
            (pbits[0], pbits[0])
        } else {
            (pbits[0], pbits[1])
        };
        for c in 0..4 {
            quant_min[c] = ((low[c] as u32) << 1 | min_pbit) as u8;
            quant_max[c] = ((high[c] as u32) << 1 | max_pbit) as u8;
        }
    }
    let actual_min = scale_color(&quant_min, p);
    let actual_max = scale_color(&quant_max, p);

    let n = p.num_selector_weights as usize;
    let mut weighted = [[0u8; 4]; 32];
    weighted[0] = actual_min;
    weighted[n - 1] = actual_max;
    let nc = if p.has_alpha { 4 } else { 3 };
    for i in 1..n - 1 {
        let w = p.selector_weights[i];
        for j in 0..nc {
            weighted[i][j] = if p.astc_endpoint_range != 0 {
                astc_interpolate_linear(actual_min[j] as u32, actual_max[j] as u32, w) as u8
            } else {
                ((actual_min[j] as u32 * (64 - w) + actual_max[j] as u32 * w + 32) >> 6) as u8
            };
        }
    }

    let lr = actual_min[0] as i32;
    let lg = actual_min[1] as i32;
    let lb = actual_min[2] as i32;
    let dr = actual_max[0] as i32 - lr;
    let dg = actual_max[1] as i32 - lg;
    let db = actual_max[2] as i32 - lb;

    let mut total_err = 0u64;
    let np = p.num_pixels as usize;
    if let Some(force) = p.force_selectors {
        for i in 0..np {
            let c = &p.pixels[i];
            let sel = force[i];
            total_err += if p.has_alpha {
                compute_color_distance_rgba(&weighted[sel as usize], c, p.perceptual, &p.weights)
            } else {
                compute_color_distance_rgb(&weighted[sel as usize], c, p.perceptual, &p.weights)
            };
            res.selectors_temp[i] = sel;
        }
    } else if !p.perceptual {
        if p.has_alpha {
            let la = actual_min[3] as i32;
            let da = actual_max[3] as i32 - la;
            let f = n as f32 / ((dr * dr + dg * dg + db * db + da * da) as f32 + 0.00000125f32);
            for i in 0..np {
                let c = &p.pixels[i];
                let (r, g, b, a) = (c[0] as i32, c[1] as i32, c[2] as i32, c[3] as i32);
                let mut best_sel =
                    (((r - lr) * dr + (g - lg) * dg + (b - lb) * db + (a - la) * da) as f32 * f
                        + 0.5) as i32;
                best_sel = clampi(best_sel, 1, n as i32 - 1);
                let err0 = compute_color_distance_rgba(
                    &weighted[best_sel as usize - 1],
                    c,
                    false,
                    &p.weights,
                );
                let mut err1 =
                    compute_color_distance_rgba(&weighted[best_sel as usize], c, false, &p.weights);
                if err0 == err1 {
                    // Prefer non-interpolation
                    if best_sel - 1 == 0 {
                        best_sel = 0;
                    }
                } else if err1 > err0 {
                    err1 = err0;
                    best_sel -= 1;
                }
                total_err += err1;
                res.selectors_temp[i] = best_sel as u8;
            }
        } else {
            let f = n as f32 / ((dr * dr + dg * dg + db * db) as f32 + 0.00000125f32);
            for i in 0..np {
                let c = &p.pixels[i];
                let (r, g, b) = (c[0] as i32, c[1] as i32, c[2] as i32);
                let mut sel =
                    (((r - lr) * dr + (g - lg) * dg + (b - lb) * db) as f32 * f + 0.5) as i32;
                sel = clampi(sel, 1, n as i32 - 1);
                let err0 =
                    compute_color_distance_rgb(&weighted[sel as usize - 1], c, false, &p.weights);
                let err1 =
                    compute_color_distance_rgb(&weighted[sel as usize], c, false, &p.weights);
                let mut best_sel = sel;
                let mut best_err = err1;
                if err0 == err1 {
                    // Prefer non-interpolation
                    if best_sel - 1 == 0 {
                        best_sel = 0;
                    }
                } else if err0 < best_err {
                    best_err = err0;
                    best_sel = sel - 1;
                }
                total_err += best_err;
                res.selectors_temp[i] = best_sel as u8;
            }
        }
    } else {
        for i in 0..np {
            let mut best_err = u64::MAX;
            let mut best_sel = 0usize;
            for j in 0..n {
                let err = if p.has_alpha {
                    compute_color_distance_rgba(&weighted[j], &p.pixels[i], true, &p.weights)
                } else {
                    compute_color_distance_rgb(&weighted[j], &p.pixels[i], true, &p.weights)
                };
                if err < best_err {
                    best_err = err;
                    best_sel = j;
                } else if err == best_err && j == n - 1 {
                    // Prefer non-interpolation
                    best_sel = j;
                }
            }
            total_err += best_err;
            res.selectors_temp[i] = best_sel as u8;
        }
    }

    if total_err < res.best_overall_err {
        res.best_overall_err = total_err;
        res.low_endpoint = *low;
        res.high_endpoint = *high;
        res.pbits = pbits;
        res.selectors[..np].copy_from_slice(&res.selectors_temp[..np]);
    }
    total_err
}

fn are_degenerate_endpoints(
    tmin: &ColorRgba,
    tmax: &ColorRgba,
    xl: &Bc7encVec4F,
    xh: &Bc7encVec4F,
) -> bool {
    (0..3).any(|i| tmin[i] == tmax[i] && (xl[i] - xh[i]).abs() > 0.0)
}

fn fix_degenerate_endpoints(
    mode: u32,
    tmin: &mut ColorRgba,
    tmax: &mut ColorRgba,
    xl: &Bc7encVec4F,
    xh: &Bc7encVec4F,
    iscale: u32,
    flags: i32,
) {
    if mode == 255 {
        for i in 0..3 {
            if tmin[i] == tmax[i] && (xl[i] - xh[i]).abs() > 0.000125 {
                if flags & 1 != 0 && tmin[i] > 0 {
                    tmin[i] -= 1;
                }
                if flags & 2 != 0 && (tmax[i] as u32) < iscale {
                    tmax[i] += 1;
                }
            }
        }
    } else if mode == 1 {
        // fix degenerate case where the input collapses to a single colorspace voxel, and we
        // loose all freedom (test with grayscale ramps)
        for i in 0..3 {
            if tmin[i] == tmax[i] && (xl[i] - xh[i]).abs() > 0.000125 {
                if tmin[i] as u32 > (iscale >> 1) {
                    if tmin[i] > 0 {
                        tmin[i] -= 1;
                    } else if (tmax[i] as u32) < iscale {
                        tmax[i] += 1;
                    }
                } else if (tmax[i] as u32) < iscale {
                    tmax[i] += 1;
                } else if tmin[i] > 0 {
                    tmin[i] -= 1;
                }
            }
        }
    }
}

#[inline]
fn differs(res: &ColorCellCompressorResults, lo: &ColorRgba, hi: &ColorRgba) -> bool {
    res.best_overall_err == u64::MAX || *lo != res.low_endpoint || *hi != res.high_endpoint
}

fn find_optimal_solution(
    mode: u32,
    mut xl: Bc7encVec4F,
    mut xh: Bc7encVec4F,
    p: &ColorCellCompressorParams,
    res: &mut ColorCellCompressorResults,
) -> u64 {
    xl = vec4f_saturate(&xl);
    xh = vec4f_saturate(&xh);

    if p.astc_endpoint_range != 0 {
        let t = bc7enc_tables();
        let range = p.astc_endpoint_range as usize;
        let levels = astc_get_levels(p.astc_endpoint_range);
        let scale = 255.0f32;
        let q = |v: &Bc7encVec4F| {
            set_clamped(
                (v[0] * scale + 0.5) as i32,
                (v[1] * scale + 0.5) as i32,
                (v[2] * scale + 0.5) as i32,
                (v[3] * scale + 0.5) as i32,
            )
        };
        let min8 = q(&xl);
        let max8 = q(&xh);
        let mut tmin = [0u8; 4];
        let mut tmax = [0u8; 4];
        for i in 0..4 {
            tmin[i] = t.astc_nearest_sorted_index[range][min8[i] as usize];
            tmax[i] = t.astc_nearest_sorted_index[range][max8[i] as usize];
        }

        if are_degenerate_endpoints(&tmin, &tmax, &xl, &xh) {
            for flags in [1, 0, 2, 3] {
                let mut a = tmin;
                let mut b = tmax;
                fix_degenerate_endpoints(mode, &mut a, &mut b, &xl, &xh, levels - 1, flags);
                if differs(res, &a, &b) {
                    evaluate_solution(&a, &b, res.pbits, p, res);
                }
            }
        } else if differs(res, &tmin, &tmax) {
            evaluate_solution(&tmin, &tmax, res.pbits, p, res);
        }

        let so = &t.astc_sorted_order_unquant[range];
        for i in 0..4 {
            res.astc_low_endpoint[i] = so[res.low_endpoint[i] as usize].index;
            res.astc_high_endpoint[i] = so[res.high_endpoint[i] as usize].index;
        }
    } else if p.has_pbits {
        let iscalep = (1i32 << (p.comp_bits + 1)) - 1;
        let scalep = iscalep as f32;
        let total_comps = if p.has_alpha { 4 } else { 3 };
        let mut best_pbits = [0u32; 2];
        let mut best_min = [0u8; 4];
        let mut best_max = [0u8; 4];

        let quant = |v: f32, pb: i32| -> u8 {
            clampi(
                ((v * scalep - pb as f32) / 2.0 + 0.5) as i32 * 2 + pb,
                pb,
                iscalep - 1 + pb,
            ) as u8
        };

        if !p.endpoints_share_pbit {
            let mut best_err0 = 1e9f32;
            let mut best_err1 = 1e9f32;
            for pb in 0..2i32 {
                let mut xmin = [0u8; 4];
                let mut xmax = [0u8; 4];
                for c in 0..4 {
                    xmin[c] = quant(xl[c], pb);
                    xmax[c] = quant(xh[c], pb);
                }
                let scaled_low = scale_color(&xmin, p);
                let scaled_high = scale_color(&xmax, p);
                let mut err0 = 0.0f32;
                let mut err1 = 0.0f32;
                for i in 0..total_comps {
                    let d0 = scaled_low[i] as f32 - xl[i] * 255.0;
                    let d1 = scaled_high[i] as f32 - xh[i] * 255.0;
                    err0 += d0 * d0;
                    err1 += d1 * d1;
                }
                if err0 < best_err0 {
                    best_err0 = err0;
                    best_pbits[0] = pb as u32;
                    for c in 0..4 {
                        best_min[c] = xmin[c] >> 1;
                    }
                }
                if err1 < best_err1 {
                    best_err1 = err1;
                    best_pbits[1] = pb as u32;
                    for c in 0..4 {
                        best_max[c] = xmax[c] >> 1;
                    }
                }
            }
        } else {
            // Endpoints share pbits
            let mut best_err = 1e9f32;
            for pb in 0..2i32 {
                let mut xmin = [0u8; 4];
                let mut xmax = [0u8; 4];
                for c in 0..4 {
                    xmin[c] = quant(xl[c], pb);
                    xmax[c] = quant(xh[c], pb);
                }
                let scaled_low = scale_color(&xmin, p);
                let scaled_high = scale_color(&xmax, p);
                let mut err = 0.0f32;
                for i in 0..total_comps {
                    let d0 = (scaled_low[i] as f32 / 255.0) - xl[i];
                    let d1 = (scaled_high[i] as f32 / 255.0) - xh[i];
                    err += d0 * d0 + d1 * d1;
                }
                if err < best_err {
                    best_err = err;
                    best_pbits = [pb as u32, pb as u32];
                    for j in 0..4 {
                        best_min[j] = xmin[j] >> 1;
                        best_max[j] = xmax[j] >> 1;
                    }
                }
            }
        }

        fix_degenerate_endpoints(
            mode,
            &mut best_min,
            &mut best_max,
            &xl,
            &xh,
            (iscalep >> 1) as u32,
            0,
        );

        if differs(res, &best_min, &best_max) || best_pbits != res.pbits {
            evaluate_solution(&best_min, &best_max, best_pbits, p, res);
        }
    } else {
        let iscale = (1i32 << p.comp_bits) - 1;
        let scale = iscale as f32;
        let q = |v: &Bc7encVec4F| {
            set_clamped(
                (v[0] * scale + 0.5) as i32,
                (v[1] * scale + 0.5) as i32,
                (v[2] * scale + 0.5) as i32,
                (v[3] * scale + 0.5) as i32,
            )
        };
        let mut tmin = q(&xl);
        let mut tmax = q(&xh);
        fix_degenerate_endpoints(mode, &mut tmin, &mut tmax, &xl, &xh, iscale as u32, 0);
        if differs(res, &tmin, &tmax) {
            evaluate_solution(&tmin, &tmax, res.pbits, p, res);
        }
    }
    res.best_overall_err
}

fn is_solid(p: &ColorCellCompressorParams, nc: usize) -> Option<[u32; 4]> {
    let c0 = p.pixels[0];
    for i in 1..p.num_pixels as usize {
        if p.pixels[i][..nc] != c0[..nc] {
            return None;
        }
    }
    Some([c0[0] as u32, c0[1] as u32, c0[2] as u32, c0[3] as u32])
}

/// `color_cell_compression`.
pub fn color_cell_compression(
    mode: u32,
    p: &ColorCellCompressorParams,
    res: &mut ColorCellCompressorResults,
    comp: &Bc7encCompressBlockParams,
) -> u64 {
    res.best_overall_err = u64::MAX;
    let np = p.num_pixels as usize;
    let one_color = one_color_kind(mode, p);

    // If the partition's colors are all the same, then just pack them as a single color.
    if p.force_selectors.is_none()
        && let Some(kind) = one_color
    {
        let nc = if kind == OneColor::Astc4Bit2BitRgba {
            4
        } else {
            3
        };
        if let Some(c) = is_solid(p, nc) {
            let (err, sel) = pack_to_one_color(kind, p, res, c);
            res.selectors[..np].fill(sel);
            return err;
        }
    }

    // Compute partition's mean color and principle axis.
    let mut mean_color = [0.0f32; 4];
    for i in 0..np {
        mean_color = vec4f_add(&mean_color, &vec4f_from_color(&p.pixels[i]));
    }
    let mean_color_scaled = vec4f_mul(&mean_color, 1.0 / p.num_pixels as f32);
    mean_color = vec4f_mul(&mean_color, 1.0 / (p.num_pixels as f32 * 255.0));
    mean_color = vec4f_saturate(&mean_color);

    let mut axis;
    if p.has_alpha {
        // Use incremental PCA for RGBA PCA, because it's simple.
        axis = [0.0f32; 4];
        for i in 0..np {
            let color = vec4f_sub(&vec4f_from_color(&p.pixels[i]), &mean_color_scaled);
            let a = vec4f_mul(&color, color[0]);
            let b = vec4f_mul(&color, color[1]);
            let c = vec4f_mul(&color, color[2]);
            let d = vec4f_mul(&color, color[3]);
            let mut n = if i != 0 { axis } else { color };
            vec4f_normalize_in_place(&mut n);
            axis[0] += vec4f_dot(&a, &n);
            axis[1] += vec4f_dot(&b, &n);
            axis[2] += vec4f_dot(&c, &n);
            axis[3] += vec4f_dot(&d, &n);
        }
        vec4f_normalize_in_place(&mut axis);
    } else {
        // Use covar technique for RGB PCA, because it doesn't require per-pixel normalization.
        let mut cov = [0.0f32; 6];
        for i in 0..np {
            let v = &p.pixels[i];
            let r = v[0] as f32 - mean_color_scaled[0];
            let g = v[1] as f32 - mean_color_scaled[1];
            let b = v[2] as f32 - mean_color_scaled[2];
            cov[0] += r * r;
            cov[1] += r * g;
            cov[2] += r * b;
            cov[3] += g * g;
            cov[4] += g * b;
            cov[5] += b * b;
        }
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
        if len < 1e-10 {
            axis = [0.0; 4];
        } else {
            len = 1.0 / len.sqrt();
            xr *= len;
            xg *= len;
            xb *= len;
            axis = [xr, xg, xb, 0.0];
        }
    }

    if vec4f_dot(&axis, &axis) < 0.5 {
        axis = if p.perceptual {
            [0.213, 0.715, 0.072, if p.has_alpha { 0.715 } else { 0.0 }]
        } else {
            [1.0, 1.0, 1.0, if p.has_alpha { 1.0 } else { 0.0 }]
        };
        vec4f_normalize_in_place(&mut axis);
    }

    let mut l = 1e9f32;
    let mut h = -1e9f32;
    for i in 0..np {
        let q = vec4f_sub(&vec4f_from_color(&p.pixels[i]), &mean_color_scaled);
        let d = vec4f_dot(&q, &axis);
        l = minimumf(l, d);
        h = maximumf(h, d);
    }
    l *= 1.0 / 255.0;
    h *= 1.0 / 255.0;

    let b0 = vec4f_mul(&axis, l);
    let b1 = vec4f_mul(&axis, h);
    let mut min_color = vec4f_saturate(&vec4f_add(&mean_color, &b0));
    let mut max_color = vec4f_saturate(&vec4f_add(&mean_color, &b1));

    let white = [1.0f32; 4];
    if vec4f_dot(&min_color, &white) > vec4f_dot(&max_color, &white) {
        std::mem::swap(&mut min_color, &mut max_color);
    }

    // First find a solution using the block's PCA.
    if find_optimal_solution(mode, min_color, max_color, p, res) == 0 {
        return 0;
    }

    for _ in 0..comp.least_squares_passes {
        // Now try to refine the solution using least squares by computing the optimal
        // endpoints from the current selectors.
        let sels = res.selectors;
        let (xl, xh) = least_squares(p, &sels);
        if find_optimal_solution(mode, xl, xh, p, res) == 0 {
            return 0;
        }
    }

    if p.force_selectors.is_none() && comp.uber_level > 0 {
        // In uber level 1, try varying the selectors a little, somewhat like cluster fit
        // would. First try incrementing the minimum selectors, then try decrementing the
        // selectrors, then try both.
        let selectors_temp = res.selectors;
        let mut selectors_temp1 = [0u8; 16];
        let nsw = p.num_selector_weights;
        let max_selector = nsw as i32 - 1;

        let mut min_sel = 256u32;
        let mut max_sel = 0u32;
        for i in 0..np {
            min_sel = min_sel.min(selectors_temp[i] as u32);
            max_sel = max_sel.max(selectors_temp[i] as u32);
        }

        for i in 0..np {
            let mut sel = selectors_temp[i] as u32;
            if sel == min_sel && sel < nsw - 1 {
                sel += 1;
            }
            selectors_temp1[i] = sel as u8;
        }
        let (xl, xh) = least_squares(p, &selectors_temp1);
        if find_optimal_solution(mode, xl, xh, p, res) == 0 {
            return 0;
        }

        for i in 0..np {
            let mut sel = selectors_temp[i] as u32;
            if sel == max_sel && sel > 0 {
                sel -= 1;
            }
            selectors_temp1[i] = sel as u8;
        }
        let (xl, xh) = least_squares(p, &selectors_temp1);
        if find_optimal_solution(mode, xl, xh, p, res) == 0 {
            return 0;
        }

        for i in 0..np {
            let mut sel = selectors_temp[i] as u32;
            if sel == min_sel && sel < nsw - 1 {
                sel += 1;
            } else if sel == max_sel && sel > 0 {
                sel -= 1;
            }
            selectors_temp1[i] = sel as u8;
        }
        let (xl, xh) = least_squares(p, &selectors_temp1);
        if find_optimal_solution(mode, xl, xh, p, res) == 0 {
            return 0;
        }

        // In uber levels 2+, try taking more advantage of endpoint extrapolation by scaling
        // the selectors in one direction or another.
        let uber_err_thresh = (p.num_pixels * 56) >> 4;
        if comp.uber_level >= 2 && res.best_overall_err > uber_err_thresh as u64 {
            let q = if comp.uber_level >= 4 {
                comp.uber_level as i32 - 2
            } else {
                1
            };
            for ly in -q..=1 {
                for hy in (max_selector - 1)..=(max_selector + q) {
                    if ly == 0 && hy == max_selector {
                        continue;
                    }
                    for i in 0..np {
                        selectors_temp1[i] = clampf(
                            (max_selector as f32 * (selectors_temp[i] as f32 - ly as f32)
                                / (hy as f32 - ly as f32)
                                + 0.5)
                                .floor(),
                            0.0,
                            max_selector as f32,
                        ) as u8;
                    }
                    let (xl, xh) = least_squares(p, &selectors_temp1);
                    if find_optimal_solution(mode, xl, xh, p, res) == 0 {
                        return 0;
                    }
                }
            }
        }
    }

    if p.force_selectors.is_none()
        && let Some(kind) = one_color
    {
        // Try encoding the partition as a single color by using the optimal single colors
        // tables to encode the block to its mean.
        let m = |c: f32| (0.5 + c * 255.0) as i32 as u32;
        let c = [
            m(mean_color[0]),
            m(mean_color[1]),
            m(mean_color[2]),
            if kind == OneColor::Astc4Bit2BitRgba {
                m(mean_color[3])
            } else {
                0
            },
        ];
        let mut avg_results = *res;
        let (avg_err, sel) = pack_to_one_color(kind, p, &mut avg_results, c);
        if avg_err < res.best_overall_err {
            *res = avg_results;
            res.selectors_temp[..np].fill(sel);
            res.selectors[..np].fill(sel);
            res.best_overall_err = avg_err;
        } else {
            // pack_*_to_one_color writes its selectors to m_pSelectors_temp either way
            res.selectors_temp[..np].fill(sel);
        }
    }

    res.best_overall_err
}

/// `color_cell_compression_est_astc`.
pub fn color_cell_compression_est_astc(
    num_weights: u32,
    num_comps: u32,
    weight_table: &[u32],
    num_pixels: u32,
    pixels: &[ColorRgba],
    best_err_so_far: u64,
    weights: &[u32; 4],
) -> u64 {
    // Find RGB bounds as an approximation of the block's principle axis
    let (mut lr, mut lg, mut lb, mut la) = (255u32, 255u32, 255u32, 255u32);
    let (mut hr, mut hg, mut hb, mut ha) = (0u32, 0u32, 0u32, 0u32);
    let np = num_pixels as usize;
    for c in &pixels[..np] {
        lr = lr.min(c[0] as u32);
        lg = lg.min(c[1] as u32);
        lb = lb.min(c[2] as u32);
        hr = hr.max(c[0] as u32);
        hg = hg.max(c[1] as u32);
        hb = hb.max(c[2] as u32);
        if num_comps == 4 {
            la = la.min(c[3] as u32);
            ha = ha.max(c[3] as u32);
        }
    }
    if num_comps != 4 {
        la = 255;
        ha = 255;
    }
    let low = [lr as u8, lg as u8, lb as u8, la as u8];
    let high = [hr as u8, hg as u8, hb as u8, ha as u8];

    // Place endpoints at bbox diagonals and compute interpolated colors
    let nw = num_weights as usize;
    let mut weighted = [[0u8; 4]; 32];
    weighted[0] = low;
    weighted[nw - 1] = high;
    for i in 1..nw - 1 {
        let w = weight_table[i];
        for c in 0..3 {
            weighted[i][c] = astc_interpolate_linear(low[c] as u32, high[c] as u32, w) as u8;
        }
        weighted[i][3] = if num_comps == 4 {
            astc_interpolate_linear(low[3] as u32, high[3] as u32, w) as u8
        } else {
            255
        };
    }

    // Compute dots and thresholds
    let ar = high[0] as i32 - low[0] as i32;
    let ag = high[1] as i32 - low[1] as i32;
    let ab = high[2] as i32 - low[2] as i32;
    let aa = high[3] as i32 - low[3] as i32;
    let alpha = num_comps == 4;

    let dot = |c: &ColorRgba| -> i32 {
        let d = c[0] as i32 * ar + c[1] as i32 * ag + c[2] as i32 * ab;
        if alpha { d + c[3] as i32 * aa } else { d }
    };
    let mut dots = [0i32; 32];
    for i in 0..nw {
        dots[i] = dot(&weighted[i]);
    }
    let mut thresh = [0i32; 31];
    for i in 0..nw - 1 {
        thresh[i] = (dots[i] + dots[i + 1] + 1) >> 1;
    }

    let unit = (weights[0] | weights[1] | weights[2] | weights[3]) == 1;
    let mut total_err = 0u64;
    for c in &pixels[..np] {
        let d = dot(c);
        // Find approximate selector
        let mut s = 0usize;
        for j in (0..nw - 1).rev() {
            if d >= thresh[j] {
                s = j + 1;
                break;
            }
        }
        // Compute error
        let e = &weighted[s];
        let dr = e[0] as i32 - c[0] as i32;
        let dg = e[1] as i32 - c[1] as i32;
        let db = e[2] as i32 - c[2] as i32;
        let da = e[3] as i32 - c[3] as i32;
        if unit {
            let mut err = dr * dr + dg * dg + db * db;
            if alpha {
                err += da * da;
            }
            total_err += err as u64;
        } else {
            let mut err = weights[0]
                .wrapping_mul((dr * dr) as u32)
                .wrapping_add(weights[1].wrapping_mul((dg * dg) as u32))
                .wrapping_add(weights[2].wrapping_mul((db * db) as u32));
            if alpha {
                err = err.wrapping_add(weights[3].wrapping_mul((da * da) as u32));
            }
            total_err += err as u64;
        }
        if total_err > best_err_so_far {
            break;
        }
    }
    total_err
}
