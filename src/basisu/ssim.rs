// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of `encoder/basisu_ssim.cpp` (`compute_ssim`) and `image_metrics::calc` for 8-bit
//! images from `encoder/basisu_enc.cpp`, with the subset of `image` / `imagef` they use.

#![allow(clippy::needless_range_loop, clippy::excessive_precision)]

/// `basisu::color_rgba`.
pub type ColorRgba = [u8; 4];
/// `basisu::vec4F`.
pub type Vec4F = [f32; 4];

/// Subset of `basisu::image`: 8-bit RGBA pixels, pitch == width.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Image {
    width: u32,
    height: u32,
    pixels: Vec<ColorRgba>,
}

impl Image {
    /// `image(w, h)`: pixels are (0, 0, 0, 255).
    pub fn new(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            pixels: vec![[0, 0, 0, 255]; (width * height) as usize],
        }
    }
    pub fn get_width(&self) -> u32 {
        self.width
    }
    pub fn get_height(&self) -> u32 {
        self.height
    }
    pub fn get(&self, x: u32, y: u32) -> ColorRgba {
        self.pixels
            .get((x + y * self.width) as usize)
            .copied()
            .unwrap_or_default()
    }
    pub fn get_mut(&mut self, x: u32, y: u32) -> Option<&mut ColorRgba> {
        self.pixels.get_mut((x + y * self.width) as usize)
    }
}

/// `color_rgba::get_709_luma`.
fn get_709_luma(c: ColorRgba) -> i32 {
    ((13938u32 * u32::from(c[0]) + 46869 * u32::from(c[1]) + 4729 * u32::from(c[2]) + 32768) >> 16)
        as i32
}

/// `color_rgba::get_601_luma`.
fn get_601_luma(c: ColorRgba) -> i32 {
    ((19595u32 * u32::from(c[0]) + 38470 * u32::from(c[1]) + 7471 * u32::from(c[2]) + 32768) >> 16)
        as i32
}

/// Subset of `basisu::imagef`.
#[derive(Debug, Clone, Default)]
struct ImageF {
    width: u32,
    height: u32,
    pixels: Vec<Vec4F>,
}

impl ImageF {
    fn new(width: u32, height: u32) -> ImageF {
        ImageF {
            width,
            height,
            pixels: vec![[0.0, 0.0, 0.0, 1.0]; (width * height) as usize],
        }
    }
    /// `imagef::set(const image&)`.
    fn from_image(src: &Image) -> ImageF {
        ImageF {
            width: src.width,
            height: src.height,
            pixels: src.pixels.iter().map(|p| p.map(f32::from)).collect(),
        }
    }
    fn at(&self, x: u32, y: u32) -> Vec4F {
        self.pixels
            .get((x + y * self.width) as usize)
            .copied()
            .unwrap_or_default()
    }
    /// `get_clamped_or_wrapped(x, y, false, false)`.
    fn clamped(&self, x: i32, y: i32) -> Vec4F {
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let y = y.clamp(0, self.height as i32 - 1) as u32;
        self.at(x, y)
    }
    /// A same sized image with `f` applied per pixel.
    fn map(&self, f: impl Fn(Vec4F) -> Vec4F) -> ImageF {
        ImageF {
            width: self.width,
            height: self.height,
            pixels: self.pixels.iter().map(|&p| f(p)).collect(),
        }
    }
    /// A same sized image combining two images per pixel.
    fn zip(&self, other: &ImageF, f: impl Fn(Vec4F, Vec4F) -> Vec4F) -> ImageF {
        ImageF {
            width: self.width,
            height: self.height,
            pixels: (0..self.pixels.len())
                .map(|i| {
                    f(
                        self.pixels[i],
                        other.pixels.get(i).copied().unwrap_or_default(),
                    )
                })
                .collect(),
        }
    }
}

/// `gauss`.
fn gauss(x: i32, y: i32, sigma_sqr: f32) -> f32 {
    let pow = (-(((x * x + y * y) as f32) / (2.0 * sigma_sqr))).exp();
    (1.0 / ((2.0 * std::f64::consts::PI * f64::from(sigma_sqr)) as f32).sqrt()) * pow
}

/// `compute_gaussian_kernel` with `cComputeGaussianFlagNormalize` (size_x / size_y odd).
fn compute_gaussian_kernel(dst: &mut [f32], size_x: i32, size_y: i32, sigma_sqr: f32) {
    let mid_x = size_x / 2;
    let mid_y = size_y / 2;
    let idx = |x: i32, y: i32| (x + y * size_x) as usize;

    let mut sum = 0.0f64;
    for x in 0..size_x {
        for y in 0..size_y {
            let g = if x > mid_x && y < mid_y {
                dst[idx(size_x - x - 1, y)]
            } else if x < mid_x && y > mid_y {
                dst[idx(x, size_y - y - 1)]
            } else if x > mid_x && y > mid_y {
                dst[idx(size_x - x - 1, size_y - y - 1)]
            } else {
                gauss(x - mid_x, y - mid_y, sigma_sqr)
            };
            dst[idx(x, y)] = g;
            sum += f64::from(g);
        }
    }

    let one_over_sum = 1.0 / sum;
    for v in dst.iter_mut().take((size_x * size_y) as usize) {
        *v = (f64::from(*v) * one_over_sum) as f32;
    }
}

/// `gaussian_filter(dst, orig_img, odd_filter_width, sigma_sqr)` (clamped, no divisors).
fn gaussian_filter(orig_img: &ImageF, odd_filter_width: u32, sigma_sqr: f32) -> ImageF {
    let w = (odd_filter_width | 1) as i32;
    let mut kernel = vec![0.0f32; (w * w) as usize];
    compute_gaussian_kernel(&mut kernel, w, w, sigma_sqr);

    let dst_width = orig_img.width;
    let dst_height = orig_img.height;
    let h = w / 2;
    let l = -h;

    let mut dst = ImageF::new(dst_width, dst_height);
    for oy in 0..dst_height as i32 {
        for ox in 0..dst_width as i32 {
            let mut c = [0.0f32; 4];
            for yd in l..=h {
                let y = oy + yd;
                for xd in l..=h {
                    let x = ox + xd;
                    let p = orig_img.clamped(x, y);
                    let wt = kernel[((xd + h) + (yd + h) * w) as usize];
                    for k in 0..4 {
                        c[k] += p[k] * wt;
                    }
                }
            }
            if let Some(d) = dst
                .pixels
                .get_mut((ox as u32 + oy as u32 * dst_width) as usize)
            {
                *d = c;
            }
        }
    }
    dst
}

/// `pow_image(src, dst, vec4F(2))`.
fn sqr_image(src: &ImageF) -> ImageF {
    src.map(|p| p.map(|v| v * v))
}

/// `mul_image(src1, src2, dst, vec4F(1))`.
fn mul_image(a: &ImageF, b: &ImageF) -> ImageF {
    a.zip(b, |x, y| std::array::from_fn(|c| x[c] * y[c] * 1.0))
}

/// `add_weighted_image(src1, 1, src2, -1, 0)`.
fn sub_image(a: &ImageF, b: &ImageF) -> ImageF {
    a.zip(b, |x, y| std::array::from_fn(|c| x[c] * 1.0 + -y[c] + 0.0))
}

/// `scale_image(src, dst, scale, 0)` followed by `adds_image(dst, value)`.
fn scale_adds_image(src: &ImageF, scale: f32, value: f32) -> ImageF {
    src.map(|p| p.map(|v| (scale * v + 0.0) + value))
}

/// `add_image` followed by `adds_image(dst, value)`.
fn add_adds_image(a: &ImageF, b: &ImageF, value: f32) -> ImageF {
    a.zip(b, |x, y| std::array::from_fn(|c| (x[c] + y[c]) + value))
}

/// `div_image(src1, src2, dst, vec4F(1))`.
fn div_image(a: &ImageF, b: &ImageF) -> ImageF {
    a.zip(b, |x, y| {
        std::array::from_fn(|c| {
            if y[c] == 0.0 {
                0.0
            } else {
                (x[c] * 1.0) / y[c]
            }
        })
    })
}

/// `avg_image`.
fn avg_image(src: &ImageF) -> Vec4F {
    let mut avg = [0.0f32; 4];
    for y in 0..src.height {
        for x in 0..src.width {
            let s = src.at(x, y);
            for c in 0..4 {
                avg[c] += s[c];
            }
        }
    }
    let n = (src.width * src.height) as f32;
    avg.map(|v| v / n)
}

/// `compute_ssim(const imagef&, const imagef&)`.
fn compute_ssim_f(a: &ImageF, b: &ImageF) -> Vec4F {
    const C1: f32 = 6.50250;
    const C2: f32 = 58.52250;
    let sigma_sqr = 1.5f32 * 1.5;

    let a_sq = sqr_image(a);
    let b_sq = sqr_image(b);
    let axb = mul_image(a, b);
    let mu1 = gaussian_filter(a, 11, sigma_sqr);
    let mu2 = gaussian_filter(b, 11, sigma_sqr);
    let mu1_sq = sqr_image(&mu1);
    let mu2_sq = sqr_image(&mu2);
    let mu1_mu2 = mul_image(&mu1, &mu2);
    let s1_sq = sub_image(&gaussian_filter(&a_sq, 11, sigma_sqr), &mu1_sq);
    let s2_sq = sub_image(&gaussian_filter(&b_sq, 11, sigma_sqr), &mu2_sq);
    let s12 = sub_image(&gaussian_filter(&axb, 11, sigma_sqr), &mu1_mu2);

    let t1 = scale_adds_image(&mu1_mu2, 2.0, C1);
    let t2 = scale_adds_image(&s12, 2.0, C2);
    let t3 = mul_image(&t1, &t2);
    let t1 = add_adds_image(&mu1_sq, &mu2_sq, C1);
    let t2 = add_adds_image(&s1_sq, &s2_sq, C2);
    let t1 = mul_image(&t1, &t2);
    let smap = div_image(&t3, &t1);
    avg_image(&smap)
}

/// `compute_ssim(const image&, const image&, luma, luma_601)`: per channel SSIM.
pub fn compute_ssim(a: &Image, b: &Image, luma: bool, luma_601: bool) -> Vec4F {
    let w = a.width.min(b.width);
    let h = a.height.min(b.height);
    if w == 0 || h == 0 {
        return [0.0; 4];
    }
    // Crop input images to equal dimensions
    let crop = |src: &Image| {
        let mut out = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let mut p = src.get(x, y);
                if luma {
                    let l = if luma_601 {
                        get_601_luma(p)
                    } else {
                        get_709_luma(p)
                    };
                    let l = l.clamp(0, 255) as u8;
                    p = [l, l, l, p[3]];
                }
                if let Some(d) = out.get_mut(x, y) {
                    *d = p;
                }
            }
        }
        out
    };
    compute_ssim_f(&ImageF::from_image(&crop(a)), &ImageF::from_image(&crop(b)))
}

/// Port of `image_metrics` (the 8-bit `calc` fields).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ImageMetrics {
    pub width: u32,
    pub height: u32,
    pub max: f64,
    pub mean: f64,
    pub mean_squared: f64,
    pub rms: f64,
    pub psnr: f64,
    pub sum_a: u64,
    pub sum_b: u64,
}

impl ImageMetrics {
    /// `image_metrics::calc(const image&, const image&, first_chan, total_chans,
    /// avg_comp_error, use_601_luma)`. `total_chans == 0` measures (601 / 709) luma.
    pub fn calc(
        &mut self,
        a: &Image,
        b: &Image,
        first_chan: u32,
        total_chans: u32,
        avg_comp_error: bool,
        use_601_luma: bool,
    ) {
        let width = a.width.min(b.width);
        let height = a.height.min(b.height);
        self.width = width;
        self.height = height;

        let mut hist = [0.0f64; 256];
        self.sum_a = 0;
        self.sum_b = 0;

        for y in 0..height {
            for x in 0..width {
                let ca = a.get(x, y);
                let cb = b.get(x, y);
                if total_chans != 0 {
                    for c in 0..total_chans as usize {
                        let k = first_chan as usize + c;
                        let (va, vb) = (i32::from(ca[k & 3]), i32::from(cb[k & 3]));
                        hist[(va - vb).unsigned_abs() as usize & 255] += 1.0;
                        self.sum_a += va as u64;
                        self.sum_b += vb as u64;
                    }
                } else {
                    let d = if use_601_luma {
                        get_601_luma(ca) - get_601_luma(cb)
                    } else {
                        get_709_luma(ca) - get_709_luma(cb)
                    };
                    hist[d.unsigned_abs() as usize & 255] += 1.0;
                    for c in 0..3 {
                        self.sum_a += u64::from(ca[c]);
                        self.sum_b += u64::from(cb[c]);
                    }
                }
            }
        }

        self.max = 0.0;
        let mut sum = 0.0f64;
        let mut sum2 = 0.0f64;
        for (i, &h) in hist.iter().enumerate() {
            if h != 0.0 {
                self.max = self.max.max(i as f64);
                let v = i as f64 * h;
                sum += v;
                sum2 += i as f64 * v;
            }
        }

        let mut total_values = f64::from(width) * f64::from(height);
        if avg_comp_error {
            total_values *= f64::from(total_chans.clamp(1, 4));
        }

        self.mean = f64::from((sum / total_values).clamp(0.0, 255.0) as f32);
        self.mean_squared = f64::from((sum2 / total_values).clamp(0.0, 255.0 * 255.0) as f32);
        self.rms = f64::from(self.mean_squared.sqrt() as f32);
        self.psnr = if self.rms != 0.0 {
            f64::from(((255.0 / self.rms).log10() * 20.0).clamp(0.0, 100.0) as f32)
        } else {
            100.0
        };
    }
}
