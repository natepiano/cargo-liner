use std::collections::HashMap;
use std::path::Path;

use super::history;
use super::status;
use super::types::LintRun;
use super::types::LintStatus;

/// Per-project lint state: run history and current status.
///
/// Private fields with methods maintain internal consistency.
/// The name matches the UI terminology for a unified domain model.
#[derive(Clone, Debug, Default)]
pub struct LintRuns {
    runs:          Vec<LintRun>,
    status:        LintStatus,
    /// Archive byte size per run, keyed by `run_id`. Populated eagerly on
    /// `set_runs` so UI reads never touch the filesystem.
    archive_bytes: HashMap<String, u64>,
}

impl LintRuns {
    pub fn runs(&self) -> &[LintRun] { &self.runs }

    pub const fn status(&self) -> &LintStatus { &self.status }

    /// Archive byte size for a single run. `None` means we have no entry
    /// for this `run_id` (run not yet seen by `set_runs`); `Some(0)` means
    /// the entry exists and the archive directory is empty. Callers must
    /// distinguish these — rendering "—" vs "0 B" — because they signal
    /// different states (missing vs known-empty) and conflating them
    /// hides bugs in the watcher/archive pipeline.
    pub fn archive_bytes(&self, run_id: &str) -> Option<u64> {
        self.archive_bytes.get(run_id).copied()
    }

    /// Replace run history and derive status from the latest run. Recomputes
    /// archive sizes from disk once; subsequent `archive_bytes` lookups are
    /// O(1) and do no I/O.
    pub fn set_runs(&mut self, runs: Vec<LintRun>, project_root: &Path) {
        self.status = runs.first().map_or(LintStatus::NoLog, status::parse_run);
        self.archive_bytes = runs
            .iter()
            .map(|run| {
                (
                    run.run_id.clone(),
                    history::run_archive_bytes(project_root, &run.run_id),
                )
            })
            .collect();
        self.runs = runs;
    }

    pub const fn set_status(&mut self, status: LintStatus) { self.status = status; }

    pub fn clear_runs(&mut self) {
        self.runs.clear();
        self.archive_bytes.clear();
        self.status = LintStatus::NoLog;
    }

    /// True iff `run_id` has an entry in the archive-size map. Distinct from
    /// `archive_bytes(run_id) == 0`, which could mean either "no entry" or
    /// "entry, zero bytes." Test-only — production code always asks for the
    /// size, never whether we know it.
    #[cfg(test)]
    pub fn has_archive_entry(&self, run_id: &str) -> bool {
        self.archive_bytes.contains_key(run_id)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::lint::types::LintRunStatus;

    fn make_run(run_id: &str) -> LintRun {
        LintRun {
            run_id:      run_id.to_string(),
            started_at:  "2026-04-24T12:00:00Z".to_string(),
            finished_at: Some("2026-04-24T12:00:01Z".to_string()),
            duration_ms: Some(1000),
            status:      LintRunStatus::Passed,
            commands:    Vec::new(),
        }
    }

    fn stub_root() -> PathBuf { PathBuf::from("/tmp/cargo-port-lint-runs-test-stub") }

    #[test]
    fn set_runs_populates_archive_entry_per_run() {
        let mut lr = LintRuns::default();
        lr.set_runs(vec![make_run("a"), make_run("b")], &stub_root());

        assert!(lr.has_archive_entry("a"));
        assert!(lr.has_archive_entry("b"));
        assert!(!lr.has_archive_entry("c"));
    }

    #[test]
    fn archive_bytes_returns_none_for_unknown_run_id() {
        let lr = LintRuns::default();
        assert_eq!(lr.archive_bytes("nonexistent"), None);
    }

    #[test]
    fn archive_bytes_returns_some_for_known_run_id_even_when_zero() {
        // After `set_runs`, the entry is present even if the archive
        // directory doesn't exist yet (size will be 0). Distinguishing
        // this from "unknown" is the whole point of the `Option` API.
        let mut lr = LintRuns::default();
        lr.set_runs(vec![make_run("a")], &stub_root());
        assert_eq!(lr.archive_bytes("a"), Some(0));
        assert_eq!(lr.archive_bytes("not-a-real-run"), None);
    }

    #[test]
    fn clear_runs_empties_archive_entries() {
        let mut lr = LintRuns::default();
        lr.set_runs(vec![make_run("a")], &stub_root());
        assert!(lr.has_archive_entry("a"));

        lr.clear_runs();
        assert!(!lr.has_archive_entry("a"));
        assert!(lr.runs().is_empty());
    }

    #[test]
    fn set_runs_replaces_previous_archive_entries() {
        let mut lr = LintRuns::default();
        lr.set_runs(vec![make_run("a")], &stub_root());
        lr.set_runs(vec![make_run("b")], &stub_root());

        assert!(!lr.has_archive_entry("a"), "old run's entry should be gone");
        assert!(lr.has_archive_entry("b"));
    }
}
