//! Reads per-project lint status from a cache-rooted protocol file.
//!
//! The log is an append-only, tab-delimited file produced by either the
//! in-process lint runtime or an external watcher. Format:
//! `{ISO-8601}\t{status}` where status is `started`, `passed`, or `failed`.

use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::FixedOffset;
use chrono::Local;
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
use super::constants::PORT_REPORT_LOG;
use super::constants::STALE_TIMEOUT;
use super::tui::Icon;
use super::tui::LINT_SPINNER;

/// Lint status derived from the last line of `port-report.log`.
#[derive(Debug, Clone)]
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

    /// Human-readable label for the detail panel.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Running(_) => "running",
            Self::Passed(_) => "passed",
            Self::Failed(_) => "failed",
            Self::Stale => "stale",
            Self::NoLog => "-",
        }
    }

    pub const fn timestamp(&self) -> Option<&DateTime<FixedOffset>> {
        match self {
            Self::Running(ts) | Self::Passed(ts) | Self::Failed(ts) => Some(ts),
            Self::Stale | Self::NoLog => None,
        }
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

/// Cache-rooted lint status file for the project.
pub fn log_path(project_root: &Path) -> PathBuf { project_dir(project_root).join(PORT_REPORT_LOG) }

/// Cache-rooted lint status file for the project under an explicit cache root.
pub fn log_path_under(cache_root: &Path, project_root: &Path) -> PathBuf {
    project_dir_under(cache_root, project_root).join(PORT_REPORT_LOG)
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

/// Append a status event to the project's protocol file under an explicit
/// cache root.
pub fn append_status_under(cache_root: &Path, project_root: &Path, status: &str) -> io::Result<()> {
    let path = log_path_under(cache_root, project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}\t{status}", Local::now().to_rfc3339())
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
    read_status_from_path(&log_path(project_root))
}

fn read_status_from_path(path: &Path) -> LintStatus {
    let Ok(mut file) = File::open(path) else {
        return LintStatus::NoLog;
    };

    let Some(last_line) = read_last_line(&mut file) else {
        return LintStatus::NoLog;
    };

    parse_line(&last_line)
}

/// Seek to end and scan backwards for the last non-empty line.
fn read_last_line(file: &mut File) -> Option<String> {
    let end = file.seek(SeekFrom::End(0)).ok()?;
    if end == 0 {
        return None;
    }

    // Read up to the last 4 KiB — more than enough for one line.
    let read_start = end.saturating_sub(4096);
    file.seek(SeekFrom::Start(read_start)).ok()?;

    let mut buf = String::new();
    file.read_to_string(&mut buf).ok()?;

    buf.lines().next_back().map(String::from)
}

/// Parse `{ISO-8601}\t{status}` into a `LintStatus`.
fn parse_line(line: &str) -> LintStatus {
    let Some((ts_str, status)) = line.split_once('\t') else {
        return LintStatus::NoLog;
    };

    let Ok(ts) = DateTime::parse_from_rfc3339(ts_str.trim()) else {
        return LintStatus::NoLog;
    };

    match status.trim() {
        "passed" => LintStatus::Passed(ts),
        "failed" => LintStatus::Failed(ts),
        "started" => {
            let elapsed = Utc::now().signed_duration_since(ts);
            if elapsed > chrono::Duration::from_std(STALE_TIMEOUT).unwrap_or_default() {
                LintStatus::Stale
            } else {
                LintStatus::Running(ts)
            }
        },
        _ => LintStatus::NoLog,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::io::Write;

    use super::*;

    // ── parse_line ──────────────────────────────────────────────────

    #[test]
    fn parse_passed() {
        let line = "2026-03-30T14:22:18-05:00\tpassed";
        assert!(matches!(parse_line(line), LintStatus::Passed(_)));
    }

    #[test]
    fn parse_failed() {
        let line = "2026-03-30T14:22:18-05:00\tfailed";
        assert!(matches!(parse_line(line), LintStatus::Failed(_)));
    }

    #[test]
    fn parse_running() {
        let ts = Utc::now().format("%+").to_string();
        let line = format!("{ts}\tstarted");
        assert!(matches!(parse_line(&line), LintStatus::Running(_)));
    }

    #[test]
    fn parse_stale() {
        let line = "2020-01-01T00:00:00+00:00\tstarted";
        assert!(matches!(parse_line(line), LintStatus::Stale));
    }

    #[test]
    fn parse_garbage() {
        assert!(matches!(parse_line("not a valid line"), LintStatus::NoLog));
    }

    #[test]
    fn parse_empty_status() {
        let line = "2026-03-30T14:22:18-05:00\t";
        assert!(matches!(parse_line(line), LintStatus::NoLog));
    }

    #[test]
    fn parse_unknown_status() {
        let line = "2026-03-30T14:22:18-05:00\trunning";
        assert!(matches!(parse_line(line), LintStatus::NoLog));
    }

    // ── read_last_line ──────────────────────────────────────────────

    #[test]
    fn read_last_line_single_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.log");
        std::fs::write(&path, "hello\n").expect("write");
        let mut file = File::open(&path).expect("open");
        assert_eq!(read_last_line(&mut file).as_deref(), Some("hello"));
    }

    #[test]
    fn read_last_line_multiple_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.log");
        std::fs::write(&path, "first\nsecond\nthird\n").expect("write");
        let mut file = File::open(&path).expect("open");
        assert_eq!(read_last_line(&mut file).as_deref(), Some("third"));
    }

    #[test]
    fn read_last_line_no_trailing_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.log");
        std::fs::write(&path, "first\nsecond").expect("write");
        let mut file = File::open(&path).expect("open");
        assert_eq!(read_last_line(&mut file).as_deref(), Some("second"));
    }

    #[test]
    fn read_last_line_empty_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.log");
        std::fs::write(&path, "").expect("write");
        let mut file = File::open(&path).expect("open");
        assert!(read_last_line(&mut file).is_none());
    }

    // ── read_status (end-to-end) ────────────────────────────────────

    fn write_log(root: &Path, content: &str) {
        let path = log_path(root);
        std::fs::create_dir_all(path.parent().expect("log file has parent"))
            .expect("create cache port-report dir");
        let mut f = File::create(path).expect("create log");
        f.write_all(content.as_bytes()).expect("write log");
    }

    #[test]
    fn read_status_passed() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_log(
            dir.path(),
            "2026-03-30T14:22:01-05:00\tstarted\n2026-03-30T14:22:18-05:00\tpassed\n",
        );
        assert!(matches!(read_status(dir.path()), LintStatus::Passed(_)));
    }

    #[test]
    fn read_status_failed() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_log(
            dir.path(),
            "2026-03-30T14:22:01-05:00\tstarted\n2026-03-30T14:22:18-05:00\tfailed\n",
        );
        assert!(matches!(read_status(dir.path()), LintStatus::Failed(_)));
    }

    #[test]
    fn read_status_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ts = Utc::now().format("%+").to_string();
        write_log(dir.path(), &format!("{ts}\tstarted\n"));
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
        write_log(dir.path(), "2020-01-01T00:00:00+00:00\tstarted\n");
        assert!(matches!(read_status(dir.path()), LintStatus::Stale));
    }

    #[test]
    fn read_status_only_reads_last_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_log(
            dir.path(),
            "2026-03-30T14:22:01-05:00\tfailed\n2026-03-30T14:22:18-05:00\tpassed\n",
        );
        assert!(
            matches!(read_status(dir.path()), LintStatus::Passed(_)),
            "should read the last line (passed), not the first (failed)"
        );
    }

    #[test]
    fn cache_log_path_does_not_live_under_project_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = log_path(dir.path());
        assert!(
            !path.starts_with(dir.path()),
            "cache log path should not recreate project directories"
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
