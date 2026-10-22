//! Port of tools/ktx/command.h / command.cpp: return codes, reporting, the
//! shared option groups and input handling.
//!
//! C++ `FatalError` exceptions become `Err(FatalError)`; the message is
//! printed to stderr when the error is created, like the C++ `Reporter`.

use std::io::{Read, Write};

use super::options::{OptionSpec, Options, ParseResult};
use crate::version::{LIBKTX_DEFAULT_VERSION, LIBKTX_VERSION};

/// Port of `ReturnCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ReturnCode {
    Success = 0,
    InvalidArguments = 1,
    IoFailure = 2,
    InvalidFile = 3,
    RuntimeError = 4,
    NotSupported = 5,
    NotImplemented = 6,
    DifferenceFound = 7,
}

/// Port of `FatalError`: the desired process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FatalError(pub i32);

impl From<ReturnCode> for FatalError {
    fn from(rc: ReturnCode) -> FatalError {
        FatalError(rc as i32)
    }
}

/// Result of a command step.
pub type CmdResult<T = ()> = Result<T, FatalError>;

/// Writes to stdout, ignoring errors (`fmt::print`).
pub fn print_out(bytes: &[u8]) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(bytes);
}

/// Writes to stderr, ignoring errors (`fmt::print(std::cerr, ...)`).
pub fn print_err(bytes: &[u8]) {
    let mut e = std::io::stderr().lock();
    let _ = e.write_all(bytes);
}

/// Flushes stdout.
pub fn flush_out() {
    let _ = std::io::stdout().lock().flush();
}

/// Port of `version(testrun)`.
pub fn version(testrun: bool) -> &'static str {
    if testrun {
        LIBKTX_DEFAULT_VERSION
    } else {
        LIBKTX_VERSION
    }
}

/// Port of `Reporter`.
#[derive(Debug, Clone, Default)]
pub struct Reporter {
    pub command_name: String,
    pub command_description: String,
}

impl Reporter {
    /// `warning`.
    pub fn warning(&self, msg: &str) {
        print_err(format!("{} warning: {msg}\n", self.command_name).as_bytes());
    }

    /// `error`.
    pub fn error(&self, msg: &str) {
        print_err(format!("{} error: {msg}\n", self.command_name).as_bytes());
    }

    /// `fatal`: prints and returns the error to propagate.
    pub fn fatal(&self, rc: ReturnCode, msg: &str) -> FatalError {
        print_err(format!("{} fatal: {msg}\n", self.command_name).as_bytes());
        rc.into()
    }

    /// `fatal_usage`.
    pub fn fatal_usage(&self, msg: &str) -> FatalError {
        print_err(
            format!(
                "{} fatal: {msg} See '{} --help'.\n",
                self.command_name, self.command_name
            )
            .as_bytes(),
        );
        ReturnCode::InvalidArguments.into()
    }
}

/// An option group (`OptionsGeneric`, `OptionsFormat`, ...).
pub trait OptionGroup {
    fn init(&mut self, opts: &mut Options);
    fn process(&mut self, opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult;
}

/// Port of `Command::parseCommandLine`. `groups` are initialised left to right
/// and processed right to left (`Combine`).
pub fn parse_command_line(
    report: &mut Reporter,
    name: &str,
    desc: &str,
    args: &[String],
    groups: &mut [&mut dyn OptionGroup],
) -> CmdResult {
    report.command_name = name.to_string();
    report.command_description = desc.to_string();
    let mut opts = Options::new(name);
    for g in groups.iter_mut() {
        g.init(&mut opts);
    }
    let parsed = match opts.parse(args) {
        Ok(p) => p,
        Err(e) => return Err(report.fatal_usage(&format!("{}.", e.0))),
    };
    for g in groups.iter_mut().rev() {
        g.process(&opts, &parsed, report)?;
    }
    Ok(())
}

/// Port of `OptionsGeneric`.
#[derive(Debug, Clone, Default)]
pub struct OptionsGeneric {
    pub testrun: bool,
}

impl OptionGroup for OptionsGeneric {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::flag(
            "h,help",
            "Print this usage message and exit",
        ));
        opts.add(OptionSpec::flag(
            "v,version",
            "Print the version number of this program and exit",
        ));
        opts.add(OptionSpec::flag(
            "testrun",
            "Indicates test run. If enabled the tool will produce deterministic output whenever possible",
        ));
    }

    fn process(&mut self, opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        self.testrun = args.as_bool("testrun");
        if args.count("help") > 0 {
            print_out(
                format!("{}: {}\n", report.command_name, report.command_description).as_bytes(),
            );
            print_out(opts.help().as_bytes());
            return Err(ReturnCode::Success.into());
        }
        if args.count("version") > 0 {
            print_out(
                format!("{} version: {}\n", opts.program(), version(self.testrun)).as_bytes(),
            );
            return Err(ReturnCode::Success.into());
        }
        Ok(())
    }
}

/// Port of `OutputFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    JsonMini,
}

/// Port of `OptionsFormat`.
#[derive(Debug, Clone, Default)]
pub struct OptionsFormat {
    pub format: OutputFormat,
}

impl OptionGroup for OptionsFormat {
    fn init(&mut self, opts: &mut Options) {
        opts.add(
            OptionSpec::value(
                "f,format",
                "Specifies the report output format. Possible options are:\n  text: Human readable text based format\n  json: Formatted JSON\n  mini-json: Minified JSON\n",
                "text|json|mini-json",
            )
            .default_value("text"),
        );
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        let format_str = args.get("format").unwrap_or_default().to_lowercase();
        self.format = match format_str.as_str() {
            "text" => OutputFormat::Text,
            "json" => OutputFormat::Json,
            "mini-json" => OutputFormat::JsonMini,
            _ => return Err(report.fatal_usage(&format!("Unsupported format: \"{format_str}\"."))),
        };
        Ok(())
    }
}

/// Port of `OptionsSingleIn`.
#[derive(Debug, Clone, Default)]
pub struct OptionsSingleIn {
    pub input_filepath: String,
}

impl OptionGroup for OptionsSingleIn {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::flag(
            "stdin",
            "Use stdin as the input file. (Using a single dash '-' as the input file has the same effect)",
        ));
        opts.add(OptionSpec::value(
            "i,input-file",
            "The input file. Using a single dash '-' as the input file will use stdin.",
            "filepath",
        ));
        opts.parse_positional(&["input-file"]);
        opts.positional_help("<input-file>");
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        if !args.unmatched().is_empty() {
            return Err(report.fatal_usage("Too many filenames specified."));
        }
        let n = args.count("stdin") + args.count("input-file");
        if n == 0 {
            return Err(report.fatal_usage(
                "Missing input file. Either <input-file> or --stdin must be specified.",
            ));
        }
        if n > 1 {
            return Err(report.fatal_usage(
                "Conflicting options: Only one can be specified from <input-file> and --stdin.",
            ));
        }
        self.input_filepath = if args.count("stdin") > 0 {
            "-".to_string()
        } else {
            args.get("input-file").unwrap_or_default().to_string()
        };
        Ok(())
    }
}

/// Port of `InputStream`: the whole input (file or stdin) in memory.
#[derive(Debug, Clone)]
pub struct InputStream {
    pub filepath: String,
    pub data: Vec<u8>,
}

impl InputStream {
    /// Opens `filepath` (`-` reads all of stdin).
    pub fn new(filepath: &str, report: &Reporter) -> CmdResult<InputStream> {
        let data = if filepath == "-" {
            let mut v = Vec::new();
            let _ = std::io::stdin().lock().read_to_end(&mut v);
            v
        } else {
            match std::fs::read(filepath) {
                Ok(d) => d,
                Err(e) => {
                    return Err(report.fatal(
                        ReturnCode::IoFailure,
                        &format!(
                            "Could not open input file \"{filepath}\": {}.",
                            super::utility::io_error_message(&e)
                        ),
                    ));
                }
            }
        };
        Ok(InputStream {
            filepath: filepath.to_string(),
            data,
        })
    }
}
