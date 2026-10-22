//! The `ktx` command line tool.

fn main() {
    let args: Vec<String> = std::env::args_os()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    std::process::exit(ktx::cli::ktx_main::ktx_main(&args));
}
