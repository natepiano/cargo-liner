use std::path::PathBuf;

use clap::Args;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "mend")]
#[command(about = "Audit Rust visibility patterns against a stricter house style")]
pub struct Cli {
    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    #[arg(long)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub fail_on_warn: bool,

    #[command(flatten)]
    pub fix: FixCli,
}

pub fn parse() -> Cli { Cli::parse_from(normalized_args()) }

#[derive(Args, Debug)]
pub struct FixCli {
    #[arg(long)]
    pub fix: bool,

    #[arg(long)]
    pub fix_pub_use: bool,

    #[arg(long)]
    pub dry_run: bool,
}

fn normalized_args() -> Vec<std::ffi::OsString> {
    let mut args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == "mend") {
        args.remove(1);
    }
    args
}
