use super::status;
use super::types::LintRun;
use super::types::LintStatus;

/// Per-project lint state: run history and current status.
///
/// Private fields with methods maintain internal consistency.
/// The name matches the UI terminology for a unified domain model.
#[derive(Clone, Debug, Default)]
pub struct LintRuns {
    runs:   Vec<LintRun>,
    status: LintStatus,
}

impl LintRuns {
    pub fn runs(&self) -> &[LintRun] { &self.runs }

    pub const fn status(&self) -> &LintStatus { &self.status }

    /// Replace run history and derive status from the latest run.
    pub fn set_runs(&mut self, runs: Vec<LintRun>) {
        self.status = runs.first().map_or(LintStatus::NoLog, status::parse_run);
        self.runs = runs;
    }

    pub const fn set_status(&mut self, status: LintStatus) { self.status = status; }

    pub fn clear_runs(&mut self) {
        self.runs.clear();
        self.status = LintStatus::NoLog;
    }
}
