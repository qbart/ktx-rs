//! Port of tools/ktx/command_deflate.cpp (`ktx deflate`).

use super::command::*;
use super::deflate_utils::OptionsDeflate;
use super::options::{OptionSpec, Options, ParseResult};
use crate::defs::{KTX_WRITER_KEY, KTX_WRITER_SCPARAMS_KEY, SupercmpScheme};
use crate::strings::ktx_supercompression_scheme_string;
use crate::{KTX_TEXTURE_CREATE_LOAD_IMAGE_DATA_BIT, Texture2, ktx_error_string};

/// `CommandDeflate::Options`.
#[derive(Debug, Clone, Default)]
struct OptionsDeflateCmd {
    quiet: bool,
    warnings_as_errors: bool,
}

impl OptionsDeflateCmd {
    const K_QUIET: &'static str = "quiet";
    const K_WARNINGS_AS_ERRORS: &'static str = "warnings-as-errors";
}

impl OptionGroup for OptionsDeflateCmd {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::flag(
            Self::K_QUIET,
            "Don't print warning when input file is already supercompressed.",
        ));
        opts.add(OptionSpec::flag(
            Self::K_WARNINGS_AS_ERRORS,
            "Exit with error when input file is already supercompressed",
        ));
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        self.quiet = args.as_bool(Self::K_QUIET);
        self.warnings_as_errors = args.as_bool(Self::K_WARNINGS_AS_ERRORS);
        if self.quiet && self.warnings_as_errors {
            return Err(report.fatal_usage(&format!(
                "Cannot specify both --{} and --{}.",
                Self::K_QUIET,
                Self::K_WARNINGS_AS_ERRORS
            )));
        }
        Ok(())
    }
}

/// `std::regex_search` of `" ?--(?:zlib|zstd) [1-9][0-9]?"`: (position, length).
fn search_deflate_option(s: &[u8]) -> Option<(usize, usize)> {
    (0..s.len()).find_map(|p| {
        let q = if s.get(p) == Some(&b' ') { p + 1 } else { p };
        let rest = s.get(q..)?;
        if !(rest.starts_with(b"--zlib ") || rest.starts_with(b"--zstd ")) {
            return None;
        }
        let mut end = q + 7;
        if !matches!(s.get(end), Some(b'1'..=b'9')) {
            return None;
        }
        end += 1;
        if s.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        Some((p, end - p))
    })
}

/// `std::regex_search` of `" ?--zcmp ?[1-9]?[0-9]? ?"`: (position, length).
fn search_zcmp_option(s: &[u8]) -> Option<(usize, usize)> {
    (0..s.len()).find_map(|p| {
        let q = if s.get(p) == Some(&b' ') { p + 1 } else { p };
        if !s.get(q..)?.starts_with(b"--zcmp") {
            return None;
        }
        let mut end = q + 6;
        if s.get(end) == Some(&b' ') {
            end += 1;
        }
        if matches!(s.get(end), Some(b'1'..=b'9')) {
            end += 1;
        }
        if s.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if s.get(end) == Some(&b' ') {
            end += 1;
        }
        Some((p, end - p))
    })
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Port of `CommandDeflate`.
#[derive(Debug, Default)]
pub struct CommandDeflate {
    report: Reporter,
    options: OptionsDeflateCmd,
    deflate: OptionsDeflate,
    in_out: OptionsSingleInSingleOut,
    generic: OptionsGeneric,
}

impl CommandDeflate {
    /// Port of `CommandDeflate::main`.
    pub fn main(args: &[String]) -> i32 {
        let mut cmd = CommandDeflate::default();
        match cmd.run(args) {
            Ok(()) => ReturnCode::Success as i32,
            Err(FatalError(rc)) => rc,
        }
    }

    fn run(&mut self, args: &[String]) -> CmdResult {
        parse_command_line(
            &mut self.report,
            "ktx deflate",
            "Deflate (supercompress) the KTX file specified as the input-file\n    and save it as the output-file.",
            args,
            &mut [
                &mut self.options,
                &mut self.deflate,
                &mut self.in_out,
                &mut self.generic,
            ],
        )?;
        // processOptions
        if self.deflate.zstd.is_none() && self.deflate.zlib.is_none() {
            return Err(self.report.fatal_usage(&format!(
                "Must specify --{}  or --{}.",
                OptionsDeflate::K_ZSTD,
                OptionsDeflate::K_ZLIB
            )));
        }
        self.execute_deflate()
    }

    fn execute_deflate(&mut self) -> CmdResult {
        let report = &self.report;
        let input = InputStream::new(&self.in_out.input_filepath, report)?;
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

        let scheme = texture.supercompression_scheme;
        if scheme != SupercmpScheme::NONE {
            if scheme == SupercmpScheme::ZLIB || scheme == SupercmpScheme::ZSTD {
                if !self.options.quiet {
                    report.warning(&format!(
                        "Modifying existing {} supercompression of {}.",
                        ktx_supercompression_scheme_string(scheme),
                        self.in_out.input_filepath
                    ));
                }
            } else {
                return Err(report.fatal(
                    ReturnCode::InvalidFile,
                    &format!(
                        "Cannot further deflate a KTX2 file supercompressed with {}.",
                        ktx_supercompression_scheme_string(scheme)
                    ),
                ));
            }
        }

        if let Some(level) = self.deflate.zstd
            && let Err(e) = texture.deflate_zstd(level)
        {
            return Err(report.fatal(
                ReturnCode::IoFailure,
                &format!("Zstd deflation failed. KTX Error: {}", ktx_error_string(e)),
            ));
        }
        if let Some(level) = self.deflate.zlib
            && let Err(e) = texture.deflate_zlib(level)
        {
            return Err(report.fatal(
                ReturnCode::IoFailure,
                &format!("ZLIB deflation failed. KTX Error: {}", ktx_error_string(e)),
            ));
        }

        // The values looked for are required to be NUL terminated.
        let find_metadata_value = |texture: &Texture2, key: &str| -> Vec<u8> {
            match texture.kv_data_head.find_value(key) {
                Ok(v) => v
                    .get(..v.len().saturating_sub(1))
                    .unwrap_or_default()
                    .to_vec(),
                Err(_) => Vec::new(),
            }
        };
        let update_metadata_value = |texture: &mut Texture2, key: &str, value: &[u8]| {
            let _ = texture.kv_data_head.delete_kv_pair(key);
            let mut v = value.to_vec();
            v.push(0); // +1 to include \0
            let _ = texture.kv_data_head.add_kv_pair(key, &v);
        };

        // KTXwriter and KTXwriterScParams metadata handling: see
        // command_deflate.cpp for the rationale.
        let mut change_writer = true;
        let mut orig_writer_name: Vec<u8> = Vec::new();
        let mut writer_sc_params = find_metadata_value(&texture, KTX_WRITER_SCPARAMS_KEY);
        if !writer_sc_params.is_empty() {
            let writer = find_metadata_value(&texture, KTX_WRITER_KEY);
            if !writer.is_empty() {
                let deflate_option_match = if [
                    &b"ktx create"[..],
                    b"ktx deflate",
                    b"ktx encode",
                    b"ktx transcode",
                ]
                .iter()
                .any(|n| contains(&writer, n))
                {
                    // Writer is member of the ktx suite.
                    search_deflate_option(&writer_sc_params)
                } else {
                    // Writer is not a member of the ktx suite
                    let m = if contains(&writer, b"ktxsc") || contains(&writer, b"toktx") {
                        search_zcmp_option(&writer_sc_params)
                    } else {
                        None
                    };
                    let end = writer
                        .iter()
                        .position(|&c| c == b' ')
                        .unwrap_or(writer.len());
                    orig_writer_name = writer.get(..end).unwrap_or_default().to_vec();
                    m
                };
                // Remove existing deflate option since its value will not
                // apply to the newly deflated data.
                if let Some((pos, len)) = deflate_option_match {
                    writer_sc_params.drain(pos..pos + len);
                }
                // Does ScParams still have data and is the original writer a
                // member of the ktx suite?
                if !writer_sc_params.is_empty() && orig_writer_name.is_empty() {
                    change_writer = false;
                }
            }
        }

        if change_writer {
            // Create or modify KTXwriter metadata.
            let writer = format!("{} {}", report.command_name, version(self.generic.testrun));
            update_metadata_value(&mut texture, KTX_WRITER_KEY, writer.as_bytes());
        }

        // Format new writerScParams. Options always contain a leading space.
        let new_sc_params = self.deflate.compress_options.as_bytes();
        let unspaced = new_sc_params.get(1..).unwrap_or_default();
        if !writer_sc_params.is_empty() {
            if change_writer {
                // Leading space unneeded as this param will be first.
                let mut v = unspaced.to_vec();
                v.extend_from_slice(b" / (from ");
                v.extend_from_slice(&orig_writer_name);
                v.extend_from_slice(b") ");
                v.extend_from_slice(&writer_sc_params);
                writer_sc_params = v;
            } else {
                writer_sc_params.extend_from_slice(new_sc_params);
            }
        } else {
            writer_sc_params = unspaced.to_vec();
        }

        // Add KTXwriterScParams metadata
        update_metadata_value(&mut texture, KTX_WRITER_SCPARAMS_KEY, &writer_sc_params);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflate_option_search() {
        assert_eq!(search_deflate_option(b"--zstd 18"), Some((0, 9)));
        assert_eq!(
            search_deflate_option(b"--encode uastc --zlib 8 x"),
            Some((14, 9))
        );
        assert_eq!(search_deflate_option(b"--zstd 0 --zlib 99"), Some((8, 10)));
        assert_eq!(search_deflate_option(b"--zstd"), None);
        assert_eq!(search_zcmp_option(b"--t2 --zcmp 8"), Some((4, 9)));
        assert_eq!(search_zcmp_option(b"--zcmp --uastc 2"), Some((0, 7)));
        assert_eq!(search_zcmp_option(b"--uastc"), None);
    }
}
