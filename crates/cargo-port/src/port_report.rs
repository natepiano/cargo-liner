//! Reads per-project Port Report state from cache-rooted JSON artifacts.

use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::FixedOffset;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

use super::cache_paths;
use super::constants::LINT_FAILED;
use super::constants::LINT_NO_LOG;
use super::constants::LINT_PASSED;
use super::constants::LINT_STALE;
use super::constants::PORT_REPORT_HISTORY_JSONL;
use super::constants::PORT_REPORT_LATEST_JSON;
use super::constants::STALE_TIMEOUT;
use super::tui::Icon;
use super::tui::LINT_SPINNER;

/// Lint status derived from the latest Port Report run record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LintStatus {
    Running(DateTime<FixedOffset>),
    Passed(DateTime<FixedOffset>),
    Failed(DateTime<FixedOffset>),
    Stale,
    NoLog,
}

impl LintStatus {
    /// Returns the `Icon` for this lint status.
    pub const fn icon(&self) -> Icon {
        match self {
            Self::Running(_) => Icon::Animated(LINT_SPINNER),
            Self::Passed(_) => Icon::Static(LINT_PASSED),
            Self::Failed(_) => Icon::Static(LINT_FAILED),
            Self::Stale => Icon::Static(LINT_STALE),
            Self::NoLog => Icon::Static(LINT_NO_LOG),
        }
    }

    const fn severity_rank(&self) -> u8 {
        match self {
            Self::NoLog => 0,
            Self::Passed(_) => 1,
            Self::Stale => 2,
            Self::Running(_) => 3,
            Self::Failed(_) => 4,
        }
    }

    pub fn combine(self, other: Self) -> Self {
        use std::cmp::Ordering;

        match self.severity_rank().cmp(&other.severity_rank()) {
            Ordering::Greater => self,
            Ordering::Less => other,
            Ordering::Equal => match (self, other) {
                (Self::Passed(lhs), Self::Passed(rhs)) => Self::Passed(lhs.max(rhs)),
                (Self::Running(lhs), Self::Running(rhs)) => Self::Running(lhs.max(rhs)),
                (Self::Failed(lhs), Self::Failed(rhs)) => Self::Failed(lhs.max(rhs)),
                (Self::Stale, Self::Stale) => Self::Stale,
                (Self::NoLog, Self::NoLog) => Self::NoLog,
                (lhs, _) => lhs,
            },
        }
    }

    pub fn aggregate<I>(statuses: I) -> Self
    where
        I: IntoIterator<Item = Self>,
    {
        statuses
            .into_iter()
            .reduce(Self::combine)
            .unwrap_or(Self::NoLog)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PortReportRunStatus {
    Running,
    Passed,
    Failed,
}

impl PortReportRunStatus {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PortReportCommandStatus {
    Pending,
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortReportCommand {
    pub name:        String,
    pub command:     String,
    pub status:      PortReportCommandStatus,
    pub duration_ms: Option<u64>,
    pub exit_code:   Option<i32>,
    pub log_file:    String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortReportRun {
    pub run_id:      String,
    pub started_at:  String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub status:      PortReportRunStatus,
    pub commands:    Vec<PortReportCommand>,
}

/// Canonical cache directory for all per-project lint status files.
pub fn cache_root() -> PathBuf { cache_paths::port_report_root() }

/// Stable per-project cache key used by both cargo-port and external scripts.
pub fn project_key(project_root: &Path) -> String {
    let mut encoded = String::new();
    for byte in project_root.to_string_lossy().as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

/// Cache-rooted directory for the project's lint watcher protocol files.
pub fn project_dir(project_root: &Path) -> PathBuf { cache_root().join(project_key(project_root)) }

/// Cache-rooted directory for the project's lint watcher protocol files under
/// an explicit cache root.
pub fn project_dir_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    cache_root.join(project_key(project_root))
}

/// Cache-rooted raw command output directory for the project under an explicit
/// cache root.
pub fn output_dir_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    project_dir_under(cache_root, project_root).join("port-report")
}

pub fn latest_path_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    project_dir_under(cache_root, project_root).join(PORT_REPORT_LATEST_JSON)
}

pub fn history_path_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    project_dir_under(cache_root, project_root).join(PORT_REPORT_HISTORY_JSONL)
}

pub fn write_latest_under(
    cache_root: &Path,
    project_root: &Path,
    run: &PortReportRun,
) -> io::Result<()> {
    let path = latest_path_under(cache_root, project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(run)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(tmp_path, path)
}

pub fn clear_latest_under(cache_root: &Path, project_root: &Path) -> io::Result<()> {
    let path = latest_path_under(cache_root, project_root);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

pub fn append_history_under(
    cache_root: &Path,
    project_root: &Path,
    run: &PortReportRun,
) -> io::Result<()> {
    let path = history_path_under(cache_root, project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let json = serde_json::to_string(run)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    writeln!(file, "{json}")
}

pub fn read_history(project_root: &Path) -> Vec<PortReportRun> {
    read_history_under(&cache_root(), project_root)
}

pub fn read_history_under(cache_root: &Path, project_root: &Path) -> Vec<PortReportRun> {
    let mut runs = read_history_file(&history_path_under(cache_root, project_root));
    let latest = read_latest_file(&latest_path_under(cache_root, project_root));

    if let Some(latest_run) = latest
        && runs
            .last()
            .is_none_or(|run| run.run_id != latest_run.run_id)
    {
        runs.push(latest_run);
    }

    runs.reverse();
    runs
}

fn read_latest_file(path: &Path) -> Option<PortReportRun> {
    let file = File::open(path).ok()?;
    serde_json::from_reader(file).ok()
}

fn read_history_file(path: &Path) -> Vec<PortReportRun> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    reader
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<PortReportRun>(&line).ok())
        .collect()
}

/// Read the last line of the project's lint status log and parse it.
pub fn read_status(project_root: &Path) -> LintStatus {
    read_status_from_path(&latest_path_under(&cache_root(), project_root))
}

fn read_status_from_path(path: &Path) -> LintStatus {
    let Some(run) = read_latest_file(path) else {
        return LintStatus::NoLog;
    };
    parse_run(&run)
}

fn parse_timestamp(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value.trim()).ok()
}

fn parse_run(run: &PortReportRun) -> LintStatus {
    let timestamp = run
        .finished_at
        .as_deref()
        .and_then(parse_timestamp)
        .or_else(|| parse_timestamp(&run.started_at));
    let Some(ts) = timestamp else {
        return LintStatus::NoLog;
    };

    match run.status {
        PortReportRunStatus::Passed => LintStatus::Passed(ts),
        PortReportRunStatus::Failed => LintStatus::Failed(ts),
        PortReportRunStatus::Running => {
            let elapsed = Utc::now().signed_duration_since(ts);
            if elapsed > chrono::Duration::from_std(STALE_TIMEOUT).unwrap_or_default() {
                LintStatus::Stale
            } else {
                LintStatus::Running(ts)
            }
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    fn run(status: PortReportRunStatus) -> PortReportRun {
        PortReportRun {
            run_id: "run-1".to_string(),
            started_at: "2026-03-30T14:22:01-05:00".to_string(),
            finished_at: Some("2026-03-30T14:22:18-05:00".to_string()),
            duration_ms: Some(17_000),
            status,
            commands: Vec::new(),
        }
    }

    // ── parse_run ───────────────────────────────────────────────────

    #[test]
    fn parse_passed() {
        assert!(matches!(
            parse_run(&run(PortReportRunStatus::Passed)),
            LintStatus::Passed(_)
        ));
    }

    #[test]
    fn parse_failed() {
        assert!(matches!(
            parse_run(&run(PortReportRunStatus::Failed)),
            LintStatus::Failed(_)
        ));
    }

    #[test]
    fn parse_running() {
        let mut run = run(PortReportRunStatus::Running);
        run.started_at = Utc::now().format("%+").to_string();
        run.finished_at = None;
        assert!(matches!(parse_run(&run), LintStatus::Running(_)));
    }

    #[test]
    fn parse_stale() {
        let mut run = run(PortReportRunStatus::Running);
        run.started_at = "2020-01-01T00:00:00+00:00".to_string();
        run.finished_at = None;
        assert!(matches!(parse_run(&run), LintStatus::Stale));
    }

    #[test]
    fn parse_garbage() {
        let mut run = run(PortReportRunStatus::Passed);
        run.started_at = "not a valid timestamp".to_string();
        run.finished_at = Some("not a valid timestamp".to_string());
        assert!(matches!(parse_run(&run), LintStatus::NoLog));
    }

    #[test]
    fn parse_empty_status() {
        let mut run = run(PortReportRunStatus::Passed);
        run.started_at.clear();
        run.finished_at = None;
        assert!(matches!(parse_run(&run), LintStatus::NoLog));
    }

    #[test]
    fn aggregate_prefers_highest_severity() {
        let ts = DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("timestamp");
        let status = LintStatus::aggregate([
            LintStatus::Passed(ts),
            LintStatus::Stale,
            LintStatus::Running(ts),
            LintStatus::Failed(ts),
        ]);
        assert!(matches!(status, LintStatus::Failed(_)));
    }

    #[test]
    fn aggregate_keeps_latest_timestamp_within_variant() {
        let older = DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("older");
        let newer = DateTime::parse_from_rfc3339("2026-03-30T15:22:18-05:00").expect("newer");
        let status = LintStatus::aggregate([LintStatus::Passed(older), LintStatus::Passed(newer)]);
        assert_eq!(status, LintStatus::Passed(newer));
    }

    // ── read_status (end-to-end) ────────────────────────────────────

    fn write_latest(root: &Path, run: &PortReportRun) {
        write_latest_under(&cache_root(), root, run).expect("write latest");
    }

    #[test]
    fn read_status_passed() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_latest(dir.path(), &run(PortReportRunStatus::Passed));
        assert!(matches!(read_status(dir.path()), LintStatus::Passed(_)));
    }

    #[test]
    fn read_status_failed() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_latest(dir.path(), &run(PortReportRunStatus::Failed));
        assert!(matches!(read_status(dir.path()), LintStatus::Failed(_)));
    }

    #[test]
    fn read_status_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut run = run(PortReportRunStatus::Running);
        run.started_at = Utc::now().format("%+").to_string();
        run.finished_at = None;
        write_latest(dir.path(), &run);
        assert!(matches!(read_status(dir.path()), LintStatus::Running(_)));
    }

    #[test]
    fn read_status_no_log() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(read_status(dir.path()), LintStatus::NoLog));
    }

    #[test]
    fn read_status_stale_started() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut run = run(PortReportRunStatus::Running);
        run.started_at = "2020-01-01T00:00:00+00:00".to_string();
        run.finished_at = None;
        write_latest(dir.path(), &run);
        assert!(matches!(read_status(dir.path()), LintStatus::Stale));
    }

    #[test]
    fn read_status_uses_latest_over_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        append_history_under(&cache_root(), dir.path(), &run(PortReportRunStatus::Failed))
            .expect("append history");
        write_latest(dir.path(), &run(PortReportRunStatus::Passed));
        assert!(
            matches!(read_status(dir.path()), LintStatus::Passed(_)),
            "should read latest.json, not older history"
        );
    }

    #[test]
    fn cache_latest_path_does_not_live_under_project_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = latest_path_under(&cache_root(), dir.path());
        assert!(
            !path.starts_with(dir.path()),
            "cache latest path should not recreate project directories"
        );
    }

    #[test]
    fn history_reads_newest_first_and_includes_latest() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let completed = PortReportRun {
            run_id:      "completed".to_string(),
            started_at:  "2026-04-01T18:00:00-04:00".to_string(),
            finished_at: Some("2026-04-01T18:00:10-04:00".to_string()),
            duration_ms: Some(10_000),
            status:      PortReportRunStatus::Passed,
            commands:    Vec::new(),
        };
        let running = PortReportRun {
            run_id:      "running".to_string(),
            started_at:  "2026-04-01T18:05:00-04:00".to_string(),
            finished_at: None,
            duration_ms: None,
            status:      PortReportRunStatus::Running,
            commands:    Vec::new(),
        };

        append_history_under(cache_dir.path(), project_dir.path(), &completed)
            .expect("append history");
        write_latest_under(cache_dir.path(), project_dir.path(), &running).expect("write latest");

        let runs = read_history_under(cache_dir.path(), project_dir.path());
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].run_id, "running");
        assert_eq!(runs[1].run_id, "completed");
    }

    #[test]
    fn latest_final_run_does_not_duplicate_completed_history() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let completed = PortReportRun {
            run_id:      "same-run".to_string(),
            started_at:  "2026-04-01T18:00:00-04:00".to_string(),
            finished_at: Some("2026-04-01T18:00:10-04:00".to_string()),
            duration_ms: Some(10_000),
            status:      PortReportRunStatus::Passed,
            commands:    Vec::new(),
        };

        append_history_under(cache_dir.path(), project_dir.path(), &completed)
            .expect("append history");
        write_latest_under(cache_dir.path(), project_dir.path(), &completed).expect("write latest");

        let runs = read_history_under(cache_dir.path(), project_dir.path());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "same-run");
    }
}
