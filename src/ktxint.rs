// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of lib/src/ktxint.h structures and lib/src/checkheader.c.

use crate::defs::{
    KTX_ENDIAN_REF, KTX_ENDIAN_REF_REV, KTX_HEADER_SIZE, KTX_IDENTIFIER_REF, KTX2_HEADER_SIZE,
    KTX2_IDENTIFIER_REF, SupercmpScheme,
};
use crate::error::{ErrorCode, KtxResult};
use crate::vkformat::{VkFormat, is_prohibited_format, is_valid_format};

fn rd_u32(b: &[u8], off: usize) -> u32 {
    match b.get(off..off + 4) {
        Some(s) => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

fn rd_u64(b: &[u8], off: usize) -> u64 {
    match b.get(off..off + 8) {
        Some(s) => u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]),
        None => 0,
    }
}

/// Port of `KTX_header` (KTX 1). Fields are as stored in the file (native order
/// of the writer); [`ktx_check_header1`] swaps them when needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KtxHeader {
    pub identifier: [u8; 12],
    pub endianness: u32,
    pub gl_type: u32,
    pub gl_type_size: u32,
    pub gl_format: u32,
    pub gl_internalformat: u32,
    pub gl_base_internalformat: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub pixel_depth: u32,
    pub number_of_array_elements: u32,
    pub number_of_faces: u32,
    pub number_of_mip_levels: u32,
    pub bytes_of_key_value_data: u32,
}

impl KtxHeader {
    /// Decodes a header from 64 little-endian bytes.
    pub fn from_bytes(b: &[u8; KTX_HEADER_SIZE]) -> KtxHeader {
        let mut identifier = [0u8; 12];
        identifier.copy_from_slice(&b[..12]);
        KtxHeader {
            identifier,
            endianness: rd_u32(b, 12),
            gl_type: rd_u32(b, 16),
            gl_type_size: rd_u32(b, 20),
            gl_format: rd_u32(b, 24),
            gl_internalformat: rd_u32(b, 28),
            gl_base_internalformat: rd_u32(b, 32),
            pixel_width: rd_u32(b, 36),
            pixel_height: rd_u32(b, 40),
            pixel_depth: rd_u32(b, 44),
            number_of_array_elements: rd_u32(b, 48),
            number_of_faces: rd_u32(b, 52),
            number_of_mip_levels: rd_u32(b, 56),
            bytes_of_key_value_data: rd_u32(b, 60),
        }
    }

    /// Encodes the header as 64 little-endian bytes.
    pub fn to_bytes(&self) -> [u8; KTX_HEADER_SIZE] {
        let mut b = [0u8; KTX_HEADER_SIZE];
        b[..12].copy_from_slice(&self.identifier);
        let fields = [
            self.endianness,
            self.gl_type,
            self.gl_type_size,
            self.gl_format,
            self.gl_internalformat,
            self.gl_base_internalformat,
            self.pixel_width,
            self.pixel_height,
            self.pixel_depth,
            self.number_of_array_elements,
            self.number_of_faces,
            self.number_of_mip_levels,
            self.bytes_of_key_value_data,
        ];
        for (i, f) in fields.iter().enumerate() {
            let o = 12 + i * 4;
            b[o..o + 4].copy_from_slice(&f.to_le_bytes());
        }
        b
    }

    /// The 12 `u32` fields after `endianness` (glType .. bytesOfKeyValueData).
    fn swap_fields(&mut self) {
        for f in [
            &mut self.gl_type,
            &mut self.gl_type_size,
            &mut self.gl_format,
            &mut self.gl_internalformat,
            &mut self.gl_base_internalformat,
            &mut self.pixel_width,
            &mut self.pixel_height,
            &mut self.pixel_depth,
            &mut self.number_of_array_elements,
            &mut self.number_of_faces,
            &mut self.number_of_mip_levels,
            &mut self.bytes_of_key_value_data,
        ] {
            *f = f.swap_bytes();
        }
    }
}

/// Port of `ktxIndexEntry32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexEntry32 {
    pub byte_offset: u32,
    pub byte_length: u32,
}

/// Port of `ktxIndexEntry64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndexEntry64 {
    pub byte_offset: u64,
    pub byte_length: u64,
}

/// Port of `KTX_header2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KtxHeader2 {
    pub identifier: [u8; 12],
    pub vk_format: VkFormat,
    pub type_size: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub pixel_depth: u32,
    pub layer_count: u32,
    pub face_count: u32,
    pub level_count: u32,
    pub supercompression_scheme: SupercmpScheme,
    pub data_format_descriptor: IndexEntry32,
    pub key_value_data: IndexEntry32,
    pub supercompression_global_data: IndexEntry64,
}

impl KtxHeader2 {
    /// Decodes a header from 80 little-endian bytes.
    pub fn from_bytes(b: &[u8; KTX2_HEADER_SIZE]) -> KtxHeader2 {
        let mut identifier = [0u8; 12];
        identifier.copy_from_slice(&b[..12]);
        KtxHeader2 {
            identifier,
            vk_format: VkFormat(rd_u32(b, 12)),
            type_size: rd_u32(b, 16),
            pixel_width: rd_u32(b, 20),
            pixel_height: rd_u32(b, 24),
            pixel_depth: rd_u32(b, 28),
            layer_count: rd_u32(b, 32),
            face_count: rd_u32(b, 36),
            level_count: rd_u32(b, 40),
            supercompression_scheme: SupercmpScheme(rd_u32(b, 44)),
            data_format_descriptor: IndexEntry32 {
                byte_offset: rd_u32(b, 48),
                byte_length: rd_u32(b, 52),
            },
            key_value_data: IndexEntry32 {
                byte_offset: rd_u32(b, 56),
                byte_length: rd_u32(b, 60),
            },
            supercompression_global_data: IndexEntry64 {
                byte_offset: rd_u64(b, 64),
                byte_length: rd_u64(b, 72),
            },
        }
    }

    /// Encodes the header as 80 little-endian bytes.
    pub fn to_bytes(&self) -> [u8; KTX2_HEADER_SIZE] {
        let mut b = [0u8; KTX2_HEADER_SIZE];
        b[..12].copy_from_slice(&self.identifier);
        let u32s = [
            self.vk_format.0,
            self.type_size,
            self.pixel_width,
            self.pixel_height,
            self.pixel_depth,
            self.layer_count,
            self.face_count,
            self.level_count,
            self.supercompression_scheme.0,
            self.data_format_descriptor.byte_offset,
            self.data_format_descriptor.byte_length,
            self.key_value_data.byte_offset,
            self.key_value_data.byte_length,
        ];
        for (i, f) in u32s.iter().enumerate() {
            let o = 12 + i * 4;
            b[o..o + 4].copy_from_slice(&f.to_le_bytes());
        }
        b[64..72].copy_from_slice(&self.supercompression_global_data.byte_offset.to_le_bytes());
        b[72..80].copy_from_slice(&self.supercompression_global_data.byte_length.to_le_bytes());
        b
    }
}

/// Port of `ktxLevelIndexEntry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LevelIndexEntry {
    pub byte_offset: u64,
    pub byte_length: u64,
    pub uncompressed_byte_length: u64,
}

/// Size in bytes of a serialized [`LevelIndexEntry`].
pub const LEVEL_INDEX_ENTRY_SIZE: usize = 24;

impl LevelIndexEntry {
    /// Decodes from 24 little-endian bytes; returns `None` if `b` is too short.
    pub fn from_bytes(b: &[u8]) -> Option<LevelIndexEntry> {
        if b.len() < LEVEL_INDEX_ENTRY_SIZE {
            return None;
        }
        Some(LevelIndexEntry {
            byte_offset: rd_u64(b, 0),
            byte_length: rd_u64(b, 8),
            uncompressed_byte_length: rd_u64(b, 16),
        })
    }

    /// Encodes as 24 little-endian bytes.
    pub fn to_bytes(&self) -> [u8; LEVEL_INDEX_ENTRY_SIZE] {
        let mut b = [0u8; LEVEL_INDEX_ENTRY_SIZE];
        b[..8].copy_from_slice(&self.byte_offset.to_le_bytes());
        b[8..16].copy_from_slice(&self.byte_length.to_le_bytes());
        b[16..].copy_from_slice(&self.uncompressed_byte_length.to_le_bytes());
        b
    }
}

/// Port of `KTX_supplemental_info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SupplementalInfo {
    pub compressed: u8,
    pub generate_mipmaps: u8,
    pub texture_dimension: u16,
}

fn check_dims_and_levels(width: u32, height: u32, depth: u32, layers: u32) -> KtxResult<u16> {
    // KTX files can store 8 types of textures: 1D, 2D, 3D, cube, and array
    // variants of these. There is no extension for 3D array textures.
    if width == 0 || (depth > 0 && height == 0) {
        return Err(ErrorCode::FileDataError);
    }
    if depth > 0 {
        if layers > 0 {
            return Err(ErrorCode::UnsupportedFeature);
        }
        Ok(3)
    } else if height > 0 {
        Ok(2)
    } else {
        Ok(1)
    }
}

fn check_level_count(levels: u32, width: u32, height: u32, depth: u32) -> KtxResult<()> {
    // Works for arrays too because height or depth will be 0.
    let max_dim = width.max(height).max(depth);
    // Check > 32 first. Shifting by 32 or more is undefined.
    if levels > 32 || levels == 0 || max_dim < (1u32 << (levels - 1)) {
        return Err(ErrorCode::FileDataError);
    }
    Ok(())
}

/// Port of `ktxCheckHeader1_`. May byte swap and modify `header`
/// (`numberOfMipLevels` 0 becomes 1).
pub fn ktx_check_header1(header: &mut KtxHeader) -> KtxResult<SupplementalInfo> {
    let mut supp = SupplementalInfo::default();
    if header.identifier != KTX_IDENTIFIER_REF {
        return Err(ErrorCode::UnknownFileFormat);
    }
    if header.endianness == KTX_ENDIAN_REF_REV {
        header.swap_fields();
        if header.gl_type_size != 1 && header.gl_type_size != 2 && header.gl_type_size != 4 {
            // Only 8-, 16-, and 32-bit types supported so far.
            return Err(ErrorCode::FileDataError);
        }
    } else if header.endianness != KTX_ENDIAN_REF {
        return Err(ErrorCode::FileDataError);
    }

    // Check glType and glFormat
    supp.compressed = 0;
    if header.gl_type == 0 || header.gl_format == 0 {
        if header.gl_type.wrapping_add(header.gl_format) != 0 {
            // either both or none of glType, glFormat must be zero
            return Err(ErrorCode::FileDataError);
        }
        supp.compressed = 1;
    }

    if header.gl_format == header.gl_internalformat {
        // glInternalFormat is either unsized (no longer supported) or glFormat is sized.
        return Err(ErrorCode::FileDataError);
    }

    supp.texture_dimension = check_dims_and_levels(
        header.pixel_width,
        header.pixel_height,
        header.pixel_depth,
        header.number_of_array_elements,
    )?;

    if header.number_of_faces == 6 {
        if supp.texture_dimension != 2 {
            // cube map needs 2D faces
            return Err(ErrorCode::FileDataError);
        }
    } else if header.number_of_faces != 1 {
        // numberOfFaces must be either 1 or 6
        return Err(ErrorCode::FileDataError);
    }

    if header.number_of_mip_levels == 0 {
        supp.generate_mipmaps = 1;
        header.number_of_mip_levels = 1;
    } else {
        supp.generate_mipmaps = 0;
    }

    check_level_count(
        header.number_of_mip_levels,
        header.pixel_width,
        header.pixel_height,
        header.pixel_depth,
    )?;
    Ok(supp)
}

/// Port of `ktxCheckHeader2_`. May modify `header` (`levelCount` 0 becomes 1).
pub fn ktx_check_header2(header: &mut KtxHeader2) -> KtxResult<SupplementalInfo> {
    let mut supp = SupplementalInfo::default();
    if header.identifier != KTX2_IDENTIFIER_REF {
        return Err(ErrorCode::UnknownFileFormat);
    }

    if is_prohibited_format(header.vk_format) {
        return Err(ErrorCode::FileDataError);
    }
    if !is_valid_format(header.vk_format) {
        return Err(ErrorCode::UnsupportedFeature);
    }
    if (header.supercompression_scheme == SupercmpScheme::BASIS_LZ
        || header.supercompression_scheme == SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE)
        && header.vk_format != VkFormat::UNDEFINED
    {
        return Err(ErrorCode::FileDataError);
    }

    supp.texture_dimension = check_dims_and_levels(
        header.pixel_width,
        header.pixel_height,
        header.pixel_depth,
        header.layer_count,
    )?;

    if header.face_count == 6 {
        if supp.texture_dimension != 2 {
            // cube map needs 2D faces
            return Err(ErrorCode::FileDataError);
        }
        if header.pixel_depth != 0 {
            // cube map cannot have depth
            return Err(ErrorCode::FileDataError);
        }
        if header.pixel_width != header.pixel_height {
            // cube map needs square faces
            return Err(ErrorCode::FileDataError);
        }
    } else if header.face_count != 1 {
        // numberOfFaces must be either 1 or 6
        return Err(ErrorCode::FileDataError);
    }

    if header.level_count == 0 {
        supp.generate_mipmaps = 1;
        header.level_count = 1;
    } else {
        supp.generate_mipmaps = 0;
    }

    match header.supercompression_scheme {
        SupercmpScheme::NONE
        | SupercmpScheme::BASIS_LZ
        | SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE
        | SupercmpScheme::ZSTD
        | SupercmpScheme::ZLIB => {}
        // Unsupported supercompression
        _ => return Err(ErrorCode::UnsupportedFeature),
    }

    check_level_count(
        header.level_count,
        header.pixel_width,
        header.pixel_height,
        header.pixel_depth,
    )?;
    Ok(supp)
}
