use serde::Deserialize;
use serde::Serialize;

/// Origin of a live lint run as reported by the runtime.
///
/// This is intentionally separate from `LintStatus`: cached/project status
/// only needs `Running` / terminal state, while toast routing must distinguish
/// startup catch-up work from later file-triggered work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintRunOrigin {
    CatchUp,
    Normal,
}

impl LintRunOrigin {
    pub const fn merged_with(self, other: Self) -> Self {
        match (self, other) {
            (Self::Normal, _) | (_, Self::Normal) => Self::Normal,
            (Self::CatchUp, Self::CatchUp) => Self::CatchUp,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LintRunStatus {
    Running,
    Passed,
    Failed,
    /// The project's environment could not be prepared, so none of its lint
    /// commands ran. Distinct from `Failed`: nothing was found wrong with the
    /// code, because nothing examined it. Only `direnv exec` projects can
    /// reach this — see `crate::lint::runtime::command::env_probe`.
    EnvUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LintCommandStatus {
    Pending,
    Passed,
    Failed,
    /// The run ended before this command started, so it has no result and no
    /// log. Written when the environment probe fails.
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LintCommand {
    pub name:        String,
    pub command:     String,
    pub status:      LintCommandStatus,
    pub duration_ms: Option<u64>,
    pub exit_code:   Option<i32>,
    pub log_file:    String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LintRun {
    pub run_id:        String,
    pub started_at:    String,
    pub finished_at:   Option<String>,
    pub duration_ms:   Option<u64>,
    pub status:        LintRunStatus,
    pub commands:      Vec<LintCommand>,
    /// Total bytes of this run's archived command logs, summed once when the
    /// run is archived (see [`crate::lint::history::archive_run_output`]) and
    /// persisted on the history line. Reading it back is O(1) — the UI no
    /// longer walks every run's directory at startup to size the list.
    #[serde(default)]
    pub archive_bytes: u64,
}
