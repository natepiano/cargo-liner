use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use clap::Args;
use clap::CommandFactory;
use clap::FromArgMatches;
use clap::Parser;

#[derive(Debug)]
pub(crate) struct Cli {
    pub build_info: bool,

    pub json: bool,

    pub fail_on_warn: bool,

    pub cargo: CargoCheckCli,

    pub manifest: ManifestCli,

    pub fix: FixCli,
}

pub(crate) fn parse(after_help: &str) -> Cli {
    let matches = RawCli::command()
        .after_long_help(after_help.to_string())
        .get_matches_from(normalized_args());
    RawCli::from_arg_matches(&matches).map_or_else(|e| e.exit(), Cli::from)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CargoCheckCli {
    pub(crate) workspace_selection: WorkspaceSelection,

    pub package: Vec<String>,

    pub exclude: Vec<String>,

    pub manifest_path: Option<PathBuf>,

    pub positional_manifest_path: Option<PathBuf>,

    pub(crate) target_selections: BTreeSet<TargetSelection>,

    pub bin: Vec<String>,

    pub example: Vec<String>,

    pub test: Vec<String>,

    pub bench: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum WorkspaceSelection {
    #[default]
    Auto,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum TargetSelection {
    All,
    Benches,
    Binaries,
    Examples,
    Library,
    Tests,
}

impl CargoCheckCli {
    pub(crate) fn explicit_manifest_path(&self) -> Option<&Path> {
        self.manifest_path
            .as_deref()
            .or(self.positional_manifest_path.as_deref())
    }
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Manifest Options")]
pub(crate) struct ManifestCli {
    /// Path to mend.toml config file
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FixCli {
    pub(crate) execution:       FixExecution,
    pub(crate) requested_fixes: BTreeSet<FixRequest>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FixExecution {
    #[default]
    ReadOnly,
    ApplyRequested,
    ApplyAll,
    PreviewRequested,
    PreviewAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FixRequest {
    Compiler,
    Mend,
    PubUse,
}

impl FixCli {
    pub(crate) fn includes(&self, requested_fix: FixRequest) -> bool {
        self.requested_fixes.contains(&requested_fix)
    }

    pub(crate) fn runs_compiler_fix(&self) -> bool {
        match self.execution {
            FixExecution::ApplyAll => true,
            FixExecution::ApplyRequested => self.includes(FixRequest::Compiler),
            FixExecution::ReadOnly | FixExecution::PreviewAll | FixExecution::PreviewRequested => {
                false
            },
        }
    }
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
    manifest: ManifestCli,

    #[command(flatten)]
    fix: RawFixCli,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[command(next_help_heading = "Package Selection")]
struct RawCargoCheckCli {
    #[command(flatten)]
    workspace: RawWorkspaceCli,

    /// Package to check
    #[arg(short = 'p', long = "package", value_name = "SPEC")]
    package: Vec<String>,

    /// Exclude package from a workspace check
    #[arg(long, value_name = "SPEC")]
    exclude: Vec<String>,

    /// Path to `Cargo.toml` or a project/workspace directory
    #[arg(long, value_name = "PATH", help_heading = "Manifest Options")]
    manifest_path: Option<PathBuf>,

    /// Positional alias for `--manifest-path`; accepts a `Cargo.toml` path or a
    /// project/workspace directory
    #[arg(
        value_name = "PATH",
        conflicts_with = "manifest_path",
        help_heading = "Manifest Options"
    )]
    positional_manifest_path: Option<PathBuf>,

    #[command(flatten)]
    primary_targets: RawPrimaryTargetCli,

    #[command(flatten)]
    secondary_targets: RawSecondaryTargetCli,

    /// Display findings only from the specified binary (analysis still
    /// runs across all targets)
    #[arg(long = "bin", value_name = "NAME", help_heading = "Target Selection")]
    bin: Vec<String>,

    /// Display findings only from the specified example (analysis still
    /// runs across all targets)
    #[arg(
        long = "example",
        value_name = "NAME",
        help_heading = "Target Selection"
    )]
    example: Vec<String>,

    /// Display findings only from the specified test target (analysis
    /// still runs across all targets)
    #[arg(long = "test", value_name = "NAME", help_heading = "Target Selection")]
    test: Vec<String>,

    /// Display findings only from the specified benchmark target
    /// (analysis still runs across all targets)
    #[arg(long = "bench", value_name = "NAME", help_heading = "Target Selection")]
    bench: Vec<String>,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
struct RawWorkspaceCli {
    /// Check all packages in the workspace
    #[arg(long)]
    workspace: bool,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
struct RawPrimaryTargetCli {
    /// Display findings from every target (default; mend always analyzes
    /// across all targets regardless of selection flags)
    #[arg(long, help_heading = "Target Selection")]
    all_targets: bool,

    /// Display findings only from this package's library (analysis still
    /// runs across all targets)
    #[arg(long, help_heading = "Target Selection")]
    lib: bool,

    /// Display findings only from binary targets (analysis still runs
    /// across all targets)
    #[arg(long, help_heading = "Target Selection")]
    bins: bool,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
struct RawSecondaryTargetCli {
    /// Display findings only from example targets (analysis still runs
    /// across all targets)
    #[arg(long, help_heading = "Target Selection")]
    examples: bool,

    /// Display findings only from test targets (analysis still runs
    /// across all targets)
    #[arg(long, help_heading = "Target Selection")]
    tests: bool,

    /// Display findings only from benchmark targets (analysis still runs
    /// across all targets)
    #[arg(long, help_heading = "Target Selection")]
    benches: bool,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[command(next_help_heading = "Mend Actions")]
struct RawFixCli {
    #[command(flatten)]
    auto_fix: RawAutoFixCli,

    #[command(flatten)]
    execution: RawFixExecutionCli,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
struct RawAutoFixCli {
    /// Auto-fix mend import and visibility findings
    #[arg(long)]
    fix: bool,

    /// Auto-fix stale `pub use` re-exports
    #[arg(long)]
    fix_pub_use: bool,

    /// Run `cargo fix` for compiler-fixable warnings
    #[arg(long)]
    fix_compiler: bool,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
struct RawFixExecutionCli {
    /// Apply all fixes (--fix + --fix-pub-use + --fix-compiler)
    #[arg(long)]
    fix_all: bool,

    /// Preview fixes without applying them
    #[arg(long)]
    dry_run: bool,
}

impl From<RawCli> for Cli {
    fn from(raw: RawCli) -> Self {
        Self {
            build_info:   raw.build_info,
            json:         raw.json,
            fail_on_warn: raw.fail_on_warn,
            cargo:        raw.cargo.into(),
            manifest:     raw.manifest,
            fix:          raw.fix.into(),
        }
    }
}

impl From<RawCargoCheckCli> for CargoCheckCli {
    fn from(raw: RawCargoCheckCli) -> Self {
        let mut target_selections = BTreeSet::new();
        if raw.primary_targets.all_targets {
            target_selections.insert(TargetSelection::All);
        }
        if raw.primary_targets.lib {
            target_selections.insert(TargetSelection::Library);
        }
        if raw.primary_targets.bins {
            target_selections.insert(TargetSelection::Binaries);
        }
        if raw.secondary_targets.examples {
            target_selections.insert(TargetSelection::Examples);
        }
        if raw.secondary_targets.tests {
            target_selections.insert(TargetSelection::Tests);
        }
        if raw.secondary_targets.benches {
            target_selections.insert(TargetSelection::Benches);
        }

        Self {
            workspace_selection: if raw.workspace.workspace {
                WorkspaceSelection::Workspace
            } else {
                WorkspaceSelection::Auto
            },
            package: raw.package,
            exclude: raw.exclude,
            manifest_path: raw.manifest_path,
            positional_manifest_path: raw.positional_manifest_path,
            target_selections,
            bin: raw.bin,
            example: raw.example,
            test: raw.test,
            bench: raw.bench,
        }
    }
}

impl From<RawFixCli> for FixCli {
    fn from(raw: RawFixCli) -> Self {
        let mut requested_fixes = BTreeSet::new();
        if raw.auto_fix.fix {
            requested_fixes.insert(FixRequest::Mend);
        }
        if raw.auto_fix.fix_pub_use {
            requested_fixes.insert(FixRequest::PubUse);
        }
        if raw.auto_fix.fix_compiler {
            requested_fixes.insert(FixRequest::Compiler);
        }

        let execution = match (
            raw.execution.fix_all,
            raw.execution.dry_run,
            requested_fixes.is_empty(),
        ) {
            (true, true, _) | (false, true, true) => FixExecution::PreviewAll,
            (true, false, _) => FixExecution::ApplyAll,
            (false, true, false) => FixExecution::PreviewRequested,
            (false, false, true) => FixExecution::ReadOnly,
            (false, false, false) => FixExecution::ApplyRequested,
        };

        Self {
            execution,
            requested_fixes,
        }
    }
}

fn normalized_args() -> Vec<std::ffi::OsString> {
    let mut args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == "mend") {
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
    use std::collections::BTreeSet;

    use clap::CommandFactory;
    use clap::FromArgMatches;

    use super::Cli;
    use super::FixCli;
    use super::FixExecution;
    use super::FixRequest;
    use super::RawCli;

    fn parse(cli_args: &[&str]) -> Cli {
        let full_argv = std::iter::once("mend").chain(cli_args.iter().copied());
        let matches = RawCli::command().get_matches_from(full_argv);
        Cli::from(RawCli::from_arg_matches(&matches).expect("test argv must parse"))
    }

    #[test]
    fn runs_compiler_fix_false_for_read_only() {
        let fix = FixCli::default();
        assert!(!fix.runs_compiler_fix());
    }

    #[test]
    fn runs_compiler_fix_false_for_preview_all() {
        let fix = FixCli {
            execution: FixExecution::PreviewAll,
            ..FixCli::default()
        };
        assert!(!fix.runs_compiler_fix());
    }

    #[test]
    fn runs_compiler_fix_false_for_preview_requested_with_compiler() {
        let fix = FixCli {
            execution:       FixExecution::PreviewRequested,
            requested_fixes: BTreeSet::from([FixRequest::Compiler]),
        };
        assert!(!fix.runs_compiler_fix());
    }

    #[test]
    fn runs_compiler_fix_true_for_apply_all() {
        let fix = FixCli {
            execution: FixExecution::ApplyAll,
            ..FixCli::default()
        };
        assert!(fix.runs_compiler_fix());
    }

    #[test]
    fn runs_compiler_fix_true_for_apply_requested_with_compiler() {
        let fix = FixCli {
            execution:       FixExecution::ApplyRequested,
            requested_fixes: BTreeSet::from([FixRequest::Compiler]),
        };
        assert!(fix.runs_compiler_fix());
    }

    #[test]
    fn runs_compiler_fix_false_for_apply_requested_without_compiler() {
        let fix = FixCli {
            execution:       FixExecution::ApplyRequested,
            requested_fixes: BTreeSet::from([FixRequest::Mend]),
        };
        assert!(!fix.runs_compiler_fix());
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
