// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of `lib/src/astc_codec.cpp`: `ktxTexture2_CompressAstcEx`, `ktxTexture2_CompressAstc`
//! and `ktxTexture2_DecodeAstc` on top of the pure Rust astcenc port. Each image is
//! compressed by `threadCount` threads sharing one context (`launchThreads`).

use crate::astcenc::{
    self, AstcencError, AstcencImage, AstcencImageData, AstcencProfile, AstcencSwizzle, AstcencSwz,
};
use crate::defs::SupercmpScheme;
use crate::dfd::khr_df::*;
use crate::dfd::queries::get_dfd_component_info_unpacked;
use crate::dfd::{
    CHANNELID, FLAGS, MODEL, QUALIFIERS, TEXELBLOCKDIMENSION0, TEXELBLOCKDIMENSION1,
    TEXELBLOCKDIMENSION2, TRANSFER, bdb, bdb_mut, dfdsetval, dfdsval, dfdval,
};
use crate::encode_params::*;
use crate::error::{ErrorCode, KtxResult};
use crate::formatsize::KTX_FORMAT_SIZE_PACKED_BIT;
use crate::texture::{FormatVersion, KtxTexture, TextureCreateInfo, TextureCreateStorage};
use crate::texture2::Texture2;
use crate::vkformat::VkFormat;

/// `mapAstcError`. The C++ asserts on libktx misuse; those cases map to the same codes.
/// `launchThreads` + `compressionWorkloadRunner`: runs `func(thread_id)` on `thread_count`
/// threads (directly on this thread when single threaded) and returns an error if any thread
/// reported one.
fn launch_threads(
    thread_count: u32,
    func: impl Fn(u32) -> Result<(), astcenc::AstcencError> + Sync,
) -> Result<(), astcenc::AstcencError> {
    if thread_count <= 1 {
        return func(0);
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..thread_count)
            .map(|thread_id| {
                let func = &func;
                s.spawn(move || func(thread_id))
            })
            .collect();
        let mut result = Ok(());
        for h in handles {
            match h.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => result = Err(e),
                Err(_) => result = Err(astcenc::AstcencError::BadContext),
            }
        }
        result
    })
}

fn map_astc_error(astc_error: AstcencError) -> ErrorCode {
    match astc_error {
        AstcencError::OutOfMem => ErrorCode::OutOfMemory,
        AstcencError::BadBlockSize
        | AstcencError::BadDecodeMode
        | AstcencError::BadFlags
        | AstcencError::BadParam
        | AstcencError::BadProfile
        | AstcencError::BadQuality
        | AstcencError::BadSwizzle => ErrorCode::InvalidValue,
        AstcencError::NotImplemented => ErrorCode::UnsupportedFeature,
        AstcencError::Success | AstcencError::BadContext | AstcencError::BadCpuFloat => {
            ErrorCode::InvalidOperation
        }
    }
}

/// `astcProfile(sRGB, ldr)`.
fn astc_profile(srgb: bool, ldr: bool) -> AstcencProfile {
    if srgb && ldr {
        AstcencProfile::LdrSrgb
    } else if !srgb {
        if ldr {
            AstcencProfile::Ldr
        } else {
            AstcencProfile::Hdr
        }
    } else {
        // HDR sRGB is not supported
        AstcencProfile::LdrSrgb
    }
}

/// `isFormatAstcLDR`.
fn is_format_astc_ldr(this: &Texture2) -> bool {
    dfdsval(bdb(&this.p_dfd), 0, QUALIFIERS) & KHR_DF_SAMPLE_DATATYPE_FLOAT == 0
}

/// `getUncompressedFormat`: RGBA8, SRGBA8 or RGBA16F.
fn get_uncompressed_format(this: &Texture2) -> VkFormat {
    let b = bdb(&this.p_dfd);
    if dfdsval(b, 0, QUALIFIERS) & KHR_DF_SAMPLE_DATATYPE_FLOAT != 0 {
        VkFormat::R16G16B16A16_SFLOAT
    } else if dfdval(b, TRANSFER) == KHR_DF_TRANSFER_SRGB {
        VkFormat::R8G8B8A8_SRGB
    } else {
        VkFormat::R8G8B8A8_UNORM
    }
}

/// `astcDefaultOptions`.
fn astc_default_options() -> AstcParams {
    AstcParams {
        struct_size: KTX_ASTC_PARAMS_SIZE,
        thread_count: 1,
        block_dimension: KTX_PACK_ASTC_BLOCK_DIMENSION_6X6,
        mode: KTX_PACK_ASTC_ENCODER_MODE_LDR,
        quality_level: KTX_PACK_ASTC_QUALITY_LEVEL_MEDIUM,
        normal_map: false,
        ..AstcParams::default()
    }
}

/// ASTC block dimensions (x, y, z) and VkFormat bases indexed by `ktx_pack_astc_block_dimension_e`.
const BLOCKS: [((u32, u32, u32), VkFormat, VkFormat); 24] = [
    (
        (4, 4, 1),
        VkFormat::ASTC_4x4_UNORM_BLOCK,
        VkFormat::ASTC_4x4_SRGB_BLOCK,
    ),
    (
        (5, 4, 1),
        VkFormat::ASTC_5x4_UNORM_BLOCK,
        VkFormat::ASTC_5x4_SRGB_BLOCK,
    ),
    (
        (5, 5, 1),
        VkFormat::ASTC_5x5_UNORM_BLOCK,
        VkFormat::ASTC_5x5_SRGB_BLOCK,
    ),
    (
        (6, 5, 1),
        VkFormat::ASTC_6x5_UNORM_BLOCK,
        VkFormat::ASTC_6x5_SRGB_BLOCK,
    ),
    (
        (6, 6, 1),
        VkFormat::ASTC_6x6_UNORM_BLOCK,
        VkFormat::ASTC_6x6_SRGB_BLOCK,
    ),
    (
        (8, 5, 1),
        VkFormat::ASTC_8x5_UNORM_BLOCK,
        VkFormat::ASTC_8x5_SRGB_BLOCK,
    ),
    (
        (8, 6, 1),
        VkFormat::ASTC_8x6_UNORM_BLOCK,
        VkFormat::ASTC_8x6_SRGB_BLOCK,
    ),
    (
        (10, 5, 1),
        VkFormat::ASTC_10x5_UNORM_BLOCK,
        VkFormat::ASTC_10x5_SRGB_BLOCK,
    ),
    (
        (10, 6, 1),
        VkFormat::ASTC_10x6_UNORM_BLOCK,
        VkFormat::ASTC_10x6_SRGB_BLOCK,
    ),
    (
        (8, 8, 1),
        VkFormat::ASTC_8x8_UNORM_BLOCK,
        VkFormat::ASTC_8x8_SRGB_BLOCK,
    ),
    (
        (10, 8, 1),
        VkFormat::ASTC_10x8_UNORM_BLOCK,
        VkFormat::ASTC_10x8_SRGB_BLOCK,
    ),
    (
        (10, 10, 1),
        VkFormat::ASTC_10x10_UNORM_BLOCK,
        VkFormat::ASTC_10x10_SRGB_BLOCK,
    ),
    (
        (12, 10, 1),
        VkFormat::ASTC_12x10_UNORM_BLOCK,
        VkFormat::ASTC_12x10_SRGB_BLOCK,
    ),
    (
        (12, 12, 1),
        VkFormat::ASTC_12x12_UNORM_BLOCK,
        VkFormat::ASTC_12x12_SRGB_BLOCK,
    ),
    (
        (3, 3, 3),
        VkFormat::ASTC_3x3x3_UNORM_BLOCK_EXT,
        VkFormat::ASTC_3x3x3_SRGB_BLOCK_EXT,
    ),
    (
        (4, 3, 3),
        VkFormat::ASTC_4x3x3_UNORM_BLOCK_EXT,
        VkFormat::ASTC_4x3x3_SRGB_BLOCK_EXT,
    ),
    (
        (4, 4, 3),
        VkFormat::ASTC_4x4x3_UNORM_BLOCK_EXT,
        VkFormat::ASTC_4x4x3_SRGB_BLOCK_EXT,
    ),
    (
        (4, 4, 4),
        VkFormat::ASTC_4x4x4_UNORM_BLOCK_EXT,
        VkFormat::ASTC_4x4x4_SRGB_BLOCK_EXT,
    ),
    (
        (5, 4, 4),
        VkFormat::ASTC_5x4x4_UNORM_BLOCK_EXT,
        VkFormat::ASTC_5x4x4_SRGB_BLOCK_EXT,
    ),
    (
        (5, 5, 4),
        VkFormat::ASTC_5x5x4_UNORM_BLOCK_EXT,
        VkFormat::ASTC_5x5x4_SRGB_BLOCK_EXT,
    ),
    (
        (5, 5, 5),
        VkFormat::ASTC_5x5x5_UNORM_BLOCK_EXT,
        VkFormat::ASTC_5x5x5_SRGB_BLOCK_EXT,
    ),
    (
        (6, 5, 5),
        VkFormat::ASTC_6x5x5_UNORM_BLOCK_EXT,
        VkFormat::ASTC_6x5x5_SRGB_BLOCK_EXT,
    ),
    (
        (6, 6, 5),
        VkFormat::ASTC_6x6x5_UNORM_BLOCK_EXT,
        VkFormat::ASTC_6x6x5_SRGB_BLOCK_EXT,
    ),
    (
        (6, 6, 6),
        VkFormat::ASTC_6x6x6_UNORM_BLOCK_EXT,
        VkFormat::ASTC_6x6x6_SRGB_BLOCK_EXT,
    ),
];

/// `astcVkFormat`: defaults to 6x6 sRGB.
fn astc_vk_format(block_size: u32, srgb: bool) -> VkFormat {
    match BLOCKS.get(block_size as usize) {
        Some(&(_, unorm, srgb_fmt)) => {
            if srgb {
                srgb_fmt
            } else {
                unorm
            }
        }
        None => VkFormat::ASTC_6x6_SRGB_BLOCK,
    }
}

/// `astcBlockDimensions`: defaults to 6x6.
fn astc_block_dimensions(block_size: u32) -> (u32, u32, u32) {
    BLOCKS.get(block_size as usize).map_or((6, 6, 1), |b| b.0)
}

/// `astcQuality`.
fn astc_quality(quality_level: u32) -> f32 {
    match quality_level {
        KTX_PACK_ASTC_QUALITY_LEVEL_FASTEST => 0.0,
        KTX_PACK_ASTC_QUALITY_LEVEL_FAST => 10.0,
        KTX_PACK_ASTC_QUALITY_LEVEL_MEDIUM => 60.0,
        KTX_PACK_ASTC_QUALITY_LEVEL_THOROUGH => 98.0,
        KTX_PACK_ASTC_QUALITY_LEVEL_EXHAUSTIVE => 100.0,
        _ => 60.0,
    }
}

/// `astcSwizzle`: an empty `inputSwizzle` means no swizzle (or the normal map one).
fn astc_swizzle(params: &AstcParams) -> AstcencSwizzle {
    let mut swizzle = AstcencSwizzle::RGBA;
    if params.input_swizzle[0] != 0 {
        let channels = [
            &mut swizzle.r,
            &mut swizzle.g,
            &mut swizzle.b,
            &mut swizzle.a,
        ];
        for (c, s) in params.input_swizzle.iter().zip(channels) {
            match c {
                b'r' => *s = AstcencSwz::R,
                b'g' => *s = AstcencSwz::G,
                b'b' => *s = AstcencSwz::B,
                b'a' => *s = AstcencSwz::A,
                b'0' => *s = AstcencSwz::Zero,
                b'1' => *s = AstcencSwz::One,
                _ => {}
            }
        }
    } else if params.normal_map {
        return AstcencSwizzle {
            r: AstcencSwz::R,
            g: AstcencSwz::R,
            b: AstcencSwz::R,
            a: AstcencSwz::G,
        };
    }
    swizzle
}

/// `unorm8x{1,2,3,4}ArrayToImage`: expands an 8-bit image with `num_components`
/// channels to RGBA (L -> LLL1, LA -> LLLA, RGB -> RGB1).
fn unorm8_array_to_image(
    data: &[u8],
    num_components: usize,
    dim_x: u32,
    dim_y: u32,
) -> AstcencImage {
    let n = (dim_x * dim_y) as usize;
    let mut out = vec![0u8; n * 4];
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let src = |c: usize| data.get(num_components * i + c).copied().unwrap_or(0);
        match num_components {
            1 => px.copy_from_slice(&[src(0), src(0), src(0), 255]),
            2 => px.copy_from_slice(&[src(0), src(0), src(0), src(1)]),
            3 => px.copy_from_slice(&[src(0), src(1), src(2), 255]),
            _ => px.copy_from_slice(&[src(0), src(1), src(2), src(3)]),
        }
    }
    AstcencImage {
        dim_x,
        dim_y,
        dim_z: 1,
        data: AstcencImageData::U8(vec![out]),
    }
}

impl Texture2 {
    /// Port of `ktxTexture2_CompressAstcEx`: encodes the uncompressed 8-bit images to
    /// ASTC, replacing the images, format and DFD of the texture.
    pub fn compress_astc_ex(&mut self, params: &AstcParams) -> KtxResult<()> {
        if params.struct_size != KTX_ASTC_PARAMS_SIZE {
            return Err(ErrorCode::InvalidValue);
        }
        if self.generate_mipmaps {
            return Err(ErrorCode::InvalidOperation);
        }
        if self.supercompression_scheme != SupercmpScheme::NONE {
            return Err(ErrorCode::InvalidOperation); // Can't apply multiple schemes.
        }
        if self.is_compressed {
            // Only non-block compressed formats can be encoded into an ASTC format.
            return Err(ErrorCode::InvalidOperation);
        }
        if self.format_size().flags & KTX_FORMAT_SIZE_PACKED_BIT != 0 {
            return Err(ErrorCode::InvalidOperation);
        }

        let (num_components, component_size) = get_dfd_component_info_unpacked(&self.p_dfd);
        let b = bdb(&self.p_dfd);
        let transfer = dfdval(b, TRANSFER);
        let srgb = transfer == KHR_DF_TRANSFER_SRGB;
        let alpha_mode = dfdval(b, FLAGS);

        if component_size != 1 {
            // Can only deal with 8-bit components at the moment
            return Err(ErrorCode::UnsupportedFeature);
        }

        let mode = if params.mode == KTX_PACK_ASTC_ENCODER_MODE_DEFAULT {
            // component_size is always 1 here
            KTX_PACK_ASTC_ENCODER_MODE_LDR
        } else {
            params.mode
        };

        if mode == KTX_PACK_ASTC_ENCODER_MODE_HDR && srgb {
            return Err(ErrorCode::InvalidOperation);
        }

        if !(srgb || transfer == KHR_DF_TRANSFER_LINEAR) {
            return Err(ErrorCode::InvalidOperation);
        }

        if self.p_data.is_none() {
            self.load_image_data(None)?;
        }

        let thread_count = params.thread_count.max(1);

        let vk_format = astc_vk_format(params.block_dimension, srgb);

        // numLevels = 0 is not allowed for block compressed formats
        self.num_levels = self.num_levels.max(1);

        // Prototype texture for sizes, data allocation and the target DFD.
        let create_info = TextureCreateInfo {
            gl_internalformat: 0,
            vk_format,
            dfd: None,
            base_width: self.base_width,
            base_height: self.base_height,
            base_depth: self.base_depth,
            generate_mipmaps: self.generate_mipmaps,
            is_array: self.is_array,
            num_dimensions: self.num_dimensions,
            num_faces: self.num_faces,
            num_layers: self.num_layers,
            num_levels: self.num_levels,
        };
        let mut prototype = Texture2::create(&create_info, TextureCreateStorage::AllocStorage)?;

        let mut flags = if params.normal_map {
            astcenc::ASTCENC_FLG_MAP_NORMAL
        } else {
            0
        };
        let (block_size_x, block_size_y, block_size_z) =
            astc_block_dimensions(params.block_dimension);
        let quality = astc_quality(params.quality_level);
        // The C++ passes the encoder mode as the `ldr` flag, so any mode selects LDR.
        let profile = astc_profile(srgb, mode != 0);
        let swizzle = astc_swizzle(params);
        if params.perceptual {
            flags |= astcenc::ASTCENC_FLG_USE_PERCEPTUAL;
        }

        let astc_config = astcenc::astcenc_config_init(
            profile,
            block_size_x,
            block_size_y,
            block_size_z,
            quality,
            flags,
        )
        .map_err(map_astc_error)?;

        let mut astc_context =
            astcenc::astcenc_context_alloc(&astc_config, thread_count).map_err(map_astc_error)?;

        let out_image_sizes: Vec<usize> = (0..self.num_levels)
            .map(|l| prototype.calc_image_size(l, FormatVersion::Two))
            .collect();
        let Some(proto_data) = prototype.p_data.as_mut() else {
            return Err(ErrorCode::OutOfMemory);
        };
        let Some(in_data) = self.base.p_data.as_ref() else {
            return Err(ErrorCode::InvalidOperation);
        };

        // Walk in reverse on levels so the output is written in file order
        let mut buffer_out = 0usize;
        for level in (0..self.num_levels).rev() {
            let width = (self.base_width >> level).max(1);
            let height = (self.base_height >> level).max(1);
            let depth = (self.base_depth >> level).max(1);

            let level_images = self.num_layers * self.num_faces * depth;
            let level_image_size_in = self.calc_image_size(level, FormatVersion::Two);
            let level_image_size_out = out_image_sizes[level as usize];

            let mut offset = self.level_data_offset(level) as usize;
            for _ in 0..level_images {
                let input_image = unorm8_array_to_image(
                    in_data.get(offset..).unwrap_or_default(),
                    num_components as usize,
                    width,
                    height,
                );

                let Some(out) = proto_data.get_mut(buffer_out..buffer_out + level_image_size_out)
                else {
                    return Err(ErrorCode::OutOfMemory);
                };
                let work = std::sync::Mutex::new(out);
                let result = launch_threads(thread_count, |thread_id| {
                    astcenc::astcenc_compress_image(
                        &astc_context,
                        &input_image,
                        &swizzle,
                        &work,
                        thread_id,
                    )
                });
                buffer_out += level_image_size_out;

                // Reset ASTC context for next image
                let _ = astcenc::astcenc_compress_reset(&mut astc_context);

                offset += level_image_size_in;
                result.map_err(map_astc_error)?;
            }
        }

        // We are done with astcencoder
        astcenc::astcenc_context_free(astc_context);

        // Fix up the current texture
        self.base.protected.format_size = prototype.base.protected.format_size;
        self.vk_format = vk_format;
        self.base.protected.type_size = prototype.base.protected.type_size;
        self.base.is_compressed = prototype.base.is_compressed;
        self.supercompression_scheme = SupercmpScheme::NONE;
        self.private_mut().required_level_alignment = prototype.private().required_level_alignment;
        self.private_mut().level_index = prototype.level_index().to_vec();
        self.p_dfd = std::mem::take(&mut prototype.p_dfd);
        self.base.p_data = prototype.base.p_data.take();
        self.base.data_size = prototype.base.data_size;
        // Restore alphaMode flags
        dfdsetval(bdb_mut(&mut self.p_dfd), FLAGS, alpha_mode);
        Ok(())
    }

    /// Port of `ktxTexture2_CompressAstc`: default options at a quality of 0 - 100.
    pub fn compress_astc(&mut self, quality: u32) -> KtxResult<()> {
        let mut params = astc_default_options();
        for level in [
            KTX_PACK_ASTC_QUALITY_LEVEL_FASTEST,
            KTX_PACK_ASTC_QUALITY_LEVEL_FAST,
            KTX_PACK_ASTC_QUALITY_LEVEL_MEDIUM,
            KTX_PACK_ASTC_QUALITY_LEVEL_THOROUGH,
            KTX_PACK_ASTC_QUALITY_LEVEL_EXHAUSTIVE,
        ] {
            if quality >= level {
                params.quality_level = level;
            }
        }
        self.compress_astc_ex(&params)
    }

    /// Port of `ktxTexture2_DecodeAstc`: decodes ASTC images to RGBA8, SRGBA8 or RGBA16F.
    pub fn decode_astc(&mut self) -> KtxResult<()> {
        let b = bdb(&self.p_dfd);
        let color_model = dfdval(b, MODEL);
        if !(color_model == KHR_DF_MODEL_ASTC || color_model == KHR_DF_MODEL_UASTC_HDR_4x4) {
            return Err(ErrorCode::InvalidOperation); // Not in astc decodable format
        }
        if self.supercompression_scheme == SupercmpScheme::BASIS_LZ
            || self.supercompression_scheme == SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE
        {
            return Err(ErrorCode::FileDataError); // Not a valid file.
        }
        // Safety check.
        if self.supercompression_scheme.0 > SupercmpScheme::END_RANGE.0 {
            return Err(ErrorCode::UnsupportedFeature); // Unsupported scheme.
        }
        if dfdsval(b, 0, CHANNELID) != KHR_DF_CHANNEL_ASTC_DATA {
            return Err(ErrorCode::FileDataError);
        }

        let vk_format = get_uncompressed_format(self);

        let create_info = TextureCreateInfo {
            gl_internalformat: 0,
            vk_format,
            dfd: None,
            base_width: self.base_width,
            base_height: self.base_height,
            base_depth: self.base_depth,
            generate_mipmaps: self.generate_mipmaps,
            is_array: self.is_array,
            num_dimensions: self.num_dimensions,
            num_faces: self.num_faces,
            num_layers: self.num_layers,
            num_levels: self.num_levels,
        };
        let mut prototype = Texture2::create(&create_info, TextureCreateStorage::AllocStorage)?;

        if self.p_data.is_none() {
            if self.is_active_stream() {
                // Load pending. Complete it.
                self.load_image_data(None)?;
            } else {
                // No data to decode.
                return Err(ErrorCode::InvalidOperation);
            }
        }

        let b = bdb(&self.p_dfd);
        let block_size_x = dfdval(b, TEXELBLOCKDIMENSION0) + 1;
        let block_size_y = dfdval(b, TEXELBLOCKDIMENSION1) + 1;
        let block_size_z = dfdval(b, TEXELBLOCKDIMENSION2) + 1;
        let transfer = dfdval(b, TRANSFER);
        let ldr = is_format_astc_ldr(self);
        let profile = astc_profile(transfer == KHR_DF_TRANSFER_SRGB, ldr);

        let astc_config = astcenc::astcenc_config_init(
            profile,
            block_size_x,
            block_size_y,
            block_size_z,
            60.0,
            0,
        )
        .map_err(map_astc_error)?;
        let mut astc_context =
            astcenc::astcenc_context_alloc(&astc_config, 1).map_err(map_astc_error)?;

        let swizzle = AstcencSwizzle::RGBA;
        for level_index in 0..self.num_levels {
            let image_width = (self.base_width >> level_index).max(1);
            let image_height = (self.base_height >> level_index).max(1);
            let image_depths = (self.base_depth >> level_index).max(1);
            for layer_index in 0..self.num_layers {
                for face_index in 0..self.num_faces {
                    for depth_slice_index in 0..image_depths {
                        let level_image_size_in =
                            self.calc_image_size(level_index, FormatVersion::Two);
                        let offset_in = self.get_image_offset(
                            level_index,
                            layer_index,
                            face_index + depth_slice_index,
                        )?;
                        let offset_out = prototype.get_image_offset(
                            level_index,
                            layer_index,
                            face_index + depth_slice_index,
                        )?;
                        let data_in = self
                            .base
                            .p_data
                            .as_ref()
                            .and_then(|d| d.get(offset_in..offset_in + level_image_size_in))
                            .ok_or(ErrorCode::InvalidOperation)?;

                        // The C++ decodes all depth slices of the level into the image
                        let texels = (image_width * image_height * image_depths) as usize * 4;
                        let plane = (image_width * image_height) as usize * 4;
                        let mut image_out = AstcencImage {
                            dim_x: image_width,
                            dim_y: image_height,
                            dim_z: image_depths,
                            data: if ldr {
                                AstcencImageData::U8(vec![vec![0; plane]; image_depths as usize])
                            } else {
                                AstcencImageData::F16(vec![vec![0; plane]; image_depths as usize])
                            },
                        };
                        let result = astcenc::astcenc_decompress_image(
                            &mut astc_context,
                            data_in,
                            &mut image_out,
                            &swizzle,
                            0,
                        );

                        // Reset ASTC context for next image
                        let _ = astcenc::astcenc_decompress_reset(&mut astc_context);
                        result.map_err(map_astc_error)?;

                        let bytes: Vec<u8> = match &image_out.data {
                            AstcencImageData::U8(p) => p.concat(),
                            AstcencImageData::F16(p) => {
                                p.concat().iter().flat_map(|h| h.to_le_bytes()).collect()
                            }
                            AstcencImageData::F32(_) => Vec::new(),
                        };
                        let texel_bytes = if ldr { 1 } else { 2 };
                        let proto_data = prototype.p_data.as_mut().ok_or(ErrorCode::OutOfMemory)?;
                        // Copy what fits in the prototype from this image offset onwards
                        let n = (texels * texel_bytes)
                            .min(proto_data.len().saturating_sub(offset_out))
                            .min(bytes.len());
                        if let (Some(dst), Some(src)) = (
                            proto_data.get_mut(offset_out..offset_out + n),
                            bytes.get(..n),
                        ) {
                            dst.copy_from_slice(src);
                        }
                    }
                }
            }
        }

        // We are done with astcdecoder
        astcenc::astcenc_context_free(astc_context);

        // Fix up the current texture
        self.base.protected.format_size = prototype.base.protected.format_size;
        self.vk_format = vk_format;
        self.base.protected.type_size = prototype.base.protected.type_size;
        self.base.is_compressed = prototype.base.is_compressed;
        self.supercompression_scheme = SupercmpScheme::NONE;
        self.private_mut().required_level_alignment = prototype.private().required_level_alignment;
        self.private_mut().level_index = prototype.level_index().to_vec();
        self.p_dfd = std::mem::take(&mut prototype.p_dfd);
        self.base.p_data = prototype.base.p_data.take();
        self.base.data_size = prototype.base.data_size;
        self.private_mut().sgd_byte_length = 0;
        self.private_mut().supercompression_global_data = Vec::new();
        Ok(())
    }
}
