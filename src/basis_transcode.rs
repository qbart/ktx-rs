#![allow(non_upper_case_globals)]
//! Port of lib/src/basis_transcode.cpp: `ktxTexture2_TranscodeBasis`.

use crate::basisu::etc1s::{LowlevelEtc1sTranscoder, TranscodeStatus};
use crate::basisu::uastc_hdr::LowlevelUastcHdr4x4Transcoder;
use crate::basisu::uastc_hdr_6x6i::LowlevelUastcHdr6x6IntermediateTranscoder;
use crate::basisu::uastc_ldr::LowlevelUastcLdr4x4Transcoder;
use crate::basisu::{
    BasisTexFormat, TranscoderState, TranscoderTextureFormat, basis_is_format_supported,
};
use crate::defs::{SupercmpScheme, TranscodeFmt, transcode_flags};
use crate::dfd::khr_df::*;
use crate::dfd::{CHANNELID, MODEL, TRANSFER, bdb, dfdsamplecount, dfdsval, dfdval, vk2dfd};
use crate::error::{ErrorCode, KtxResult};
use crate::texture::{FormatVersion, KtxTexture, TextureCreateInfo, TextureCreateStorage};
use crate::texture2::Texture2;
use crate::vkformat::VkFormat;

/// Port of `alpha_content_e` (basis_sgd.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaContent {
    None,
    Alpha,
    Green,
}

/// Size of `ktxBasisLzGlobalHeader`.
pub const BASIS_LZ_GLOBAL_HEADER_SIZE: usize = 20;
/// Size of `ktxBasisLzEtc1sImageDesc`.
pub const BASIS_LZ_ETC1S_IMAGE_DESC_SIZE: usize = 20;

/// Port of `ktxBasisLzGlobalHeader`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BasisLzGlobalHeader {
    pub endpoint_count: u16,
    pub selector_count: u16,
    pub endpoints_byte_length: u32,
    pub selectors_byte_length: u32,
    pub tables_byte_length: u32,
    pub extended_byte_length: u32,
}

impl BasisLzGlobalHeader {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let b = b.get(..BASIS_LZ_GLOBAL_HEADER_SIZE)?;
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        Some(BasisLzGlobalHeader {
            endpoint_count: u16_at(0),
            selector_count: u16_at(2),
            endpoints_byte_length: u32_at(4),
            selectors_byte_length: u32_at(8),
            tables_byte_length: u32_at(12),
            extended_byte_length: u32_at(16),
        })
    }
}

/// Port of `ktxBasisLzEtc1sImageDesc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BasisLzEtc1sImageDesc {
    pub image_flags: u32,
    pub rgb_slice_byte_offset: u32,
    pub rgb_slice_byte_length: u32,
    pub alpha_slice_byte_offset: u32,
    pub alpha_slice_byte_length: u32,
}

impl BasisLzEtc1sImageDesc {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let b = b.get(..BASIS_LZ_ETC1S_IMAGE_DESC_SIZE)?;
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        Some(BasisLzEtc1sImageDesc {
            image_flags: u32_at(0),
            rgb_slice_byte_offset: u32_at(4),
            rgb_slice_byte_length: u32_at(8),
            alpha_slice_byte_offset: u32_at(12),
            alpha_slice_byte_length: u32_at(16),
        })
    }
}

/// Port of `ktx2transcoderFormat`.
fn ktx2transcoder_format(fmt: TranscodeFmt) -> TranscoderTextureFormat {
    match fmt {
        TranscodeFmt::ASTC_HDR_4x4_RGBA => TranscoderTextureFormat::ASTC_HDR_4x4_RGBA,
        TranscodeFmt::ASTC_HDR_6x6_RGBA => TranscoderTextureFormat::ASTC_HDR_6x6_RGBA,
        TranscodeFmt::BC6HU_RGB => TranscoderTextureFormat::BC6H,
        f => TranscoderTextureFormat(f.0),
    }
}

/// Size of `ktxUASTCHDR6x6IntermediateImageDesc`.
pub const UASTC_HDR_6X6_INTERMEDIATE_IMAGE_DESC_SIZE: usize = 12;

/// Port of `ktxUASTCHDR6x6IntermediateImageDesc` (offsets relative to the
/// start of the mip level's data).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UastcHdr6x6IntermediateImageDesc {
    pub rgb_slice_byte_offset: u32,
    pub rgb_slice_byte_length: u32,
    pub rgb_slice_type: u32,
}

impl UastcHdr6x6IntermediateImageDesc {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let b = b.get(..UASTC_HDR_6X6_INTERMEDIATE_IMAGE_DESC_SIZE)?;
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        Some(UastcHdr6x6IntermediateImageDesc {
            rgb_slice_byte_offset: u32_at(0),
            rgb_slice_byte_length: u32_at(4),
            rgb_slice_type: u32_at(8),
        })
    }
}

impl Texture2 {
    /// Port of `ktxTexture2_TranscodeBasis`.
    pub fn transcode_basis(
        &mut self,
        mut output_format: TranscodeFmt,
        transcode_flags_in: u32,
    ) -> KtxResult<()> {
        let color_model = dfdval(bdb(&self.p_dfd), MODEL);
        if color_model != KHR_DF_MODEL_UASTC
            && color_model != KHR_DF_MODEL_UASTC_HDR_4x4
            && color_model != KHR_DF_MODEL_UASTC_HDR_6x6
            && self.supercompression_scheme != SupercmpScheme::BASIS_LZ
        {
            return Err(ErrorCode::InvalidOperation); // Not in a transcodable format.
        }
        if (self.supercompression_scheme == SupercmpScheme::BASIS_LZ
            || self.supercompression_scheme == SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE)
            && self.supercompression_global_data().is_empty()
        {
            return Err(ErrorCode::InvalidOperation);
        }

        // Early exit for redundant transcode from UASTC HDR 4x4 to ASTC 4x4.
        if color_model == KHR_DF_MODEL_UASTC_HDR_4x4
            && output_format == TranscodeFmt::ASTC_HDR_4x4_RGBA
        {
            let new_dfd = vk2dfd(VkFormat::ASTC_4x4_SFLOAT_BLOCK).ok_or(ErrorCode::OutOfMemory)?;
            self.p_dfd = new_dfd;
            self.vk_format = VkFormat::ASTC_4x4_SFLOAT_BLOCK;
            return Ok(());
        }

        if transcode_flags_in & transcode_flags::PVRTC_DECODE_TO_NEXT_POW2 != 0 {
            return Err(ErrorCode::UnsupportedFeature);
        }
        if (output_format == TranscodeFmt::PVRTC1_4_RGB
            || output_format == TranscodeFmt::PVRTC1_4_RGBA)
            && (!self.base_width.is_power_of_two() || !self.base_height.is_power_of_two())
        {
            return Err(ErrorCode::InvalidOperation);
        }

        let b = bdb(&self.p_dfd);
        let srgb = dfdval(b, TRANSFER) == KHR_DF_TRANSFER_SRGB;
        let mut alpha_content = AlphaContent::None;
        if color_model == KHR_DF_MODEL_ETC1S {
            if dfdsamplecount(b) == 2 {
                match dfdsval(b, 1, CHANNELID) {
                    KHR_DF_CHANNEL_ETC1S_AAA => alpha_content = AlphaContent::Alpha,
                    KHR_DF_CHANNEL_ETC1S_GGG => alpha_content = AlphaContent::Green,
                    _ => return Err(ErrorCode::FileDataError),
                }
            }
        } else {
            match dfdsval(b, 0, CHANNELID) {
                KHR_DF_CHANNEL_UASTC_RGBA => alpha_content = AlphaContent::Alpha,
                KHR_DF_CHANNEL_UASTC_RRRG => alpha_content = AlphaContent::Green,
                _ => {}
            }
        }

        // Do some format mapping.
        let has_alpha = alpha_content != AlphaContent::None;
        output_format = match output_format {
            TranscodeFmt::BC1_OR_3 => {
                if has_alpha {
                    TranscodeFmt::BC3_RGBA
                } else {
                    TranscodeFmt::BC1_RGB
                }
            }
            TranscodeFmt::ETC => {
                if has_alpha {
                    TranscodeFmt::ETC2_RGBA
                } else {
                    TranscodeFmt::ETC1_RGB
                }
            }
            TranscodeFmt::PVRTC1_4_RGBA => {
                if has_alpha {
                    TranscodeFmt::PVRTC1_4_RGBA
                } else {
                    TranscodeFmt::PVRTC1_4_RGB
                }
            }
            TranscodeFmt::PVRTC2_4_RGBA => {
                if has_alpha {
                    TranscodeFmt::PVRTC2_4_RGBA
                } else {
                    TranscodeFmt::PVRTC2_4_RGB
                }
            }
            f => f,
        };

        let pick = |s: VkFormat, u: VkFormat| if srgb { s } else { u };
        let vk_format = match output_format {
            TranscodeFmt::ETC1_RGB => pick(
                VkFormat::ETC2_R8G8B8_SRGB_BLOCK,
                VkFormat::ETC2_R8G8B8_UNORM_BLOCK,
            ),
            TranscodeFmt::ETC2_RGBA => pick(
                VkFormat::ETC2_R8G8B8A8_SRGB_BLOCK,
                VkFormat::ETC2_R8G8B8A8_UNORM_BLOCK,
            ),
            TranscodeFmt::ETC2_EAC_R11 => VkFormat::EAC_R11_UNORM_BLOCK,
            TranscodeFmt::ETC2_EAC_RG11 => VkFormat::EAC_R11G11_UNORM_BLOCK,
            TranscodeFmt::BC1_RGB => {
                pick(VkFormat::BC1_RGB_SRGB_BLOCK, VkFormat::BC1_RGB_UNORM_BLOCK)
            }
            TranscodeFmt::BC3_RGBA => pick(VkFormat::BC3_SRGB_BLOCK, VkFormat::BC3_UNORM_BLOCK),
            TranscodeFmt::BC4_R => VkFormat::BC4_UNORM_BLOCK,
            TranscodeFmt::BC5_RG => VkFormat::BC5_UNORM_BLOCK,
            TranscodeFmt::PVRTC1_4_RGB | TranscodeFmt::PVRTC1_4_RGBA => pick(
                VkFormat::PVRTC1_4BPP_SRGB_BLOCK_IMG,
                VkFormat::PVRTC1_4BPP_UNORM_BLOCK_IMG,
            ),
            TranscodeFmt::PVRTC2_4_RGB | TranscodeFmt::PVRTC2_4_RGBA => pick(
                VkFormat::PVRTC2_4BPP_SRGB_BLOCK_IMG,
                VkFormat::PVRTC2_4BPP_UNORM_BLOCK_IMG,
            ),
            TranscodeFmt::BC7_RGBA => pick(VkFormat::BC7_SRGB_BLOCK, VkFormat::BC7_UNORM_BLOCK),
            TranscodeFmt::ASTC_4x4_RGBA => pick(
                VkFormat::ASTC_4x4_SRGB_BLOCK,
                VkFormat::ASTC_4x4_UNORM_BLOCK,
            ),
            TranscodeFmt::ASTC_HDR_4x4_RGBA => VkFormat::ASTC_4x4_SFLOAT_BLOCK,
            TranscodeFmt::ASTC_HDR_6x6_RGBA => VkFormat::ASTC_6x6_SFLOAT_BLOCK,
            TranscodeFmt::BC6HU_RGB => VkFormat::BC6H_UFLOAT_BLOCK,
            TranscodeFmt::RGB565 => VkFormat::R5G6B5_UNORM_PACK16,
            TranscodeFmt::BGR565 => VkFormat::B5G6R5_UNORM_PACK16,
            TranscodeFmt::RGBA4444 => VkFormat::R4G4B4A4_UNORM_PACK16,
            TranscodeFmt::RGBA32 => pick(VkFormat::R8G8B8A8_SRGB, VkFormat::R8G8B8A8_UNORM),
            TranscodeFmt::RGB_HALF => VkFormat::R16G16B16_SFLOAT,
            TranscodeFmt::RGBA_HALF => VkFormat::R16G16B16A16_SFLOAT,
            TranscodeFmt::RGB_9E5 => VkFormat::E5B9G9R9_UFLOAT_PACK32,
            _ => return Err(ErrorCode::InvalidValue),
        };

        let texture_format = match color_model {
            KHR_DF_MODEL_UASTC => BasisTexFormat::UastcLdr4x4,
            KHR_DF_MODEL_UASTC_HDR_4x4 => BasisTexFormat::UastcHdr4x4,
            KHR_DF_MODEL_UASTC_HDR_6x6 => BasisTexFormat::UastcHdr6x6Intermediate,
            _ => BasisTexFormat::Etc1s,
        };
        if !basis_is_format_supported(ktx2transcoder_format(output_format), texture_format) {
            return Err(ErrorCode::UnsupportedFeature);
        }

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

        if self.p_data.is_none() {
            if self.is_active_stream() {
                self.load_image_data(None)?;
            } else {
                return Err(ErrorCode::InvalidOperation);
            }
        }

        if texture_format == BasisTexFormat::Etc1s {
            self.transcode_lz_etc1s(
                alpha_content,
                &mut prototype,
                output_format,
                transcode_flags_in,
            )?;
        } else {
            self.transcode_uastc(
                color_model,
                alpha_content,
                &mut prototype,
                output_format,
                transcode_flags_in,
            )?;
        }

        // Fix up the current texture.
        self.base.protected.format_size = prototype.base.protected.format_size;
        self.vk_format = vk_format;
        self.base.protected.type_size = prototype.base.protected.type_size;
        self.base.is_compressed = prototype.base.is_compressed;
        self.supercompression_scheme = SupercmpScheme::NONE;
        self.private.required_level_alignment = prototype.private.required_level_alignment;
        self.private.level_index = prototype.private.level_index.clone();
        self.p_dfd = std::mem::take(&mut prototype.p_dfd);
        self.base.p_data = prototype.base.p_data.take();
        self.base.data_size = prototype.base.data_size;
        self.private.sgd_byte_length = 0;
        self.private.supercompression_global_data = Vec::new();
        Ok(())
    }

    /// Port of `ktxTexture2_transcodeLzEtc1s`.
    fn transcode_lz_etc1s(
        &self,
        alpha_content: AlphaContent,
        prototype: &mut Texture2,
        output_format: TranscodeFmt,
        transcode_flags_in: u32,
    ) -> KtxResult<()> {
        let bgd = self.supercompression_global_data();
        let bgdh = BasisLzGlobalHeader::from_bytes(bgd).ok_or(ErrorCode::FileDataError)?;
        if bgdh.endpoints_byte_length == 0
            || bgdh.selectors_byte_length == 0
            || bgdh.tables_byte_length == 0
        {
            return Err(ErrorCode::FileDataError);
        }

        // Index of the first image of each level; last entry = total images.
        let num_levels = self.num_levels as usize;
        let layers_faces = self.num_layers * self.num_faces;
        let mut first_images = vec![0u32; num_levels + 1];
        for level in 0..num_levels {
            let depth = self
                .base_depth
                .checked_shr(level as u32)
                .unwrap_or(0)
                .max(1);
            first_images[level + 1] = first_images[level] + layers_faces * depth;
        }
        let image_count = first_images[num_levels] as usize;

        let endpoints_off =
            BASIS_LZ_GLOBAL_HEADER_SIZE + BASIS_LZ_ETC1S_IMAGE_DESC_SIZE * image_count;
        let selectors_off = endpoints_off + bgdh.endpoints_byte_length as usize;
        let tables_off = selectors_off + bgdh.selectors_byte_length as usize;
        let tables_end = tables_off + bgdh.tables_byte_length as usize;
        if tables_end > bgd.len() {
            return Err(ErrorCode::FileDataError);
        }

        let mut bit = LowlevelEtc1sTranscoder::new();
        let mut xcoder_states = vec![
            TranscoderState::new();
            if self.is_video {
                self.num_faces as usize
            } else {
                1
            }
        ];
        if !bit.decode_palettes(
            bgdh.endpoint_count as u32,
            &bgd[endpoints_off..selectors_off],
            bgdh.selector_count as u32,
            &bgd[selectors_off..tables_off],
        ) {
            return Err(ErrorCode::TranscodeFailed);
        }
        if !bit.decode_tables(&bgd[tables_off..tables_end]) {
            return Err(ErrorCode::TranscodeFailed);
        }

        let output_block_byte_length = (prototype.format_size().block_size_in_bits / 8) as u64;
        if output_block_byte_length == 0 {
            return Err(ErrorCode::TranscodeFailed);
        }
        let xcoded_data_length = prototype.data_size as u64 / output_block_byte_length;
        let proto_level_index = prototype.level_index().to_vec();
        let level_image_sizes_out: Vec<u64> = (0..num_levels)
            .map(|l| prototype.calc_image_size(l as u32, FormatVersion::Two) as u64)
            .collect();
        let src = self.p_data.as_deref().ok_or(ErrorCode::InvalidOperation)?;
        let dst = prototype
            .base
            .p_data
            .as_deref_mut()
            .ok_or(ErrorCode::OutOfMemory)?;
        let target = ktx2transcoder_format(output_format);

        for level in (0..num_levels).rev() {
            let level_offset = self.level_data_offset(level as u32);
            let mut write_offset = proto_level_index.get(level).map_or(0, |e| e.byte_offset);
            let level_width = self
                .base_width
                .checked_shr(level as u32)
                .unwrap_or(0)
                .max(1);
            let level_height = self
                .base_height
                .checked_shr(level as u32)
                .unwrap_or(0)
                .max(1);
            let level_blocks_x = level_width.div_ceil(4);
            let level_blocks_y = level_height.div_ceil(4);
            let depth = self
                .base_depth
                .checked_shr(level as u32)
                .unwrap_or(0)
                .max(1);
            let num_images = self.num_layers * self.num_faces * depth;
            let level_image_size_out = level_image_sizes_out[level];
            let mut state_index = 0usize;
            for image in first_images[level]..first_images[level] + num_images {
                let desc_off =
                    BASIS_LZ_GLOBAL_HEADER_SIZE + BASIS_LZ_ETC1S_IMAGE_DESC_SIZE * image as usize;
                let image_desc = BasisLzEtc1sImageDesc::from_bytes(&bgd[desc_off..])
                    .ok_or(ErrorCode::FileDataError)?;
                let si = state_index;
                state_index += 1;
                if state_index == xcoder_states.len() {
                    state_index = 0;
                }
                if alpha_content != AlphaContent::None
                    && (image_desc.alpha_slice_byte_offset == 0
                        || image_desc.alpha_slice_byte_length == 0)
                {
                    return Err(ErrorCode::FileDataError);
                }
                let write_offset_blocks = write_offset / output_block_byte_length;
                let out = dst
                    .get_mut(write_offset as usize..)
                    .ok_or(ErrorCode::TranscodeFailed)?;
                let status = bit.transcode_image(
                    target,
                    out,
                    xcoded_data_length.saturating_sub(write_offset_blocks) as u32,
                    src,
                    level_blocks_x,
                    level_blocks_y,
                    level_width,
                    level_height,
                    level as u32,
                    level_offset + image_desc.rgb_slice_byte_offset as u64,
                    image_desc.rgb_slice_byte_length,
                    level_offset + image_desc.alpha_slice_byte_offset as u64,
                    image_desc.alpha_slice_byte_length,
                    transcode_flags_in,
                    alpha_content != AlphaContent::None,
                    self.is_video,
                    0,
                    &mut xcoder_states[si],
                    0,
                );
                match status {
                    TranscodeStatus::Ok => {}
                    TranscodeStatus::Unsupported => return Err(ErrorCode::UnsupportedFeature),
                    TranscodeStatus::Failed => return Err(ErrorCode::TranscodeFailed),
                }
                write_offset += level_image_size_out;
            }
        }
        Ok(())
    }

    /// Port of `ktxTexture2_transcodeUastc`.
    fn transcode_uastc(
        &self,
        color_model: u32,
        alpha_content: AlphaContent,
        prototype: &mut Texture2,
        output_format: TranscodeFmt,
        transcode_flags_in: u32,
    ) -> KtxResult<()> {
        if color_model == KHR_DF_MODEL_UASTC {
            self.transcode_uastc_ldr_4x4(
                alpha_content,
                prototype,
                output_format,
                transcode_flags_in,
            )
        } else if color_model == KHR_DF_MODEL_UASTC_HDR_4x4 {
            self.transcode_uastc_hdr_4x4(
                alpha_content,
                prototype,
                output_format,
                transcode_flags_in,
            )
        } else if color_model == KHR_DF_MODEL_UASTC_HDR_6x6 {
            self.transcode_uastc_hdr_6x6_intermediate(
                alpha_content,
                prototype,
                output_format,
                transcode_flags_in,
            )
        } else {
            Err(ErrorCode::UnsupportedFeature)
        }
    }

    /// Port of `transcodeUastcHDR6x6_intermediate`.
    fn transcode_uastc_hdr_6x6_intermediate(
        &self,
        alpha_content: AlphaContent,
        prototype: &mut Texture2,
        output_format: TranscodeFmt,
        transcode_flags_in: u32,
    ) -> KtxResult<()> {
        let output_block_byte_length = (prototype.format_size().block_size_in_bits / 8) as u64;
        if output_block_byte_length == 0 {
            return Err(ErrorCode::TranscodeFailed);
        }
        let xcoded_data_length = prototype.data_size as u64 / output_block_byte_length;
        let proto_level_index = prototype.level_index().to_vec();
        let num_levels = self.num_levels as usize;
        let level_image_sizes_out: Vec<u64> = (0..num_levels)
            .map(|l| prototype.calc_image_size(l as u32, FormatVersion::Two) as u64)
            .collect();

        let uit = LowlevelUastcHdr6x6IntermediateTranscoder::new();
        let mut xcoder_states = vec![
            TranscoderState::new();
            if self.is_video {
                self.num_faces as usize
            } else {
                1
            }
        ];

        // The image description seek table; offsets are relative to each
        // mipmap level's data.
        let sgd = self.supercompression_global_data();
        let total_image_descs = (sgd.len() / UASTC_HDR_6X6_INTERMEDIATE_IMAGE_DESC_SIZE) as u64;

        // Descriptions are stored level 0 first, each level contributing
        // numLayers * numFaces * depth(level) images.
        let mut first_images = vec![0u64; num_levels + 1];
        for l in 0..num_levels {
            let depth = self.base_depth.checked_shr(l as u32).unwrap_or(0).max(1);
            first_images[l + 1] = first_images[l]
                + u64::from(self.num_layers) * u64::from(self.num_faces) * u64::from(depth);
        }
        // A descriptor table whose size does not match exactly is corrupt.
        if !sgd
            .len()
            .is_multiple_of(UASTC_HDR_6X6_INTERMEDIATE_IMAGE_DESC_SIZE)
            || first_images[num_levels] != total_image_descs
        {
            return Err(ErrorCode::FileDataError);
        }

        let src = self.p_data.as_deref().ok_or(ErrorCode::InvalidOperation)?;
        let dst = prototype
            .base
            .p_data
            .as_deref_mut()
            .ok_or(ErrorCode::OutOfMemory)?;
        let target = ktx2transcoder_format(output_format);

        for level in (0..num_levels).rev() {
            let shr = |v: u32| v.checked_shr(level as u32).unwrap_or(0).max(1);
            let mut write_offset = proto_level_index.get(level).map_or(0, |e| e.byte_offset);
            let level_width = shr(self.base_width);
            let level_height = shr(self.base_height);
            // UASTC HDR 6x6i texel block dimensions
            let level_blocks_x = level_width.div_ceil(6);
            let level_blocks_y = level_height.div_ceil(6);
            let depth = shr(self.base_depth);
            let level_image_count = self.num_layers * self.num_faces * depth;
            let level_image_size_out = level_image_sizes_out[level];

            // Offset and length of the mipmap level's data within the KTX2 file.
            let level_data_offset = self.level_data_offset(level as u32);
            let level_data_length = self.level_index().get(level).map_or(0, |e| e.byte_length);
            if level_data_length > u64::from(u32::MAX) {
                return Err(ErrorCode::FileDataError);
            }
            let level_data = usize::try_from(level_data_offset)
                .ok()
                .and_then(|o| src.get(o..o.checked_add(level_data_length as usize)?))
                .ok_or(ErrorCode::FileDataError)?;

            let mut state_index = 0usize;
            for image in 0..u64::from(level_image_count) {
                let si = state_index;
                state_index += 1;
                if state_index == xcoder_states.len() {
                    state_index = 0;
                }
                let sgd_image_desc_index = first_images[level] + image;
                if sgd_image_desc_index >= total_image_descs {
                    return Err(ErrorCode::TranscodeFailed);
                }
                let desc = UastcHdr6x6IntermediateImageDesc::from_bytes(
                    &sgd[sgd_image_desc_index as usize
                        * UASTC_HDR_6X6_INTERMEDIATE_IMAGE_DESC_SIZE..],
                )
                .ok_or(ErrorCode::FileDataError)?;
                let write_offset_blocks = write_offset / output_block_byte_length;
                let out = dst
                    .get_mut(write_offset as usize..)
                    .ok_or(ErrorCode::TranscodeFailed)?;
                let status = uit.transcode_image(
                    target,
                    out,
                    xcoded_data_length.saturating_sub(write_offset_blocks) as u32,
                    level_data,
                    level_blocks_x,
                    level_blocks_y,
                    level_width,
                    level_height,
                    level as u32,
                    u64::from(desc.rgb_slice_byte_offset),
                    desc.rgb_slice_byte_length,
                    transcode_flags_in,
                    alpha_content != AlphaContent::None,
                    self.is_video,
                    0,
                    &mut xcoder_states[si],
                    0,
                    -1,
                    -1,
                );
                match status {
                    TranscodeStatus::Ok => {}
                    TranscodeStatus::Unsupported => return Err(ErrorCode::UnsupportedFeature),
                    TranscodeStatus::Failed => return Err(ErrorCode::TranscodeFailed),
                }
                write_offset += level_image_size_out;
            }
        }
        Ok(())
    }

    /// Port of `transcodeUastcLDR4x4`.
    fn transcode_uastc_ldr_4x4(
        &self,
        alpha_content: AlphaContent,
        prototype: &mut Texture2,
        output_format: TranscodeFmt,
        transcode_flags_in: u32,
    ) -> KtxResult<()> {
        self.transcode_uastc_4x4(
            &LowlevelUastcLdr4x4Transcoder::new(),
            alpha_content,
            prototype,
            output_format,
            transcode_flags_in,
        )
    }

    /// Port of `transcodeUastcHDR4x4`.
    fn transcode_uastc_hdr_4x4(
        &self,
        alpha_content: AlphaContent,
        prototype: &mut Texture2,
        output_format: TranscodeFmt,
        transcode_flags_in: u32,
    ) -> KtxResult<()> {
        self.transcode_uastc_4x4(
            &LowlevelUastcHdr4x4Transcoder::new(),
            alpha_content,
            prototype,
            output_format,
            transcode_flags_in,
        )
    }

    /// Shared body of `transcodeUastcLDR4x4` and `transcodeUastcHDR4x4`
    /// (identical in C apart from the low-level transcoder type).
    fn transcode_uastc_4x4(
        &self,
        uit: &dyn UastcImageTranscoder,
        alpha_content: AlphaContent,
        prototype: &mut Texture2,
        output_format: TranscodeFmt,
        transcode_flags_in: u32,
    ) -> KtxResult<()> {
        let output_block_byte_length = (prototype.format_size().block_size_in_bits / 8) as u64;
        if output_block_byte_length == 0 {
            return Err(ErrorCode::TranscodeFailed);
        }
        let xcoded_data_length = prototype.data_size as u64 / output_block_byte_length;
        let proto_level_index = prototype.level_index().to_vec();
        let num_levels = self.num_levels as usize;
        let level_image_sizes_out: Vec<u64> = (0..num_levels)
            .map(|l| prototype.calc_image_size(l as u32, FormatVersion::Two) as u64)
            .collect();
        let level_image_sizes_in: Vec<u64> = (0..num_levels)
            .map(|l| self.calc_image_size(l as u32, FormatVersion::Two) as u64)
            .collect();

        let mut xcoder_states = vec![
            TranscoderState::new();
            if self.is_video {
                self.num_faces as usize
            } else {
                1
            }
        ];
        let src = self.p_data.as_deref().ok_or(ErrorCode::InvalidOperation)?;
        let dst = prototype
            .base
            .p_data
            .as_deref_mut()
            .ok_or(ErrorCode::OutOfMemory)?;
        let target = ktx2transcoder_format(output_format);

        for level in (0..num_levels).rev() {
            let shr = |v: u32| v.checked_shr(level as u32).unwrap_or(0).max(1);
            let mut write_offset = proto_level_index.get(level).map_or(0, |e| e.byte_offset);
            let level_width = shr(self.base_width);
            let level_height = shr(self.base_height);
            let level_blocks_x = level_width.div_ceil(4);
            let level_blocks_y = level_height.div_ceil(4);
            let depth = shr(self.base_depth);
            let level_image_count = self.num_layers * self.num_faces * depth;
            let level_image_size_in = level_image_sizes_in[level];
            let level_image_size_out = level_image_sizes_out[level];
            let mut level_image_offset_in = self.level_data_offset(level as u32);
            let mut state_index = 0usize;
            for _ in 0..level_image_count {
                let si = state_index;
                state_index += 1;
                if state_index == xcoder_states.len() {
                    state_index = 0;
                }
                let write_offset_blocks = write_offset / output_block_byte_length;
                let out = dst
                    .get_mut(write_offset as usize..)
                    .ok_or(ErrorCode::TranscodeFailed)?;
                let status = uit.transcode_image(
                    target,
                    out,
                    xcoded_data_length.saturating_sub(write_offset_blocks) as u32,
                    src,
                    level_blocks_x,
                    level_blocks_y,
                    level_width,
                    level_height,
                    level as u32,
                    level_image_offset_in,
                    level_image_size_in as u32,
                    transcode_flags_in,
                    alpha_content != AlphaContent::None,
                    self.is_video,
                    0,
                    &mut xcoder_states[si],
                    0,
                    -1,
                    -1,
                );
                match status {
                    TranscodeStatus::Ok => {}
                    TranscodeStatus::Unsupported => return Err(ErrorCode::UnsupportedFeature),
                    TranscodeStatus::Failed => return Err(ErrorCode::TranscodeFailed),
                }
                write_offset += level_image_size_out;
                level_image_offset_in += level_image_size_in;
            }
        }
        Ok(())
    }
}

/// The `transcode_image` entry point shared by the UASTC 4x4 low-level transcoders.
trait UastcImageTranscoder {
    #[allow(clippy::too_many_arguments)]
    fn transcode_image(
        &self,
        target_format: TranscoderTextureFormat,
        output: &mut [u8],
        output_blocks_buf_size_in_blocks_or_pixels: u32,
        compressed_data: &[u8],
        num_blocks_x: u32,
        num_blocks_y: u32,
        orig_width: u32,
        orig_height: u32,
        level_index: u32,
        slice_offset: u64,
        slice_length: u32,
        decode_flags: u32,
        has_alpha: bool,
        is_video: bool,
        output_row_pitch_in_blocks_or_pixels: u32,
        state: &mut TranscoderState,
        output_rows_in_pixels: u32,
        channel0: i32,
        channel1: i32,
    ) -> TranscodeStatus;
}

macro_rules! impl_uastc_image_transcoder {
    ($t:ty) => {
        impl UastcImageTranscoder for $t {
            fn transcode_image(
                &self,
                target_format: TranscoderTextureFormat,
                output: &mut [u8],
                output_blocks_buf_size_in_blocks_or_pixels: u32,
                compressed_data: &[u8],
                num_blocks_x: u32,
                num_blocks_y: u32,
                orig_width: u32,
                orig_height: u32,
                level_index: u32,
                slice_offset: u64,
                slice_length: u32,
                decode_flags: u32,
                has_alpha: bool,
                is_video: bool,
                output_row_pitch_in_blocks_or_pixels: u32,
                state: &mut TranscoderState,
                output_rows_in_pixels: u32,
                channel0: i32,
                channel1: i32,
            ) -> TranscodeStatus {
                <$t>::transcode_image(
                    self,
                    target_format,
                    output,
                    output_blocks_buf_size_in_blocks_or_pixels,
                    compressed_data,
                    num_blocks_x,
                    num_blocks_y,
                    orig_width,
                    orig_height,
                    level_index,
                    slice_offset,
                    slice_length,
                    decode_flags,
                    has_alpha,
                    is_video,
                    output_row_pitch_in_blocks_or_pixels,
                    state,
                    output_rows_in_pixels,
                    channel0,
                    channel1,
                )
            }
        }
    };
}

impl_uastc_image_transcoder!(LowlevelUastcLdr4x4Transcoder);
impl_uastc_image_transcoder!(LowlevelUastcHdr4x4Transcoder);
