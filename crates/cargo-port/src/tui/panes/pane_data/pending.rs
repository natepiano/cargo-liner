use super::BuildMode;
use super::RunTargetKind;

/// How an owned Cargo invocation selects its package.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CargoPackageInvocation {
    /// Let Cargo select the workspace's default package.
    #[default]
    WorkspaceDefault,
    /// Run the named package within a workspace.
    Package(String),
}

/// A launch request waiting for the one Cargo Port-owned run slot.
#[derive(Clone)]
pub struct PendingExampleRun {
    pub abs_path:                 String,
    pub target_name:              String,
    pub display_path:             String,
    pub cargo_package_invocation: CargoPackageInvocation,
    pub run_target_kind:          RunTargetKind,
    pub build_mode:               BuildMode,
    pub required_features:        Vec<String>,
}

/// Whether a CI fetch should sync recent runs or discover older history.
#[derive(Clone, Copy)]
pub enum CiFetchKind {
    /// Fetch runs older than the oldest cached run.
    Older,
    /// Re-sync the most recent N runs, refreshing stale failures.
    Sync,
}

/// A pending request to fetch more CI runs for a project.
pub struct PendingCiFetch {
    pub project_path:      String,
    pub ci_run_count:      u32,
    pub oldest_created_at: Option<String>,
    pub ci_fetch_kind:     CiFetchKind,
}
