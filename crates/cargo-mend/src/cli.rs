use std::path::PathBuf;

use clap::Args;
use clap::CommandFactory;
use clap::FromArgMatches;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "mend")]
#[command(about = "Audit Rust visibility patterns against a stricter house style")]
#[command(long_about = "\
Audit Rust visibility patterns against a stricter house style.

Phases:
  1. check   - runs `cargo check` to collect compiler warnings
  2. driver  - runs the mend rustc driver to analyze visibility patterns
  3. analyze - scans source files for import and style issues

Use --fix, --fix-pub-use, or --fix-compiler to auto-fix findings.
Use --fix-all to apply all fixes at once.")]
pub(crate) struct Cli {
    /// Path to Cargo.toml
    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    /// Path to mend.toml config file
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Output findings as JSON
    #[arg(long)]
    pub json: bool,

    /// Exit with non-zero status on warnings
    #[arg(long)]
    pub fail_on_warn: bool,

    #[command(flatten)]
    pub fix: FixCli,
}

pub(crate) fn parse(after_help: &str) -> Cli {
    let matches = Cli::command()
        .after_long_help(after_help.to_string())
        .get_matches_from(normalized_args());
    Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}

#[derive(Args, Debug)]
pub(crate) struct FixCli {
    /// Auto-fix mend import and visibility findings
    #[arg(long)]
    pub fix: bool,

    /// Auto-fix stale `pub use` re-exports
    #[arg(long)]
    pub fix_pub_use: bool,

    /// Run `cargo fix` for compiler-fixable warnings
    #[arg(long)]
    pub fix_compiler: bool,

    /// Apply all fixes (--fix + --fix-pub-use + --fix-compiler)
    #[arg(long)]
    pub fix_all: bool,

    /// Preview fixes without applying them
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
