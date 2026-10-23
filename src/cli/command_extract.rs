//! Port of tools/ktx/command_extract.cpp (`ktx extract`).
//!
//! Raw and PNG (lodepng) output are complete. EXR (tinyexr), ASTC decoding
//! (astcenc) and 4:2:2 unpacking are not ported yet: those outputs fail with
//! `NOT_IMPLEMENTED` (see ROADMAP).

use super::command::*;
use super::format_descriptor::{FormatDescriptor, convert_unorm, extract_bits};
use super::formats::{to_string_model, to_string_vk_format};
use super::fragment_uri::{FragmentURI, SelectorRange, StoiError, parse_fragment_uri, stoi};
use super::options::{OptionSpec, Options, ParseResult};
use super::transcode_utils::{OptionsTranscodeTarget, transcode};
use super::utility::{ceil_div, io_error_message};
use crate::defs::KTX_MAP_RANGE_KEY;
use crate::dfd::get_primaries;
use crate::dfd::khr_df::*;
use crate::lodepng;
use crate::texture::KtxTexture;
use crate::{
    KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, Texture2, TranscodeFmt, VkFormat, ktx_error_string,
};

/// `CommandExtract::OptionsExtract`.
#[derive(Debug, Clone, Default)]
struct OptionsExtract {
    output_path: String,
    fragment_uri: FragmentURI,
    depth: SelectorRange,
    level_flag_used: bool,
    layer_flag_used: bool,
    face_flag_used: bool,
    depth_flag_used: bool,
    uri_flag_used: bool,
    global_all: bool,
    raw: bool,
}

impl OptionsExtract {
    const K_OUTPUT: &'static str = "output";
    const K_STDOUT: &'static str = "stdout";
    const K_TRANSCODE: &'static str = "transcode";
    const K_URI: &'static str = "uri";
    const K_LEVEL: &'static str = "level";
    const K_LAYER: &'static str = "layer";
    const K_FACE: &'static str = "face";
    const K_DEPTH: &'static str = "depth";
    const K_ALL: &'static str = "all";
    const K_RAW: &'static str = "raw";
}

impl OptionGroup for OptionsExtract {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::value(
            Self::K_OUTPUT,
            "Output filepath for single, output directory for multiple image export.",
            "<filepath>",
        ));
        opts.add(OptionSpec::flag(
            Self::K_STDOUT,
            "Use stdout as the output file. (Using a single dash '-' as the output file has the same effect)",
        ));
        opts.add(OptionSpec::value(
            Self::K_TRANSCODE,
            "Transcode the texture to the target format before executing the extract steps. Requires the input file to be transcodable. Block compressed transcode targets can only be saved in raw format. Case-insensitive.\nPossible options are: etc-rgb | etc-rgba | eac-r11 | eac-rg11 | bc1 | bc3 | bc4 | bc5 | bc7 | astc | r8 | rg8 | rgb8 | rgba8.\netc-rgb is ETC1; etc-rgba, eac-r11 and eac-rg11 are ETC2.",
            "<target>",
        ));
        opts.add(OptionSpec::value(Self::K_URI, "KTX Fragment URI.", "<uri>"));
        opts.add(OptionSpec::value(
            Self::K_LEVEL,
            "Level to extract. When 'all' is used every level is exported. Defaults to 0.",
            "[0-9]+ | all",
        ));
        opts.add(OptionSpec::value(
            Self::K_LAYER,
            "Layer to extract. When 'all' is used every layer is exported. Defaults to 0.",
            "[0-9]+ | all",
        ));
        opts.add(OptionSpec::value(
            Self::K_FACE,
            "Face to extract. When 'all' is used every face is exported. Defaults to 0.",
            "[0-5] | all",
        ));
        opts.add(OptionSpec::value(
            Self::K_DEPTH,
            "Depth slice to extract. When 'all' is used every depth is exported. Defaults to 0.",
            "[0-9]+ | all",
        ));
        opts.add(OptionSpec::flag(
            Self::K_ALL,
            "Extract every image slice from the texture.",
        ));
        opts.add(OptionSpec::flag(
            Self::K_RAW,
            "Extract the raw image data without any conversion.",
        ));
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        match args.get(Self::K_OUTPUT) {
            Some(o) if args.count(Self::K_OUTPUT) > 0 => self.output_path = o.to_string(),
            _ => return Err(report.fatal_usage("Missing output file or directory path.")),
        }

        let parse_selector = |name: &str, found: &mut bool| -> CmdResult<Option<SelectorRange>> {
            if args.count(name) == 0 {
                return Ok(None);
            }
            let s = args.get(name).unwrap_or_default().to_lowercase();
            *found = true;
            if s == Self::K_ALL {
                return Ok(Some(SelectorRange::all()));
            }
            match stoi(&s) {
                Ok(v) => Ok(Some(SelectorRange::index(v as u32))),
                Err(StoiError::InvalidArgument) => Err(report.fatal_usage(&format!(
                    "Invalid {name} value \"{s}\". The value must be a either a number or \"all\"."
                ))),
                Err(StoiError::OutOfRange) => {
                    Err(report.fatal_usage(&format!("Out of range {name} value \"{s}\": stoi.")))
                }
            }
        };

        let mut level = parse_selector(Self::K_LEVEL, &mut self.level_flag_used)?;
        let mut layer = parse_selector(Self::K_LAYER, &mut self.layer_flag_used)?;
        let mut face = parse_selector(Self::K_FACE, &mut self.face_flag_used)?;
        let mut depth = parse_selector(Self::K_DEPTH, &mut self.depth_flag_used)?;
        self.raw = args.as_bool(Self::K_RAW);
        self.global_all = args.as_bool(Self::K_ALL);

        if self.global_all {
            for (sel, name) in [
                (&level, Self::K_LEVEL),
                (&layer, Self::K_LAYER),
                (&face, Self::K_FACE),
                (&depth, Self::K_DEPTH),
            ] {
                if sel.is_some() {
                    return Err(report.fatal_usage(&format!(
                        "Conflicting options: --{name} cannot be used with --all."
                    )));
                }
            }
            level = Some(SelectorRange::all());
            layer = Some(SelectorRange::all());
            face = Some(SelectorRange::all());
            depth = Some(SelectorRange::all());
        }

        let to_stdout = self.output_path == "-";
        if self.global_all && to_stdout {
            return Err(
                report.fatal_usage("stdout cannot be used with multi-output '--all' extract.")
            );
        }
        for (sel, name) in [
            (&level, Self::K_LEVEL),
            (&layer, Self::K_LAYER),
            (&face, Self::K_FACE),
            (&depth, Self::K_DEPTH),
        ] {
            if sel.as_ref().is_some_and(SelectorRange::is_all) && to_stdout {
                return Err(report.fatal_usage(&format!(
                    "stdout cannot be used with multi-output '--{name} all' extract."
                )));
            }
        }

        if args.count(Self::K_URI) > 0 {
            self.uri_flag_used = true;

            if self.global_all {
                return Err(
                    report.fatal_usage("Conflicting options: --all cannot be used with --uri.")
                );
            }
            for (used, name) in [
                (self.level_flag_used, Self::K_LEVEL),
                (self.layer_flag_used, Self::K_LAYER),
                (self.face_flag_used, Self::K_FACE),
            ] {
                if used {
                    return Err(report.fatal_usage(&format!(
                        "Conflicting options: --{name} cannot be used with --uri."
                    )));
                }
            }

            self.fragment_uri = match parse_fragment_uri(args.get(Self::K_URI).unwrap_or_default())
            {
                Ok(u) => u,
                Err(what) => {
                    return Err(
                        report.fatal_usage(&format!("Failed to parse Fragment URI: {what}"))
                    );
                }
            };

            let u = &self.fragment_uri;
            let is_multi_output_fragment_uri = (!u.mip.is_undefined() && u.mip.is_multi())
                || (!u.stratal.is_undefined() && u.stratal.is_multi())
                || (!u.facial.is_undefined() && u.facial.is_multi());
            if is_multi_output_fragment_uri && to_stdout {
                return Err(
                    report.fatal_usage("stdout cannot be used with multi-output '--uri' extract.")
                );
            }
        } else {
            // Merge every other selection method into the fragmentURI
            self.fragment_uri.mip = level.unwrap_or_else(|| SelectorRange::index(0));
            self.fragment_uri.stratal = layer.unwrap_or_else(|| SelectorRange::index(0));
            self.fragment_uri.facial = face.unwrap_or_else(|| SelectorRange::index(0));
        }

        self.depth = depth.unwrap_or_else(|| SelectorRange::index(0));
        Ok(())
    }
}

/// `initOptions` positional override (runs after every group's `init`).
#[derive(Debug, Clone, Default)]
struct ExtractPositional;

impl OptionGroup for ExtractPositional {
    fn init(&mut self, opts: &mut Options) {
        opts.parse_positional(&["input-file", OptionsExtract::K_OUTPUT]);
        opts.positional_help("<input-file> <output>");
    }

    fn process(&mut self, _opts: &Options, _args: &ParseResult, _report: &Reporter) -> CmdResult {
        Ok(())
    }
}

/// TinyEXR pixel types.
const TINYEXR_PIXELTYPE_UINT: i32 = 0;
const TINYEXR_PIXELTYPE_HALF: i32 = 1;
const TINYEXR_PIXELTYPE_FLOAT: i32 = 2;

/// How `saveImageFile` stores a format.
#[derive(Debug, Clone, PartialEq)]
enum ImageSave {
    Png,
    Astc,
    Unpack422,
    /// EXR with per-channel pixel types (one type means every channel).
    Exr(Vec<i32>),
}

/// The `saveImageFile` switch.
fn image_save_kind(vk: VkFormat) -> Option<ImageSave> {
    use ImageSave::*;
    let exr = |t: i32| Some(Exr(vec![t]));
    match vk {
        VkFormat::A8_UNORM_KHR
        | VkFormat::R8_UNORM
        | VkFormat::R8_SRGB
        | VkFormat::R8G8_UNORM
        | VkFormat::R8G8_SRGB
        | VkFormat::R8G8B8_UNORM
        | VkFormat::R8G8B8_SRGB
        | VkFormat::B8G8R8_UNORM
        | VkFormat::B8G8R8_SRGB
        | VkFormat::A8B8G8R8_UNORM_PACK32
        | VkFormat::A8B8G8R8_SRGB_PACK32
        | VkFormat::R8G8B8A8_UNORM
        | VkFormat::R8G8B8A8_SRGB
        | VkFormat::B8G8R8A8_UNORM
        | VkFormat::B8G8R8A8_SRGB => Some(Png),

        VkFormat::R4G4_UNORM_PACK8
        | VkFormat::R5G6B5_UNORM_PACK16
        | VkFormat::B5G6R5_UNORM_PACK16
        | VkFormat::R4G4B4A4_UNORM_PACK16
        | VkFormat::B4G4R4A4_UNORM_PACK16
        | VkFormat::R5G5B5A1_UNORM_PACK16
        | VkFormat::B5G5R5A1_UNORM_PACK16
        | VkFormat::A1R5G5B5_UNORM_PACK16
        | VkFormat::A1B5G5R5_UNORM_PACK16_KHR
        | VkFormat::A4R4G4B4_UNORM_PACK16
        | VkFormat::A4B4G4R4_UNORM_PACK16
        | VkFormat::R10X6_UNORM_PACK16
        | VkFormat::R10X6G10X6_UNORM_2PACK16
        | VkFormat::R10X6G10X6B10X6A10X6_UNORM_4PACK16
        | VkFormat::R12X4_UNORM_PACK16
        | VkFormat::R12X4G12X4_UNORM_2PACK16
        | VkFormat::R12X4G12X4B12X4A12X4_UNORM_4PACK16
        | VkFormat::R16_UNORM
        | VkFormat::R16G16_UNORM
        | VkFormat::R16G16B16_UNORM
        | VkFormat::R16G16B16A16_UNORM
        | VkFormat::A2R10G10B10_UNORM_PACK32
        | VkFormat::A2B10G10R10_UNORM_PACK32
        | VkFormat::D16_UNORM => Some(Png),

        VkFormat::G8B8G8R8_422_UNORM
        | VkFormat::B8G8R8G8_422_UNORM
        | VkFormat::G10X6B10X6G10X6R10X6_422_UNORM_4PACK16
        | VkFormat::B10X6G10X6R10X6G10X6_422_UNORM_4PACK16
        | VkFormat::G12X4B12X4G12X4R12X4_422_UNORM_4PACK16
        | VkFormat::B12X4G12X4R12X4G12X4_422_UNORM_4PACK16
        | VkFormat::G16B16G16R16_422_UNORM
        | VkFormat::B16G16R16G16_422_UNORM => Some(Unpack422),

        VkFormat::R8_UINT
        | VkFormat::R8_SINT
        | VkFormat::R8G8_UINT
        | VkFormat::R8G8_SINT
        | VkFormat::R8G8B8_UINT
        | VkFormat::R8G8B8_SINT
        | VkFormat::B8G8R8_UINT
        | VkFormat::B8G8R8_SINT
        | VkFormat::R8G8B8A8_UINT
        | VkFormat::R8G8B8A8_SINT
        | VkFormat::B8G8R8A8_UINT
        | VkFormat::B8G8R8A8_SINT
        | VkFormat::A2R10G10B10_UINT_PACK32
        | VkFormat::A2R10G10B10_SINT_PACK32
        | VkFormat::A2B10G10R10_UINT_PACK32
        | VkFormat::A2B10G10R10_SINT_PACK32
        | VkFormat::R16_SFLOAT
        | VkFormat::R16G16_SFLOAT
        | VkFormat::R16G16B16_SFLOAT
        | VkFormat::R16G16B16A16_SFLOAT
        | VkFormat::B10G11R11_UFLOAT_PACK32
        | VkFormat::E5B9G9R9_UFLOAT_PACK32
        | VkFormat::S8_UINT => exr(TINYEXR_PIXELTYPE_HALF),

        VkFormat::R16_UINT
        | VkFormat::R16_SINT
        | VkFormat::R16G16_UINT
        | VkFormat::R16G16_SINT
        | VkFormat::R16G16B16_UINT
        | VkFormat::R16G16B16_SINT
        | VkFormat::R16G16B16A16_UINT
        | VkFormat::R16G16B16A16_SINT
        | VkFormat::R32_SFLOAT
        | VkFormat::R32G32_SFLOAT
        | VkFormat::R32G32B32_SFLOAT
        | VkFormat::R32G32B32A32_SFLOAT
        | VkFormat::X8_D24_UNORM_PACK32
        | VkFormat::D32_SFLOAT => exr(TINYEXR_PIXELTYPE_FLOAT),

        VkFormat::R32_UINT
        | VkFormat::R32G32_UINT
        | VkFormat::R32G32B32_UINT
        | VkFormat::R32G32B32A32_UINT => exr(TINYEXR_PIXELTYPE_UINT),

        VkFormat::D16_UNORM_S8_UINT
        | VkFormat::D24_UNORM_S8_UINT
        | VkFormat::D32_SFLOAT_S8_UINT => {
            Some(Exr(vec![TINYEXR_PIXELTYPE_FLOAT, TINYEXR_PIXELTYPE_HALF]))
        }

        _ if is_astc_2d(vk) => Some(Astc),
        _ => None,
    }
}

/// The 2D ASTC formats of the switch (3D ASTC is not supported).
fn is_astc_2d(vk: VkFormat) -> bool {
    (VkFormat::ASTC_4x4_UNORM_BLOCK.0..=VkFormat::ASTC_12x12_SRGB_BLOCK.0).contains(&vk.0)
        || (VkFormat::ASTC_4x4_SFLOAT_BLOCK.0..=VkFormat::ASTC_12x12_SFLOAT_BLOCK.0).contains(&vk.0)
}

/// Port of `CommandExtract`.
#[derive(Debug, Default)]
pub struct CommandExtract {
    report: Reporter,
    options: OptionsExtract,
    target: OptionsTranscodeTarget<false>,
    single_in: OptionsSingleIn,
    generic: OptionsGeneric,
}

impl CommandExtract {
    /// Port of `CommandExtract::main`.
    pub fn main(args: &[String]) -> i32 {
        let mut cmd = CommandExtract::default();
        match cmd.run(args) {
            Ok(()) => ReturnCode::Success as i32,
            Err(FatalError(rc)) => rc,
        }
    }

    fn run(&mut self, args: &[String]) -> CmdResult {
        parse_command_line(
            &mut self.report,
            "ktx extract",
            "Extract one or multiple images from the KTX2 file specified as the input-file argument\n    and, based on the format, save them as Raw, EXR or PNG image files to the output-path.",
            args,
            &mut [
                &mut self.options,
                &mut self.target,
                &mut self.single_in,
                &mut self.generic,
                &mut ExtractPositional,
            ],
        )?;
        // processOptions
        if !self.options.raw
            && let Some(t) = self.target.transcode_target
            && t != TranscodeFmt::RGBA32
            && t != TranscodeFmt::RGBA_HALF
        {
            return Err(self.report.fatal_usage(&format!(
                "Transcode to \"{}\" for non-raw extract is not supported. For PNG/EXR output only r8, rg8, rgb8, rgba8 and rgba16f are supported.",
                self.target.transcode_target_name
            )));
        }
        self.execute_extract()
    }

    fn execute_extract(&mut self) -> CmdResult {
        let report = &self.report;
        let opts = &self.options;
        let uri = &opts.fragment_uri;
        let input = InputStream::new(&self.single_in.input_filepath, report)?;
        validate_tool_input(&input.data, report)?;

        let mut texture =
            match Texture2::create_from_memory(&input.data, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT)
            {
                Ok(t) => t,
                Err(e) => {
                    return Err(report.fatal(
                        ReturnCode::InvalidFile,
                        &format!("Failed to create KTX2 texture: {}", ktx_error_string(e)),
                    ));
                }
            };
        let invalid = |msg: String| Err(report.fatal(ReturnCode::InvalidFile, &msg));

        // CLI request validation
        if !uri.mip.validate(texture.num_levels) {
            return invalid(format!(
                "Requested level index {} is missing. The input file only has {} level(s).",
                uri.mip, texture.num_levels
            ));
        }

        if ((opts.uri_flag_used && !uri.stratal.is_undefined()) || opts.layer_flag_used)
            && !texture.is_array
        {
            return invalid(if uri.stratal.is_all() {
                "Requested all layers from a non-array texture.".to_string()
            } else {
                format!(
                    "Requested layer index {} from a non-array texture.",
                    uri.stratal
                )
            });
        }

        if !uri.stratal.validate(texture.num_layers) {
            return invalid(format!(
                "Requested layer index {} is missing. The input file only has {} layer(s).",
                uri.stratal, texture.num_layers
            ));
        }

        if ((opts.uri_flag_used && !uri.facial.is_undefined()) || opts.face_flag_used)
            && !texture.is_cubemap
        {
            return invalid(if uri.facial.is_all() {
                "Requested all faces from a non-cubemap texture.".to_string()
            } else {
                format!(
                    "Requested face index {} from a non-cubemap texture.",
                    uri.facial
                )
            });
        }

        if !uri.facial.validate(texture.num_faces) {
            return invalid(format!(
                "Requested face index {} is missing. The input file only has {} face(s).",
                uri.facial, texture.num_faces
            ));
        }

        if !opts.global_all && opts.depth_flag_used && texture.num_dimensions != 3 {
            return invalid(if opts.depth.is_all() {
                "Requested all depth slices from a non-3D texture.".to_string()
            } else {
                format!(
                    "Requested depth slice index {} from a non-3D texture.",
                    opts.depth
                )
            });
        }

        let (mut scale, mut offset) = (1.0f32, 0.0f32);
        let map_range = find_float_metadata_value(&texture, KTX_MAP_RANGE_KEY);
        if map_range.len() == 2 {
            scale = map_range[0];
            offset = map_range[1];
        } else if !map_range.is_empty() {
            return invalid("The input file has invalid ktxMapRange metadata.".to_string());
        }
        // Transcoding
        if texture.is_transcodable() {
            texture = transcode(texture, &mut self.target, report)?;
        } else if self.target.transcode_target.is_some() {
            return invalid(format!(
                "Requested transcode \"{}\" but the KTX file is not transcodable.",
                self.target.transcode_target_name
            ));
        }

        let format = FormatDescriptor::from_dfd(&texture.p_dfd);
        let block_size_z = format.texel_block_dimension[2] + 1;

        let last_exported_level = if uri.mip.is_all() {
            texture.num_levels.wrapping_sub(1)
        } else {
            uri.mip.last()
        };
        let last_exported_level_depth_count = 1u32.max(
            ceil_div(texture.base_depth, block_size_z)
                .checked_shr(last_exported_level)
                .unwrap_or(0),
        );
        if opts.depth_flag_used
            && !opts.depth.is_all()
            && opts.depth.last() > last_exported_level_depth_count
        {
            return invalid(format!(
                "Requested depth slice index {} is missing. The input file only has {} depth slice(s) in level {}.",
                opts.depth, last_exported_level_depth_count, last_exported_level
            ));
        }

        // Setup output directory
        let is_multi_output = (!uri.mip.is_undefined() && uri.mip.is_multi())
            || (!uri.stratal.is_undefined() && uri.stratal.is_multi())
            || (!uri.facial.is_undefined() && uri.facial.is_multi())
            || ((opts.global_all || opts.depth_flag_used) && opts.depth.is_multi());
        let output_path = std::path::Path::new(&opts.output_path);
        let dir = if is_multi_output {
            if output_path.exists() && !output_path.is_dir() {
                return Err(report.fatal_usage(&format!(
                    "Specified output path must be a directory for multi-output extract: \"{}\".",
                    opts.output_path
                )));
            }
            Some(output_path)
        } else {
            output_path.parent().filter(|p| !p.as_os_str().is_empty())
        };
        if let Some(dir) = dir
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            return Err(report.fatal(
                ReturnCode::IoFailure,
                &format!(
                    "Failed to create the output directory \"{}\": filesystem error: cannot create directories: {} [{}].",
                    dir.display(),
                    io_error_message(&e),
                    dir.display()
                ),
            ));
        }

        let data = texture.get_data().unwrap_or_default();
        let vk_format = texture.vk_format;
        let selected = |sel: &SelectorRange, index: u32| {
            if sel.is_undefined() {
                index == 0
            } else {
                sel.contains(index)
            }
        };

        // Iterate
        for level_index in 0..texture.num_levels {
            if !selected(&uri.mip, level_index) {
                continue;
            }

            let image_size = texture.get_image_size(level_index);
            let shr = |v: u32| 1u32.max(v.checked_shr(level_index).unwrap_or(0));
            let image_width = shr(texture.base_width);
            let image_height = shr(texture.base_height);
            let image_depth = shr(texture.base_depth);

            for layer_index in 0..texture.num_layers {
                if !selected(&uri.stratal, layer_index) {
                    continue;
                }

                for face_index in 0..texture.num_faces {
                    if !selected(&uri.facial, face_index) {
                        continue;
                    }
                    let image = |depth_index: u32| -> &[u8] {
                        let off = texture
                            .get_image_offset(level_index, layer_index, face_index + depth_index)
                            .unwrap_or(0);
                        data.get(off..off + image_size).unwrap_or_default()
                    };

                    if image_depth > 1 && !opts.global_all && !opts.depth_flag_used && opts.raw {
                        // If the texture type is 3D / 3D Array and the "all" or "depth" option is not set,
                        // the whole 3D block of pixel data is selected according to the "level" and "layer"
                        // option. This extraction path requires the "raw" option to be enabled.
                        let output_filepath = if is_multi_output {
                            format!(
                                "{}/output{}{}{}.raw",
                                opts.output_path,
                                if texture.num_levels > 1 {
                                    format!("_level{level_index}")
                                } else {
                                    String::new()
                                },
                                if texture.is_cubemap {
                                    format!("_face{face_index}")
                                } else {
                                    String::new()
                                },
                                if texture.is_array {
                                    format!("_layer{layer_index}")
                                } else {
                                    String::new()
                                },
                                // Depth is not part of the name as the whole 3D image is raw exported
                            )
                        } else {
                            opts.output_path.clone()
                        };

                        let mut file = OutputStream::new(&output_filepath, report)?;
                        for depth_index in 0..image_depth {
                            file.write(image(depth_index), report)?;
                        }
                        continue;
                    }

                    // Iterate z_slice_of_blocks
                    for depth_index in 0..ceil_div(image_depth, block_size_z) {
                        if !opts.depth.contains(depth_index) {
                            continue; // Skip
                        }

                        let output_filepath = if is_multi_output {
                            format!(
                                "{}/output{}{}{}{}",
                                opts.output_path,
                                if texture.num_levels > 1 {
                                    format!("_level{level_index}")
                                } else {
                                    String::new()
                                },
                                if texture.is_cubemap {
                                    format!("_face{face_index}")
                                } else {
                                    String::new()
                                },
                                if texture.is_array {
                                    format!("_layer{layer_index}")
                                } else {
                                    String::new()
                                },
                                if texture.base_depth > 1 {
                                    format!("_depth{depth_index}")
                                } else {
                                    String::new()
                                },
                            )
                        } else {
                            opts.output_path.clone()
                        };

                        if opts.raw {
                            save_raw_file(
                                report,
                                output_filepath,
                                is_multi_output,
                                image(depth_index),
                            )?;
                        } else {
                            save_image_file(
                                report,
                                output_filepath,
                                is_multi_output,
                                image(depth_index),
                                vk_format,
                                &format,
                                (image_width, image_height),
                                (scale, offset),
                            )?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Port of `findFloatMetadataValue`.
fn find_float_metadata_value(texture: &Texture2, key: &str) -> Vec<f32> {
    match texture.kv_data_head.find_value(key) {
        Ok(v) => v
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Port of `saveRawFile`.
fn save_raw_file(
    report: &Reporter,
    mut filepath: String,
    append_extension: bool,
    data: &[u8],
) -> CmdResult {
    if append_extension && filepath != "-" {
        filepath += ".raw";
    }
    let mut file = OutputStream::new(&filepath, report)?;
    file.write(data, report)
}

/// Port of `savePNG`.
#[allow(clippy::too_many_arguments)]
fn save_png(
    report: &Reporter,
    mut filepath: String,
    append_extension: bool,
    vk_format: VkFormat,
    format: &FormatDescriptor,
    width: u32,
    height: u32,
    data: &[u8],
) -> CmdResult {
    if append_extension && filepath != "-" {
        filepath += ".png";
    }

    let depth_format = vk_format == VkFormat::D16_UNORM;
    let alpha_format = vk_format == VkFormat::A8_UNORM_KHR;

    let mut packed_channel_count = 0usize;
    let mut unpacked_channel_count: u32 = 3;
    let mut color_type = lodepng::LCT_RGB;
    let mut bit_depth: u32 = 8;
    let mut src_offsets = [0u32; 4];
    let mut src_bits = [0u32; 4];
    let mut dst_channels = [0u32; 4];

    let mut add_channel = |dst_channel: u32, sample: &super::format_descriptor::Sample| {
        src_offsets[packed_channel_count] = sample.bit_offset;
        src_bits[packed_channel_count] = sample.bit_length + 1;
        dst_channels[packed_channel_count] = dst_channel;
        packed_channel_count += 1;
        if sample.bit_length >= 8 {
            bit_depth = 16;
        }
    };

    let mut state = lodepng::State::new();

    if format.model != KHR_DF_MODEL_RGBSDA {
        return Err(report.fatal(
            ReturnCode::NotSupported,
            &format!(
                "PNG saving is not supported for {} with {}.",
                to_string_model(format.model),
                to_string_vk_format(vk_format)
            ),
        ));
    }

    if let Some(sample) = format.find(KHR_DF_CHANNEL_RGBSDA_R) {
        add_channel(0, sample);
    }
    if let Some(sample) = format.find(KHR_DF_CHANNEL_RGBSDA_G) {
        add_channel(1, sample);
    }
    if let Some(sample) = format.find(KHR_DF_CHANNEL_RGBSDA_B) {
        add_channel(2, sample);
    }
    if let Some(sample) = format.find(KHR_DF_CHANNEL_RGBSDA_A) {
        if alpha_format {
            // For alpha-only formats we use grey-alpha
            unpacked_channel_count = 2;
            color_type = lodepng::LCT_GREY_ALPHA;
            add_channel(1, sample);
        } else {
            unpacked_channel_count = 4;
            color_type = lodepng::LCT_RGBA;
            add_channel(3, sample);
        }
    }
    if let Some(sample) = format.find(KHR_DF_CHANNEL_RGBSDA_D) {
        if !depth_format {
            return Err(report.fatal(
                ReturnCode::NotSupported,
                &format!(
                    "PNG saving encountered unexpected depth channel in non-depth format {}.",
                    to_string_vk_format(vk_format)
                ),
            ));
        }
        // For depth-only formats we use grey
        unpacked_channel_count = 1;
        color_type = lodepng::LCT_GREY;
        add_channel(0, sample);
    }

    let byte_depth = (bit_depth / 8) as usize;
    let pixel_bytes = format.pixel_byte_count() as usize;

    state.info_raw.bitdepth = bit_depth;
    state.info_png.color.colortype = color_type;
    state.info_png.color.bitdepth = bit_depth;
    state.info_raw.colortype = color_type;

    // Include sBit chunk if needed
    let include_sbits = src_bits.iter().any(|&b| b != 0 && b != bit_depth);
    if include_sbits {
        let sbit = |b: u32| if b == 0 { bit_depth } else { b };
        state.info_png.sbit_defined = true;
        state.info_png.sbit_r = sbit(src_bits[0]);
        state.info_png.sbit_g = sbit(src_bits[1]);
        state.info_png.sbit_b = sbit(src_bits[2]);
        state.info_png.sbit_a = sbit(src_bits[3]);
    }

    let (w, h) = (width as usize, height as usize);
    let channels = unpacked_channel_count as usize;
    let mut unpacked_image = vec![0u8; w * h * channels * byte_depth];
    for y in 0..h {
        for x in 0..w {
            let raw_pixel = data.get((y * w + x) * pixel_bytes..).unwrap_or_default();
            for c in 0..packed_channel_count {
                let bits = src_bits[c];
                let value = convert_unorm(
                    extract_bits(raw_pixel, src_offsets[c], bits),
                    bits,
                    bit_depth,
                );
                let o = (y * w * channels + x * channels + dst_channels[c] as usize) * byte_depth;
                if byte_depth == 1 {
                    unpacked_image[o] = value as u8;
                } else {
                    // LodePNG uses big endian input
                    unpacked_image[o..o + 2].copy_from_slice(&(value as u16).to_be_bytes());
                }
            }
        }
    }

    // Include sRGB chunk if needed
    if format.transfer == KHR_DF_TRANSFER_SRGB {
        state.info_png.srgb_defined = true;
        state.info_png.srgb_intent = 0;
    } else if format.transfer == KHR_DF_TRANSFER_LINEAR
        || format.transfer == KHR_DF_TRANSFER_UNSPECIFIED
    {
        // Virtually every tool takes absence of colorspace info to mean sRGB so write gamma 1.
        state.info_png.srgb_defined = false;
        state.info_png.gama_defined = true;
        state.info_png.gama_gamma = 100000;
    }

    // Output primaries as cHRM chunk
    if let Some(p) = get_primaries(format.primaries) {
        let fixed = |v: f32| (100000.0f32 * v) as u32;
        let info = &mut state.info_png;
        info.chrm_defined = true;
        info.chrm_red_x = fixed(p.Rx);
        info.chrm_red_y = fixed(p.Ry);
        info.chrm_green_x = fixed(p.Gx);
        info.chrm_green_y = fixed(p.Gy);
        info.chrm_blue_x = fixed(p.Bx);
        info.chrm_blue_y = fixed(p.By);
        info.chrm_white_x = fixed(p.Wx);
        info.chrm_white_y = fixed(p.Wy);
    }

    let png = match lodepng::encode(&unpacked_image, width, height, &mut state) {
        Ok(png) => png,
        Err(error) => {
            return Err(report.fatal(
                ReturnCode::InvalidFile,
                &format!(
                    "PNG Encoder error {error}: {}.",
                    lodepng::lodepng_error_text(error)
                ),
            ));
        }
    };

    let mut file = OutputStream::new(&filepath, report)?;
    file.write(&png, report)
}

/// Port of `saveImageFile`.
#[allow(clippy::too_many_arguments)]
fn save_image_file(
    report: &Reporter,
    filepath: String,
    append_extension: bool,
    data: &[u8],
    vk_format: VkFormat,
    format: &FormatDescriptor,
    (width, height): (u32, u32),
    _map_range: (f32, f32),
) -> CmdResult {
    let what = match image_save_kind(vk_format) {
        Some(ImageSave::Png) => {
            return save_png(
                report,
                filepath,
                append_extension,
                vk_format,
                format,
                width,
                height,
                data,
            );
        }
        Some(ImageSave::Astc) => "ASTC decoding",
        Some(ImageSave::Unpack422) => "4:2:2 unpacking",
        Some(ImageSave::Exr(_)) => "EXR",
        None => {
            return Err(report.fatal(
                ReturnCode::InvalidFile,
                &format!(
                    "Requested format conversion from {} is not supported.",
                    to_string_vk_format(vk_format)
                ),
            ));
        }
    };
    Err(report.fatal(
        ReturnCode::NotImplemented,
        &format!("{what} output is not implemented yet. Use --raw."),
    ))
}
