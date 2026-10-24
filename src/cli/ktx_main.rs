//! Port of tools/ktx/ktx_main.cpp: the `ktx` command dispatcher.

use super::command::*;
use super::command_compare::CommandCompare;
use super::command_create::CommandCreate;
use super::command_deflate::CommandDeflate;
use super::command_extract::CommandExtract;
use super::command_help::CommandHelp;
use super::command_info::CommandInfo;
use super::command_transcode::CommandTranscode;
use super::command_validate::CommandValidate;
use super::options::{OptionSpec, Options};

fn print_usage(options: &Options) -> String {
    let mut s = options.help();
    s.push('\n');
    s.push_str("Available commands:\n");
    s.push_str("  convert    Convert another texture file type to a KTX2 file\n");
    s.push_str("  create     Create a KTX2 file from various input files\n");
    s.push_str("  deflate    Deflate (supercompress) a KTX2 file\n");
    s.push_str("  encode     Encode a KTX2 file\n");
    s.push_str("  extract    Extract selected images from a KTX2 file\n");
    s.push_str("  transcode  Transcode a KTX2 file\n");
    s.push_str("  info       Print information about a KTX2 file\n");
    s.push_str("  validate   Validate a KTX2 file\n");
    s.push_str("  compare    Compare two KTX2 files\n");
    s.push_str("  help       Display help information about the ktx tool\n");
    s.push('\n');
    s.push_str("For detailed usage and description of each subcommand use 'ktx help <command>'\nor 'ktx <command> --help'\n");
    s
}

/// Port of `Tools::main` (no or unknown sub-command).
fn tools_main(args: &[String]) -> i32 {
    let mut options = Options::new("ktx");
    options.custom_help("[<command>] [OPTION...]");
    options.add(OptionSpec::flag(
        "h,help",
        "Print this usage message and exit",
    ));
    options.add(OptionSpec::flag(
        "v,version",
        "Print the version number of this program and exit",
    ));
    options.add(OptionSpec::flag(
        "testrun",
        "Indicates test run. If enabled the tool will produce deterministic output whenever possible",
    ));
    options.allow_unrecognised_options();

    let parsed = match options.parse(args) {
        Ok(p) => p,
        Err(e) => {
            print_err(format!("{}: {}\n", options.program(), e.0).as_bytes());
            print_err(print_usage(&options).as_bytes());
            return ReturnCode::InvalidArguments as i32;
        }
    };
    let testrun = parsed.as_bool("testrun");
    if parsed.count("help") > 0 {
        print_out(
            format!(
                "{}: Unified CLI frontend for the KTX-Software library with sub-commands for specific operations.\n",
                options.program()
            )
            .as_bytes(),
        );
        print_out(print_usage(&options).as_bytes());
        return ReturnCode::Success as i32;
    }
    if parsed.count("version") > 0 {
        print_out(format!("{} version: {}\n", options.program(), version(testrun)).as_bytes());
        return ReturnCode::Success as i32;
    }
    match parsed.unmatched().first() {
        None => print_err(format!("{}: Missing command.\n", options.program()).as_bytes()),
        Some(first) => {
            let arg1 = args.get(1).map(String::as_str).unwrap_or_default();
            if !arg1.starts_with('-') {
                print_err(
                    format!("{}: Unrecognized command: \"{arg1}\"\n", options.program()).as_bytes(),
                );
            } else {
                print_err(
                    format!(
                        "{}: Unrecognized argument: \"{first}\"\n",
                        options.program()
                    )
                    .as_bytes(),
                );
            }
        }
    }
    print_err(print_usage(&options).as_bytes());
    ReturnCode::InvalidArguments as i32
}

/// Port of `main`: dispatches `args[1]` to a built-in sub-command.
pub fn ktx_main(args: &[String]) -> i32 {
    let rc = match args.get(1).map(String::as_str) {
        Some("help") => CommandHelp::main(args.get(1..).unwrap_or_default()),
        Some("create") => CommandCreate::main(args.get(1..).unwrap_or_default()),
        Some("compare") => CommandCompare::main(args.get(1..).unwrap_or_default()),
        Some("extract") => CommandExtract::main(args.get(1..).unwrap_or_default()),
        Some("deflate") => CommandDeflate::main(args.get(1..).unwrap_or_default()),
        Some("transcode") => CommandTranscode::main(args.get(1..).unwrap_or_default()),
        Some("info") => CommandInfo::main(args.get(1..).unwrap_or_default()),
        Some("validate") => CommandValidate::main(args.get(1..).unwrap_or_default()),
        _ => tools_main(args),
    };
    flush_out();
    rc
}
