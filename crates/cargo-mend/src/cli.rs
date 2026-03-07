use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "vischeck")]
#[command(about = "Audit Rust visibility patterns against a stricter house style")]
pub(super) struct Cli {
    #[arg(long)]
    pub(super) manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub(super) config: Option<PathBuf>,

    #[arg(long)]
    pub(super) json: bool,

    #[arg(long)]
    pub(super) fail_on_warn: bool,
}

pub(super) fn parse() -> Cli { Cli::parse_from(normalized_args()) }

fn normalized_args() -> Vec<std::ffi::OsString> {
    let mut args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == "vischeck") {
        args.remove(1);
    }
    args
}
