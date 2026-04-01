//! Reads `target/port-report.log` to determine per-project lint status.
//!
//! The log is an append-only, tab-delimited file produced by an external
//! lint watcher. Format: `{ISO-8601}\t{status}` where status is
//! `started`, `passed`, or `failed`. cargo-port is a pure reader.

use std::fs::File;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::Path;

use chrono::DateTime;
use chrono::FixedOffset;
use chrono::Utc;

use super::constants::LINT_FAILED;
use super::constants::LINT_NO_LOG;
use super::constants::LINT_PASSED;
use super::constants::LINT_STALE;
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

/// Read the last line of `{project_root}/target/port-report.log` and parse it.
pub fn read_status(project_root: &Path) -> LintStatus {
    let path = project_root.join("target").join(PORT_REPORT_LOG);
    let Ok(mut file) = File::open(&path) else {
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
        let target = root.join("target");
        std::fs::create_dir_all(&target).expect("create target dir");
        let mut f = File::create(target.join(PORT_REPORT_LOG)).expect("create log");
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
}
