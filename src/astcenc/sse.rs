//! 4-lane kernels of `astcenc_vecmathlib_sse_4.h` on `std::arch`. The vector types keep their
//! array storage; each kernel casts to `__m128` / `__m128i` (`bytemuck`, free) and runs the
//! intrinsic. SSE4.1 instructions are used when the build enables them, otherwise the SSE2
//! sequences below give bit identical results.
//!
//! On other architectures the same functions are plain lane loops.
//
// SAFETY (whole module): every intrinsic used here only requires SSE or SSE2, which are part of
// the x86_64 baseline and therefore always available, or SSE4.1 / AVX2 behind
// `cfg(target_feature = ...)`, i.e. only when the compiled-for CPU has them. None of them
// dereference pointers.
#![allow(unsafe_code)]

pub type F4 = [f32; 4];
pub type I4 = [i32; 4];
pub type M4 = [u32; 4];

#[cfg(target_arch = "x86_64")]
mod imp {
    use super::{F4, I4, M4};
    use std::arch::x86_64::*;

    #[inline(always)]
    fn f(a: F4) -> __m128 {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn i(a: I4) -> __m128i {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn mi(a: M4) -> __m128i {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn of(a: __m128) -> F4 {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn oi(a: __m128i) -> I4 {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn om(a: __m128) -> M4 {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn omi(a: __m128i) -> M4 {
        bytemuck::cast(a)
    }

    macro_rules! fop {
        ($name:ident, $intr:ident) => {
            #[inline(always)]
            pub fn $name(a: F4, b: F4) -> F4 {
                of(unsafe { $intr(f(a), f(b)) })
            }
        };
    }
    macro_rules! fcmp {
        ($name:ident, $intr:ident) => {
            #[inline(always)]
            pub fn $name(a: F4, b: F4) -> M4 {
                om(unsafe { $intr(f(a), f(b)) })
            }
        };
    }
    macro_rules! iop {
        ($name:ident, $intr:ident) => {
            #[inline(always)]
            pub fn $name(a: I4, b: I4) -> I4 {
                oi(unsafe { $intr(i(a), i(b)) })
            }
        };
    }

    fop!(add_ps, _mm_add_ps);
    fop!(sub_ps, _mm_sub_ps);
    fop!(mul_ps, _mm_mul_ps);
    fop!(div_ps, _mm_div_ps);
    fop!(min_ps, _mm_min_ps);
    fop!(max_ps, _mm_max_ps);
    fcmp!(cmpeq_ps, _mm_cmpeq_ps);
    fcmp!(cmpneq_ps, _mm_cmpneq_ps);
    fcmp!(cmplt_ps, _mm_cmplt_ps);
    fcmp!(cmpgt_ps, _mm_cmpgt_ps);
    fcmp!(cmple_ps, _mm_cmple_ps);
    fcmp!(cmpge_ps, _mm_cmpge_ps);
    iop!(add_epi32, _mm_add_epi32);
    iop!(sub_epi32, _mm_sub_epi32);
    iop!(and_si128, _mm_and_si128);
    iop!(or_si128, _mm_or_si128);
    iop!(xor_si128, _mm_xor_si128);

    #[inline(always)]
    pub fn sqrt_ps(a: F4) -> F4 {
        of(unsafe { _mm_sqrt_ps(f(a)) })
    }

    /// `_mm_cvttps_epi32`.
    #[inline(always)]
    pub fn cvttps_epi32(a: F4) -> I4 {
        oi(unsafe { _mm_cvttps_epi32(f(a)) })
    }

    /// `_mm_cvtepi32_ps`.
    #[inline(always)]
    pub fn cvtepi32_ps(a: I4) -> F4 {
        of(unsafe { _mm_cvtepi32_ps(i(a)) })
    }

    #[inline(always)]
    pub fn cmpeq_epi32(a: I4, b: I4) -> M4 {
        omi(unsafe { _mm_cmpeq_epi32(i(a), i(b)) })
    }
    #[inline(always)]
    pub fn cmplt_epi32(a: I4, b: I4) -> M4 {
        omi(unsafe { _mm_cmplt_epi32(i(a), i(b)) })
    }
    #[inline(always)]
    pub fn cmpgt_epi32(a: I4, b: I4) -> M4 {
        omi(unsafe { _mm_cmpgt_epi32(i(a), i(b)) })
    }

    /// `_mm_blendv_ps` (`b` where the mask sign bit is set).
    #[inline(always)]
    pub fn blendv_ps(a: F4, b: F4, m: M4) -> F4 {
        #[cfg(target_feature = "sse4.1")]
        {
            of(unsafe { _mm_blendv_ps(f(a), f(b), bytemuck::cast(m)) })
        }
        #[cfg(not(target_feature = "sse4.1"))]
        {
            of(unsafe {
                _mm_castsi128_ps(blendv_si(
                    _mm_castps_si128(f(a)),
                    _mm_castps_si128(f(b)),
                    mi(m),
                ))
            })
        }
    }

    /// Integer `blendv` on the mask sign bit.
    #[inline(always)]
    pub fn blendv_epi32(a: I4, b: I4, m: M4) -> I4 {
        oi(blendv_si(i(a), i(b), mi(m)))
    }

    #[inline(always)]
    fn blendv_si(a: __m128i, b: __m128i, m: __m128i) -> __m128i {
        unsafe {
            #[cfg(target_feature = "sse4.1")]
            {
                _mm_castps_si128(_mm_blendv_ps(
                    _mm_castsi128_ps(a),
                    _mm_castsi128_ps(b),
                    _mm_castsi128_ps(m),
                ))
            }
            #[cfg(not(target_feature = "sse4.1"))]
            {
                let m = _mm_srai_epi32::<31>(m);
                _mm_or_si128(_mm_andnot_si128(m, a), _mm_and_si128(m, b))
            }
        }
    }

    /// `_mm_round_ps` nearest. SSE2: add / subtract 2^23 on the magnitude, keep values of 2^23
    /// and above (and NaN / infinities, whose compare fails), restore the sign.
    #[inline(always)]
    pub fn round_ps(a: F4) -> F4 {
        unsafe {
            #[cfg(target_feature = "sse4.1")]
            {
                of(_mm_round_ps::<
                    { _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC },
                >(f(a)))
            }
            #[cfg(not(target_feature = "sse4.1"))]
            {
                let v = f(a);
                let sign = _mm_set1_ps(-0.0);
                let c = _mm_set1_ps(8388608.0);
                let mag = _mm_andnot_ps(sign, v);
                let r = _mm_sub_ps(_mm_add_ps(mag, c), c);
                let r = _mm_or_ps(r, _mm_and_ps(sign, v));
                let small = _mm_cmplt_ps(mag, c);
                of(_mm_or_ps(_mm_and_ps(small, r), _mm_andnot_ps(small, v)))
            }
        }
    }

    /// `_mm_mullo_epi32` (wrapping).
    #[inline(always)]
    pub fn mullo_epi32(a: I4, b: I4) -> I4 {
        unsafe {
            #[cfg(target_feature = "sse4.1")]
            {
                oi(_mm_mullo_epi32(i(a), i(b)))
            }
            #[cfg(not(target_feature = "sse4.1"))]
            {
                let (a, b) = (i(a), i(b));
                let even = _mm_mul_epu32(a, b);
                let odd = _mm_mul_epu32(_mm_srli_epi64::<32>(a), _mm_srli_epi64::<32>(b));
                let even = _mm_shuffle_epi32::<0b00_00_10_00>(even);
                let odd = _mm_shuffle_epi32::<0b00_00_10_00>(odd);
                oi(_mm_unpacklo_epi32(even, odd))
            }
        }
    }

    /// `_mm_min_epi32`.
    #[inline(always)]
    pub fn min_epi32(a: I4, b: I4) -> I4 {
        #[cfg(target_feature = "sse4.1")]
        {
            oi(unsafe { _mm_min_epi32(i(a), i(b)) })
        }
        #[cfg(not(target_feature = "sse4.1"))]
        {
            blendv_epi32(a, b, cmpgt_epi32(a, b))
        }
    }

    /// `_mm_max_epi32`.
    #[inline(always)]
    pub fn max_epi32(a: I4, b: I4) -> I4 {
        #[cfg(target_feature = "sse4.1")]
        {
            oi(unsafe { _mm_max_epi32(i(a), i(b)) })
        }
        #[cfg(not(target_feature = "sse4.1"))]
        {
            blendv_epi32(a, b, cmplt_epi32(a, b))
        }
    }

    /// `_mm_sll_epi32` (count > 31 gives 0).
    #[inline(always)]
    pub fn sll_epi32(a: I4, s: u32) -> I4 {
        oi(unsafe { _mm_sll_epi32(i(a), _mm_cvtsi32_si128(s as i32)) })
    }
    /// `_mm_srl_epi32` (count > 31 gives 0).
    #[inline(always)]
    pub fn srl_epi32(a: I4, s: u32) -> I4 {
        oi(unsafe { _mm_srl_epi32(i(a), _mm_cvtsi32_si128(s as i32)) })
    }
    /// `_mm_sra_epi32` (count > 31 fills with the sign).
    #[inline(always)]
    pub fn sra_epi32(a: I4, s: u32) -> I4 {
        oi(unsafe { _mm_sra_epi32(i(a), _mm_cvtsi32_si128(s as i32)) })
    }
}

#[cfg(not(target_arch = "x86_64"))]
mod imp {
    use super::super::vecmath::{cvtt, max_ps as max1, min_ps as min1, round_even};
    use super::{F4, I4, M4};

    #[inline(always)]
    fn b(c: bool) -> u32 {
        (c as u32).wrapping_neg()
    }
    #[inline(always)]
    fn sel(a: u32, b: u32, m: u32) -> u32 {
        let m = ((m as i32) >> 31) as u32;
        (a & !m) | (b & m)
    }
    macro_rules! f2 {
        ($name:ident, |$x:ident, $y:ident| $e:expr) => {
            #[inline(always)]
            pub fn $name(a: F4, c: F4) -> F4 {
                std::array::from_fn(|k| {
                    let ($x, $y) = (a[k], c[k]);
                    $e
                })
            }
        };
    }
    macro_rules! fc {
        ($name:ident, |$x:ident, $y:ident| $e:expr) => {
            #[inline(always)]
            pub fn $name(a: F4, c: F4) -> M4 {
                std::array::from_fn(|k| {
                    let ($x, $y) = (a[k], c[k]);
                    b($e)
                })
            }
        };
    }
    macro_rules! i2 {
        ($name:ident, |$x:ident, $y:ident| $e:expr) => {
            #[inline(always)]
            pub fn $name(a: I4, c: I4) -> I4 {
                std::array::from_fn(|k| {
                    let ($x, $y) = (a[k], c[k]);
                    $e
                })
            }
        };
    }
    macro_rules! ic {
        ($name:ident, |$x:ident, $y:ident| $e:expr) => {
            #[inline(always)]
            pub fn $name(a: I4, c: I4) -> M4 {
                std::array::from_fn(|k| {
                    let ($x, $y) = (a[k], c[k]);
                    b($e)
                })
            }
        };
    }
    f2!(add_ps, |x, y| x + y);
    f2!(sub_ps, |x, y| x - y);
    f2!(mul_ps, |x, y| x * y);
    f2!(div_ps, |x, y| x / y);
    f2!(min_ps, |x, y| min1(x, y));
    f2!(max_ps, |x, y| max1(x, y));
    fc!(cmpeq_ps, |x, y| x == y);
    fc!(cmpneq_ps, |x, y| x != y);
    fc!(cmplt_ps, |x, y| x < y);
    fc!(cmpgt_ps, |x, y| x > y);
    fc!(cmple_ps, |x, y| x <= y);
    fc!(cmpge_ps, |x, y| x >= y);
    i2!(add_epi32, |x, y| x.wrapping_add(y));
    i2!(sub_epi32, |x, y| x.wrapping_sub(y));
    i2!(and_si128, |x, y| x & y);
    i2!(or_si128, |x, y| x | y);
    i2!(xor_si128, |x, y| x ^ y);
    i2!(mullo_epi32, |x, y| x.wrapping_mul(y));
    i2!(min_epi32, |x, y| x.min(y));
    i2!(max_epi32, |x, y| x.max(y));
    ic!(cmpeq_epi32, |x, y| x == y);
    ic!(cmplt_epi32, |x, y| x < y);
    ic!(cmpgt_epi32, |x, y| x > y);
    pub fn sqrt_ps(a: F4) -> F4 {
        a.map(f32::sqrt)
    }
    pub fn round_ps(a: F4) -> F4 {
        a.map(round_even)
    }
    pub fn cvttps_epi32(a: F4) -> I4 {
        a.map(cvtt)
    }
    pub fn cvtepi32_ps(a: I4) -> F4 {
        a.map(|x| x as f32)
    }
    pub fn blendv_ps(a: F4, c: F4, m: M4) -> F4 {
        std::array::from_fn(|k| f32::from_bits(sel(a[k].to_bits(), c[k].to_bits(), m[k])))
    }
    pub fn blendv_epi32(a: I4, c: I4, m: M4) -> I4 {
        std::array::from_fn(|k| sel(a[k] as u32, c[k] as u32, m[k]) as i32)
    }
    pub fn sll_epi32(a: I4, s: u32) -> I4 {
        a.map(|x| if s > 31 { 0 } else { ((x as u32) << s) as i32 })
    }
    pub fn srl_epi32(a: I4, s: u32) -> I4 {
        a.map(|x| if s > 31 { 0 } else { ((x as u32) >> s) as i32 })
    }
    pub fn sra_epi32(a: I4, s: u32) -> I4 {
        a.map(|x| x >> s.min(31))
    }
}

/// The lane kernels for one vector width: `[_; 4]` maps to the SSE kernels; `[_; 8]` to the
/// AVX2 kernels (`astcenc_vecmathlib_avx2_8.h`) when the build enables AVX2, otherwise to two
/// 4-lane halves.
pub trait Lanes: Copy {
    type I: Copy;
    type M: Copy;
    fn add_ps(self, b: Self) -> Self;
    fn sub_ps(self, b: Self) -> Self;
    fn mul_ps(self, b: Self) -> Self;
    fn div_ps(self, b: Self) -> Self;
    fn min_ps(self, b: Self) -> Self;
    fn max_ps(self, b: Self) -> Self;
    fn sqrt_ps(self) -> Self;
    fn round_ps(self) -> Self;
    fn cmpeq_ps(self, b: Self) -> Self::M;
    fn cmpneq_ps(self, b: Self) -> Self::M;
    fn cmplt_ps(self, b: Self) -> Self::M;
    fn cmpgt_ps(self, b: Self) -> Self::M;
    fn cmple_ps(self, b: Self) -> Self::M;
    fn cmpge_ps(self, b: Self) -> Self::M;
    fn blendv_ps(self, b: Self, m: Self::M) -> Self;
    fn cvttps_epi32(self) -> Self::I;
}

/// Integer kernels for one vector width.
pub trait LanesI: Copy {
    type F: Copy;
    type M: Copy;
    fn add_epi32(self, b: Self) -> Self;
    fn sub_epi32(self, b: Self) -> Self;
    fn mullo_epi32(self, b: Self) -> Self;
    fn and_si(self, b: Self) -> Self;
    fn or_si(self, b: Self) -> Self;
    fn xor_si(self, b: Self) -> Self;
    fn min_epi32(self, b: Self) -> Self;
    fn max_epi32(self, b: Self) -> Self;
    fn cmpeq_epi32(self, b: Self) -> Self::M;
    fn cmplt_epi32(self, b: Self) -> Self::M;
    fn cmpgt_epi32(self, b: Self) -> Self::M;
    fn blendv_epi32(self, b: Self, m: Self::M) -> Self;
    fn sll_epi32(self, s: u32) -> Self;
    fn srl_epi32(self, s: u32) -> Self;
    fn sra_epi32(self, s: u32) -> Self;
    fn cvtepi32_ps(self) -> Self::F;
}

macro_rules! lanes4 {
    ($($f:ident($($a:ident),*) -> $r:ty;)*) => {
        $(#[inline(always)] fn $f(self $(, $a: Self)*) -> $r { imp::$f(self $(, $a)*) })*
    };
}

impl Lanes for F4 {
    type I = I4;
    type M = M4;
    lanes4! {
        add_ps(b) -> Self; sub_ps(b) -> Self; mul_ps(b) -> Self; div_ps(b) -> Self;
        min_ps(b) -> Self; max_ps(b) -> Self; sqrt_ps() -> Self; round_ps() -> Self;
        cmpeq_ps(b) -> M4; cmpneq_ps(b) -> M4; cmplt_ps(b) -> M4; cmpgt_ps(b) -> M4;
        cmple_ps(b) -> M4; cmpge_ps(b) -> M4; cvttps_epi32() -> I4;
    }
    #[inline(always)]
    fn blendv_ps(self, b: Self, m: M4) -> Self {
        imp::blendv_ps(self, b, m)
    }
}

impl LanesI for I4 {
    type F = F4;
    type M = M4;
    lanes4! {
        add_epi32(b) -> Self; sub_epi32(b) -> Self; mullo_epi32(b) -> Self;
        min_epi32(b) -> Self; max_epi32(b) -> Self;
        cmpeq_epi32(b) -> M4; cmplt_epi32(b) -> M4; cmpgt_epi32(b) -> M4; cvtepi32_ps() -> F4;
    }
    #[inline(always)]
    fn and_si(self, b: Self) -> Self {
        imp::and_si128(self, b)
    }
    #[inline(always)]
    fn or_si(self, b: Self) -> Self {
        imp::or_si128(self, b)
    }
    #[inline(always)]
    fn xor_si(self, b: Self) -> Self {
        imp::xor_si128(self, b)
    }
    #[inline(always)]
    fn blendv_epi32(self, b: Self, m: M4) -> Self {
        imp::blendv_epi32(self, b, m)
    }
    #[inline(always)]
    fn sll_epi32(self, s: u32) -> Self {
        imp::sll_epi32(self, s)
    }
    #[inline(always)]
    fn srl_epi32(self, s: u32) -> Self {
        imp::srl_epi32(self, s)
    }
    #[inline(always)]
    fn sra_epi32(self, s: u32) -> Self {
        imp::sra_epi32(self, s)
    }
}

#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
mod wide8 {
    //! 8 lanes as two 4-lane halves.
    use super::*;

    #[inline(always)]
    fn split<T: Copy>(a: [T; 8]) -> [[T; 4]; 2] {
        let (c, _) = a.as_chunks::<4>();
        [c[0], c[1]]
    }
    #[inline(always)]
    fn join<T: Copy>(a: [T; 4], b: [T; 4]) -> [T; 8] {
        [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]]
    }
    macro_rules! halves {
        ($($f:ident($($a:ident),*) -> $r:ty;)*) => {
            $(#[inline(always)] fn $f(self $(, $a: Self)*) -> $r {
                let [x0, x1] = split(self);
                $(let $a = split($a);)*
                join(x0.$f($($a[0]),*), x1.$f($($a[1]),*))
            })*
        };
    }

    impl Lanes for [f32; 8] {
        type I = [i32; 8];
        type M = [u32; 8];
        halves! {
            add_ps(b) -> Self; sub_ps(b) -> Self; mul_ps(b) -> Self; div_ps(b) -> Self;
            min_ps(b) -> Self; max_ps(b) -> Self; sqrt_ps() -> Self; round_ps() -> Self;
            cmpeq_ps(b) -> [u32; 8]; cmpneq_ps(b) -> [u32; 8]; cmplt_ps(b) -> [u32; 8];
            cmpgt_ps(b) -> [u32; 8]; cmple_ps(b) -> [u32; 8]; cmpge_ps(b) -> [u32; 8];
            cvttps_epi32() -> [i32; 8];
        }
        #[inline(always)]
        fn blendv_ps(self, b: Self, m: [u32; 8]) -> Self {
            let ([a0, a1], [b0, b1], [m0, m1]) = (split(self), split(b), split(m));
            join(a0.blendv_ps(b0, m0), a1.blendv_ps(b1, m1))
        }
    }

    impl LanesI for [i32; 8] {
        type F = [f32; 8];
        type M = [u32; 8];
        halves! {
            add_epi32(b) -> Self; sub_epi32(b) -> Self; mullo_epi32(b) -> Self;
            and_si(b) -> Self; or_si(b) -> Self; xor_si(b) -> Self;
            min_epi32(b) -> Self; max_epi32(b) -> Self;
            cmpeq_epi32(b) -> [u32; 8]; cmplt_epi32(b) -> [u32; 8]; cmpgt_epi32(b) -> [u32; 8];
            cvtepi32_ps() -> [f32; 8];
        }
        #[inline(always)]
        fn blendv_epi32(self, b: Self, m: [u32; 8]) -> Self {
            let ([a0, a1], [b0, b1], [m0, m1]) = (split(self), split(b), split(m));
            join(a0.blendv_epi32(b0, m0), a1.blendv_epi32(b1, m1))
        }
        #[inline(always)]
        fn sll_epi32(self, s: u32) -> Self {
            let [a0, a1] = split(self);
            join(a0.sll_epi32(s), a1.sll_epi32(s))
        }
        #[inline(always)]
        fn srl_epi32(self, s: u32) -> Self {
            let [a0, a1] = split(self);
            join(a0.srl_epi32(s), a1.srl_epi32(s))
        }
        #[inline(always)]
        fn sra_epi32(self, s: u32) -> Self {
            let [a0, a1] = split(self);
            join(a0.sra_epi32(s), a1.sra_epi32(s))
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
mod wide8 {
    //! `astcenc_vecmathlib_avx2_8.h` kernels (only built when AVX2 is enabled at compile time).
    use super::*;
    use std::arch::x86_64::*;

    type F8 = [f32; 8];
    type I8 = [i32; 8];
    type M8 = [u32; 8];

    #[inline(always)]
    fn f(a: F8) -> __m256 {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn i(a: I8) -> __m256i {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn m(a: M8) -> __m256 {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn of(a: __m256) -> F8 {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn oi(a: __m256i) -> I8 {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn om(a: __m256) -> M8 {
        bytemuck::cast(a)
    }
    #[inline(always)]
    fn omi(a: __m256i) -> M8 {
        bytemuck::cast(a)
    }

    macro_rules! f2 {
        ($($name:ident => $e:expr;)*) => {
            $(#[inline(always)] fn $name(self, b: Self) -> Self {
                let (x, y) = (f(self), f(b));
                of(unsafe { $e(x, y) })
            })*
        };
    }
    macro_rules! fc {
        ($($name:ident => $imm:ident;)*) => {
            $(#[inline(always)] fn $name(self, b: Self) -> M8 {
                om(unsafe { _mm256_cmp_ps::<$imm>(f(self), f(b)) })
            })*
        };
    }
    macro_rules! i2 {
        ($($name:ident => $e:expr;)*) => {
            $(#[inline(always)] fn $name(self, b: Self) -> Self {
                let (x, y) = (i(self), i(b));
                oi(unsafe { $e(x, y) })
            })*
        };
    }

    impl Lanes for F8 {
        type I = I8;
        type M = M8;
        f2! {
            add_ps => _mm256_add_ps; sub_ps => _mm256_sub_ps; mul_ps => _mm256_mul_ps;
            div_ps => _mm256_div_ps; min_ps => _mm256_min_ps; max_ps => _mm256_max_ps;
        }
        fc! {
            cmpeq_ps => _CMP_EQ_OQ; cmpneq_ps => _CMP_NEQ_UQ; cmplt_ps => _CMP_LT_OQ;
            cmpgt_ps => _CMP_GT_OQ; cmple_ps => _CMP_LE_OQ; cmpge_ps => _CMP_GE_OQ;
        }
        #[inline(always)]
        fn sqrt_ps(self) -> Self {
            of(unsafe { _mm256_sqrt_ps(f(self)) })
        }
        #[inline(always)]
        fn round_ps(self) -> Self {
            of(unsafe {
                _mm256_round_ps::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(f(self))
            })
        }
        #[inline(always)]
        fn blendv_ps(self, b: Self, mk: M8) -> Self {
            of(unsafe { _mm256_blendv_ps(f(self), f(b), m(mk)) })
        }
        #[inline(always)]
        fn cvttps_epi32(self) -> I8 {
            oi(unsafe { _mm256_cvttps_epi32(f(self)) })
        }
    }

    impl LanesI for I8 {
        type F = F8;
        type M = M8;
        i2! {
            add_epi32 => _mm256_add_epi32; sub_epi32 => _mm256_sub_epi32;
            mullo_epi32 => _mm256_mullo_epi32; and_si => _mm256_and_si256;
            or_si => _mm256_or_si256; xor_si => _mm256_xor_si256;
            min_epi32 => _mm256_min_epi32; max_epi32 => _mm256_max_epi32;
        }
        #[inline(always)]
        fn cmpeq_epi32(self, b: Self) -> M8 {
            omi(unsafe { _mm256_cmpeq_epi32(i(self), i(b)) })
        }
        #[inline(always)]
        fn cmplt_epi32(self, b: Self) -> M8 {
            omi(unsafe { _mm256_cmpgt_epi32(i(b), i(self)) })
        }
        #[inline(always)]
        fn cmpgt_epi32(self, b: Self) -> M8 {
            omi(unsafe { _mm256_cmpgt_epi32(i(self), i(b)) })
        }
        #[inline(always)]
        fn blendv_epi32(self, b: Self, mk: M8) -> Self {
            oi(unsafe {
                _mm256_castps_si256(_mm256_blendv_ps(
                    _mm256_castsi256_ps(i(self)),
                    _mm256_castsi256_ps(i(b)),
                    m(mk),
                ))
            })
        }
        #[inline(always)]
        fn sll_epi32(self, s: u32) -> Self {
            oi(unsafe { _mm256_sll_epi32(i(self), _mm_cvtsi32_si128(s as i32)) })
        }
        #[inline(always)]
        fn srl_epi32(self, s: u32) -> Self {
            oi(unsafe { _mm256_srl_epi32(i(self), _mm_cvtsi32_si128(s as i32)) })
        }
        #[inline(always)]
        fn sra_epi32(self, s: u32) -> Self {
            oi(unsafe { _mm256_sra_epi32(i(self), _mm_cvtsi32_si128(s as i32)) })
        }
        #[inline(always)]
        fn cvtepi32_ps(self) -> F8 {
            of(unsafe { _mm256_cvtepi32_ps(i(self)) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astcenc::vecmath::{cvtt, max_ps as max1, min_ps as min1, round_even};

    fn sel(a: u32, b: u32, m: u32) -> u32 {
        if m & 0x8000_0000 != 0 { b } else { a }
    }

    fn values() -> Vec<f32> {
        let mut v = vec![
            0.0f32,
            -0.0,
            0.5,
            -0.5,
            1.5,
            2.5,
            -2.5,
            1e10,
            -1e10,
            3e9,
            -3e9,
            8388608.5,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MIN_POSITIVE,
        ];
        let mut x = 0x9e37_79b9u32;
        for k in 0..4000 {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            v.push(if k % 2 == 0 {
                f32::from_bits(x)
            } else {
                (x >> 8) as f32 / 4096.0 - 2048.0
            });
        }
        v
    }

    fn check8(a: [f32; 8], b: [f32; 8], ia: [i32; 8], ib: [i32; 8], m: [u32; 8]) {
        let fb = |x: f32| x.to_bits();
        let same = |x: [f32; 8], y: [f32; 8]| {
            x.iter()
                .zip(&y)
                .all(|(p, q)| fb(*p) == fb(*q) || (p.is_nan() && q.is_nan()))
        };
        let mk = |f: &dyn Fn(f32, f32) -> bool| -> [u32; 8] {
            std::array::from_fn(|k| if f(a[k], b[k]) { u32::MAX } else { 0 })
        };
        let fz =
            |f: &dyn Fn(f32, f32) -> f32| -> [f32; 8] { std::array::from_fn(|k| f(a[k], b[k])) };
        let iz =
            |f: &dyn Fn(i32, i32) -> i32| -> [i32; 8] { std::array::from_fn(|k| f(ia[k], ib[k])) };
        let imk = |f: &dyn Fn(i32, i32) -> bool| -> [u32; 8] {
            std::array::from_fn(|k| if f(ia[k], ib[k]) { u32::MAX } else { 0 })
        };
        assert!(same(a.add_ps(b), fz(&|x, y| x + y)));
        assert!(same(a.sub_ps(b), fz(&|x, y| x - y)));
        assert!(same(a.mul_ps(b), fz(&|x, y| x * y)));
        assert!(same(a.div_ps(b), fz(&|x, y| x / y)));
        assert!(same(a.min_ps(b), fz(&min1)));
        assert!(same(a.max_ps(b), fz(&max1)));
        assert!(same(a.sqrt_ps(), a.map(f32::sqrt)));
        assert!(same(a.round_ps(), a.map(round_even)), "{a:?}");
        assert_eq!(a.cmpeq_ps(b), mk(&|x, y| x == y));
        assert_eq!(a.cmpneq_ps(b), mk(&|x, y| x != y));
        assert_eq!(a.cmplt_ps(b), mk(&|x, y| x < y));
        assert_eq!(a.cmpgt_ps(b), mk(&|x, y| x > y));
        assert_eq!(a.cmple_ps(b), mk(&|x, y| x <= y));
        assert_eq!(a.cmpge_ps(b), mk(&|x, y| x >= y));
        let want: [f32; 8] = std::array::from_fn(|k| f32::from_bits(sel(fb(a[k]), fb(b[k]), m[k])));
        assert!(same(a.blendv_ps(b, m), want));
        assert_eq!(a.cvttps_epi32(), a.map(cvtt));
        assert_eq!(ia.add_epi32(ib), iz(&i32::wrapping_add));
        assert_eq!(ia.sub_epi32(ib), iz(&i32::wrapping_sub));
        assert_eq!(ia.mullo_epi32(ib), iz(&i32::wrapping_mul));
        assert_eq!(ia.and_si(ib), iz(&|x, y| x & y));
        assert_eq!(ia.or_si(ib), iz(&|x, y| x | y));
        assert_eq!(ia.xor_si(ib), iz(&|x, y| x ^ y));
        assert_eq!(ia.min_epi32(ib), iz(&i32::min));
        assert_eq!(ia.max_epi32(ib), iz(&i32::max));
        assert_eq!(ia.cmpeq_epi32(ib), imk(&|x, y| x == y));
        assert_eq!(ia.cmplt_epi32(ib), imk(&|x, y| x < y));
        assert_eq!(ia.cmpgt_epi32(ib), imk(&|x, y| x > y));
        let want: [i32; 8] = std::array::from_fn(|k| sel(ia[k] as u32, ib[k] as u32, m[k]) as i32);
        assert_eq!(ia.blendv_epi32(ib, m), want);
        for s in [0, 1, 7, 31, 32, 40] {
            assert_eq!(
                ia.sll_epi32(s),
                ia.map(|x| if s > 31 { 0 } else { ((x as u32) << s) as i32 })
            );
            assert_eq!(
                ia.srl_epi32(s),
                ia.map(|x| if s > 31 { 0 } else { ((x as u32) >> s) as i32 })
            );
            assert_eq!(ia.sra_epi32(s), ia.map(|x| x >> s.min(31)));
        }
        assert_eq!(ia.cvtepi32_ps(), ia.map(|x| x as f32));
        // The 4-lane kernels on the low half.
        let lo = |x: [f32; 8]| -> [f32; 4] { [x[0], x[1], x[2], x[3]] };
        let (a4, b4) = (lo(a), lo(b));
        assert!(same(
            {
                let r = a4.round_ps();
                [r[0], r[1], r[2], r[3], 0.0, 0.0, 0.0, 0.0]
            },
            {
                let r = a.round_ps();
                [r[0], r[1], r[2], r[3], 0.0, 0.0, 0.0, 0.0]
            }
        ));
        assert_eq!(a4.cmpneq_ps(b4), [0, 1, 2, 3].map(|k| a.cmpneq_ps(b)[k]));
    }

    #[test]
    fn kernels_match_scalar() {
        let v = values();
        let mut x = 7u32;
        let mut next = || {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            x
        };
        for c in 0..v.len() / 2 {
            let a: [f32; 8] = std::array::from_fn(|k| v[(c * 8 + k) % v.len()]);
            let mut b: [f32; 8] = std::array::from_fn(|k| v[(c * 5 + k * 3 + 1) % v.len()]);
            if c % 3 == 0 {
                b[c % 8] = a[c % 8];
            }
            let ia: [i32; 8] = std::array::from_fn(|k| {
                let r = next() as i32;
                if k % 2 == 0 { r } else { r >> 20 }
            });
            let ib: [i32; 8] = std::array::from_fn(|k| {
                if c % 4 == 0 && k == 2 {
                    ia[2]
                } else {
                    next() as i32 >> (k * 3)
                }
            });
            let m: [u32; 8] = std::array::from_fn(|_| next());
            check8(a, b, ia, ib, m);
        }
    }
}
