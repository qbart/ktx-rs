//! Port of astcenc's `astcenc_vecmathlib_sse_4.h`, `astcenc_vecmathlib_common_4.h` and
//! `astcenc_vecmathlib_avx2_8.h` as used by KTX-Software's x86-64 build (AVX2, SSE 4.1,
//! F16C, invariance on, no FP contraction).
//!
//! Every lane operation is IEEE exact like the intrinsics; the emulated details are the
//! `minps`/`maxps` NaN rules (the second operand is returned when either is NaN), ordered
//! compares (`!=` is unordered), `cvttps` overflow to `i32::MIN`, round-half-even `roundps`,
//! the 4-wide `abs` (`max(0 - a, a)`, so `abs(-0) == -0`), and the fixed horizontal
//! reduction orders. Masks keep their lane bits; `select` uses the sign bit like `blendv`.

#![allow(clippy::needless_range_loop)]

use super::sse::{Lanes, LanesI};
use std::ops::{Add, AddAssign, BitAnd, BitOr, BitXor, Div, Mul, Not, Sub};

/// AVX2 build width (`ASTCENC_SIMD_WIDTH`).
pub const SIMD_WIDTH: usize = 8;

/// `_mm_min_ps` lane: `a < b ? a : b`.
#[inline(always)]
pub fn min_ps(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `_mm_max_ps` lane: `a > b ? a : b`.
#[inline(always)]
pub fn max_ps(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// `cvttss2si` (NaN / out of range → `i32::MIN`). Branch free so lane loops vectorize.
#[inline(always)]
pub fn cvtt(f: f32) -> i32 {
    let in_range = (-2147483648.0..2147483648.0).contains(&f);
    let v = f as i32;
    let keep = -(in_range as i32);
    (v & keep) | (i32::MIN & !keep)
}

/// `roundps` nearest (ties to even). `f32::round_ties_even` is a libm call without SSE4.1;
/// adding and subtracting 2^23 rounds the same way in the default rounding mode, and values
/// of 2^23 and above (and NaN / infinities) are already integral.
#[inline(always)]
pub fn round_even(f: f32) -> f32 {
    const C: f32 = 8388608.0;
    let a = f.abs();
    let r = ((a + C) - C).copysign(f);
    if a < C { r } else { f }
}

const TRUE_BITS: u32 = 0xFFFF_FFFF;

#[inline(always)]
fn bits(b: bool) -> u32 {
    if b { TRUE_BITS } else { 0 }
}

macro_rules! vtypes {
    ($vf:ident, $vi:ident, $vm:ident, $n:expr) => {
        /// Float vector.
        #[derive(Debug, Clone, Copy, PartialEq, Default)]
        pub struct $vf {
            pub m: [f32; $n],
        }
        /// Integer vector.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub struct $vi {
            pub m: [i32; $n],
        }
        /// Lane mask (lane bits as produced by the intrinsics).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub struct $vm {
            pub m: [u32; $n],
        }

        impl $vf {
            #[inline(always)]
            pub fn splat(a: f32) -> Self {
                Self { m: [a; $n] }
            }
            #[inline(always)]
            pub fn zero() -> Self {
                Self { m: [0.0; $n] }
            }
            /// Unaligned / aligned load (`vfloatN(const float*)`, `loada`).
            #[inline(always)]
            pub fn load(p: &[f32]) -> Self {
                if let Some(a) = p.first_chunk::<$n>() {
                    return Self { m: *a };
                }
                let mut m = [0.0; $n];
                for (d, s) in m.iter_mut().zip(p) {
                    *d = *s;
                }
                Self { m }
            }
            #[inline(always)]
            pub fn loada(p: &[f32]) -> Self {
                Self::load(p)
            }
            #[inline(always)]
            pub fn load1(p: &f32) -> Self {
                Self::splat(*p)
            }
            #[inline(always)]
            pub fn store(self, p: &mut [f32]) {
                if let Some(a) = p.first_chunk_mut::<$n>() {
                    *a = self.m;
                    return;
                }
                for (d, s) in p.iter_mut().zip(self.m) {
                    *d = s;
                }
            }
            #[inline(always)]
            pub fn lane(&self, l: usize) -> f32 {
                self.m[l]
            }
            #[inline(always)]
            pub fn set_lane(&mut self, l: usize, a: f32) {
                self.m[l] = a;
            }
            #[inline(always)]
            pub fn eq(self, b: Self) -> $vm {
                $vm {
                    m: self.m.cmpeq_ps(b.m),
                }
            }
            /// Unordered not-equal.
            #[inline(always)]
            pub fn ne(self, b: Self) -> $vm {
                $vm {
                    m: self.m.cmpneq_ps(b.m),
                }
            }
            #[inline(always)]
            pub fn lt(self, b: Self) -> $vm {
                $vm {
                    m: self.m.cmplt_ps(b.m),
                }
            }
            #[inline(always)]
            pub fn gt(self, b: Self) -> $vm {
                $vm {
                    m: self.m.cmpgt_ps(b.m),
                }
            }
            #[inline(always)]
            pub fn le(self, b: Self) -> $vm {
                $vm {
                    m: self.m.cmple_ps(b.m),
                }
            }
            #[inline(always)]
            pub fn ge(self, b: Self) -> $vm {
                $vm {
                    m: self.m.cmpge_ps(b.m),
                }
            }
        }

        impl Add for $vf {
            type Output = Self;
            #[inline(always)]
            fn add(self, b: Self) -> Self {
                Self {
                    m: self.m.add_ps(b.m),
                }
            }
        }
        impl Add<f32> for $vf {
            type Output = Self;
            #[inline(always)]
            fn add(self, b: f32) -> Self {
                self + Self::splat(b)
            }
        }
        impl AddAssign for $vf {
            #[inline(always)]
            fn add_assign(&mut self, b: Self) {
                *self = *self + b;
            }
        }
        impl Sub for $vf {
            type Output = Self;
            #[inline(always)]
            fn sub(self, b: Self) -> Self {
                Self {
                    m: self.m.sub_ps(b.m),
                }
            }
        }
        impl Sub<f32> for $vf {
            type Output = Self;
            #[inline(always)]
            fn sub(self, b: f32) -> Self {
                self - Self::splat(b)
            }
        }
        impl Mul for $vf {
            type Output = Self;
            #[inline(always)]
            fn mul(self, b: Self) -> Self {
                Self {
                    m: self.m.mul_ps(b.m),
                }
            }
        }
        impl Mul<f32> for $vf {
            type Output = Self;
            #[inline(always)]
            fn mul(self, b: f32) -> Self {
                self * Self::splat(b)
            }
        }
        impl Mul<$vf> for f32 {
            type Output = $vf;
            #[inline(always)]
            fn mul(self, b: $vf) -> $vf {
                $vf::splat(self) * b
            }
        }
        impl Div for $vf {
            type Output = Self;
            #[inline(always)]
            fn div(self, b: Self) -> Self {
                Self {
                    m: self.m.div_ps(b.m),
                }
            }
        }
        impl Div<f32> for $vf {
            type Output = Self;
            #[inline(always)]
            fn div(self, b: f32) -> Self {
                self / Self::splat(b)
            }
        }
        impl Div<$vf> for f32 {
            type Output = $vf;
            #[inline(always)]
            fn div(self, b: $vf) -> $vf {
                $vf::splat(self) / b
            }
        }

        impl $vi {
            #[inline(always)]
            pub fn splat(a: i32) -> Self {
                Self { m: [a; $n] }
            }
            #[inline(always)]
            pub fn zero() -> Self {
                Self { m: [0; $n] }
            }
            #[inline(always)]
            pub fn load(p: &[i32]) -> Self {
                if let Some(a) = p.first_chunk::<$n>() {
                    return Self { m: *a };
                }
                let mut m = [0; $n];
                for (d, s) in m.iter_mut().zip(p) {
                    *d = *s;
                }
                Self { m }
            }
            #[inline(always)]
            pub fn loada(p: &[i32]) -> Self {
                Self::load(p)
            }
            /// `vintN(const uint8_t*)`: N bytes zero extended.
            #[inline(always)]
            pub fn from_u8(p: &[u8]) -> Self {
                let mut m = [0; $n];
                for (d, s) in m.iter_mut().zip(p) {
                    *d = *s as i32;
                }
                Self { m }
            }
            /// `vintN::load(const uint8_t*)`: 4N bytes as little endian i32 lanes.
            #[inline(always)]
            pub fn load_bytes(p: &[u8]) -> Self {
                let mut m = [0; $n];
                for (i, d) in m.iter_mut().enumerate() {
                    let b = |k: usize| p.get(i * 4 + k).copied().unwrap_or(0);
                    *d = i32::from_le_bytes([b(0), b(1), b(2), b(3)]);
                }
                Self { m }
            }
            #[inline(always)]
            pub fn lane_id() -> Self {
                let mut m = [0; $n];
                for (i, d) in m.iter_mut().enumerate() {
                    *d = i as i32;
                }
                Self { m }
            }
            #[inline(always)]
            pub fn lane(&self, l: usize) -> i32 {
                self.m[l]
            }
            #[inline(always)]
            pub fn set_lane(&mut self, l: usize, a: i32) {
                self.m[l] = a;
            }
            #[inline(always)]
            pub fn store(self, p: &mut [i32]) {
                for (d, s) in p.iter_mut().zip(self.m) {
                    *d = s;
                }
            }
            #[inline(always)]
            pub fn eq(self, b: Self) -> $vm {
                $vm {
                    m: self.m.cmpeq_epi32(b.m),
                }
            }
            #[inline(always)]
            pub fn ne(self, b: Self) -> $vm {
                !self.eq(b)
            }
            #[inline(always)]
            pub fn lt(self, b: Self) -> $vm {
                $vm {
                    m: self.m.cmplt_epi32(b.m),
                }
            }
            #[inline(always)]
            pub fn gt(self, b: Self) -> $vm {
                $vm {
                    m: self.m.cmpgt_epi32(b.m),
                }
            }
            /// `lsl<s>`.
            #[inline(always)]
            pub fn lsl(self, s: u32) -> Self {
                Self {
                    m: self.m.sll_epi32(s),
                }
            }
            /// `lsr<s>`.
            #[inline(always)]
            pub fn lsr(self, s: u32) -> Self {
                Self {
                    m: self.m.srl_epi32(s),
                }
            }
            /// `asr<s>`.
            #[inline(always)]
            pub fn asr(self, s: u32) -> Self {
                Self {
                    m: self.m.sra_epi32(s),
                }
            }
        }

        impl Add for $vi {
            type Output = Self;
            #[inline(always)]
            fn add(self, b: Self) -> Self {
                Self {
                    m: self.m.add_epi32(b.m),
                }
            }
        }
        impl Add<i32> for $vi {
            type Output = Self;
            #[inline(always)]
            fn add(self, b: i32) -> Self {
                self + Self::splat(b)
            }
        }
        impl AddAssign for $vi {
            #[inline(always)]
            fn add_assign(&mut self, b: Self) {
                *self = *self + b;
            }
        }
        impl Sub for $vi {
            type Output = Self;
            #[inline(always)]
            fn sub(self, b: Self) -> Self {
                Self {
                    m: self.m.sub_epi32(b.m),
                }
            }
        }
        impl Sub<i32> for $vi {
            type Output = Self;
            #[inline(always)]
            fn sub(self, b: i32) -> Self {
                self - Self::splat(b)
            }
        }
        impl Mul for $vi {
            type Output = Self;
            #[inline(always)]
            fn mul(self, b: Self) -> Self {
                Self {
                    m: self.m.mullo_epi32(b.m),
                }
            }
        }
        impl Mul<i32> for $vi {
            type Output = Self;
            #[inline(always)]
            fn mul(self, b: i32) -> Self {
                self * Self::splat(b)
            }
        }
        impl BitAnd for $vi {
            type Output = Self;
            #[inline(always)]
            fn bitand(self, b: Self) -> Self {
                Self {
                    m: self.m.and_si(b.m),
                }
            }
        }
        impl BitAnd<i32> for $vi {
            type Output = Self;
            #[inline(always)]
            fn bitand(self, b: i32) -> Self {
                self & Self::splat(b)
            }
        }
        impl BitOr for $vi {
            type Output = Self;
            #[inline(always)]
            fn bitor(self, b: Self) -> Self {
                Self {
                    m: self.m.or_si(b.m),
                }
            }
        }
        impl BitOr<i32> for $vi {
            type Output = Self;
            #[inline(always)]
            fn bitor(self, b: i32) -> Self {
                self | Self::splat(b)
            }
        }
        impl BitXor for $vi {
            type Output = Self;
            #[inline(always)]
            fn bitxor(self, b: Self) -> Self {
                Self {
                    m: self.m.xor_si(b.m),
                }
            }
        }
        impl BitXor<i32> for $vi {
            type Output = Self;
            #[inline(always)]
            fn bitxor(self, b: i32) -> Self {
                self ^ Self::splat(b)
            }
        }
        impl Not for $vi {
            type Output = Self;
            #[inline(always)]
            fn not(self) -> Self {
                Self {
                    m: self.m.map(|x| !x),
                }
            }
        }

        impl $vm {
            #[inline(always)]
            pub fn splat(a: bool) -> Self {
                Self { m: [bits(a); $n] }
            }
            /// `lane<l>()`: the lane bits read as a float compared with 0.
            #[inline(always)]
            pub fn lane(&self, l: usize) -> bool {
                f32::from_bits(self.m[l]) != 0.0
            }
        }
        impl BitAnd for $vm {
            type Output = Self;
            #[inline(always)]
            fn bitand(self, b: Self) -> Self {
                let mut m = self.m;
                for i in 0..$n {
                    m[i] &= b.m[i];
                }
                Self { m }
            }
        }
        impl BitOr for $vm {
            type Output = Self;
            #[inline(always)]
            fn bitor(self, b: Self) -> Self {
                let mut m = self.m;
                for i in 0..$n {
                    m[i] |= b.m[i];
                }
                Self { m }
            }
        }
        impl BitXor for $vm {
            type Output = Self;
            #[inline(always)]
            fn bitxor(self, b: Self) -> Self {
                let mut m = self.m;
                for i in 0..$n {
                    m[i] ^= b.m[i];
                }
                Self { m }
            }
        }
        impl Not for $vm {
            type Output = Self;
            #[inline(always)]
            fn not(self) -> Self {
                Self {
                    m: self.m.map(|x| !x),
                }
            }
        }

        impl From<$vm> for $vi {
            /// The mask lanes as integers (`_mm_castps_si128`).
            #[inline(always)]
            fn from(v: $vm) -> $vi {
                $vi {
                    m: v.m.map(|x| x as i32),
                }
            }
        }

        impl VecOps for $vf {
            type I = $vi;
            type M = $vm;
            const N: usize = $n;
            #[inline(always)]
            fn splat_(a: f32) -> Self {
                Self::splat(a)
            }
            #[inline(always)]
            fn lanes(&self) -> &[f32] {
                &self.m
            }
        }
    };
}

/// Shared lane-generic helpers.
pub trait VecOps: Copy {
    type I;
    type M;
    const N: usize;
    fn splat_(a: f32) -> Self;
    fn lanes(&self) -> &[f32];
}

vtypes!(Vfloat4, Vint4, Vmask4, 4);
vtypes!(Vfloat8, Vint8, Vmask8, 8);

macro_rules! common_fns {
    ($vf:ident, $vi:ident, $vm:ident, $n:expr) => {
        impl $vf {
            #[inline(always)]
            pub fn min(self, b: Self) -> Self {
                Self {
                    m: self.m.min_ps(b.m),
                }
            }
            #[inline(always)]
            pub fn max(self, b: Self) -> Self {
                Self {
                    m: self.m.max_ps(b.m),
                }
            }
            #[inline(always)]
            pub fn min_s(self, b: f32) -> Self {
                self.min(Self::splat(b))
            }
            #[inline(always)]
            pub fn max_s(self, b: f32) -> Self {
                self.max(Self::splat(b))
            }
            /// `clamp(min, max, a)`: `min(max(a, min), max)`.
            #[inline(always)]
            pub fn clamp(lo: f32, hi: f32, a: Self) -> Self {
                a.max_s(lo).min_s(hi)
            }
            #[inline(always)]
            pub fn clampzo(self) -> Self {
                Self::clamp(0.0, 1.0, self)
            }
            /// `round` (nearest, ties to even).
            #[inline(always)]
            pub fn round(self) -> Self {
                Self {
                    m: self.m.round_ps(),
                }
            }
            #[inline(always)]
            pub fn sqrt(self) -> Self {
                Self {
                    m: self.m.sqrt_ps(),
                }
            }
            /// `select(a, b, cond)`: `b` where the mask lane sign bit is set.
            #[inline(always)]
            pub fn select(a: Self, b: Self, cond: $vm) -> Self {
                Self {
                    m: a.m.blendv_ps(b.m, cond.m),
                }
            }
            /// `gatherf(base, indices)`.
            #[inline(always)]
            pub fn gatherf(base: &[f32], idx: $vi) -> Self {
                Self {
                    m: idx.m.map(|i| base.get(i as usize).copied().unwrap_or(0.0)),
                }
            }
            /// `gatherf_byte_inds`.
            #[inline(always)]
            pub fn gatherf_byte_inds(base: &[f32], idx: &[u8]) -> Self {
                // Full vector of indices: clamped (provably in bounds, no per lane branch) and
                // out of range lanes zeroed with a select.
                if let (Some(ix), Some(last)) = (idx.first_chunk::<$n>(), base.len().checked_sub(1))
                {
                    let mut m = [0.0; $n];
                    for k in 0..$n {
                        let i = ix[k] as usize;
                        let v = base[i.min(last)];
                        m[k] = if i <= last { v } else { 0.0 };
                    }
                    return Self { m };
                }
                let mut m = [0.0; $n];
                for (d, i) in m.iter_mut().zip(idx) {
                    *d = base.get(*i as usize).copied().unwrap_or(0.0);
                }
                Self { m }
            }
            #[inline(always)]
            pub fn float_to_int(self) -> $vi {
                $vi {
                    m: self.m.cvttps_epi32(),
                }
            }
            #[inline(always)]
            pub fn float_to_int_rtn(self) -> $vi {
                (self + 0.5).float_to_int()
            }
            #[inline(always)]
            pub fn float_as_int(self) -> $vi {
                $vi {
                    m: self.m.map(|f| f.to_bits() as i32),
                }
            }
        }
        impl $vi {
            #[inline(always)]
            pub fn min(self, b: Self) -> Self {
                Self {
                    m: self.m.min_epi32(b.m),
                }
            }
            #[inline(always)]
            pub fn max(self, b: Self) -> Self {
                Self {
                    m: self.m.max_epi32(b.m),
                }
            }
            #[inline(always)]
            pub fn clamp(lo: i32, hi: i32, a: Self) -> Self {
                a.max(Self::splat(lo)).min(Self::splat(hi))
            }
            #[inline(always)]
            pub fn hmin_s(self) -> i32 {
                self.m.iter().copied().fold(i32::MAX, i32::min)
            }
            #[inline(always)]
            pub fn hmax_s(self) -> i32 {
                self.m.iter().copied().fold(i32::MIN, i32::max)
            }
            #[inline(always)]
            pub fn hmin(self) -> Self {
                Self::splat(self.hmin_s())
            }
            #[inline(always)]
            pub fn hmax(self) -> Self {
                Self::splat(self.hmax_s())
            }
            #[inline(always)]
            pub fn select(a: Self, b: Self, cond: $vm) -> Self {
                Self {
                    m: a.m.blendv_epi32(b.m, cond.m),
                }
            }
            #[inline(always)]
            pub fn int_to_float(self) -> $vf {
                $vf {
                    m: self.m.cvtepi32_ps(),
                }
            }
            #[inline(always)]
            pub fn int_as_float(self) -> $vf {
                $vf {
                    m: self.m.map(|i| f32::from_bits(i as u32)),
                }
            }
            /// `store_nbytes` of the low lanes' bytes (`pack_and_store_low_bytes`).
            #[inline(always)]
            pub fn pack_low_bytes(self) -> [u8; $n] {
                self.m.map(|x| x as u8)
            }
        }
        impl $vm {
            /// `mask()`: the lane sign bits.
            #[inline(always)]
            pub fn mask(self) -> u32 {
                let mut r = 0;
                for i in 0..$n {
                    r |= ((self.m[i] >> 31) & 1) << i;
                }
                r
            }
            #[inline(always)]
            pub fn any(self) -> bool {
                self.mask() != 0
            }
            #[inline(always)]
            pub fn all(self) -> bool {
                self.mask() == (1u32 << $n) - 1
            }
        }
    };
}

common_fns!(Vfloat4, Vint4, Vmask4, 4);
common_fns!(Vfloat8, Vint8, Vmask8, 8);

// ------------------------------------------------------------------- 4-wide specifics

impl Vfloat4 {
    #[inline(always)]
    pub fn new(a: f32, b: f32, c: f32, d: f32) -> Vfloat4 {
        Vfloat4 { m: [a, b, c, d] }
    }
    /// `vfloat3(a, b, c)` (lane 3 zero).
    #[inline(always)]
    pub fn new3(a: f32, b: f32, c: f32) -> Vfloat4 {
        Vfloat4 { m: [a, b, c, 0.0] }
    }
    /// `swz<l0, l1>()` (lanes 2, 3 zero).
    #[inline(always)]
    pub fn swz2(self, l0: usize, l1: usize) -> Vfloat4 {
        Vfloat4::new(self.m[l0], self.m[l1], 0.0, 0.0)
    }
    /// `swz<l0, l1, l2>()` (lane 3 zero).
    #[inline(always)]
    pub fn swz3(self, l0: usize, l1: usize, l2: usize) -> Vfloat4 {
        Vfloat4::new(self.m[l0], self.m[l1], self.m[l2], 0.0)
    }
    /// `swz<l0, l1, l2, l3>()`.
    #[inline(always)]
    pub fn swz4(self, l0: usize, l1: usize, l2: usize, l3: usize) -> Vfloat4 {
        Vfloat4::new(self.m[l0], self.m[l1], self.m[l2], self.m[l3])
    }
    /// `abs`: `max(0 - a, a)` (so `-0` stays `-0`, NaN passes through).
    #[inline(always)]
    pub fn abs(self) -> Vfloat4 {
        (Vfloat4::zero() - self).max(self)
    }
    /// `hmin` (broadcast).
    #[inline(always)]
    pub fn hmin(self) -> Vfloat4 {
        Vfloat4::splat(self.hmin_s())
    }
    #[inline(always)]
    pub fn hmin_s(self) -> f32 {
        let a = self.m;
        min_ps(min_ps(a[0], a[2]), min_ps(a[1], a[3]))
    }
    /// `hmin_rgb_s`: lane 3 replaced by lane 0.
    #[inline(always)]
    pub fn hmin_rgb_s(self) -> f32 {
        let mut a = self;
        a.m[3] = a.m[0];
        a.hmin_s()
    }
    #[inline(always)]
    pub fn hmax(self) -> Vfloat4 {
        Vfloat4::splat(self.hmax_s())
    }
    #[inline(always)]
    pub fn hmax_s(self) -> f32 {
        let a = self.m;
        max_ps(max_ps(a[0], a[2]), max_ps(a[1], a[3]))
    }
    /// `hadd_s`: `(a0 + a2) + (a1 + a3)`.
    #[inline(always)]
    pub fn hadd_s(self) -> f32 {
        let a = self.m;
        (a[0] + a[2]) + (a[1] + a[3])
    }
    /// `hadd_rgb_s`: `a0 + a1 + a2`.
    #[inline(always)]
    pub fn hadd_rgb_s(self) -> f32 {
        self.m[0] + self.m[1] + self.m[2]
    }
    #[inline(always)]
    pub fn dot_s(self, b: Vfloat4) -> f32 {
        (self * b).hadd_s()
    }
    #[inline(always)]
    pub fn dot(self, b: Vfloat4) -> Vfloat4 {
        Vfloat4::splat(self.dot_s(b))
    }
    #[inline(always)]
    pub fn dot3_s(self, b: Vfloat4) -> f32 {
        (self * b).hadd_rgb_s()
    }
    #[inline(always)]
    pub fn dot3(self, b: Vfloat4) -> Vfloat4 {
        let d = self.dot3_s(b);
        Vfloat4::new(d, d, d, 0.0)
    }
    /// `haccumulate(accum, a)`.
    #[inline(always)]
    pub fn haccumulate(accum: &mut Vfloat4, a: Vfloat4) {
        *accum += a;
    }
    /// `haccumulate(accum, a, m)`.
    #[inline(always)]
    pub fn haccumulate_masked(accum: &mut Vfloat4, a: Vfloat4, m: Vmask4) {
        Vfloat4::haccumulate(accum, Vfloat4::select(Vfloat4::zero(), a, m));
    }
}

impl Vint4 {
    #[inline(always)]
    pub fn new(a: i32, b: i32, c: i32, d: i32) -> Vint4 {
        Vint4 { m: [a, b, c, d] }
    }
    /// `hadd_rgb_s`.
    #[inline(always)]
    pub fn hadd_rgb_s(self) -> i32 {
        self.m[0].wrapping_add(self.m[1]).wrapping_add(self.m[2])
    }
}

impl Vmask4 {
    #[inline(always)]
    pub fn new(a: bool, b: bool, c: bool, d: bool) -> Vmask4 {
        Vmask4 {
            m: [bits(a), bits(b), bits(c), bits(d)],
        }
    }
}

// ------------------------------------------------------------------- 8-wide specifics

impl Vfloat8 {
    /// `abs`: sign bit cleared.
    #[inline(always)]
    pub fn abs(self) -> Vfloat8 {
        Vfloat8 {
            m: self.m.map(|f| f32::from_bits(f.to_bits() & 0x7fff_ffff)),
        }
    }
    #[inline(always)]
    fn halves(self) -> (Vfloat4, Vfloat4) {
        let a = self.m;
        (
            Vfloat4::new(a[0], a[1], a[2], a[3]),
            Vfloat4::new(a[4], a[5], a[6], a[7]),
        )
    }
    /// `hmin_s`.
    #[inline(always)]
    pub fn hmin_s(self) -> f32 {
        let (lo, hi) = self.halves();
        let v = lo.min(hi).m;
        min_ps(min_ps(v[0], v[1]), min_ps(v[2], v[3]))
    }
    #[inline(always)]
    pub fn hmin(self) -> Vfloat8 {
        Vfloat8::splat(self.hmin_s())
    }
    /// `hmax_s`.
    #[inline(always)]
    pub fn hmax_s(self) -> f32 {
        let (lo, hi) = self.halves();
        let v = lo.max(hi).m;
        max_ps(max_ps(v[0], v[1]), max_ps(v[2], v[3]))
    }
    #[inline(always)]
    pub fn hmax(self) -> Vfloat8 {
        Vfloat8::splat(self.hmax_s())
    }
    /// `hadd_s`: two sequential 4-wide sums.
    #[inline(always)]
    pub fn hadd_s(self) -> f32 {
        let (lo, hi) = self.halves();
        lo.hadd_s() + hi.hadd_s()
    }
    /// `haccumulate(vfloat4& accum, vfloat8 a)`.
    #[inline(always)]
    pub fn haccumulate(accum: &mut Vfloat4, a: Vfloat8) {
        let (lo, hi) = a.halves();
        Vfloat4::haccumulate(accum, lo);
        Vfloat4::haccumulate(accum, hi);
    }
    /// `haccumulate(vfloat4& accum, vfloat8 a, vmask8 m)`.
    #[inline(always)]
    pub fn haccumulate_masked(accum: &mut Vfloat4, a: Vfloat8, m: Vmask8) {
        Vfloat8::haccumulate(accum, Vfloat8::select(Vfloat8::zero(), a, m));
    }
}

/// The AVX2 build's `vfloat` / `vint` / `vmask`.
pub type Vfloat = Vfloat8;
pub type Vint = Vint8;
pub type Vmask = Vmask8;

/// `round_up_to_simd_multiple_vla`.
#[inline(always)]
pub const fn round_up_to_simd_multiple_vla(count: usize) -> usize {
    count.div_ceil(SIMD_WIDTH) * SIMD_WIDTH
}

/// `round_down_to_simd_multiple_vla`.
#[inline(always)]
pub const fn round_down_to_simd_multiple_vla(count: usize) -> usize {
    count & !(SIMD_WIDTH - 1)
}

/// `round_down_to_simd_multiple_4`.
#[inline(always)]
pub const fn round_down_to_simd_multiple_4(count: usize) -> usize {
    count & !3
}

/// `vint_from_size`.
#[inline(always)]
pub fn vint_from_size(a: usize) -> Vint {
    Vint::splat(a as i32)
}

/// `popcount` (64-bit).
#[inline(always)]
pub fn popcount(v: u64) -> i32 {
    v.count_ones() as i32
}

/// `bit_transfer_signed`.
#[inline(always)]
pub fn bit_transfer_signed(input0: &mut Vint4, input1: &mut Vint4) {
    *input1 = input1.lsr(1) | (*input0 & 0x80);
    *input0 = input0.lsr(1) & 0x3F;
    let mask = (*input0 & 0x20).ne(Vint4::zero());
    *input0 = Vint4::select(*input0, *input0 - 0x40, mask);
}

// ------------------------------------------------------------------- astcenc_vecmathlib.h

pub const PI: f32 = std::f32::consts::PI;
pub const PI_OVER_TWO: f32 = std::f32::consts::FRAC_PI_2;

/// `change_sign`: `a` with the sign of `b` xored in.
#[inline(always)]
pub fn change_sign(a: Vfloat, b: Vfloat) -> Vfloat {
    let ia = a.float_as_int();
    let ib = b.float_as_int();
    let sign_mask = Vint::splat(0x8000_0000u32 as i32);
    (ia ^ (ib & sign_mask)).int_as_float()
}

/// `atan` (approximation).
#[inline(always)]
pub fn atan(x: Vfloat) -> Vfloat {
    let c = x.abs().gt(Vfloat::splat(1.0));
    let z = change_sign(Vfloat::splat(PI_OVER_TWO), x);
    let mut y = Vfloat::select(x, Vfloat::splat(1.0) / x, c);
    y = y / (y * y * Vfloat::splat(0.28) + Vfloat::splat(1.0));
    Vfloat::select(y, z - y, c)
}

/// `atan2` (approximation).
#[inline(always)]
pub fn atan2(y: Vfloat, x: Vfloat) -> Vfloat {
    let z = atan((y / x).abs());
    let xmask = x.lt(Vfloat::zero());
    change_sign(Vfloat::select(z, Vfloat::splat(PI) - z, xmask), y)
}

/// `normalize`.
#[inline(always)]
pub fn normalize(a: Vfloat4) -> Vfloat4 {
    let length = a.dot(a);
    a / length.sqrt()
}

/// `normalize_safe`.
#[inline(always)]
pub fn normalize_safe(a: Vfloat4, safe: Vfloat4) -> Vfloat4 {
    let length = a.dot(a);
    if length.lane(0) != 0.0 {
        return a / length.sqrt();
    }
    safe
}

/// `POLY` evaluation `((((c5 * x + c4) * x + c3) ...) * x + c0`.
#[inline(always)]
fn poly(x: Vfloat4, c: &[f32]) -> Vfloat4 {
    let mut r = Vfloat4::splat(c[c.len() - 1]);
    for &ci in c[..c.len() - 1].iter().rev() {
        r = r * x + ci;
    }
    r
}

/// `exp2` (approximation).
#[allow(clippy::excessive_precision)]
#[inline(always)]
pub fn exp2(x: Vfloat4) -> Vfloat4 {
    let x = Vfloat4::clamp(-126.99999, 129.0, x);
    let ipart = (x - 0.5).float_to_int();
    let fpart = x - ipart.int_to_float();
    // Integer contrib, using 1 << ipart
    let iexp = (ipart + 127).lsl(23).int_as_float();
    // Fractional contrib, using polynomial fit of 2^x in range [-0.5, 0.5)
    let fexp = poly(
        fpart,
        &[
            9.9999994e-1,
            6.9315308e-1,
            2.4015361e-1,
            5.5826318e-2,
            8.9893397e-3,
            1.8775767e-3,
        ],
    );
    iexp * fexp
}

/// `log2` (approximation).
#[allow(clippy::excessive_precision)]
#[inline(always)]
pub fn log2(x: Vfloat4) -> Vfloat4 {
    let exp = Vint4::splat(0x7F80_0000);
    let mant = Vint4::splat(0x007F_FFFF);
    let one = Vint4::splat(0x3F80_0000);
    let i = x.float_as_int();
    let e = ((i & exp).lsr(23) - 127).int_to_float();
    let m = ((i & mant) | one).int_as_float();
    // Polynomial fit of log2(x)/(x - 1), for x in range [1, 2)
    let mut p = poly(
        m,
        &[
            2.8882704548164776201,
            -2.52074962577807006663,
            1.48116647521213171641,
            -0.465725644288844778798,
            0.0596515482674574969533,
        ],
    );
    // Increases the polynomial degree, but ensures that log2(1) == 0
    p = p * (m - 1.0);
    p + e
}

/// `pow` (approximation, exact for `y == 0`).
#[inline(always)]
pub fn pow(x: Vfloat4, y: Vfloat4) -> Vfloat4 {
    let zero_mask = y.eq(Vfloat4::splat(0.0));
    let estimate = exp2(log2(x) * y);
    Vfloat4::select(estimate, Vfloat4::splat(1.0), zero_mask)
}

/// `astc::pow`.
#[inline(always)]
pub fn pow_s(x: f32, y: f32) -> f32 {
    pow(Vfloat4::splat(x), Vfloat4::splat(y)).lane(0)
}

/// `frexp`.
#[inline(always)]
pub fn frexp(a: Vfloat4, exp: &mut Vint4) -> Vfloat4 {
    let ai = a.float_as_int();
    *exp = (ai.lsr(23) & 0xFF) - 126;
    let manti = (ai & (0x807F_FFFFu32 as i32)) | 0x3F00_0000;
    manti.int_as_float()
}

/// `float_to_lns`.
#[inline(always)]
pub fn float_to_lns(a: Vfloat4) -> Vfloat4 {
    let mut exp = Vint4::zero();
    let mant = frexp(a, &mut exp);
    // Do these early before we start messing about ...
    let mask_underflow_nan = !a.gt(Vfloat4::splat(1.0 / 67108864.0));
    let mask_infinity = a.ge(Vfloat4::splat(65536.0));
    // If input is smaller than 2^-14, multiply by 2^25 and don't bias.
    let exp_lt_m13 = exp.lt(Vint4::splat(-13));
    let a1a = a * 33554432.0;
    let expa = Vint4::zero();
    let a1b = (mant - 0.5) * 4096.0;
    let expb = exp + 14;
    let mut a = Vfloat4::select(a1b, a1a, exp_lt_m13);
    let exp = Vint4::select(expb, expa, exp_lt_m13);
    let a_lt_384 = a.lt(Vfloat4::splat(384.0));
    let a_lt_1408 = a.le(Vfloat4::splat(1408.0));
    let a2a = a * (4.0f32 / 3.0f32);
    let a2b = a + 128.0;
    let a2c = (a + 512.0) * (4.0f32 / 5.0f32);
    a = a2c;
    a = Vfloat4::select(a, a2b, a_lt_1408);
    a = Vfloat4::select(a, a2a, a_lt_384);
    a = a + (exp.int_to_float() * 2048.0) + 1.0;
    a = Vfloat4::select(a, Vfloat4::splat(65535.0), mask_infinity);
    Vfloat4::select(a, Vfloat4::zero(), mask_underflow_nan)
}

/// `clz`.
#[inline(always)]
pub fn clz(a: Vint4) -> Vint4 {
    let a = (!a.lsr(8)) & a;
    let a = a.int_to_float().float_as_int();
    let a = Vint4::splat(127 + 31) - a.lsr(23);
    Vint4::clamp(0, 32, a)
}

/// `two_to_the_n`.
#[inline(always)]
pub fn two_to_the_n(a: Vint4) -> Vint4 {
    let exp = (a + 127).lsl(23);
    exp.int_as_float().float_to_int()
}

/// `unorm16_to_sf16`.
#[inline(always)]
pub fn unorm16_to_sf16(p: Vint4) -> Vint4 {
    let fp16_one = Vint4::splat(0x3C00);
    let fp16_small = p.lsl(8);
    let is_one = p.eq(Vint4::splat(0xFFFF));
    let is_small = p.lt(Vint4::splat(4));
    let lz = clz(p) - 16;
    let mut p = p * two_to_the_n(lz + 1);
    p = p & Vint4::splat(0xFFFF);
    p = p.lsr(6);
    p = p | (Vint4::splat(14) - lz).lsl(10);
    let r = Vint4::select(p, fp16_one, is_one);
    Vint4::select(r, fp16_small, is_small)
}

/// `lns_to_sf16`.
#[inline(always)]
pub fn lns_to_sf16(p: Vint4) -> Vint4 {
    let mc = p & 0x7FF;
    let ec = p.lsr(11);
    let mc_512 = mc * 3;
    let mask_512 = mc.lt(Vint4::splat(512));
    let mc_1536 = mc * 4 - 512;
    let mask_1536 = mc.lt(Vint4::splat(1536));
    let mc_else = mc * 5 - 2048;
    let mut mt = mc_else;
    mt = Vint4::select(mt, mc_1536, mask_1536);
    mt = Vint4::select(mt, mc_512, mask_512);
    let res = ec.lsl(10) | mt.lsr(3);
    res.min(Vint4::splat(0x7BFF))
}

/// Vector `float16_to_float` (`_mm_packs_epi32` signed saturation, then F16C):
/// lanes >= 0x8000 become 0x7FFF (NaN).
#[inline(always)]
pub fn float16_to_float_v(a: Vint4) -> Vfloat4 {
    Vfloat4 {
        m: a.m
            .map(|x| super::float16_to_float(x.clamp(-32768, 32767) as i16 as u16)),
    }
}

/// Vector `float_to_float16` (F16C, round to nearest even).
#[inline(always)]
pub fn float_to_float16_v(a: Vfloat4) -> Vint4 {
    Vint4 {
        m: a.m.map(|f| super::float_to_float16(f) as i32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_even_matches_round_ties_even() {
        let mut x = 0x1234_5678u32;
        let mut values = vec![
            0.5f32,
            1.5,
            2.5,
            -0.5,
            -1.5,
            -2.5,
            -0.0,
            0.0,
            0.49999997,
            8388607.5,
            -8388607.5,
            8388608.0,
            16777217.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MAX,
            f32::MIN,
            f32::NAN,
        ];
        for _ in 0..1_000_000 {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            values.push(f32::from_bits(x));
        }
        for f in values {
            let (a, b) = (round_even(f), f.round_ties_even());
            assert!(
                a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()),
                "{f}: {a} {b}"
            );
        }
    }

    #[test]
    fn cvtt_matches_cvttss2si() {
        for (f, want) in [
            (1.9f32, 1),
            (-1.9, -1),
            (2147483520.0, 2147483520),
            (2147483648.0, i32::MIN),
            (-2147483648.0, i32::MIN),
            (-2147483904.0, i32::MIN),
            (f32::NAN, i32::MIN),
            (f32::INFINITY, i32::MIN),
            (f32::NEG_INFINITY, i32::MIN),
        ] {
            assert_eq!(cvtt(f), want, "{f}");
        }
    }
}
