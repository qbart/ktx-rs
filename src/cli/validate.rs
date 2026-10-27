// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of tools/ktx/validate.h and validate.cpp: the KTX2 file validator.
//!
//! C++ `fatal` issues throw `FatalValidationError`; here they unwind as
//! [`Stop::Fatal`]. A `{fmt}` `format_error` (a message called with too few
//! arguments) escapes the C++ validator as a `std::exception`; it unwinds as
//! [`Stop::Runtime`] and surfaces from `validate_*` as `Err(message)`.

use std::collections::HashSet;

use super::fmt::{Arg, try_format};
use super::formats::*;
use super::utility::{align, ceil_div, log2, popcount, validate_utf8};
use super::validation_messages::*;
use crate::defs::{KTX2_HEADER_SIZE, KTX2_IDENTIFIER_REF, SupercmpScheme, TranscodeFmt};
use crate::dfd::interpret::*;
use crate::dfd::khr_df::*;
use crate::dfd::vk2dfd;
use crate::error::{ErrorCode, ktx_error_string};
use crate::fmt_args;
use crate::ktxint::{KtxHeader2, LEVEL_INDEX_ENTRY_SIZE, LevelIndexEntry};
use crate::strings::ETC1S_P_FRAME;
use crate::texture::{
    KTX_TEXTURE_CREATE_CHECK_GLTF_BASISU_BIT, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT,
};
use crate::texture2::Texture2;
use crate::vkformat::{
    VK_FORMAT_MAX_STANDARD_ENUM, VkFormat, is_prohibited_format, vk_format_type_size,
};

/// Port of `MAX_NUM_DFD_BLOCKS`.
pub const MAX_NUM_DFD_BLOCKS: u32 = 10;
/// Port of `MAX_NUM_BDFD_SAMPLES`.
pub const MAX_NUM_BDFD_SAMPLES: u32 = 16;
/// Port of `MAX_NUM_KV_ENTRIES`.
pub const MAX_NUM_KV_ENTRIES: u32 = 100;

/// Exit code for an invalid file (`rc::INVALID_FILE`).
pub const RC_INVALID_FILE: i32 = 3;

/// Port of `IssueType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueType {
    Warning,
    Error,
    Fatal,
}

impl IssueType {
    /// Port of `toString(IssueType)`.
    pub fn as_str(self) -> &'static str {
        match self {
            IssueType::Warning => "warning",
            IssueType::Error => "error",
            IssueType::Fatal => "fatal",
        }
    }
}

/// Port of `Issue` (and its `IssueWarning` / `IssueError` / `IssueFatal` kinds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Issue {
    pub type_: IssueType,
    pub id: u16,
    pub message: &'static str,
    pub details_fmt: &'static str,
}

/// Port of `ValidationReport`. `details` is raw bytes (keys are printed as found).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub type_: IssueType,
    pub id: u16,
    pub message: String,
    pub details: Vec<u8>,
}

/// Why validation stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stop {
    /// A fatal issue was reported (`FatalValidationError`).
    Fatal,
    /// A `std::exception` escaped (its `what()`).
    Runtime(String),
}

type VResult<T = ()> = Result<T, Stop>;

/// Port of `DFDHeader` / `BDFD` (the parts the validator reads).
#[derive(Debug, Clone, Copy, Default)]
struct Bdfd {
    vendor_id: u32,
    descriptor_type: u32,
    version_number: u32,
    descriptor_block_size: u32,
    model: u32,
    primaries: u32,
    transfer: u32,
    texel_block_dimension: [u32; 4],
    bytes_planes: [u8; 8],
}

fn word(b: &[u8], i: usize) -> u32 {
    match b.get(i * 4..i * 4 + 4) {
        Some(s) => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

impl Bdfd {
    /// Decodes the first 8 (header) or 24 bytes; missing bytes read as 0.
    fn parse(b: &[u8]) -> Bdfd {
        let (w0, w1, w2, w3) = (word(b, 0), word(b, 1), word(b, 2), word(b, 3));
        let mut bytes_planes = [0u8; 8];
        for (i, p) in bytes_planes.iter_mut().enumerate() {
            *p = b.get(16 + i).copied().unwrap_or(0);
        }
        Bdfd {
            vendor_id: w0 & 0x1FFFF,
            descriptor_type: w0 >> 17,
            version_number: w1 & 0xFFFF,
            descriptor_block_size: w1 >> 16,
            model: w2 & 0xFF,
            primaries: (w2 >> 8) & 0xFF,
            transfer: (w2 >> 16) & 0xFF,
            texel_block_dimension: [w3 & 0xFF, (w3 >> 8) & 0xFF, (w3 >> 16) & 0xFF, w3 >> 24],
            bytes_planes,
        }
    }

    fn match_texel_block_dimensions(&self, d: [u32; 4]) -> bool {
        self.texel_block_dimension == d
    }
}

/// Port of `SampleType`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SampleType {
    bit_offset: u32,
    bit_length: u32,
    channel_id: u32,
    qualifier_linear: u32,
    qualifier_exponent: u32,
    qualifier_signed: u32,
    qualifier_float: u32,
    sample_position: [u32; 4],
    lower: u32,
    upper: u32,
}

impl SampleType {
    fn parse(b: &[u8]) -> SampleType {
        let (w0, w1) = (word(b, 0), word(b, 1));
        SampleType {
            bit_offset: w0 & 0xFFFF,
            bit_length: (w0 >> 16) & 0xFF,
            channel_id: (w0 >> 24) & 0xF,
            qualifier_linear: (w0 >> 28) & 1,
            qualifier_exponent: (w0 >> 29) & 1,
            qualifier_signed: (w0 >> 30) & 1,
            qualifier_float: w0 >> 31,
            sample_position: [w1 & 0xFF, (w1 >> 8) & 0xFF, (w1 >> 16) & 0xFF, w1 >> 24],
            lower: word(b, 2),
            upper: word(b, 3),
        }
    }
}

fn parse_samples(b: &[u8], count: usize) -> Vec<SampleType> {
    (0..count)
        .map(|i| SampleType::parse(b.get(i * 16..).unwrap_or_default()))
        .collect()
}

/// Port of `ValidationContext` over in-memory file data (`ValidationContextMemory`).
struct ValidationContext<'a> {
    callback: &'a mut dyn FnMut(&ValidationReport),
    data: &'a [u8],

    treat_warnings_as_error: bool,
    check_gltf_basisu: bool,

    return_code: i32,
    num_error: u32,
    num_warning: u32,

    header: KtxHeader2,
    num_layers: u32,
    num_levels: u32,
    dimension_count: u32,
    num_samples: u32,
    level_indices: Vec<LevelIndexEntry>,

    expected_color_model: Option<u32>,
    expected_bytes_planes: Option<[u8; 8]>,
    expected_block_dimension: [Option<u8>; 4],
    expected_type_size: Option<u32>,
    expected_samples: Option<Vec<SampleType>>,

    parsed_color_model: Option<u32>,
    parsed_transfer_function: Option<u32>,
    parsed_block_byte_length: Option<u8>,
    parsed_block_dimension: [Option<u8>; 3],

    found_ktx_anim_data: bool,
    found_ktx_cubemap_incomplete: bool,
    found_ktx_writer: bool,
    found_ktx_writer_sc_params: bool,
}

impl<'a> ValidationContext<'a> {
    fn new(
        warnings_as_errors: bool,
        gltf_basisu: bool,
        callback: &'a mut dyn FnMut(&ValidationReport),
        data: &'a [u8],
    ) -> Self {
        ValidationContext {
            callback,
            data,
            treat_warnings_as_error: warnings_as_errors,
            check_gltf_basisu: gltf_basisu,
            return_code: 0,
            num_error: 0,
            num_warning: 0,
            header: KtxHeader2::default(),
            num_layers: 0,
            num_levels: 0,
            dimension_count: 0,
            num_samples: 0,
            level_indices: Vec::new(),
            expected_color_model: None,
            expected_bytes_planes: None,
            expected_block_dimension: [None; 4],
            expected_type_size: None,
            expected_samples: None,
            parsed_color_model: None,
            parsed_transfer_function: None,
            parsed_block_byte_length: None,
            parsed_block_dimension: [None; 3],
            found_ktx_anim_data: false,
            found_ktx_cubemap_incomplete: false,
            found_ktx_writer: false,
            found_ktx_writer_sc_params: false,
        }
    }

    fn report(&mut self, type_: IssueType, issue: &Issue, args: &[Arg]) -> VResult {
        let details =
            try_format(issue.details_fmt, args).map_err(|e| Stop::Runtime(e.to_string()))?;
        (self.callback)(&ValidationReport {
            type_,
            id: issue.id,
            message: issue.message.to_string(),
            details,
        });
        Ok(())
    }

    fn warning(&mut self, issue: &Issue, args: Vec<Arg>) -> VResult {
        self.num_warning += 1;
        if self.treat_warnings_as_error {
            self.return_code = RC_INVALID_FILE;
            self.report(IssueType::Error, issue, &args)
        } else {
            self.report(issue.type_, issue, &args)
        }
    }

    fn error(&mut self, issue: &Issue, args: Vec<Arg>) -> VResult {
        self.num_error += 1;
        self.return_code = RC_INVALID_FILE;
        self.report(issue.type_, issue, &args)
    }

    fn fatal<T>(&mut self, issue: &Issue, args: Vec<Arg>) -> VResult<T> {
        self.num_error += 1;
        self.return_code = RC_INVALID_FILE;
        self.report(issue.type_, issue, &args)?;
        Err(Stop::Fatal)
    }

    /// `ValidationContextMemory::read`.
    fn read(&mut self, offset: u64, size: u64, name: &str) -> VResult<Vec<u8>> {
        let len = self.data.len() as u64;
        let available = len.saturating_sub(offset);
        if available < size {
            return self.fatal(
                &IOError::UnexpectedEOF,
                fmt_args![size, offset, name, available],
            );
        }
        Ok(self
            .data
            .get(offset as usize..(offset + size) as usize)
            .unwrap_or_default()
            .to_vec())
    }

    fn vk_format(&self) -> VkFormat {
        self.header.vk_format
    }

    fn scheme(&self) -> SupercmpScheme {
        self.header.supercompression_scheme
    }

    fn vk_str(&self) -> String {
        to_string_vk_format(self.vk_format())
    }

    /// Port of `validateAlignmentPaddingZeros` over `buf[pos..min(end, align(pos))]`.
    fn validate_alignment_padding_zeros(
        &mut self,
        buf: &[u8],
        pos: usize,
        end: usize,
        alignment: usize,
        issue: &Issue,
        what: &str,
    ) -> VResult {
        let stop = end.min(align(pos as u64, alignment as u64) as usize);
        for i in pos..stop {
            let b = buf.get(i).copied().unwrap_or(0);
            if b != 0 {
                self.error(issue, fmt_args![b, what])?;
            }
        }
        Ok(())
    }

    /// Port of `ValidationContext::validate`.
    fn validate(&mut self, do_create_and_transcode_checks: bool) -> VResult<i32> {
        self.validate_header()?;
        self.validate_indices()?;
        self.calculate_expected_dfd(self.vk_format())?;
        self.validate_dfd()?;
        self.validate_level_index()?; // Must come after the DFD parsed
        self.validate_kvd()?;
        self.validate_sgd()?;
        self.validate_paddings()?;
        if do_create_and_transcode_checks {
            self.validate_create_and_transcode()?;
        }
        Ok(self.return_code)
    }

    fn validate_header(&mut self) -> VResult {
        let raw = self.read(0, KTX2_HEADER_SIZE as u64, "the header")?;
        let mut b = [0u8; KTX2_HEADER_SIZE];
        b.copy_from_slice(
            raw.get(..KTX2_HEADER_SIZE)
                .unwrap_or(&[0; KTX2_HEADER_SIZE]),
        );
        self.header = KtxHeader2::from_bytes(&b);
        let h = self.header;
        let vk_format = h.vk_format;
        let scheme = h.supercompression_scheme;
        let vf = vk_format.0 as i32;

        // Validate file identifier
        if h.identifier != KTX2_IDENTIFIER_REF {
            return self.fatal(&FileError::NotKTX2, vec![]);
        }

        // Validate vkFormat
        if is_prohibited_format(vk_format) {
            self.error(&HeaderData::ProhibitedFormat, fmt_args![self.vk_str()])?;
        } else if !is_format_valid(vk_format) {
            if vf <= VK_FORMAT_MAX_STANDARD_ENUM as i32 {
                self.error(&HeaderData::InvalidFormat, fmt_args![self.vk_str()])?;
            }
            if (VK_FORMAT_MAX_STANDARD_ENUM as i32) < vf && vf < 1_000_001_000 {
                self.error(&HeaderData::InvalidFormat, fmt_args![self.vk_str()])?;
            }
            if 1_000_001_000 <= vf && !is_format_known(vk_format) {
                self.warning(&HeaderData::UnknownFormat, fmt_args![self.vk_str()])?;
            }
        }

        if (scheme == SupercmpScheme::BASIS_LZ
            || scheme == SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE)
            && vk_format != VkFormat::UNDEFINED
        {
            self.error(&HeaderData::VkFormatAndBasis, fmt_args![self.vk_str()])?;
        }

        // Validate typeSize
        if (vk_format == VkFormat::UNDEFINED || is_format_block_compressed(vk_format))
            && h.type_size != 1
        {
            self.error(
                &HeaderData::TypeSizeNotOne,
                fmt_args![h.type_size, self.vk_str()],
            )?;
        }
        // Additional checks are performed on typeSize after the DFD is parsed

        // Validate image dimensions
        if h.pixel_width == 0 {
            self.error(&HeaderData::WidthZero, vec![])?;
        }
        if is_format_block_compressed(vk_format) && h.pixel_height == 0 {
            self.error(
                &HeaderData::BlockCompressedNoHeight,
                fmt_args![self.vk_str()],
            )?;
        }
        if is_supercompression_block_compressed(scheme) && h.pixel_height == 0 {
            self.error(
                &HeaderData::BlockCompressedNoHeight,
                fmt_args![to_string_scheme(scheme)],
            )?;
        }
        // Additional block-compressed formats (like UASTC) are detected after the DFD is parsed to validate pixelHeight

        if h.face_count == 6 && h.pixel_width != h.pixel_height {
            self.error(
                &HeaderData::CubeHeightWidthMismatch,
                fmt_args![h.pixel_width, h.pixel_height],
            )?;
        }
        if h.pixel_depth != 0 && h.pixel_height == 0 {
            self.error(&HeaderData::DepthNoHeight, fmt_args![h.pixel_depth])?;
        }
        if is_format_3d_block_compressed(vk_format) && h.pixel_depth == 0 {
            self.error(
                &HeaderData::DepthBlockCompressedNoDepth,
                fmt_args![self.vk_str()],
            )?;
        }
        if (is_format_depth(vk_format) || is_format_stencil(vk_format)) && h.pixel_depth != 0 {
            self.error(
                &HeaderData::DepthStencilFormatWithDepth,
                fmt_args![h.pixel_depth, self.vk_str()],
            )?;
        }
        if h.face_count == 6 && h.pixel_depth != 0 {
            self.error(&HeaderData::CubeWithDepth, fmt_args![h.pixel_depth])?;
        }

        // Detect dimension counts
        if h.pixel_depth != 0 {
            self.dimension_count = 3;
            if h.layer_count != 0 {
                self.warning(&HeaderData::ThreeDArray, vec![])?; // Warning on 3D Array textures
            }
        } else if h.pixel_height != 0 {
            self.dimension_count = 2;
        } else {
            self.dimension_count = 1;
        }

        // Validate layerCount to actual number of layers.
        self.num_layers = h.layer_count.max(1);

        // Validate faceCount
        if h.face_count != 6 && h.face_count != 1 {
            self.error(&HeaderData::InvalidFaceCount, fmt_args![h.face_count])?;
        }

        // Validate levelCount
        if is_format_block_compressed(vk_format) && h.level_count == 0 {
            self.error(
                &HeaderData::BlockCompressedNoLevel,
                fmt_args![self.vk_str()],
            )?;
        }
        if is_supercompression_block_compressed(scheme) && h.level_count == 0 {
            self.error(
                &HeaderData::BlockCompressedNoLevel,
                fmt_args![to_string_scheme(scheme)],
            )?;
        }

        self.num_levels = h.level_count.max(1);

        // This test works for arrays too because height or depth will be 0.
        let max_dim = h.pixel_width.max(h.pixel_height).max(h.pixel_depth);
        if max_dim < 1u32.wrapping_shl(self.num_levels - 1) {
            // Can't have more mip levels than 1 + log2(max(width, height, depth))
            self.error(
                &HeaderData::TooManyMipLevels,
                fmt_args![self.num_levels, max_dim],
            )?;
        }

        // Validate supercompressionScheme
        if SupercmpScheme::BEGIN_VENDOR_RANGE.0 <= scheme.0
            && scheme.0 <= SupercmpScheme::END_VENDOR_RANGE.0
        {
            self.warning(
                &HeaderData::VendorSupercompression,
                fmt_args![to_string_scheme(scheme)],
            )?;
        } else if SupercmpScheme::END_RANGE.0 < scheme.0 {
            // (`scheme < KTX_SS_BEGIN_RANGE` is never true for the unsigned value.)
            self.error(
                &HeaderData::InvalidSupercompression,
                fmt_args![to_string_scheme(scheme)],
            )?;
        }

        // Validate GLTF KHR_texture_basisu compatibility, if needed
        if self.check_gltf_basisu {
            if !matches!(
                scheme,
                SupercmpScheme::NONE | SupercmpScheme::BASIS_LZ | SupercmpScheme::ZSTD
            ) {
                self.error(
                    &HeaderData::InvalidSupercompressionGLTFBU,
                    fmt_args![to_string_scheme(scheme)],
                )?;
            }
            // NOTE: pixelHeight == 0 already covered by other error codes
            if h.pixel_depth != 0 {
                self.error(
                    &HeaderData::InvalidTextureTypeGLTFBU,
                    fmt_args!["pixelDepth", h.pixel_depth, 0u32],
                )?;
            }
            if h.layer_count != 0 {
                self.error(
                    &HeaderData::InvalidTextureTypeGLTFBU,
                    fmt_args!["layerCount", h.layer_count, 0u32],
                )?;
            }
            if h.face_count != 1 {
                self.error(
                    &HeaderData::InvalidTextureTypeGLTFBU,
                    fmt_args!["faceCount", h.face_count, 1u32],
                )?;
            }
            if !h.pixel_width.is_multiple_of(4) {
                self.error(
                    &HeaderData::InvalidPixelWidthHeightGLTFBU,
                    fmt_args!["pixelWidth", h.pixel_width],
                )?;
            }
            if !h.pixel_height.is_multiple_of(4) {
                self.error(
                    &HeaderData::InvalidPixelWidthHeightGLTFBU,
                    fmt_args!["pixelHeight", h.pixel_height],
                )?;
            }
            // Check that levelCount is 1 or that the full mip pyramid is present
            let full = 1 + log2(max_dim);
            if h.level_count != 1 && h.level_count != full {
                self.error(
                    &HeaderData::InvalidLevelCountGLTFBU,
                    fmt_args![h.level_count, full],
                )?;
            }
        }
        Ok(())
    }

    fn validate_indices(&mut self) -> VResult {
        let h = self.header;
        let scheme = h.supercompression_scheme;
        let dfd = h.data_format_descriptor;
        let kvd = h.key_value_data;
        let sgd = h.supercompression_global_data;

        // Validate dataFormatDescriptor index
        if dfd.byte_offset == 0 || dfd.byte_length == 0 {
            self.error(
                &HeaderData::IndexDFDMissing,
                fmt_args![dfd.byte_offset, dfd.byte_length],
            )?;
        }

        let level_index_size = LEVEL_INDEX_ENTRY_SIZE as u64 * u64::from(self.num_levels);
        let mut expected = align(KTX2_HEADER_SIZE as u64 + level_index_size, 4);
        if expected != u64::from(dfd.byte_offset) {
            self.error(
                &HeaderData::IndexDFDInvalidOffset,
                fmt_args![dfd.byte_offset, expected],
            )?;
        }
        expected = expected.wrapping_add(u64::from(dfd.byte_length));

        if dfd.byte_offset != 0 && dfd.byte_length != 0 && kvd.byte_offset != 0 {
            let gap = kvd.byte_offset.wrapping_sub(dfd.byte_offset);
            if dfd.byte_length != gap {
                self.error(
                    &HeaderData::IndexDFDInvalidLength,
                    fmt_args![dfd.byte_length, gap],
                )?;
            }
        }

        // Validate keyValueData index
        if kvd.byte_length != 0 {
            expected = align(expected, 4);
            if expected != u64::from(kvd.byte_offset) {
                self.error(
                    &HeaderData::IndexKVDInvalidOffset,
                    fmt_args![kvd.byte_offset, expected],
                )?;
            }
            expected = expected.wrapping_add(u64::from(kvd.byte_length));
        } else if kvd.byte_offset != 0 {
            self.error(
                &HeaderData::IndexKVDOffsetWithoutLength,
                fmt_args![kvd.byte_offset],
            )?;
        }

        // Validate supercompressionGlobalData index
        if sgd.byte_length != 0 {
            expected = align(expected, 8);
            if expected != sgd.byte_offset {
                self.error(
                    &HeaderData::IndexSGDInvalidOffset,
                    fmt_args![sgd.byte_offset, expected],
                )?;
            }
        } else if sgd.byte_offset != 0 {
            self.error(
                &HeaderData::IndexSGDOffsetWithoutLength,
                fmt_args![sgd.byte_offset],
            )?;
        }

        if is_supercompression_with_global_data(scheme) {
            if sgd.byte_length == 0 {
                self.error(
                    &HeaderData::IndexSGDMissing,
                    fmt_args![to_string_scheme(scheme)],
                )?;
            }
        } else if is_supercompression_with_no_global_data(scheme) && sgd.byte_length != 0 {
            self.error(
                &HeaderData::IndexSGDNotApplicable,
                fmt_args![sgd.byte_length, to_string_scheme(scheme)],
            )?;
        }
        Ok(())
    }

    fn block_dimension(&self, i: usize) -> u32 {
        let parsed = self.parsed_block_dimension.get(i).copied().flatten();
        u32::from(self.expected_block_dimension[i].or(parsed).unwrap_or(0))
    }

    fn calc_image_size(&self, level: u32) -> u64 {
        let level_width = self.header.pixel_width.wrapping_shr(level).max(1);
        let level_height = self.header.pixel_height.wrapping_shr(level).max(1);
        let block_count_x = ceil_div(level_width, 1 + self.block_dimension(0));
        let block_count_y = ceil_div(level_height, 1 + self.block_dimension(1));
        let block_size = match self.expected_bytes_planes {
            Some(p) => p[0],
            None => self.parsed_block_byte_length.unwrap_or(0),
        };
        u64::from(
            block_count_x
                .wrapping_mul(block_count_y)
                .wrapping_mul(u32::from(block_size)),
        )
    }

    fn calc_layer_size(&self, level: u32) -> u64 {
        let level_depth = self.header.pixel_depth.wrapping_shr(level).max(1);
        let block_count_z = ceil_div(level_depth, 1 + self.block_dimension(2));
        // As there are no 3D cubemaps, the image's z block count will always be 1 for
        // cubemaps and numFaces will always be 1 for 3D textures so the multiplication is safe.
        self.calc_image_size(level)
            .wrapping_mul(u64::from(block_count_z))
            .wrapping_mul(u64::from(self.header.face_count))
    }

    fn calc_level_size(&self, level: u32) -> u64 {
        self.calc_layer_size(level)
            .wrapping_mul(u64::from(self.num_layers))
    }

    fn calc_level_offset(&self, first_level_offset: u64, alignment: u64, level: u32) -> u64 {
        // Calculate the expected base offset in the file
        let mut level_offset = align(first_level_offset, alignment);
        let mut i = self.num_levels - 1;
        while i > level {
            level_offset = level_offset.wrapping_add(self.calc_level_size(i));
            level_offset = align(level_offset, alignment);
            i -= 1;
        }
        level_offset
    }

    fn validate_level_index(&mut self) -> VResult {
        let level_index_offset = KTX2_HEADER_SIZE as u64;
        let level_index_size = LEVEL_INDEX_ENTRY_SIZE as u64 * u64::from(self.num_levels);
        let raw = self.read(level_index_offset, level_index_size, "the level index")?;
        self.level_indices = raw
            .chunks(LEVEL_INDEX_ENTRY_SIZE)
            .filter_map(LevelIndexEntry::from_bytes)
            .collect();

        let h = self.header;
        let block_byte_length: u8 = match self.expected_bytes_planes {
            Some(p) => p[0],
            None => match self.parsed_block_byte_length {
                Some(0) => 1,
                v => v.unwrap_or(0),
            },
        };
        let required_level_alignment: u64 = if h.supercompression_scheme != SupercmpScheme::NONE {
            1
        } else {
            lcm(u64::from(block_byte_length), 4)
        };

        let sgd = h.supercompression_global_data;
        let kvd = h.key_value_data;
        let dfd = h.data_format_descriptor;
        let mut expected_first_level_offset = if sgd.byte_length != 0 {
            sgd.byte_length.wrapping_add(sgd.byte_offset)
        } else if kvd.byte_length != 0 {
            u64::from(kvd.byte_length.wrapping_add(kvd.byte_offset))
        } else if dfd.byte_length != 0 {
            u64::from(dfd.byte_length.wrapping_add(dfd.byte_offset))
        } else {
            level_index_offset + level_index_size
        };
        expected_first_level_offset = align(expected_first_level_offset, required_level_alignment);

        // The first (largest) mip level is the first in the index and the last in the file.
        for i in 1..self.level_indices.len() {
            let (prev, cur) = (self.level_indices[i - 1], self.level_indices[i]);
            if cur.uncompressed_byte_length > prev.uncompressed_byte_length {
                self.error(
                    &LevelIndex::IncorrectIndexOrder,
                    fmt_args![
                        i - 1,
                        prev.uncompressed_byte_length,
                        i,
                        cur.uncompressed_byte_length
                    ],
                )?;
            }
            if cur.byte_offset > prev.byte_offset {
                self.error(
                    &LevelIndex::IncorrectLevelOrder,
                    fmt_args![i - 1, prev.byte_offset, i, cur.byte_offset],
                )?;
            }
        }

        let mut last_byte_offset = expected_first_level_offset;
        let mut last_byte_length = 0u64;
        let vk_format = h.vk_format;
        let scheme = h.supercompression_scheme;

        // Count down, so we can check the distance between levels for the UNDEFINED and SUPERCOMPRESSION cases.
        for index in (0..self.level_indices.len() as u32).rev() {
            let level = self.level_indices[index as usize];

            // Validate byteOffset
            let known_level_offset =
                vk_format != VkFormat::UNDEFINED && scheme == SupercmpScheme::NONE;
            // If the exact level sizes are unknown we have to trust the byteLengths.
            let expected_offset = if known_level_offset {
                self.calc_level_offset(expected_first_level_offset, required_level_alignment, index)
            } else {
                align(
                    last_byte_offset.wrapping_add(last_byte_length),
                    required_level_alignment,
                )
            };

            if required_level_alignment != 0
                && !level.byte_offset.is_multiple_of(required_level_alignment)
            {
                self.error(
                    &LevelIndex::IncorrectByteOffsetUnaligned,
                    fmt_args![
                        index,
                        level.byte_offset,
                        required_level_alignment,
                        expected_offset
                    ],
                )?;
            } else if level.byte_offset != expected_offset {
                self.error(
                    &LevelIndex::IncorrectByteOffset,
                    fmt_args![index, level.byte_offset, expected_offset],
                )?;
            }

            // Workaround: Disable ByteLength validations for the 3D ASTC encoder which currently ignores partial Z blocks in our test files
            let disable_byte_length_validation = is_format_3d_block_compressed(vk_format)
                && !h
                    .pixel_depth
                    .is_multiple_of(u32::from(self.expected_block_dimension[2].unwrap_or(0)) + 1);

            if !disable_byte_length_validation {
                // Validate byteLength
                if vk_format != VkFormat::UNDEFINED && scheme == SupercmpScheme::NONE {
                    let expected_length = self.calc_level_size(index);
                    if level.byte_length != expected_length {
                        self.error(
                            &LevelIndex::IncorrectByteLength,
                            fmt_args![index, level.byte_length, expected_length],
                        )?;
                    }
                }

                // Validate uncompressedByteLength
                if scheme == SupercmpScheme::BASIS_LZ {
                    if level.uncompressed_byte_length != 0 {
                        self.error(
                            &LevelIndex::NonZeroUBLForBLZE,
                            fmt_args![index, level.uncompressed_byte_length],
                        )?;
                    }
                } else if scheme == SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE {
                    if level.uncompressed_byte_length != 0 {
                        self.error(
                            &LevelIndex::NonZeroUBLForUH6X6IE,
                            fmt_args![index, level.uncompressed_byte_length],
                        )?;
                    }
                } else if vk_format != VkFormat::UNDEFINED {
                    if scheme == SupercmpScheme::NONE {
                        let expected_uncompressed = self.calc_level_size(index);
                        if level.uncompressed_byte_length != expected_uncompressed {
                            self.error(
                                &LevelIndex::IncorrectUncompressedByteLength,
                                fmt_args![
                                    index,
                                    level.uncompressed_byte_length,
                                    expected_uncompressed
                                ],
                            )?;
                        }
                    }
                } else if level.uncompressed_byte_length == 0 {
                    self.error(&LevelIndex::ZeroUncompressedLength, fmt_args![index])?;
                } else if !level
                    .uncompressed_byte_length
                    .is_multiple_of(u64::from(h.face_count.wrapping_mul(self.num_layers).max(1)))
                {
                    // On the other branches uncompressedByteLength is always checked exactly,
                    // so this is the only branch where this checks yields useful information
                    self.error(
                        &LevelIndex::InvalidUncompressedLength,
                        fmt_args![index, level.uncompressed_byte_length],
                    )?;
                }
            }

            last_byte_offset = level.byte_offset;
            last_byte_length = level.byte_length;
        }
        Ok(())
    }

    fn calculate_expected_dfd(&mut self, format: VkFormat) -> VResult {
        if format == VkFormat::UNDEFINED || !is_format_valid(format) || is_prohibited_format(format)
        {
            return Ok(());
        }
        let Some(dfd) = vk2dfd(format) else {
            return self.error(
                &Validator::CreateExpectedDFDFailure,
                fmt_args![to_string_vk_format(format)],
            );
        };
        let bdfd: Vec<u8> = dfd.iter().skip(1).flat_map(|w| w.to_le_bytes()).collect();
        let block = Bdfd::parse(&bdfd);
        let sample_count = (block.descriptor_block_size.wrapping_sub(24) / 16) as usize;
        self.expected_samples = Some(parse_samples(
            bdfd.get(24..).unwrap_or_default(),
            sample_count,
        ));
        self.expected_color_model = Some(block.model);
        self.expected_bytes_planes = Some(block.bytes_planes);
        for (e, d) in self
            .expected_block_dimension
            .iter_mut()
            .zip(block.texel_block_dimension)
        {
            *e = Some(d as u8);
        }
        self.expected_type_size = Some(vk_format_type_size(format));
        Ok(())
    }

    fn validate_dfd(&mut self) -> VResult {
        let dfd = self.header.data_format_descriptor;
        if dfd.byte_offset == 0 || dfd.byte_length == 0 {
            return Ok(()); // There is no DFD block
        }
        let buffer = self.read(
            u64::from(dfd.byte_offset),
            u64::from(dfd.byte_length),
            "the DFD",
        )?;
        let end = buffer.len();
        let dfd_total_size = word(&buffer, 0);
        let mut it = 4usize;

        if dfd.byte_length != dfd_total_size {
            self.error(
                &DFD::SizeMismatch,
                fmt_args![dfd.byte_length, dfd_total_size],
            )?;
        }

        let mut num_blocks = 0u32;
        let mut found_bdfd = false;

        while it < end {
            let remaining = end - it;
            num_blocks += 1;
            if num_blocks > MAX_NUM_DFD_BLOCKS {
                self.warning(&DFD::TooManyDFDBlocks, fmt_args![num_blocks, remaining])?;
                break;
            }
            if remaining < 8 {
                self.error(&DFD::NotEnoughDataForBlockHeader, fmt_args![remaining])?;
                break;
            }
            let block_bytes = buffer.get(it..).unwrap_or_default();
            let block_header = Bdfd::parse(block_bytes.get(..8).unwrap_or_default());
            let size = block_header.descriptor_block_size;

            if size < 8 {
                self.error(
                    &DFD::DescriptorBlockSizeTooSmall,
                    fmt_args![num_blocks, size],
                )?;
            } else if size as usize > remaining {
                self.error(
                    &DFD::DescriptorBlockSizeTooBig,
                    fmt_args![num_blocks, size, remaining],
                )?;
            } else if block_header.vendor_id == KHR_DF_VENDORID_KHRONOS
                && block_header.descriptor_type == KHR_DF_KHR_DESCRIPTORTYPE_BASICFORMAT
            {
                if std::mem::replace(&mut found_bdfd, true) {
                    self.warning(&DFD::MultipleBDFD, fmt_args![num_blocks])?;
                } else if size < 24 {
                    self.error(
                        &DFD::BasicDescriptorBlockSizeTooSmall,
                        fmt_args![num_blocks, size],
                    )?;
                } else if num_blocks != 1 {
                    // The Basic DFD block has to be the first block.
                    found_bdfd = false;
                } else {
                    let block = Bdfd::parse(block_bytes);
                    if !(block.descriptor_block_size - 24).is_multiple_of(16) {
                        self.error(
                            &DFD::BasicDescriptorBlockSizeInvalid,
                            fmt_args![num_blocks, block.descriptor_block_size],
                        )?;
                    }
                    let num_samples_stored = (block.descriptor_block_size - 24) / 16;
                    let num_samples_validating = MAX_NUM_BDFD_SAMPLES.min(num_samples_stored);
                    if num_samples_stored > MAX_NUM_BDFD_SAMPLES {
                        self.warning(
                            &DFD::TooManySamples,
                            fmt_args![
                                num_blocks,
                                num_samples_stored,
                                MAX_NUM_BDFD_SAMPLES,
                                num_samples_stored - num_samples_validating,
                                u64::from(block.descriptor_block_size)
                                    - 24
                                    - u64::from(num_samples_validating) * 16
                            ],
                        )?;
                    }
                    // Samples are located at the end of the block
                    let samples = parse_samples(
                        block_bytes.get(24..).unwrap_or_default(),
                        num_samples_validating as usize,
                    );
                    let words: Vec<u32> = (0..buffer.len().div_ceil(4))
                        .map(|i| word(&buffer, i))
                        .collect();
                    self.validate_dfd_basic(num_blocks, &words, &block, samples)?;
                }
            } else if block_header.vendor_id == KHR_DF_VENDORID_KHRONOS
                && (block_header.descriptor_type == KHR_DF_KHR_DESCRIPTORTYPE_ADDITIONAL_DIMENSIONS
                    || block_header.descriptor_type == KHR_DF_KHR_DESCRIPTORTYPE_ADDITIONAL_PLANES)
            {
                // TODO (C++): Implement DFD validation for ADDITIONAL_DIMENSIONS / ADDITIONAL_PLANES
            } else {
                self.warning(
                    &DFD::UnknownDFDBlock,
                    fmt_args![
                        num_blocks,
                        to_string_vendor_id(block_header.vendor_id),
                        to_string_descriptor_type(
                            block_header.vendor_id,
                            block_header.descriptor_type
                        )
                    ],
                )?;
            }

            it += size.max(8) as usize;
        }

        if !found_bdfd {
            self.error(&DFD::MissingBDFD, vec![])?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn validate_dfd_basic(
        &mut self,
        block_index: u32,
        dfd: &[u32],
        block: &Bdfd,
        samples: Vec<SampleType>,
    ) -> VResult {
        let h = self.header;
        let vk_format = h.vk_format;
        let scheme = h.supercompression_scheme;
        let model = block.model;
        let transfer = block.transfer;
        let bp = block.bytes_planes;
        let bp_args = |v: Vec<Arg>| -> Vec<Arg> {
            let mut a = v;
            a.extend(bp.iter().map(|&b| Arg::from(b)));
            a
        };

        self.num_samples = samples.len() as u32;
        self.parsed_color_model = Some(model);
        self.parsed_transfer_function = Some(transfer);
        self.parsed_block_byte_length = Some(bp[0]);
        for i in 0..3 {
            self.parsed_block_dimension[i] = Some(block.texel_block_dimension[i] as u8);
        }

        if model == KHR_DF_MODEL_UASTC_HDR_4x4 && vk_format != VkFormat::ASTC_4x4_SFLOAT_BLOCK {
            self.error(
                &DFD::UH4X4ModelRequiresASTC4X4FVkFormat,
                fmt_args![self.vk_str()],
            )?;
        }

        // Validate versionNumber
        if block.version_number != KHR_DF_VERSIONNUMBER_1_3 {
            self.error(
                &DFD::BasicVersionNotSupported,
                fmt_args![block_index, to_string_version_number(block.version_number)],
            )?;
        }

        // Validate transferFunction
        if transfer > KHR_DF_TRANSFER_HLG_UNNORMALIZED_OETF {
            self.error(
                &DFD::BasicInvalidTransferFunction,
                fmt_args![block_index, transfer],
            )?;
        }

        if is_format_srgb(vk_format) && transfer != KHR_DF_TRANSFER_SRGB {
            self.error(
                &DFD::BasicSRGBMismatch,
                fmt_args![block_index, to_string_transfer(transfer), self.vk_str()],
            )?;
        }
        if is_format_not_srgb_but_has_srgb_variant(vk_format) && transfer == KHR_DF_TRANSFER_SRGB {
            self.error(
                &DFD::BasicNotSRGBMismatch,
                fmt_args![block_index, self.vk_str()],
            )?;
        }

        // Validate colorModel
        if is_format_422(vk_format) {
            if !is_prohibited_format(vk_format) && model != KHR_DF_MODEL_YUVSDA {
                self.error(
                    &DFD::IncorrectModelFor422,
                    fmt_args![block_index, to_string_model(model), self.vk_str()],
                )?;
            }
        } else if is_format_block_compressed(vk_format) {
            let expected_bc = get_color_model_for_block_compressed_format(vk_format);
            if model != expected_bc && model != KHR_DF_MODEL_UASTC_HDR_4x4 {
                self.error(
                    &DFD::IncorrectModelForBlock,
                    fmt_args![
                        block_index,
                        to_string_model(model),
                        self.vk_str(),
                        to_string_model(expected_bc)
                    ],
                )?;
            }
        } else if vk_format != VkFormat::UNDEFINED && model != KHR_DF_MODEL_RGBSDA {
            self.error(
                &DFD::IncorrectModelForRGB,
                fmt_args![block_index, to_string_model(model), self.vk_str()],
            )?;
        }

        if scheme == SupercmpScheme::BASIS_LZ && model != KHR_DF_MODEL_ETC1S {
            self.error(
                &DFD::IncorrectModelForBLZE,
                fmt_args![block_index, to_string_model(model)],
            )?;
        }

        // Check GLTF KHR_texture_basisu specific errors
        if self.check_gltf_basisu {
            match model {
                KHR_DF_MODEL_ETC1S => {
                    // Supercompression already verified above, only need to check samples.
                    if let Some(s0) = samples.first() {
                        let ch = |i: usize| samples.get(i).map_or(0, |s| s.channel_id);
                        match s0.channel_id {
                            KHR_DF_CHANNEL_ETC1S_RGB => {
                                if samples.len() > 1 && ch(1) != KHR_DF_CHANNEL_ETC1S_AAA {
                                    self.error(
                                        &DFD::InvalidChannelGLTFBU,
                                        fmt_args![
                                            block_index,
                                            "KHR_DF_MODEL_ETC1S",
                                            2u32,
                                            to_string_channel(KHR_DF_MODEL_ETC1S, ch(1)),
                                            "KHR_DF_CHANNEL_ETC1S_AAA when sample #0 channelId is KHR_DF_CHANNEL_ETC1S_RGB"
                                        ],
                                    )?;
                                }
                            }
                            KHR_DF_CHANNEL_ETC1S_RRR => {
                                if samples.len() > 1 && ch(1) != KHR_DF_CHANNEL_ETC1S_GGG {
                                    self.error(
                                        &DFD::InvalidChannelGLTFBU,
                                        fmt_args![
                                            block_index,
                                            "KHR_DF_MODEL_ETC1S",
                                            2u32,
                                            to_string_channel(KHR_DF_MODEL_ETC1S, ch(1)),
                                            "KHR_DF_CHANNEL_ETC1S_GGG when sample #0 channelId is KHR_DF_CHANNEL_ETC1S_RRR"
                                        ],
                                    )?;
                                }
                            }
                            c => {
                                self.error(
                                    &DFD::InvalidChannelGLTFBU,
                                    fmt_args![
                                        block_index,
                                        "KHR_DF_MODEL_ETC1S",
                                        1u32,
                                        to_string_channel(KHR_DF_MODEL_ETC1S, c),
                                        "KHR_DF_CHANNEL_ETC1S_RGB or KHR_DF_CHANNEL_ETC1S_RRR"
                                    ],
                                )?;
                            }
                        }
                    }
                }
                KHR_DF_MODEL_UASTC => {
                    if scheme != SupercmpScheme::NONE && scheme != SupercmpScheme::ZSTD {
                        self.error(
                            &DFD::IncompatibleModelGLTFBU,
                            fmt_args![
                                block_index,
                                "KHR_DF_MODEL_UASTC",
                                to_string_scheme(scheme),
                                "KTX_SS_NONE or KTX_SS_ZSTD"
                            ],
                        )?;
                    }
                    if let Some(s0) = samples.first()
                        && !matches!(
                            s0.channel_id,
                            KHR_DF_CHANNEL_UASTC_RGB
                                | KHR_DF_CHANNEL_UASTC_RGBA
                                | KHR_DF_CHANNEL_UASTC_RRR
                                | KHR_DF_CHANNEL_UASTC_RG
                        )
                    {
                        self.error(
                            &DFD::InvalidChannelGLTFBU,
                            fmt_args![
                                block_index,
                                "KHR_DF_MODEL_UASTC",
                                0u32,
                                to_string_channel(KHR_DF_MODEL_UASTC, s0.channel_id),
                                "KHR_DF_CHANNEL_UASTC_RGB, KHR_DF_CHANNEL_UASTC_RGBA, KHR_DF_CHANNEL_UASTC_RRR, or KHR_DF_CHANNEL_UASTC_RG"
                            ],
                        )?;
                    }
                }
                _ => {
                    self.error(
                        &DFD::IncorrectModelGLTFBU,
                        fmt_args![block_index, to_string_model(model)],
                    )?;
                }
            }

            if (block.primaries != KHR_DF_PRIMARIES_BT709 || transfer != KHR_DF_TRANSFER_SRGB)
                && (block.primaries != KHR_DF_PRIMARIES_UNSPECIFIED
                    || transfer != KHR_DF_TRANSFER_LINEAR)
            {
                self.error(
                    &DFD::InvalidColorSpaceGLTFBU,
                    fmt_args![
                        block_index,
                        to_string_primaries(block.primaries),
                        to_string_transfer(transfer)
                    ],
                )?;
            }
        }

        // Validate colorPrimaries
        if !is_color_primaries_valid(block.primaries) {
            self.error(
                &DFD::InvalidColorPrimaries,
                fmt_args![block_index, block.primaries],
            )?;
        }

        // Validate samples[].channelId
        for (i, sample) in samples.iter().enumerate() {
            if !is_channel_id_valid(model, sample.channel_id) {
                self.error(
                    &DFD::InvalidChannelForModel,
                    fmt_args![
                        block_index,
                        i + 1,
                        to_string_channel(model, sample.channel_id),
                        to_string_model(model)
                    ],
                )?;
            }
        }

        // Validate: bytesPlanes, texelBlockDimensions and samples
        match scheme {
            SupercmpScheme::NONE | SupercmpScheme::ZSTD | SupercmpScheme::ZLIB => {
                if vk_format != VkFormat::UNDEFINED {
                    self.validate_dfd_samples_vk(block_index, dfd, block, &samples)?;
                } else if model == KHR_DF_MODEL_UASTC {
                    // Validate UASTC
                    if samples.len() != 1 {
                        self.error(
                            &DFD::InvalidSampleCount,
                            fmt_args![block_index, samples.len(), "UASTC", "1"],
                        )?;
                    }
                    if !block.match_texel_block_dimensions([3, 3, 0, 0]) {
                        let d = block.texel_block_dimension;
                        self.error(
                            &DFD::InvalidTexelBlockDimension,
                            fmt_args![
                                block_index,
                                d[0] + 1,
                                d[1] + 1,
                                d[2] + 1,
                                d[3] + 1,
                                4u32,
                                4u32,
                                1u32,
                                1u32,
                                "UASTC"
                            ],
                        )?;
                    }
                    if scheme != SupercmpScheme::NONE && bp[0] == 0 {
                        self.warning(
                            &DFD::BytesPlanesUnsized,
                            fmt_args![block_index, bp[0], to_string_scheme(scheme)],
                        )?;
                    } else if bp != [16, 0, 0, 0, 0, 0, 0, 0] {
                        let mut a = bp_args(fmt_args![block_index]);
                        a.extend(fmt_args![
                            "UASTC", 16u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32
                        ]);
                        self.error(&DFD::BytesPlanesMismatch, a)?;
                    }
                    if let Some(s0) = samples.first() {
                        if s0.bit_offset != 0 {
                            self.error(
                                &DFD::InvalidBitOffsetForUASTC,
                                fmt_args![block_index, 1u32, s0.bit_offset],
                            )?;
                        }
                        if s0.bit_length != 127 {
                            self.error(
                                &DFD::InvalidBitLengthForUASTC,
                                fmt_args![block_index, 1u32, s0.bit_length],
                            )?;
                        }
                        if s0.lower != 0 {
                            self.error(
                                &DFD::InvalidLower,
                                fmt_args![block_index, 1u32, s0.lower, "UASTC", 0u32],
                            )?;
                        }
                        if s0.upper != u32::MAX {
                            self.error(
                                &DFD::InvalidUpper,
                                fmt_args![
                                    block_index,
                                    1u32,
                                    s0.upper,
                                    "UASTC",
                                    "UINT32_MAX (0xFFFFFFFF)"
                                ],
                            )?;
                        }
                    }
                } else {
                    // Ensure there are at least some samples
                    if samples.is_empty() {
                        self.error(&DFD::ZeroSamples, fmt_args![block_index])?;
                    }
                    if scheme == SupercmpScheme::NONE {
                        if bp[0] == 0 {
                            self.error(&DFD::BytesPlane0Zero, fmt_args![block_index, bp[0]])?;
                        }
                        if bp[1..].iter().any(|&b| b != 0) {
                            self.error(
                                &DFD::MultiplaneFormatsNotSupported,
                                bp_args(fmt_args![block_index]),
                            )?;
                        }
                    }
                }
            }
            SupercmpScheme::BASIS_LZ => {
                // This descriptor should have 1 or 2 samples with bitLength 63 and bitOffsets 0 and 64.
                if samples.is_empty() || samples.len() > 2 {
                    self.error(
                        &DFD::InvalidSampleCount,
                        fmt_args![block_index, samples.len(), "BasisLZ/ETC1S", "1 or 2"],
                    )?;
                }
                if !block.match_texel_block_dimensions([3, 3, 0, 0]) {
                    let d = block.texel_block_dimension;
                    self.error(
                        &DFD::InvalidTexelBlockDimension,
                        fmt_args![
                            block_index,
                            d[0] + 1,
                            d[1] + 1,
                            d[2] + 1,
                            d[3] + 1,
                            4u32,
                            4u32,
                            1u32,
                            1u32,
                            "BasisLZ/ETC1S"
                        ],
                    )?;
                }
                if bp[0] == 0 {
                    self.warning(
                        &DFD::BytesPlanesUnsized,
                        fmt_args![block_index, bp[0], to_string_scheme(scheme)],
                    )?;
                } else {
                    let expected_bytes_plane1: u8 = if samples.len() == 2 { 8 } else { 0 };
                    let ss_type = format!("BasisLZ_ETC1S with {} slices", samples.len());
                    if bp != [8, expected_bytes_plane1, 0, 0, 0, 0, 0, 0] {
                        let mut a = bp_args(fmt_args![block_index]);
                        a.extend(fmt_args![
                            ss_type,
                            8u32,
                            expected_bytes_plane1,
                            0u32,
                            0u32,
                            0u32,
                            0u32,
                            0u32,
                            0u32
                        ]);
                        self.error(&DFD::BytesPlanesMismatch, a)?;
                    }
                }
                for (i, s) in samples.iter().take(2).enumerate() {
                    let i = i as u32;
                    let expected_offset = if i == 0 { 0u32 } else { 64u32 };
                    if s.bit_offset != expected_offset {
                        self.error(
                            &DFD::InvalidBitOffsetForBLZE,
                            fmt_args![block_index, i + 1, s.bit_offset, expected_offset],
                        )?;
                    }
                    if s.bit_length != 63 {
                        self.error(
                            &DFD::InvalidBitLengthForBLZE,
                            fmt_args![block_index, i + 1, s.bit_length],
                        )?;
                    }
                    if s.lower != 0 {
                        self.error(
                            &DFD::InvalidLower,
                            fmt_args![block_index, i + 1, s.lower, "BasisLZ/ETC1S", 0u32],
                        )?;
                    }
                    if s.upper != u32::MAX {
                        self.error(
                            &DFD::InvalidUpper,
                            fmt_args![
                                block_index,
                                i + 1,
                                s.upper,
                                "BasisLZ/ETC1S",
                                "UINT32_MAX (0xFFFFFFFF)"
                            ],
                        )?;
                    }
                }
            }
            _ => {}
        }

        // Checks that were deferred during header parsing until the BDFD is available
        if vk_format == VkFormat::UNDEFINED
            && !is_supercompression_block_compressed(scheme)
            && is_color_model_block_compressed(model)
        {
            if h.pixel_height == 0 {
                self.error(
                    &HeaderData::BlockCompressedNoHeight,
                    fmt_args![to_string_model(model)],
                )?;
            }
            if h.level_count == 0 {
                self.error(
                    &HeaderData::BlockCompressedNoLevel,
                    fmt_args![to_string_model(model)],
                )?;
            }
        }

        if vk_format != VkFormat::UNDEFINED && !is_format_block_compressed(vk_format) {
            if is_color_model_block_compressed(model) {
                if h.type_size != 1 {
                    self.error(
                        &HeaderData::TypeSizeNotOne,
                        fmt_args![to_string_model(model)],
                    )?;
                }
            } else if let Some(ts) = self.expected_type_size
                && h.type_size != ts
            {
                self.error(
                    &HeaderData::TypeSizeMismatch,
                    fmt_args![h.type_size, self.vk_str(), ts],
                )?;
            }
        }
        Ok(())
    }

    /// The `vkFormat != VK_FORMAT_UNDEFINED` branch of the sample checks.
    fn validate_dfd_samples_vk(
        &mut self,
        block_index: u32,
        dfd: &[u32],
        block: &Bdfd,
        samples: &[SampleType],
    ) -> VResult {
        let h = self.header;
        let vk_format = h.vk_format;
        let scheme = h.supercompression_scheme;
        let model = block.model;
        let transfer = block.transfer;
        let bp = block.bytes_planes;
        let vk = self.vk_str();

        if let Some(mut expected_samples) = self.expected_samples.take() {
            if samples.len() != expected_samples.len() {
                self.error(
                    &DFD::SampleCountMismatch,
                    fmt_args![
                        block_index,
                        samples.len(),
                        vk.clone(),
                        expected_samples.len()
                    ],
                )?;
            }
            // UASTC 4x4 requires lower to be 0.
            if model == KHR_DF_MODEL_UASTC_HDR_4x4 {
                for e in expected_samples.iter_mut() {
                    e.lower = 0;
                    e.qualifier_signed = 0;
                }
            }
            let expected_model = self
                .expected_color_model
                .unwrap_or(KHR_DF_MODEL_UNSPECIFIED);
            let result = (|| -> VResult {
                for (i, (parsed, expected)) in
                    samples.iter().zip(expected_samples.iter()).enumerate()
                {
                    let n = i + 1;
                    let mismatch = |this: &mut Self, field: &str, p: Arg, e: Arg| {
                        this.error(
                            &DFD::FormatMismatch,
                            vec![
                                Arg::from(block_index),
                                Arg::from(n),
                                Arg::from(field),
                                p,
                                e,
                                Arg::from(vk.as_str()),
                            ],
                        )
                    };
                    if parsed.bit_offset != expected.bit_offset {
                        mismatch(
                            self,
                            "bitOffset",
                            parsed.bit_offset.into(),
                            expected.bit_offset.into(),
                        )?;
                    }
                    if parsed.bit_length != expected.bit_length {
                        mismatch(
                            self,
                            "bitLength",
                            parsed.bit_length.into(),
                            expected.bit_length.into(),
                        )?;
                    }
                    if parsed.channel_id != expected.channel_id {
                        mismatch(
                            self,
                            "channelId",
                            to_string_channel(model, parsed.channel_id).into(),
                            to_string_channel(expected_model, expected.channel_id).into(),
                        )?;
                    }
                    if transfer == KHR_DF_TRANSFER_UNSPECIFIED || transfer == KHR_DF_TRANSFER_LINEAR
                    {
                        if parsed.qualifier_linear != 0 {
                            self.error(
                                &DFD::InvalidQualifierLinearForLinearTF,
                                fmt_args![
                                    block_index,
                                    n,
                                    parsed.qualifier_linear,
                                    to_string_transfer(transfer),
                                    0u32
                                ],
                            )?;
                        }
                    } else if transfer <= KHR_DF_TRANSFER_HLG_UNNORMALIZED_OETF {
                        if parsed.channel_id == KHR_DF_CHANNEL_RGBSDA_ALPHA
                            && parsed.qualifier_linear == 0
                        {
                            self.error(
                                &DFD::InvalidQualifierLinearForNonLinearTF,
                                fmt_args![
                                    block_index,
                                    n,
                                    parsed.qualifier_linear,
                                    to_string_transfer(transfer),
                                    parsed.channel_id,
                                    "(alpha)",
                                    1u32
                                ],
                            )?;
                        }
                        if parsed.channel_id != KHR_DF_CHANNEL_RGBSDA_ALPHA
                            && parsed.qualifier_linear != 0
                        {
                            self.error(
                                &DFD::InvalidQualifierLinearForNonLinearTF,
                                fmt_args![
                                    block_index,
                                    n,
                                    parsed.qualifier_linear,
                                    to_string_transfer(transfer),
                                    parsed.channel_id,
                                    "(non-alpha)",
                                    0u32
                                ],
                            )?;
                        }
                    }
                    if parsed.qualifier_exponent != expected.qualifier_exponent {
                        mismatch(
                            self,
                            "qualifierExponent",
                            parsed.qualifier_exponent.into(),
                            expected.qualifier_exponent.into(),
                        )?;
                    }
                    if parsed.qualifier_signed != expected.qualifier_signed {
                        mismatch(
                            self,
                            "qualifierSigned",
                            parsed.qualifier_signed.into(),
                            expected.qualifier_signed.into(),
                        )?;
                    }
                    if parsed.qualifier_float != expected.qualifier_float {
                        mismatch(
                            self,
                            "qualifierFloat",
                            parsed.qualifier_float.into(),
                            expected.qualifier_float.into(),
                        )?;
                    }
                    // For 4:2:2 formats the X sample positions can vary
                    if !is_format_422(vk_format)
                        && parsed.sample_position[0] != expected.sample_position[0]
                    {
                        mismatch(
                            self,
                            "samplePosition0",
                            parsed.sample_position[0].into(),
                            expected.sample_position[0].into(),
                        )?;
                    }
                    for k in 1..4 {
                        if parsed.sample_position[k] != expected.sample_position[k] {
                            let field =
                                ["", "samplePosition1", "samplePosition2", "samplePosition3"][k];
                            mismatch(
                                self,
                                field,
                                parsed.sample_position[k].into(),
                                expected.sample_position[k].into(),
                            )?;
                        }
                    }
                    if parsed.lower != expected.lower {
                        mismatch(
                            self,
                            "sampleLower",
                            parsed.lower.into(),
                            expected.lower.into(),
                        )?;
                    }
                    if parsed.upper != expected.upper {
                        mismatch(
                            self,
                            "sampleUpper",
                            parsed.upper.into(),
                            expected.upper.into(),
                        )?;
                    }
                }
                Ok(())
            })();
            self.expected_samples = Some(expected_samples);
            result?;
        }

        if scheme != SupercmpScheme::NONE && bp[0] == 0 {
            self.warning(
                &DFD::BytesPlanesUnsized,
                fmt_args![block_index, bp[0], to_string_scheme(scheme)],
            )?;
        } else if let Some(expected) = self.expected_bytes_planes
            && expected != bp
        {
            let mut a = fmt_args![block_index];
            a.extend(bp.iter().map(|&b| Arg::from(b)));
            a.push(Arg::from(vk.as_str()));
            a.extend(expected.iter().map(|&b| Arg::from(b)));
            self.error(&DFD::BytesPlanesMismatch, a)?;
        }

        if !is_format_block_compressed(vk_format) && !is_prohibited_format(vk_format) {
            let (mut r, mut g, mut b, mut a) = Default::default();
            let mut component_byte_length = 0u32;
            let mut result = interpret_dfd(
                dfd,
                &mut r,
                &mut g,
                &mut b,
                &mut a,
                &mut component_byte_length,
            );

            // Reset the "false" positive error in interpretDFD with VK_FORMAT_E5B9G9R9_UFLOAT_PACK32
            if vk_format == VkFormat::E5B9G9R9_UFLOAT_PACK32
                && result == I_UNSUPPORTED_NONTRIVIAL_ENDIANNESS
            {
                result = 0;
            }
            if result >= I_UNSUPPORTED_ERROR_BIT {
                match result {
                    I_UNSUPPORTED_MULTIPLE_PLANES => {
                        let mut args = fmt_args![block_index];
                        args.extend(bp.iter().map(|&b| Arg::from(b)));
                        self.error(&DFD::MultiplaneFormatsNotSupported, args)?;
                    }
                    I_UNSUPPORTED_MIXED_CHANNELS => {
                        self.error(&DFD::InterpretDFDMixedChannels, fmt_args![block_index])?;
                    }
                    I_UNSUPPORTED_MULTIPLE_SAMPLE_LOCATIONS => {
                        self.error(&DFD::InterpretDFDMultisample, fmt_args![block_index])?;
                    }
                    I_UNSUPPORTED_NONTRIVIAL_ENDIANNESS => {
                        self.error(
                            &DFD::InterpretDFDNonTrivialEndianness,
                            fmt_args![block_index],
                        )?;
                    }
                    // I_UNSUPPORTED_CHANNEL_TYPES: already checked elsewhere with more detailed error message
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn validate_kvd(&mut self) -> VResult {
        let kvd = self.header.key_value_data;
        if kvd.byte_offset == 0 || kvd.byte_length == 0 {
            return Ok(()); // There is no KVD block
        }
        let buffer = self.read(
            u64::from(kvd.byte_offset),
            u64::from(kvd.byte_length),
            "the Key/Value Data",
        )?;
        let end = buffer.len();

        // (key, value offset, value size)
        let mut entries: Vec<(Vec<u8>, usize, u32)> = Vec::new();
        let mut keys: HashSet<Vec<u8>> = HashSet::new();

        let mut num_kv_entry = 0u32;
        let mut entry = 0usize;
        while entry < end {
            let remaining = end - entry;
            num_kv_entry += 1;
            if num_kv_entry > MAX_NUM_KV_ENTRIES {
                self.warning(
                    &Metadata::TooManyEntries,
                    fmt_args![num_kv_entry - 1, remaining],
                )?;
                entry = end;
                break;
            }
            if remaining < 6 {
                self.error(&Metadata::NotEnoughDataForAnEntry, fmt_args![remaining])?;
                entry = end;
                break;
            }

            let mut size_kv = word(buffer.get(entry..).unwrap_or_default(), 0);
            let kv_start = entry + 4;

            if size_kv < 2 {
                self.error(&Metadata::KeyAndValueByteLengthTooSmall, fmt_args![size_kv])?;
            } else {
                if kv_start as u64 + u64::from(size_kv) > end as u64 {
                    let bytes_left = (end - kv_start) as u32;
                    self.error(
                        &Metadata::KeyAndValueByteLengthTooLarge,
                        fmt_args![size_kv, bytes_left],
                    )?;
                    size_kv = bytes_left; // Attempt recovery to read out at least the key
                }
                let kv = buffer
                    .get(kv_start..kv_start + size_kv as usize)
                    .unwrap_or_default();

                // Determine key, finding the null terminator
                let size_key = kv.iter().position(|&c| c == 0).unwrap_or(kv.len());

                if size_key == 0 {
                    self.error(&Metadata::KeyEmpty, vec![])?;
                } else {
                    let key_has_null = size_key != size_kv as usize;
                    let mut key = kv.get(..size_key).unwrap_or_default();
                    let value_ofs = kv_start + size_key + 1;
                    let size_value = if key_has_null {
                        size_kv - size_key as u32 - 1
                    } else {
                        0
                    };

                    // Check for BOM
                    if key.starts_with(b"\xEF\xBB\xBF") {
                        key = key.get(3..).unwrap_or_default();
                        self.error(&Metadata::KeyForbiddenBOM, fmt_args![key])?;
                    }
                    if let Some(invalid) = validate_utf8(key) {
                        self.error(&Metadata::KeyInvalidUTF8, fmt_args![key, invalid])?;
                    }
                    if !key_has_null {
                        self.error(&Metadata::KeyMissingNullTerminator, fmt_args![key])?;
                    }
                    entries.push((key.to_vec(), value_ofs, size_value));
                    if !keys.insert(key.to_vec()) {
                        self.error(&Metadata::DuplicateKey, fmt_args![key])?;
                    }
                }
            }

            // Finish entry
            entry = entry + 4 + size_kv as usize;
            self.validate_alignment_padding_zeros(
                &buffer,
                entry,
                end,
                4,
                &Metadata::PaddingNotZero,
                "after a Key-Value entry",
            )?;
            entry = align(entry as u64, 4) as usize;
        }

        if entry != end {
            // Being super explicit about the specs. This check might be overkill as other checks often cover this case
            self.error(&Metadata::SizesDontAddUp, fmt_args![entry, kvd.byte_length])?;
        }

        if self.header.supercompression_global_data.byte_length != 0 {
            self.validate_alignment_padding_zeros(
                &buffer,
                entry,
                end,
                8,
                &Metadata::PaddingNotZero,
                "after the last KVD entry",
            )?;
        }

        if !entries.is_sorted_by(|a, b| a.0 <= b.0) {
            self.error(&Metadata::OutOfOrder, vec![])?;
            entries.sort_by(|a, b| a.0.cmp(&b.0));
        }

        for (key, value_ofs, size) in &entries {
            let data = buffer
                .get(*value_ofs..*value_ofs + *size as usize)
                .unwrap_or_default();
            match key.as_slice() {
                b"KTXcubemapIncomplete" => self.validate_ktx_cubemap_incomplete(data)?,
                b"KTXorientation" => self.validate_ktx_orientation(data)?,
                b"KTXmapRange" => self.validate_ktx_map_range(data)?,
                b"KTXglFormat" => self.validate_ktx_gl_format(data)?,
                b"KTXdxgiFormat__" => self.validate_ktx_dxgi_format(data)?,
                b"KTXmetalPixelFormat" => self.validate_ktx_metal_pixel_format(data)?,
                b"KTXswizzle" => self.validate_ktx_swizzle(data)?,
                b"KTXwriter" => self.validate_ktx_writer(data)?,
                b"KTXwriterScParams" => self.validate_ktx_writer_sc_params(data)?,
                b"KTXastcDecodeMode" => self.validate_ktx_astc_decode_mode(data)?,
                b"KTXanimData" => self.validate_ktx_anim_data(data)?,
                k => {
                    if k.starts_with(b"KTX") || k.starts_with(b"ktx") {
                        self.error(&Metadata::UnknownReservedKey, fmt_args![k])?;
                    } else {
                        self.warning(&Metadata::CustomMetadata, fmt_args![k])?;
                    }
                }
            }
        }

        if self.found_ktx_anim_data && self.found_ktx_cubemap_incomplete {
            self.error(&Metadata::KTXanimDataWithCubeIncomplete, vec![])?;
        }
        if !self.found_ktx_writer {
            if self.found_ktx_writer_sc_params {
                self.error(&Metadata::KTXwriterRequiredButMissing, vec![])?;
            } else {
                self.warning(&Metadata::KTXwriterMissing, vec![])?;
            }
        }
        Ok(())
    }

    /// Splits a value that should be NUL terminated: (has NUL, string part).
    fn null_terminated(data: &[u8]) -> (bool, &[u8]) {
        let has_null = data.last() == Some(&0);
        let value = if has_null {
            data.get(..data.len() - 1).unwrap_or_default()
        } else {
            data
        };
        (has_null, value)
    }

    fn validate_ktx_cubemap_incomplete(&mut self, data: &[u8]) -> VResult {
        self.found_ktx_cubemap_incomplete = true;
        let size = data.len();
        if size != 1 {
            self.error(&Metadata::KTXcubemapIncompleteInvalidSize, fmt_args![size])?;
        }
        let Some(&first) = data.first() else {
            return Ok(());
        };
        let mut value = first;
        if value & 0b1100_0000 != 0 {
            self.error(
                &Metadata::KTXcubemapIncompleteInvalidBitSet,
                fmt_args![value],
            )?;
        }
        value &= 0b0011_1111; // Error recovery

        let pop_count = popcount(u32::from(value));
        if pop_count == 6 {
            self.warning(&Metadata::KTXcubemapIncompleteAllBitsSet, vec![])?;
        }
        if pop_count == 0 {
            self.error(&Metadata::KTXcubemapIncompleteNoBitSet, vec![])?;
        }
        let h = self.header;
        if pop_count != 0 && !h.layer_count.is_multiple_of(pop_count) {
            self.error(
                &Metadata::KTXcubemapIncompleteIncompatibleLayerCount,
                fmt_args![h.layer_count, pop_count],
            )?;
        }
        if h.face_count != 1 {
            self.error(
                &Metadata::KTXcubemapIncompleteWithFaceCountNot1,
                fmt_args![h.face_count],
            )?;
        }
        if h.pixel_height != h.pixel_width {
            self.error(
                &HeaderData::CubeHeightWidthMismatch,
                fmt_args![h.pixel_width, h.pixel_height],
            )?;
        }
        if h.pixel_depth != 0 {
            self.error(&HeaderData::CubeWithDepth, fmt_args![h.pixel_depth])?;
        }
        Ok(())
    }

    fn validate_ktx_orientation(&mut self, data: &[u8]) -> VResult {
        let (has_null, value) = Self::null_terminated(data);
        if !has_null {
            self.error(&Metadata::KTXorientationMissingNull, vec![])?;
        }
        let dims = self.dimension_count as usize;
        if value.len() != dims {
            self.error(
                &Metadata::KTXorientationIncorrectDimension,
                fmt_args![value.len(), self.dimension_count],
            )?;
        }
        let checks = [(b'r', b'l'), (b'd', b'u'), (b'o', b'i')];
        for (i, (a, b)) in checks.iter().enumerate() {
            if let Some(&c) = value.get(i)
                && dims > i
                && c != *a
                && c != *b
            {
                self.error(
                    &Metadata::KTXorientationInvalidValue,
                    fmt_args![i, Arg::C(c), Arg::C(*a), Arg::C(*b)],
                )?;
            }
        }
        if self.check_gltf_basisu && value != b"rd" {
            self.error(&Metadata::KTXorientationInvalidGLTFBU, fmt_args![value])?;
        }
        Ok(())
    }

    fn validate_ktx_map_range(&mut self, data: &[u8]) -> VResult {
        let dfd = self.header.data_format_descriptor;
        let buffer = self.read(
            u64::from(dfd.byte_offset),
            u64::from(dfd.byte_length),
            "the ff DFD",
        )?;
        let bdb = buffer.get(4..).unwrap_or_default();
        let block_size = word(bdb, 1) >> 16;
        let n_samples = block_size.wrapping_sub(24) / 16;
        for sample in 0..n_samples as usize {
            let qualifiers = (word(bdb, 6 + sample * 4) >> 24) & 0xF0;
            if qualifiers & KHR_DF_SAMPLE_DATATYPE_LINEAR == 0
                && qualifiers & KHR_DF_SAMPLE_DATATYPE_FLOAT == 0
            {
                return self.error(&Metadata::KTXmapRangeInvalidFormat, vec![]);
            }
        }
        if data.len() != 8 {
            return self.error(&Metadata::KTXmapRangeInvalidSize, fmt_args![data.len()]);
        }
        let scale = f32::from_bits(word(data, 0));
        let offset = f32::from_bits(word(data, 1));
        if !scale.is_finite() {
            return self.error(&Metadata::KTXmapRangeInvalidScale, fmt_args![scale]);
        }
        if !offset.is_finite() {
            return self.error(&Metadata::KTXmapRangeInvalidOffset, fmt_args![offset]);
        }
        Ok(())
    }

    fn validate_ktx_gl_format(&mut self, data: &[u8]) -> VResult {
        if self.vk_format() != VkFormat::UNDEFINED {
            self.error(&Metadata::KTXglFormatWithVkFormat, fmt_args![self.vk_str()])?;
        }
        if data.len() != 12 {
            return self.error(&Metadata::KTXglFormatInvalidSize, fmt_args![data.len()]);
        }
        let (gl_format, gl_type) = (word(data, 1), word(data, 2));
        if gl_format != 0 || gl_type != 0 {
            let scheme = self.scheme();
            if is_supercompression_block_compressed(scheme) {
                self.error(
                    &Metadata::KTXglFormatInvalidValueForCompressed,
                    fmt_args![gl_format, gl_type, to_string_scheme(scheme)],
                )?;
            } else if let Some(m) = self.parsed_color_model
                && is_color_model_block_compressed(m)
            {
                self.error(
                    &Metadata::KTXglFormatInvalidValueForCompressed,
                    fmt_args![gl_format, gl_type, to_string_model(m)],
                )?;
            }
        }
        Ok(())
    }

    fn validate_ktx_dxgi_format(&mut self, data: &[u8]) -> VResult {
        if self.vk_format() != VkFormat::UNDEFINED {
            self.error(
                &Metadata::KTXdxgiFormatWithVkFormat,
                fmt_args![self.vk_str()],
            )?;
        }
        if data.len() != 4 {
            self.error(&Metadata::KTXdxgiFormatInvalidSize, fmt_args![data.len()])?;
        }
        Ok(())
    }

    fn validate_ktx_metal_pixel_format(&mut self, data: &[u8]) -> VResult {
        if self.vk_format() != VkFormat::UNDEFINED {
            self.error(
                &Metadata::KTXmetalPixelFormatWithVkFormat,
                fmt_args![self.vk_str()],
            )?;
        }
        if data.len() != 4 {
            self.error(
                &Metadata::KTXmetalPixelFormatInvalidSize,
                fmt_args![data.len()],
            )?;
        }
        Ok(())
    }

    fn validate_ktx_swizzle(&mut self, data: &[u8]) -> VResult {
        let (has_null, value) = Self::null_terminated(data);
        if !has_null {
            self.error(&Metadata::KTXswizzleMissingNull, vec![])?;
        }
        if value.len() != 4 {
            self.error(&Metadata::KTXswizzleInvalidSize, fmt_args![data.len()])?;
        }
        for (i, &c) in value.iter().take(4).enumerate() {
            if !b"rgba01".contains(&c) {
                self.error(&Metadata::KTXswizzleInvalidValue, fmt_args![i, Arg::C(c)])?;
            }
        }
        let vk = self.vk_format();
        if is_format_stencil(vk) || is_format_depth(vk) {
            self.warning(
                &Metadata::KTXswizzleWithDepthOrStencil,
                fmt_args![self.vk_str()],
            )?;
        }
        if self.check_gltf_basisu && value != b"rgba" {
            self.error(&Metadata::KTXswizzleInvalidGLTFBU, fmt_args![value])?;
        }
        Ok(())
    }

    fn validate_ktx_writer(&mut self, data: &[u8]) -> VResult {
        self.found_ktx_writer = true;
        let (has_null, value) = Self::null_terminated(data);
        if !has_null {
            self.error(&Metadata::KTXwriterMissingNull, vec![])?;
        }
        if let Some(invalid) = validate_utf8(value) {
            self.error(&Metadata::KTXwriterInvalidUTF8, fmt_args![invalid])?;
        }
        Ok(())
    }

    fn validate_ktx_writer_sc_params(&mut self, data: &[u8]) -> VResult {
        self.found_ktx_writer_sc_params = true;
        let (has_null, value) = Self::null_terminated(data);
        if !has_null {
            self.error(&Metadata::KTXwriterScParamsMissingNull, vec![])?;
        }
        if let Some(invalid) = validate_utf8(value) {
            self.error(&Metadata::KTXwriterScParamsInvalidUTF8, fmt_args![invalid])?;
        }
        Ok(())
    }

    fn validate_ktx_astc_decode_mode(&mut self, data: &[u8]) -> VResult {
        let (has_null, value) = Self::null_terminated(data);
        if !has_null {
            self.error(&Metadata::KTXastcDecodeModeMissingNull, vec![])?;
        }
        if value != b"rgb9e5" && value != b"unorm8" {
            self.error(&Metadata::KTXastcDecodeModeInvalidValue, fmt_args![value])?;
        }
        match self.parsed_color_model {
            Some(m) if m != KHR_DF_MODEL_ASTC => {
                self.warning(
                    &Metadata::KTXastcDecodeModeNotASTC,
                    fmt_args![to_string_model(m)],
                )?;
            }
            _ => {
                if value == b"unorm8" && !is_format_astc_ldr(self.vk_format()) {
                    self.error(
                        &Metadata::KTXastcDecodeModeunorm8NotLDR,
                        fmt_args![self.vk_str()],
                    )?;
                }
            }
        }
        if let Some(tf) = self.parsed_transfer_function
            && tf == KHR_DF_TRANSFER_SRGB
        {
            self.warning(
                &Metadata::KTXastcDecodeModeWithsRGB,
                fmt_args![to_string_transfer(tf)],
            )?;
        }
        Ok(())
    }

    fn validate_ktx_anim_data(&mut self, data: &[u8]) -> VResult {
        self.found_ktx_anim_data = true;
        if data.len() != 12 {
            self.error(&Metadata::KTXanimDataInvalidSize, fmt_args![data.len()])?;
        }
        if self.header.layer_count == 0 {
            self.error(
                &Metadata::KTXanimDataNotArray,
                fmt_args![self.header.layer_count],
            )?;
        }
        Ok(())
    }

    fn level_index(&self, level: u32) -> LevelIndexEntry {
        self.level_indices
            .get(level as usize)
            .copied()
            .unwrap_or_default()
    }

    fn validate_sgd(&mut self) -> VResult {
        let sgd = self.header.supercompression_global_data;
        if sgd.byte_offset == 0 || sgd.byte_length == 0 {
            return Ok(()); // There is no SGD block
        }
        let buffer = self.read(sgd.byte_offset, sgd.byte_length, "the SGD")?;
        let h = self.header;
        let scheme = h.supercompression_scheme;
        let sgd_len = sgd.byte_length;

        match scheme {
            SupercmpScheme::UASTC_HDR_6X6_INTERMEDIATE => {
                if sgd_len < 12 {
                    return self.error(&SGD::UH6X6IESizeTooSmall, fmt_args![sgd_len]);
                }
                if !sgd_len.is_multiple_of(12) {
                    return self.error(&SGD::UH6X6IEByteLengthInvalidSize, fmt_args![sgd_len]);
                }
                let mut i = 0usize;
                for level in 0..self.num_levels {
                    let level_entry = self.level_index(level);
                    for layer in 0..self.num_layers {
                        for face in 0..h.face_count {
                            for z_slice in 0..h.pixel_depth.wrapping_shr(level).max(1) {
                                let image = buffer.get(i * 12..).unwrap_or_default();
                                i += 1;
                                let (rgb_ofs, rgb_len, slice_type) =
                                    (word(image, 0), word(image, 1), word(image, 2));
                                let loc = fmt_args![level, layer, face, z_slice];
                                let with = |extra: Vec<Arg>| {
                                    let mut a = loc.clone();
                                    a.extend(extra);
                                    a
                                };
                                if rgb_len == 0 {
                                    self.error(
                                        &SGD::UH6X6IEZeroRGBLength,
                                        with(fmt_args![rgb_len]),
                                    )?;
                                }
                                if u64::from(rgb_ofs.wrapping_add(rgb_len))
                                    > level_entry.byte_length
                                {
                                    self.error(
                                        &SGD::UH6X6IEInvalidRGBSlice,
                                        with(fmt_args![rgb_ofs, rgb_len, level_entry.byte_length]),
                                    )?;
                                }
                                let sgd_profile = (slice_type >> 8) as u8;
                                let sgd_version = slice_type as u8;
                                if sgd_profile != 0xAB {
                                    self.error(
                                        &SGD::UH6X6IEInvalidRGBSliceTypeProfile,
                                        with(fmt_args![sgd_profile]),
                                    )?;
                                }
                                if sgd_version != 0xCD && sgd_version != 0xCE {
                                    self.error(
                                        &SGD::UH6X6IEInvalidRGBSliceTypeVersion,
                                        with(fmt_args![sgd_version]),
                                    )?;
                                }
                                let hb = self.read(
                                    level_entry.byte_offset.wrapping_add(u64::from(rgb_ofs)),
                                    6,
                                    "Headers",
                                )?;
                                let hw = |k: usize| u16::from_le_bytes([hb[k * 2], hb[k * 2 + 1]]);
                                let profile = (hw(0) >> 8) as u8;
                                let version = hw(0) as u8;
                                if profile != sgd_profile {
                                    self.error(
                                        &SGD::UH6X6IERGBSliceTypeProfileMismatch,
                                        with(fmt_args![sgd_profile, profile]),
                                    )?;
                                }
                                if version != sgd_version {
                                    self.error(
                                        &SGD::UH6X6IERGBSliceTypeVersionMismatch,
                                        with(fmt_args![sgd_version, version]),
                                    )?;
                                }
                                let expected_width =
                                    h.pixel_width.wrapping_shr(level).max(1) as u16;
                                let expected_height =
                                    h.pixel_height.wrapping_shr(level).max(1) as u16;
                                if hw(1) != expected_width || hw(2) != expected_height {
                                    self.error(
                                        &SGD::UH6X6IEInvalidRGBSliceDimensionsInData,
                                        with(fmt_args![
                                            hw(1),
                                            hw(2),
                                            expected_width,
                                            expected_height
                                        ]),
                                    )?;
                                }
                            }
                        }
                    }
                }
            }
            SupercmpScheme::BASIS_LZ => {
                let mut image_count = 0u32;
                for level in 0..self.num_levels {
                    // numFaces * depth is only reasonable because they can't both be > 1. There are no 3D cubemaps
                    image_count = image_count.wrapping_add(
                        self.num_layers
                            .wrapping_mul(h.face_count)
                            .wrapping_mul(h.pixel_depth.wrapping_shr(level).max(1)),
                    );
                }
                // Validate GlobalHeader
                if sgd_len < 20 {
                    return self.error(&SGD::BLZESizeTooSmallHeader, fmt_args![sgd_len]);
                }
                let endpoints_len = word(&buffer, 1);
                let selectors_len = word(&buffer, 2);
                let tables_len = word(&buffer, 3);
                let extended_len = word(&buffer, 4);
                let expected_len = 20u64
                    + 20 * u64::from(image_count)
                    + u64::from(endpoints_len)
                    + u64::from(selectors_len)
                    + u64::from(tables_len)
                    + u64::from(extended_len);
                if sgd_len != expected_len {
                    self.error(
                        &SGD::BLZESizeIncorrect,
                        fmt_args![sgd_len, image_count, expected_len],
                    )?;
                }
                if self.parsed_color_model == Some(KHR_DF_MODEL_ETC1S) && extended_len != 0 {
                    self.error(&SGD::BLZEExtendedByteLengthNotZero, fmt_args![extended_len])?;
                }
                // Validate ImageDesc
                if sgd_len < 20 + 20 * u64::from(image_count) {
                    return Ok(());
                }
                let mut found_p_frame = false;
                let mut i = 0usize;
                for level in 0..self.num_levels {
                    let level_len = self.level_index(level).byte_length;
                    for layer in 0..self.num_layers {
                        for face in 0..h.face_count {
                            for z_slice in 0..h.pixel_depth.wrapping_shr(level).max(1) {
                                let image = buffer.get(20 + i * 20..).unwrap_or_default();
                                i += 1;
                                let flags = word(image, 0);
                                let (rgb_ofs, rgb_len) = (word(image, 1), word(image, 2));
                                let (alpha_ofs, alpha_len) = (word(image, 3), word(image, 4));
                                let loc = fmt_args![level, layer, face, z_slice];
                                let with = |extra: Vec<Arg>| {
                                    let mut a = loc.clone();
                                    a.extend(extra);
                                    a
                                };
                                if flags & ETC1S_P_FRAME != 0 {
                                    found_p_frame = true;
                                }
                                if flags & !ETC1S_P_FRAME != 0 {
                                    self.error(
                                        &SGD::BLZEInvalidImageFlagBit,
                                        with(fmt_args![flags]),
                                    )?;
                                }
                                if rgb_len == 0 {
                                    self.error(&SGD::BLZEZeroRGBLength, with(fmt_args![rgb_len]))?;
                                }
                                if u64::from(rgb_ofs.wrapping_add(rgb_len)) > level_len {
                                    self.error(
                                        &SGD::BLZEInvalidRGBSlice,
                                        with(fmt_args![rgb_ofs, rgb_len, level_len]),
                                    )?;
                                }
                                if u64::from(alpha_ofs.wrapping_add(alpha_len)) > level_len {
                                    self.error(
                                        &SGD::BLZEInvalidAlphaSlice,
                                        with(fmt_args![alpha_ofs, alpha_len, level_len]),
                                    )?;
                                }
                                // Crosscheck with the DFD numSamples
                                if alpha_len == 0 && self.num_samples == 2 {
                                    self.error(&SGD::BLZEDFDMismatchAlpha, with(vec![]))?;
                                }
                                if alpha_len != 0 && self.num_samples == 1 {
                                    self.error(
                                        &SGD::BLZEDFDMismatchNoAlpha,
                                        with(fmt_args![alpha_len]),
                                    )?;
                                }
                            }
                        }
                    }
                }
                if found_p_frame && !self.found_ktx_anim_data {
                    self.error(&SGD::BLZENoAnimationSequencesPFrame, vec![])?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_paddings(&mut self) -> VResult {
        let level_index_offset = KTX2_HEADER_SIZE as u64;
        let level_index_size = LEVEL_INDEX_ENTRY_SIZE as u64 * u64::from(self.num_levels);
        let h = self.header;
        let mut position = level_index_offset + level_index_size;

        let mut blocks: Vec<(u64, u64, String)> = vec![
            (
                u64::from(h.data_format_descriptor.byte_offset),
                u64::from(h.data_format_descriptor.byte_length),
                "DFD".into(),
            ),
            (
                u64::from(h.key_value_data.byte_offset),
                u64::from(h.key_value_data.byte_length),
                "KVD".into(),
            ),
            (
                h.supercompression_global_data.byte_offset,
                h.supercompression_global_data.byte_length,
                "SGD".into(),
            ),
        ];
        for (i, l) in self.level_indices.iter().enumerate().rev() {
            blocks.push((l.byte_offset, l.byte_length, format!("image level {i}")));
        }

        for (offset, size, name) in blocks {
            if offset == 0 || size == 0 {
                continue; // Block is missing, skip
            }
            if offset < position {
                position = position.max(offset.wrapping_add(size));
                continue; // Just ignore invalid block placements regarding padding checks
            }
            let padding_size = offset - position;
            let buffer = self.read(
                position,
                padding_size,
                &format!("the padding before {name}"),
            )?;
            if let Some(i) = buffer.iter().position(|&b| b != 0) {
                let at = format!("before {name} at offset {}", position + i as u64);
                // Only report the first non-zero byte per padding, no need to spam
                self.error(&Metadata::PaddingNotZero, fmt_args![buffer[i], at])?;
            }
            position = offset.wrapping_add(size);
        }
        Ok(())
    }

    fn validate_create_and_transcode(&mut self) -> VResult {
        let mut flags = KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT;
        if self.check_gltf_basisu {
            flags |= KTX_TEXTURE_CREATE_CHECK_GLTF_BASISU_BIT;
        }
        let created = Texture2::create_from_memory(self.data, flags);
        let scheme_str = to_string_scheme(self.scheme());

        if self.num_error == 0 && self.num_warning == 0 {
            match created {
                Err(ErrorCode::UnsupportedFeature) => {
                    self.warning(&Validator::UnsupportedFeature, vec![])?;
                }
                Err(ErrorCode::DecompressLengthError) => {
                    self.error(
                        &LevelIndex::UncompressedByteLengthMismatch,
                        fmt_args![scheme_str],
                    )?;
                }
                Err(ErrorCode::DecompressChecksumError) => {
                    self.error(&Validator::DecompressChecksumError, fmt_args![scheme_str])?;
                }
                Ok(mut texture) => {
                    let model = self.parsed_color_model;
                    let result = if model == Some(KHR_DF_MODEL_ETC1S) {
                        texture.transcode_basis(TranscodeFmt::ETC2_RGBA, 0)
                    } else if model == Some(KHR_DF_MODEL_UASTC) {
                        texture.transcode_basis(TranscodeFmt::ASTC_4x4_RGBA, 0)
                    } else {
                        Ok(())
                    };
                    if let Err(e) = result {
                        self.error(
                            &Validator::TranscodeFailure,
                            fmt_args![to_string_model(model.unwrap_or(0)), ktx_error_string(e)],
                        )?;
                    }
                }
                Err(e) => {
                    return self.fatal(&Validator::CreateFailure, fmt_args![ktx_error_string(e)]);
                }
            }
        } else if created.is_ok() && self.num_error != 0 {
            self.warning(&Validator::SupportedNonConformantFile, vec![])?;
        }
        Ok(())
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// `std::lcm` (0 if either is 0).
fn lcm(a: u64, b: u64) -> u64 {
    if a == 0 || b == 0 {
        0
    } else {
        a / gcd(a, b) * b
    }
}

/// Port of `validateMemory`. Returns the exit code (0 or 3), or the message of
/// a runtime error that escaped the C++ validator.
pub fn validate_memory(
    data: &[u8],
    warnings_as_errors: bool,
    gltf_basisu: bool,
    callback: &mut dyn FnMut(&ValidationReport),
) -> Result<i32, String> {
    let mut ctx = ValidationContext::new(warnings_as_errors, gltf_basisu, callback, data);
    match ctx.validate(true) {
        Ok(rc) => Ok(rc),
        Err(Stop::Fatal) => Ok(RC_INVALID_FILE),
        Err(Stop::Runtime(what)) => Err(what),
    }
}

/// Port of `validateNamedFile`.
pub fn validate_named_file(
    filepath: &str,
    warnings_as_errors: bool,
    gltf_basisu: bool,
    callback: &mut dyn FnMut(&ValidationReport),
) -> Result<i32, String> {
    match std::fs::read(filepath) {
        Ok(data) => validate_memory(&data, warnings_as_errors, gltf_basisu, callback),
        Err(e) => {
            let args = fmt_args![filepath, super::utility::io_error_message(&e)];
            let details = super::fmt::format(IOError::FileOpen.details_fmt, &args);
            callback(&ValidationReport {
                type_: IssueType::Fatal,
                id: IOError::FileOpen.id,
                message: IOError::FileOpen.message.to_string(),
                details,
            });
            Ok(RC_INVALID_FILE)
        }
    }
}
