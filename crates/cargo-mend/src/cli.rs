use std::path::PathBuf;

use clap::Args;
use clap::CommandFactory;
use clap::FromArgMatches;
use clap::Parser;

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
pub(crate) struct Cli {
    /// Show detailed build metadata and exit
    #[arg(long)]
    pub build_info: bool,

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
    #[command(flatten)]
    pub(crate) workspace: WorkspaceCli,

    /// Package to check
    #[arg(short = 'p', long = "package", value_name = "SPEC")]
    pub package: Vec<String>,

    /// Exclude package from a workspace check
    #[arg(long, value_name = "SPEC")]
    pub exclude: Vec<String>,

    /// Path to `Cargo.toml` or a project/workspace directory
    #[arg(long, value_name = "PATH", help_heading = "Manifest Options")]
    pub manifest_path: Option<PathBuf>,

    /// Positional alias for `--manifest-path`; accepts a `Cargo.toml` path or a
    /// project/workspace directory
    #[arg(
        value_name = "PATH",
        conflicts_with = "manifest_path",
        help_heading = "Manifest Options"
    )]
    pub positional_manifest_path: Option<PathBuf>,

    #[command(flatten)]
    pub(crate) primary_targets: PrimaryTargetCli,

    #[command(flatten)]
    pub(crate) secondary_targets: SecondaryTargetCli,

    /// Check the specified binary
    #[arg(long = "bin", value_name = "NAME", help_heading = "Target Selection")]
    pub bin: Vec<String>,

    /// Check the specified example
    #[arg(
        long = "example",
        value_name = "NAME",
        help_heading = "Target Selection"
    )]
    pub example: Vec<String>,

    /// Check the specified test target
    #[arg(long = "test", value_name = "NAME", help_heading = "Target Selection")]
    pub test: Vec<String>,

    /// Check the specified benchmark target
    #[arg(long = "bench", value_name = "NAME", help_heading = "Target Selection")]
    pub bench: Vec<String>,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WorkspaceCli {
    /// Check all packages in the workspace
    #[arg(long)]
    pub(crate) workspace: bool,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PrimaryTargetCli {
    /// Check all targets
    #[arg(long, help_heading = "Target Selection")]
    pub(crate) all_targets: bool,

    /// Check only this package's library
    #[arg(long, help_heading = "Target Selection")]
    pub(crate) lib: bool,

    /// Check all binary targets
    #[arg(long, help_heading = "Target Selection")]
    pub(crate) bins: bool,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SecondaryTargetCli {
    /// Check all example targets
    #[arg(long, help_heading = "Target Selection")]
    pub(crate) examples: bool,

    /// Check all test targets
    #[arg(long, help_heading = "Target Selection")]
    pub(crate) tests: bool,

    /// Check all benchmark targets
    #[arg(long, help_heading = "Target Selection")]
    pub(crate) benches: bool,
}

impl CargoCheckCli {
    pub(crate) fn explicit_manifest_path(&self) -> Option<&std::path::Path> {
        self.manifest_path
            .as_deref()
            .or(self.positional_manifest_path.as_deref())
    }

    pub(crate) const fn workspace(&self) -> bool { self.workspace.workspace }

    pub(crate) const fn all_targets(&self) -> bool { self.primary_targets.all_targets }

    pub(crate) const fn lib(&self) -> bool { self.primary_targets.lib }

    pub(crate) const fn bins(&self) -> bool { self.primary_targets.bins }

    pub(crate) const fn examples(&self) -> bool { self.secondary_targets.examples }

    pub(crate) const fn tests(&self) -> bool { self.secondary_targets.tests }

    pub(crate) const fn benches(&self) -> bool { self.secondary_targets.benches }
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
    #[command(flatten)]
    pub(crate) auto_fix: AutoFixCli,

    #[command(flatten)]
    pub(crate) execution: FixExecutionCli,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AutoFixCli {
    /// Auto-fix mend import and visibility findings
    #[arg(long)]
    pub(crate) fix: bool,

    /// Auto-fix stale `pub use` re-exports
    #[arg(long)]
    pub(crate) fix_pub_use: bool,

    /// Run `cargo fix` for compiler-fixable warnings
    #[arg(long)]
    pub(crate) fix_compiler: bool,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FixExecutionCli {
    /// Apply all fixes (--fix + --fix-pub-use + --fix-compiler)
    #[arg(long)]
    pub(crate) fix_all: bool,

    /// Preview fixes without applying them
    #[arg(long)]
    pub(crate) dry_run: bool,
}

impl FixCli {
    pub(crate) const fn fix(&self) -> bool { self.auto_fix.fix }

    pub(crate) const fn fix_pub_use(&self) -> bool { self.auto_fix.fix_pub_use }

    pub(crate) const fn fix_compiler(&self) -> bool { self.auto_fix.fix_compiler }

    pub(crate) const fn fix_all(&self) -> bool { self.execution.fix_all }

    pub(crate) const fn dry_run(&self) -> bool { self.execution.dry_run }
}

fn normalized_args() -> Vec<std::ffi::OsString> {
    let mut args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == "mend") {
        args.remove(1);
    }
    args
}
