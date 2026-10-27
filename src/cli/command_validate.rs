// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of tools/ktx/command_validate.cpp (`ktx validate`).

use super::command::*;
use super::options::{OptionSpec, Options, ParseResult};
use super::utility::{PrintIndent, escape_json_copy};
use super::validate::{ValidationReport, validate_memory};

/// `CommandValidate::OptionsValidate`.
#[derive(Debug, Clone, Default)]
struct OptionsValidate {
    warnings_as_errors: bool,
    gltf_basisu: bool,
}

impl OptionGroup for OptionsValidate {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::flag(
            "e,warnings-as-errors",
            "Treat warnings as errors.",
        ));
        opts.add(OptionSpec::flag(
            "g,gltf-basisu",
            "Check compatibility with KHR_texture_basisu glTF extension.",
        ));
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, _report: &Reporter) -> CmdResult {
        self.warnings_as_errors = args.as_bool("warnings-as-errors");
        self.gltf_basisu = args.as_bool("gltf-basisu");
        Ok(())
    }
}

/// Port of `CommandValidate`.
#[derive(Debug, Default)]
pub struct CommandValidate {
    report: Reporter,
    validate: OptionsValidate,
    format: OptionsFormat,
    single_in: OptionsSingleIn,
    generic: OptionsGeneric,
}

/// Prints one issue as text (`{type}-{id:04}: {message}\n    {details}\n`).
pub fn write_issue_text(out: &mut Vec<u8>, issue: &ValidationReport) {
    out.extend_from_slice(
        format!(
            "{}-{:04}: {}\n",
            issue.type_.as_str(),
            issue.id,
            issue.message
        )
        .as_bytes(),
    );
    out.extend_from_slice(b"    ");
    out.extend_from_slice(&issue.details);
    out.push(b'\n');
}

/// Prints one issue as a JSON message object (without its closing brace).
pub fn write_issue_json(
    out: &mut Vec<u8>,
    pi: PrintIndent,
    first: &mut bool,
    space: &str,
    nl: &str,
    issue: &ValidationReport,
) {
    if !std::mem::replace(first, false) {
        pi.print(out, 2, format!("}},{nl}").as_bytes());
    }
    pi.print(out, 2, format!("{{{nl}").as_bytes());
    pi.print(
        out,
        3,
        format!("\"id\":{space}{},{nl}", issue.id).as_bytes(),
    );
    pi.print(
        out,
        3,
        format!("\"type\":{space}\"{}\",{nl}", issue.type_.as_str()).as_bytes(),
    );
    pi.print(out, 3, format!("\"message\":{space}\"").as_bytes());
    out.extend_from_slice(&escape_json_copy(issue.message.as_bytes()));
    out.extend_from_slice(format!("\",{nl}").as_bytes());
    pi.print(out, 3, format!("\"details\":{space}\"").as_bytes());
    out.extend_from_slice(&escape_json_copy(&issue.details));
    out.extend_from_slice(format!("\"{nl}").as_bytes());
}

impl CommandValidate {
    /// Port of `CommandValidate::main`.
    pub fn main(args: &[String]) -> i32 {
        let mut cmd = CommandValidate::default();
        match cmd.run(args) {
            Ok(()) => ReturnCode::Success as i32,
            Err(FatalError(rc)) => rc,
        }
    }

    fn run(&mut self, args: &[String]) -> CmdResult {
        parse_command_line(
            &mut self.report,
            "ktx validate",
            "Validates the Khronos texture format version 2 (KTX2) file specified\n    as the input-file argument. It prints any found errors and warnings to stdout.",
            args,
            &mut [
                &mut self.validate,
                &mut self.format,
                &mut self.single_in,
                &mut self.generic,
            ],
        )?;
        self.execute_validate()
    }

    fn execute_validate(&mut self) -> CmdResult {
        let input = InputStream::new(&self.single_in.input_filepath, &self.report)?;
        let (wae, gltf) = (self.validate.warnings_as_errors, self.validate.gltf_basisu);

        match self.format.format {
            OutputFormat::Text => {
                let mut messages = Vec::new();
                let result = validate_memory(&input.data, wae, gltf, &mut |issue| {
                    write_issue_text(&mut messages, issue);
                });
                let validation_result = self.runtime(result)?;
                if !messages.is_empty() {
                    let status = if validation_result == 0 {
                        "successful"
                    } else {
                        "failed"
                    };
                    print_out(format!("Validation {status}\n\n").as_bytes());
                    print_out(&messages);
                }
                if validation_result != 0 {
                    return Err(ReturnCode::InvalidFile.into());
                }
            }
            OutputFormat::Json | OutputFormat::JsonMini => {
                let json = self.format.format == OutputFormat::Json;
                let indent_width = if json { 4 } else { 0 };
                let space = if json { " " } else { "" };
                let nl = if json { "\n" } else { "" };
                let pi = PrintIndent {
                    indent_base: 0,
                    indent_width,
                };

                let mut messages = Vec::new();
                let mut first = true;
                let result = validate_memory(&input.data, wae, gltf, &mut |issue| {
                    write_issue_json(&mut messages, pi, &mut first, space, nl, issue);
                });
                let validation_result = self.runtime(result)?;

                let mut out = Vec::new();
                pi.print(&mut out, 0, format!("{{{nl}").as_bytes());
                pi.print(
                    &mut out,
                    1,
                    format!("\"$schema\":{space}\"https://schema.khronos.org/ktx/validate_v0.json\",{nl}").as_bytes(),
                );
                pi.print(
                    &mut out,
                    1,
                    format!("\"valid\":{space}{},{nl}", validation_result == 0).as_bytes(),
                );
                if !first {
                    pi.print(&mut out, 1, format!("\"messages\":{space}[{nl}").as_bytes());
                    out.extend_from_slice(&messages);
                    pi.print(&mut out, 2, format!("}}{nl}").as_bytes());
                    pi.print(&mut out, 1, format!("]{nl}").as_bytes());
                } else {
                    pi.print(
                        &mut out,
                        1,
                        format!("\"messages\":{space}[]{nl}").as_bytes(),
                    );
                }
                pi.print(&mut out, 0, format!("}}{nl}").as_bytes());
                print_out(&out);

                if validation_result != 0 {
                    return Err(ReturnCode::InvalidFile.into());
                }
            }
        }
        Ok(())
    }

    /// A `std::exception` escaping validation: `{name} fatal: {what}`, exit 4.
    fn runtime(&self, result: Result<i32, String>) -> CmdResult<i32> {
        result.map_err(|what| runtime_error(&self.report, &what))
    }
}
