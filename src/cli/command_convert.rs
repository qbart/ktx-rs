//! Port of tools/ktx/command_convert.cpp (`ktx convert`): KTX v1 → KTX v2.

use std::path::PathBuf;

use super::command::*;
use super::options::{OptionSpec, Options, ParseResult};
use crate::defs::{KTX_ORIENTATION_KEY, KTX_WRITER_KEY};
use crate::gl_format::{GL_COMPRESSED_RGBA_ASTC_4x4_KHR, GL_COMPRESSED_RGBA_ASTC_12x12_KHR};
use crate::glformat_str::gl_internalformat_string;
use crate::vk_format::RgbaAstcMapping;
use crate::{ErrorCode, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, Texture1, ktx_error_string};

/// `CommandConvert::input_type_e`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputType {
    Ktx,
}

/// `CommandConvert::OptionsConvert`.
#[derive(Debug, Clone, Default)]
struct OptionsConvert {
    drop_bad_orientation: bool,
    map_astc_rgba_to_ldr: bool,
    input_type: Option<InputType>,
}

const K_MAP_ASTC_RGBA_TO_LDR: &str = "map-astc-rgba-to-ldr";
const K_DROP_BAD_ORIENTATION: &str = "drop-bad-orientation";
const K_INPUT_TYPE: &str = "input-type";

impl OptionGroup for OptionsConvert {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::value(
            &format!("t,{K_INPUT_TYPE}"),
            "Specify the type of input file. Currently must be ktx.",
            "<type>",
        ));
        opts.add(OptionSpec::flag(
            &format!("l,{K_MAP_ASTC_RGBA_TO_LDR}"),
            "Map GL_COMPRESSED_RGBA_ASTC_*_KHR formats to VK_FORMAT_ASTC_*_UNORM_BLOCK (LDR) formats. By default they will be mapped to VK_FORMAT_ASTC_*_SFLOAT_BLOCK (HDR) formats.",
        ));
        opts.add(OptionSpec::flag(
            &format!("d,{K_DROP_BAD_ORIENTATION}"),
            "Drop bad orientation metadata, such as \"KTXOrientation\", instead of fixing it.",
        ));
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        if args.count(K_INPUT_TYPE) > 0 {
            let given = args.get(K_INPUT_TYPE).unwrap_or_default();
            match given.to_uppercase().as_str() {
                "KTX" => self.input_type = Some(InputType::Ktx),
                _ => {
                    return Err(report.fatal_usage(&format!(
                        "Invalid or unsupported type specified as --{K_INPUT_TYPE} argument: \"{given}\"."
                    )));
                }
            }
        }
        if self.input_type.is_none() {
            return Err(report.fatal_usage(&format!("--{K_INPUT_TYPE} <type> must be specified")));
        }
        self.drop_bad_orientation = args.as_bool(K_DROP_BAD_ORIENTATION);
        self.map_astc_rgba_to_ldr = args.as_bool(K_MAP_ASTC_RGBA_TO_LDR);
        Ok(())
    }
}

/// Port of `CommandConvert`.
#[derive(Debug, Default)]
pub struct CommandConvert {
    report: Reporter,
    convert: OptionsConvert,
    in_out: OptionsSingleInSingleOut<true>,
    generic: OptionsGeneric,
}

/// `std::filesystem::path::replace_extension("ktx2")`.
fn replace_extension(p: &mut PathBuf) {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = match name.rfind('.') {
        Some(i) if i > 0 && name != ".." => name[..i].to_string(),
        _ => name,
    };
    p.set_file_name(format!("{stem}.ktx2"));
}

impl CommandConvert {
    /// Port of `CommandConvert::main`.
    pub fn main(args: &[String]) -> i32 {
        let mut cmd = CommandConvert::default();
        match cmd.run(args) {
            Ok(()) => ReturnCode::Success as i32,
            Err(FatalError(rc)) => rc,
        }
    }

    fn run(&mut self, args: &[String]) -> CmdResult {
        parse_command_line(
            &mut self.report,
            "ktx convert",
            "Convert the non-KTX2 texture file specified as the input-file argument,\n    optionally supercompress the result, and save it as the output-file.",
            args,
            &mut [&mut self.convert, &mut self.in_out, &mut self.generic],
        )?;
        self.execute_convert()
    }

    fn execute_convert(&mut self) -> CmdResult {
        let report = self.report.clone();
        let input = InputStream::new(&self.in_out.input_filepath, &report)?;

        let mut output_filepath = PathBuf::from(&self.in_out.output_filepath);
        let mut using_input_name = false;
        // If no output path given or output is a directory, use input path/filename
        // changing extension to or adding ".ktx2".
        if self.in_out.output_filepath.is_empty() {
            output_filepath = PathBuf::from(&self.in_out.input_filepath);
            using_input_name = true;
        } else if self.in_out.output_filepath.ends_with('/') || output_filepath.is_dir() {
            let input_path = PathBuf::from(&self.in_out.input_filepath);
            if let Some(name) = input_path.file_name() {
                output_filepath.push(name);
            }
            using_input_name = true;
        }
        if using_input_name {
            replace_extension(&mut output_filepath);
        }
        if let Some(parent) = output_filepath.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
        }
        let out_path = output_filepath.to_string_lossy().into_owned();
        let mut output = OutputStream::new(&out_path, &report)?;

        if self.convert.input_type == Some(InputType::Ktx)
            && let Err(e) = self.convert_ktx(&input, &mut output)
        {
            output.remove_on_destruct();
            return Err(e);
        }

        if !output.is_stdout() {
            drop(output);
            let mut messages = String::new();
            let data = InputStream::new(&out_path, &report)?.data;
            let result = super::validate::validate_memory(&data, false, false, &mut |issue| {
                messages += &format!(
                    "{}-{:04}: {}\n",
                    issue.type_.as_str(),
                    issue.id,
                    issue.message
                );
                messages += &format!("    {}\n", String::from_utf8_lossy(&issue.details));
            });
            match result {
                Ok(0) => {}
                Ok(rc) => {
                    return Err(report.fatal_rc(rc, &format!(
                        "Validation of converted file \"{out_path}\" failed. This is likely due to an internal issue in the tool. If, after looking at the validation messages below, you agree, please open an issue at https://github.com/KhronosGroup/KTX-Software/issues. The file has not been deleted so you can attach it to the issue.\n\n{messages}"
                    )));
                }
                Err(what) => return Err(runtime_error(&report, &what)),
            }
        }
        Ok(())
    }

    /// `convertKtx`.
    fn convert_ktx(&mut self, input: &InputStream, output: &mut OutputStream) -> CmdResult {
        let report = &self.report;
        let mut texture =
            match Texture1::create_from_memory(&input.data, KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT)
            {
                Ok(t) => t,
                Err(ErrorCode::UnsupportedTextureType) => {
                    let gl_internalformat = input
                        .data
                        .get(28..32)
                        .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    return Err(report.fatal(
                    ReturnCode::NotSupported,
                    &format!(
                        "Format of input file, {}, is unsupported or has no equivalent VkFormat.",
                        gl_internalformat_string(gl_internalformat)
                    ),
                ));
                }
                Err(e) => {
                    return Err(report.fatal(
                        ReturnCode::InvalidFile,
                        &format!("Failed to create KTX texture: {}", ktx_error_string(e)),
                    ));
                }
            };

        if self.convert.map_astc_rgba_to_ldr
            && !(GL_COMPRESSED_RGBA_ASTC_4x4_KHR..=GL_COMPRESSED_RGBA_ASTC_12x12_KHR)
                .contains(&texture.gl_internalformat)
        {
            self.convert.map_astc_rgba_to_ldr = false;
        }

        // Some in-the-wild KTX files have incorrect KTXOrientation
        // Warn about dropping invalid metadata. Like the C++ loop, the entry
        // following a deleted one is skipped.
        let mut i = 0;
        while let Some(entry) = texture.kv_data_head.entry(i) {
            let key_str = String::from_utf8_lossy(entry.get_key().1).into_owned();
            if (key_str.contains("ktx") || key_str.contains("KTX"))
                && key_str != KTX_ORIENTATION_KEY
                && key_str != KTX_WRITER_KEY
            {
                if key_str == "KTXOrientation" && !self.convert.drop_bad_orientation {
                    let value = entry.get_value().1.to_vec();
                    let _ = texture
                        .kv_data_head
                        .add_kv_pair(KTX_ORIENTATION_KEY, &value);
                } else {
                    report.warning(&format!("Dropping unrecognized KTX metadata \"{key_str}\""));
                }
                let _ = texture.kv_data_head.delete_entry(i);
            }
            i += 1;
        }

        // Add required writer metadata.
        let mut writer =
            format!("{} {}", report.command_name, version(self.generic.testrun)).into_bytes();
        writer.push(0);
        let _ = texture.kv_data_head.add_kv_pair(KTX_WRITER_KEY, &writer);

        if self.convert.map_astc_rgba_to_ldr {
            texture.set_rgba_astc_mapping(RgbaAstcMapping::ToLdr);
        }

        match texture.write_ktx2_to_memory() {
            Ok(bytes) => output.write(&bytes, report),
            Err(e) => {
                if !output.is_stdout() {
                    let _ = std::fs::remove_file(&output.filepath);
                }
                Err(report.fatal(
                    ReturnCode::IoFailure,
                    &format!(
                        "Failed to write KTX file \"{}\": KTX error: {}.",
                        output.filepath,
                        ktx_error_string(e)
                    ),
                ))
            }
        }
    }
}
