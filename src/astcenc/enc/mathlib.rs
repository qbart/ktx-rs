// SPDX-FileCopyrightText: 2011-2026 Arm Limited
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from astc-encoder and modified; see NOTICE.md.
//! Port of the scalar helpers of `astcenc_mathlib.h` / `astcenc_mathlib.cpp` used by the
//! compressor (C++ casts and comparisons as on x86-64).

use ktx_macros::isa;

#[isa]
/// `astc::min` (`p < q ? p : q`; NaN gives `q`).
#[inline]
pub fn min(p: f32, q: f32) -> f32 {
    if p < q { p } else { q }
}

#[isa]
/// `astc::max` (`p > q ? p : q`; NaN gives `q`).
#[inline]
pub fn max(p: f32, q: f32) -> f32 {
    if p > q { p } else { q }
}

#[isa]
/// `astc::clamp(v, mn, mx)` (NaN gives `mn`).
#[inline]
pub fn clamp(v: f32, mn: f32, mx: f32) -> f32 {
    if v > mx {
        mx
    } else if v > mn {
        v
    } else {
        mn
    }
}

#[isa]
/// `astc::clamp1f`.
#[inline]
pub fn clamp1f(v: f32) -> f32 {
    clamp(v, 0.0, 1.0)
}

#[isa]
/// `astc::clamp255f`.
#[inline]
pub fn clamp255f(v: f32) -> f32 {
    clamp(v, 0.0, 255.0)
}

#[isa]
/// `astc::flt_rd`.
#[inline]
pub fn flt_rd(v: f32) -> f32 {
    v.floor()
}

#[isa]
/// `static_cast<int>(float)` (`cvttss2si`).
#[inline]
pub fn f2i(v: f32) -> i32 {
    super::vecmath::cvtt(v)
}

#[isa]
/// `astc::flt2int_rtn`.
#[inline]
pub fn flt2int_rtn(v: f32) -> i32 {
    f2i(v + 0.5)
}

#[isa]
/// `astc::flt2int_rd`.
#[inline]
pub fn flt2int_rd(v: f32) -> i32 {
    f2i(v)
}

#[isa]
/// `astc::rsqrt`.
#[inline]
pub fn rsqrt(v: f32) -> f32 {
    1.0 / v.sqrt()
}

#[isa]
/// `astc::rand_init`.
pub fn rand_init() -> [u64; 2] {
    [0xfaf9e171cea1ec6b, 0xf1b318cc06af5d71]
}

#[isa]
/// `astc::rand` (xoroshiro128+).
pub fn rand(state: &mut [u64; 2]) -> u64 {
    let s0 = state[0];
    let mut s1 = state[1];
    let res = s0.wrapping_add(s1);
    s1 ^= s0;
    state[0] = s0.rotate_left(24) ^ s1 ^ (s1 << 16);
    state[1] = s1.rotate_left(37);
    res
}
