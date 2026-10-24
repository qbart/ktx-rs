//! `ktx.h` encoder parameter structs and enums: `ktxBasisParams`,
//! `ktxAstcParams`, `ktx_basis_codec_e`, `ktx_pack_uastc_flag_bits_e`,
//! `ktx_pack_astc_*`. Used by `ktxTexture2_CompressBasisEx` /
//! `ktxTexture2_CompressAstcEx` and the `ktx create` / `ktx encode` options.

pub type BasisCodecE = u32;
pub const KTX_BASIS_CODEC_NONE: BasisCodecE = 0;
pub const KTX_BASIS_CODEC_ETC1S: BasisCodecE = 1;
pub const KTX_BASIS_CODEC_UASTC_LDR_4X4: BasisCodecE = 2;
pub const KTX_BASIS_CODEC_UASTC_HDR_4X4: BasisCodecE = 3;
pub const KTX_BASIS_CODEC_UASTC_HDR_6X6_INTERMEDIATE: BasisCodecE = 4;

pub const KTX_PACK_UASTC_LEVEL_FASTEST: u32 = 0;
pub const KTX_PACK_UASTC_LEVEL_FASTER: u32 = 1;
pub const KTX_PACK_UASTC_LEVEL_DEFAULT: u32 = 2;
pub const KTX_PACK_UASTC_LEVEL_SLOWER: u32 = 3;
pub const KTX_PACK_UASTC_LEVEL_VERYSLOW: u32 = 4;
pub const KTX_PACK_UASTC_MAX_LEVEL: u32 = KTX_PACK_UASTC_LEVEL_VERYSLOW;
pub const KTX_PACK_UASTC_LEVEL_MASK: u32 = 0x7;
pub const KTX_PACK_UASTC_FAVOR_UASTC_ERROR: u32 = 8;
pub const KTX_PACK_UASTC_FAVOR_BC7_ERROR: u32 = 16;
pub const KTX_PACK_UASTC_ETC1_FASTER_HINTS: u32 = 64;
pub const KTX_PACK_UASTC_ETC1_FASTEST_HINTS: u32 = 128;
pub const KTX_PACK_UASTC_ETC1_DISABLE_FLIP_AND_INDIVIDUAL: u32 = 256;

pub const KTX_PACK_ASTC_QUALITY_LEVEL_FASTEST: u32 = 0;
pub const KTX_PACK_ASTC_QUALITY_LEVEL_FAST: u32 = 10;
pub const KTX_PACK_ASTC_QUALITY_LEVEL_MEDIUM: u32 = 60;
pub const KTX_PACK_ASTC_QUALITY_LEVEL_THOROUGH: u32 = 98;
pub const KTX_PACK_ASTC_QUALITY_LEVEL_EXHAUSTIVE: u32 = 100;
pub const KTX_PACK_ASTC_QUALITY_LEVEL_MAX: u32 = KTX_PACK_ASTC_QUALITY_LEVEL_EXHAUSTIVE;

pub const KTX_PACK_ASTC_BLOCK_DIMENSION_4X4: u32 = 0;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_5X4: u32 = 1;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_5X5: u32 = 2;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_6X5: u32 = 3;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_6X6: u32 = 4;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_8X5: u32 = 5;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_8X6: u32 = 6;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_10X5: u32 = 7;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_10X6: u32 = 8;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_8X8: u32 = 9;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_10X8: u32 = 10;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_10X10: u32 = 11;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_12X10: u32 = 12;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_12X12: u32 = 13;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_3X3X3: u32 = 14;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_4X3X3: u32 = 15;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_4X4X3: u32 = 16;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_4X4X4: u32 = 17;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_5X4X4: u32 = 18;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_5X5X4: u32 = 19;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_5X5X5: u32 = 20;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_6X5X5: u32 = 21;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_6X6X5: u32 = 22;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_6X6X6: u32 = 23;
pub const KTX_PACK_ASTC_BLOCK_DIMENSION_MAX: u32 = KTX_PACK_ASTC_BLOCK_DIMENSION_6X6X6;

pub const KTX_PACK_ASTC_ENCODER_MODE_DEFAULT: u32 = 0;
pub const KTX_PACK_ASTC_ENCODER_MODE_LDR: u32 = 1;
pub const KTX_PACK_ASTC_ENCODER_MODE_HDR: u32 = 2;
pub const KTX_PACK_ASTC_ENCODER_MODE_MAX: u32 = KTX_PACK_ASTC_ENCODER_MODE_HDR;

/// `KTX_ETC1S_DEFAULT_COMPRESSION_LEVEL`.
pub const KTX_ETC1S_DEFAULT_COMPRESSION_LEVEL: u32 = 2;

/// Port of `ktxBasisParams`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BasisParams {
    pub struct_size: u32,
    pub codec: BasisCodecE,
    pub verbose: bool,
    pub no_sse: bool,
    pub thread_count: u32,
    pub etc1s_compression_level: u32,
    pub quality_level: u32,
    pub max_endpoints: u32,
    pub endpoint_rdo_threshold: f32,
    pub max_selectors: u32,
    pub selector_rdo_threshold: f32,
    pub input_swizzle: [u8; 4],
    pub normal_map: bool,
    pub separate_rg_to_rgb_a: bool,
    pub pre_swizzle: bool,
    pub no_endpoint_rdo: bool,
    pub no_selector_rdo: bool,
    pub uastc_flags: u32,
    pub uastc_rdo: bool,
    pub uastc_rdo_quality_scalar: f32,
    pub uastc_rdo_dict_size: u32,
    pub uastc_rdo_max_smooth_block_error_scale: f32,
    pub uastc_rdo_max_smooth_block_std_dev: f32,
    pub uastc_rdo_dont_favor_simpler_modes: bool,
    pub uastc_rdo_no_multithreading: bool,
    pub uastc_hdr_quality: u32,
    pub uastc_hdr_uber_mode: bool,
    pub uastc_hdr_ultra_quant: bool,
    pub uastc_hdr_favor_astc: bool,
    pub uastc_hdr_lambda: f32,
    pub uastc_hdr_level: u32,
}

/// `sizeof(ktxBasisParams)`.
pub const KTX_BASIS_PARAMS_SIZE: u32 = 96;

/// Port of `ktxAstcParams`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AstcParams {
    pub struct_size: u32,
    pub verbose: bool,
    pub thread_count: u32,
    pub block_dimension: u32,
    pub mode: u32,
    pub quality_level: u32,
    pub normal_map: bool,
    pub perceptual: bool,
    pub input_swizzle: [u8; 4],
}

/// `sizeof(ktxAstcParams)`.
pub const KTX_ASTC_PARAMS_SIZE: u32 = 32;
