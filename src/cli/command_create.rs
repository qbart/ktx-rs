//! Port of tools/ktx/command_create.cpp (`ktx create`).
//!
//! The UASTC HDR 6x6i encoder is not ported yet: `--encode uastc-hdr-6x6i` is parsed and
//! validated like the C++ but fails with `NOT_IMPLEMENTED` when the encoding step is reached.

use std::rc::Rc;

use super::command::*;
use super::deflate_utils::OptionsDeflate;
use super::encode_utils::*;
use super::format_descriptor::FormatDescriptor;
use super::formats::*;
use super::imageio::image::*;
use super::imageio::{
    self, ImageError, ImageInput, ImageInputFormatType, ImageSpec, Origin, bit_ceil,
    format_type_to_string, origin_to_string,
};
use super::metrics_utils::MetricsCalculator;
use super::options::{OptionSpec, Options, ParseResult};
use super::utility::{ceil_div, fmt_in_file, log2};
use crate::basisu::resampler::BoundaryOp;
use crate::defs::{
    KTX_MAP_RANGE_KEY, KTX_ORIENTATION_KEY, KTX_SWIZZLE_KEY, KTX_WRITER_KEY,
    KTX_WRITER_SCPARAMS_KEY,
};
use crate::dfd::khr_df::*;
use crate::dfd::{
    BITLENGTH, CHANNELID, FLAGS, PRIMARIES, QUALIFIERS, TRANSFER, dfdsamplecount, dfdsetsval,
    dfdsetval, dfdsval,
};
use crate::encode_params::*;
use crate::texture::{KtxTexture, TextureCreateInfo, TextureCreateStorage};
use crate::vkformat::is_prohibited_format;
use crate::{Texture2, VkFormat, ktx_error_string};

/// Port of `OptionsCreate`.
#[derive(Debug, Clone)]
pub struct OptionsCreate {
    pub _1d: bool,
    pub cubemap: bool,
    pub vk_format: VkFormat,
    pub format_desc: FormatDescriptor,
    pub raw: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub depth: Option<u32>,
    pub layers: Option<u32>,
    pub levels: Option<u32>,
    pub offset: Option<f32>,
    pub scale: Option<f32>,
    pub map_range_runtime: bool,
    pub mipmap_runtime: bool,
    pub mipmap_generate: bool,
    pub mipmap_filter: Option<String>,
    pub default_mipmap_filter: String,
    pub mipmap_filter_scale: Option<f32>,
    pub default_mipmap_filter_scale: f32,
    pub mipmap_wrap: Option<BoundaryOp>,
    pub default_mipmap_wrap: BoundaryOp,
    pub swizzle: Option<String>,
    pub swizzle_input: Option<String>,
    pub image_scale: Option<f32>,
    pub convert_tf: Option<u32>,
    pub assign_tf: Option<u32>,
    pub assign_primaries: Option<u32>,
    pub convert_primaries: Option<u32>,
    pub assign_texcoord_origin: Option<Origin>,
    pub convert_texcoord_origin: Option<Origin>,
    pub fail_on_color_conversions: bool,
    pub warn_on_color_conversions: bool,
    pub no_warn_on_color_conversions: bool,
    pub fail_on_origin_changes: bool,
    pub warn_on_origin_changes: bool,
    pub normalize: bool,
    pub premultiply_alpha: bool,
    /// Raw `--format` / `--assign-tf` / `--convert-tf` arguments for error messages.
    format_arg: String,
    assign_tf_arg: String,
    convert_tf_arg: String,
}

impl Default for OptionsCreate {
    fn default() -> Self {
        OptionsCreate {
            _1d: false,
            cubemap: false,
            vk_format: VkFormat::UNDEFINED,
            format_desc: FormatDescriptor::default(),
            raw: false,
            width: None,
            height: None,
            depth: None,
            layers: None,
            levels: None,
            offset: None,
            scale: None,
            map_range_runtime: false,
            mipmap_runtime: false,
            mipmap_generate: false,
            mipmap_filter: None,
            default_mipmap_filter: "lanczos4".into(),
            mipmap_filter_scale: None,
            default_mipmap_filter_scale: 1.0,
            mipmap_wrap: None,
            default_mipmap_wrap: BoundaryOp::Clamp,
            swizzle: None,
            swizzle_input: None,
            image_scale: None,
            convert_tf: None,
            assign_tf: None,
            assign_primaries: None,
            convert_primaries: None,
            assign_texcoord_origin: None,
            convert_texcoord_origin: None,
            fail_on_color_conversions: false,
            warn_on_color_conversions: false,
            no_warn_on_color_conversions: false,
            fail_on_origin_changes: false,
            warn_on_origin_changes: false,
            normalize: false,
            premultiply_alpha: false,
            format_arg: String::new(),
            assign_tf_arg: String::new(),
            convert_tf_arg: String::new(),
        }
    }
}

const K_FORMAT: &str = "format";
const K_1D: &str = "1d";
const K_CUBEMAP: &str = "cubemap";
const K_RAW: &str = "raw";
const K_WIDTH: &str = "width";
const K_HEIGHT: &str = "height";
const K_DEPTH: &str = "depth";
const K_LAYERS: &str = "layers";
const K_LEVELS: &str = "levels";
const K_RUNTIME_MIPMAP: &str = "runtime-mipmap";
const K_GENERATE_MIPMAP: &str = "generate-mipmap";
const K_ENCODE: &str = "encode";
const K_NORMALIZE: &str = "normalize";
const K_SWIZZLE: &str = "swizzle";
const K_INPUT_SWIZZLE: &str = "input-swizzle";
const K_ASSIGN_TF: &str = "assign-tf";
const K_ASSIGN_PRIMARIES: &str = "assign-primaries";
const K_ASSIGN_TEXCOORD_ORIGIN: &str = "assign-texcoord-origin";
const K_CONVERT_TF: &str = "convert-tf";
const K_CONVERT_PRIMARIES: &str = "convert-primaries";
const K_CONVERT_TEXCOORD_ORIGIN: &str = "convert-texcoord-origin";
const K_FAIL_ON_COLOR_CONVERSIONS: &str = "fail-on-color-conversions";
const K_WARN_ON_COLOR_CONVERSIONS: &str = "warn-on-color-conversions";
const K_NO_WARN_ON_COLOR_CONVERSIONS: &str = "no-warn-on-color-conversions";
const K_FAIL_ON_ORIGIN_CHANGES: &str = "fail-on-origin-changes";
const K_WARN_ON_ORIGIN_CHANGES: &str = "warn-on-origin-changes";
const K_MIPMAP_FILTER: &str = "mipmap-filter";
const K_MIPMAP_FILTER_SCALE: &str = "mipmap-filter-scale";
const K_MIPMAP_WRAP: &str = "mipmap-wrap";
const K_SCALE: &str = "scale";
const K_PREMULTIPLY_ALPHA: &str = "premultiply-alpha";
const K_MAP_RANGE_AUTO: &str = "map-range-auto";
const K_MAP_RANGE_OFFSET: &str = "map-range-offset";
const K_MAP_RANGE_SCALE: &str = "map-range-scale";

/// `parseTransferFunction` table (aliases resolve to the same enumerators).
fn transfer_from_name(s: &str) -> Option<u32> {
    Some(match s {
        "NONE" => KHR_DF_TRANSFER_UNSPECIFIED,
        "LINEAR" => KHR_DF_TRANSFER_LINEAR,
        "SRGB" | "SRGB_EOTF" | "SCRGB" | "SCRGB_EOTF" => KHR_DF_TRANSFER_SRGB,
        "ITU" | "ITU_OETF" | "BT601" | "BT601_OETF" | "BT709" | "BT709_OETF" | "BT2020"
        | "BT2020_OETF" | "SMPTE170M" | "SMPTE170M_EOTF" | "SMPTE170M_OETF" => KHR_DF_TRANSFER_ITU,
        "NTSC" | "NTSC_EOTF" => KHR_DF_TRANSFER_NTSC,
        "SLOG" | "SLOG_OETF" => KHR_DF_TRANSFER_SLOG,
        "SLOG2" | "SLOG2_OETF" => KHR_DF_TRANSFER_SLOG2,
        "BT1886" | "BT1886_EOTF" => KHR_DF_TRANSFER_BT1886,
        "HLG_OETF" => KHR_DF_TRANSFER_HLG_OETF,
        "HLG_EOTF" => KHR_DF_TRANSFER_HLG_EOTF,
        "PQ_OETF" => KHR_DF_TRANSFER_PQ_OETF,
        "PQ_EOTF" => KHR_DF_TRANSFER_PQ_EOTF,
        "DCIP3" | "DCIP3_EOTF" => KHR_DF_TRANSFER_DCIP3,
        "PAL_OETF" => KHR_DF_TRANSFER_PAL_OETF,
        "PAL625_EOTF" => KHR_DF_TRANSFER_PAL625_EOTF,
        "ST240" | "ST240_EOTF" | "ST240_OETF" => KHR_DF_TRANSFER_ST240,
        "ACESCC" | "ACESCC_OETF" => KHR_DF_TRANSFER_ACESCC,
        "ACESCCT" | "ACESCCT_OETF" => KHR_DF_TRANSFER_ACESCCT,
        "ADOBERGB" | "ADOBERGB_EOTF" => KHR_DF_TRANSFER_ADOBERGB,
        "HLG_UNNORMALIZED_OETF" => KHR_DF_TRANSFER_HLG_UNNORMALIZED_OETF,
        _ => return None,
    })
}

/// `parseColorPrimaries` table.
fn primaries_from_name(s: &str) -> Option<u32> {
    Some(match s {
        "NONE" => KHR_DF_PRIMARIES_UNSPECIFIED,
        "BT709" | "SRGB" => KHR_DF_PRIMARIES_BT709,
        "BT601_EBU" => KHR_DF_PRIMARIES_BT601_EBU,
        "BT601_SMPTE" => KHR_DF_PRIMARIES_BT601_SMPTE,
        "BT2020" => KHR_DF_PRIMARIES_BT2020,
        "CIEXYZ" => KHR_DF_PRIMARIES_CIEXYZ,
        "ACES" => KHR_DF_PRIMARIES_ACES,
        "ACESCC" => KHR_DF_PRIMARIES_ACESCC,
        "NTSC1953" => KHR_DF_PRIMARIES_NTSC1953,
        "PAL525" => KHR_DF_PRIMARIES_PAL525,
        "DISPLAYP3" => KHR_DF_PRIMARIES_DISPLAYP3,
        "ADOBERGB" => KHR_DF_PRIMARIES_ADOBERGB,
        _ => return None,
    })
}

/// List of formats that have supported format conversions.
fn is_convertable_format(f: VkFormat) -> bool {
    use VkFormat as F;
    is_format_astc_ldr_2d(f)
        || matches!(
            f,
            F::R8_UNORM
                | F::R8_SRGB
                | F::R8G8_UNORM
                | F::R8G8_SRGB
                | F::R8G8B8_UNORM
                | F::R8G8B8_SRGB
                | F::B8G8R8_UNORM
                | F::B8G8R8_SRGB
                | F::R8G8B8A8_UNORM
                | F::R8G8B8A8_SRGB
                | F::B8G8R8A8_UNORM
                | F::B8G8R8A8_SRGB
                | F::A8B8G8R8_UNORM_PACK32
                | F::A8B8G8R8_SRGB_PACK32
                | F::R4G4_UNORM_PACK8
                | F::R5G6B5_UNORM_PACK16
                | F::B5G6R5_UNORM_PACK16
                | F::R4G4B4A4_UNORM_PACK16
                | F::B4G4R4A4_UNORM_PACK16
                | F::R5G5B5A1_UNORM_PACK16
                | F::B5G5R5A1_UNORM_PACK16
                | F::A1R5G5B5_UNORM_PACK16
                | F::A4R4G4B4_UNORM_PACK16
                | F::A4B4G4R4_UNORM_PACK16
                | F::R10X6_UNORM_PACK16
                | F::R10X6G10X6_UNORM_2PACK16
                | F::R10X6G10X6B10X6A10X6_UNORM_4PACK16
                | F::R12X4_UNORM_PACK16
                | F::R12X4G12X4_UNORM_2PACK16
                | F::R12X4G12X4B12X4A12X4_UNORM_4PACK16
                | F::R16_UNORM
                | F::R16G16_UNORM
                | F::R16G16B16_UNORM
                | F::R16G16B16A16_UNORM
                | F::A2R10G10B10_UNORM_PACK32
                | F::A2B10G10R10_UNORM_PACK32
                | F::G8B8G8R8_422_UNORM
                | F::B8G8R8G8_422_UNORM
                | F::G10X6B10X6G10X6R10X6_422_UNORM_4PACK16
                | F::B10X6G10X6R10X6G10X6_422_UNORM_4PACK16
                | F::G12X4B12X4G12X4R12X4_422_UNORM_4PACK16
                | F::B12X4G12X4R12X4G12X4_422_UNORM_4PACK16
                | F::G16B16G16R16_422_UNORM
                | F::B16G16R16G16_422_UNORM
                | F::R8_UINT
                | F::R8_SINT
                | F::R16_UINT
                | F::R16_SINT
                | F::R32_UINT
                | F::R8G8_UINT
                | F::R8G8_SINT
                | F::R16G16_UINT
                | F::R16G16_SINT
                | F::R32G32_UINT
                | F::R8G8B8_UINT
                | F::R8G8B8_SINT
                | F::B8G8R8_UINT
                | F::B8G8R8_SINT
                | F::R16G16B16_UINT
                | F::R16G16B16_SINT
                | F::R32G32B32_UINT
                | F::R8G8B8A8_UINT
                | F::R8G8B8A8_SINT
                | F::B8G8R8A8_UINT
                | F::B8G8R8A8_SINT
                | F::A8B8G8R8_UINT_PACK32
                | F::A8B8G8R8_SINT_PACK32
                | F::R16G16B16A16_UINT
                | F::R16G16B16A16_SINT
                | F::R32G32B32A32_UINT
                | F::A2R10G10B10_UINT_PACK32
                | F::A2R10G10B10_SINT_PACK32
                | F::A2B10G10R10_SINT_PACK32
                | F::A2B10G10R10_UINT_PACK32
                | F::R16_SFLOAT
                | F::R16G16_SFLOAT
                | F::R16G16B16_SFLOAT
                | F::R16G16B16A16_SFLOAT
                | F::R32_SFLOAT
                | F::R32G32_SFLOAT
                | F::R32G32B32_SFLOAT
                | F::R32G32B32A32_SFLOAT
                | F::B10G11R11_UFLOAT_PACK32
                | F::E5B9G9R9_UFLOAT_PACK32
                | F::D16_UNORM
                | F::X8_D24_UNORM_PACK32
                | F::D32_SFLOAT
                | F::S8_UINT
                | F::D16_UNORM_S8_UINT
                | F::D24_UNORM_S8_UINT
                | F::D32_SFLOAT_S8_UINT
                | F::A8_UNORM_KHR
                | F::A1B5G5R5_UNORM_PACK16_KHR
        )
}

/// The 2D LDR ASTC formats (UNORM / SRGB) and their KTX_PACK_ASTC_BLOCK_DIMENSION.
fn astc_block_dimension(f: VkFormat) -> Option<u32> {
    use VkFormat as F;
    Some(match f {
        F::ASTC_4x4_UNORM_BLOCK | F::ASTC_4x4_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_4X4,
        F::ASTC_5x4_UNORM_BLOCK | F::ASTC_5x4_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_5X4,
        F::ASTC_5x5_UNORM_BLOCK | F::ASTC_5x5_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_5X5,
        F::ASTC_6x5_UNORM_BLOCK | F::ASTC_6x5_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_6X5,
        F::ASTC_6x6_UNORM_BLOCK | F::ASTC_6x6_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_6X6,
        F::ASTC_8x5_UNORM_BLOCK | F::ASTC_8x5_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_8X5,
        F::ASTC_8x6_UNORM_BLOCK | F::ASTC_8x6_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_8X6,
        F::ASTC_8x8_UNORM_BLOCK | F::ASTC_8x8_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_8X8,
        F::ASTC_10x5_UNORM_BLOCK | F::ASTC_10x5_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_10X5,
        F::ASTC_10x6_UNORM_BLOCK | F::ASTC_10x6_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_10X6,
        F::ASTC_10x8_UNORM_BLOCK | F::ASTC_10x8_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_10X8,
        F::ASTC_10x10_UNORM_BLOCK | F::ASTC_10x10_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_10X10,
        F::ASTC_12x10_UNORM_BLOCK | F::ASTC_12x10_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_12X10,
        F::ASTC_12x12_UNORM_BLOCK | F::ASTC_12x12_SRGB_BLOCK => KTX_PACK_ASTC_BLOCK_DIMENSION_12X12,
        _ => return None,
    })
}

fn is_format_astc_ldr_2d(f: VkFormat) -> bool {
    astc_block_dimension(f).is_some()
}

impl OptionsCreate {
    fn parse_transfer_function(
        args: &ParseResult,
        arg_name: &str,
        report: &Reporter,
    ) -> CmdResult<Option<u32>> {
        if args.count(arg_name) == 0 {
            return Ok(None);
        }
        let given = args.get(arg_name).unwrap_or_default();
        let upper = given.to_uppercase();
        let s = upper.strip_prefix("KHR_DF_TRANSFER_").unwrap_or(&upper);
        match transfer_from_name(s) {
            Some(v) => Ok(Some(v)),
            None => Err(report.fatal_usage(&format!(
                "Invalid or unsupported transfer specified as --{arg_name} argument: \"{given}\"."
            ))),
        }
    }

    fn parse_color_primaries(
        args: &ParseResult,
        arg_name: &str,
        report: &Reporter,
    ) -> CmdResult<Option<u32>> {
        if args.count(arg_name) == 0 {
            return Ok(None);
        }
        let given = args.get(arg_name).unwrap_or_default();
        let upper = given.to_uppercase();
        let s = upper.strip_prefix("KHR_DF_PRIMARIES_").unwrap_or(&upper);
        match primaries_from_name(s) {
            Some(v) => Ok(Some(v)),
            None => Err(report.fatal_usage(&format!(
                "Invalid or unsupported primaries specified as --{arg_name} argument: \"{given}\"."
            ))),
        }
    }

    /// `parseTexcoordOrigin`: `(top|bottom)-left(-(front|back))?`.
    fn parse_texcoord_origin(
        args: &ParseResult,
        num_dimensions: u32,
        arg_name: &str,
        report: &Reporter,
    ) -> CmdResult<Option<Origin>> {
        if args.count(arg_name) == 0 {
            return Ok(None);
        }
        let origin_str = args.get(arg_name).unwrap_or_default().to_lowercase();
        let parts: Vec<&str> = origin_str.split('-').collect();
        let parsed = match parts.as_slice() {
            [y @ ("top" | "bottom"), "left"] => Some((*y, "")),
            [y @ ("top" | "bottom"), "left", z @ ("front" | "back")] => Some((*y, *z)),
            _ => None,
        };
        let Some((y, z)) = parsed else {
            return Err(report.fatal_usage(&format!(
                "Invalid or unsupported origin specified as --{arg_name} argument: \"{origin_str}\"."
            )));
        };
        if num_dimensions == 3 && z.is_empty() {
            return Err(report.fatal_usage(&format!(
                "Z origin must be specified in --{arg_name} argument for a 3D texture."
            )));
        }
        let mut orig = Origin {
            x: Origin::E_LEFT,
            y: if y == "bottom" {
                Origin::E_BOTTOM
            } else {
                Origin::E_TOP
            },
            ..Origin::default()
        };
        if args.count(K_CUBEMAP) > 0 && (orig.x != Origin::E_LEFT || orig.y != Origin::E_TOP) {
            return Err(report.fatal_usage(&format!(
                "--{arg_name} argument must be top-left for a cubemap."
            )));
        }
        if num_dimensions == 3 {
            // Like the C++ (`compare` returns 0 for a match): "front" gives eBack.
            orig.z = if z != "front" {
                Origin::E_FRONT
            } else {
                Origin::E_BACK
            };
        }
        Ok(Some(orig))
    }

    fn check_swizzle(s: &str, opt: &str, report: &Reporter) -> CmdResult<String> {
        let sw = s.to_lowercase();
        if sw.len() != 4 || !sw.chars().all(|c| "rgba01".contains(c)) {
            return Err(report.fatal_usage(&format!(
                "Invalid --{opt} value: \"{sw}\". The value must match the \"[rgba01]{{4}}\" regex."
            )));
        }
        Ok(sw)
    }
}

impl OptionGroup for OptionsCreate {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::value(K_FORMAT, "KTX format enum that specifies the image data format. The enum names are matching the VkFormats without the VK_FORMAT_ prefix. The VK_FORMAT_ prefix is ignored if present.\nWhen used with --encode it specifies the target format before the encoding step. In this case it must be one of:\n    R8_UNORM\n    R8_SRGB\n    R8G8_UNORM\n    R8G8_SRGB\n    R8G8B8_UNORM\n    R8G8B8_SRGB\n    R8G8B8A8_UNORM\n    R8G8B8A8_SRGB\n    R16G16B16_SFLOAT\n    R16G16B16A16_SFLOAT\nIf the format is an ASTC format the ASTC encoder specific options become valid, otherwise they are ignored.\nThe format will be used to verify and load all input files into a texture before encoding. Case insensitive. Required.", "<enum>"));
        opts.add(OptionSpec::flag(
            K_1D,
            "Create a 1D texture. If not set the texture will be a 2D or 3D texture.",
        ));
        opts.add(OptionSpec::flag(
            K_CUBEMAP,
            "Create a cubemap texture. If not set the texture will be a 2D or 3D texture.",
        ));
        opts.add(OptionSpec::flag(K_RAW, "Create from raw image data."));
        opts.add(OptionSpec::value(K_WIDTH, "Base level width in pixels. Required with --raw. For non-raw, if not set, the image width is used otherwise the image is resampled to this width and any provided mip levels are resampled proportionately. For non-raw it enables use of the 'Generate Mipmap' options to tune the resampler.", "[0-9]+").u32());
        opts.add(OptionSpec::value(K_HEIGHT, "Base level height in pixels. Required with --raw. For non-raw, if not set, the image height is used otherwise the image is resampled to this height and any provided mip levels are resampled proportionately. For non-raw it enables use of the 'Generate Mipmap' options to tune the resampler.", "[0-9]+").u32());
        opts.add(
            OptionSpec::value(
                K_DEPTH,
                "Base level depth in pixels. If set the texture will be a 3D texture.",
                "[0-9]+",
            )
            .u32(),
        );
        opts.add(
            OptionSpec::value(
                K_LAYERS,
                "Number of layers. If set the texture will be an array texture.",
                "[0-9]+",
            )
            .u32(),
        );
        opts.add(OptionSpec::value(K_LEVELS, "Number of mip levels.", "[0-9]+").u32());
        opts.add(OptionSpec::flag(
            K_RUNTIME_MIPMAP,
            "Runtime mipmap generation mode.",
        ));
        opts.add(OptionSpec::flag(K_GENERATE_MIPMAP, "Causes mipmaps to be generated during texture creation. It enables the use of 'Generate Mipmap' options. If --levels is not specified the maximum possible mip level will be generated. This option is mutually exclusive with --runtime-mipmap and cannot be used with SINT, UINT or 3D textures or --raw."));
        opts.add(OptionSpec::value(K_SCALE, "Scale images as they are loaded. Cannot be used with --raw. It enables use of the 'Generate Mipmap' options to tune the resampler.", "<float>").f32());
        opts.add(OptionSpec::value(K_ENCODE, "Encode the created KTX file. Case insensitive.\nPossible options are: basis-lz | uastc | uastc-ldr-4x4 | uastc-hdr-4x4 | uastc-hdr-6x6i", "<codec>"));
        opts.add(OptionSpec::flag(K_NORMALIZE, "Normalize input normals to have a unit length. Only valid for\nlinear normal textures with 2 or more components. For 2-component\ninputs 2D unit normals are calculated. Do not use these 2D unit\nnormals to generate X+Y normals with --normal-mode. For 4-component\ninputs a 3D unit normal is calculated. 1.0 is used for the value of\nthe 4th component. Cannot be used with --raw."));
        opts.add(OptionSpec::flag(K_MAP_RANGE_AUTO, "If needed, map input values to the range supported by the target --format and --encode options.\nMapping excludes the alpha channel if present.\nIf enabled, this flag overrides arguments for --map-rang-offset and --map-range-scale."));
        opts.add(OptionSpec::value(K_MAP_RANGE_SCALE, "Scale input values before offseting by the --map-rang-offset value. Defaults to 1.0.", "<scale>").f32().default_value("1.0"));
        opts.add(
            OptionSpec::value(
                K_MAP_RANGE_OFFSET,
                "Offset input values after scaling by the --map-rang-scale value. Defaults to 0.0.",
                "<offset>",
            )
            .f32()
            .default_value("0.0"),
        );
        opts.add(OptionSpec::flag(K_PREMULTIPLY_ALPHA, "Pre-multiplies the color components of the input pixels by the alpha component before encoding and sets the flag in the metadata. Cannot be used with --normalize or --raw."));
        opts.add(OptionSpec::value(
            K_SWIZZLE,
            "KTX swizzle metadata.",
            "[rgba01]{4}",
        ));
        opts.add(OptionSpec::value(
            K_INPUT_SWIZZLE,
            "Pre-swizzle input channels.",
            "[rgba01]{4}",
        ));
        opts.add(OptionSpec::value(K_ASSIGN_TF, "Force the created texture to have the specified transfer function, ignoring the transfer function of the input file(s). Possible options match the khr_df_transfer_e enumerators without the KHR_DF_TRANSFER_ prefix. The KHR_DF_TRANSFER_ prefix is ignored if present. Case insensitive.\nThe options are: linear | srgb | srgb_eotf | scrgb | scrgb_eotf | itu | itu_oetf | bt601 | bt601_oetf | bt709 | bt709_oetf | bt2020 | bt2020_oetf | smpte170m | smpte170m_oetf | smpte170m_eotf | ntsc | ntsc_eotf | slog | slog_oetf | slog2 | slog2_oetf | bt1886 | bt1886_eotf | hlg_oetf | hlg_eotf | pq_oetf | pg_eotf | dcip3 | dcip3_eotf | pal_oetf | pal625_eotf | st240 | st240_oetf | st240_eotf | acescc | acescc_oetf | acescct | acescct_oetf | abobergb | adobergb_eotf", "<tf>"));
        opts.add(OptionSpec::value(K_ASSIGN_PRIMARIES, "Force the created texture to have the specified color primaries, ignoring the color primaries of the input file(s). Possible options match the khr_df_primaries_e enumerators without the KHR_DF_PRIMARIES_ prefix. The KHR_DF_PRIMARIES_ prefix is ignored if present. Case insensitive.\nThe options are: none | bt709 | srgb | bt601_ebu | bt601_smpte | bt2020 | ciexyz | aces | acescc | ntsc1953 | pal525 | displayp3 | adobergb.", "<primaries>"));
        opts.add(OptionSpec::value(K_ASSIGN_TEXCOORD_ORIGIN, "Force the created texture to indicate that the texture coordinate origin s=0, t=0 is at the specified corner of the image. Case insensitive.\nPossible options are top-left | bottom-left. -front | -back can be appended and one of these is required when --depth is specified. Must be top-left if --cubemap is specified.\nAbsent --convert-texcoord-origin, the effect of this option is to cause KTXorientation metadata indicating the specified origin to be written to the output file.", "<origin>"));
        opts.add(OptionSpec::value(K_CONVERT_TF, "Convert the input image(s) to the specified transfer function, if different from the transfer function of the input file(s). If both this and --assign-tf are specified, conversion will be performed from the assigned transfer function to the transfer function specified by this option, if different. Case insensitive.\nPossible options are: linear | srgb. The following srgb aliases are also supported: srgb_eotf | scrgb | scrgb_eotf", "<tf>"));
        opts.add(OptionSpec::value(K_CONVERT_PRIMARIES, "Convert the image image(s) to the specified color primaries, if different from the color primaries of the input file(s) or the one specified by --assign-primaries. If both this and --assign-primaries are specified, conversion will be performed from  the assigned primaries to the primaries specified by this option, if different. This option is not allowed to be specified when --assign-primaries is set to 'none'. Possible options match the khr_df_transfer_e enumerators without the KHR_DF_TRANSFER_ prefix. The KHR_DF_PRIMARIES_ prefix is ignored if present. Case insensitive.\nThe options are: bt709 | srgb | bt601_ebu | bt601_smpte | bt2020 | ciexyz | aces | acescc | ntsc1953 | pal525 | displayp3 | adobergb.", "<primaries>"));
        opts.add(OptionSpec::value(K_CONVERT_TEXCOORD_ORIGIN, "Convert the input image(s) so the texture coordinate origin s=0, t=0, is at the specified corner of the image. If both this and --assign-texcoord-origin are specified, conversion will be performed from the assigned origin to the origin specified by this option, if different. Case insensitive.\nPossible options are top-left | bottom-left. -front | -back can be appended and one of these is required when --depth is specified. Must be top-left if --cubemap is specified.\nInput images whose origin does not match corner will be flipped vertically. KTXorientation metadata indicating the specified origin is written to the output file.", "<origin>"));
        opts.add(OptionSpec::flag(
            K_FAIL_ON_COLOR_CONVERSIONS,
            "Generates an error if any of the input images would need to be color converted.",
        ));
        opts.add(OptionSpec::flag(
            K_WARN_ON_COLOR_CONVERSIONS,
            "Generates a warning if any of the input images are color converted.",
        ));
        opts.add(OptionSpec::flag(K_NO_WARN_ON_COLOR_CONVERSIONS, "Disable all warnings about color conversions including that for visually lossy conversions. Overrides --warn-on-color-conversions should both be specified."));
        opts.add(OptionSpec::flag(K_FAIL_ON_ORIGIN_CHANGES, "Generates an error if any of the input images would need to have their origin changed."));
        opts.add(OptionSpec::flag(
            K_WARN_ON_ORIGIN_CHANGES,
            "Generates a warning if any of the input images have their origin changed.",
        ));

        let g = "Generate Mipmap";
        opts.add(OptionSpec::value(K_MIPMAP_FILTER, "Specifies the filter to use when generating the mipmaps. Case insensitive. Ignored unless --generate-mipmap, --scale, --width or --height are specified for non-raw input.\nPossible options are: box | tent | bell | b-spline | mitchell | blackman | lanczos3 | lanczos4 | lanczos6 | lanczos12 | kaiser | gaussian | catmullrom | quadratic_interp | quadratic_approx |  quadratic_mix. Defaults to lanczos4.", "<filter>").group(g));
        opts.add(OptionSpec::value(K_MIPMAP_FILTER_SCALE, "The filter scale to use. Defaults to 1.0. Ignored unless --generate-mipmap, --scale, --width or --height are specified for non-raw input.", "<float>").f32().group(g));
        opts.add(OptionSpec::value(K_MIPMAP_WRAP, "Specify how to sample pixels near the image boundaries. Case insensitive. Ignored unless --generate-mipmap, --scale, --width or --height are specified for non-raw input.\nPossible options are: wrap | reflect | clamp. Defaults to clamp.", "<mode>").group(g));
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        let usage = |m: String| -> FatalError { report.fatal_usage(&m) };
        self._1d = args.as_bool(K_1D);
        self.cubemap = args.as_bool(K_CUBEMAP);
        self.raw = args.as_bool(K_RAW);

        let opt_u32 = |k: &str| (args.count(k) > 0).then(|| args.as_u32(k));
        self.width = opt_u32(K_WIDTH);
        self.height = opt_u32(K_HEIGHT);
        self.depth = opt_u32(K_DEPTH);
        self.layers = opt_u32(K_LAYERS);
        self.levels = opt_u32(K_LEVELS);

        self.mipmap_runtime = args.as_bool(K_RUNTIME_MIPMAP);
        self.mipmap_generate = args.as_bool(K_GENERATE_MIPMAP);

        if args.count(K_MIPMAP_FILTER) > 0 {
            let f = args.get(K_MIPMAP_FILTER).unwrap_or_default().to_lowercase();
            const FILTERS: [&str; 16] = [
                "box",
                "tent",
                "bell",
                "b-spline",
                "mitchell",
                "blackman",
                "lanczos3",
                "lanczos4",
                "lanczos6",
                "lanczos12",
                "kaiser",
                "gaussian",
                "catmullrom",
                "quadratic_interp",
                "quadratic_approx",
                "quadratic_mix",
            ];
            if !FILTERS.contains(&f.as_str()) {
                return Err(usage(format!(
                    "Invalid or unsupported mipmap filter specified as --mipmap-filter argument: \"{f}\"."
                )));
            }
            self.mipmap_filter = Some(f);
        }
        if args.count(K_MIPMAP_FILTER_SCALE) > 0 {
            self.mipmap_filter_scale = Some(args.as_f32(K_MIPMAP_FILTER_SCALE));
        }
        if args.count(K_MIPMAP_WRAP) > 0 {
            let w = args.get(K_MIPMAP_WRAP).unwrap_or_default().to_lowercase();
            self.mipmap_wrap = Some(match w.as_str() {
                "clamp" => BoundaryOp::Clamp,
                "wrap" => BoundaryOp::Wrap,
                "reflect" => BoundaryOp::Reflect,
                _ => {
                    return Err(usage(format!(
                        "Invalid or unsupported mipmap wrap mode specified as --mipmap-wrap argument: \"{w}\"."
                    )));
                }
            });
        }

        if args.count(K_NORMALIZE) > 0 {
            if self.raw {
                return Err(usage(format!(
                    "Option --{K_NORMALIZE} cannot be used with --{K_RAW}."
                )));
            }
            self.normalize = true;
        }
        if args.count(K_SWIZZLE) > 0 {
            self.swizzle = Some(Self::check_swizzle(
                args.get(K_SWIZZLE).unwrap_or_default(),
                K_SWIZZLE,
                report,
            )?);
        }
        if args.count(K_INPUT_SWIZZLE) > 0 {
            self.swizzle_input = Some(Self::check_swizzle(
                args.get(K_INPUT_SWIZZLE).unwrap_or_default(),
                K_INPUT_SWIZZLE,
                report,
            )?);
        }

        let num_dimensions = if args.count(K_DEPTH) > 0 {
            3
        } else if args.count(K_1D) > 0 {
            1
        } else {
            2
        };
        self.assign_texcoord_origin =
            Self::parse_texcoord_origin(args, num_dimensions, K_ASSIGN_TEXCOORD_ORIGIN, report)?;
        self.convert_texcoord_origin =
            Self::parse_texcoord_origin(args, num_dimensions, K_CONVERT_TEXCOORD_ORIGIN, report)?;

        if args.count(K_FORMAT) > 0 {
            let format_str = args.get(K_FORMAT).unwrap_or_default().to_string();
            match parse_vk_format(&format_str) {
                Some(f) => self.vk_format = f,
                None => {
                    return Err(usage(format!(
                        "The requested format is invalid or unsupported: \"{format_str}\"."
                    )));
                }
            }
            self.format_arg = format_str;
        } else {
            return Err(usage("Required option 'format' is missing.".into()));
        }

        if args.count(K_SCALE) > 0 {
            if args.count(K_WIDTH) > 0 || args.count(K_HEIGHT) > 0 {
                return Err(usage(format!(
                    "{K_SCALE} cannot be used with {K_WIDTH} or {K_HEIGHT}."
                )));
            }
            self.image_scale = Some(args.as_f32(K_SCALE));
        }

        if args.count(K_MAP_RANGE_AUTO) > 0 {
            self.map_range_runtime = true;
        } else {
            self.offset = Some(args.as_f32(K_MAP_RANGE_OFFSET));
            self.scale = Some(args.as_f32(K_MAP_RANGE_SCALE));
        }

        let vk = self.vk_format;
        if is_prohibited_format(vk) {
            return Err(usage(format!(
                "The requested {} format is prohibited in KTX files.",
                to_string_vk_format(vk)
            )));
        }
        if !self.raw && !is_convertable_format(vk) {
            return Err(usage(format!(
                "Unsupported format for non-raw create: {}.",
                to_string_vk_format(vk)
            )));
        }
        if self.raw {
            if self.width.is_none() {
                return Err(usage(
                    "Option --width is missing but is required for --raw texture creation.".into(),
                ));
            }
            if self.height.is_none() {
                return Err(usage(
                    "Option --height is missing but is required for --raw texture creation.".into(),
                ));
            }
        }
        if self.width == Some(0) {
            return Err(usage("The --width cannot be 0.".into()));
        }
        if self.height == Some(0) {
            return Err(usage("The --height cannot be 0.".into()));
        }
        if self.layers == Some(0) {
            return Err(usage("The --layers cannot be 0.".into()));
        }
        if self.levels == Some(0) {
            return Err(usage("The --levels cannot be 0.".into()));
        }
        if self.depth == Some(0) {
            return Err(usage("The --depth cannot be 0.".into()));
        }
        if self.raw {
            let (w, h, d) = (
                self.width.unwrap_or(1),
                self.height.unwrap_or(1),
                self.depth.unwrap_or(1),
            );
            let max_levels = log2(w.max(h.max(d))) + 1;
            if self.levels.unwrap_or(1) > max_levels {
                return Err(usage(format!(
                    "Requested {} levels is too many. With base size {w}x{h}x{d} the texture can only have {max_levels} levels at most.",
                    self.levels.unwrap_or(1)
                )));
            }
        }
        if self._1d && self.height.is_some_and(|h| h != 1) {
            return Err(usage("For --1d textures the --height must be 1.".into()));
        }
        if let (Some(l), Some(d)) = (self.layers, self.depth) {
            return Err(usage(format!(
                "3D array texture creation is unsupported. --layers is {l} and --depth is {d}."
            )));
        }
        if self.cubemap
            && let Some(d) = self.depth
        {
            return Err(usage(format!(
                "Cubemaps cannot have 3D textures. --depth is {d}."
            )));
        }
        if self.mipmap_runtime && self.levels.unwrap_or(1) > 1 {
            return Err(usage(
                "Conflicting options: --runtime-mipmap cannot be used with more than 1 --levels."
                    .into(),
            ));
        }
        if self.mipmap_generate && self.mipmap_runtime {
            return Err(usage("Conflicting options: --generate-mipmap and --runtime-mipmap cannot be used together.".into()));
        }
        if self.mipmap_generate && self.raw {
            return Err(usage(
                "Conflicting options: --generate-mipmap cannot be used with --raw.".into(),
            ));
        }
        if self.mipmap_generate && self.depth.is_some() {
            return Err(usage("Mipmap generation for 3D textures is not supported: --generate-mipmap cannot be used with --depth.".into()));
        }

        self.format_desc = FormatDescriptor::from_vk_format(vk, report)?;

        self.convert_tf = Self::parse_transfer_function(args, K_CONVERT_TF, report)?;
        self.assign_tf = Self::parse_transfer_function(args, K_ASSIGN_TF, report)?;
        self.convert_tf_arg = args.get(K_CONVERT_TF).unwrap_or_default().to_string();
        self.assign_tf_arg = args.get(K_ASSIGN_TF).unwrap_or_default().to_string();

        if let Some(tf) = self.convert_tf
            && tf != KHR_DF_TRANSFER_LINEAR
            && tf != KHR_DF_TRANSFER_SRGB
        {
            return Err(usage(format!(
                "Unsupported transfer function {} for --{K_CONVERT_TF}.",
                self.convert_tf_arg
            )));
        }

        self.convert_primaries = Self::parse_color_primaries(args, K_CONVERT_PRIMARIES, report)?;
        self.assign_primaries = Self::parse_color_primaries(args, K_ASSIGN_PRIMARIES, report)?;

        if self.convert_primaries.is_some()
            && self.assign_primaries == Some(KHR_DF_PRIMARIES_UNSPECIFIED)
        {
            return Err(usage(format!(
                "Option --{K_CONVERT_PRIMARIES} cannot be used when --{K_ASSIGN_PRIMARIES} is set to 'none'."
            )));
        }
        if self.raw {
            if self.convert_tf.is_some() {
                return Err(usage(format!(
                    "Option {K_CONVERT_TF} cannot be used with --{K_RAW}."
                )));
            }
            if self.convert_primaries.is_some() {
                return Err(usage(format!(
                    "Option {K_CONVERT_PRIMARIES} cannot be used with --{K_RAW}."
                )));
            }
            if self.convert_texcoord_origin.is_some() {
                return Err(usage(format!(
                    "Option {K_CONVERT_TEXCOORD_ORIGIN} cannot be used with --{K_RAW}."
                )));
            }
            if self.image_scale.is_some() {
                return Err(usage(format!(
                    "Option {K_SCALE} cannot be used with --{K_RAW}."
                )));
            }
        }

        if self.format_desc.transfer == KHR_DF_TRANSFER_SRGB {
            let err = |v: &str, opt: &str| {
                usage(format!(
                    "Invalid value \"{v}\" to --{opt} for format \"{}\". Transfer function must be sRGB for sRGB formats.",
                    self.format_arg
                ))
            };
            if self.convert_tf.is_none()
                && let Some(a) = self.assign_tf
            {
                if a != KHR_DF_TRANSFER_UNSPECIFIED && a != KHR_DF_TRANSFER_SRGB {
                    return Err(err(&self.assign_tf_arg, K_ASSIGN_TF));
                }
            } else if self.convert_tf.is_some_and(|c| c != KHR_DF_TRANSFER_SRGB) {
                return Err(err(&self.convert_tf_arg, K_CONVERT_TF));
            }
        }

        if is_format_srgb(vk) && self.normalize {
            return Err(usage(format!(
                "Option --{K_NORMALIZE} cannot be used with sRGB formats."
            )));
        }

        if is_format_not_srgb_but_has_srgb_variant(vk) {
            let err = |v: &str, opt: &str| {
                usage(format!(
                    "Invalid value \"{v}\" to --{opt} for format \"{}\". Transfer function must not be sRGB for a non-sRGB VkFormat with sRGB variant.",
                    self.format_arg
                ))
            };
            if self.convert_tf.is_none() && self.assign_tf == Some(KHR_DF_TRANSFER_SRGB) {
                return Err(err(&self.assign_tf_arg, K_ASSIGN_TF));
            } else if self.convert_tf == Some(KHR_DF_TRANSFER_SRGB) {
                return Err(err(&self.convert_tf_arg, K_CONVERT_TF));
            }
        }

        if args.count(K_FAIL_ON_COLOR_CONVERSIONS) > 0 {
            self.fail_on_color_conversions = true;
        }
        if args.count(K_WARN_ON_COLOR_CONVERSIONS) > 0 {
            if self.fail_on_color_conversions {
                return Err(usage(format!(
                    "The options --{K_FAIL_ON_COLOR_CONVERSIONS} and --{K_WARN_ON_COLOR_CONVERSIONS} are mutually exclusive."
                )));
            }
            self.warn_on_color_conversions = true;
        }
        if args.count(K_NO_WARN_ON_COLOR_CONVERSIONS) > 0 {
            self.no_warn_on_color_conversions = true;
            self.warn_on_color_conversions = false;
        }
        if args.count(K_FAIL_ON_ORIGIN_CHANGES) > 0 {
            self.fail_on_origin_changes = true;
        }
        if args.count(K_WARN_ON_ORIGIN_CHANGES) > 0 {
            if self.fail_on_origin_changes {
                return Err(usage(format!(
                    "The options --{K_FAIL_ON_ORIGIN_CHANGES} and --{K_WARN_ON_ORIGIN_CHANGES} are mutually exclusive."
                )));
            }
            self.warn_on_origin_changes = true;
        }
        if args.count(K_PREMULTIPLY_ALPHA) > 0 {
            if self.normalize {
                return Err(usage(format!(
                    "The options --{K_PREMULTIPLY_ALPHA} cannot be used with --{K_NORMALIZE}"
                )));
            }
            if self.raw {
                return Err(usage(format!(
                    "The options --{K_PREMULTIPLY_ALPHA} cannot be used with --{K_RAW}"
                )));
            }
            self.premultiply_alpha = true;
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// `SrcColorSpaceInfo`.
#[derive(Default)]
struct SrcColorSpaceInfo {
    used_transfer_function: u32,
    used_primaries: u32,
    transfer_function: Option<Box<dyn TransferFunction>>,
    color_primaries: Option<ColorPrimaries>,
}

/// `DstColorSpaceInfo`.
#[derive(Default)]
struct DstColorSpaceInfo {
    transfer_function: Option<Box<dyn TransferFunction>>,
    color_primaries: Option<ColorPrimaries>,
}

/// `ColorSpaceInfo`.
#[derive(Default)]
struct ColorSpaceInfo {
    src: SrcColorSpaceInfo,
    dst: DstColorSpaceInfo,
}

/// Port of `CommandCreate`.
#[derive(Debug, Default)]
pub struct CommandCreate {
    report: Reporter,
    options: OptionsCreate,
    astc: OptionsEncodeASTC,
    basis: OptionsEncodeBasis<false>,
    common: OptionsEncodeCommon,
    metrics: OptionsMetrics,
    deflate: OptionsDeflate,
    in_out: OptionsMultiInSingleOut,
    generic: OptionsGeneric,

    num_levels: u32,
    num_layers: u32,
    num_faces: u32,
    base_depth: u32,
    target_channel_count: u32,
    scale: f32,
    offset: f32,
}

/// `std::exception` escaping to `main`: "{command} fatal: {what}", RUNTIME_ERROR.
fn exception(report: &Reporter, what: &str) -> FatalError {
    report.fatal(ReturnCode::RuntimeError, what)
}

fn image_err(report: &Reporter, e: ImageError) -> FatalError {
    exception(report, e.what())
}

impl CommandCreate {
    /// Port of `CommandCreate::main`.
    pub fn main(args: &[String]) -> i32 {
        let mut cmd = CommandCreate {
            scale: 1.0,
            offset: 0.0,
            ..CommandCreate::default()
        };
        match cmd.run(args) {
            Ok(()) => ReturnCode::Success as i32,
            Err(FatalError(rc)) => rc,
        }
    }

    fn run(&mut self, args: &[String]) -> CmdResult {
        let parsed = parse_command_line(
            &mut self.report,
            "ktx create",
            "Create, encode and supercompress a KTX2 file from the input images specified as the\n    input-file... arguments and save it as the output-file.",
            args,
            &mut [
                &mut self.options,
                &mut self.astc,
                &mut self.basis,
                &mut self.common,
                &mut self.metrics,
                &mut self.deflate,
                &mut self.in_out,
                &mut self.generic,
            ],
        )?;
        self.process_options(&parsed)?;
        self.execute_create()
    }

    fn usage(&self, msg: String) -> FatalError {
        self.report.fatal_usage(&msg)
    }

    fn fatal(&self, rc: ReturnCode, msg: String) -> FatalError {
        self.report.fatal(rc, &msg)
    }

    /// `checkNumInputImages`.
    fn check_num_input_images(&self) -> CmdResult {
        let block_size_z = if is_format_3d_block_compressed(self.options.vk_format) {
            self.options.format_desc.texel_block_dimension[2] + 1
        } else {
            1
        };
        let mut expected: u32 = 0;
        let levels = if self.options.mipmap_generate {
            1
        } else {
            self.options.levels.unwrap_or(1)
        };
        for i in 0..levels {
            // If --generate-mipmap is set the input only contains the base level images
            let d = (self.base_depth.checked_shr(i).unwrap_or(0)).max(1);
            expected = expected.wrapping_add(
                self.num_layers
                    .wrapping_mul(self.num_faces)
                    .wrapping_mul(ceil_div(d, block_size_z)),
            );
        }
        let n = self.in_out.input_filepaths.len();
        if n != expected as usize {
            return Err(self.usage(format!(
                "Too {} input images for {} level{}, {} layer, {} face and {} depth. Provided {} but expected {}.",
                if n > expected as usize { "many" } else { "few" },
                self.num_levels,
                if self.options.mipmap_generate { " (mips generated)" } else { "" },
                self.num_layers,
                self.num_faces,
                self.base_depth,
                n,
                expected
            )));
        }
        Ok(())
    }

    /// `CommandCreate::processOptions` after `options.process`.
    fn process_options(&mut self, args: &ParseResult) -> CmdResult {
        self.num_levels = self.options.levels.unwrap_or(1);
        self.num_layers = self.options.layers.unwrap_or(1);
        self.num_faces = if self.options.cubemap { 6 } else { 1 };
        self.base_depth = self.options.depth.unwrap_or(1);

        if self.options.raw {
            // options.levels <= max for dimensions was checked in CreateOptions::process
            self.check_num_input_images()?;
        }

        let vk = self.options.vk_format;
        if !is_format_astc(vk) {
            for astc_option in OptionsEncodeASTC::K_ASTC_OPTIONS {
                if args.count(astc_option) > 0 {
                    return Err(self.usage(format!(
                        "--{astc_option} can only be used with ASTC formats."
                    )));
                }
            }
        } else {
            fill_options_codec_astc(&self.common, &mut self.astc.params);
            if self.common.no_sse {
                return Err(self.usage(format!(
                    "--{} is not allowed with ASTC encode",
                    OptionsEncodeCommon::K_NO_SSE
                )));
            }
        }

        let codec = self.basis.selected_codec;
        if codec == BasisCodec::BasisLZ {
            if self.deflate.zstd.is_some() {
                return Err(
                    self.usage("Cannot encode to BasisLZ and supercompress with Zstd.".into())
                );
            }
            if self.deflate.zlib.is_some() {
                return Err(
                    self.usage("Cannot encode to BasisLZ and supercompress with ZLIB.".into())
                );
            }
        }
        if codec == BasisCodec::UastcHdr6x6i {
            if self.deflate.zstd.is_some() {
                return Err(self
                    .usage("Cannot encode to UASTC HDR 6x6i and supercompress with Zstd.".into()));
            }
            if self.deflate.zlib.is_some() {
                return Err(self
                    .usage("Cannot encode to UASTC HDR 6x6i and supercompress with ZLIB.".into()));
            }
        }
        let hdr_codec = codec == BasisCodec::UastcHdr4x4 || codec == BasisCodec::UastcHdr6x6i;
        let sfloat16 = vk == VkFormat::R16G16B16_SFLOAT || vk == VkFormat::R16G16B16A16_SFLOAT;
        if hdr_codec && self.options.raw && !sfloat16 {
            return Err(self.usage("Cannot encode to UASTC-HDR from RAW due to incorrect format value. The format should be either R16G16B16_SFLOAT or R16G16B16A16_SFLOAT.".into()));
        }
        if codec == BasisCodec::BasisLZ || codec == BasisCodec::UastcLdr4x4 {
            use VkFormat as F;
            if !matches!(
                vk,
                F::R8_UNORM
                    | F::R8_SRGB
                    | F::R8G8_UNORM
                    | F::R8G8_SRGB
                    | F::R8G8B8_UNORM
                    | F::R8G8B8_SRGB
                    | F::R8G8B8A8_UNORM
                    | F::R8G8B8A8_SRGB
            ) {
                return Err(self.usage(format!(
                    "Only R8, RG8, RGB8, or RGBA8 UNORM and SRGB formats can be encoded to {}, but format is {}.",
                    to_string_basis_codec(codec),
                    to_string_vk_format(vk)
                )));
            }
        } else if hdr_codec && !sfloat16 {
            return Err(self.usage(format!(
                "Only RGB16 or RGBA16 SFLOAT can be encoded to {}, but format is {}.",
                to_string_basis_codec(codec),
                to_string_vk_format(vk)
            )));
        }

        let basis_codec = matches!(
            codec,
            BasisCodec::BasisLZ
                | BasisCodec::UastcLdr4x4
                | BasisCodec::UastcHdr4x4
                | BasisCodec::UastcHdr6x6i
        );
        let can_compare = basis_codec || is_format_astc(vk);
        if basis_codec {
            fill_options_codec_basis(&self.common, &mut self.basis.params);
        }
        if self.metrics.compare_ssim && !can_compare {
            return Err(self.usage(
                "--compare-ssim can only be used with BasisLZ, UASTC or ASTC encoding.".into(),
            ));
        }
        if self.metrics.compare_psnr && !can_compare {
            return Err(self.usage(
                "--compare-psnr can only be used with BasisLZ, UASTC or ASTC encoding.".into(),
            ));
        }

        if is_format_astc(vk) && !self.options.raw {
            self.astc.encode_astc = true;
            match astc_block_dimension(vk) {
                Some(dim) => {
                    self.astc.params.mode = KTX_PACK_ASTC_ENCODER_MODE_LDR;
                    self.astc.params.block_dimension = dim;
                }
                None => {
                    return Err(self.fatal(
                        ReturnCode::NotSupported,
                        format!(
                            "{} is unsupported for ASTC encoding.",
                            to_string_vk_format(vk)
                        ),
                    ));
                }
            }
        }
        if self.options._1d && self.astc.encode_astc {
            return Err(self.usage(format!(
                "ASTC format {} cannot be used for 1 dimensional textures (indicated by --1d).",
                to_string_vk_format(vk)
            )));
        }
        Ok(())
    }
}

/// `createColorPrimaries` (BT709 for unknown values).
fn create_color_primaries(primaries: u32) -> ColorPrimaries {
    match primaries {
        KHR_DF_PRIMARIES_BT601_EBU => ColorPrimaries::bt601_625_ebu(),
        KHR_DF_PRIMARIES_BT601_SMPTE => ColorPrimaries::bt601_525_smpte(),
        KHR_DF_PRIMARIES_BT2020 => ColorPrimaries::bt2020(),
        KHR_DF_PRIMARIES_CIEXYZ => ColorPrimaries::ciexyz(),
        KHR_DF_PRIMARIES_ACES => ColorPrimaries::aces(),
        KHR_DF_PRIMARIES_ACESCC => ColorPrimaries::acescc(),
        KHR_DF_PRIMARIES_NTSC1953 => ColorPrimaries::ntsc1953(),
        KHR_DF_PRIMARIES_PAL525 => ColorPrimaries::pal525(),
        KHR_DF_PRIMARIES_DISPLAYP3 => ColorPrimaries::display_p3(),
        KHR_DF_PRIMARIES_ADOBERGB => ColorPrimaries::adobe_rgb(),
        _ => ColorPrimaries::bt709(),
    }
}

/// `std::to_string(float)` (`%f`).
fn std_to_string(v: f32) -> String {
    format!("{:.6}", v)
}

/// The `KTXmapRange` value (`struct { float scale; float offset; }`).
fn map_range_bytes(scale: f32, offset: f32) -> Vec<u8> {
    let mut v = scale.to_le_bytes().to_vec();
    v.extend_from_slice(&offset.to_le_bytes());
    v
}

impl CommandCreate {
    fn warning(&self, msg: &str) {
        self.report.warning(msg);
    }

    /// `foreachImage` order: level > layer > face > z slice (of blocks).
    fn image_list(&self) -> Vec<(String, u32, u32, u32, u32)> {
        let mut out = Vec::new();
        let mut it = self.in_out.input_filepaths.iter();
        let levels = if self.options.mipmap_generate {
            1
        } else {
            self.num_levels
        };
        let block_z = self.options.format_desc.texel_block_dimension[2] + 1;
        for level in 0..levels {
            let slices = ceil_div(
                self.base_depth.checked_shr(level).unwrap_or(0).max(1),
                block_z,
            );
            for layer in 0..self.num_layers {
                for face in 0..self.num_faces {
                    for slice in 0..slices {
                        // inputFilepaths size was already validated during arg parsing
                        if let Some(p) = it.next() {
                            out.push((p.clone(), level, layer, face, slice));
                        }
                    }
                }
            }
        }
        out
    }

    /// `readRawFile`.
    fn read_raw_file(&self, filepath: &str) -> CmdResult<Vec<u8>> {
        Ok(InputStream::new(filepath, &self.report)?.data)
    }

    /// `createTexture`.
    fn create_texture(&self, target: &ImageSpec) -> CmdResult<Texture2> {
        let o = &self.options;
        let mut ci = TextureCreateInfo {
            vk_format: o.vk_format,
            num_faces: self.num_faces,
            num_layers: self.num_layers,
            is_array: o.layers.is_some_and(|l| l > 0),
            base_width: target.width(),
            base_height: target.height(),
            base_depth: target.depth(),
            num_dimensions: if o._1d {
                1
            } else if o.depth.is_some_and(|d| d > 0) {
                3
            } else {
                2
            },
            ..TextureCreateInfo::default()
        };
        if o.mipmap_runtime {
            ci.generate_mipmaps = true;
            ci.num_levels = 1;
        } else {
            ci.generate_mipmaps = false;
            ci.num_levels = if o.mipmap_generate {
                let max_levels = log2(target.width().max(target.height().max(target.depth()))) + 1;
                o.levels.unwrap_or(max_levels)
            } else {
                o.levels.unwrap_or(1)
            };
        }
        let mut texture = match Texture2::create(&ci, TextureCreateStorage::AllocStorage) {
            Ok(t) => t,
            Err(e) => {
                return Err(self.fatal(
                    ReturnCode::RuntimeError,
                    format!(
                        "Failed to create ktxTexture: libktx error: {}",
                        ktx_error_string(e)
                    ),
                ));
            }
        };

        let fmt = target.format();
        if let Some(bdb) = texture.p_dfd.get_mut(1..) {
            dfdsetval(bdb, PRIMARIES, fmt.primaries);
            dfdsetval(bdb, TRANSFER, fmt.transfer);
            if o.premultiply_alpha {
                dfdsetval(bdb, FLAGS, KHR_DF_FLAG_ALPHA_PREMULTIPLIED);
            }
            // Qualifier is only set for non-linear transfer functions. ktxTexture2_Create will
            // already have set the qualifier for _SRGB formats.
            if fmt.transfer != KHR_DF_TRANSFER_UNSPECIFIED
                && fmt.transfer != KHR_DF_TRANSFER_LINEAR
                && !is_format_srgb(o.vk_format)
            {
                for s in 0..dfdsamplecount(bdb) {
                    if dfdsval(bdb, s, CHANNELID) == KHR_DF_CHANNEL_RGBSDA_ALPHA {
                        let q = dfdsval(bdb, s, QUALIFIERS);
                        dfdsetsval(bdb, s, QUALIFIERS, q | KHR_DF_SAMPLE_DATATYPE_LINEAR);
                    }
                }
            }
        }

        // Add KTXorientation metadata
        if o.assign_texcoord_origin.is_some() || o.convert_texcoord_origin.is_some() {
            let origin = target.origin();
            let mut orientation = String::from(if origin.x == Origin::E_LEFT { "r" } else { "l" });
            if !o._1d {
                orientation.push_str(if origin.y == Origin::E_TOP { "d" } else { "u" });
                if o.depth.is_some() {
                    orientation.push_str(if origin.z == Origin::E_FRONT {
                        "i"
                    } else {
                        "o"
                    });
                }
            }
            let mut value = orientation.into_bytes();
            value.push(0);
            let _ = texture
                .kv_data_head
                .add_kv_pair(KTX_ORIENTATION_KEY, &value);
        }
        Ok(texture)
    }

    /// `loadInputImage`.
    fn load_input_image(&self, input: &mut dyn ImageInput) -> CmdResult<Box<dyn Image>> {
        let spec = input.spec().clone();
        let (width, height) = (spec.width(), spec.height());
        let input_bit_length = spec.format().largest_channel_bit_length();
        let request_bit_length = bit_ceil(input_bit_length).max(8);
        let (mut image, load_format): (Box<dyn Image>, VkFormat) = match input.format_type() {
            ImageInputFormatType::ExrUint => (
                Box::new(Rgba32Image::new(width, height)),
                VkFormat::R32G32B32A32_UINT,
            ),
            ImageInputFormatType::ExrFloat => (
                Box::new(Rgba32fImage::new(width, height)),
                VkFormat::R32G32B32A32_SFLOAT,
            ),
            _ => match request_bit_length {
                8 => (
                    Box::new(Rgba8Image::new(width, height)),
                    VkFormat::R8G8B8A8_UNORM,
                ),
                16 => (
                    Box::new(Rgba16Image::new(width, height)),
                    VkFormat::R16G16B16A16_UNORM,
                ),
                _ => {
                    return Err(self.fatal(
                        ReturnCode::InvalidFile,
                        format!("Unsupported format with {request_bit_length}-bit channels."),
                    ));
                }
            },
        };
        let load_format = FormatDescriptor::from_vk_format(load_format, &self.report)?;
        let n = image.get_byte_count();
        let buf = image.as_bytes_mut();
        let len = n.min(buf.len());
        input
            .read_image(&mut buf[..len], 0, 0, &load_format)
            .map_err(|e| image_err(&self.report, e))?;
        Ok(image)
    }

    /// `determineSourceColorSpace`.
    fn determine_source_color_space(
        &self,
        input: &dyn ImageInput,
        src: &mut SrcColorSpaceInfo,
    ) -> CmdResult {
        let o = &self.options;
        let spec = input.spec();
        let fmt = spec.format();
        let is_png = input.format_name() == "png";

        src.color_primaries = None;
        src.used_primaries = fmt.primaries;
        if let Some(p) = o.assign_primaries {
            src.used_primaries = p;
        } else if fmt.primaries == KHR_DF_PRIMARIES_UNSPECIFIED && is_png {
            self.warning(&format!(
                "No color primaries in PNG input file \"{}\", defaulting to BT.709.",
                input.filename()
            ));
            src.used_primaries = KHR_DF_PRIMARIES_BT709;
        }

        if let Some(cp) = o.convert_primaries {
            if src.used_primaries == KHR_DF_PRIMARIES_UNSPECIFIED {
                return Err(self.fatal(ReturnCode::InvalidFile, format!(
                    "Cannot convert primaries as no information about the color primaries is available in the input file \"{}\". Use --{K_ASSIGN_PRIMARIES} to specify one.",
                    input.filename()
                )));
            } else if cp != src.used_primaries {
                src.color_primaries = Some(create_color_primaries(src.used_primaries));
            }
        }

        let channel_bit_length = || -> CmdResult<u32> {
            fmt.channel_bit_length()
                .map_err(|e| exception(&self.report, &e))
        };

        src.transfer_function = None;
        if let Some(tf) = o.assign_tf {
            src.used_transfer_function = tf;
        } else {
            // Set image's transfer function as indicated by metadata.
            src.used_transfer_function = fmt.transfer;
            if fmt.transfer == KHR_DF_TRANSFER_UNSPECIFIED {
                let gamma = fmt.oe_gamma;
                if !fmt.icc_profile_name.is_empty() {
                    return Err(self.fatal(ReturnCode::InvalidFile, format!(
                        "Input file \"{}\" contains unsupported ICC profile \"{}\". Use --{K_ASSIGN_TF} to specify a transfer function.",
                        input.filename(),
                        String::from_utf8_lossy(&fmt.icc_profile_name)
                    )));
                } else if gamma > 0.0 {
                    if gamma > 0.45450 && gamma < 0.45460 {
                        if !o.no_warn_on_color_conversions
                            && o.convert_tf != Some(KHR_DF_TRANSFER_SRGB)
                        {
                            self.warning(&format!("Converting gamma 2.2f to sRGB. Use --{K_ASSIGN_TF} srgb to force treating input as sRGB or --{K_CONVERT_TF} srgb to approve conversion."));
                        }
                        src.transfer_function = Some(Box::new(TransferFunctionGamma::new(gamma)));
                    } else if gamma == 1.0 {
                        src.used_transfer_function = KHR_DF_TRANSFER_LINEAR;
                    } else {
                        // We allow any gamma, there is no reason why we could not
                        // allow such input
                        src.transfer_function = Some(Box::new(TransferFunctionGamma::new(gamma)));
                    }
                } else if is_png {
                    // If 8-bit, treat as sRGB, otherwise treat as linear.
                    let bits = channel_bit_length()?;
                    src.used_transfer_function = if bits == 8 {
                        KHR_DF_TRANSFER_SRGB
                    } else {
                        KHR_DF_TRANSFER_LINEAR
                    };
                    self.warning(&format!(
                        "No transfer function can be determined from {bits}-bit PNG input file \"{}\", defaulting to {}. Use --{K_ASSIGN_TF} to override.",
                        input.filename(),
                        to_string_transfer(src.used_transfer_function)
                    ));
                }
            }
        }
        if src.transfer_function.is_none() {
            src.transfer_function = match src.used_transfer_function {
                KHR_DF_TRANSFER_LINEAR => Some(Box::new(TransferFunctionLinear)),
                KHR_DF_TRANSFER_SRGB => Some(Box::new(TransferFunctionSRGB)),
                KHR_DF_TRANSFER_ITU => Some(Box::new(TransferFunctionITU::default())),
                KHR_DF_TRANSFER_PQ_EOTF => Some(Box::new(TransferFunctionBT2100PqEotf)),
                // Lack of will be handled if a color transformation attempted.
                _ => None,
            };
        }
        Ok(())
    }

    /// `determineTargetColorSpace`.
    fn determine_target_color_space(
        &self,
        input: &dyn ImageInput,
        target: &mut ImageSpec,
        csi: &mut ColorSpaceInfo,
    ) -> CmdResult {
        self.determine_source_color_space(input, &mut csi.src)?;
        let o = &self.options;
        if let Some(cp) = o.convert_primaries {
            target.format_mut().set_primaries(cp);
            // Okay to set this even if no conversion needed.
            csi.dst.color_primaries = Some(create_color_primaries(cp));
        } else {
            target.format_mut().set_primaries(csi.src.used_primaries);
        }
        // target transfer is set from the --format value or --assignTF or --convertTF.
        if let Some(tf) = o.assign_tf {
            target.format_mut().set_transfer(tf);
        }
        if let Some(tf) = o.convert_tf {
            target.format_mut().set_transfer(tf);
        }
        csi.dst.transfer_function = match target.format().transfer {
            KHR_DF_TRANSFER_LINEAR => Some(Box::new(TransferFunctionLinear)),
            KHR_DF_TRANSFER_SRGB => Some(Box::new(TransferFunctionSRGB)),
            _ => None,
        };
        Ok(())
    }

    /// `determineSourceOrigin`.
    fn determine_source_origin(&self, input: &dyn ImageInput) -> Origin {
        self.options
            .assign_texcoord_origin
            .unwrap_or(*input.spec().origin())
    }

    /// `determineTargetOrigin`.
    fn determine_target_origin(
        &self,
        input: &dyn ImageInput,
        target: &mut ImageSpec,
    ) -> CmdResult<Origin> {
        let used = self.determine_source_origin(input);
        target.set_origin(used);
        if let Some(co) = self.options.convert_texcoord_origin {
            if used.unspecified() {
                return Err(self.fatal(ReturnCode::InvalidFile, format!(
                    "Cannot convert texcoord origin as no information about the origin is available in the input file \"{}\". Use --{K_ASSIGN_TEXCOORD_ORIGIN} to specify one.",
                    input.filename()
                )));
            } else if co != used {
                target.set_origin(co);
            }
        }
        Ok(used)
    }

    /// `checkSpecsMatch`.
    fn check_specs_match(&self, current: &dyn ImageInput, first_spec: &ImageSpec) -> CmdResult {
        let o = &self.options;
        let ff = first_spec.format();
        let cf = current.spec().format();
        let name = current.filename();

        if cf.transfer != ff.transfer {
            let (c, f) = (
                to_string_transfer(cf.transfer),
                to_string_transfer(ff.transfer),
            );
            if o.assign_tf.is_some() {
                self.warning(&format!("Input image \"{name}\" has different transfer function ({c}) than the first image ({f}) but will be treated identically as specified by the --assign-tf option."));
            } else if o.convert_tf.is_some() {
                self.warning(&format!("Input image \"{name}\" has different transfer function ({c}) than the first image ({f}) and thus will go through different transfer function conversion to the target transfer function specified by the --convert-tf option."));
            } else {
                return Err(self.fatal(ReturnCode::InvalidFile, format!("Input image \"{name}\" has different transfer function ({c}) than the first image ({f}). Use --assign-tf or --convert-tf to specify handling and stop this error.")));
            }
        }

        if cf.oe_gamma != ff.oe_gamma {
            let g = |v: f32| {
                if v != -1.0 {
                    std_to_string(v)
                } else {
                    "no gamma".to_string()
                }
            };
            let (c, f) = (g(cf.oe_gamma), g(ff.oe_gamma));
            if o.assign_tf.is_some() {
                self.warning(&format!("Input image \"{name}\" has different gamma ({c}) than the first image ({f}) but will be treated identically as specified by the --assign-tf option."));
            } else if o.convert_tf.is_some() {
                self.warning(&format!("Input image \"{name}\" has different gamma ({c}) than the first image ({f}) and thus will go through different transfer function conversion to the target transfer function specified by the --convert-tf option."));
            } else {
                return Err(self.fatal(ReturnCode::InvalidFile, format!("Input image \"{name}\" has different gamma ({c}) than the first image ({f}). Use --assign-tf or --convert-tf to specify handling and stop this error.")));
            }
        }

        if cf.primaries != ff.primaries {
            let (c, f) = (
                to_string_primaries(cf.primaries),
                to_string_primaries(ff.primaries),
            );
            if o.assign_primaries.is_some() {
                self.warning(&format!("Input image \"{name}\" has different primaries ({c}) than the first image ({f}) but will be treated identically as specified by the --assign-primaries option."));
            } else if o.convert_primaries.is_some() {
                self.warning(&format!("Input image \"{name}\" has different primaries ({c}) than the first image ({f}) and thus will go through different primaries conversion to the target primaries specified by the --convert-primaries option."));
            } else {
                return Err(self.fatal(ReturnCode::InvalidFile, format!("Input image \"{name}\" has different primaries ({c}) than the first image ({f}). Use --assign-primaries or --convert-primaries to specify handling and stop this error.")));
            }
        }

        if cf.channel_count() != ff.channel_count() {
            self.warning(&format!(
                "Input image \"{name}\" has a different component count than the first image."
            ));
        }

        if current.spec().origin() != first_spec.origin() {
            let (c, f) = (
                origin_to_string(current.spec().origin()),
                origin_to_string(first_spec.origin()),
            );
            if o.assign_texcoord_origin.is_some() {
                self.warning(&format!("Input image \"{name}\" has different texcoord origin ({c}) than the first image ({f}) but will be treated identically as specified by the --assign-texcoord-origin option."));
            } else if o.convert_texcoord_origin.is_some() {
                self.warning(&format!("Input image \"{name}\" has different texcoord origin ({c}) than the first image ({f}) and thus will go through different origin conversion to the target origin specified by the --convert-texcoord-origin option."));
            } else {
                return Err(self.fatal(ReturnCode::InvalidFile, format!("Input image \"{name}\" has different texcoord origin ({c}) than the first image ({f}). Use --assign-texcoord-origin or --convert-texcoord-origin to specify handling and stop this error.")));
            }
        }
        Ok(())
    }
}

/// Target data kinds of `CommandCreate::convert`.
enum Conv {
    Unorm(u32, u32),
    UnormSBits(u32, u32, u32),
    UnormPacked([u32; 4]),
    SFloat(u32, u32),
    Uint(u32, u32),
    Sint(u32, u32),
    UintPacked([u32; 4]),
    SintPacked([u32; 4]),
    B10G11R11,
    E5B9G9R9,
}

/// Required input data type of `convert`'s `requireUNORM` / `requireSFloat` / `requireUINT`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Req {
    Unorm,
    SFloat,
    Uint,
}

impl CommandCreate {
    /// `CommandCreate::convert`. `swizzle_done` suppresses the BGR(A)-style
    /// swizzles that `convert*` would otherwise apply to `image` in place.
    fn convert(
        &self,
        image: &mut dyn Image,
        vk_format: VkFormat,
        input: &dyn ImageInput,
        swizzle_done: bool,
    ) -> CmdResult<Vec<u8>> {
        use VkFormat as F;
        let input_bit_depth = input.spec().format().largest_channel_bit_length().max(8);
        let fname = input.filename().to_string();
        let ftype = input.format_type();

        let require = |req: Req, bit_depth: u32| -> CmdResult {
            let accepted = match req {
                Req::Unorm => !matches!(
                    ftype,
                    ImageInputFormatType::ExrUint | ImageInputFormatType::ExrFloat
                ),
                Req::SFloat => ftype == ImageInputFormatType::ExrFloat,
                Req::Uint => ftype == ImageInputFormatType::ExrUint,
            };
            if !accepted {
                let name = match req {
                    Req::Unorm => "UNORM",
                    Req::SFloat => "SFLOAT",
                    Req::Uint => "UINT",
                };
                return Err(self.fatal(ReturnCode::InvalidFile, format!(
                    "{fname}: Input file data type \"{}\" does not match the expected input data type of {bit_depth} bit \"{name}\" for {}.",
                    format_type_to_string(ftype),
                    to_string_vk_format(vk_format)
                )));
            }
            if input_bit_depth < bit_depth {
                return Err(self.fatal(ReturnCode::InvalidFile, format!(
                    "{fname}: Not enough precision to convert {input_bit_depth} bit input to {bit_depth} bit output for {}.",
                    to_string_vk_format(vk_format)
                )));
            }
            if input_bit_depth > bit_ceil(bit_depth) {
                self.warning(&format!(
                    "{fname}: Possible loss of precision with converting {input_bit_depth} bit input to {bit_depth} bit output for {}.",
                    to_string_vk_format(vk_format)
                ));
            }
            Ok(())
        };
        let sw = |s: &'static str| if swizzle_done { "" } else { s };
        let unsupported = || {
            Err(self.fatal(
                ReturnCode::InvalidArguments,
                format!(
                    "Unsupported format for non-raw create: {}.",
                    to_string_vk_format(self.options.vk_format)
                ),
            ))
        };

        let (req, depth, conv, swizzle): (Req, u32, Conv, &str) = match vk_format {
            F::R8_UNORM | F::R8_SRGB => (Req::Unorm, 8, Conv::Unorm(1, 8), ""),
            F::R8G8_UNORM | F::R8G8_SRGB => (Req::Unorm, 8, Conv::Unorm(2, 8), ""),
            F::R8G8B8_UNORM | F::R8G8B8_SRGB => (Req::Unorm, 8, Conv::Unorm(3, 8), ""),
            F::B8G8R8_UNORM | F::B8G8R8_SRGB => (Req::Unorm, 8, Conv::Unorm(3, 8), sw("bgr1")),
            F::R8G8B8A8_UNORM
            | F::R8G8B8A8_SRGB
            | F::A8B8G8R8_UNORM_PACK32
            | F::A8B8G8R8_SRGB_PACK32 => (Req::Unorm, 8, Conv::Unorm(4, 8), ""),
            F::B8G8R8A8_UNORM | F::B8G8R8A8_SRGB => (Req::Unorm, 8, Conv::Unorm(4, 8), sw("bgra")),
            f if is_format_astc_ldr_2d(f) => {
                // ASTC texture data composition is performed via
                // R8G8B8A8_UNORM followed by the ASTC encoding
                require(Req::Unorm, 8)?;
                return Ok(Vec::new());
            }
            F::R4G4_UNORM_PACK8 => (Req::Unorm, 8, Conv::UnormPacked([4, 4, 0, 0]), ""),
            F::R5G6B5_UNORM_PACK16 => (Req::Unorm, 8, Conv::UnormPacked([5, 6, 5, 0]), ""),
            F::B5G6R5_UNORM_PACK16 => (Req::Unorm, 8, Conv::UnormPacked([5, 6, 5, 0]), sw("bgr1")),
            F::R4G4B4A4_UNORM_PACK16 => (Req::Unorm, 8, Conv::UnormPacked([4, 4, 4, 4]), ""),
            F::B4G4R4A4_UNORM_PACK16 => {
                (Req::Unorm, 8, Conv::UnormPacked([4, 4, 4, 4]), sw("bgra"))
            }
            F::R5G5B5A1_UNORM_PACK16 => (Req::Unorm, 8, Conv::UnormPacked([5, 5, 5, 1]), ""),
            F::B5G5R5A1_UNORM_PACK16 => {
                (Req::Unorm, 8, Conv::UnormPacked([5, 5, 5, 1]), sw("bgra"))
            }
            F::A1R5G5B5_UNORM_PACK16 => {
                (Req::Unorm, 8, Conv::UnormPacked([1, 5, 5, 5]), sw("argb"))
            }
            F::A1B5G5R5_UNORM_PACK16_KHR => {
                (Req::Unorm, 8, Conv::UnormPacked([1, 5, 5, 5]), sw("abgr"))
            }
            F::A4R4G4B4_UNORM_PACK16 => {
                (Req::Unorm, 8, Conv::UnormPacked([4, 4, 4, 4]), sw("argb"))
            }
            F::A4B4G4R4_UNORM_PACK16 => {
                (Req::Unorm, 8, Conv::UnormPacked([4, 4, 4, 4]), sw("abgr"))
            }
            F::R10X6_UNORM_PACK16 => (Req::Unorm, 10, Conv::UnormSBits(1, 16, 10), ""),
            F::R10X6G10X6_UNORM_2PACK16 => (Req::Unorm, 10, Conv::UnormSBits(2, 16, 10), ""),
            F::R10X6G10X6B10X6A10X6_UNORM_4PACK16 => {
                (Req::Unorm, 10, Conv::UnormSBits(4, 16, 10), "")
            }
            F::R12X4_UNORM_PACK16 => (Req::Unorm, 12, Conv::UnormSBits(1, 16, 12), ""),
            F::R12X4G12X4_UNORM_2PACK16 => (Req::Unorm, 12, Conv::UnormSBits(2, 16, 12), ""),
            F::R12X4G12X4B12X4A12X4_UNORM_4PACK16 => {
                (Req::Unorm, 12, Conv::UnormSBits(4, 16, 12), "")
            }
            F::R16_UNORM => (Req::Unorm, 16, Conv::Unorm(1, 16), ""),
            F::R16G16_UNORM => (Req::Unorm, 16, Conv::Unorm(2, 16), ""),
            F::R16G16B16_UNORM => (Req::Unorm, 16, Conv::Unorm(3, 16), ""),
            F::R16G16B16A16_UNORM => (Req::Unorm, 16, Conv::Unorm(4, 16), ""),
            F::A2R10G10B10_UNORM_PACK32 => (
                Req::Unorm,
                10,
                Conv::UnormPacked([2, 10, 10, 10]),
                sw("argb"),
            ),
            F::A2B10G10R10_UNORM_PACK32 => (
                Req::Unorm,
                10,
                Conv::UnormPacked([2, 10, 10, 10]),
                sw("abgr"),
            ),
            F::G8B8G8R8_422_UNORM
            | F::B8G8R8G8_422_UNORM
            | F::G10X6B10X6G10X6R10X6_422_UNORM_4PACK16
            | F::B10X6G10X6R10X6G10X6_422_UNORM_4PACK16
            | F::G12X4B12X4G12X4R12X4_422_UNORM_4PACK16
            | F::B12X4G12X4R12X4G12X4_422_UNORM_4PACK16
            | F::G16B16G16R16_422_UNORM
            | F::B16G16R16G16_422_UNORM => return unsupported(),

            F::R8_UINT => (Req::SFloat, 16, Conv::Uint(1, 8), ""),
            F::R8_SINT => (Req::SFloat, 16, Conv::Sint(1, 8), ""),
            F::R16_UINT => (Req::SFloat, 32, Conv::Uint(1, 16), ""),
            F::R16_SINT => (Req::SFloat, 32, Conv::Sint(1, 16), ""),
            F::R32_UINT => (Req::Uint, 32, Conv::Uint(1, 32), ""),
            F::R8G8_UINT => (Req::SFloat, 16, Conv::Uint(2, 8), ""),
            F::R8G8_SINT => (Req::SFloat, 16, Conv::Sint(2, 8), ""),
            F::R16G16_UINT => (Req::SFloat, 32, Conv::Uint(2, 16), ""),
            F::R16G16_SINT => (Req::SFloat, 32, Conv::Sint(2, 16), ""),
            F::R32G32_UINT => (Req::Uint, 32, Conv::Uint(2, 32), ""),
            F::R8G8B8_UINT => (Req::SFloat, 16, Conv::Uint(3, 8), ""),
            F::R8G8B8_SINT => (Req::SFloat, 16, Conv::Sint(3, 8), ""),
            F::B8G8R8_UINT => (Req::SFloat, 16, Conv::Uint(3, 8), sw("bgr1")),
            F::B8G8R8_SINT => (Req::SFloat, 16, Conv::Sint(3, 8), sw("bgr1")),
            F::R16G16B16_UINT => (Req::SFloat, 32, Conv::Uint(3, 16), ""),
            F::R16G16B16_SINT => (Req::SFloat, 32, Conv::Sint(3, 16), ""),
            F::R32G32B32_UINT => (Req::Uint, 32, Conv::Uint(3, 32), ""),
            F::R8G8B8A8_UINT | F::A8B8G8R8_UINT_PACK32 => (Req::SFloat, 16, Conv::Uint(4, 8), ""),
            F::R8G8B8A8_SINT | F::A8B8G8R8_SINT_PACK32 => (Req::SFloat, 16, Conv::Sint(4, 8), ""),
            F::B8G8R8A8_UINT => (Req::SFloat, 16, Conv::Uint(4, 8), sw("bgra")),
            F::B8G8R8A8_SINT => (Req::SFloat, 16, Conv::Sint(4, 8), sw("bgra")),
            F::R16G16B16A16_UINT => (Req::SFloat, 32, Conv::Uint(4, 16), ""),
            F::R16G16B16A16_SINT => (Req::SFloat, 32, Conv::Sint(4, 16), ""),
            F::R32G32B32A32_UINT => (Req::Uint, 32, Conv::Uint(4, 32), ""),
            F::A2R10G10B10_UINT_PACK32 => (
                Req::SFloat,
                16,
                Conv::UintPacked([2, 10, 10, 10]),
                sw("argb"),
            ),
            F::A2R10G10B10_SINT_PACK32 => (
                Req::SFloat,
                16,
                Conv::SintPacked([2, 10, 10, 10]),
                sw("argb"),
            ),
            F::A2B10G10R10_UINT_PACK32 => (
                Req::SFloat,
                16,
                Conv::UintPacked([2, 10, 10, 10]),
                sw("abgr"),
            ),
            F::A2B10G10R10_SINT_PACK32 => (
                Req::SFloat,
                16,
                Conv::SintPacked([2, 10, 10, 10]),
                sw("abgr"),
            ),
            F::R16_SFLOAT => (Req::SFloat, 16, Conv::SFloat(1, 16), ""),
            F::R16G16_SFLOAT => (Req::SFloat, 16, Conv::SFloat(2, 16), ""),
            F::R16G16B16_SFLOAT => (Req::SFloat, 16, Conv::SFloat(3, 16), ""),
            F::R16G16B16A16_SFLOAT => (Req::SFloat, 16, Conv::SFloat(4, 16), ""),
            F::R32_SFLOAT => (Req::SFloat, 32, Conv::SFloat(1, 32), ""),
            F::R32G32_SFLOAT => (Req::SFloat, 32, Conv::SFloat(2, 32), ""),
            F::R32G32B32_SFLOAT => (Req::SFloat, 32, Conv::SFloat(3, 32), ""),
            F::R32G32B32A32_SFLOAT => (Req::SFloat, 32, Conv::SFloat(4, 32), ""),
            F::B10G11R11_UFLOAT_PACK32 => (Req::SFloat, 16, Conv::B10G11R11, ""),
            F::E5B9G9R9_UFLOAT_PACK32 => (Req::SFloat, 16, Conv::E5B9G9R9, ""),
            F::D16_UNORM
            | F::X8_D24_UNORM_PACK32
            | F::D32_SFLOAT
            | F::S8_UINT
            | F::D16_UNORM_S8_UINT
            | F::D24_UNORM_S8_UINT
            | F::D32_SFLOAT_S8_UINT => return unsupported(),
            // Special case for alpha-only
            F::A8_UNORM_KHR => (Req::Unorm, 8, Conv::Unorm(1, 8), sw("a000")),
            _ => {
                return Err(self.fatal(
                    ReturnCode::InvalidArguments,
                    format!(
                        "Requested format conversion is not yet implemented for: {}.",
                        to_string_vk_format(self.options.vk_format)
                    ),
                ));
            }
        };
        require(req, depth)?;
        if !swizzle.is_empty() {
            image.swizzle(swizzle);
        }
        Ok(match conv {
            Conv::Unorm(n, bits) => image.get_unorm(n, bits, 0),
            Conv::UnormSBits(n, bits, s) => image.get_unorm(n, bits, s),
            Conv::UnormPacked(c) => image.get_unorm_packed(c[0], c[1], c[2], c[3]),
            Conv::SFloat(n, bits) => image.get_sfloat(n, bits),
            Conv::Uint(n, bits) => image.get_uint(n, bits),
            Conv::Sint(n, bits) => image.get_sint(n, bits),
            Conv::UintPacked(c) => image.get_uint_packed(c[0], c[1], c[2], c[3]),
            Conv::SintPacked(c) => image.get_sint_packed(c[0], c[1], c[2], c[3]),
            Conv::B10G11R11 => image.get_b10g11r11(),
            Conv::E5B9G9R9 => image.get_e5b9g9r9(),
        })
    }

    fn mipmap_filter(&self) -> String {
        self.options
            .mipmap_filter
            .clone()
            .unwrap_or_else(|| self.options.default_mipmap_filter.clone())
    }

    fn mipmap_filter_scale(&self) -> f32 {
        self.options
            .mipmap_filter_scale
            .unwrap_or(self.options.default_mipmap_filter_scale)
    }

    fn mipmap_wrap(&self) -> BoundaryOp {
        self.options
            .mipmap_wrap
            .unwrap_or(self.options.default_mipmap_wrap)
    }

    /// `scaleImage`.
    fn scale_image(&self, image: &dyn Image, width: u32, height: u32) -> CmdResult<Box<dyn Image>> {
        image
            .resample(
                width,
                height,
                &self.mipmap_filter(),
                self.mipmap_filter_scale(),
                self.mipmap_wrap(),
            )
            .map_err(|e| {
                self.fatal(
                    ReturnCode::RuntimeError,
                    format!("Image resampling failed: {}", e.what()),
                )
            })
    }

    /// `generateMipLevels`.
    #[allow(clippy::too_many_arguments)]
    fn generate_mip_levels(
        &self,
        texture: &mut Texture2,
        image: &mut dyn Image,
        input: &dyn ImageInput,
        num_mip_levels: u32,
        layer_index: u32,
        face_index: u32,
        depth_slice_index: u32,
    ) -> CmdResult {
        let vk = texture.vk_format;
        if is_format_int(vk) {
            return Err(self.fatal(
                ReturnCode::NotSupported,
                format!(
                    "Mipmap generation for SINT or UINT format {} is not supported.",
                    to_string_vk_format(vk)
                ),
            ));
        }
        let base_width = image.get_width();
        let base_height = image.get_height();
        for mip_level in 1..num_mip_levels {
            let w = base_width.checked_shr(mip_level).unwrap_or(0).max(1);
            let h = base_height.checked_shr(mip_level).unwrap_or(0).max(1);
            let mut level_image = image
                .resample(
                    w,
                    h,
                    &self.mipmap_filter(),
                    self.mipmap_filter_scale(),
                    self.mipmap_wrap(),
                )
                .map_err(|e| {
                    self.fatal(
                        ReturnCode::RuntimeError,
                        format!("Mipmap generation failed: {}", e.what()),
                    )
                })?;
            if self.options.normalize {
                image.normalize();
            }
            let data = self.convert(level_image.as_mut(), self.options.vk_format, input, true)?;
            let _ = texture.set_image_from_memory(
                mip_level,
                layer_index,
                face_index + depth_slice_index,
                &data,
            );
        }
        Ok(())
    }
}

/// `mapRangeCompute`: (scale, offset).
fn map_range_compute(old_min: f32, old_max: f32, new_min: f32, new_max: f32) -> (f32, f32) {
    let old_range = old_max - old_min;
    let new_range = new_max - new_min;
    // If old range fully overlaps the new range
    if old_min >= new_min && old_max <= new_max {
        return (1.0, 0.0);
    }
    let offset_min = if old_min >= new_min {
        0.0
    } else {
        new_min - old_min
    };
    let offset_max = if old_max <= new_max {
        0.0
    } else {
        old_max - new_max
    };
    if !(offset_min != 0.0 && offset_max != 0.0)
        && (old_max + offset_min < new_max)
        && (old_min - offset_max < new_min)
    {
        (
            1.0,
            if offset_min != 0.0 {
                offset_min
            } else {
                offset_max
            },
        )
    } else {
        let scale = new_range / old_range;
        (scale, new_min - old_min * scale)
    }
}

impl CommandCreate {
    /// `executeCreate`.
    fn execute_create(&mut self) -> CmdResult {
        let report = self.report.clone();
        let warning_fn: imageio::WarningCallback = {
            let r = report.clone();
            Rc::new(move |w: &str| r.warning(w))
        };

        let mut texture: Option<Texture2> = None;
        self.target_channel_count = self.options.format_desc.channel_count();

        let mut target = ImageSpec::default();
        let mut csi = ColorSpaceInfo::default();
        let mut first_image = true;
        let mut first_image_spec = ImageSpec::default();
        let mut max_levels = 1u32;

        for (input_filepath, level_index, layer_index, face_index, depth_slice_index) in
            self.image_list()
        {
            if self.options.raw {
                if std::mem::replace(&mut first_image, false) {
                    target = ImageSpec::new(
                        self.options.width.unwrap_or(1),
                        self.options.height.unwrap_or(1),
                        self.options.depth.unwrap_or(1),
                        Origin::default(),
                        self.options.format_desc.clone(),
                    );
                    if self.options.cubemap && target.width() != target.height() {
                        return Err(self.fatal(ReturnCode::InvalidFile, format!(
                            "--cubemap specified but the input image \"{}\" with size {}x{} is not square.",
                            fmt_in_file(&input_filepath), target.width(), target.height()
                        )));
                    }
                    if let Some(tf) = self.options.assign_tf {
                        target.format_mut().set_transfer(tf);
                    }
                    if let Some(p) = self.options.assign_primaries {
                        target.format_mut().set_primaries(p);
                    }
                    if let Some(o) = self.options.assign_texcoord_origin {
                        target.set_origin(o);
                    }
                    texture = Some(self.create_texture(&target)?);
                }
                let raw_data = self.read_raw_file(&input_filepath)?;
                let Some(tex) = texture.as_mut() else { break };
                let expected = tex.get_image_size(level_index);
                if raw_data.len() != expected {
                    return Err(self.fatal(ReturnCode::InvalidFile, format!(
                        "Raw input file \"{}\" with {} bytes for level {} does not match the expected size of {} bytes.",
                        fmt_in_file(&input_filepath), raw_data.len(), level_index, expected
                    )));
                }
                // Faces and Depths are mutually exclusive, Addition is acceptable
                let _ = tex.set_image_from_memory(
                    level_index,
                    layer_index,
                    face_index + depth_slice_index,
                    &raw_data,
                );
                continue;
            }

            let mut input = imageio::open(&input_filepath, Some(warning_fn.clone()))
                .map_err(|e| image_err(&report, e))?;
            // Loading multiple subimage from the same input is not supported
            input
                .seek_subimage(0, 0)
                .map_err(|e| image_err(&report, e))?;

            let used_source_origin;
            if std::mem::replace(&mut first_image, false) {
                let spec = input.spec();
                let (target_width, target_height) = match self.options.image_scale {
                    Some(s) => (
                        (spec.width() as f32 * s) as u32,
                        (spec.height() as f32 * s) as u32,
                    ),
                    None => (
                        self.options.width.unwrap_or(spec.width()),
                        self.options.height.unwrap_or(spec.height()),
                    ),
                };
                target = ImageSpec::new(
                    target_width,
                    target_height,
                    self.options.depth.unwrap_or(1),
                    Origin::default(),
                    self.options.format_desc.clone(),
                );
                if self.options.cubemap && target.width() != target.height() {
                    return Err(self.fatal(ReturnCode::InvalidFile, format!(
                        "--cubemap specified but the input image \"{}\" with size {}x{} is not square.",
                        fmt_in_file(&input_filepath), target.width(), target.height()
                    )));
                }
                if self.options._1d && target.height() != 1 {
                    return Err(self.fatal(ReturnCode::InvalidFile, format!(
                        "For --1d textures the input image height must be 1, but for \"{}\" it was {}.",
                        fmt_in_file(&input_filepath), target.height()
                    )));
                }
                let max_dimension = target.width().max(target.height().max(self.base_depth));
                max_levels = log2(max_dimension) + 1;
                if self.options.levels.unwrap_or(1) > max_levels {
                    let base_expl = if self.options.width.is_some() || self.options.height.is_some()
                    {
                        "a requested base image size of".to_string()
                    } else if let Some(s) = self.options.image_scale {
                        format!(
                            "base input image \"{}\" sized {}x{} * scale {} being",
                            fmt_in_file(&input_filepath),
                            spec.width(),
                            spec.height(),
                            super::fmt::float_shortest(s)
                        )
                    } else {
                        format!(
                            "base input image \"{}\" sized",
                            fmt_in_file(&input_filepath)
                        )
                    };
                    return Err(self.usage(format!(
                        "Requested {} levels is too many. With {base_expl} {}x{} and depth {} the texture can only have {max_levels} levels at most.",
                        self.options.levels.unwrap_or(1), target.width(), target.height(), self.base_depth
                    )));
                }

                self.check_num_input_images()?;

                if self.astc.encode_astc {
                    self.select_astc_mode(spec.format().largest_channel_bit_length());
                }
                first_image_spec = spec.clone();

                self.determine_target_color_space(input.as_ref(), &mut target, &mut csi)?;
                used_source_origin = self.determine_target_origin(input.as_ref(), &mut target)?;
                texture = Some(self.create_texture(&target)?);
            } else {
                self.check_specs_match(input.as_ref(), &first_image_spec)?;
                self.determine_source_color_space(input.as_ref(), &mut csi.src)?;
                used_source_origin = self.determine_source_origin(input.as_ref());
            }

            let expected_w = first_image_spec
                .width()
                .checked_shr(level_index)
                .unwrap_or(0)
                .max(1);
            let expected_h = first_image_spec
                .height()
                .checked_shr(level_index)
                .unwrap_or(0)
                .max(1);
            let target_w = target.width().checked_shr(level_index).unwrap_or(0).max(1);
            let target_h = target.height().checked_shr(level_index).unwrap_or(0).max(1);

            let (in_w, in_h) = (input.spec().width(), input.spec().height());
            if in_w != expected_w || in_h != expected_h {
                return Err(self.fatal(ReturnCode::InvalidFile, format!(
                    "Input image \"{}\" with size {in_w}x{in_h} does not match expected size {expected_w}x{expected_h} for level {level_index}.",
                    fmt_in_file(&input_filepath)
                )));
            }
            let mut image = self.load_input_image(input.as_mut())?;

            // Need to do color conversion if either the transfer functions or primaries don't
            // match. Primaries conversion requires decode to linear then reencode thus
            // transferFunctions are always required.
            let tfmt = target.format().clone();
            if tfmt.transfer != csi.src.used_transfer_function
                || tfmt.primaries != csi.src.used_primaries
            {
                let Some(src_tf) = csi.src.transfer_function.as_deref() else {
                    let source = if self.options.assign_tf.is_some() {
                        format!("specified with --{K_ASSIGN_TF}")
                    } else {
                        format!("used by input file \"{}\"", fmt_in_file(&input_filepath))
                    };
                    let mut msg = format!(
                        "Colorspace conversion requires unsupported decode from {} {source}.",
                        to_string_transfer(csi.src.used_transfer_function)
                    );
                    if self.options.assign_tf.is_none() {
                        msg += &format!(
                            " Use an image processing tool to convert it or use --{K_ASSIGN_TF}, with or without --{K_CONVERT_TF}, to specify handling."
                        );
                    }
                    return Err(self.fatal(ReturnCode::NotSupported, msg));
                };
                let Some(dst_tf) = csi.dst.transfer_function.as_deref() else {
                    let source = format!(
                        "specified with --{}",
                        if self.options.convert_tf.is_some() {
                            K_CONVERT_TF
                        } else {
                            K_ASSIGN_TF
                        }
                    );
                    let mut msg = format!(
                        "Colorspace conversion requires unsupported encode to {} {source}.",
                        to_string_transfer(tfmt.transfer)
                    );
                    // Transfer functions derived from --format values are supported.
                    if tfmt.primaries != csi.src.used_primaries {
                        msg += &format!(
                            " Decode and encode with transfer function is required to convert primaries to {}.",
                            to_string_primaries(tfmt.primaries)
                        );
                    }
                    return Err(self.fatal(ReturnCode::NotSupported, msg));
                };
                if !self.options.no_warn_on_color_conversions
                    && tfmt.model == KHR_DF_MODEL_RGBSDA
                    && tfmt.transfer == KHR_DF_TRANSFER_LINEAR
                {
                    let bit_length = match tfmt.channel_bit_length() {
                        Ok(b) => b,
                        // This happens if channels have different bit length. Check just R.
                        Err(_) => tfmt
                            .channel_bit_length_of(KHR_DF_CHANNEL_RGBSDA_R)
                            .unwrap_or(0),
                    };
                    // Per Poynton, >= 14 bits is enough to handle all transitions
                    // visible to a human
                    if bit_length < 14
                        && (csi.src.used_transfer_function == KHR_DF_TRANSFER_SRGB
                            || csi.src.used_transfer_function == KHR_DF_TRANSFER_ITU)
                    {
                        self.warning(&format!(
                            "Input file \"{}\" is undergoing a visual lossy color conversion from {} to KHR_DF_TRANSFER_LINEAR. Specify an _SRGB format with --{K_FORMAT} to prevent this warning.",
                            fmt_in_file(&input_filepath),
                            to_string_transfer(csi.src.used_transfer_function)
                        ));
                    }
                }
                if tfmt.primaries != csi.src.used_primaries {
                    let src_p = csi
                        .src
                        .color_primaries
                        .unwrap_or_else(ColorPrimaries::bt709);
                    let dst_p = csi
                        .dst
                        .color_primaries
                        .unwrap_or_else(ColorPrimaries::bt709);
                    let primary_transform = src_p.transform_to(&dst_p);
                    if self.options.fail_on_color_conversions {
                        return Err(self.fatal(ReturnCode::InvalidFile, format!(
                            "Input file \"{}\" would need color conversion as input and output primaries are different. Use --assign-primaries and do not use --convert-primaries to avoid unwanted color conversions.",
                            fmt_in_file(&input_filepath)
                        )));
                    }
                    if self.options.warn_on_color_conversions {
                        self.warning(&format!(
                            "Input file \"{}\" is color converted as input and output primaries are different. Use --assign-primaries and do not use --convert-primaries to avoid unwanted color conversions.",
                            fmt_in_file(&input_filepath)
                        ));
                    }
                    // Transform transfer function with primary transform
                    image.transform_color_space(src_tf, dst_tf, Some(&primary_transform));
                } else {
                    if self.options.fail_on_color_conversions {
                        return Err(self.fatal(ReturnCode::InvalidFile, format!(
                            "Input file \"{}\" would need color conversion as input and output transfer functions are different. Use --assign-tf and do not use --convert-tf to avoid unwanted color conversions.",
                            fmt_in_file(&input_filepath)
                        )));
                    }
                    if self.options.warn_on_color_conversions {
                        self.warning(&format!(
                            "Input file \"{}\" is color converted as input and output transfer functions are different. Use --assign-tf and do not use --convert-tf to avoid unwanted color conversions.",
                            fmt_in_file(&input_filepath)
                        ));
                    }
                    // Transform transfer function without primary transform
                    image.transform_color_space(src_tf, dst_tf, None);
                }
            }
            // Must be set before operations like resampling. Needed even when
            // transformColorSpace is not called.
            image.set_primaries(tfmt.primaries);
            image.set_transfer_function(tfmt.transfer);

            if image.get_width() != target_w || image.get_height() != target_h {
                image = self.scale_image(image.as_ref(), target_w, target_h)?;
            }

            if *target.origin() != used_source_origin {
                if self.options.fail_on_origin_changes {
                    return Err(self.fatal(ReturnCode::InvalidFile, format!(
                        "Input file \"{}\" would need to be y-flipped as input and output origins are different. Use --{K_ASSIGN_TEXCOORD_ORIGIN} and do not use --{K_CONVERT_TEXCOORD_ORIGIN} to avoid unwanted origin conversions.",
                        fmt_in_file(&input_filepath)
                    )));
                }
                if self.options.warn_on_origin_changes {
                    self.warning(&format!(
                        "Input file \"{}\" is y-flipped as input and output origins are different. Use --{K_ASSIGN_TEXCOORD_ORIGIN} and do not use --{K_CONVERT_TEXCOORD_ORIGIN} to avoid unwanted origin conversions.",
                        fmt_in_file(&input_filepath)
                    ));
                }
                // Only difference allowed by CLI is y down or y up.
                image.yflip();
            }

            if self.options.normalize {
                if tfmt.transfer != KHR_DF_TRANSFER_UNSPECIFIED
                    && tfmt.transfer != KHR_DF_TRANSFER_LINEAR
                {
                    if let Some(tf) = self.options.convert_tf {
                        return Err(self.usage(format!(
                            "--{K_CONVERT_TF} value is {}. Normalize can only be used if the transfer function is linear or none.",
                            to_string_transfer(tf)
                        )));
                    } else if let Some(tf) = self.options.assign_tf {
                        return Err(self.usage(format!(
                            "--{K_ASSIGN_TF} value is {}. Normalize can only be used if the transfer function is linear or none.",
                            to_string_transfer(tf)
                        )));
                    }
                }
                image.normalize();
            }

            let Some(tex) = texture.as_mut() else { break };
            let bdb: Vec<u32> = tex.p_dfd.get(1..).unwrap_or_default().to_vec();
            let num_samples = dfdsamplecount(&bdb);
            let is_floating_point = (0..num_samples).any(|s| {
                dfdsval(&bdb, s, QUALIFIERS) & KHR_DF_SAMPLE_DATATYPE_FLOAT
                    == KHR_DF_SAMPLE_DATATYPE_FLOAT
            });

            if is_floating_point {
                if self.options.map_range_runtime {
                    let data = image.get_sfloat(3, 32);
                    let mut range_min = f32::MAX;
                    let mut range_max = f32::MIN_POSITIVE;
                    for c in data.as_chunks::<4>().0 {
                        let v = f32::from_le_bytes(*c);
                        // std::min / std::max
                        if v < range_min {
                            range_min = v;
                        }
                        if range_max < v {
                            range_max = v;
                        }
                    }
                    let (mut target_min, mut target_max) = (0.0f32, 0.0f32);
                    let hdr4x4 = self.basis.params.codec == KTX_BASIS_CODEC_UASTC_HDR_4X4;
                    for s in 0..num_samples {
                        let bitlength = dfdsval(&bdb, s, BITLENGTH) + 1;
                        if bitlength == 16 {
                            target_min = if hdr4x4 { 0.0 } else { -65504.0 };
                            target_max = 65504.0;
                        } else if bitlength == 32 {
                            target_min = if hdr4x4 { 0.0 } else { f32::MIN_POSITIVE };
                            target_max = f32::MAX;
                        }
                    }
                    let (s, o) = map_range_compute(range_min, range_max, target_min, target_max);
                    image.map_range(s, o);
                    self.offset = -o / s;
                    self.scale = 1.0 / s;
                } else {
                    let (s, o) = (
                        self.options.scale.unwrap_or(1.0),
                        self.options.offset.unwrap_or(0.0),
                    );
                    image.map_range(s, o);
                    self.offset = -o / s;
                    self.scale = 1.0 / s;
                }
            }

            if let Some(sw) = &self.options.swizzle_input {
                image.swizzle(sw);
            }

            if self.options.premultiply_alpha {
                if image.get_component_count() < 4 {
                    return Err(self.usage(
                        "PremultiplyAlpha can only be used if the input image has alpha channels."
                            .into(),
                    ));
                }
                image.premultiply_alpha();
            }

            let image_data = self.convert(
                image.as_mut(),
                self.options.vk_format,
                input.as_ref(),
                false,
            )?;
            let Some(tex) = texture.as_mut() else { break };
            let _ = tex.set_image_from_memory(
                level_index,
                layer_index,
                face_index + depth_slice_index,
                &image_data,
            );

            if self.options.mipmap_generate {
                let num_mip_levels = self.options.levels.unwrap_or(max_levels);
                self.generate_mip_levels(
                    tex,
                    image.as_mut(),
                    input.as_ref(),
                    num_mip_levels,
                    layer_index,
                    face_index,
                    depth_slice_index,
                )?;
            }
        }

        let Some(mut texture) = texture else {
            return Ok(());
        };

        // Add KTXwriter metadata
        let writer = format!("{} {}", report.command_name, version(self.generic.testrun));
        let mut value = writer.into_bytes();
        value.push(0);
        let _ = texture.kv_data_head.add_kv_pair(KTX_WRITER_KEY, &value);

        // Add KTXswizzle metadata
        if let Some(sw) = &self.options.swizzle {
            let mut value = sw.clone().into_bytes();
            value.push(0);
            let _ = texture.kv_data_head.add_kv_pair(KTX_SWIZZLE_KEY, &value);
        }

        // Add KTXmapRange metadata
        let codec = self.basis.params.codec;
        if (self.scale != 1.0 || self.offset != 0.0)
            && (self.options.vk_format == VkFormat::R16G16B16_SFLOAT
                || self.options.vk_format == VkFormat::R16G16B16A16_SFLOAT
                || codec == KTX_BASIS_CODEC_UASTC_HDR_4X4
                || codec == KTX_BASIS_CODEC_UASTC_HDR_6X6_INTERMEDIATE)
        {
            let _ = texture
                .kv_data_head
                .add_kv_pair(KTX_MAP_RANGE_KEY, &map_range_bytes(self.scale, self.offset));
        }

        // Encode and apply compression
        let mut metrics = MetricsCalculator::default();
        metrics.save_reference_images(&texture, &self.metrics);

        if matches!(self.basis.selected_codec, BasisCodec::UastcHdr6x6i) {
            // TODO: the UASTC HDR 6x6i encoder is not ported yet.
            return Err(self.fatal(ReturnCode::NotImplemented, format!(
                "Failed to encode KTX2 file with codec \"{}\". The UASTC HDR 6x6i encoder is not implemented.",
                self.basis.selected_codec as u32
            )));
        }
        if self.basis.selected_codec != BasisCodec::None {
            self.encode_basis(&mut texture)?;
        }
        if self.astc.encode_astc {
            self.encode_astc(&mut texture)?;
        }

        let output_path = std::path::Path::new(&self.in_out.output_filepath);
        if let Some(parent) = output_path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut output = OutputStream::new(&self.in_out.output_filepath, &report)?;

        // If output is stdout, we write metrics to stderr (this is done to avoid polluting the
        // KTX file binary in stdout)
        metrics.decode_and_calculate_metrics(
            &mut texture,
            &self.metrics,
            &report,
            output.is_stdout(),
        )?;

        if let Some(level) = self.deflate.zstd
            && let Err(e) = texture.deflate_zstd(level)
        {
            return Err(self.fatal(
                ReturnCode::RuntimeError,
                format!("Zstd deflation failed. KTX Error: {}", ktx_error_string(e)),
            ));
        }
        if let Some(level) = self.deflate.zlib
            && let Err(e) = texture.deflate_zlib(level)
        {
            return Err(self.fatal(
                ReturnCode::RuntimeError,
                format!("ZLIB deflation failed. KTX Error: {}", ktx_error_string(e)),
            ));
        }

        // Add KTXwriterScParams metadata if ASTC encoding, BasisU encoding, or other
        // supercompression was used
        let sc_params = format!(
            "{}{}{}{}",
            self.astc.astc_options,
            self.basis.codec_options,
            self.common.common_options,
            self.deflate.compress_options
        );
        if let Some(rest) = sc_params.strip_prefix(' ') {
            // Options always contain a leading space; size() bytes from c_str() + 1
            // include the terminating NUL.
            let mut value = rest.as_bytes().to_vec();
            value.push(0);
            let _ = texture
                .kv_data_head
                .add_kv_pair(KTX_WRITER_SCPARAMS_KEY, &value);
        }

        // Save output file
        output.write_ktx2(&mut texture, &report)
    }

    /// `encodeASTC`.
    fn encode_basis(&self, texture: &mut Texture2) -> CmdResult {
        if let Err(e) = texture.compress_basis_ex(&self.basis.params) {
            return Err(self.fatal(
                ReturnCode::RuntimeError,
                format!(
                    "Failed to encode KTX2 file with codec \"{}\". KTX Error: {}",
                    self.basis.selected_codec as u32,
                    ktx_error_string(e)
                ),
            ));
        }
        Ok(())
    }

    fn encode_astc(&self, texture: &mut Texture2) -> CmdResult {
        if let Err(e) = texture.compress_astc_ex(&self.astc.params) {
            return Err(self.fatal(
                ReturnCode::RuntimeError,
                format!(
                    "Failed to encode KTX2 file with codec ASTC. KTX Error: {}",
                    ktx_error_string(e)
                ),
            ));
        }
        Ok(())
    }

    /// `selectASTCMode`.
    fn select_astc_mode(&mut self, bit_length: u32) {
        let p = &mut self.astc.params;
        if p.mode == KTX_PACK_ASTC_ENCODER_MODE_DEFAULT {
            // If no astc mode option is specified and if input is <= 8bit
            // default to LDR otherwise default to HDR
            p.mode = if bit_length <= 8 {
                KTX_PACK_ASTC_ENCODER_MODE_LDR
            } else {
                KTX_PACK_ASTC_ENCODER_MODE_HDR
            };
        } else if bit_length > 8 && p.mode == KTX_PACK_ASTC_ENCODER_MODE_LDR {
            // Input is > 8-bit and user wants LDR, issue quality loss warning.
            self.report.warning("Input file is 16-bit but ASTC LDR option is specified. Expect quality loss in the output.");
        } else if bit_length < 16 && p.mode == KTX_PACK_ASTC_ENCODER_MODE_HDR {
            // Input is < 16-bit and user wants HDR, issue warning.
            self.report
                .warning("Input file is not 16-bit but HDR option is specified.");
        }
        // ASTC Encoding is performed by first creating a RGBA8 texture then encode it afterward
        self.options.vk_format = if is_format_srgb(self.options.vk_format) {
            VkFormat::R8G8B8A8_SRGB
        } else {
            VkFormat::R8G8B8A8_UNORM
        };
    }
}
