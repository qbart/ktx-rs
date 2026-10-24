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
) -> CmdResult<ParseResult> {
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
    Ok(parsed)
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

/// Port of `OptionsSingleInSingleOut<OUTPUT_FILE_OPTIONAL>`.
#[derive(Debug, Clone, Default)]
pub struct OptionsSingleInSingleOut<const OUTPUT_FILE_OPTIONAL: bool = false> {
    pub input_filepath: String,
    pub output_filepath: String,
}

impl<const OUTPUT_FILE_OPTIONAL: bool> OptionGroup
    for OptionsSingleInSingleOut<OUTPUT_FILE_OPTIONAL>
{
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::flag(
            "stdin",
            "Use stdin as the input file. (Using a single dash '-' as the input file has the same effect)",
        ));
        opts.add(OptionSpec::flag(
            "stdout",
            "Use stdout as the output file. (Using a single dash '-' as the output file has the same effect)",
        ));
        opts.add(OptionSpec::value(
            "i,input-file",
            "The input file. Using a single dash '-' as the input file will use stdin.",
            "filepath",
        ));
        opts.add(OptionSpec::value(
            "o,output-file",
            "The output file. Using a single dash '-' as the output file will use stdout.",
            "filepath",
        ));
        opts.parse_positional(&["input-file", "output-file"]);
        opts.positional_help("<input-file> <output-file>");
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        if !args.unmatched().is_empty() {
            return Err(report.fatal_usage("Too many filenames specified."));
        }
        let n_in = args.count("stdin") + args.count("input-file");
        if n_in == 0 {
            return Err(report.fatal_usage(
                "Missing input file. Either <input-file> or --stdin must be specified.",
            ));
        }
        if n_in > 1 {
            return Err(report.fatal_usage(
                "Conflicting options: Only one can be specified from <input-file> and --stdin.",
            ));
        }
        let n_out = args.count("stdout") + args.count("output-file");
        if n_out == 0 && !OUTPUT_FILE_OPTIONAL {
            return Err(report.fatal_usage(
                "Missing output file. Either <output-file> or --stdout must be specified.",
            ));
        }
        if n_out > 1 {
            return Err(report.fatal_usage(
                "Conflicting options: Only one can be specified from <output-file> and --stdout.",
            ));
        }
        self.input_filepath = if args.count("stdin") > 0 {
            "-".to_string()
        } else {
            args.get("input-file").unwrap_or_default().to_string()
        };
        if args.count("stdout") > 0 {
            self.output_filepath = "-".to_string();
        } else if args.count("output-file") > 0 {
            self.output_filepath = args.get("output-file").unwrap_or_default().to_string();
        }
        Ok(())
    }
}

/// Port of `OptionsMultiInSingleOut`.
#[derive(Debug, Clone, Default)]
pub struct OptionsMultiInSingleOut {
    pub input_filepaths: Vec<String>,
    pub output_filepath: String,
}

impl OptionsMultiInSingleOut {
    /// `loadFileList`: one name per line, leading spaces and trailing
    /// spaces / newlines stripped. `@@list` names are relative to the list.
    fn load_file_list(
        f: &str,
        mut relativize: bool,
        files: &mut Vec<String>,
        report: &Reporter,
    ) -> CmdResult {
        let list_name = f.get(if relativize { 2 } else { 1 }..).unwrap_or_default();
        let text = match std::fs::read(list_name) {
            Ok(t) => t,
            Err(e) => {
                return Err(report.fatal(
                    ReturnCode::RuntimeError,
                    &format!(
                        "Opening filename list: \"{list_name}\" failed: {}\n",
                        super::utility::io_error_message(&e)
                    ),
                ));
            }
        };
        let mut dirname = String::new();
        if relativize {
            match list_name.rfind('/') {
                None => relativize = false,
                Some(end) => dirname = list_name.get(..end + 1).unwrap_or_default().to_string(),
            }
        }
        // fgets with a 4096 byte buffer: longer lines are split.
        for line in text.split_inclusive(|&c| c == b'\n') {
            for chunk in line.chunks(4095) {
                let mut s: &[u8] = chunk;
                while let [b' ', rest @ ..] = s {
                    s = rest;
                }
                while let [rest @ .., b' ' | b'\n' | b'\r'] = s {
                    s = rest;
                }
                if !s.is_empty() {
                    let name = String::from_utf8_lossy(s);
                    if relativize {
                        files.push(format!("{dirname}{name}"));
                    } else {
                        files.push(name.into_owned());
                    }
                }
            }
        }
        Ok(())
    }
}

impl OptionGroup for OptionsMultiInSingleOut {
    fn init(&mut self, opts: &mut Options) {
        opts.add(OptionSpec::flag(
            "stdin",
            "Use stdin as the first input file. (Using a single dash '-' as the first input file has the same effect)",
        ));
        opts.add(OptionSpec::flag(
            "stdout",
            "Use stdout as the output file. (Using a single dash '-' as the output file has the same effect)",
        ));
        opts.add(
            OptionSpec::value(
                "files",
                "Input/output files. Last file specified will be used as output. Using a single dash '-' as an input or output file will use stdin/stdout. A filepath prefixed with @ is read as a file name listing file. Listing text files specify which actual files to process, one file name per line. Names can be absolute paths or relative to the current directory when the application is run. If the file is prefixed with @b @@ the names must be relative to the listing file.",
                "<filepath>",
            )
            .container(),
        );
        opts.parse_positional(&["files"]);
        opts.positional_help("<input-file...> <output-file>");
    }

    fn process(&mut self, _opts: &Options, args: &ParseResult, report: &Reporter) -> CmdResult {
        let mut files: Vec<String> = Vec::new();
        if args.count("stdin") > 0 {
            files.push("-".into());
        }
        files.extend(args.get_all("files").iter().cloned());
        if args.count("stdout") > 0 {
            files.push("-".into());
        }
        if files.is_empty() {
            return Err(report.fatal_usage("Input and output files must be specified."));
        }
        if files.len() < 2 {
            return Err(report.fatal_usage(&format!(
                "{} file must be specified.",
                if args.count("stdout") == 0 {
                    "Output"
                } else {
                    "Input"
                }
            )));
        }
        self.output_filepath = files.pop().unwrap_or_default();
        for f in files {
            if f.starts_with('@') {
                let rel = f.as_bytes().get(1) == Some(&b'@');
                Self::load_file_list(&f, rel, &mut self.input_filepaths, report)?;
            } else {
                self.input_filepaths.push(f);
            }
        }
        if self.input_filepaths.iter().filter(|f| *f == "-").count() > 1 {
            return Err(
                report.fatal_usage("'-' or --stdin as input file was specified more than once.")
            );
        }
        Ok(())
    }
}

/// Port of `OutputStream`: a created (truncated) file or stdout.
#[derive(Debug)]
pub struct OutputStream {
    pub filepath: String,
    file: Option<std::fs::File>,
    remove_at_destruct: bool,
}

impl OutputStream {
    /// Creates `filepath` (`-` is stdout).
    pub fn new(filepath: &str, report: &Reporter) -> CmdResult<OutputStream> {
        let file = if filepath == "-" {
            None
        } else {
            match std::fs::File::create(filepath) {
                Ok(f) => Some(f),
                Err(e) => {
                    return Err(report.fatal(
                        ReturnCode::IoFailure,
                        &format!(
                            "Could not open output file \"{filepath}\": {}",
                            super::utility::io_error_message(&e)
                        ),
                    ));
                }
            }
        };
        Ok(OutputStream {
            filepath: filepath.to_string(),
            file,
            remove_at_destruct: false,
        })
    }

    pub fn is_stdout(&self) -> bool {
        self.file.is_none()
    }

    /// `removeOnDestruct`.
    pub fn remove_on_destruct(&mut self) {
        self.remove_at_destruct = true;
    }

    /// `write`.
    pub fn write(&mut self, data: &[u8], report: &Reporter) -> CmdResult {
        let result = match self.file.as_mut() {
            Some(f) => f.write_all(data),
            None => std::io::stdout().lock().write_all(data),
        };
        result.map_err(|e| {
            report.fatal(
                ReturnCode::IoFailure,
                &format!(
                    "Failed to write output file \"{}\": {}.",
                    super::utility::fmt_out_file(&self.filepath),
                    super::utility::io_error_message(&e)
                ),
            )
        })
    }

    /// `writeKTX2`.
    pub fn write_ktx2(&mut self, texture: &mut crate::Texture2, report: &Reporter) -> CmdResult {
        match texture.write_to_memory() {
            Ok(bytes) => self.write(&bytes, report),
            Err(e) => {
                if !self.is_stdout() {
                    let _ = std::fs::remove_file(&self.filepath);
                }
                Err(report.fatal(
                    ReturnCode::IoFailure,
                    &format!(
                        "Failed to write KTX file \"{}\": {}.",
                        super::utility::fmt_out_file(&self.filepath),
                        crate::ktx_error_string(e)
                    ),
                ))
            }
        }
    }
}

impl Drop for OutputStream {
    fn drop(&mut self) {
        if let Some(f) = self.file.take() {
            drop(f);
            if self.remove_at_destruct {
                let _ = std::fs::remove_file(&self.filepath);
            }
        }
    }
}

/// A `std::exception` reaching a command's `main`: prints
/// `{name} fatal: {what}` and exits with `RUNTIME_ERROR`.
pub fn runtime_error(report: &Reporter, what: &str) -> FatalError {
    print_err(format!("{} fatal: {what}\n", report.command_name).as_bytes());
    ReturnCode::RuntimeError.into()
}

/// Port of `validateToolInput`: validation issues go to stderr; a failed
/// validation exits with its return code.
pub fn validate_tool_input(data: &[u8], report: &Reporter) -> CmdResult {
    let result = super::validate::validate_memory(data, false, false, &mut |issue| {
        print_err(
            format!(
                "{}-{:04}: {}\n",
                issue.type_.as_str(),
                issue.id,
                issue.message
            )
            .as_bytes(),
        );
        let mut details = b"    ".to_vec();
        details.extend_from_slice(&issue.details);
        details.push(b'\n');
        print_err(&details);
    });
    match result {
        Ok(0) => Ok(()),
        Ok(rc) => Err(FatalError(rc)),
        Err(what) => Err(runtime_error(report, &what)),
    }
}
