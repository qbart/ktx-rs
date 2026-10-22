//! Port of tools/ktx/command_info.cpp (`ktx info`).

use super::command::*;
use super::command_validate::{write_issue_json, write_issue_text};
use super::utility::{PrintIndent, fmt_in_file};
use super::validate::validate_memory;
use crate::defs::{KTX_IDENTIFIER_REF, KTX2_HEADER_SIZE, KTX2_IDENTIFIER_REF};
use crate::error::{KtxResult, ktx_error_string};
use crate::info::{
    ktx_write_ktx1_info_text_for_stream, ktx_write_ktx2_info_json_for_stream,
    ktx_write_ktx2_info_text_for_stream,
};
use crate::stream::MemStream;

/// Port of `CommandInfo`.
#[derive(Debug, Default)]
pub struct CommandInfo {
    report: Reporter,
    format: OptionsFormat,
    single_in: OptionsSingleIn,
    generic: OptionsGeneric,
}

impl CommandInfo {
    /// Port of `CommandInfo::main`.
    pub fn main(args: &[String]) -> i32 {
        let mut cmd = CommandInfo::default();
        match cmd.run(args) {
            Ok(()) => ReturnCode::Success as i32,
            Err(FatalError(rc)) => rc,
        }
    }

    fn run(&mut self, args: &[String]) -> CmdResult {
        parse_command_line(
            &mut self.report,
            "ktx info",
            "Prints information about the KTX v1 or v2 file specified as the input-file argument.\n    The command implicitly calls validate for v2 files and prints any found errors\n    and warnings to stdout. For KTX v1 files only text format output is supported.",
            args,
            &mut [&mut self.format, &mut self.single_in, &mut self.generic],
        )?;
        self.execute_info()
    }

    fn execute_info(&mut self) -> CmdResult {
        let input = InputStream::new(&self.single_in.input_filepath, &self.report)?;

        let mut v2 = true;
        let result = if input.data.get(..12) == Some(&KTX_IDENTIFIER_REF[..]) {
            if self.format.format != OutputFormat::Text {
                return Err(self
                    .report
                    .fatal_usage("JSON output formats are not supported for KTX v1 files."));
            }
            v2 = false;
            let mut out = Vec::new();
            let result = match MemStream::construct_ro(&input.data) {
                Ok(mut stream) => ktx_write_ktx1_info_text_for_stream(&mut out, &mut stream),
                Err(e) => Err(e),
            };
            print_out(&out);
            result
        } else {
            match self.format.format {
                OutputFormat::Text => self.print_info_text(&input.data)?,
                OutputFormat::Json => self.print_info_json(&input.data, false)?,
                OutputFormat::JsonMini => self.print_info_json(&input.data, true)?,
            }
        };

        if let Err(e) = result {
            return Err(self.report.fatal(
                ReturnCode::InvalidFile,
                &format!(
                    "Failed to process KTX {} file \"{}\": {}",
                    if v2 { "v2" } else { "v1" },
                    fmt_in_file(&self.single_in.input_filepath),
                    ktx_error_string(e)
                ),
            ));
        }
        Ok(())
    }

    /// Port of `printInfoText`.
    fn print_info_text(&self, data: &[u8]) -> CmdResult<KtxResult<()>> {
        let mut messages = Vec::new();
        let result = validate_memory(data, false, false, &mut |issue| {
            write_issue_text(&mut messages, issue);
        });
        let validation_result = self.runtime(result)?;

        let mut out = Vec::new();
        let status = if validation_result == 0 {
            "successful"
        } else {
            "failed"
        };
        out.extend_from_slice(format!("Validation {status}\n").as_bytes());
        if !messages.is_empty() {
            out.push(b'\n');
            out.extend_from_slice(&messages);
        }
        out.push(b'\n');

        let result = match MemStream::construct_ro(data) {
            Ok(mut stream) => ktx_write_ktx2_info_text_for_stream(&mut out, &mut stream),
            Err(e) => Err(e),
        };
        print_out(&out);
        Ok(if validation_result == 0 {
            result
        } else {
            Ok(())
        })
    }

    /// Port of `printInfoJSON`.
    fn print_info_json(&self, data: &[u8], minified: bool) -> CmdResult<KtxResult<()>> {
        let base_indent = 0;
        let indent_width = if minified { 0 } else { 4 };
        let space = if minified { "" } else { " " };
        let nl = if minified { "" } else { "\n" };
        let pi = PrintIndent {
            indent_base: base_indent,
            indent_width,
        };

        let mut messages = Vec::new();
        let mut first = true;
        let result = validate_memory(data, false, false, &mut |issue| {
            write_issue_json(&mut messages, pi, &mut first, space, nl, issue);
        });
        let validation_result = self.runtime(result)?;

        // Workaround detection to decide if ktx will produce any output into
        // the json to eliminate a trailing comma
        let identifier_is_correct = data.get(..12) == Some(&KTX2_IDENTIFIER_REF[..]);
        let ktx_will_print_output = identifier_is_correct && data.len() >= KTX2_HEADER_SIZE;
        let comma = if ktx_will_print_output { "," } else { "" };

        let mut out = Vec::new();
        pi.print(&mut out, 0, format!("{{{nl}").as_bytes());
        pi.print(
            &mut out,
            1,
            format!("\"$schema\":{space}\"https://schema.khronos.org/ktx/info_v1.json\",{nl}")
                .as_bytes(),
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
            pi.print(&mut out, 1, format!("]{comma}{nl}").as_bytes());
        } else {
            pi.print(
                &mut out,
                1,
                format!("\"messages\":{space}[]{comma}{nl}").as_bytes(),
            );
        }

        let result = match MemStream::construct_ro(data) {
            Ok(mut stream) => ktx_write_ktx2_info_json_for_stream(
                &mut out,
                &mut stream,
                base_indent as u32 + 1,
                indent_width as u32,
                minified,
            ),
            Err(e) => Err(e),
        };
        pi.print(&mut out, 0, format!("}}{nl}").as_bytes());
        print_out(&out);
        Ok(if validation_result == 0 {
            result
        } else {
            Ok(())
        })
    }

    /// A `std::exception` escaping validation: `{name} fatal: {what}`, exit 4.
    fn runtime(&self, result: Result<i32, String>) -> CmdResult<i32> {
        result.map_err(|what| {
            print_err(format!("{} fatal: {what}\n", self.report.command_name).as_bytes());
            FatalError(ReturnCode::RuntimeError as i32)
        })
    }
}
