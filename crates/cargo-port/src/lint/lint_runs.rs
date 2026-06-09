use std::collections::HashMap;

use chrono::DateTime;
use chrono::FixedOffset;

use super::status;
use super::types::LintRun;
use super::types::LintRunStatus;
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

    /// `started_at` of the most recent terminal run (newest run that is not
    /// `Running`), parsed to a timestamp. `None` when there is no such run or
    /// it fails to parse. The startup staleness check compares this against
    /// the newest source-file mtime to decide whether an edit post-dates the
    /// last lint.
    pub fn last_started_at(&self) -> Option<DateTime<FixedOffset>> {
        self.runs
            .iter()
            .find(|run| !matches!(run.status, LintRunStatus::Running))
            .and_then(|run| status::parse_timestamp(&run.started_at))
    }

    /// Archive byte size for a single run. `None` means we have no entry
    /// for this `run_id` (run not yet seen by `set_runs`); `Some(0)` means
    /// the entry exists and the archive directory is empty. Callers must
    /// distinguish these — rendering "—" vs "0 B" — because they signal
    /// different states (missing vs known-empty) and conflating them
    /// hides bugs in the watcher/archive pipeline.
    pub fn archive_bytes(&self, run_id: &str) -> Option<u64> {
        self.archive_bytes.get(run_id).copied()
    }

    /// Replace run history and derive status from the latest run. Archive
    /// sizes are read straight off each run (persisted when the run was
    /// archived), so this does no disk I/O and `archive_bytes` lookups stay
    /// O(1).
    pub fn set_runs(&mut self, runs: Vec<LintRun>) {
        self.status = runs.first().map_or(LintStatus::NoLog, status::parse_run);
        self.archive_bytes = runs
            .iter()
            .map(|run| (run.run_id.clone(), run.archive_bytes))
            .collect();
        self.runs = runs;
    }

    /// Replace run history from a cache load without replacing a live
    /// worker's `Running` status.
    pub fn set_hydrated_runs(&mut self, runs: Vec<LintRun>) {
        let live_status =
            matches!(self.status, LintStatus::Running(_)).then(|| self.status.clone());
        self.set_runs(runs);
        if let Some(status) = live_status {
            self.status = status;
        }
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
    use super::*;
    use crate::lint::types::LintRunStatus;

    fn make_run(run_id: &str) -> LintRun {
        LintRun {
            run_id:        run_id.to_string(),
            started_at:    "2026-04-24T12:00:00Z".to_string(),
            finished_at:   Some("2026-04-24T12:00:01Z".to_string()),
            duration_ms:   Some(1000),
            status:        LintRunStatus::Passed,
            commands:      Vec::new(),
            archive_bytes: 0,
        }
    }

    #[test]
    fn set_runs_populates_archive_entry_per_run() {
        let mut lr = LintRuns::default();
        lr.set_runs(vec![make_run("a"), make_run("b")]);

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
    fn archive_bytes_returns_the_size_persisted_on_the_run() {
        // The entry is present for every run in the set; its value is the
        // `archive_bytes` field carried on the run. Distinguishing a present
        // entry from an unknown run is the whole point of the `Option` API.
        let mut lr = LintRuns::default();
        lr.set_runs(vec![make_run("a")]);
        assert_eq!(lr.archive_bytes("a"), Some(0));
        assert_eq!(lr.archive_bytes("not-a-real-run"), None);
    }

    #[test]
    fn clear_runs_empties_archive_entries() {
        let mut lr = LintRuns::default();
        lr.set_runs(vec![make_run("a")]);
        assert!(lr.has_archive_entry("a"));

        lr.clear_runs();
        assert!(!lr.has_archive_entry("a"));
        assert!(lr.runs().is_empty());
    }

    #[test]
    fn set_runs_replaces_previous_archive_entries() {
        let mut lr = LintRuns::default();
        lr.set_runs(vec![make_run("a")]);
        lr.set_runs(vec![make_run("b")]);

        assert!(!lr.has_archive_entry("a"), "old run's entry should be gone");
        assert!(lr.has_archive_entry("b"));
    }

    #[test]
    fn last_started_at_skips_running_and_parses_newest_terminal() {
        fn run_at(run_id: &str, started_at: &str, status: LintRunStatus) -> LintRun {
            LintRun {
                run_id: run_id.to_string(),
                started_at: started_at.to_string(),
                finished_at: Some(started_at.to_string()),
                duration_ms: Some(1),
                status,
                commands: Vec::new(),
                archive_bytes: 0,
            }
        }

        let mut lr = LintRuns::default();
        assert_eq!(lr.last_started_at(), None);

        // Stored newest-first: the leading Running run is skipped; the next
        // terminal run supplies the start timestamp.
        lr.set_runs(vec![
            run_at("c", "2026-04-24T12:00:05Z", LintRunStatus::Running),
            run_at("b", "2026-04-24T12:00:03Z", LintRunStatus::Passed),
            run_at("a", "2026-04-24T12:00:00Z", LintRunStatus::Failed),
        ]);

        let expected = DateTime::parse_from_rfc3339("2026-04-24T12:00:03Z")
            .ok()
            .map(|ts| ts.timestamp());
        assert_eq!(lr.last_started_at().map(|ts| ts.timestamp()), expected);
    }
}
