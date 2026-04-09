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
  1. check   - runs `cargo check` with the mend rustc wrapper
  2. analyze - scans source files for import and style issues

Use --fix, --fix-pub-use, or --fix-compiler to auto-fix findings.
Use --fix-all to apply all fixes at once.")]
pub(crate) struct Cli {
    /// JSON output
    #[arg(long)]
    pub json: bool,

    /// Fail on warnings
    #[arg(long)]
    pub fail_on_warn: bool,

    #[command(flatten)]
    pub cargo: CargoCheckCli,

    #[command(flatten)]
    pub manifest: ManifestCli,

    #[command(flatten)]
    pub fix: FixCli,
}

pub(crate) fn parse(after_help: &str) -> Cli {
    let matches = Cli::command()
        .after_long_help(after_help.to_string())
        .get_matches_from(normalized_args());
    Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[command(next_help_heading = "Package Selection")]
pub(crate) struct CargoCheckCli {
    /// Check all packages in the workspace
    #[arg(long)]
    pub workspace: bool,

    /// Package to check
    #[arg(short = 'p', long = "package", value_name = "SPEC")]
    pub package: Vec<String>,

    /// Exclude package from a workspace check
    #[arg(long, value_name = "SPEC")]
    pub exclude: Vec<String>,

    /// Path to Cargo.toml
    #[arg(long, value_name = "PATH", help_heading = "Manifest Options")]
    pub manifest_path: Option<PathBuf>,

    /// Check all targets
    #[arg(long, help_heading = "Target Selection")]
    pub all_targets: bool,

    /// Check only this package's library
    #[arg(long, help_heading = "Target Selection")]
    pub lib: bool,

    /// Check all binary targets
    #[arg(long, help_heading = "Target Selection")]
    pub bins: bool,

    /// Check all example targets
    #[arg(long, help_heading = "Target Selection")]
    pub examples: bool,

    /// Check all test targets
    #[arg(long, help_heading = "Target Selection")]
    pub tests: bool,

    /// Check all benchmark targets
    #[arg(long, help_heading = "Target Selection")]
    pub benches: bool,

    /// Check the specified binary
    #[arg(long = "bin", value_name = "NAME", help_heading = "Target Selection")]
    pub bin: Vec<String>,

    /// Check the specified example
    #[arg(long = "example", value_name = "NAME", help_heading = "Target Selection")]
    pub example: Vec<String>,

    /// Check the specified test target
    #[arg(long = "test", value_name = "NAME", help_heading = "Target Selection")]
    pub test: Vec<String>,

    /// Check the specified benchmark target
    #[arg(long = "bench", value_name = "NAME", help_heading = "Target Selection")]
    pub bench: Vec<String>,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Manifest Options")]
pub(crate) struct ManifestCli {
    /// Path to mend.toml config file
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Mend Actions")]
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
