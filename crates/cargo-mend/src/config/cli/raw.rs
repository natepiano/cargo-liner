use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

use clap::Args;
use clap::CommandFactory;
use clap::FromArgMatches;
use clap::Parser;

use super::BuildInfoMode;
use super::Cli;
use super::ManifestCli;
use super::WarningPolicy;
use super::fix::RawFixCli;
use super::target::RawCargoCheckCli;
use crate::compiler::CARGO_SUBCOMMAND_MEND;
use crate::reporting::OutputFormat;

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[command(next_help_heading = "Manifest Options")]
struct RawManifestCli {
    /// Path to mend.toml config file
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
}

#[derive(Parser, Debug)]
#[command(name = "mend")]
#[command(about = "Audit Rust visibility patterns against a stricter house style")]
#[command(version)]
#[command(long_about = "\
Audit Rust visibility patterns against a stricter house style.

Phases:
  1. check   - runs `cargo check` with the mend rustc wrapper
  2. analyze - scans source files for import and style issues

Use --fix, --fix-pub-use, or --fix-compiler to auto-fix findings.
Use --fix-all to apply all fixes at once.")]
struct RawCli {
    /// Show detailed build metadata and exit
    #[arg(long)]
    build_info: bool,

    /// JSON output
    #[arg(long)]
    json: bool,

    /// Fail on warnings
    #[arg(long)]
    fail_on_warn: bool,

    #[command(flatten)]
    cargo: RawCargoCheckCli,

    #[command(flatten)]
    manifest: RawManifestCli,

    #[command(flatten)]
    fix: RawFixCli,
}

impl From<RawCli> for Cli {
    fn from(raw: RawCli) -> Self {
        Self {
            build_info:     if raw.build_info {
                BuildInfoMode::Show
            } else {
                BuildInfoMode::Run
            },
            output_format:  if raw.json {
                OutputFormat::Json
            } else {
                OutputFormat::Human
            },
            warning_policy: if raw.fail_on_warn {
                WarningPolicy::Fail
            } else {
                WarningPolicy::Allow
            },
            cargo:          raw.cargo.into(),
            manifest:       raw.manifest.into(),
            fix:            raw.fix.into(),
        }
    }
}

impl From<RawManifestCli> for ManifestCli {
    fn from(raw: RawManifestCli) -> Self { Self { config: raw.config } }
}

pub(super) fn parse(after_help: &str) -> Cli {
    let matches = RawCli::command()
        .after_long_help(after_help.to_string())
        .get_matches_from(normalized_args());
    RawCli::from_arg_matches(&matches).map_or_else(|e| e.exit(), Cli::from)
}

fn normalized_args() -> Vec<OsString> {
    let mut args: Vec<_> = env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == CARGO_SUBCOMMAND_MEND) {
        args.remove(1);
    }
    args
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::iter;

    use clap::CommandFactory;
    use clap::FromArgMatches;

    use super::RawCli;
    use crate::config::cli::Cli;
    use crate::config::cli::FixExecution;
    use crate::config::cli::FixRequest;

    fn parse(cli_args: &[&str]) -> Cli {
        let full_argv = iter::once("mend").chain(cli_args.iter().copied());
        let matches = RawCli::command().get_matches_from(full_argv);
        Cli::from(RawCli::from_arg_matches(&matches).expect("test argv must parse"))
    }

    #[test]
    fn dry_run_with_fix_compiler_does_not_mutate() {
        let cli = parse(&["--dry-run", "--fix-compiler"]);
        assert_eq!(cli.fix.execution, FixExecution::PreviewRequested);
        assert!(cli.fix.includes(FixRequest::Compiler));
        assert!(!cli.fix.runs_compiler_fix());
    }

    #[test]
    fn dry_run_with_fix_all_does_not_mutate() {
        let cli = parse(&["--dry-run", "--fix-all"]);
        assert_eq!(cli.fix.execution, FixExecution::PreviewAll);
        assert!(!cli.fix.runs_compiler_fix());
    }

    #[test]
    fn fix_compiler_alone_does_mutate() {
        let cli = parse(&["--fix-compiler"]);
        assert_eq!(cli.fix.execution, FixExecution::ApplyRequested);
        assert!(cli.fix.runs_compiler_fix());
    }

    #[test]
    fn fix_all_alone_does_mutate() {
        let cli = parse(&["--fix-all"]);
        assert_eq!(cli.fix.execution, FixExecution::ApplyAll);
        assert!(cli.fix.runs_compiler_fix());
    }
}
