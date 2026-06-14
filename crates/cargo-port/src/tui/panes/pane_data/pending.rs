use super::BuildMode;
use super::RunTargetKind;

pub struct PendingExampleRun {
    pub abs_path:          String,
    pub target_name:       String,
    pub display_path:      String,
    pub package_name:      Option<String>,
    pub run_target_kind:   RunTargetKind,
    pub build_mode:        BuildMode,
    pub required_features: Vec<String>,
}

/// Whether a CI fetch should sync recent runs or discover older history.
#[derive(Clone, Copy)]
pub enum CiFetchKind {
    /// Fetch runs older than the oldest cached run.
    FetchOlder,
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
