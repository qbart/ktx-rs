//! Port of basisu `encoder/basisu_resample_filters.cpp`.
//!
//! Mixed float/double evaluation follows the C++ promotions exactly (float
//! literals are rounded to `f32` before being widened).

use std::f64::consts::PI;

pub type ResampleFilterFunc = fn(f32) -> f32;

/// Port of `resample_filter`.
#[derive(Debug, Clone, Copy)]
pub struct ResampleFilter {
    pub name: &'static str,
    pub func: ResampleFilterFunc,
    pub support: f32,
}

pub const BASISU_BOX_FILTER_SUPPORT: f32 = 0.5;
pub const BASISU_TENT_FILTER_SUPPORT: f32 = 1.0;
pub const BASISU_GAUSSIAN_FILTER_SUPPORT: f32 = 1.25;
pub const BASISU_BELL_FILTER_SUPPORT: f32 = 1.5;

/// pulse/Fourier window
pub fn box_filter(t: f32) -> f32 {
    // make_clist() calls the filter function with t inverted (pos = left, neg = right)
    if (-0.5..0.5).contains(&t) { 1.0 } else { 0.0 }
}

/// box (*) box, bilinear/triangle
pub fn tent_filter(mut t: f32) -> f32 {
    if t < 0.0 {
        t = -t;
    }
    if t < 1.0 { 1.0 - t } else { 0.0 }
}

/// box (*) box (*) box
pub fn bell_filter(mut t: f32) -> f32 {
    if t < 0.0 {
        t = -t;
    }
    if t < 0.5 {
        return 0.75 - (t * t);
    }
    if t < 1.5 {
        t -= 1.5;
        return 0.5 * (t * t);
    }
    0.0
}

const B_SPLINE_SUPPORT: f32 = 2.0;
/// box (*) box (*) box (*) box
fn b_spline_filter(mut t: f32) -> f32 {
    if t < 0.0 {
        t = -t;
    }
    if t < 1.0 {
        let tt = t * t;
        (0.5 * tt * t) - tt + (2.0f32 / 3.0f32)
    } else if t < 2.0 {
        t = 2.0 - t;
        (1.0f32 / 6.0f32) * (t * t * t)
    } else {
        0.0
    }
}

// Dodgson, N., "Quadratic Interpolation for Image Resampling"
const QUADRATIC_SUPPORT: f32 = 1.5;
fn quadratic(mut t: f32, r: f32) -> f32 {
    if t < 0.0 {
        t = -t;
    }
    if t < QUADRATIC_SUPPORT {
        let tt = t * t;
        if t <= 0.5 {
            (-2.0 * r) * tt + 0.5 * (r + 1.0)
        } else {
            (r * tt) + (-2.0 * r - 0.5) * t + (3.0f32 / 4.0f32) * (r + 1.0)
        }
    } else {
        0.0
    }
}

fn quadratic_interp_filter(t: f32) -> f32 {
    quadratic(t, 1.0)
}

fn quadratic_approx_filter(t: f32) -> f32 {
    quadratic(t, 0.5)
}

fn quadratic_mix_filter(t: f32) -> f32 {
    quadratic(t, 0.8)
}

// Mitchell, D. and A. Netravali, "Reconstruction Filters in Computer Graphics."
fn mitchell(mut t: f32, b: f32, c: f32) -> f32 {
    let tt = t * t;
    if t < 0.0 {
        t = -t;
    }
    if t < 1.0 {
        t = ((12.0 - 9.0 * b - 6.0 * c) * (t * tt))
            + ((-18.0 + 12.0 * b + 6.0 * c) * tt)
            + (6.0 - 2.0 * b);
        t / 6.0
    } else if t < 2.0 {
        t = ((-b - 6.0 * c) * (t * tt))
            + ((6.0 * b + 30.0 * c) * tt)
            + ((-12.0 * b - 48.0 * c) * t)
            + (8.0 * b + 24.0 * c);
        t / 6.0
    } else {
        0.0
    }
}

const MITCHELL_SUPPORT: f32 = 2.0;
fn mitchell_filter(t: f32) -> f32 {
    mitchell(t, 1.0f32 / 3.0f32, 1.0f32 / 3.0f32)
}

const CATMULL_ROM_SUPPORT: f32 = 2.0;
fn catmull_rom_filter(t: f32) -> f32 {
    mitchell(t, 0.0, 0.5)
}

fn sinc(mut x: f64) -> f64 {
    x *= PI;
    if x < 0.01f32 as f64 && x > -0.01f32 as f64 {
        return 1.0 + x * x * ((-1.0f32 / 6.0f32) as f64 + x * x * 1.0 / 120.0);
    }
    x.sin() / x
}

fn clean(t: f64) -> f32 {
    const EPSILON: f32 = 0.0000125;
    if t.abs() < EPSILON as f64 {
        return 0.0;
    }
    t as f32
}

#[allow(clippy::excessive_precision)]
fn blackman_exact_window(x: f64) -> f64 {
    0.42659071f32 as f64
        + 0.49656062f32 as f64 * (PI * x).cos()
        + 0.07684867f32 as f64 * (2.0 * PI * x).cos()
}

const BLACKMAN_SUPPORT: f32 = 3.0;
fn blackman_filter(mut t: f32) -> f32 {
    if t < 0.0 {
        t = -t;
    }
    if t < 3.0 {
        clean(sinc(t as f64) * blackman_exact_window((t / 3.0) as f64))
    } else {
        0.0
    }
}

/// with blackman window
pub fn gaussian_filter(mut t: f32) -> f32 {
    if t < 0.0 {
        t = -t;
    }
    if t < BASISU_GAUSSIAN_FILTER_SUPPORT {
        clean(
            (-2.0 * t * t).exp() as f64
                * (2.0 / PI).sqrt()
                * blackman_exact_window((t / BASISU_GAUSSIAN_FILTER_SUPPORT) as f64),
        )
    } else {
        0.0
    }
}

// Windowed sinc -- see "Jimm Blinn's Corner: Dirty Pixels" pg. 26.
fn lanczos(mut t: f32, support: f32) -> f32 {
    if t < 0.0 {
        t = -t;
    }
    if t < support {
        clean(sinc(t as f64) * sinc((t / support) as f64))
    } else {
        0.0
    }
}

const LANCZOS3_SUPPORT: f32 = 3.0;
fn lanczos3_filter(t: f32) -> f32 {
    lanczos(t, LANCZOS3_SUPPORT)
}

const LANCZOS4_SUPPORT: f32 = 4.0;
fn lanczos4_filter(t: f32) -> f32 {
    lanczos(t, LANCZOS4_SUPPORT)
}

const LANCZOS6_SUPPORT: f32 = 6.0;
fn lanczos6_filter(t: f32) -> f32 {
    lanczos(t, LANCZOS6_SUPPORT)
}

const LANCZOS12_SUPPORT: f32 = 12.0;
fn lanczos12_filter(t: f32) -> f32 {
    lanczos(t, LANCZOS12_SUPPORT)
}

fn bessel0(x: f64) -> f64 {
    const EPSILON_RATIO: f64 = 1E-16;
    let xh = 0.5 * x;
    let mut sum = 1.0;
    let mut pow = 1.0;
    let mut k = 0i32;
    let mut ds = 1.0;
    while ds > sum * EPSILON_RATIO {
        k += 1;
        pow *= xh / k as f64;
        ds = pow * pow;
        sum += ds;
    }
    sum
}

fn kaiser(alpha: f64, half_width: f64, x: f64) -> f64 {
    let ratio = x / half_width;
    bessel0(alpha * (1.0 - ratio * ratio).sqrt()) / bessel0(alpha)
}

const KAISER_SUPPORT: f32 = 3.0;
fn kaiser_filter(mut t: f32) -> f32 {
    if t < 0.0 {
        t = -t;
    }
    if t < KAISER_SUPPORT {
        // db atten
        let att: f32 = 40.0;
        let att = att as f64;
        let alpha = ((0.58417 * (att - 20.96)).ln() * 0.4).exp() + 0.07886 * (att - 20.96);
        let alpha = alpha as f32;
        return clean(sinc(t as f64) * kaiser(alpha as f64, 3.0, t as f64));
    }
    0.0
}

pub const G_RESAMPLE_FILTERS: [ResampleFilter; 16] = [
    ResampleFilter {
        name: "box",
        func: box_filter,
        support: BASISU_BOX_FILTER_SUPPORT,
    },
    ResampleFilter {
        name: "tent",
        func: tent_filter,
        support: BASISU_TENT_FILTER_SUPPORT,
    },
    ResampleFilter {
        name: "bell",
        func: bell_filter,
        support: BASISU_BELL_FILTER_SUPPORT,
    },
    ResampleFilter {
        name: "b-spline",
        func: b_spline_filter,
        support: B_SPLINE_SUPPORT,
    },
    ResampleFilter {
        name: "mitchell",
        func: mitchell_filter,
        support: MITCHELL_SUPPORT,
    },
    ResampleFilter {
        name: "blackman",
        func: blackman_filter,
        support: BLACKMAN_SUPPORT,
    },
    ResampleFilter {
        name: "lanczos3",
        func: lanczos3_filter,
        support: LANCZOS3_SUPPORT,
    },
    ResampleFilter {
        name: "lanczos4",
        func: lanczos4_filter,
        support: LANCZOS4_SUPPORT,
    },
    ResampleFilter {
        name: "lanczos6",
        func: lanczos6_filter,
        support: LANCZOS6_SUPPORT,
    },
    ResampleFilter {
        name: "lanczos12",
        func: lanczos12_filter,
        support: LANCZOS12_SUPPORT,
    },
    ResampleFilter {
        name: "kaiser",
        func: kaiser_filter,
        support: KAISER_SUPPORT,
    },
    ResampleFilter {
        name: "gaussian",
        func: gaussian_filter,
        support: BASISU_GAUSSIAN_FILTER_SUPPORT,
    },
    ResampleFilter {
        name: "catmullrom",
        func: catmull_rom_filter,
        support: CATMULL_ROM_SUPPORT,
    },
    ResampleFilter {
        name: "quadratic_interp",
        func: quadratic_interp_filter,
        support: QUADRATIC_SUPPORT,
    },
    ResampleFilter {
        name: "quadratic_approx",
        func: quadratic_approx_filter,
        support: QUADRATIC_SUPPORT,
    },
    ResampleFilter {
        name: "quadratic_mix",
        func: quadratic_mix_filter,
        support: QUADRATIC_SUPPORT,
    },
];

pub const G_NUM_RESAMPLE_FILTERS: i32 = G_RESAMPLE_FILTERS.len() as i32;

/// Port of `find_resample_filter`; -1 when not found.
pub fn find_resample_filter(name: &str) -> i32 {
    G_RESAMPLE_FILTERS
        .iter()
        .position(|f| f.name == name)
        .map_or(-1, |i| i as i32)
}
