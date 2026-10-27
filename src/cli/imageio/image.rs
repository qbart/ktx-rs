// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of `tools/imageio/image.hpp`: transfer functions, color primaries,
//! `color<T, N>` and the `Image` / `ImageT<T, N>` image classes used by
//! `ktx create` (conversion, color space transforms, scaling, mipmaps).
//!
//! `ImageT<T, N>` stores `[T; N]` pixels. Casts follow the C++ on x86-64
//! (float -> int through `cvttss2si`, int -> int modular). Exceptions thrown by
//! `resample` are returned as `ImageError::Runtime`. `operator uint8_t*` is
//! `as_bytes` / `as_bytes_mut`.

#![allow(clippy::excessive_precision, clippy::needless_range_loop)]

use std::any::Any;

use super::{ImageError, ImageResult};
use crate::basisu::resampler::{BASISU_RESAMPLER_MAX_DIMENSION, BoundaryOp, Resampler, Status};
use crate::dfd::khr_df::{
    KHR_DF_PRIMARIES_BT709, KHR_DF_TRANSFER_SRGB, KHR_DF_TRANSFER_UNSPECIFIED,
};
use crate::tinyexr::float_to_half_full as float_to_half;

// ---------------------------------------------------------------------------
// x86-64 conversions of the C++ casts.

/// `cvttss2si` 32-bit (out of range / NaN: `INT_MIN`).
fn cvt32(f: f32) -> i32 {
    if f.is_nan() || f >= 2147483648.0 || f < -2147483648.0 {
        i32::MIN
    } else {
        f as i32
    }
}

/// `cvttss2si` 64-bit (out of range / NaN: `INT64_MIN`).
fn cvt64(f: f32) -> i64 {
    if f.is_nan() || f >= 9223372036854775808.0 || f < -9223372036854775808.0 {
        i64::MIN
    } else {
        f as i64
    }
}

/// GCC float -> uint64_t.
fn f32_to_u64(f: f32) -> u64 {
    if f >= 9223372036854775808.0 {
        (cvt64(f - 9223372036854775808.0) as u64) ^ (1u64 << 63)
    } else {
        cvt64(f) as u64
    }
}

/// `a + b` as `addss a, b`: with two NaN operands the first one is kept (LLVM may
/// commute the operands of a plain `+`).
fn add_x86(a: f32, b: f32) -> f32 {
    if a.is_nan() && b.is_nan() {
        f32::from_bits(a.to_bits() | 0x0040_0000)
    } else {
        a + b
    }
}

/// `cclamp`.
pub fn cclamp<T: PartialOrd>(value: T, low: T, high: T) -> T {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

/// `saturate`.
pub fn saturate(value: f32) -> f32 {
    cclamp(value, 0.0, 1.0)
}

/// Component types of `color<componentType, N>`.
pub trait Component: Copy + Default + PartialEq + PartialOrd + bytemuck::Pod + 'static {
    const SIZE: u32;
    const IS_FLOAT: bool;
    /// `one()`.
    fn one() -> Self;
    /// `std::numeric_limits<T>::min()` (smallest positive normal for float).
    fn min_value() -> Self;
    fn max_value() -> Self;
    fn zero() -> Self {
        Self::default()
    }
    /// `static_cast<float>(v)`.
    fn to_f32(self) -> f32;
    /// `static_cast<componentType>(float)`.
    fn from_f32(f: f32) -> Self;
    /// `static_cast<componentType>(uint32_t)`.
    fn from_u32(v: u32) -> Self;
    /// `static_cast<uint32_t>(v)`.
    fn to_u32(self) -> u32;
    /// `static_cast<int32_t>(v)`.
    fn to_i32(self) -> i32;
    /// `static_cast<uintN_t>` / `static_cast<intN_t>` of the value promoted for `getUINT` /
    /// `getSINT`, as the low `bytes` bytes.
    fn to_int_bytes(self, bytes: u32, signed: bool) -> u64;
    /// `componentType{1}`.
    fn unit() -> Self;
}

macro_rules! int_component {
    ($t:ty, $size:expr) => {
        impl Component for $t {
            const SIZE: u32 = $size;
            const IS_FLOAT: bool = false;
            fn one() -> Self {
                <$t>::MAX
            }
            fn min_value() -> Self {
                <$t>::MIN
            }
            fn max_value() -> Self {
                <$t>::MAX
            }
            fn to_f32(self) -> f32 {
                self as f32
            }
            fn from_f32(f: f32) -> Self {
                if $size == 4 && <$t>::MIN == 0 as $t {
                    cvt64(f) as $t
                } else {
                    cvt32(f) as $t
                }
            }
            fn from_u32(v: u32) -> Self {
                v as $t
            }
            fn to_u32(self) -> u32 {
                self as u32
            }
            fn to_i32(self) -> i32 {
                self as i32
            }
            fn to_int_bytes(self, _bytes: u32, _signed: bool) -> u64 {
                self as i64 as u64
            }
            fn unit() -> Self {
                1
            }
        }
    };
}

int_component!(u8, 1);
int_component!(u16, 2);
int_component!(u32, 4);
int_component!(i8, 1);
int_component!(i16, 2);
int_component!(i32, 4);

impl Component for f32 {
    const SIZE: u32 = 4;
    const IS_FLOAT: bool = true;
    fn one() -> Self {
        1.0
    }
    fn min_value() -> Self {
        f32::MIN_POSITIVE
    }
    fn max_value() -> Self {
        f32::MAX
    }
    fn to_f32(self) -> f32 {
        self
    }
    fn from_f32(f: f32) -> Self {
        f
    }
    fn from_u32(v: u32) -> Self {
        v as f32
    }
    fn to_u32(self) -> u32 {
        cvt64(self) as u32
    }
    fn to_i32(self) -> i32 {
        cvt32(self)
    }
    fn to_int_bytes(self, bytes: u32, signed: bool) -> u64 {
        match (bytes, signed) {
            (8, false) => f32_to_u64(self),
            (8, true) => cvt64(self) as u64,
            (4, false) => cvt64(self) as u64,
            _ => cvt32(self) as i64 as u64,
        }
    }
    fn unit() -> Self {
        1.0
    }
}

// ---------------------------------------------------------------------------
// Transfer functions and color primaries.

/// Port of `ColorPrimaryTransform`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ColorPrimaryTransform {
    pub matrix: [[f32; 3]; 3],
}

impl ColorPrimaryTransform {
    pub fn new(elements: &[f32; 9]) -> ColorPrimaryTransform {
        let mut matrix = [[0.0; 3]; 3];
        for (i, row) in matrix.iter_mut().enumerate() {
            for (j, m) in row.iter_mut().enumerate() {
                *m = elements[i * 3 + j];
            }
        }
        ColorPrimaryTransform { matrix }
    }
}

/// Port of `TransferFunction`.
pub trait TransferFunction {
    fn encode(&self, intensity: f32) -> f32;
    fn decode(&self, brightness: f32) -> f32;
}

/// Port of `TransferFunctionLinear`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TransferFunctionLinear;

impl TransferFunction for TransferFunctionLinear {
    fn encode(&self, intensity: f32) -> f32 {
        intensity
    }
    fn decode(&self, brightness: f32) -> f32 {
        brightness
    }
}

/// Port of `TransferFunctionGamma`.
#[derive(Debug, Clone, Copy)]
pub struct TransferFunctionGamma {
    oe_gamma: f32,
    eo_gamma: f32,
}

impl TransferFunctionGamma {
    pub fn new(oe_gamma: f32) -> TransferFunctionGamma {
        TransferFunctionGamma {
            oe_gamma,
            eo_gamma: 1.0 / oe_gamma,
        }
    }
}

impl TransferFunction for TransferFunctionGamma {
    fn encode(&self, intensity: f32) -> f32 {
        saturate(intensity.powf(self.oe_gamma))
    }
    fn decode(&self, brightness: f32) -> f32 {
        saturate(brightness.powf(self.eo_gamma))
    }
}

/// Port of `TransferFunctionSRGB`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TransferFunctionSRGB;

impl TransferFunction for TransferFunctionSRGB {
    fn encode(&self, intensity: f32) -> f32 {
        if intensity < 0.0031308 {
            12.92 * intensity
        } else {
            1.055 * intensity.powf(1.0f32 / 2.4f32) - 0.055
        }
    }
    fn decode(&self, brightness: f32) -> f32 {
        if brightness < 0.04045 {
            saturate(brightness * (1.0f32 / 12.92f32))
        } else {
            saturate(((brightness + 0.055) * (1.0f32 / 1.055f32)).powf(2.4))
        }
    }
}

/// Port of `TransferFunctionITU` (Netpbm's continuous BT.709 curve).
#[derive(Debug, Clone, Copy)]
pub struct TransferFunctionITU {
    eo_gamma: f32,
    oe_gamma: f32,
    linear_cutoff: f32,
    linear_expansion: f32,
}

impl Default for TransferFunctionITU {
    fn default() -> Self {
        let eo_gamma = 2.2f32;
        let oe_gamma = 1.0 / eo_gamma;
        let linear_cutoff = 0.018f32;
        let linear_expansion = (1.099 * linear_cutoff.powf(oe_gamma) - 0.099) / linear_cutoff;
        TransferFunctionITU {
            eo_gamma,
            oe_gamma,
            linear_cutoff,
            linear_expansion,
        }
    }
}

impl TransferFunction for TransferFunctionITU {
    fn encode(&self, intensity: f32) -> f32 {
        if intensity < self.linear_cutoff {
            intensity * self.linear_expansion
        } else {
            1.099 * intensity.powf(self.oe_gamma) - 0.099
        }
    }
    fn decode(&self, brightness: f32) -> f32 {
        if brightness < self.linear_cutoff * self.linear_expansion {
            brightness / self.linear_expansion
        } else {
            ((brightness + 0.099) / 1.099).powf(self.eo_gamma)
        }
    }
}

/// Port of `TransferFunctionBT2100_PQ_EOTF`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TransferFunctionBT2100PqEotf;

impl TransferFunctionBT2100PqEotf {
    const M1: f32 = 0.1593017578125;
    const M2: f32 = 78.84375;
    const C1: f32 = 0.8359375;
    const C2: f32 = 18.8515625;
    const C3: f32 = 18.6875;
}

impl TransferFunction for TransferFunctionBT2100PqEotf {
    fn decode(&self, brightness: f32) -> f32 {
        let ym1 = brightness.powf(Self::M1);
        ((Self::C1 + Self::C2 * ym1) / (1.0 + Self::C3 * ym1)).powf(Self::M2)
    }
    fn encode(&self, intensity: f32) -> f32 {
        let rm2 = 1.0f32 / Self::M2;
        let erm2 = intensity.powf(rm2);
        let a = erm2 - Self::C1;
        // std::max(a, 0.f)
        let m = if a < 0.0 { 0.0 } else { a };
        (m / (Self::C2 - Self::C3 * erm2)).powf(Self::M1)
    }
}

/// Port of `ColorPrimaries` and its `ColorPrimaries*` subclasses (constructors below).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorPrimaries {
    pub to_xyz: ColorPrimaryTransform,
    pub from_xyz: ColorPrimaryTransform,
}

impl ColorPrimaries {
    pub fn new(to_xyz: &[f32; 9], from_xyz: &[f32; 9]) -> ColorPrimaries {
        ColorPrimaries {
            to_xyz: ColorPrimaryTransform::new(to_xyz),
            from_xyz: ColorPrimaryTransform::new(from_xyz),
        }
    }

    pub fn transform_to(&self, target: &ColorPrimaries) -> ColorPrimaryTransform {
        let mut result = ColorPrimaryTransform::default();
        for i in 0..3 {
            for j in 0..3 {
                result.matrix[i][j] = 0.0;
                for k in 0..3 {
                    result.matrix[i][j] += self.to_xyz.matrix[i][k] * target.from_xyz.matrix[k][j];
                }
            }
        }
        result
    }

    #[rustfmt::skip]
    pub fn bt709() -> ColorPrimaries {
        Self::new(
            &[0.412391, 0.357584, 0.180481, 0.212639, 0.715169, 0.072192, 0.019331, 0.119195, 0.950532],
            &[3.240970, -1.537383, -0.498611, -0.969244, 1.875968, 0.041555, 0.055630, -0.203977, 1.056972],
        )
    }

    #[rustfmt::skip]
    pub fn bt601_625_ebu() -> ColorPrimaries {
        Self::new(
            &[0.430554, 0.341550, 0.178352, 0.222004, 0.706655, 0.071341, 0.020182, 0.129553, 0.939322],
            &[3.063361, -1.393390, -0.475824, -0.969244, 1.875968, 0.041555, 0.067861, -0.228799, 1.069090],
        )
    }

    #[rustfmt::skip]
    pub fn bt601_525_smpte() -> ColorPrimaries {
        Self::new(
            &[0.393521, 0.365258, 0.191677, 0.212376, 0.701060, 0.086564, 0.018739, 0.111934, 0.958385],
            &[3.506003, -1.739791, -0.544058, -1.069048, 1.977779, 0.035171, 0.056307, -0.196976, 1.049952],
        )
    }

    #[rustfmt::skip]
    pub fn bt2020() -> ColorPrimaries {
        Self::new(
            &[0.636958, 0.144617, 0.168881, 0.262700, 0.677998, 0.059302, 0.000000, 0.028073, 1.060985],
            &[1.716651, -0.355671, -0.253366, -0.666684, 1.616481, 0.015769, 0.017640, -0.042771, 0.942103],
        )
    }

    #[rustfmt::skip]
    pub fn ciexyz() -> ColorPrimaries {
        Self::new(
            &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        )
    }

    #[rustfmt::skip]
    #[allow(clippy::excessive_precision)]
    pub fn aces() -> ColorPrimaries {
        Self::new(
            &[0.9525523959, 0.0000000000, 0.0000936786, 0.3439664498, 0.7281660966, -0.0721325464, 0.0000000000, 0.0000000000, 1.0088251844],
            &[1.0498110175, 0.0000000000, -0.0000974845, -0.4959030231, 1.3733130458, 0.0982400361, 0.0000000000, 0.0000000000, 0.9912520182],
        )
    }

    #[rustfmt::skip]
    #[allow(clippy::excessive_precision)]
    pub fn acescc() -> ColorPrimaries {
        Self::new(
            &[0.6624541811, 0.1340042065, 0.1561876870, 0.2722287168, 0.6740817658, 0.0536895174, -0.0055746495, 0.0040607335, 1.0103391003],
            &[1.6410233797, -0.3248032942, -0.2464246952, -0.6636628587, 1.6153315917, 0.0167563477, 0.0117218943, -0.0082844420, 0.9883948585],
        )
    }

    #[rustfmt::skip]
    pub fn ntsc1953() -> ColorPrimaries {
        Self::new(
            &[0.606993, 0.173449, 0.200571, 0.298967, 0.586421, 0.114612, 0.000000, 0.066076, 1.117469],
            &[1.909675, -0.532365, -0.288161, -0.984965, 1.999777, -0.028317, 0.058241, -0.118246, 0.896554],
        )
    }

    #[rustfmt::skip]
    pub fn pal525() -> ColorPrimaries {
        Self::new(
            &[0.415394, 0.354637, 0.210677, 0.224181, 0.680675, 0.095145, 0.019781, 0.108679, 1.053387],
            &[3.321392, -1.648181, -0.515410, -1.101064, 2.037011, 0.036225, 0.051228, -0.179211, 0.955260],
        )
    }

    #[rustfmt::skip]
    #[allow(clippy::excessive_precision)]
    pub fn display_p3() -> ColorPrimaries {
        Self::new(
            &[0.4865709486, 0.2656676932, 0.1982172852, 0.2289745641, 0.6917385218, 0.0792869141, 0.0000000000, 0.0451133819, 1.0439441689],
            &[2.4934969119, -0.9313836179, -0.4027107845, -0.8294889696, 1.7626640603, 0.0236246858, 0.0358458302, -0.0761723893, 0.9568845240],
        )
    }

    #[rustfmt::skip]
    #[allow(clippy::excessive_precision)]
    pub fn adobe_rgb() -> ColorPrimaries {
        Self::new(
            &[0.5766690429, 0.1855582379, 0.1882286462, 0.2973449753, 0.6273635663, 0.0752914585, 0.0270313614, 0.0706888525, 0.9913375368],
            &[2.0415879038, -0.5650069743, -0.3447313508, -0.9692436363, 1.8759675015, 0.0415550574, 0.0134442806, -0.1183623922, 1.0151749944],
        )
    }
}

// ---------------------------------------------------------------------------
// glm packing used by getB10G11R11 / getE5B9G9R9.

fn float_to_11bit(x: f32) -> u32 {
    if x == 0.0 {
        0
    } else if x.is_nan() {
        !0
    } else if x.is_infinite() {
        0x1f << 6
    } else {
        let f = x.to_bits();
        ((((f & 0x7f80_0000).wrapping_sub(0x3800_0000)) >> 17) & 0x07c0) | ((f >> 17) & 0x003f)
    }
}

fn float_to_10bit(x: f32) -> u32 {
    if x == 0.0 {
        0
    } else if x.is_nan() {
        !0
    } else if x.is_infinite() {
        0x1f << 5
    } else {
        let f = x.to_bits();
        ((((f & 0x7f80_0000).wrapping_sub(0x3800_0000)) >> 18) & 0x03e0) | ((f >> 18) & 0x001f)
    }
}

/// glm `packF2x11_1x10`.
pub fn pack_f2x11_1x10(v: [f32; 3]) -> u32 {
    (float_to_11bit(v[0]) & ((1 << 11) - 1))
        | ((float_to_11bit(v[1]) & ((1 << 11) - 1)) << 11)
        | ((float_to_10bit(v[2]) & ((1 << 10) - 1)) << 22)
}

fn glm_max(x: f32, y: f32) -> f32 {
    if x > y { x } else { y }
}

fn glm_min(x: f32, y: f32) -> f32 {
    if x < y { x } else { y }
}

/// glm `packF3x9_E1x5`.
pub fn pack_f3x9_e1x5(v: [f32; 3]) -> u32 {
    let shared_exp_max = ((2.0f32.powf(9.0) - 1.0) / 2.0f32.powf(9.0)) * 2.0f32.powf(31.0 - 15.0);
    let color = v.map(|c| glm_min(glm_max(c, 0.0), shared_exp_max));
    let max_color = glm_max(color[0], glm_max(color[1], color[2]));

    let exp_shared_p = glm_max(-15.0 - 1.0, max_color.log2().floor()) + 1.0 + 15.0;
    let max_shared = (max_color / 2.0f32.powf(exp_shared_p - 15.0 - 9.0) + 0.5).floor();
    let exp_shared = if max_shared == 2.0f32.powf(9.0) {
        exp_shared_p + 1.0
    } else {
        exp_shared_p
    };

    let scale = 2.0f32.powf(exp_shared - 15.0 - 9.0);
    let comp = color.map(|c| cvt64((c / scale + 0.5).floor()) as u32);
    (comp[0] & 0x1ff)
        | ((comp[1] & 0x1ff) << 9)
        | ((comp[2] & 0x1ff) << 18)
        | ((cvt64(exp_shared) as u32 & 0x1f) << 27)
}

// ---------------------------------------------------------------------------
// imageio_utility.h conversions.

/// `imageio::convertUNORM`.
pub fn convert_unorm(raw_bits: u32, source_bits: u32, target_bits: u32) -> u32 {
    crate::cli::format_descriptor::convert_unorm(raw_bits, source_bits, target_bits)
}

/// `imageio::convertUINT`.
pub fn convert_uint(mut raw_bits: u32, source_bits: u32, target_bits: u32) -> u32 {
    let mask = |bits: u32| {
        if bits == 32 {
            u32::MAX
        } else {
            (1u32 << bits).wrapping_sub(1)
        }
    };
    raw_bits &= mask(source_bits);
    if target_bits < source_bits {
        raw_bits &= mask(target_bits);
    }
    raw_bits
}

/// `imageio::convertSINT` (sign and magnitude).
pub fn convert_sint(raw_bits: u32, source_bits: u32, target_bits: u32) -> u32 {
    let source_sign_mask = 1u32.wrapping_shl(source_bits.wrapping_sub(1));
    let sign = raw_bits & source_sign_mask != 0;
    let source_value_mask = 1u32
        .wrapping_shl(source_bits.wrapping_sub(1))
        .wrapping_sub(1);
    let source_value = raw_bits & source_value_mask;
    let target_sign_bit_index = target_bits.wrapping_sub(1);
    let target_value_mask = 1u32
        .wrapping_shl(target_bits.wrapping_sub(1))
        .wrapping_sub(1);

    let mut result = (sign as u32).wrapping_shl(target_sign_bit_index);
    if target_bits < source_bits {
        result |= source_value & target_value_mask;
    } else {
        result |= source_value;
    }
    result
}

// ---------------------------------------------------------------------------
// color<componentType, N> helpers on `[T; N]`.

/// `color::operator[]` (index clamped to the last component).
fn comp<T: Component, const N: usize>(c: &[T; N], i: usize) -> T {
    c[i.min(N - 1)]
}

/// `color::set` (index clamped to the last component).
fn set_comp<T: Component, const N: usize>(c: &mut [T; N], i: usize, v: T) {
    c[i.min(N - 1)] = v;
}

const GC_M: [f32; 5] = [0.0, 128.0, 32768.0, 0.0, 2147483648.0];
const GC_S: [u32; 5] = [0, 255, 65535, 0, 4294967295];

/// `vec3<componentType>::normalize` on (r, g, b).
fn vec3_normalize<T: Component>(v: &mut [f32; 3]) {
    let base_normalize = |v: &mut [f32; 3]| {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if len > 0.0 {
            v.iter_mut().for_each(|c| *c /= len);
        }
    };
    if T::IS_FLOAT {
        base_normalize(v);
        return;
    }
    let i = T::SIZE as usize;
    // Zero normals in range [-1, 1] can't be normalized
    if GC_M[i] == v[0] && GC_M[i] == v[1] && GC_M[i] == v[2] {
        return;
    }
    let s = GC_S[i];
    for c in v.iter_mut() {
        *c = (*c as f64 / s as f64) as f32 * 2.0 - 1.0;
    }
    v.iter_mut().for_each(|c| *c = cclamp(*c, -1.0, 1.0));
    base_normalize(v);
    for c in v.iter_mut() {
        *c = ((*c + 1.0) * s as f32 * 0.5 + 0.5).floor();
    }
    v.iter_mut().for_each(|c| *c = cclamp(*c, 0.0, s as f32));
}

/// `color<T, N>::normalize`.
pub fn color_normalize<T: Component, const N: usize>(c: &mut [T; N]) {
    match N {
        1 => {
            // Normalizing single channel image doesn't make much sense
            // Here I assume single channel color is (X, 0, 0, 0)
            if c[0] != T::zero() {
                c[0] = T::from_u32(GC_S[T::SIZE as usize]);
            }
        }
        2 => {
            let mut v = [
                c[0].to_f32(),
                c[1].to_f32(),
                GC_S[T::SIZE as usize] as f32 * 0.5,
            ];
            vec3_normalize::<T>(&mut v);
            c[0] = T::from_f32(v[0]);
            c[1] = T::from_f32(v[1]);
        }
        _ => {
            let mut v = [c[0].to_f32(), c[1].to_f32(), c[2].to_f32()];
            vec3_normalize::<T>(&mut v);
            for (d, s) in c.iter_mut().zip(v) {
                *d = T::from_f32(s);
            }
        }
    }
}

fn rcp_one<T: Component>() -> f32 {
    if T::IS_FLOAT {
        1.0
    } else {
        1.0 / T::max_value().to_f32()
    }
}

fn swizzle_pixel<T: Component, const N: usize>(src: &[T; N], swizzle: u8) -> T {
    match swizzle {
        b'r' => comp(src, 0),
        b'g' => comp(src, 1),
        b'b' => comp(src, 2),
        b'a' => comp(src, 3),
        b'1' => T::one(),
        _ => T::zero(),
    }
}

// ---------------------------------------------------------------------------
// Image / ImageT

/// Port of the abstract `Image` class.
pub trait Image: Any {
    fn get_width(&self) -> u32;
    fn get_height(&self) -> u32;
    fn get_pixel_count(&self) -> u32 {
        self.get_width().wrapping_mul(self.get_height())
    }
    fn get_transfer_function(&self) -> u32;
    fn set_transfer_function(&mut self, tf: u32);
    fn get_primaries(&self) -> u32;
    fn set_primaries(&mut self, primaries: u32);

    /// `operator uint8_t*`.
    fn as_bytes(&self) -> &[u8];
    fn as_bytes_mut(&mut self) -> &mut [u8];

    fn get_byte_count(&self) -> usize;
    fn get_pixel_size(&self) -> u32;
    fn get_component_count(&self) -> u32;
    fn get_component_size(&self) -> u32;
    fn create_image(&self, width: u32, height: u32) -> Box<dyn Image>;
    /// Should only be used if the stored image data is UNORM convertable (with optional
    /// significant bit count).
    fn get_unorm(&self, num_channels: u32, target_bits: u32, s_bits: u32) -> Vec<u8>;
    /// Should only be used if the stored image data is UNORM convertable (packed into a
    /// single word).
    fn get_unorm_packed(&self, c0: u32, c1: u32, c2: u32, c3: u32) -> Vec<u8>;
    fn get_sfloat(&self, num_channels: u32, target_bits: u32) -> Vec<u8>;
    fn get_b10g11r11(&self) -> Vec<u8>;
    fn get_e5b9g9r9(&self) -> Vec<u8>;
    fn get_uint(&self, num_channels: u32, target_bits: u32) -> Vec<u8>;
    fn get_sint(&self, num_channels: u32, target_bits: u32) -> Vec<u8>;
    fn get_uint_packed(&self, c0: u32, c1: u32, c2: u32, c3: u32) -> Vec<u8>;
    fn get_sint_packed(&self, c0: u32, c1: u32, c2: u32, c3: u32) -> Vec<u8>;
    fn resample(
        &self,
        target_width: u32,
        target_height: u32,
        filter: &str,
        filter_scale: f32,
        wrap_mode: BoundaryOp,
    ) -> ImageResult<Box<dyn Image>>;
    fn yflip(&mut self) -> &mut dyn Image;
    fn transform_color_space(
        &mut self,
        decode: &dyn TransferFunction,
        encode: &dyn TransferFunction,
        transform_primaries: Option<&ColorPrimaryTransform>,
    ) -> &mut dyn Image;
    fn normalize(&mut self) -> &mut dyn Image;
    fn swizzle(&mut self, swizzle: &str) -> &mut dyn Image;
    /// `copyToR`; the destination must be a 1-channel image of the same component type.
    fn copy_to_r(&self, dst: &mut dyn Image, swizzle: &str) -> ImageResult<()>;
    fn copy_to_rg(&self, dst: &mut dyn Image, swizzle: &str) -> ImageResult<()>;
    fn copy_to_rgb(&self, dst: &mut dyn Image, swizzle: &str) -> ImageResult<()>;
    fn copy_to_rgba(&self, dst: &mut dyn Image, swizzle: &str) -> ImageResult<()>;
    fn premultiply_alpha(&mut self) -> &mut dyn Image;
    fn map_range(&mut self, scale: f32, offset: f32) -> &mut dyn Image;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Port of `ImageT<componentType, componentCount>`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageT<T: Component, const N: usize> {
    width: u32,
    height: u32,
    transfer_function: u32,
    primaries: u32,
    pub pixels: Vec<[T; N]>,
}

pub type R8Image = ImageT<u8, 1>;
pub type Rg8Image = ImageT<u8, 2>;
pub type Rgb8Image = ImageT<u8, 3>;
pub type Rgba8Image = ImageT<u8, 4>;
pub type R16Image = ImageT<u16, 1>;
pub type Rg16Image = ImageT<u16, 2>;
pub type Rgb16Image = ImageT<u16, 3>;
pub type Rgba16Image = ImageT<u16, 4>;
pub type R32Image = ImageT<u32, 1>;
pub type Rg32Image = ImageT<u32, 2>;
pub type Rgb32Image = ImageT<u32, 3>;
pub type Rgba32Image = ImageT<u32, 4>;
pub type R8sImage = ImageT<i8, 1>;
pub type Rg8sImage = ImageT<i8, 2>;
pub type Rgb8sImage = ImageT<i8, 3>;
pub type Rgba8sImage = ImageT<i8, 4>;
pub type R16sImage = ImageT<i16, 1>;
pub type Rg16sImage = ImageT<i16, 2>;
pub type Rgb16sImage = ImageT<i16, 3>;
pub type Rgba16sImage = ImageT<i16, 4>;
pub type R32sImage = ImageT<i32, 1>;
pub type Rg32sImage = ImageT<i32, 2>;
pub type Rgb32sImage = ImageT<i32, 3>;
pub type Rgba32sImage = ImageT<i32, 4>;
pub type R32fImage = ImageT<f32, 1>;
pub type Rg32fImage = ImageT<f32, 2>;
pub type Rgb32fImage = ImageT<f32, 3>;
pub type Rgba32fImage = ImageT<f32, 4>;

impl<T: Component, const N: usize> ImageT<T, N> {
    /// Zero initialized `w` x `h` image.
    pub fn new(w: u32, h: u32) -> ImageT<T, N> {
        ImageT {
            width: w,
            height: h,
            transfer_function: KHR_DF_TRANSFER_UNSPECIFIED,
            primaries: KHR_DF_PRIMARIES_BT709,
            pixels: vec![[T::zero(); N]; (w as usize) * (h as usize)],
        }
    }

    /// `operator()(x, y)`.
    pub fn at(&self, x: u32, y: u32) -> Option<&[T; N]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.pixels
            .get(x as usize + y as usize * self.width as usize)
    }

    pub fn at_mut(&mut self, x: u32, y: u32) -> Option<&mut [T; N]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.pixels
            .get_mut(x as usize + y as usize * self.width as usize)
    }

    fn copy_to<const M: usize>(&self, dst: &mut dyn Image, swizzle: &str) -> ImageResult<()> {
        let Some(dst) = dst.as_any_mut().downcast_mut::<ImageT<T, M>>() else {
            return Err(ImageError::Runtime(
                "copyTo: incompatible destination image".into(),
            ));
        };
        dst.transfer_function = self.transfer_function;
        dst.primaries = self.primaries;
        let sw = swizzle.as_bytes();
        for (d, s) in dst.pixels.iter_mut().zip(&self.pixels) {
            let mut c = 0;
            while c < M {
                if c < N {
                    d[c] = swizzle_pixel(s, sw.get(c).copied().unwrap_or(0));
                } else {
                    break;
                }
                c += 1;
            }
            while c < M {
                d[c] = if c < 3 { T::zero() } else { T::one() };
                c += 1;
            }
        }
        Ok(())
    }

    fn resample_impl(
        &self,
        target_width: u32,
        target_height: u32,
        filter: &str,
        filter_scale: f32,
        wrap_mode: BoundaryOp,
    ) -> ImageResult<ImageT<T, N>> {
        let mut target = ImageT::<T, N>::new(target_width, target_height);
        target.transfer_function = self.transfer_function;
        target.primaries = self.primaries;

        let source_width = self.width;
        let source_height = self.height;
        let max_dim = BASISU_RESAMPLER_MAX_DIMENSION as u32;
        if source_width.max(source_height) > max_dim || target_width.max(target_height) > max_dim {
            return Err(ImageError::Runtime(format!(
                "Image larger than max supported size of {BASISU_RESAMPLER_MAX_DIMENSION}"
            )));
        }

        let check = |r: &Resampler| -> ImageResult<()> {
            match r.status() {
                Status::Okay => Ok(()),
                Status::OutOfMemory => Err(ImageError::Runtime(
                    "Resampler or Resampler::put_line out of memory.".into(),
                )),
                Status::BadFilterName => {
                    Err(ImageError::Runtime(format!("Unknown filter: {filter}")))
                }
                Status::ScanBufferFull => Err(ImageError::Runtime(
                    "Resampler::put_line scan buffer full.".into(),
                )),
            }
        };

        // Float types handled as SFloat HDR otherwise UNROM LDR is assumed
        let is_hdr = T::IS_FLOAT;
        let mut samples: Vec<Vec<f32>> = Vec::with_capacity(N);
        let mut resamplers: Vec<Resampler> = Vec::with_capacity(N);
        for i in 0..N {
            let (cx, cy) = match resamplers.first() {
                Some(r0) if i > 0 => (r0.get_clist_x(), r0.get_clist_y()),
                _ => (None, None),
            };
            let r = Resampler::new(
                source_width as i32,
                source_height as i32,
                target_width as i32,
                target_height as i32,
                wrap_mode,
                0.0,
                if is_hdr { 0.0 } else { 1.0 },
                Some(filter),
                cx,
                cy,
                filter_scale,
                filter_scale,
                0.0,
                0.0,
            );
            check(&r)?;
            resamplers.push(r);
            samples.push(vec![0.0; source_width as usize]);
        }

        let tf_srgb = TransferFunctionSRGB;
        let tf_linear = TransferFunctionLinear;
        let tf: &dyn TransferFunction = if self.transfer_function == KHR_DF_TRANSFER_SRGB {
            &tf_srgb
        } else {
            &tf_linear
        };
        let tf_for = |c: usize| -> &dyn TransferFunction { if c == 3 { &tf_linear } else { tf } };
        let one = T::one().to_f32();
        let in_scale = 1.0 / one;

        let mut target_y = 0usize;
        for source_y in 0..source_height as usize {
            // Put source lines into resampler(s)
            for source_x in 0..source_width as usize {
                let Some(sp) = self.pixels.get(source_y * source_width as usize + source_x) else {
                    continue;
                };
                for (c, s) in samples.iter_mut().enumerate() {
                    let v = comp(sp, c);
                    let value = if T::IS_FLOAT {
                        v.to_f32()
                    } else {
                        v.to_f32() * in_scale
                    };
                    // c == 3: Alpha channel always uses tfLinear
                    s[source_x] = tf_for(c).decode(value);
                }
            }
            for (r, s) in resamplers.iter_mut().zip(&samples) {
                if !r.put_line(s) {
                    check(r)?;
                }
            }

            // Retrieve any output lines
            loop {
                let mut lines: Vec<Option<Vec<f32>>> = Vec::with_capacity(N);
                for r in resamplers.iter_mut() {
                    lines.push(r.get_line().map(|l| l.to_vec()));
                }
                if lines.first().is_none_or(|l| l.is_none()) {
                    break; // No new output line, break from retrieve and place in a new source line
                }
                for target_x in 0..target_width as usize {
                    let Some(tp) = target
                        .pixels
                        .get_mut(target_y * target_width as usize + target_x)
                    else {
                        continue;
                    };
                    for (c, line) in lines.iter().enumerate() {
                        let linear = line
                            .as_ref()
                            .and_then(|l| l.get(target_x))
                            .copied()
                            .unwrap_or(0.0);
                        // c == 3: Alpha channel always uses tfLinear
                        let out = tf_for(c).encode(linear);
                        tp[c] = if T::IS_FLOAT {
                            T::from_f32(out)
                        } else if out.is_nan() || out < 0.0 {
                            T::zero()
                        } else if out > 1.0 {
                            T::one()
                        } else {
                            T::from_f32(out * one + 0.5)
                        };
                    }
                }
                target_y += 1;
            }
        }
        Ok(target)
    }
}

impl<T: Component, const N: usize> Image for ImageT<T, N> {
    fn get_width(&self) -> u32 {
        self.width
    }
    fn get_height(&self) -> u32 {
        self.height
    }
    fn get_transfer_function(&self) -> u32 {
        self.transfer_function
    }
    fn set_transfer_function(&mut self, tf: u32) {
        self.transfer_function = tf;
    }
    fn get_primaries(&self) -> u32 {
        self.primaries
    }
    fn set_primaries(&mut self, primaries: u32) {
        self.primaries = primaries;
    }

    fn as_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(self.pixels.as_flattened())
    }
    fn as_bytes_mut(&mut self) -> &mut [u8] {
        bytemuck::cast_slice_mut(self.pixels.as_flattened_mut())
    }

    fn get_byte_count(&self) -> usize {
        self.get_pixel_count() as usize * self.get_pixel_size() as usize
    }
    fn get_pixel_size(&self) -> u32 {
        N as u32 * T::SIZE
    }
    fn get_component_count(&self) -> u32 {
        N as u32
    }
    fn get_component_size(&self) -> u32 {
        T::SIZE
    }
    fn create_image(&self, width: u32, height: u32) -> Box<dyn Image> {
        Box::new(ImageT::<T, N>::new(width, height))
    }

    fn get_unorm(&self, num_channels: u32, target_bits: u32, s_bits: u32) -> Vec<u8> {
        let source_bits = T::SIZE * 8;
        let target_bytes = (target_bits / 8) as usize;
        let mask = if s_bits == 0 {
            0xFFFF_FFFF
        } else {
            1u32.wrapping_shl(s_bits)
                .wrapping_sub(1)
                .wrapping_shl(target_bits.wrapping_sub(s_bits))
        };
        let nc = num_channels as usize;
        let mut data = vec![0u8; self.pixels.len() * nc * target_bytes];
        for (p, chunk) in self
            .pixels
            .iter()
            .zip(data.chunks_exact_mut((nc * target_bytes).max(1)))
        {
            for c in 0..nc {
                let source = if c < N {
                    p[c]
                } else if c != 3 {
                    T::zero()
                } else {
                    T::one()
                };
                let value = convert_unorm(source.to_u32(), source_bits, target_bits) & mask;
                let t = &mut chunk[c * target_bytes..(c + 1) * target_bytes];
                match target_bytes {
                    1 => t.copy_from_slice(&(value as u8).to_le_bytes()),
                    2 => t.copy_from_slice(&(value as u16).to_le_bytes()),
                    4 => t.copy_from_slice(&value.to_le_bytes()),
                    _ => {}
                }
            }
        }
        data
    }

    fn get_unorm_packed(&self, c0: u32, c1: u32, c2: u32, c3: u32) -> Vec<u8> {
        let target_pack_bytes =
            (c0.wrapping_add(c1).wrapping_add(c2).wrapping_add(c3) / 8) as usize;
        let source_bits = T::SIZE * 8;
        let mut data = vec![0u8; self.pixels.len() * target_pack_bytes];
        if !matches!(target_pack_bytes, 1 | 2 | 4 | 8) {
            return data;
        }
        for (p, t) in self
            .pixels
            .iter()
            .zip(data.chunks_exact_mut(target_pack_bytes))
        {
            let mut pack: u64 = 0;
            let ch = |i: usize, dflt: T| if N > i { p[i] } else { dflt };
            if c0 > 0 {
                let v = convert_unorm(ch(0, T::zero()).to_u32(), source_bits, c0) as u64;
                pack |= v.wrapping_shl(c1 + c2 + c3);
            }
            if c1 > 0 {
                let v = convert_unorm(ch(1, T::zero()).to_u32(), source_bits, c1) as u64;
                pack |= v.wrapping_shl(c2 + c3);
            }
            if c2 > 0 {
                let v = convert_unorm(ch(2, T::zero()).to_u32(), source_bits, c2) as u64;
                pack |= v.wrapping_shl(c3);
            }
            if c3 > 0 {
                pack |= convert_unorm(ch(3, T::one()).to_u32(), source_bits, c3) as u64;
            }
            t.copy_from_slice(&pack.to_le_bytes()[..target_pack_bytes]);
        }
        data
    }

    fn get_sfloat(&self, num_channels: u32, target_bits: u32) -> Vec<u8> {
        let target_bytes = (target_bits / 8) as usize;
        let nc = num_channels as usize;
        let mut data = vec![0u8; self.pixels.len() * nc * target_bytes];
        for (p, chunk) in self
            .pixels
            .iter()
            .zip(data.chunks_exact_mut((nc * target_bytes).max(1)))
        {
            for c in 0..nc {
                let value = if c < N {
                    p[c]
                } else if c != 3 {
                    T::zero()
                } else {
                    T::unit()
                };
                let t = &mut chunk[c * target_bytes..(c + 1) * target_bytes];
                if T::SIZE as usize == target_bytes {
                    t.copy_from_slice(bytemuck::bytes_of(&value));
                } else if target_bytes == 2 {
                    t.copy_from_slice(&float_to_half(value.to_f32()).to_le_bytes());
                } else if target_bytes == 4 {
                    t.copy_from_slice(&value.to_f32().to_le_bytes());
                }
            }
        }
        data
    }

    fn get_b10g11r11(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            let rgb = [
                comp(p, 0).to_f32(),
                comp(p, 1).to_f32(),
                comp(p, 2).to_f32(),
            ];
            data.extend_from_slice(&pack_f2x11_1x10(rgb).to_le_bytes());
        }
        data
    }

    fn get_e5b9g9r9(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            let rgb = [
                comp(p, 0).to_f32(),
                comp(p, 1).to_f32(),
                comp(p, 2).to_f32(),
            ];
            data.extend_from_slice(&pack_f3x9_e1x5(rgb).to_le_bytes());
        }
        data
    }

    fn get_uint(&self, num_channels: u32, target_bits: u32) -> Vec<u8> {
        get_int(self, num_channels, target_bits, false)
    }

    fn get_sint(&self, num_channels: u32, target_bits: u32) -> Vec<u8> {
        get_int(self, num_channels, target_bits, true)
    }

    fn get_uint_packed(&self, c0: u32, c1: u32, c2: u32, c3: u32) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            let mut pack: u32 = 0;
            pack |= convert_uint(comp(p, 0).to_u32(), 32, c0).wrapping_shl(c1 + c2 + c3);
            pack |= convert_uint(comp(p, 1).to_u32(), 32, c1).wrapping_shl(c2 + c3);
            pack |= convert_uint(comp(p, 2).to_u32(), 32, c2).wrapping_shl(c3);
            pack |= convert_uint(comp(p, 3).to_u32(), 32, c3);
            data.extend_from_slice(&pack.to_le_bytes());
        }
        data
    }

    fn get_sint_packed(&self, c0: u32, c1: u32, c2: u32, c3: u32) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            let v = |i: usize| comp(p, i).to_i32() as u32;
            let mut pack: u32 = 0;
            pack |= convert_sint(v(0), 32, c0).wrapping_shl(c1 + c2 + c3);
            pack |= convert_sint(v(1), 32, c1).wrapping_shl(c2 + c3);
            pack |= convert_sint(v(2), 32, c2).wrapping_shl(c3);
            pack |= convert_sint(v(3), 32, c3);
            data.extend_from_slice(&pack.to_le_bytes());
        }
        data
    }

    fn resample(
        &self,
        target_width: u32,
        target_height: u32,
        filter: &str,
        filter_scale: f32,
        wrap_mode: BoundaryOp,
    ) -> ImageResult<Box<dyn Image>> {
        let t = self.resample_impl(target_width, target_height, filter, filter_scale, wrap_mode)?;
        Ok(Box::new(t))
    }

    fn yflip(&mut self) -> &mut dyn Image {
        let w = self.width as usize;
        if self.height > 1 && w > 0 {
            let mut sy = self.height - 1;
            let mut dy = 0u32;
            while sy >= self.height / 2 && sy != 0 {
                if sy != dy {
                    let (lo, hi) = (dy.min(sy) as usize, dy.max(sy) as usize);
                    let (a, b) = self.pixels.split_at_mut(hi * w);
                    a[lo * w..(lo + 1) * w].swap_with_slice(&mut b[..w]);
                }
                sy -= 1;
                dy += 1;
            }
        }
        self
    }

    fn transform_color_space(
        &mut self,
        decode: &dyn TransferFunction,
        encode: &dyn TransferFunction,
        transform_primaries: Option<&ColorPrimaryTransform>,
    ) -> &mut dyn Image {
        // Don't transform the alpha component.
        let components = N.min(3);
        let rcp = rcp_one::<T>();
        let one = T::one().to_f32();
        let (lo, hi) = (T::min_value().to_f32(), T::max_value().to_f32());
        for c in self.pixels.iter_mut() {
            let mut intensity = [0.0f32; 3];
            // Decode source transfer function
            for comp in 0..components {
                intensity[comp] = decode.decode(c[comp].to_f32() * rcp);
            }
            // If needed, transform primaries
            if let Some(m) = transform_primaries {
                let orig = intensity;
                for j in 0..components {
                    intensity[j] = 0.0;
                    for k in 0..components {
                        intensity[j] = add_x86(intensity[j], m.matrix[j][k] * orig[k]);
                    }
                }
            }
            // Encode destination transfer function
            for comp in 0..components {
                let brightness = encode.encode(intensity[comp]);
                // clamp(value, color::min, color::max) is required as static_cast has
                // platform-specific behaviors and on certain platforms can over or underflow
                c[comp] = T::from_f32(cclamp((brightness * one).round(), lo, hi));
            }
        }
        self
    }

    fn normalize(&mut self) -> &mut dyn Image {
        self.pixels.iter_mut().for_each(color_normalize);
        self
    }

    fn swizzle(&mut self, swizzle: &str) -> &mut dyn Image {
        let sw = swizzle.as_bytes();
        for p in self.pixels.iter_mut() {
            let src = *p;
            for c in 0..N {
                set_comp(p, c, swizzle_pixel(&src, sw.get(c).copied().unwrap_or(0)));
            }
        }
        self
    }

    fn copy_to_r(&self, dst: &mut dyn Image, swizzle: &str) -> ImageResult<()> {
        self.copy_to::<1>(dst, swizzle)
    }
    fn copy_to_rg(&self, dst: &mut dyn Image, swizzle: &str) -> ImageResult<()> {
        self.copy_to::<2>(dst, swizzle)
    }
    fn copy_to_rgb(&self, dst: &mut dyn Image, swizzle: &str) -> ImageResult<()> {
        self.copy_to::<3>(dst, swizzle)
    }
    fn copy_to_rgba(&self, dst: &mut dyn Image, swizzle: &str) -> ImageResult<()> {
        self.copy_to::<4>(dst, swizzle)
    }

    fn premultiply_alpha(&mut self) -> &mut dyn Image {
        if N < 4 {
            return self; // Nothing to do (no alpha channel)
        }
        let tf_srgb = TransferFunctionSRGB;
        let tf_linear = TransferFunctionLinear;
        let tf: &dyn TransferFunction = if self.transfer_function == KHR_DF_TRANSFER_SRGB {
            &tf_srgb
        } else {
            &tf_linear
        };
        let rcp = rcp_one::<T>();
        let one = T::one().to_f32();
        for c in self.pixels.iter_mut() {
            let alpha = c[3].to_f32() * rcp;
            for comp in 0..3 {
                let linear = tf.decode(c[comp].to_f32() * rcp) * alpha;
                let srgb = tf.encode(linear);
                c[comp] = if T::IS_FLOAT {
                    T::from_f32(srgb)
                } else {
                    T::from_f32(srgb * one + 0.5)
                };
            }
        }
        self
    }

    fn map_range(&mut self, scale: f32, offset: f32) -> &mut dyn Image {
        if (scale == 1.0 && offset == 0.0) || !T::IS_FLOAT {
            return self;
        }
        let count = N.min(3); // Skip alpha
        for c in self.pixels.iter_mut() {
            for v in c.iter_mut().take(count) {
                *v = T::from_f32(v.to_f32() * scale + offset);
            }
        }
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn get_int<T: Component, const N: usize>(
    img: &ImageT<T, N>,
    num_channels: u32,
    target_bits: u32,
    signed: bool,
) -> Vec<u8> {
    let target_bytes = (target_bits / 8) as usize;
    let nc = num_channels as usize;
    let mut data = vec![0u8; img.pixels.len() * nc * target_bytes];
    if !matches!(target_bytes, 1 | 2 | 4 | 8) {
        return data;
    }
    for (p, chunk) in img
        .pixels
        .iter()
        .zip(data.chunks_exact_mut((nc * target_bytes).max(1)))
    {
        for c in 0..nc {
            let value = if c < N {
                p[c]
            } else if c != 3 {
                T::zero()
            } else {
                T::unit()
            };
            let v = value.to_int_bytes(target_bytes as u32, signed);
            chunk[c * target_bytes..(c + 1) * target_bytes]
                .copy_from_slice(&v.to_le_bytes()[..target_bytes]);
        }
    }
    data
}
