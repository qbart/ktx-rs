//! Port of tools/ktx/command_extract.cpp (`ktx extract`).
//!
//! Raw, PNG (lodepng), EXR (tinyexr), ASTC decoding (astcenc) and 4:2:2
//! unpacking are ported.

use super::command::*;
use super::format_descriptor::*;
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

use crate::tinyexr::{
    ExrAttribute, ExrChannel, TINYEXR_PIXELTYPE_FLOAT, TINYEXR_PIXELTYPE_HALF,
    TINYEXR_PIXELTYPE_UINT, save_exr_image_to_memory,
};

/// How `saveImageFile` stores a format.
#[derive(Debug, Clone, PartialEq)]
enum ImageSave {
    Png,
    Astc,
    Unpack422,
    /// EXR with per-channel pixel types (one type means every channel) and
    /// whether the ktxMapRange scale / offset is passed on.
    Exr(Vec<i32>, bool),
}

/// The `saveImageFile` switch.
fn image_save_kind(vk: VkFormat) -> Option<ImageSave> {
    use ImageSave::*;
    let exr = |t: i32| Some(Exr(vec![t], false));
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
        | VkFormat::S8_UINT => exr(TINYEXR_PIXELTYPE_HALF),

        VkFormat::R16_SFLOAT
        | VkFormat::R16G16_SFLOAT
        | VkFormat::R16G16B16_SFLOAT
        | VkFormat::R16G16B16A16_SFLOAT
        | VkFormat::B10G11R11_UFLOAT_PACK32
        | VkFormat::E5B9G9R9_UFLOAT_PACK32 => Some(Exr(vec![TINYEXR_PIXELTYPE_HALF], true)),

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
        | VkFormat::D32_SFLOAT_S8_UINT => Some(Exr(
            vec![TINYEXR_PIXELTYPE_FLOAT, TINYEXR_PIXELTYPE_HALF],
            false,
        )),

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

/// `saveEXR` channel description.
struct ExrSourceChannel {
    offset: u32,
    bits: u32,
    name: &'static str,
    is_float: bool,
    is_signed: bool,
    is_normalized: bool,
}

/// Port of `saveEXR` (both overloads: one pixel type means every channel).
#[allow(clippy::too_many_arguments)]
fn save_exr(
    report: &Reporter,
    mut filepath: String,
    append_extension: bool,
    vk_format: VkFormat,
    format: &FormatDescriptor,
    width: u32,
    height: u32,
    pixel_types: &[i32],
    data: &[u8],
    map_range_scale: f32,
    map_range_offset: f32,
) -> CmdResult {
    let is_e5b9g9r9 = vk_format == VkFormat::E5B9G9R9_UFLOAT_PACK32;
    let num_channels = if is_e5b9g9r9 {
        3
    } else {
        format.channel_count as usize
    };
    let pixel_types: Vec<i32> = if pixel_types.len() == 1 {
        vec![pixel_types[0]; num_channels]
    } else {
        pixel_types.to_vec()
    };

    if append_extension && filepath != "-" {
        filepath += ".exr";
    }

    let mut channels: Vec<ExrSourceChannel> = Vec::new();
    if format.model == KHR_DF_MODEL_RGBSDA {
        // Must be ABGR order, since most of EXR viewers expect this channel order by convention.
        for (channel_id, name) in [
            (KHR_DF_CHANNEL_RGBSDA_A, "A"),
            (KHR_DF_CHANNEL_RGBSDA_D, "D"),
            (KHR_DF_CHANNEL_RGBSDA_S, "S"),
            (KHR_DF_CHANNEL_RGBSDA_B, "B"),
            (KHR_DF_CHANNEL_RGBSDA_G, "G"),
            (KHR_DF_CHANNEL_RGBSDA_R, "R"),
        ] {
            if let Some(sample) = format.find(channel_id) {
                let one = if sample.qualifier_float {
                    1.0f32.to_bits()
                } else {
                    1
                };
                channels.push(ExrSourceChannel {
                    offset: sample.bit_offset,
                    bits: sample.bit_length + 1,
                    name,
                    is_float: sample.qualifier_float,
                    is_signed: sample.qualifier_signed,
                    is_normalized: sample.upper != one,
                });
            }
        }
    } else {
        return Err(report.fatal(
            ReturnCode::NotSupported,
            &format!(
                "EXR saving is unsupported for {} with {}.",
                to_string_model(format.model),
                to_string_vk_format(vk_format)
            ),
        ));
    }

    let pixel_bytes = format.pixel_byte_count() as usize;
    let do_mapping = !(map_range_scale == 1.0 && map_range_offset == 0.0);
    let mut requested_pixel_types = pixel_types.clone();

    // Image data is prepared with either floats or uint32
    // (half output is will be converted from float by TinyEXR during save)
    let (w, h) = (width as usize, height as usize);
    let mut images: Vec<Vec<u32>> = vec![vec![0; w * h]; num_channels];

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let raw_pixel = data.get(i * pixel_bytes..).unwrap_or_default();

            if is_e5b9g9r9 {
                // Special case for VK_FORMAT_E5B9G9R9_UFLOAT_PACK32
                let [r, g, b] = unpack_f3x9_e1x5(extract_bits(raw_pixel, 0, 32));
                let map = |v: f32| {
                    if do_mapping {
                        v * map_range_scale + map_range_offset
                    } else {
                        v
                    }
                };
                if do_mapping {
                    requested_pixel_types.fill(TINYEXR_PIXELTYPE_FLOAT);
                }
                images[2][i] = map(r).to_bits();
                images[1][i] = map(g).to_bits();
                images[0][i] = map(b).to_bits();
                continue;
            }
            for c in 0..num_channels {
                let Some(channel) = channels.get(c) else {
                    continue;
                };
                let (scale, offset) = if channel.name == "A" {
                    (1.0, 0.0)
                } else {
                    (map_range_scale, map_range_offset)
                };
                let bits = channel.bits;
                let value = extract_bits(raw_pixel, channel.offset, bits);
                let pixel_type = pixel_types
                    .get(c)
                    .copied()
                    .unwrap_or(TINYEXR_PIXELTYPE_HALF);
                let target = if pixel_type == TINYEXR_PIXELTYPE_FLOAT
                    || pixel_type == TINYEXR_PIXELTYPE_HALF
                {
                    let f = if channel.is_float {
                        if do_mapping {
                            requested_pixel_types[c] = TINYEXR_PIXELTYPE_FLOAT;
                        }
                        if channel.is_signed {
                            convert_sfloat_to_float(value, bits, scale, offset)
                        } else {
                            convert_ufloat_to_float(value, bits, scale, offset)
                        }
                    } else if channel.is_normalized {
                        if do_mapping {
                            requested_pixel_types[c] = TINYEXR_PIXELTYPE_FLOAT;
                        }
                        if channel.is_signed {
                            convert_snorm_to_float(value, bits, scale, offset)
                        } else {
                            convert_unorm_to_float(value, bits, scale, offset)
                        }
                    } else if channel.is_signed {
                        convert_sint_to_float(value, bits)
                    } else {
                        convert_uint_to_float(value)
                    };
                    f.to_bits()
                } else {
                    // TINYEXR_PIXELTYPE_UINT
                    match (channel.is_float, channel.is_signed) {
                        (true, true) => convert_sfloat_to_uint(value, bits),
                        (false, false) => value,
                        // convertUFloatToUInt / convertSIntToUInt: not yet implemented in C++
                        _ => 0,
                    }
                };
                images[c][i] = target;
            }
        }
    }

    let exr_channels: Vec<ExrChannel> = images
        .into_iter()
        .enumerate()
        .map(|(c, data)| ExrChannel {
            name: channels.get(c).map_or("", |ch| ch.name).to_string(),
            // pixel type of the input image
            pixel_type: if pixel_types.get(c) == Some(&TINYEXR_PIXELTYPE_UINT) {
                TINYEXR_PIXELTYPE_UINT
            } else {
                TINYEXR_PIXELTYPE_FLOAT
            },
            // pixel type of the output image to be stored in .EXR file
            requested_pixel_type: requested_pixel_types[c],
            data,
        })
        .collect();

    // Output primaries as chromaticities
    let mut attributes = Vec::new();
    if let Some(p) = get_primaries(format.primaries) {
        let value = [p.Rx, p.Ry, p.Gx, p.Gy, p.Bx, p.By, p.Wx, p.Wy]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        attributes.push(ExrAttribute {
            name: "chromaticities".to_string(),
            attr_type: "chromaticities".to_string(),
            value,
        });
    }

    let file_data =
        match save_exr_image_to_memory(width as i32, height as i32, &exr_channels, &attributes) {
            Ok(d) => d,
            Err(e) => {
                return Err(
                    report.fatal(ReturnCode::IoFailure, &format!("EXR Encoder error: {e}."))
                );
            }
        };

    let mut file = OutputStream::new(&filepath, report)?;
    file.write(&file_data, report)
}

/// Port of `decodeAndSaveASTC`.
#[allow(clippy::too_many_arguments)]
fn decode_and_save_astc(
    report: &Reporter,
    filepath: String,
    append_extension: bool,
    vk_format: VkFormat,
    format: &FormatDescriptor,
    (width, height): (u32, u32),
    (scale, offset): (f32, f32),
    compressed_data: &[u8],
) -> CmdResult {
    use crate::astcenc::*;
    let thread_count = 1;
    let block_size_x = format.texel_block_dimension[0] + 1;
    let block_size_y = format.texel_block_dimension[1] + 1;
    let block_size_z = format.texel_block_dimension[2] + 1;
    let swizzle = AstcencSwizzle::RGBA;

    let do_mapping = !(scale == 1.0 && offset == 0.0);
    let ldr = super::formats_gen::is_format_astc_ldr(vk_format);
    let srgb = super::formats_gen::is_format_srgb(vk_format);
    let profile = if srgb {
        AstcencProfile::LdrSrgb
    } else if ldr {
        AstcencProfile::Ldr
    } else {
        AstcencProfile::Hdr
    };
    let runtime = |what: &str, ec: AstcencError| {
        report.fatal(
            ReturnCode::RuntimeError,
            &format!("{what}: {}", astcenc_get_error_string(ec)),
        )
    };
    let config = astcenc_config_init(
        profile,
        block_size_x,
        block_size_y,
        block_size_z,
        ASTCENC_PRE_MEDIUM,
        ASTCENC_FLG_DECOMPRESS_ONLY,
    )
    .map_err(|ec| runtime("ASTC Codec config init failed", ec))?;
    let mut context = astcenc_context_alloc(&config, thread_count)
        .map_err(|ec| runtime("ASTC Codec context alloc failed", ec))?;

    let n = width as usize * height as usize * 4;
    let (data, uncompressed_vk_format) = if ldr {
        (
            AstcencImageData::U8(vec![vec![0; n]]),
            if srgb {
                VkFormat::R8G8B8A8_SRGB
            } else {
                VkFormat::R8G8B8A8_UNORM
            },
        )
    } else if do_mapping {
        (
            AstcencImageData::F32(vec![vec![0.0; n]]),
            VkFormat::R32G32B32A32_SFLOAT,
        )
    } else {
        (
            AstcencImageData::F16(vec![vec![0; n]]),
            VkFormat::R16G16B16A16_SFLOAT,
        )
    };
    let mut image = AstcencImage {
        dim_x: width,
        dim_y: height,
        dim_z: 1, // 3D ASTC formats are currently not supported
        data,
    };
    astcenc_decompress_image(&mut context, compressed_data, &mut image, &swizzle, 0)
        .map_err(|ec| runtime("ASTC Codec decompress failed", ec))?;
    let _ = astcenc_decompress_reset(&mut context);

    let uncompressed: Vec<u8> = match image.data {
        AstcencImageData::U8(mut p) => p.pop().unwrap_or_default(),
        AstcencImageData::F16(p) => p.iter().flatten().flat_map(|v| v.to_le_bytes()).collect(),
        AstcencImageData::F32(p) => p
            .iter()
            .flatten()
            .flat_map(|v| {
                let v = if do_mapping { v * scale + offset } else { *v };
                v.to_le_bytes()
            })
            .collect(),
    };

    // Range mapping has been handled here so no need to pass the parameters on to
    // this recursive call.
    let uncompressed_format = FormatDescriptor::from_vk_format(uncompressed_vk_format, report)?;
    save_image_file(
        report,
        filepath,
        append_extension,
        &uncompressed,
        uncompressed_vk_format,
        &uncompressed_format,
        (width, height),
        (1.0, 0.0),
    )
}

/// `remap` (utility.h).
fn remap(value: f32, from_lo: f32, from_hi: f32, to_lo: f32, to_hi: f32) -> f32 {
    to_lo + (value - from_lo) * (to_hi - to_lo) / (from_hi - from_lo)
}

/// `convertFloatToUNORM`.
fn convert_float_to_unorm(value: f32, num_bits: u32) -> u32 {
    let max = ((1u64 << num_bits) - 1) as u32;
    if value.is_nan() || value < 0.0 {
        return 0;
    }
    if value > 1.0 {
        return max;
    }
    (value * max as f32 + 0.5) as u32
}

/// Port of `unpackAndSave422`: 4:2:2 blocks (Y0, Y1, U, V) are unpacked to two
/// RGB pixels (V, Y, U order like the C++) with linear chroma interpolation.
fn unpack_and_save_422(
    report: &Reporter,
    filepath: String,
    append_extension: bool,
    format: &FormatDescriptor,
    (width, height): (u32, u32),
    data: &[u8],
) -> CmdResult {
    // Create a custom format with the same precision but with only 3 channels
    // Reuse similar 4 channel VkFormats and drop the last channel (There is no RGB variant of 10X6 and 12X4)
    let precision = format
        .find(KHR_DF_CHANNEL_YUVSDA_Y)
        .map_or(0, |s| s.bit_length + 1);
    let unpacked_vk = match precision {
        8 => VkFormat::R8G8B8A8_UNORM,
        10 => VkFormat::R10X6G10X6B10X6A10X6_UNORM_4PACK16,
        12 => VkFormat::R12X4G12X4B12X4A12X4_UNORM_4PACK16,
        16 => VkFormat::R16G16B16A16_UNORM,
        _ => VkFormat::UNDEFINED,
    };
    let mut unpacked_format = FormatDescriptor::from_vk_format(unpacked_vk, report)?;
    unpacked_format.remove_last_channel();

    // 1 pixel (block) with 4 channel is unpacked to 2 pixel with 3 channels: Y0,Y1,U,V -> R0,G0,B0,R1,G1,B1
    let block_yuv_bytes = format.pixel_byte_count() as usize;
    let block_dimension_x = format.texel_block_dimension[0] + 1;
    let pixel_bytes = unpacked_format.pixel_byte_count() as usize;
    let channel_bytes = pixel_bytes / 3;

    let (mut y0, mut y1, mut u, mut v) = ((0u32, 0u32, 0u32), (0, 0, 0), (0, 0, 0), (0, 0, 0));
    for sample in &format.samples {
        let entry = (
            sample.bit_offset,
            sample.bit_length + 1,
            sample.sample_position[0],
        );
        match sample.channel_id {
            KHR_DF_CHANNEL_YUVSDA_Y => {
                if y0.1 != 0 {
                    y1 = entry;
                } else {
                    y0 = entry;
                }
            }
            KHR_DF_CHANNEL_YUVSDA_U => u = entry,
            KHR_DF_CHANNEL_YUVSDA_V => v = entry,
            _ => {}
        }
    }
    if y0.2 > y1.2 {
        // Ensure that y0 (as we refer to) is the left sample
        std::mem::swap(&mut y0, &mut y1);
    }

    let position = |p: u32| (p * block_dimension_x) as f32 / 256.0;
    let position_y0 = position(y0.2);
    let position_y1 = position(y1.2);
    let position_u = position(u.2);
    let position_v = position(v.2);
    let block_size = block_dimension_x as f32;

    let (w, h) = (width as usize, height as usize);
    let mut unpacked_data = vec![0u8; w * h * pixel_bytes];
    let block_count_x = (width / block_dimension_x) as usize;
    let value = |block: usize, (offset, bits, _): (u32, u32, u32)| {
        let raw = data.get(block * block_yuv_bytes..).unwrap_or_default();
        convert_unorm_to_float(extract_bits(raw, offset, bits), bits, 1.0, 0.0)
    };
    for y in 0..h {
        for x in 0..block_count_x {
            let left = y * block_count_x + if x == 0 { 0 } else { x - 1 };
            let center = y * block_count_x + x;
            let right = y * block_count_x
                + if x == block_count_x - 1 {
                    block_count_x - 1
                } else {
                    x + 1
                };

            let value_left_y1 = value(left, y1);
            let value_left_u = value(left, u);
            let value_left_v = value(left, v);
            let value_y0 = value(center, y0);
            let value_y1 = value(center, y1);
            let value_u = value(center, u);
            let value_v = value(center, v);
            let value_right_y0 = value(right, y0);
            let value_right_u = value(right, u);
            let value_right_v = value(right, v);

            #[allow(clippy::too_many_arguments)]
            let interpolate_y = |pos: f32,
                                 pos0: f32,
                                 value0: f32,
                                 pos1: f32,
                                 value1: f32,
                                 pos2: f32,
                                 value2: f32,
                                 pos3: f32,
                                 value3: f32| {
                if pos < pos1 {
                    remap(pos, pos0, pos1, value0, value1)
                } else if pos < pos2 {
                    remap(pos, pos1, pos2, value1, value2)
                } else {
                    remap(pos, pos2, pos3, value2, value3)
                }
            };
            let interpolate_uv = |pos: f32,
                                  pos0: f32,
                                  value0: f32,
                                  pos1: f32,
                                  value1: f32,
                                  pos2: f32,
                                  value2: f32| {
                if pos < pos1 {
                    remap(pos, pos0, pos1, value0, value1)
                } else {
                    remap(pos, pos1, pos2, value1, value2)
                }
            };

            let mut set_pixel = |pixel: usize, pos: f32| {
                let r = convert_float_to_unorm(
                    interpolate_uv(
                        pos,
                        position_v - block_size,
                        value_left_v,
                        position_v,
                        value_v,
                        position_v + block_size,
                        value_right_v,
                    ),
                    u.1,
                );
                let g = convert_float_to_unorm(
                    interpolate_y(
                        pos,
                        position_y1 - block_size,
                        value_left_y1,
                        position_y0,
                        value_y0,
                        position_y1,
                        value_y1,
                        position_y0 + block_size,
                        value_right_y0,
                    ),
                    y0.1,
                );
                let b = convert_float_to_unorm(
                    interpolate_uv(
                        pos,
                        position_u - block_size,
                        value_left_u,
                        position_u,
                        value_u,
                        position_u + block_size,
                        value_right_u,
                    ),
                    u.1,
                );
                let o = pixel * pixel_bytes;
                for (c, val) in [r, g, b].into_iter().enumerate() {
                    let bytes = val.to_le_bytes();
                    let dst = o + c * channel_bytes;
                    if let Some(d) = unpacked_data.get_mut(dst..dst + channel_bytes) {
                        d.copy_from_slice(&bytes[..channel_bytes]);
                    }
                }
            };
            set_pixel(y * w + x * 2, 0.5);
            set_pixel(y * w + x * 2 + 1, 1.5);
        }
    }

    // Save unpacked data as RGBSDA with the custom format
    save_png(
        report,
        filepath,
        append_extension,
        VkFormat::UNDEFINED,
        &unpacked_format,
        width,
        height,
        &unpacked_data,
    )
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
    (scale, offset): (f32, f32),
) -> CmdResult {
    match image_save_kind(vk_format) {
        Some(ImageSave::Png) => save_png(
            report,
            filepath,
            append_extension,
            vk_format,
            format,
            width,
            height,
            data,
        ),
        Some(ImageSave::Astc) => {
            // ASTC decode will recurse into this function with the uncompressed data and format
            decode_and_save_astc(
                report,
                filepath,
                append_extension,
                vk_format,
                format,
                (width, height),
                (scale, offset),
                data,
            )
        }
        Some(ImageSave::Unpack422) => unpack_and_save_422(
            report,
            filepath,
            append_extension,
            format,
            (width, height),
            data,
        ),
        Some(ImageSave::Exr(pixel_types, map_range)) => {
            let (scale, offset) = if map_range {
                (scale, offset)
            } else {
                (1.0, 0.0)
            };
            save_exr(
                report,
                filepath,
                append_extension,
                vk_format,
                format,
                width,
                height,
                &pixel_types,
                data,
                scale,
                offset,
            )
        }
        None => Err(report.fatal(
            ReturnCode::InvalidFile,
            &format!(
                "Requested format conversion from {} is not supported.",
                to_string_vk_format(vk_format)
            ),
        )),
    }
}
