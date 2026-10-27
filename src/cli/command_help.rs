// SPDX-FileCopyrightText: 2010-2026 Mark Callow, The Khronos Group Inc. and KTX-Software contributors
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from KTX-Software and modified; see NOTICE.md.
//! Port of tools/ktx/command_help.cpp (`ktx help`).

use super::command::*;
use super::options::{OptionSpec, Options, ParseResult};

/// Port of `OptionsHelp`.
#[derive(Debug, Clone, Default)]
struct OptionsHelp {
    command: Option<String>,
}

const COMMAND_TABLE: [&str; 10] = [
    "convert",
    "create",
    "deflate",
    "encode",
    "extract",
    "transcode",
    "info",
    "validate",
    "compare",
    "help",
];

impl OptionGroup for OptionsHelp {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::value(
            "command",
            "The command for which usage should be displayed.",
            "",
        ));
        opts.parse_positional(&["command"]);
        opts.positional_help("<command>");
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        if args.count("command") > 0 {
            let command = args.get("command").unwrap_or_default().to_lowercase();
            if !COMMAND_TABLE.contains(&command.as_str()) {
                return Err(
                    report.fatal_usage(&format!("Invalid command specified: \"{command}\"."))
                );
            }
            self.command = Some(command);
        }
        Ok(())
    }
}

/// Port of `CommandHelp`.
#[derive(Debug, Default)]
pub struct CommandHelp {
    report: Reporter,
    help: OptionsHelp,
    generic: OptionsGeneric,
}

impl CommandHelp {
    /// Port of `CommandHelp::main`.
    pub fn main(args: &[String]) -> i32 {
        let mut cmd = CommandHelp::default();
        match cmd.run(args) {
            Ok(()) => ReturnCode::Success as i32,
            Err(FatalError(rc)) => rc,
        }
    }

    fn run(&mut self, args: &[String]) -> CmdResult {
        parse_command_line(
            &mut self.report,
            "ktx help",
            "Displays the man page of a specific ktx command specified as the command argument.\nIf the command option is missing the main ktx tool man page will be displayed.",
            args,
            &mut [&mut self.help, &mut self.generic],
        )?;
        self.execute_help()
    }

    /// Port of `executeHelp`: opens the html page (Windows) or the man page
    /// (`man`, preferring the page installed next to the executable).
    fn execute_help(&self) -> CmdResult {
        let (sep, command) = match &self.help.command {
            Some(c) => ("_", c.as_str()),
            None => ("", ""),
        };
        let executable_path = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .map_err(|e| {
                self.report.fatal(
                    ReturnCode::RuntimeError,
                    &format!(
                        "Failed to determine executable path: ERROR {}",
                        e.raw_os_error().unwrap_or(0)
                    ),
                )
            })?;
        let executable_dir = executable_path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        if cfg!(windows) {
            let page = format!(
                "{executable_dir}\\..\\share\\doc\\KTX-Software\\html\\ktxtools\\ktx{sep}{command}.html"
            );
            let status = std::process::Command::new("cmd")
                .args(["/C", "start", "", &page])
                .status();
            if !status.is_ok_and(|s| s.success()) {
                return Err(self.report.fatal(
                    ReturnCode::RuntimeError,
                    "Failed to open the html documentation: ERROR 0",
                ));
            }
        } else {
            let man_file = format!("{executable_dir}/../share/man/man1/ktx{sep}{command}.1");
            let system_command = if std::path::Path::new(&man_file).exists() {
                // We have relative access to the man file, prioritize opening it
                // that way to support custom install locations
                format!("man \"{man_file}\"")
            } else {
                format!("man ktx{sep}{command}")
            };
            let _ = std::process::Command::new("sh")
                .arg("-c")
                .arg(&system_command)
                .status();
        }
        Ok(())
    }
}
