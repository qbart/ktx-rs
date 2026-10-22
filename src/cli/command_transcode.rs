//! Port of tools/ktx/command_transcode.cpp (`ktx transcode`).

use super::command::*;
use super::deflate_utils::OptionsDeflate;
use super::options::{OptionSpec, Options, ParseResult};
use super::transcode_utils::{OptionsTranscodeTarget, transcode};
use crate::defs::{KTX_WRITER_KEY, KTX_WRITER_SCPARAMS_KEY};
use crate::{KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, Texture2, ktx_error_string};

/// `CommandTranscode::OptionsTranscode`.
#[derive(Debug, Clone, Default)]
struct OptionsTranscode;

impl OptionGroup for OptionsTranscode {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::value(
            "target",
            "Target transcode format. If the target option is not set the r8, rg8, rgb8 or rgba8 target will be selected for LDR payloads based on the number of channels in the input texture. rgba16f will be selected for HDR payloads. Case-insensitive.\nPossible options are: etc-rgb | etc-rgba | eac-r11 | eac-rg11 | bc1 | bc3 | bc4 | bc5 | bc6hu | bc7 | astc | astc-hdr-4x4 | astc-hdr-6x6 | r8 | rg8 | rgb8 | rgba8 | rgb16f | rgba16f | rgb9e5.\nKTX files with UASTC HDR payloads can only be transcoded to HDR formats: astc-hdr-*, rgb16f, rgba16f or rgb9e5. LDR payloads can only be transcoded to LDR formats. etc-rgb is ETC1; etc-rgba, eac-r11 and eac-rg11 are ETC2.",
            "<target>",
        ));
    }

    fn process(&mut self, _opts: &Options, _args: &ParseResult, _report: &Reporter) -> CmdResult {
        Ok(())
    }
}

/// Port of `CommandTranscode`.
#[derive(Debug, Default)]
pub struct CommandTranscode {
    report: Reporter,
    transcode: OptionsTranscode,
    target: OptionsTranscodeTarget<true>,
    deflate: OptionsDeflate,
    in_out: OptionsSingleInSingleOut,
    generic: OptionsGeneric,
}

impl CommandTranscode {
    /// Port of `CommandTranscode::main`.
    pub fn main(args: &[String]) -> i32 {
        let mut cmd = CommandTranscode::default();
        match cmd.run(args) {
            Ok(()) => ReturnCode::Success as i32,
            Err(FatalError(rc)) => rc,
        }
    }

    fn run(&mut self, args: &[String]) -> CmdResult {
        parse_command_line(
            &mut self.report,
            "ktx transcode",
            "Transcode the KTX file specified as the input-file argument,\n    optionally supercompress the result, and save it as the output-file.",
            args,
            &mut [
                &mut self.transcode,
                &mut self.target,
                &mut self.deflate,
                &mut self.in_out,
                &mut self.generic,
            ],
        )?;
        self.execute_transcode()
    }

    fn execute_transcode(&mut self) -> CmdResult {
        let report = &self.report;
        let input = InputStream::new(&self.in_out.input_filepath, report)?;
        validate_tool_input(&input.data, report)?;

        let texture =
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

        if !texture.is_transcodable() {
            return Err(report.fatal(ReturnCode::InvalidFile, "KTX file is not transcodable."));
        }

        let mut texture = transcode(texture, &mut self.target, report)?;

        if let Some(level) = self.deflate.zstd
            && let Err(e) = texture.deflate_zstd(level)
        {
            return Err(report.fatal(
                ReturnCode::RuntimeError,
                &format!("Zstd deflation failed. KTX Error: {}", ktx_error_string(e)),
            ));
        }
        if let Some(level) = self.deflate.zlib
            && let Err(e) = texture.deflate_zlib(level)
        {
            return Err(report.fatal(
                ReturnCode::RuntimeError,
                &format!("ZLIB deflation failed. KTX Error: {}", ktx_error_string(e)),
            ));
        }

        // Modify KTXwriter metadata
        let writer = format!("{} {}", report.command_name, version(self.generic.testrun));
        let mut value = writer.into_bytes();
        value.push(0); // include the \0
        let _ = texture.kv_data_head.delete_kv_pair(KTX_WRITER_KEY);
        let _ = texture.kv_data_head.add_kv_pair(KTX_WRITER_KEY, &value);

        // Add KTXwriterScParams metadata if supercompression was used
        let _ = texture.kv_data_head.delete_kv_pair(KTX_WRITER_SCPARAMS_KEY);
        if let Some(params) = self.deflate.compress_options.strip_prefix(' ') {
            // Options always contain a leading space; the value excludes it
            // and its terminating NUL (size() bytes from c_str() + 1).
            let mut value = params.as_bytes().to_vec();
            value.push(0);
            let _ = texture
                .kv_data_head
                .add_kv_pair(KTX_WRITER_SCPARAMS_KEY, &value);
        }

        // Save output file
        let output_path = std::path::Path::new(&self.in_out.output_filepath);
        if let Some(parent) = output_path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut output = OutputStream::new(&self.in_out.output_filepath, report)?;
        output.write_ktx2(&mut texture, report)
    }
}
