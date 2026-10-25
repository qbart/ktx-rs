//! Pure Rust port of the Basis Universal transcoder (`external/basis_universal/transcoder`).
//!
//! Only the parts reachable from libktx are ported. Functions keep basisu's
//! `bool` success convention internally; libktx maps failures to `KtxResult`.

pub mod astc;
pub mod astc_6x6_hdr;
pub mod astc_hdr_enc;
pub mod astc_helpers;
pub mod atc;
pub mod backend;
pub mod basis_file;
pub mod bc6h;
pub mod bc7enc;
pub mod bc7m5;
pub mod bitreader;
pub mod comp;
pub mod dxt;
pub mod eac;
pub mod enc;
pub mod etc;
pub mod etc1s;
pub mod etc_enc;
pub mod etc_enc_tables;
pub mod frontend;
pub mod gpu_texture;
pub mod hdr_tables;
pub mod huffman_enc;
pub mod miniz;
pub mod pvrtc;
pub mod pvrtc2;
pub mod resample_filters;
pub mod resampler;
pub mod ssim;
pub mod tables;
pub mod uastc;
pub mod uastc_bc1;
pub mod uastc_bc7;
pub mod uastc_enc;
pub mod uastc_etc;
pub mod uastc_hdr;
pub mod uastc_hdr_4x4_enc;
pub mod uastc_hdr_6x6i;
pub mod uastc_ldr;
pub mod uastc_pvrtc;
pub mod uastc_tables;
pub mod vq;

/// Port of `transcoder_texture_format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct TranscoderTextureFormat(pub u32);

#[allow(non_upper_case_globals)]
impl TranscoderTextureFormat {
    pub const ETC1_RGB: Self = Self(0);
    pub const ETC2_RGBA: Self = Self(1);
    pub const BC1_RGB: Self = Self(2);
    pub const BC3_RGBA: Self = Self(3);
    pub const BC4_R: Self = Self(4);
    pub const BC5_RG: Self = Self(5);
    pub const BC7_RGBA: Self = Self(6);
    pub const BC7_ALT: Self = Self(7);
    pub const PVRTC1_4_RGB: Self = Self(8);
    pub const PVRTC1_4_RGBA: Self = Self(9);
    pub const ASTC_LDR_4x4_RGBA: Self = Self(10);
    pub const ATC_RGB: Self = Self(11);
    pub const ATC_RGBA: Self = Self(12);
    pub const RGBA32: Self = Self(13);
    pub const RGB565: Self = Self(14);
    pub const BGR565: Self = Self(15);
    pub const RGBA4444: Self = Self(16);
    pub const FXT1_RGB: Self = Self(17);
    pub const PVRTC2_4_RGB: Self = Self(18);
    pub const PVRTC2_4_RGBA: Self = Self(19);
    pub const ETC2_EAC_R11: Self = Self(20);
    pub const ETC2_EAC_RG11: Self = Self(21);
    pub const BC6H: Self = Self(22);
    pub const ASTC_HDR_4x4_RGBA: Self = Self(23);
    pub const RGB_HALF: Self = Self(24);
    pub const RGBA_HALF: Self = Self(25);
    pub const RGB_9E5: Self = Self(26);
    pub const ASTC_HDR_6x6_RGBA: Self = Self(27);
    pub const TOTAL_TEXTURE_FORMATS: Self = Self(41);
}

/// Port of `basis_tex_format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasisTexFormat {
    Etc1s,
    UastcLdr4x4,
    UastcHdr4x4,
    AstcHdr6x6,
    UastcHdr6x6Intermediate,
}

/// Port of `block_format` (subset used by the ported transcoders).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockFormat {
    Etc1,
    Etc2Rgba,
    Bc1,
    Bc3,
    Bc4,
    Bc5,
    Pvrtc1_4Rgb,
    Pvrtc1_4Rgba,
    Bc7,
    Bc7M5Color,
    Bc7M5Alpha,
    Etc2EacA8,
    AstcLdr4x4,
    AtcRgb,
    AtcRgbaInterpolatedAlpha,
    Fxt1Rgb,
    Pvrtc2_4Rgb,
    Pvrtc2_4Rgba,
    Etc2EacR11,
    Etc2EacRg11,
    Indices,
    Rgb32,
    Rgba32,
    A32,
    Rgb565,
    Bgr565,
    Rgba4444Color,
    Rgba4444Alpha,
    Rgba4444ColorOpaque,
    Rgba4444,
    RgbaHalf,
    RgbHalf,
    Rgb9e5,
    Uastc4x4,
    UastcHdr4x4,
    Bc6h,
    AstcHdr4x4,
    AstcHdr6x6,
}

/// Port of `basisu_decode_flags`.
pub mod decode_flags {
    pub const PVRTC_DECODE_TO_NEXT_POW2: u32 = 2;
    pub const TRANSCODE_ALPHA_DATA_TO_OPAQUE_FORMATS: u32 = 4;
    pub const BC1_FORBID_THREE_COLOR_BLOCKS: u32 = 8;
    pub const OUTPUT_HAS_ALPHA_INDICES: u32 = 16;
    pub const HIGH_QUALITY: u32 = 32;
    pub const NO_ETC1S_CHROMA_FILTERING: u32 = 64;
}

use TranscoderTextureFormat as F;

/// Port of `basis_get_bytes_per_block_or_pixel`.
pub fn basis_get_bytes_per_block_or_pixel(fmt: TranscoderTextureFormat) -> u32 {
    match fmt {
        F::ETC1_RGB
        | F::BC1_RGB
        | F::BC4_R
        | F::PVRTC1_4_RGB
        | F::PVRTC1_4_RGBA
        | F::ATC_RGB
        | F::PVRTC2_4_RGB
        | F::PVRTC2_4_RGBA
        | F::ETC2_EAC_R11 => 8,
        F::RGBA32 | F::RGB_9E5 => 4,
        F::RGB565 | F::BGR565 | F::RGBA4444 => 2,
        F::RGB_HALF => 6,
        F::RGBA_HALF => 8,
        f if f.0 <= 40 => 16,
        _ => 0,
    }
}

/// Port of `basis_transcoder_format_is_uncompressed`.
pub fn basis_transcoder_format_is_uncompressed(fmt: TranscoderTextureFormat) -> bool {
    matches!(
        fmt,
        F::RGBA32 | F::RGB565 | F::BGR565 | F::RGBA4444 | F::RGB_HALF | F::RGBA_HALF | F::RGB_9E5
    )
}

/// Port of `basis_block_format_is_uncompressed`.
pub fn basis_block_format_is_uncompressed(fmt: BlockFormat) -> bool {
    use BlockFormat::*;
    matches!(
        fmt,
        Rgb32
            | Rgba32
            | A32
            | Rgb565
            | Bgr565
            | Rgba4444
            | Rgba4444Color
            | Rgba4444Alpha
            | Rgba4444ColorOpaque
            | RgbaHalf
            | RgbHalf
            | Rgb9e5
    )
}

/// Port of `basis_get_uncompressed_bytes_per_pixel`.
pub fn basis_get_uncompressed_bytes_per_pixel(fmt: TranscoderTextureFormat) -> u32 {
    match fmt {
        F::RGBA32 | F::RGB_9E5 => 4,
        F::RGB565 | F::BGR565 | F::RGBA4444 => 2,
        F::RGB_HALF => 6,
        F::RGBA_HALF => 8,
        _ => 0,
    }
}

/// Port of `basis_get_block_width`.
pub fn basis_get_block_width(fmt: TranscoderTextureFormat) -> u32 {
    match fmt.0 {
        17 => 8,
        27 => 6,
        28 | 29 => 5,
        30 | 31 => 6,
        32 | 33 | 36 => 8,
        34 | 35 | 37 | 38 => 10,
        39 | 40 => 12,
        _ => 4,
    }
}

/// Port of `basis_get_block_height`.
pub fn basis_get_block_height(fmt: TranscoderTextureFormat) -> u32 {
    match fmt.0 {
        27 => 6,
        29 | 30 | 32 | 34 => 5,
        31 | 33 | 35 => 6,
        36 | 37 => 8,
        38 | 39 => 10,
        40 => 12,
        _ => 4,
    }
}

/// Port of `basis_is_format_supported` for the source formats libktx uses.
pub fn basis_is_format_supported(tex_type: TranscoderTextureFormat, fmt: BasisTexFormat) -> bool {
    match fmt {
        BasisTexFormat::AstcHdr6x6 | BasisTexFormat::UastcHdr6x6Intermediate => {
            matches!(
                tex_type,
                F::ASTC_HDR_6x6_RGBA | F::BC6H | F::RGBA_HALF | F::RGB_HALF | F::RGB_9E5
            )
        }
        BasisTexFormat::UastcHdr4x4 => {
            matches!(
                tex_type,
                F::ASTC_HDR_4x4_RGBA | F::BC6H | F::RGBA_HALF | F::RGB_HALF | F::RGB_9E5
            )
        }
        BasisTexFormat::UastcLdr4x4 => {
            !(matches!(
                tex_type,
                F::PVRTC2_4_RGB
                    | F::PVRTC2_4_RGBA
                    | F::ATC_RGB
                    | F::ATC_RGBA
                    | F::FXT1_RGB
                    | F::ASTC_HDR_4x4_RGBA
                    | F::ASTC_HDR_6x6_RGBA
                    | F::BC6H
                    | F::RGBA_HALF
                    | F::RGB_HALF
                    | F::RGB_9E5
            ) || (28..=40).contains(&tex_type.0))
                && tex_type.0 < F::TOTAL_TEXTURE_FORMATS.0
        }
        BasisTexFormat::Etc1s => matches!(
            tex_type,
            F::ETC1_RGB
                | F::RGBA32
                | F::RGB565
                | F::BGR565
                | F::RGBA4444
                | F::BC1_RGB
                | F::BC4_R
                | F::BC5_RG
                | F::BC3_RGBA
                | F::PVRTC1_4_RGB
                | F::PVRTC1_4_RGBA
                | F::BC7_RGBA
                | F::BC7_ALT
                | F::ETC2_RGBA
                | F::ASTC_LDR_4x4_RGBA
                | F::ATC_RGB
                | F::ATC_RGBA
                | F::FXT1_RGB
                | F::PVRTC2_4_RGB
                | F::PVRTC2_4_RGBA
                | F::ETC2_EAC_R11
                | F::ETC2_EAC_RG11
        ),
    }
}

/// Port of `basis_validate_output_buffer_size`.
pub fn basis_validate_output_buffer_size(
    target_format: TranscoderTextureFormat,
    output_blocks_buf_size_in_blocks_or_pixels: u32,
    orig_width: u32,
    orig_height: u32,
    mut output_row_pitch_in_blocks_or_pixels: u32,
    mut output_rows_in_pixels: u32,
) -> bool {
    if basis_transcoder_format_is_uncompressed(target_format) {
        if output_row_pitch_in_blocks_or_pixels == 0 {
            output_row_pitch_in_blocks_or_pixels = orig_width;
        }
        if output_rows_in_pixels == 0 {
            output_rows_in_pixels = orig_height;
        }
        (output_blocks_buf_size_in_blocks_or_pixels as u64)
            >= output_rows_in_pixels as u64 * output_row_pitch_in_blocks_or_pixels as u64
    } else {
        let bw = basis_get_block_width(target_format);
        let bh = basis_get_block_height(target_format);
        let total = orig_width.div_ceil(bw) as u64 * orig_height.div_ceil(bh) as u64;
        output_blocks_buf_size_in_blocks_or_pixels as u64 >= total
    }
}

/// Port of `basis_compute_transcoded_image_size_in_bytes`.
pub fn basis_compute_transcoded_image_size_in_bytes(
    target_format: TranscoderTextureFormat,
    orig_width: u32,
    orig_height: u32,
) -> u32 {
    if basis_transcoder_format_is_uncompressed(target_format) {
        return orig_width * basis_get_uncompressed_bytes_per_pixel(target_format) * orig_height;
    }
    let bytes_per_block = basis_get_bytes_per_block_or_pixel(target_format);
    if target_format == F::PVRTC1_4_RGB || target_format == F::PVRTC1_4_RGBA {
        let width = (orig_width + 3) & !3;
        let height = (orig_height + 3) & !3;
        return (width.max(8) * height.max(8) * 4).div_ceil(8);
    }
    let bw = basis_get_block_width(target_format);
    let bh = basis_get_block_height(target_format);
    orig_width.div_ceil(bw) * orig_height.div_ceil(bh) * bytes_per_block
}

/// Port of `basisu_transcoder_state`.
#[derive(Debug, Clone)]
pub struct TranscoderState {
    pub block_endpoint_preds: [Vec<BlockPreds>; 2],
    /// `[alpha_flag][level_index]`
    pub prev_frame_indices: [[Vec<u32>; TranscoderState::MAX_PREV_FRAME_LEVELS]; 2],
}

/// Port of `basisu_transcoder_state::block_preds`.
#[derive(Debug, Clone, Copy, Default)]
pub struct BlockPreds {
    pub endpoint_index: u16,
    pub pred_bits: u8,
}

impl TranscoderState {
    pub const MAX_PREV_FRAME_LEVELS: usize = 16;

    pub fn new() -> TranscoderState {
        TranscoderState {
            block_endpoint_preds: [Vec::new(), Vec::new()],
            prev_frame_indices: [Default::default(), Default::default()],
        }
    }

    /// Port of `clear`.
    pub fn clear(&mut self) {
        *self = TranscoderState::new();
    }
}

impl Default for TranscoderState {
    fn default() -> Self {
        Self::new()
    }
}
