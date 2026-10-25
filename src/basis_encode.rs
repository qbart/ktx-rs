//! Port of `lib/src/basis_encode.cpp`: `ktxTexture2_CompressBasisEx` and
//! `ktxTexture2_CompressBasis` on top of the pure Rust `basis_compressor` port. The images are
//! copied (and swizzled) into compressor images, encoded to a `.basis` file, which is then
//! unpicked into the KTX2 level data, BasisLZ global data and an ETC1S / UASTC DFD.
//!
//! The UASTC HDR 4x4 and 6x6 intermediate codecs fail with `UnsupportedFeature` until their
//! encoders are ported. `noSSE` has no effect: the port has no SIMD kernels and its output
//! matches the SSE 4.1 build.

use crate::basis_transcode::{
    AlphaContent, BASIS_LZ_ETC1S_IMAGE_DESC_SIZE, BASIS_LZ_GLOBAL_HEADER_SIZE,
};
use crate::basisu::basis_file::{
    BASIS_SLICE_DESC_SIZE, BasisFileHeader, BasisSliceDesc, C_BASIS_HEADER_FLAG_HAS_ALPHA_SLICES,
    C_SLICE_DESC_FLAGS_FRAME_IS_I_FRAME, C_SLICE_DESC_FLAGS_HAS_ALPHA,
};
use crate::basisu::comp::{
    BasisCompressor, BasisCompressorErrorCode, BasisCompressorParams, Image,
};
use crate::basisu::frontend::{C_BASIS_TEX_TYPE_2D, C_BASIS_TEX_TYPE_VIDEO_FRAMES};
use crate::defs::{KTX_SWIZZLE_KEY, SupercmpScheme};
use crate::dfd::khr_df::*;
use crate::dfd::queries::get_dfd_component_info_unpacked;
use crate::dfd::{
    BITLENGTH, BITOFFSET, BYTESPLANE0, BYTESPLANE1, CHANNELID, DESCRIPTORBLOCKSIZE, DESCRIPTORTYPE,
    FLAGS, MODEL, PRIMARIES, QUALIFIERS, SAMPLELOWER, SAMPLEPOSITION_ALL, SAMPLEUPPER, TRANSFER,
    VENDORID, VERSIONNUMBER, bdb, bdb_mut, dfdsetsval, dfdsetval, dfdval,
};
use crate::encode_params::*;
use crate::error::{ErrorCode, KtxResult};
use crate::formatsize::{FormatSize, KTX_FORMAT_SIZE_PACKED_BIT};
use crate::texture::KtxTexture;
use crate::texture2::Texture2;
use crate::vkformat::VkFormat;

/// `swizzle_e`.
pub type SwizzleE = u8;
pub const R: SwizzleE = 1;
pub const G: SwizzleE = 2;
pub const B: SwizzleE = 3;
pub const A: SwizzleE = 4;
pub const ZERO: SwizzleE = 5;
pub const ONE: SwizzleE = 6;

/// `copy_rgba_to_rgba`.
fn copy_rgba_to_rgba(rgbadst: &mut [u8], rgbasrc: &[u8], image_size: usize) {
    let n = image_size.min(rgbadst.len()).min(rgbasrc.len());
    rgbadst[..n].copy_from_slice(&rgbasrc[..n]);
}

/// `copy_rgb_to_rgba`: no swizzle, alpha set to 0xff to convince Basis there is no alpha.
fn copy_rgb_to_rgba(rgbadst: &mut [u8], rgbsrc: &[u8], image_size: usize) {
    let src = &rgbsrc[..image_size.min(rgbsrc.len())];
    for (d, s) in rgbadst.as_chunks_mut::<4>().0.iter_mut().zip(src.chunks(3)) {
        d[..s.len()].copy_from_slice(s);
        d[3] = 0xff;
    }
}

/// Port of `swizzle_to_rgba` (public, like the C++, so the unit tests can use it). Expects 8-bit
/// components and no row padding. Source bytes past the end of `rgbasrc` read as 0; an invalid
/// swizzle leaves the destination component unchanged (the C++ asserts).
pub fn swizzle_to_rgba(
    rgbadst: &mut [u8],
    rgbasrc: &[u8],
    src_len: u32,
    image_size: usize,
    swizzle: &[SwizzleE; 4],
) {
    let src_len = src_len.max(1) as usize;
    let mut i = 0;
    let mut d = 0;
    while i < image_size && d + 4 <= rgbadst.len() {
        let src = |c: usize| rgbasrc.get(i + c).copied().unwrap_or(0);
        for (c, &s) in swizzle.iter().enumerate() {
            let v = match s {
                R => src(0),
                G => src(1),
                B => src(2),
                A => src(3),
                ZERO => 0x00,
                ONE => 0xff,
                _ => continue,
            };
            rgbadst[d + c] = v;
        }
        d += 4;
        i += src_len;
    }
}

/// Writes a 1 or 2 sample BDB with the common fields of the rewritten Basis DFDs.
fn new_basis_dfd(
    cbdb: &[u32],
    model: u32,
    sample_count: u32,
    dim: u32,
    bytes_plane0: u32,
) -> Vec<u32> {
    let ndb_size = (KHR_DF_WORD_SAMPLESTART + sample_count * KHR_DF_WORD_SAMPLEWORDS) * 4;
    let ndfd_size = ndb_size + 4;
    let mut ndfd = vec![0u32; (ndfd_size / 4) as usize];
    ndfd[0] = ndfd_size;
    let nbdb = bdb_mut(&mut ndfd);
    dfdsetval(nbdb, VENDORID, KHR_DF_VENDORID_KHRONOS);
    dfdsetval(nbdb, DESCRIPTORTYPE, KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT);
    dfdsetval(nbdb, VERSIONNUMBER, KHR_DF_VERSIONNUMBER_LATEST);
    dfdsetval(nbdb, DESCRIPTORBLOCKSIZE, ndb_size);
    dfdsetval(nbdb, MODEL, model);
    dfdsetval(nbdb, PRIMARIES, dfdval(cbdb, PRIMARIES));
    dfdsetval(nbdb, TRANSFER, dfdval(cbdb, TRANSFER));
    dfdsetval(nbdb, FLAGS, dfdval(cbdb, FLAGS));
    nbdb[KHR_DF_WORD_TEXELBLOCKDIMENSION0 as usize] =
        dim | (dim << KHR_DF_SHIFT_TEXELBLOCKDIMENSION1);
    nbdb[KHR_DF_WORD_BYTESPLANE0 as usize] = 0;
    dfdsetval(nbdb, BYTESPLANE0, bytes_plane0);
    nbdb[KHR_DF_WORD_BYTESPLANE4 as usize] = 0;
    ndfd
}

fn set_sample(
    nbdb: &mut [u32],
    s: u32,
    channel_id: u32,
    qualifiers: u32,
    bit_offset: u32,
    bit_length: u32,
    upper: u32,
) {
    dfdsetsval(nbdb, s, CHANNELID, channel_id);
    dfdsetsval(nbdb, s, QUALIFIERS, qualifiers);
    dfdsetsval(nbdb, s, SAMPLEPOSITION_ALL, 0);
    dfdsetsval(nbdb, s, BITOFFSET, bit_offset);
    dfdsetsval(nbdb, s, BITLENGTH, bit_length);
    dfdsetsval(nbdb, s, SAMPLELOWER, 0);
    dfdsetsval(nbdb, s, SAMPLEUPPER, upper);
}

impl Texture2 {
    /// `ktxTexture2_rewriteDfd4BasisLzETC1S`: changes the DFD to unsized ETC1S, accounting for
    /// the Basis compressor dropping an all 1's alpha channel.
    fn rewrite_dfd4_basis_lz_etc1s(
        &mut self,
        alpha_content: AlphaContent,
        is_luminance: bool,
        swizzle: Option<&[SwizzleE; 4]>,
    ) {
        let new_sample_count = if alpha_content != AlphaContent::None {
            2
        } else {
            1
        };
        let mut ndfd = new_basis_dfd(bdb(&self.p_dfd), KHR_DF_MODEL_ETC1S, new_sample_count, 3, 8);
        let nbdb = bdb_mut(&mut ndfd);
        if alpha_content != AlphaContent::None {
            dfdsetval(nbdb, BYTESPLANE1, 8);
        }
        for sample in 0..new_sample_count {
            let (channel_id, bit_offset) = if sample == 0 {
                let rrr = !is_luminance && swizzle.is_some_and(|s| s[0] == s[1] && s[1] == s[2]);
                (
                    if rrr {
                        KHR_DF_CHANNEL_ETC1S_RRR
                    } else {
                        KHR_DF_CHANNEL_ETC1S_RGB
                    },
                    0,
                )
            } else {
                let id = match alpha_content {
                    AlphaContent::Alpha => KHR_DF_CHANNEL_ETC1S_AAA,
                    AlphaContent::Green => KHR_DF_CHANNEL_ETC1S_GGG,
                    // This is just to quiet a compiler warning.
                    AlphaContent::None => KHR_DF_CHANNEL_ETC1S_RGB,
                };
                (id, 64)
            };
            set_sample(nbdb, sample, channel_id, 0, bit_offset, 63, u32::MAX);
        }
        self.p_dfd = ndfd;
    }

    /// `ktxTexture2_rewriteDfd4Uastc`.
    fn rewrite_dfd4_uastc(
        &mut self,
        alpha_content: AlphaContent,
        is_luminance: bool,
        swizzle: Option<&[SwizzleE; 4]>,
    ) {
        let mut ndfd = new_basis_dfd(bdb(&self.p_dfd), KHR_DF_MODEL_UASTC, 1, 3, 16);
        let nbdb = bdb_mut(&mut ndfd);
        // The C++ compares against 0 and 1 (not ZERO and ONE), which never matches a swizzle.
        let channel_id = if alpha_content == AlphaContent::Alpha {
            KHR_DF_CHANNEL_UASTC_RGBA
        } else if alpha_content == AlphaContent::Green {
            KHR_DF_CHANNEL_UASTC_RRRG
        } else if swizzle.is_some_and(|s| s[2] == 0 && s[3] == 1) {
            KHR_DF_CHANNEL_UASTC_RG
        } else if !is_luminance && swizzle.is_some_and(|s| s[0] == s[1] && s[1] == s[2]) {
            KHR_DF_CHANNEL_UASTC_RRR
        } else {
            KHR_DF_CHANNEL_UASTC_RGB
        };
        set_sample(nbdb, 0, channel_id, 0, 0, 127, u32::MAX);
        self.p_dfd = ndfd;
    }

    /// Port of `ktxTexture2_CompressBasisEx`: encodes the uncompressed images to ETC1S
    /// (supercompressed with BasisLZ) or UASTC, replacing the images, DFD and format.
    pub fn compress_basis_ex(&mut self, params: &BasisParams) -> KtxResult<()> {
        if params.struct_size != KTX_BASIS_PARAMS_SIZE {
            return Err(ErrorCode::InvalidValue);
        }
        if self.generate_mipmaps {
            return Err(ErrorCode::InvalidOperation);
        }
        if self.supercompression_scheme != SupercmpScheme::NONE {
            return Err(ErrorCode::InvalidOperation); // Can't apply multiple schemes.
        }
        if self.is_compressed {
            // Only non-block compressed formats can be encoded into a Basis format.
            return Err(ErrorCode::InvalidOperation);
        }
        if self.format_size().flags & KTX_FORMAT_SIZE_PACKED_BIT != 0 {
            return Err(ErrorCode::InvalidOperation);
        }

        let (num_components, component_size) = get_dfd_component_info_unpacked(&self.p_dfd);
        if component_size != 1 && component_size != 2 {
            return Err(ErrorCode::InvalidOperation); // Basis can have either 8-bit or 16-bit components.
        }
        if num_components == 1 && params.normal_map {
            return Err(ErrorCode::InvalidOperation); // Not enough components.
        }

        if self.p_data.is_none() {
            self.load_image_data(None)?;
        }

        let mut cparams = BasisCompressorParams {
            read_source_images: false,
            write_output_basis_or_ktx2_files: false,
            create_ktx2_file: false,
            status_output: params.verbose,
            ..Default::default()
        };
        match params.codec {
            KTX_BASIS_CODEC_ETC1S => cparams.uastc = false,
            KTX_BASIS_CODEC_UASTC_LDR_4X4 => cparams.uastc = true,
            KTX_BASIS_CODEC_UASTC_HDR_4X4 | KTX_BASIS_CODEC_UASTC_HDR_6X6_INTERMEDIATE => {
                // TODO: UASTC HDR encoders (docs/spec/basis_encode.md stages 5, 6).
                return Err(ErrorCode::UnsupportedFeature);
            }
            _ => {}
        }

        // Calculate number of images
        let layers_faces = self.num_layers * self.num_faces;
        let num_images: u32 = (1..=self.num_levels)
            .map(|level| layers_faces * self.base_depth.checked_shr(level - 1).unwrap_or(0).max(1))
            .sum();

        // Since we have to copy the data into the compressor images anyway do the swizzle here.
        const RG_TO_RGBA_MAPPING_ETC1S: [SwizzleE; 4] = [R, R, R, G];
        const RG_TO_RGBA_MAPPING_UASTC: [SwizzleE; 4] = [R, G, ZERO, ONE];
        const NORMAL_MAP_MAPPING: [SwizzleE; 4] = [R, R, R, G];
        const R_TO_RGBA_MAPPING: [SwizzleE; 4] = [R, R, R, ONE];
        let mut meta_mapping: [SwizzleE; 4] = [0; 4];
        let mut comp_mapping: Option<[SwizzleE; 4]> = None;

        let mut alpha_content = AlphaContent::None;
        let mut is_luminance = false;

        let nul = params
            .input_swizzle
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(4);
        let mut swizzle_string: Vec<u8> = params.input_swizzle[..nul].to_vec();
        if params.pre_swizzle {
            if !swizzle_string.is_empty() {
                return Err(ErrorCode::InvalidOperation);
            }
            if let Ok(entry) = self.kv_data_head.find_entry(KTX_SWIZZLE_KEY) {
                // Do it this way in case there is no NUL terminator.
                swizzle_string = entry.get_value().1.to_vec();
                // Remove the swizzle as it is no longer needed.
                let _ = self.kv_data_head.delete_kv_pair(KTX_SWIZZLE_KEY);
            }
        }

        if swizzle_string.is_empty() {
            // Set appropriate default swizzle
            if params.normal_map {
                comp_mapping = Some(NORMAL_MAP_MAPPING);
                alpha_content = AlphaContent::Green;
            } else if num_components == 1 {
                comp_mapping = Some(R_TO_RGBA_MAPPING);
            } else if num_components == 2 {
                if cparams.uastc {
                    comp_mapping = Some(RG_TO_RGBA_MAPPING_UASTC);
                } else {
                    comp_mapping = Some(RG_TO_RGBA_MAPPING_ETC1S);
                    alpha_content = AlphaContent::Green;
                }
            } else if num_components == 4 {
                alpha_content = AlphaContent::Alpha;
            }
        } else {
            // Only set comp_mapping for cases we can't shortcut.
            if num_components < 3
                || (num_components == 3 && swizzle_string != b"rgb1")
                || (num_components == 4 && swizzle_string != b"rgba")
            {
                for (i, m) in meta_mapping.iter_mut().enumerate() {
                    match swizzle_string.get(i) {
                        Some(b'r') => *m = R,
                        Some(b'g') => *m = G,
                        Some(b'b') => *m = B,
                        Some(b'a') => *m = A,
                        Some(b'0') => *m = ZERO,
                        Some(b'1') => *m = ONE,
                        _ => {}
                    }
                }
                comp_mapping = Some(meta_mapping);
            }
            if !params.normal_map {
                // An incoming swizzle of RRR1 or RRRG is assumed to be for a luminance texture.
                if meta_mapping[0] == meta_mapping[1] && meta_mapping[1] == meta_mapping[2] {
                    is_luminance = true;
                }
                if meta_mapping[3] != ONE {
                    alpha_content = AlphaContent::Alpha;
                }
            } else {
                alpha_content = AlphaContent::Green;
            }
        }

        // NOTA BENE: It is advantageous for Basis LZ compression to order mipmap levels from
        // largest to smallest.
        let mut source_images = Vec::with_capacity(num_images as usize);
        {
            let Some(data) = self.base.p_data.as_ref() else {
                return Err(ErrorCode::InvalidOperation);
            };
            for level in 0..self.num_levels {
                let width = self.base_width.checked_shr(level).unwrap_or(0).max(1);
                let height = self.base_height.checked_shr(level).unwrap_or(0).max(1);
                let depth = self.base_depth.checked_shr(level).unwrap_or(0).max(1);
                let image_size = self.get_image_size(level);
                for layer in 0..self.num_layers {
                    let face_slices = if self.num_faces == 1 {
                        depth
                    } else {
                        self.num_faces
                    };
                    for slice in 0..face_slices {
                        let offset = self.get_image_offset(level, layer, slice)?;
                        let src = data.get(offset..).unwrap_or_default();
                        let mut img = Image::with_size(width, height);
                        let dst = img.get_pixels_mut().as_flattened_mut();
                        match (&comp_mapping, num_components) {
                            (Some(m), _) => {
                                swizzle_to_rgba(dst, src, num_components, image_size, m)
                            }
                            (None, 4) => copy_rgba_to_rgba(dst, src, image_size),
                            (None, 3) => copy_rgb_to_rgba(dst, src, image_size),
                            _ => {}
                        }
                        source_images.push(img);
                    }
                }
            }
        }
        cparams.source_images = source_images;

        // No longer needed. Reduce memory footprint.
        self.base.p_data = None;
        self.base.data_size = 0;

        // Setup rest of compressor parameters
        cparams.job_pool_threads = params.thread_count.max(1);

        let b = bdb(&self.p_dfd);
        cparams.set_srgb_options(dfdval(b, TRANSFER) == KHR_DF_TRANSFER_SRGB);
        cparams.mip_gen = false; // We provide the mip levels.

        if cparams.uastc {
            cparams.pack_uastc_ldr_4x4_flags = params.uastc_flags;
            if params.uastc_rdo {
                cparams.rdo_uastc_ldr_4x4 = true;
                if params.uastc_rdo_quality_scalar > 0.0 {
                    cparams
                        .rdo_uastc_ldr_4x4_quality_scalar
                        .set(params.uastc_rdo_quality_scalar);
                }
                if params.uastc_rdo_dict_size > 0 {
                    cparams
                        .rdo_uastc_ldr_4x4_dict_size
                        .set(params.uastc_rdo_dict_size as i32);
                }
                if params.uastc_rdo_max_smooth_block_error_scale > 0.0 {
                    cparams
                        .rdo_uastc_ldr_4x4_max_smooth_block_error_scale
                        .set(params.uastc_rdo_max_smooth_block_error_scale);
                }
                if params.uastc_rdo_max_smooth_block_std_dev > 0.0 {
                    cparams
                        .rdo_uastc_ldr_4x4_smooth_block_max_std_dev
                        .set(params.uastc_rdo_max_smooth_block_std_dev);
                }
                // C++ quirk kept: both assignments target the favor simpler modes flag, so
                // uastcRDONoMultithreading decides it and multithreading stays on.
                cparams.rdo_uastc_ldr_4x4_favor_simpler_modes_in_rdo_mode =
                    !params.uastc_rdo_dont_favor_simpler_modes;
                cparams.rdo_uastc_ldr_4x4_favor_simpler_modes_in_rdo_mode =
                    !params.uastc_rdo_no_multithreading;
            }
            cparams.hdr_favor_astc = params.uastc_hdr_favor_astc;
            if params.uastc_hdr_lambda > 0.0 {
                cparams
                    .rdo_uastc_ldr_4x4_quality_scalar
                    .set(params.uastc_hdr_lambda);
                cparams.rdo_uastc_ldr_4x4 = true;
            }
        } else {
            // ETC1S-related params.
            cparams
                .etc1s_compression_level
                .set(params.etc1s_compression_level as i32);

            // Override qualityLevel if both of max{Endpoint,Selector}s have been set.
            if params.max_endpoints != 0 && params.max_selectors != 0 {
                cparams.etc1s_max_endpoint_clusters = params.max_endpoints;
                cparams.etc1s_max_selector_clusters = params.max_selectors;
            } else if params.quality_level != 0 {
                cparams.etc1s_max_endpoint_clusters = 0;
                cparams.etc1s_max_selector_clusters = 0;
                cparams.quality_level = params.quality_level as i32;
            } else {
                cparams.etc1s_max_endpoint_clusters = 0;
                cparams.etc1s_max_selector_clusters = 0;
                cparams.quality_level = 128;
            }

            if params.endpoint_rdo_threshold > 0.0 {
                cparams
                    .endpoint_rdo_thresh
                    .set(params.endpoint_rdo_threshold);
            }
            if params.selector_rdo_threshold > 0.0 {
                cparams
                    .selector_rdo_thresh
                    .set(params.selector_rdo_threshold);
            }

            if params.normal_map {
                cparams.no_endpoint_rdo = true;
                cparams.no_selector_rdo = true;
            } else {
                cparams.no_endpoint_rdo = params.no_endpoint_rdo;
                cparams.no_selector_rdo = params.no_selector_rdo;
            }
        }

        // The encoder uses video to decide whether to create p-frames. Any other setting than
        // 2D is likely to fail the encoder's .basis oriented validity checks.
        cparams.tex_type = if self.is_video {
            C_BASIS_TEX_TYPE_VIDEO_FRAMES
        } else {
            C_BASIS_TEX_TYPE_2D
        };

        let mut c = BasisCompressor::new();
        // init() only fails if told to read source image files.
        let _ = c.init(&cparams);
        let ec = c.process();
        if ec != BasisCompressorErrorCode::Success {
            return Err(ErrorCode::InvalidOperation);
        }

        // Compression successful. Now unpick the basis output and copy the info and images to
        // this texture.
        let bf = c.get_output_basis_file();
        let Some(bfh) = BasisFileHeader::from_bytes(bf) else {
            return Err(ErrorCode::InvalidOperation);
        };
        let slice_at = |i: usize| {
            let ofs = bfh.slice_desc_file_ofs as usize + i * BASIS_SLICE_DESC_SIZE as usize;
            bf.get(ofs..)
                .and_then(BasisSliceDesc::from_bytes)
                .unwrap_or_default()
        };
        let num_levels = self.num_levels as usize;
        let mut level_file_offsets = vec![0u32; num_levels];
        let mut level_byte_lengths = vec![0u32; num_levels];
        let mut level_uncompressed = vec![0u32; num_levels];
        let mut image_data_size: u32 = 0;
        let mut bgd: Vec<u8> = Vec::new();
        let mut slice = 0usize;

        if cparams.uastc {
            for level in 0..num_levels {
                let depth = self
                    .base_depth
                    .checked_shr(level as u32)
                    .unwrap_or(0)
                    .max(1);
                let level_image_count = self.num_layers * self.num_faces * depth;
                let mut level_byte_length = 0u32;
                level_file_offsets[level] = slice_at(slice).file_ofs;
                for _ in 0..level_image_count {
                    let s = slice_at(slice);
                    image_data_size += s.file_size;
                    level_byte_length += s.file_size;
                    slice += 1;
                }
                level_byte_lengths[level] = level_byte_length;
                level_uncompressed[level] = level_byte_length;
            }
        } else {
            // Allocate supercompression global data and write its header.
            let bgd_size = BASIS_LZ_GLOBAL_HEADER_SIZE
                + BASIS_LZ_ETC1S_IMAGE_DESC_SIZE * num_images as usize
                + bfh.endpoint_cb_file_size as usize
                + bfh.selector_cb_file_size as usize
                + bfh.tables_file_size as usize;
            bgd.reserve_exact(bgd_size);
            bgd.extend_from_slice(&(bfh.total_endpoints as u16).to_le_bytes());
            bgd.extend_from_slice(&(bfh.total_selectors as u16).to_le_bytes());
            bgd.extend_from_slice(&bfh.endpoint_cb_file_size.to_le_bytes());
            bgd.extend_from_slice(&bfh.selector_cb_file_size.to_le_bytes());
            bgd.extend_from_slice(&bfh.tables_file_size.to_le_bytes());
            bgd.extend_from_slice(&0u32.to_le_bytes());

            // Write the index of slice descriptions to the global data. Slices are in the order
            // they were passed in, i.e. by mip level; alpha slices are the odd ones.
            let has_alpha_slices = bfh.flags & C_BASIS_HEADER_FLAG_HAS_ALPHA_SLICES != 0;
            for level in 0..num_levels {
                let depth = self
                    .base_depth
                    .checked_shr(level as u32)
                    .unwrap_or(0)
                    .max(1);
                let mut level_byte_length = 0u32;
                level_file_offsets[level] = slice_at(slice).file_ofs;
                for _layer in 0..self.num_layers {
                    let face_slices = if self.num_faces == 1 {
                        depth
                    } else {
                        self.num_faces
                    };
                    for _ in 0..face_slices {
                        let s = slice_at(slice);
                        level_byte_length += s.file_size;
                        let rgb_ofs = s.file_ofs.wrapping_sub(level_file_offsets[level]);
                        let rgb_len = s.file_size;
                        let (mut alpha_ofs, mut alpha_len) = (0, 0);
                        let mut flags_slice = s;
                        if has_alpha_slices {
                            slice += 1;
                            let a = slice_at(slice);
                            level_byte_length += a.file_size;
                            alpha_ofs = a.file_ofs.wrapping_sub(level_file_offsets[level]);
                            alpha_len = a.file_size;
                            flags_slice = a;
                        }
                        // Set the PFrame flag, inverse of the .basis IFrame flag.
                        let image_flags = if self.is_video {
                            (flags_slice.flags & !C_SLICE_DESC_FLAGS_HAS_ALPHA)
                                ^ C_SLICE_DESC_FLAGS_FRAME_IS_I_FRAME
                        } else {
                            0
                        };
                        for v in [image_flags, rgb_ofs, rgb_len, alpha_ofs, alpha_len] {
                            bgd.extend_from_slice(&v.to_le_bytes());
                        }
                        slice += 1;
                    }
                }
                level_byte_lengths[level] = level_byte_length;
                level_uncompressed[level] = 0;
                image_data_size += level_byte_length;
            }

            // Copy the global code books & huffman tables to global data.
            for (ofs, size) in [
                (bfh.endpoint_cb_file_ofs, bfh.endpoint_cb_file_size),
                (bfh.selector_cb_file_ofs, bfh.selector_cb_file_size),
                (bfh.tables_file_ofs, bfh.tables_file_size),
            ] {
                let Some(src) = bf.get(ofs as usize..(ofs + size) as usize) else {
                    return Err(ErrorCode::InvalidOperation);
                };
                bgd.extend_from_slice(src);
            }
        }

        // The basis encoder removes an input alpha channel if every alpha pixel in every image
        // is 255. The DFD must reflect the encoded data.
        if bfh.flags & C_BASIS_HEADER_FLAG_HAS_ALPHA_SLICES == 0 {
            alpha_content = AlphaContent::None;
        }

        let mut new_data = Vec::new();
        if new_data
            .try_reserve_exact(image_data_size as usize)
            .is_err()
        {
            return Err(ErrorCode::OutOfMemory);
        }

        let swizzle = comp_mapping.as_ref();
        let required_level_alignment = if params.codec == KTX_BASIS_CODEC_UASTC_LDR_4X4 {
            self.rewrite_dfd4_uastc(alpha_content, is_luminance, swizzle);
            4 * 4
        } else {
            self.rewrite_dfd4_basis_lz_etc1s(alpha_content, is_luminance, swizzle);
            self.supercompression_scheme = SupercmpScheme::BASIS_LZ;
            1
        };
        // Reflect this in the formatSize
        self.base.protected.format_size = FormatSize::init_from_dfd(&mut self.p_dfd)?;
        self.vk_format = VkFormat::UNDEFINED;
        self.base.is_compressed = true;
        self.base.protected.type_size = 1;

        // Copy in the compressed image data, smallest level first.
        let mut level_offset: u64 = 0;
        let mut level_offsets = vec![0u64; num_levels];
        for level in (0..num_levels).rev() {
            level_offsets[level] = level_offset;
            let ofs = level_file_offsets[level] as usize;
            let len = level_byte_lengths[level] as usize;
            let Some(src) = bf.get(ofs..ofs + len) else {
                return Err(ErrorCode::InvalidOperation);
            };
            new_data.resize(level_offset as usize, 0);
            new_data.extend_from_slice(src);
            level_offset += ktx_padn(required_level_alignment, len as u64);
        }
        let data_size = image_data_size as usize;
        new_data.resize(new_data.len().max(data_size), 0);
        new_data.truncate(data_size);

        let p = self.private_mut();
        p.required_level_alignment = required_level_alignment as u32;
        for level in 0..num_levels {
            if let Some(e) = p.level_index.get_mut(level) {
                e.byte_offset = level_offsets[level];
                e.byte_length = level_byte_lengths[level] as u64;
                e.uncompressed_byte_length = level_uncompressed[level] as u64;
            }
        }
        if !cparams.uastc {
            p.sgd_byte_length = bgd.len() as u64;
            p.supercompression_global_data = bgd;
        }
        self.base.p_data = Some(new_data);
        self.base.data_size = data_size;
        Ok(())
    }

    /// Port of `ktxTexture2_CompressBasis`: ETC1S at a quality of 1 - 255 (0 selects 128).
    pub fn compress_basis(&mut self, quality: u32) -> KtxResult<()> {
        let params = BasisParams {
            struct_size: KTX_BASIS_PARAMS_SIZE,
            thread_count: 1,
            etc1s_compression_level: KTX_ETC1S_DEFAULT_COMPRESSION_LEVEL,
            quality_level: quality,
            ..Default::default()
        };
        self.compress_basis_ex(&params)
    }
}

/// `_KTX_PADN(n, nbytes)`.
fn ktx_padn(n: u64, nbytes: u64) -> u64 {
    nbytes.div_ceil(n) * n
}
