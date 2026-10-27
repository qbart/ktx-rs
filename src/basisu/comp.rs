// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of `encoder/basisu_comp.cpp` (`basis_compressor`) and the parts of `basisu_enc.h`
//! `image` / `imagef` it needs: ETC1S (frontend + backend), UASTC LDR 4x4 (+ RDO), UASTC HDR
//! 4x4 and ASTC HDR 6x6 / UASTC HDR 6x6 intermediate encoding of caller supplied images into a
//! `.basis` file, as driven by libktx `ktxTexture2_CompressBasisEx`.
//!
//! Not ported (unreachable from libktx, which never reads files, writes outputs, generates
//! mipmaps or asks for statistics): source file / DDS / alpha file loading, resampling, y flip,
//! normal map renormalization, user mipmap chains, `m_compute_stats` / `m_validate_output_data`
//! transcode checks, KTX2 output, debug images, OpenCL, `m_status_output` prints. The XUASTC /
//! ASTC LDR modes (never selected by libktx) return `FailedEncodeUastc`.

use super::BasisTexFormat;
use super::astc_hdr_6x6_enc::{AstcHdr6x6GlobalConfig, compress_photo};
use super::astc_hdr_enc::{
    ASTC_HDR_MAX_VAL, MAX_HALF_FLOAT, compute_block_error, float_to_half_non_neg_no_nan_inf,
};
use super::backend::{
    BasisuBackend, BasisuBackendOutput, BasisuBackendParams, BasisuBackendSliceDesc,
};
use super::basis_file::BasisuFile;
use super::bc6h::astc_hdr_transcode_to_bc6h;
use super::enc::{ColorRgba, crc16};
use super::frontend::{
    BASISU_DEFAULT_ETC1S_COMPRESSION_LEVEL, BASISU_MAX_ETC1S_COMPRESSION_LEVEL, BasisuFrontend,
    BasisuFrontendParams, C_BASIS_TEX_TYPE_2D, C_BASIS_TEX_TYPE_CUBEMAP_ARRAY,
    C_BASIS_TEX_TYPE_VIDEO_FRAMES, C_MAX_ENDPOINT_CLUSTERS, C_MAX_SELECTOR_CLUSTERS, PixelBlock,
};
use super::gpu_texture::unpack_bc6h;
use super::job_pool::for_each_chunk_mut;
use super::uastc_enc::{
    C_PACK_UASTC_FAVOR_SIMPLER_MODES, C_PACK_UASTC_LEVEL_DEFAULT,
    UASTC_RDO_DEFAULT_MAX_ALLOWED_RMS_INCREASE_RATIO, UASTC_RDO_DEFAULT_MAX_SMOOTH_BLOCK_STD_DEV,
    UASTC_RDO_DEFAULT_SKIP_BLOCK_RMS_THRESH, UASTC_RDO_DEFAULT_SMOOTH_BLOCK_MAX_ERROR_SCALE,
    UastcRdoParams, encode_uastc, uastc_rdo,
};
use super::uastc_hdr_4x4_enc::{
    UastcHdr4x4CodecOptions, astc_hdr_4x4_enc_block, astc_hdr_4x4_pack_results_to_block,
};

pub const BASISU_MAX_SUPPORTED_TEXTURE_DIMENSION: u32 = 16384;
pub const BASISU_DEFAULT_ENDPOINT_RDO_THRESH: f32 = 1.5;
pub const BASISU_DEFAULT_SELECTOR_RDO_THRESH: f32 = 1.25;
pub const BASISU_MAX_SLICES: u32 = 0xFF_FFFF;
pub const BASISU_RDO_UASTC_DICT_SIZE_DEFAULT: i32 = 4096;
pub const BASISU_RDO_UASTC_DICT_SIZE_MIN: i32 = 64;
pub const BASISU_RDO_UASTC_DICT_SIZE_MAX: i32 = 65536;

/// `g_black_color`.
const G_BLACK_COLOR: ColorRgba = [0, 0, 0, 255];

/// `basisu::image`: an RGBA8 image with a pitch (in pixels).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    pitch: u32,
    pixels: Vec<ColorRgba>,
}

impl Image {
    pub fn new() -> Self {
        Self::default()
    }

    /// `image(w, h)`.
    pub fn with_size(w: u32, h: u32) -> Self {
        let mut img = Self::default();
        img.resize(w, h);
        img
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn get_width(&self) -> u32 {
        self.width
    }

    pub fn get_height(&self) -> u32 {
        self.height
    }

    pub fn get_pitch(&self) -> u32 {
        self.pitch
    }

    pub fn get_total_pixels(&self) -> u32 {
        self.width * self.height
    }

    pub fn get_block_width(&self, w: u32) -> u32 {
        self.width.div_ceil(w)
    }

    pub fn get_block_height(&self, h: u32) -> u32 {
        self.height.div_ceil(h)
    }

    /// `get_ptr()`: all `pitch * height` pixels.
    pub fn get_pixels(&self) -> &[ColorRgba] {
        &self.pixels
    }

    pub fn get_pixels_mut(&mut self) -> &mut [ColorRgba] {
        &mut self.pixels
    }

    /// `resize(w, h)` with the black background.
    pub fn resize(&mut self, w: u32, h: u32) -> &mut Self {
        self.crop(w, h, u32::MAX, G_BLACK_COLOR)
    }

    /// `crop(w, h, p, background)` (with `init_image`).
    pub fn crop(&mut self, w: u32, h: u32, p: u32, background: ColorRgba) -> &mut Self {
        let p = if p == u32::MAX { w } else { p };
        if w == self.width && h == self.height && p == self.pitch {
            return self;
        }
        if w == 0 || h == 0 || p == 0 {
            self.clear();
            return self;
        }
        let cur_state = std::mem::take(&mut self.pixels);
        self.pixels = vec![background; (p * h) as usize];
        if self.width != 0 || self.height != 0 {
            for y in 0..h {
                for x in 0..w.min(self.width) {
                    if y < self.height {
                        self.pixels[(x + y * p) as usize] =
                            cur_state[(x + y * self.pitch) as usize];
                    }
                }
            }
        }
        self.width = w;
        self.height = h;
        self.pitch = p;
        self
    }

    /// `operator()(x, y)`.
    pub fn get(&self, x: u32, y: u32) -> ColorRgba {
        self.pixels[(x + y * self.pitch) as usize]
    }

    pub fn get_mut(&mut self, x: u32, y: u32) -> &mut ColorRgba {
        &mut self.pixels[(x + y * self.pitch) as usize]
    }

    /// `get_clamped(x, y)`.
    pub fn get_clamped(&self, x: i32, y: i32) -> ColorRgba {
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let y = y.clamp(0, self.height as i32 - 1) as u32;
        self.get(x, y)
    }

    /// `set_clipped(x, y, c)`.
    pub fn set_clipped(&mut self, x: i32, y: i32, c: ColorRgba) -> &mut Self {
        if (x as u32) < self.width && (y as u32) < self.height {
            *self.get_mut(x as u32, y as u32) = c;
        }
        self
    }

    /// `crop_dup_borders(w, h)`: enlarges the image, duplicating the edge pixels.
    pub fn crop_dup_borders(&mut self, w: u32, h: u32) -> &mut Self {
        let (orig_w, orig_h) = (self.width, self.height);
        self.resize(w, h);
        if orig_w != 0 && orig_h != 0 {
            if self.width > orig_w {
                for x in orig_w..self.width {
                    for y in 0..self.height {
                        let c =
                            self.get_clamped(x.min(orig_w - 1) as i32, y.min(orig_h - 1) as i32);
                        self.set_clipped(x as i32, y as i32, c);
                    }
                }
            }
            if self.height > orig_h {
                for y in orig_h..self.height {
                    for x in 0..self.width {
                        let c =
                            self.get_clamped(x.min(orig_w - 1) as i32, y.min(orig_h - 1) as i32);
                        self.set_clipped(x as i32, y as i32, c);
                    }
                }
            }
        }
        self
    }

    /// `extract_block_clamped(pDst, src_x, src_y, w, h)`.
    pub fn extract_block_clamped(
        &self,
        dst: &mut [ColorRgba],
        src_x: u32,
        src_y: u32,
        w: u32,
        h: u32,
    ) {
        for y in 0..h {
            for x in 0..w {
                dst[(x + y * w) as usize] =
                    self.get_clamped((src_x + x) as i32, (src_y + y) as i32);
            }
        }
    }

    /// `has_alpha(channel)`.
    pub fn has_alpha(&self, channel: usize) -> bool {
        (0..self.height).any(|y| (0..self.width).any(|x| self.get(x, y)[channel] < 255))
    }

    /// `set_alpha(a)`.
    pub fn set_alpha(&mut self, a: u8) -> &mut Self {
        for y in 0..self.height {
            for x in 0..self.width {
                self.get_mut(x, y)[3] = a;
            }
        }
        self
    }
}

/// `basisu::imagef`: an RGBA float image with a pitch (in pixels).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageF {
    width: u32,
    height: u32,
    pitch: u32,
    pixels: Vec<[f32; 4]>,
}

impl ImageF {
    pub fn new() -> Self {
        Self::default()
    }

    /// `imagef(w, h)`.
    pub fn with_size(w: u32, h: u32) -> Self {
        let mut img = Self::default();
        img.resize(w, h);
        img
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn get_width(&self) -> u32 {
        self.width
    }

    pub fn get_height(&self) -> u32 {
        self.height
    }

    pub fn get_block_width(&self, w: u32) -> u32 {
        self.width.div_ceil(w)
    }

    pub fn get_block_height(&self, h: u32) -> u32 {
        self.height.div_ceil(h)
    }

    pub fn get_pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }

    pub fn get_pixels_mut(&mut self) -> &mut [[f32; 4]] {
        &mut self.pixels
    }

    /// `resize(w, h)` (background (0, 0, 0, 1)).
    pub fn resize(&mut self, w: u32, h: u32) -> &mut Self {
        self.crop(w, h, u32::MAX, [0.0, 0.0, 0.0, 1.0])
    }

    /// `crop(w, h, p, background)`.
    pub fn crop(&mut self, w: u32, h: u32, p: u32, background: [f32; 4]) -> &mut Self {
        let p = if p == u32::MAX { w } else { p };
        if w == self.width && h == self.height && p == self.pitch {
            return self;
        }
        if w == 0 || h == 0 || p == 0 {
            self.clear();
            return self;
        }
        let cur_state = std::mem::take(&mut self.pixels);
        self.pixels = vec![background; (p * h) as usize];
        for y in 0..h.min(self.height) {
            for x in 0..w.min(self.width) {
                self.pixels[(x + y * p) as usize] = cur_state[(x + y * self.pitch) as usize];
            }
        }
        self.width = w;
        self.height = h;
        self.pitch = p;
        self
    }

    pub fn get(&self, x: u32, y: u32) -> [f32; 4] {
        self.pixels[(x + y * self.pitch) as usize]
    }

    pub fn get_mut(&mut self, x: u32, y: u32) -> &mut [f32; 4] {
        &mut self.pixels[(x + y * self.pitch) as usize]
    }

    pub fn get_clamped(&self, x: i32, y: i32) -> [f32; 4] {
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let y = y.clamp(0, self.height as i32 - 1) as u32;
        self.get(x, y)
    }

    pub fn set_clipped(&mut self, x: i32, y: i32, c: [f32; 4]) -> &mut Self {
        if (x as u32) < self.width && (y as u32) < self.height {
            *self.get_mut(x as u32, y as u32) = c;
        }
        self
    }

    /// `crop_dup_borders(w, h)`.
    pub fn crop_dup_borders(&mut self, w: u32, h: u32) -> &mut Self {
        let (orig_w, orig_h) = (self.width, self.height);
        self.resize(w, h);
        if orig_w != 0 && orig_h != 0 {
            if self.width > orig_w {
                for x in orig_w..self.width {
                    for y in 0..self.height {
                        let c =
                            self.get_clamped(x.min(orig_w - 1) as i32, y.min(orig_h - 1) as i32);
                        self.set_clipped(x as i32, y as i32, c);
                    }
                }
            }
            if self.height > orig_h {
                for y in orig_h..self.height {
                    for x in 0..self.width {
                        let c =
                            self.get_clamped(x.min(orig_w - 1) as i32, y.min(orig_h - 1) as i32);
                        self.set_clipped(x as i32, y as i32, c);
                    }
                }
            }
        }
        self
    }

    /// `extract_block_clamped(pDst, src_x, src_y, w, h)`.
    pub fn extract_block_clamped(
        &self,
        dst: &mut [[f32; 4]],
        src_x: u32,
        src_y: u32,
        w: u32,
        h: u32,
    ) {
        for y in 0..h {
            for x in 0..w {
                dst[(x + y * w) as usize] =
                    self.get_clamped((src_x + x) as i32, (src_y + y) as i32);
            }
        }
    }

    /// `clean_astc_hdr_pixels`: zeroes NaN / Inf / -0 / negative components and clamps to
    /// `highest_mag`, printing a warning per kind like the C++. Returns false if anything
    /// changed.
    pub fn clean_astc_hdr_pixels(&mut self, highest_mag: f32) -> bool {
        let mut status = true;
        let (mut nan_msg, mut inf_msg, mut neg_zero_msg, mut neg_msg, mut clamp_msg) =
            (false, false, false, false, false);
        for iy in 0..self.height {
            for ix in 0..self.width {
                for p in self.get_mut(ix, iy).iter_mut() {
                    if p.is_nan() || p.is_infinite() || p.to_bits() == 0x8000_0000 {
                        if p.is_nan() && !nan_msg {
                            eprintln!("One or more input pixels was NaN, setting to 0.");
                            nan_msg = true;
                        }
                        if p.is_infinite() && !inf_msg {
                            eprintln!("One or more input pixels was INF, setting to 0.");
                            inf_msg = true;
                        }
                        if p.to_bits() == 0x8000_0000 && !neg_zero_msg {
                            eprintln!("One or more input pixels was -0, setting them to 0.");
                            neg_zero_msg = true;
                        }
                        *p = 0.0;
                        status = false;
                    } else {
                        if *p < 0.0 {
                            *p = 0.0;
                            if !neg_msg {
                                eprintln!(
                                    "One or more input pixels was negative -- setting these pixel components to 0 because ASTC HDR doesn't support signed values."
                                );
                                neg_msg = true;
                            }
                            status = false;
                        }
                        if *p > highest_mag {
                            *p = highest_mag;
                            if !clamp_msg {
                                eprintln!(
                                    "One or more input pixels had to be clamped to {highest_mag:.6}."
                                );
                                clamp_msg = true;
                            }
                            status = false;
                        }
                    }
                }
            }
        }
        status
    }
}

/// `param<T>`: a value clamped to `[min, max]` on assignment that remembers whether it was set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Param<T: Copy + PartialOrd> {
    pub value: T,
    pub def: T,
    pub min: T,
    pub max: T,
    pub changed: bool,
}

impl<T: Copy + PartialOrd> Param<T> {
    pub const fn new(def: T, min: T, max: T) -> Self {
        Param {
            value: def,
            def,
            min,
            max,
            changed: false,
        }
    }

    pub fn clear(&mut self) {
        self.value = self.def;
        self.changed = false;
    }

    pub fn get(&self) -> T {
        self.value
    }

    /// `operator=`: clamps and marks the value changed.
    pub fn set(&mut self, v: T) {
        self.value = if v < self.min {
            self.min
        } else if self.max < v {
            self.max
        } else {
            v
        };
        self.changed = true;
    }

    pub fn was_changed(&self) -> bool {
        self.changed
    }
}

impl Param<f32> {
    /// `operator*=`: not clamped, marks the value changed.
    pub fn mul_assign(&mut self, v: f32) {
        self.value *= v;
        self.changed = true;
    }
}

/// `hdr_modes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdrModes {
    UastcHdr4x4,
    AstcHdr6x6,
    UastcHdr6x6Intermediate,
}

/// `basis_compressor_params` (LDR subset; file / mipmap / stats / KTX2 options are not ported).
#[derive(Debug, Clone, PartialEq)]
pub struct BasisCompressorParams {
    pub uastc: bool,
    pub hdr: bool,
    pub hdr_mode: HdrModes,
    pub status_output: bool,
    pub source_images: Vec<Image>,
    pub source_images_hdr: Vec<ImageF>,
    pub y_flip: bool,
    pub debug: bool,
    pub validate_etc1s: bool,
    pub debug_images: bool,
    pub etc1s_compression_level: Param<i32>,
    pub perceptual: bool,
    pub no_selector_rdo: bool,
    pub selector_rdo_thresh: Param<f32>,
    pub no_endpoint_rdo: bool,
    pub endpoint_rdo_thresh: Param<f32>,
    pub read_source_images: bool,
    pub write_output_basis_or_ktx2_files: bool,
    pub compute_stats: bool,
    pub check_for_alpha: bool,
    pub force_alpha: bool,
    pub multithreading: bool,
    pub swizzle: [u8; 4],
    pub disable_hierarchical_endpoint_codebooks: bool,
    pub mip_gen: bool,
    pub mip_srgb: bool,
    pub etc1s_max_endpoint_clusters: u32,
    pub etc1s_max_selector_clusters: u32,
    pub quality_level: i32,
    pub tex_type: u32,
    pub userdata0: u32,
    pub userdata1: u32,
    pub us_per_frame: u32,
    pub pack_uastc_ldr_4x4_flags: u32,
    pub rdo_uastc_ldr_4x4: bool,
    pub rdo_uastc_ldr_4x4_quality_scalar: Param<f32>,
    pub rdo_uastc_ldr_4x4_dict_size: Param<i32>,
    pub rdo_uastc_ldr_4x4_max_smooth_block_error_scale: Param<f32>,
    pub rdo_uastc_ldr_4x4_smooth_block_max_std_dev: Param<f32>,
    pub rdo_uastc_ldr_4x4_max_allowed_rms_increase_ratio: Param<f32>,
    pub rdo_uastc_ldr_4x4_skip_block_rms_thresh: Param<f32>,
    pub rdo_uastc_ldr_4x4_favor_simpler_modes_in_rdo_mode: bool,
    pub rdo_uastc_ldr_4x4_multithreading: bool,
    pub create_ktx2_file: bool,
    pub ktx2_and_basis_srgb_transfer_function: bool,
    pub validate_output_data: bool,
    pub hdr_favor_astc: bool,
    pub uastc_hdr_4x4_options: UastcHdr4x4CodecOptions,
    pub astc_hdr_6x6_options: AstcHdr6x6GlobalConfig,
    /// `m_pJob_pool->get_total_threads()`; 0 = no job pool (`init` fails like C++).
    pub job_pool_threads: u32,
}

impl Default for BasisCompressorParams {
    fn default() -> Self {
        BasisCompressorParams {
            uastc: false,
            hdr: false,
            hdr_mode: HdrModes::UastcHdr4x4,
            status_output: true,
            source_images: Vec::new(),
            source_images_hdr: Vec::new(),
            y_flip: false,
            debug: false,
            validate_etc1s: false,
            debug_images: false,
            etc1s_compression_level: Param::new(
                BASISU_DEFAULT_ETC1S_COMPRESSION_LEVEL as i32,
                0,
                BASISU_MAX_ETC1S_COMPRESSION_LEVEL as i32,
            ),
            perceptual: true,
            no_selector_rdo: false,
            selector_rdo_thresh: Param::new(BASISU_DEFAULT_SELECTOR_RDO_THRESH, 0.0, 1e10),
            no_endpoint_rdo: false,
            endpoint_rdo_thresh: Param::new(BASISU_DEFAULT_ENDPOINT_RDO_THRESH, 0.0, 1e10),
            read_source_images: false,
            write_output_basis_or_ktx2_files: false,
            compute_stats: false,
            check_for_alpha: true,
            force_alpha: false,
            multithreading: true,
            swizzle: [0, 1, 2, 3],
            disable_hierarchical_endpoint_codebooks: false,
            mip_gen: false,
            mip_srgb: false,
            etc1s_max_endpoint_clusters: 0,
            etc1s_max_selector_clusters: 0,
            quality_level: -1,
            tex_type: C_BASIS_TEX_TYPE_2D,
            userdata0: 0,
            userdata1: 0,
            us_per_frame: 0,
            pack_uastc_ldr_4x4_flags: C_PACK_UASTC_LEVEL_DEFAULT,
            rdo_uastc_ldr_4x4: false,
            rdo_uastc_ldr_4x4_quality_scalar: Param::new(1.0, 0.001, 50.0),
            rdo_uastc_ldr_4x4_dict_size: Param::new(
                BASISU_RDO_UASTC_DICT_SIZE_DEFAULT,
                BASISU_RDO_UASTC_DICT_SIZE_MIN,
                BASISU_RDO_UASTC_DICT_SIZE_MAX,
            ),
            rdo_uastc_ldr_4x4_max_smooth_block_error_scale: Param::new(
                UASTC_RDO_DEFAULT_SMOOTH_BLOCK_MAX_ERROR_SCALE,
                1.0,
                300.0,
            ),
            rdo_uastc_ldr_4x4_smooth_block_max_std_dev: Param::new(
                UASTC_RDO_DEFAULT_MAX_SMOOTH_BLOCK_STD_DEV,
                0.01,
                65536.0,
            ),
            rdo_uastc_ldr_4x4_max_allowed_rms_increase_ratio: Param::new(
                UASTC_RDO_DEFAULT_MAX_ALLOWED_RMS_INCREASE_RATIO,
                0.01,
                100.0,
            ),
            rdo_uastc_ldr_4x4_skip_block_rms_thresh: Param::new(
                UASTC_RDO_DEFAULT_SKIP_BLOCK_RMS_THRESH,
                0.01,
                100.0,
            ),
            rdo_uastc_ldr_4x4_favor_simpler_modes_in_rdo_mode: true,
            rdo_uastc_ldr_4x4_multithreading: true,
            create_ktx2_file: false,
            ktx2_and_basis_srgb_transfer_function: true,
            validate_output_data: false,
            hdr_favor_astc: false,
            uastc_hdr_4x4_options: UastcHdr4x4CodecOptions::default(),
            astc_hdr_6x6_options: AstcHdr6x6GlobalConfig::default(),
            job_pool_threads: 0,
        }
    }
}

impl BasisCompressorParams {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// `set_srgb_options`.
    pub fn set_srgb_options(&mut self, srgb_flag: bool) {
        self.perceptual = srgb_flag;
        self.mip_srgb = srgb_flag;
        self.ktx2_and_basis_srgb_transfer_function = srgb_flag;
    }
}

/// `basis_compressor::error_code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasisCompressorErrorCode {
    Success = 0,
    FailedInitializing,
    FailedReadingSourceImages,
    FailedValidating,
    FailedEncodeUastc,
    FailedFrontEnd,
    FailedFrontendExtract,
    FailedBackend,
    FailedCreateBasisFile,
    FailedWritingOutput,
    FailedUastcRdoPostProcess,
    FailedCreateKtx2File,
    FailedInvalidParameters,
}

/// `basis_compressor` (LDR paths).
#[derive(Default)]
pub struct BasisCompressor {
    params: BasisCompressorParams,
    fmt_mode_block_width: u32,
    fmt_mode_block_height: u32,
    slice_descs: Vec<BasisuBackendSliceDesc>,
    slice_images: Vec<Image>,
    slice_images_hdr: Vec<ImageF>,
    total_blocks: u32,
    hdr_image_scale: f32,
    any_source_image_has_alpha: bool,
    source_blocks: Vec<PixelBlock>,
    source_blocks_hdr: Vec<[[f32; 4]; 16]>,
    frontend: BasisuFrontend,
    backend: BasisuBackend,
    uastc_backend_output: BasisuBackendOutput,
    basis_file: BasisuFile,
    output_basis_file: Vec<u8>,
    total_slice_orig_texels: u32,
    basis_file_size: u32,
    basis_bits_per_texel: f32,
}

use BasisCompressorErrorCode as Ec;

impl BasisCompressor {
    pub fn new() -> Self {
        BasisCompressor {
            fmt_mode_block_width: 4,
            fmt_mode_block_height: 4,
            ..Default::default()
        }
    }

    /// The output `.basis` file; valid if `process` succeeded.
    pub fn get_output_basis_file(&self) -> &[u8] {
        &self.output_basis_file
    }

    pub fn get_params(&self) -> &BasisCompressorParams {
        &self.params
    }

    pub fn get_basis_file_size(&self) -> u32 {
        self.basis_file_size
    }

    pub fn get_basis_bits_per_texel(&self) -> f32 {
        self.basis_bits_per_texel
    }

    /// `get_hdr_image_scale`: the factor the HDR source was scaled down by (1 if not).
    pub fn get_hdr_image_scale(&self) -> f32 {
        self.hdr_image_scale
    }

    pub fn get_any_source_image_has_alpha(&self) -> bool {
        self.any_source_image_has_alpha
    }

    fn get_block_width(&self) -> u32 {
        self.fmt_mode_block_width
    }

    fn get_block_height(&self) -> u32 {
        self.fmt_mode_block_height
    }

    /// `check_for_hdr_inputs` (no source filenames in this port).
    fn check_for_hdr_inputs(&mut self) {
        if self.params.source_images.is_empty() && !self.params.source_images_hdr.is_empty() {
            // Assume they want UASTC HDR if they've specified any HDR source images.
            self.params.hdr = true;
        }
        if self.params.hdr {
            self.params.uastc = true;
        }
    }

    /// `sanity_check_input_params`.
    fn sanity_check_input_params(&self) -> bool {
        // Reading source files is not supported, so images must be supplied, LDR or HDR.
        let p = &self.params;
        if p.read_source_images {
            return false;
        }
        if p.source_images.is_empty() {
            !p.source_images_hdr.is_empty()
        } else {
            p.source_images_hdr.is_empty()
        }
    }

    /// `basis_compressor::init`.
    pub fn init(&mut self, params: &BasisCompressorParams) -> bool {
        if params.job_pool_threads == 0 {
            return false;
        }
        self.params = params.clone();
        if self.params.compute_stats && !self.params.validate_output_data {
            self.params.validate_output_data = true;
        }
        self.total_slice_orig_texels = 0;
        self.basis_file_size = 0;
        self.basis_bits_per_texel = 0.0;
        self.hdr_image_scale = 1.0;
        self.check_for_hdr_inputs();
        if self.params.hdr {
            self.params.ktx2_and_basis_srgb_transfer_function = false;
        }
        self.sanity_check_input_params()
    }

    /// `pick_format_mode`.
    fn pick_format_mode(&mut self) -> bool {
        self.fmt_mode_block_width = 4;
        self.fmt_mode_block_height = 4;
        if self.params.hdr && self.params.hdr_mode != HdrModes::UastcHdr4x4 {
            self.fmt_mode_block_width = 6;
            self.fmt_mode_block_height = 6;
        }
        true
    }

    /// `basis_compressor::process`.
    pub fn process(&mut self) -> BasisCompressorErrorCode {
        if !self.pick_format_mode() {
            return Ec::FailedInvalidParameters;
        }
        if !self.read_source_images() {
            return Ec::FailedReadingSourceImages;
        }
        if !self.validate_texture_type_constraints() {
            return Ec::FailedValidating;
        }
        if self.params.create_ktx2_file {
            // KTX2 output is not ported (libktx writes the KTX2 file itself).
            return Ec::FailedCreateKtx2File;
        }
        if !self.extract_source_blocks() {
            return Ec::FailedFrontEnd;
        }
        if self.params.hdr {
            let ec = if self.params.hdr_mode == HdrModes::UastcHdr4x4 {
                self.encode_slices_to_uastc_4x4_hdr()
            } else {
                self.encode_slices_to_astc_6x6_hdr()
            };
            if ec != Ec::Success {
                return ec;
            }
        } else if self.params.uastc {
            let ec = self.encode_slices_to_uastc_4x4_ldr();
            if ec != Ec::Success {
                return ec;
            }
        } else {
            if !self.process_frontend() {
                return Ec::FailedFrontEnd;
            }
            if !self.process_backend() {
                return Ec::FailedBackend;
            }
        }
        if !self.create_basis_file_and_transcode() {
            return Ec::FailedCreateBasisFile;
        }
        if self.params.write_output_basis_or_ktx2_files {
            return Ec::FailedWritingOutput;
        }
        Ec::Success
    }

    /// `encode_slices_to_uastc_4x4_ldr`. Blocks are encoded in jobs of 256 on the job pool.
    fn encode_slices_to_uastc_4x4_ldr(&mut self) -> BasisCompressorErrorCode {
        let out = &mut self.uastc_backend_output;
        out.tex_format = BasisTexFormat::UastcLdr4x4;
        out.etc1s = false;
        out.slice_desc = self.slice_descs.clone();
        out.slice_image_data = vec![Vec::new(); self.slice_descs.len()];
        out.slice_image_crcs = vec![0; self.slice_descs.len()];

        let mut uastc_flags = self.params.pack_uastc_ldr_4x4_flags;
        if self.params.rdo_uastc_ldr_4x4
            && self
                .params
                .rdo_uastc_ldr_4x4_favor_simpler_modes_in_rdo_mode
        {
            uastc_flags |= C_PACK_UASTC_FAVOR_SIMPLER_MODES;
        }

        for (slice_index, slice_desc) in self.slice_descs.iter().enumerate() {
            // gpu_image::init(cUASTC4x4, orig_width, orig_height)
            let num_blocks_x = slice_desc.orig_width.div_ceil(4);
            let num_blocks_y = slice_desc.orig_height.div_ceil(4);
            let total_blocks = num_blocks_x * num_blocks_y;
            let source_image = &self.slice_images[slice_index];

            const N: usize = 256;
            let mut tex = vec![[0u8; 16]; total_blocks as usize];
            let ok = for_each_chunk_mut(
                self.params.job_pool_threads,
                &mut tex,
                N,
                |first_index, dest_blocks| {
                    for (k, dest_block) in dest_blocks.iter_mut().enumerate() {
                        let block_index = (first_index + k) as u32;
                        let block_x = block_index % num_blocks_x;
                        let block_y = block_index / num_blocks_x;
                        let mut block_pixels = [[0u8; 4]; 16];
                        source_image.extract_block_clamped(
                            &mut block_pixels,
                            block_x * 4,
                            block_y * 4,
                            4,
                            4,
                        );
                        let mut rgba = [0u8; 64];
                        rgba.copy_from_slice(block_pixels.as_flattened());
                        encode_uastc(&rgba, dest_block, uastc_flags);
                    }
                },
            );
            if !ok {
                return Ec::FailedEncodeUastc;
            }

            if self.params.rdo_uastc_ldr_4x4 {
                let rdo_params = UastcRdoParams {
                    lambda: self.params.rdo_uastc_ldr_4x4_quality_scalar.get(),
                    max_allowed_rms_increase_ratio: self
                        .params
                        .rdo_uastc_ldr_4x4_max_allowed_rms_increase_ratio
                        .get(),
                    skip_block_rms_thresh: self
                        .params
                        .rdo_uastc_ldr_4x4_skip_block_rms_thresh
                        .get(),
                    lz_dict_size: self.params.rdo_uastc_ldr_4x4_dict_size.get() as u32,
                    smooth_block_max_error_scale: self
                        .params
                        .rdo_uastc_ldr_4x4_max_smooth_block_error_scale
                        .get(),
                    max_smooth_block_std_dev: self
                        .params
                        .rdo_uastc_ldr_4x4_smooth_block_max_std_dev
                        .get(),
                    ..Default::default()
                };
                let first = slice_desc.first_block_index as usize;
                let Some(blocks) = self.source_blocks.get(first..first + total_blocks as usize)
                else {
                    return Ec::FailedUastcRdoPostProcess;
                };
                let pixels: Vec<ColorRgba> =
                    blocks.iter().flat_map(|b| b.iter().copied()).collect();
                let total_jobs = if self.params.rdo_uastc_ldr_4x4_multithreading {
                    self.params.job_pool_threads.min(4)
                } else {
                    0
                };
                let (status, _) = uastc_rdo(
                    total_blocks,
                    &mut tex,
                    &pixels,
                    &rdo_params,
                    self.params.pack_uastc_ldr_4x4_flags,
                    total_jobs,
                );
                if !status {
                    return Ec::FailedUastcRdoPostProcess;
                }
            }

            let data: Vec<u8> = tex.iter().flat_map(|b| b.iter().copied()).collect();
            out.slice_image_crcs[slice_index] = crc16(&data, 0);
            out.slice_image_data[slice_index] = data;
        }
        Ec::Success
    }

    /// `read_source_images` for caller supplied LDR or HDR images.
    fn read_source_images(&mut self) -> bool {
        if self.params.hdr {
            return self.read_source_images_hdr() && self.check_slice_descs();
        }
        let total_source_files = self.params.source_images.len() as u32;
        if total_source_files == 0 {
            return false;
        }
        self.slice_descs.clear();
        self.slice_images.clear();
        self.total_blocks = 0;
        self.any_source_image_has_alpha = false;

        let identity_swizzle = self.params.swizzle == [0, 1, 2, 3];
        let mut source_images: Vec<Image> = Vec::with_capacity(total_source_files as usize);

        // First load all source images, and determine if any have an alpha channel.
        for source_file_index in 0..total_source_files as usize {
            let mut file_image = std::mem::take(&mut self.params.source_images[source_file_index]);
            let mut alpha_swizzled = false;
            if !identity_swizzle {
                let s = self.params.swizzle.map(|c| c as usize & 3);
                for c in file_image.get_pixels_mut() {
                    *c = [c[s[0]], c[s[1]], c[s[2]], c[s[3]]];
                }
                alpha_swizzled = self.params.swizzle[3] != 3;
            }

            let mut has_alpha = false;
            if self.params.force_alpha || alpha_swizzled {
                has_alpha = true;
            } else if !self.params.check_for_alpha {
                file_image.set_alpha(255);
            } else if file_image.has_alpha(3) {
                has_alpha = true;
            }
            if has_alpha {
                self.any_source_image_has_alpha = true;
            }

            if self.params.y_flip {
                // Not ported; libktx flips images itself.
                return false;
            }

            let (width, height) = (file_image.get_width(), file_image.get_height());
            if width == 0 || height == 0 {
                return false;
            }
            if width > BASISU_MAX_SUPPORTED_TEXTURE_DIMENSION
                || height > BASISU_MAX_SUPPORTED_TEXTURE_DIMENSION
            {
                return false;
            }
            source_images.push(file_image);
        }
        // The caller's images were moved out above, as C++ copies them.
        self.params.source_images.clear();

        let bw = self.get_block_width();
        let bh = self.get_block_height();

        // Now, for each source image, create the slices corresponding to that image.
        for (source_file_index, file_image) in source_images.into_iter().enumerate() {
            // m_mip_gen is not ported: one slice (mip 0) per image.
            let mut slices = vec![file_image];
            let mut mip_indices = vec![0u32];

            if self.any_source_image_has_alpha && !self.params.uastc {
                // For ETC1S, if source has alpha, then even mips will have RGB, and odd mips will
                // have alpha in RGB.
                let mut alpha_slices = Vec::with_capacity(slices.len() * 2);
                let mut new_mip_indices = Vec::new();
                for (i, lvl) in slices.iter().enumerate() {
                    let mut lvl_rgb = lvl.clone();
                    let mut lvl_a = lvl_rgb.clone();
                    for c in lvl_a.get_pixels_mut() {
                        let a = c[3];
                        *c = [a, a, a, 255];
                    }
                    lvl_rgb.set_alpha(255);
                    alpha_slices.push(lvl_rgb);
                    new_mip_indices.push(i as u32);
                    alpha_slices.push(lvl_a);
                    new_mip_indices.push(i as u32);
                }
                slices = alpha_slices;
                mip_indices = new_mip_indices;
            }

            for (slice_index, mut slice_image) in slices.into_iter().enumerate() {
                let orig_width = slice_image.get_width();
                let orig_height = slice_image.get_height();

                let mut is_alpha_slice = false;
                if self.any_source_image_has_alpha {
                    is_alpha_slice = if self.params.uastc {
                        slice_image.has_alpha(3)
                    } else {
                        slice_index & 1 != 0
                    };
                }

                // Enlarge the source image to block boundaries, duplicating edge pixels if
                // necessary to avoid introducing extra colors into blocks.
                let w = slice_image.get_block_width(bw) * bw;
                let h = slice_image.get_block_height(bh) * bh;
                slice_image.crop_dup_borders(w, h);

                let mut slice_desc = BasisuBackendSliceDesc {
                    first_block_index: self.total_blocks,
                    orig_width,
                    orig_height,
                    width: slice_image.get_width(),
                    height: slice_image.get_height(),
                    num_blocks_x: slice_image.get_block_width(bw),
                    num_blocks_y: slice_image.get_block_height(bh),
                    ..Default::default()
                };
                slice_desc.num_macroblocks_x = (slice_desc.num_blocks_x + 1) >> 1;
                slice_desc.num_macroblocks_y = (slice_desc.num_blocks_y + 1) >> 1;
                slice_desc.source_file_index = source_file_index as u32;
                slice_desc.mip_index = mip_indices[slice_index];
                slice_desc.alpha = is_alpha_slice;
                slice_desc.iframe = false;
                if self.params.tex_type == C_BASIS_TEX_TYPE_VIDEO_FRAMES {
                    // ETC1S: only the first frame is currently an iframe; UASTC: all.
                    slice_desc.iframe = self.params.uastc || source_file_index == 0;
                }

                self.total_blocks += slice_desc.num_blocks_x * slice_desc.num_blocks_y;
                self.slice_descs.push(slice_desc);
                self.slice_images.push(slice_image);
            }
        }

        self.check_slice_descs()
    }

    /// The HDR part of `read_source_images` (HDR inputs have no alpha).
    fn read_source_images_hdr(&mut self) -> bool {
        let total_source_files = self.params.source_images_hdr.len();
        if total_source_files == 0 {
            return false;
        }
        self.slice_descs.clear();
        self.slice_images_hdr.clear();
        self.total_blocks = 0;
        self.any_source_image_has_alpha = false;
        let identity_swizzle = self.params.swizzle == [0, 1, 2, 3];
        let images = std::mem::take(&mut self.params.source_images_hdr);
        let mut source_images = Vec::with_capacity(total_source_files);
        for mut file_image in images {
            self.clean_hdr_image(&mut file_image);
            if !identity_swizzle {
                // For now, alpha is always 1.0f in UASTC HDR.
                let s = self.params.swizzle.map(|c| c as usize & 3);
                for c in file_image.get_pixels_mut() {
                    *c = [c[s[0]], c[s[1]], c[s[2]], 1.0];
                }
            }
            if self.params.y_flip {
                // Not ported; libktx flips images itself.
                return false;
            }
            let (width, height) = (file_image.get_width(), file_image.get_height());
            if width == 0 || height == 0 {
                return false;
            }
            if width > BASISU_MAX_SUPPORTED_TEXTURE_DIMENSION
                || height > BASISU_MAX_SUPPORTED_TEXTURE_DIMENSION
            {
                return false;
            }
            source_images.push(file_image);
        }

        let bw = self.get_block_width();
        let bh = self.get_block_height();
        for (source_file_index, mut slice_image) in source_images.into_iter().enumerate() {
            let orig_width = slice_image.get_width();
            let orig_height = slice_image.get_height();
            // Don't pad in 6x6 mode, the lower level compressor handles it.
            if self.params.hdr_mode == HdrModes::UastcHdr4x4 {
                let w = slice_image.get_block_width(bw) * bw;
                let h = slice_image.get_block_height(bh) * bh;
                slice_image.crop_dup_borders(w, h);
            }
            let mut slice_desc = BasisuBackendSliceDesc {
                first_block_index: self.total_blocks,
                orig_width,
                orig_height,
                width: slice_image.get_width(),
                height: slice_image.get_height(),
                num_blocks_x: slice_image.get_block_width(bw),
                num_blocks_y: slice_image.get_block_height(bh),
                source_file_index: source_file_index as u32,
                ..Default::default()
            };
            slice_desc.num_macroblocks_x = (slice_desc.num_blocks_x + 1) >> 1;
            slice_desc.num_macroblocks_y = (slice_desc.num_blocks_y + 1) >> 1;
            // If it's not ETC1S, all slices are currently i-frames.
            slice_desc.iframe = self.params.tex_type == C_BASIS_TEX_TYPE_VIDEO_FRAMES;
            self.total_blocks += slice_desc.num_blocks_x * slice_desc.num_blocks_y;
            self.slice_descs.push(slice_desc);
            self.slice_images_hdr.push(slice_image);
        }
        true
    }

    /// `clean_hdr_image`: rescales images whose maximum exceeds `ASTC_HDR_MAX_VAL` and cleans
    /// pixels ASTC HDR can't encode.
    fn clean_hdr_image(&mut self, src_img: &mut ImageF) {
        let mut max_used_val = 0f32;
        for c in src_img.get_pixels() {
            for &v in c.iter().take(3) {
                max_used_val = if max_used_val < v { v } else { max_used_val };
            }
        }
        let mut hdr_image_scale = 1.0f64;
        // If the max value can't be encoded safely to ASTC HDR, rescale the source image.
        if max_used_val > ASTC_HDR_MAX_VAL {
            hdr_image_scale = (max_used_val / ASTC_HDR_MAX_VAL) as f64;
            let inv_hdr_image_scale = ASTC_HDR_MAX_VAL as f64 / max_used_val as f64;
            for c in src_img.get_pixels_mut() {
                for v in c.iter_mut().take(3) {
                    *v = (*v as f64 * inv_hdr_image_scale).min(ASTC_HDR_MAX_VAL as f64) as f32;
                }
            }
            println!(
                "Warning: The input HDR image's maximum used float value was {max_used_val:.6}, which is too high to encode as ASTC HDR. The image's components have been linearly scaled so the maximum used value is {:.6}, by multiplying by {inv_hdr_image_scale:.6}.",
                ASTC_HDR_MAX_VAL
            );
            println!(
                "The decoded/sampled ASTC HDR texture will have to be scaled up by {hdr_image_scale:.6}. See the \"HDRScale\" KTX2 key value field."
            );
        }
        // Remember the scale factor so it can be written to the output file.
        self.hdr_image_scale = hdr_image_scale as f32;
        // Final check of the input pixels for anything bad that could cause encoding problems.
        if !src_img.clean_astc_hdr_pixels(ASTC_HDR_MAX_VAL) {
            println!(
                "Warning: clean_astc_hdr_pixels() had to modify the input image to encode to ASTC HDR - see previous warning(s)."
            );
        }
    }

    /// The slice sanity checks at the end of `read_source_images`.
    fn check_slice_descs(&self) -> bool {
        // Make sure we don't have too many slices
        if self.slice_descs.len() as u32 > BASISU_MAX_SLICES {
            return false;
        }

        // Basic sanity check on the slices
        for w in self.slice_descs.windows(2) {
            let (prev, cur) = (&w[0], &w[1]);
            let image_delta = cur.source_file_index as i32 - prev.source_file_index as i32;
            if image_delta > 1 {
                return false;
            }
            if image_delta == 0 && cur.mip_index as i32 - prev.mip_index as i32 > 1 {
                return false;
            }
        }

        for (i, slice_desc) in self.slice_descs.iter().enumerate() {
            if self.any_source_image_has_alpha {
                if !self.params.uastc {
                    // For ETC1S, alpha slices must be at odd slice indices.
                    if slice_desc.alpha {
                        if i & 1 == 0 {
                            return false;
                        }
                        let prev = &self.slice_descs[i - 1];
                        if prev.source_file_index != slice_desc.source_file_index
                            || prev.alpha
                            || prev.mip_index != slice_desc.mip_index
                            || prev.num_blocks_x != slice_desc.num_blocks_x
                            || prev.num_blocks_y != slice_desc.num_blocks_y
                        {
                            return false;
                        }
                    } else if i & 1 != 0 {
                        return false;
                    }
                }
            } else if slice_desc.alpha {
                return false;
            }
            if slice_desc.orig_width > slice_desc.width
                || slice_desc.orig_height > slice_desc.height
            {
                return false;
            }
            if slice_desc.source_file_index == 0
                && self.params.tex_type == C_BASIS_TEX_TYPE_VIDEO_FRAMES
                && !slice_desc.iframe
            {
                return false;
            }
        }
        true
    }

    /// `validate_texture_type_constraints`.
    fn validate_texture_type_constraints(&self) -> bool {
        // In 2D mode anything goes.
        if self.params.tex_type == C_BASIS_TEX_TYPE_2D {
            return true;
        }
        let total_basis_images = self
            .slice_descs
            .iter()
            .map(|d| d.source_file_index + 1)
            .max()
            .unwrap_or(0);
        if self.params.tex_type == C_BASIS_TEX_TYPE_CUBEMAP_ARRAY && total_basis_images % 6 != 0 {
            return false;
        }
        // All mip0's must have the same dimensions and each image the same # of mipmap levels.
        let mut image_mipmap_levels = vec![0u32; total_basis_images as usize];
        let mut dims: Option<(u32, u32)> = None;
        for d in &self.slice_descs {
            let l = &mut image_mipmap_levels[d.source_file_index as usize];
            *l = (*l).max(d.mip_index + 1);
            if d.mip_index != 0 {
                continue;
            }
            match dims {
                None => dims = Some((d.orig_width, d.orig_height)),
                Some(wh) if wh != (d.orig_width, d.orig_height) => return false,
                _ => {}
            }
        }
        image_mipmap_levels
            .iter()
            .all(|&l| l == image_mipmap_levels[0])
    }

    /// `encode_slices_to_astc_6x6_hdr`: ASTC HDR 6x6 and UASTC HDR 6x6 intermediate.
    fn encode_slices_to_astc_6x6_hdr(&mut self) -> BasisCompressorErrorCode {
        let out = &mut self.uastc_backend_output;
        out.tex_format = if self.params.hdr_mode == HdrModes::AstcHdr6x6 {
            BasisTexFormat::AstcHdr6x6
        } else {
            BasisTexFormat::UastcHdr6x6Intermediate
        };
        out.etc1s = false;
        out.srgb = false;
        out.slice_desc = self.slice_descs.clone();
        out.slice_image_data = vec![Vec::new(); self.slice_descs.len()];
        out.slice_image_crcs = vec![0; self.slice_descs.len()];
        let mut global_cfg = self.params.astc_hdr_6x6_options;
        global_cfg.image_stats = self.params.compute_stats;
        global_cfg.debug_images = self.params.debug_images;
        global_cfg.output_images = self.params.debug_images;
        global_cfg.debug_output = self.params.debug;
        global_cfg.status_output = self.params.status_output || self.params.debug;
        for (slice_index, source_image) in self.slice_images_hdr.iter().enumerate() {
            let Some((intermediate, astc)) =
                compress_photo(source_image, &global_cfg, self.params.job_pool_threads)
            else {
                return Ec::FailedEncodeUastc;
            };
            let dst_buf = if self.params.hdr_mode == HdrModes::AstcHdr6x6 {
                astc
            } else {
                intermediate
            };
            out.slice_image_crcs[slice_index] = crc16(&dst_buf, 0);
            out.slice_image_data[slice_index] = dst_buf;
        }
        Ec::Success
    }

    /// `encode_slices_to_uastc_4x4_hdr`. The 256 block jobs are independent, so they run in
    /// order on the calling thread. Debug block statistics are not ported.
    fn encode_slices_to_uastc_4x4_hdr(&mut self) -> BasisCompressorErrorCode {
        let out = &mut self.uastc_backend_output;
        out.tex_format = BasisTexFormat::UastcHdr4x4;
        out.etc1s = false;
        out.srgb = false;
        out.slice_desc = self.slice_descs.clone();
        out.slice_image_data = vec![Vec::new(); self.slice_descs.len()];
        out.slice_image_crcs = vec![0; self.slice_descs.len()];

        let opts = &mut self.params.uastc_hdr_4x4_options;
        if !self.params.perceptual {
            opts.base.r_err_scale = 1.0;
            opts.base.g_err_scale = 1.0;
        }
        const DEFAULT_BC6H_ERROR_WEIGHT: f32 = 0.65;
        const LOWEST_BC6H_ERROR_WEIGHT: f32 = 0.1;
        opts.bc6h_err_weight = if self.params.hdr_favor_astc {
            LOWEST_BC6H_ERROR_WEIGHT
        } else {
            DEFAULT_BC6H_ERROR_WEIGHT
        };
        let opts = *opts;
        let bc6h_err_weight = opts.bc6h_err_weight as f64;
        let astc_err_weight = 1.0 - bc6h_err_weight;

        let mut all_results = Vec::with_capacity(256);
        for (slice_index, slice_desc) in self.slice_descs.iter().enumerate() {
            // gpu_image::init(cUASTC_HDR_4x4, orig_width, orig_height)
            let num_blocks_x = slice_desc.orig_width.div_ceil(4);
            let num_blocks_y = slice_desc.orig_height.div_ceil(4);
            let total_blocks = num_blocks_x * num_blocks_y;
            let source_image = &self.slice_images_hdr[slice_index];
            let mut tex = vec![[0u8; 16]; total_blocks as usize];
            for (block_index, dest_block) in tex.iter_mut().enumerate() {
                let block_x = block_index as u32 % num_blocks_x;
                let block_y = block_index as u32 / num_blocks_x;
                let mut block_pixels = [[0f32; 4]; 16];
                source_image.extract_block_clamped(
                    &mut block_pixels,
                    block_x * 4,
                    block_y * 4,
                    4,
                    4,
                );
                let mut rgb_pixels = [0f32; 48];
                let mut rgb_pixels_half = [0u16; 48];
                for i in 0..16 {
                    for c in 0..3 {
                        rgb_pixels[i * 3 + c] = block_pixels[i][c];
                        rgb_pixels_half[i * 3 + c] =
                            float_to_half_non_neg_no_nan_inf(block_pixels[i][c]);
                    }
                }
                if !astc_hdr_4x4_enc_block(&rgb_pixels, &rgb_pixels_half, &opts, &mut all_results) {
                    return Ec::FailedEncodeUastc;
                }
                let mut src_half = [[0u16; 3]; 16];
                for (i, d) in src_half.iter_mut().enumerate() {
                    d.copy_from_slice(&rgb_pixels_half[i * 3..i * 3 + 3]);
                }
                let mut best_err = 1e30f32 as f64;
                let mut best_result_index = 0usize;
                for (i, r) in all_results.iter_mut().enumerate() {
                    let mut unpacked_bc6h_block = [[0u16; 3]; 16];
                    unpack_bc6h(&r.bc6h_block, &mut unpacked_bc6h_block, false);
                    r.bc6h_block_error =
                        compute_block_error(16, &src_half, &unpacked_bc6h_block, &opts.base);
                    let overall_err =
                        r.bc6h_block_error * bc6h_err_weight + r.best_block_error * astc_err_weight;
                    if i == 0 || overall_err < best_err {
                        best_err = overall_err;
                        best_result_index = i;
                    }
                }
                let Some(best_results) = all_results.get(best_result_index) else {
                    return Ec::FailedEncodeUastc;
                };
                if !astc_hdr_4x4_pack_results_to_block(dest_block, best_results) {
                    return Ec::FailedEncodeUastc;
                }
                // Verify that this block is valid UASTC HDR and transcodes to BC6H (except in
                // fastest mode).
                if opts.level > 0 {
                    let mut transcoded_bc6h_blk = [0u8; 16];
                    if !astc_hdr_transcode_to_bc6h(dest_block, &mut transcoded_bc6h_blk) {
                        return Ec::FailedEncodeUastc;
                    }
                }
            }
            let data: Vec<u8> = tex.iter().flat_map(|b| b.iter().copied()).collect();
            out.slice_image_crcs[slice_index] = crc16(&data, 0);
            out.slice_image_data[slice_index] = data;
        }
        Ec::Success
    }

    /// `extract_source_blocks`.
    fn extract_source_blocks(&mut self) -> bool {
        if self.params.hdr {
            // No need to extract blocks in 6x6 mode, but the 4x4 compressors want 4x4 blocks.
            if self.params.hdr_mode != HdrModes::UastcHdr4x4 {
                return true;
            }
            self.source_blocks_hdr = vec![[[0f32; 4]; 16]; self.total_blocks as usize];
            for (slice_desc, img) in self.slice_descs.iter().zip(&self.slice_images_hdr) {
                for block_y in 0..slice_desc.num_blocks_y {
                    for block_x in 0..slice_desc.num_blocks_x {
                        let idx = (slice_desc.first_block_index
                            + block_x
                            + block_y * slice_desc.num_blocks_x)
                            as usize;
                        let blk = &mut self.source_blocks_hdr[idx];
                        img.extract_block_clamped(blk, block_x * 4, block_y * 4, 4, 4);
                        // Additional (technically optional) early sanity checking of the block texels.
                        for p in blk.iter() {
                            for &v in p.iter().take(3) {
                                if v.is_nan()
                                    || v.is_infinite()
                                    || v < 0.0
                                    || v as f64 > MAX_HALF_FLOAT
                                {
                                    return false;
                                }
                            }
                        }
                    }
                }
            }
            return true;
        }
        self.source_blocks = vec![[[0u8; 4]; 16]; self.total_blocks as usize];
        for (slice_desc, img) in self.slice_descs.iter().zip(&self.slice_images) {
            for block_y in 0..slice_desc.num_blocks_y {
                for block_x in 0..slice_desc.num_blocks_x {
                    let idx = (slice_desc.first_block_index
                        + block_x
                        + block_y * slice_desc.num_blocks_x) as usize;
                    img.extract_block_clamped(
                        &mut self.source_blocks[idx],
                        block_x * 4,
                        block_y * 4,
                        4,
                        4,
                    );
                }
            }
        }
        true
    }

    /// `process_frontend`.
    fn process_frontend(&mut self) -> bool {
        let total_texels = (self.total_blocks as f32 * 16.0) as f64;

        let mut endpoint_clusters = self.params.etc1s_max_endpoint_clusters as i32;
        let mut selector_clusters = self.params.etc1s_max_selector_clusters as i32;
        if endpoint_clusters > C_MAX_ENDPOINT_CLUSTERS as i32
            || selector_clusters > C_MAX_SELECTOR_CLUSTERS as i32
        {
            return false;
        }

        if self.params.quality_level != -1 {
            let quality = (self.params.quality_level as f32 / 255.0).clamp(0.0, 1.0);

            let bits_per_endpoint_cluster = 14.0f32;
            let max_desired_endpoint_cluster_bits_per_texel = 1.0f32;
            let mut max_endpoints = ((max_desired_endpoint_cluster_bits_per_texel as f64
                * total_texels)
                / bits_per_endpoint_cluster as f64) as i32;

            let mid = 128.0f32 / 255.0;
            let mut color_endpoint_quality = quality;
            let endpoint_split_point = 0.5f32;
            const ENDPOINT_CODEBOOK_MID_QUALITY_CODEBOOK_SIZE: i32 = 4800;
            const MAX_ENDPOINT_CODEBOOK_SIZE: i32 = 8192;

            if color_endpoint_quality <= mid {
                color_endpoint_quality = lerp(
                    0.0,
                    endpoint_split_point,
                    (color_endpoint_quality / mid).powf(0.65),
                );
                max_endpoints =
                    max_endpoints.clamp(256, ENDPOINT_CODEBOOK_MID_QUALITY_CODEBOOK_SIZE);
                max_endpoints = (max_endpoints as u32).min(self.total_blocks) as i32;
                if max_endpoints < 64 {
                    max_endpoints = 64;
                }
                endpoint_clusters =
                    ((0.5 + lerp(32.0, max_endpoints as f32, color_endpoint_quality)) as u32)
                        .clamp(32, C_MAX_ENDPOINT_CLUSTERS) as i32;
            } else {
                color_endpoint_quality = ((color_endpoint_quality - mid) / (1.0 - mid)).powf(1.6);
                max_endpoints = max_endpoints.clamp(256, MAX_ENDPOINT_CODEBOOK_SIZE);
                max_endpoints = (max_endpoints as u32).min(self.total_blocks) as i32;
                if max_endpoints < ENDPOINT_CODEBOOK_MID_QUALITY_CODEBOOK_SIZE {
                    max_endpoints = ENDPOINT_CODEBOOK_MID_QUALITY_CODEBOOK_SIZE;
                }
                endpoint_clusters = ((0.5
                    + lerp(
                        ENDPOINT_CODEBOOK_MID_QUALITY_CODEBOOK_SIZE as f32,
                        max_endpoints as f32,
                        color_endpoint_quality,
                    )) as u32)
                    .clamp(32, C_MAX_ENDPOINT_CLUSTERS) as i32;
            }

            let bits_per_selector_cluster = 14.0f32;
            let max_desired_selector_cluster_bits_per_texel = 1.0f32;
            let mut max_selectors = ((max_desired_selector_cluster_bits_per_texel as f64
                * total_texels)
                / bits_per_selector_cluster as f64) as i32;
            max_selectors = max_selectors.clamp(256, C_MAX_SELECTOR_CLUSTERS as i32);
            max_selectors = (max_selectors as u32).min(self.total_blocks) as i32;

            let color_selector_quality = quality.powf(2.62);
            if max_selectors < 96 {
                max_selectors = 96;
            }
            selector_clusters = ((0.5 + lerp(96.0, max_selectors as f32, color_selector_quality))
                as u32)
                .clamp(8, C_MAX_SELECTOR_CLUSTERS) as i32;

            let p = &mut self.params;
            if p.quality_level >= 223 {
                if !p.selector_rdo_thresh.was_changed() {
                    if !p.endpoint_rdo_thresh.was_changed() {
                        p.endpoint_rdo_thresh.mul_assign(0.25);
                    }
                    if !p.selector_rdo_thresh.was_changed() {
                        p.selector_rdo_thresh.mul_assign(0.25);
                    }
                }
            } else if p.quality_level >= 192 {
                if !p.endpoint_rdo_thresh.was_changed() {
                    p.endpoint_rdo_thresh.mul_assign(0.5);
                }
                if !p.selector_rdo_thresh.was_changed() {
                    p.selector_rdo_thresh.mul_assign(0.5);
                }
            } else if p.quality_level >= 160 {
                if !p.endpoint_rdo_thresh.was_changed() {
                    p.endpoint_rdo_thresh.mul_assign(0.75);
                }
                if !p.selector_rdo_thresh.was_changed() {
                    p.selector_rdo_thresh.mul_assign(0.75);
                }
            } else if p.quality_level >= 129 {
                let l = (quality - 129.0 / 255.0) / ((160 - 129) as f32 / 255.0);
                if !p.endpoint_rdo_thresh.was_changed() {
                    p.endpoint_rdo_thresh.mul_assign(lerp(1.0, 0.75, l));
                }
                if !p.selector_rdo_thresh.was_changed() {
                    p.selector_rdo_thresh.mul_assign(lerp(1.0, 0.75, l));
                }
            }
        }

        let p = BasisuFrontendParams {
            source_blocks: std::mem::take(&mut self.source_blocks),
            max_endpoint_clusters: endpoint_clusters as u32,
            max_selector_clusters: selector_clusters as u32,
            perceptual: self.params.perceptual,
            debug_stats: self.params.debug,
            debug_images: self.params.debug_images,
            compression_level: self.params.etc1s_compression_level.get() as u32,
            tex_type: self.params.tex_type,
            multithreaded: self.params.multithreading,
            disable_hierarchical_endpoint_codebooks: self
                .params
                .disable_hierarchical_endpoint_codebooks,
            validate: self.params.validate_etc1s,
            job_pool_threads: self.params.job_pool_threads,
            ..Default::default()
        };
        if !self.frontend.init(&p) {
            return false;
        }
        // C++ ignores the result; it is only false when BASISU_FRONTEND_VERIFY would abort.
        self.frontend.compress()
    }

    /// `process_backend`.
    fn process_backend(&mut self) -> bool {
        let mut backend_params = BasisuBackendParams {
            debug: self.params.debug,
            debug_images: self.params.debug_images,
            etc1s: true,
            compression_level: self.params.etc1s_compression_level.get() as u32,
            ..Default::default()
        };
        if !self.params.no_endpoint_rdo {
            backend_params.endpoint_rdo_quality_thresh = self.params.endpoint_rdo_thresh.get();
        }
        if !self.params.no_selector_rdo {
            backend_params.selector_rdo_quality_thresh = self.params.selector_rdo_thresh.get();
        }
        backend_params.used_global_codebooks = false;
        backend_params.validate = self.params.validate_output_data;

        self.backend.init(&backend_params, &self.slice_descs);
        self.backend.encode(&mut self.frontend) != 0
    }

    /// `create_basis_file_and_transcode` (the optional transcode validation is not ported).
    fn create_basis_file_and_transcode(&mut self) -> bool {
        let encoded_output = if self.params.uastc {
            &self.uastc_backend_output
        } else {
            self.backend.get_output()
        };
        if !self.basis_file.init(
            encoded_output,
            self.params.tex_type,
            self.params.userdata0,
            self.params.userdata1,
            self.params.y_flip,
            self.params.us_per_frame,
        ) {
            return false;
        }
        self.output_basis_file = self.basis_file.get_compressed_data().to_vec();
        let total_orig_pixels: u32 = self
            .slice_descs
            .iter()
            .map(|d| d.orig_width * d.orig_height)
            .sum();
        self.total_slice_orig_texels = total_orig_pixels;
        self.basis_file_size = self.output_basis_file.len() as u32;
        self.basis_bits_per_texel = if total_orig_pixels != 0 {
            (self.output_basis_file.len() as f32 * 8.0) / total_orig_pixels as f32
        } else {
            0.0
        };
        if self.params.validate_output_data {
            // Transcode validation of the output is not ported.
            return false;
        }
        true
    }
}

/// `lerp(a, b, s)`.
fn lerp(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}
