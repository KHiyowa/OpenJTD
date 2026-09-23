#![doc = include_str!("../README.md")]

mod input;

#[path = "main/mod.rs"]
mod cli;

pub(crate) const BROKEN_PIPE_EXIT: &str = "__rjtd_broken_pipe__";

fn main() {
    let code = match run(std::env::args().skip(1)) {
        Ok(()) => 0,
        Err(message) if message == BROKEN_PIPE_EXIT => 0,
        Err(message) => {
            eprintln!("error: {message}");
            2
        }
    };

    std::process::exit(code);
}

fn run(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let mut args = args.into_iter();

    match args.next().as_deref() {
        None | Some("-h") | Some("--help") => cli::help::print_help(),
        Some("streams") => cli::container::run_streams(args),
        Some("info") => cli::container::run_info(args),
        Some("dump-stream") => cli::container::run_dump_stream(args),
        Some("cfb-map") => cli::container::run_cfb_map(args),
        Some("cfb-dir") => cli::container::run_cfb_dir(args),
        Some("cat") => cli::text_token::run_cat(args),
        Some("sheets") => cli::sheets::run_sheets(args),
        Some("export") => cli::render::run_export(args),
        Some(command) => Err(format!("unknown command: {command}")),
    }
}
