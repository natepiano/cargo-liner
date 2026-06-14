use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use clap::Args;

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
    All,
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
    pub fn explicit_manifest_path(&self) -> Option<&Path> {
        self.manifest_path
            .as_deref()
            .or(self.positional_manifest_path.as_deref())
    }
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[command(next_help_heading = "Package Selection")]
pub(super) struct RawCargoCheckCli {
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
                WorkspaceSelection::All
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
