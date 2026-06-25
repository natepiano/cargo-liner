use std::cmp::Ordering;
use std::path::Path;
use std::time::SystemTime;

use chrono::DateTime;
use chrono::FixedOffset;
use chrono::Utc;

use super::paths;
use super::read_write;
use super::run::LintRun;
use super::run::LintRunStatus;
use crate::config::DiscoveryLint;
use crate::constants::STALE_TIMEOUT;

/// Display-agnostic discriminant of [`LintStatus`]. The TUI integration
/// layer (`crate::tui::integration::lint_display`) maps this to the
/// concrete `tui_pane::Icon` used at render time, keeping `lint/` free
/// of UI-framework imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintStatusKind {
    Running,
    Passed,
    Failed,
    Stale,
    NoLog,
}

/// Lint status derived from the latest lint run record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LintStatus {
    Running(DateTime<FixedOffset>),
    Passed(DateTime<FixedOffset>),
    Failed(DateTime<FixedOffset>),
    Stale,
    #[default]
    NoLog,
}

impl LintStatus {
    /// Returns the display-agnostic [`LintStatusKind`] discriminant.
    pub const fn kind(&self) -> LintStatusKind {
        match self {
            Self::Running(_) => LintStatusKind::Running,
            Self::Passed(_) => LintStatusKind::Passed,
            Self::Failed(_) => LintStatusKind::Failed,
            Self::Stale => LintStatusKind::Stale,
            Self::NoLog => LintStatusKind::NoLog,
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

/// Lint status loaded from cache during startup/sync.
///
/// This deliberately cannot represent `Running`: cache hydration is historical
/// state, while `LintStatus::Running` is reserved for a live worker in this
/// process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CachedLintStatus {
    Passed(DateTime<FixedOffset>),
    Failed(DateTime<FixedOffset>),
    #[default]
    NoLog,
}

impl CachedLintStatus {
    pub const fn from_lint_status(status: &LintStatus) -> Option<Self> {
        match status {
            LintStatus::Passed(timestamp) => Some(Self::Passed(*timestamp)),
            LintStatus::Failed(timestamp) => Some(Self::Failed(*timestamp)),
            LintStatus::NoLog => Some(Self::NoLog),
            LintStatus::Running(_) | LintStatus::Stale => None,
        }
    }

    pub const fn into_lint_status(self) -> LintStatus {
        match self {
            Self::Passed(timestamp) => LintStatus::Passed(timestamp),
            Self::Failed(timestamp) => LintStatus::Failed(timestamp),
            Self::NoLog => LintStatus::NoLog,
        }
    }

    /// Should this project be linted as the startup phase closes?
    ///
    /// - `NoLog` (never linted) is the discovery case, gated by `on_discovery` so turning discovery
    ///   linting off does not lint every project on the first launch.
    /// - A terminal result (`Passed`/`Failed`) re-lints only when a source file changed since that
    ///   run began — `max_source_mtime` newer than `last_started_at`. This staleness check is
    ///   independent of `on_discovery`: an edited project always re-lints.
    pub fn should_lint_on_startup(
        &self,
        last_started_at: Option<DateTime<FixedOffset>>,
        max_source_mtime: Option<SystemTime>,
        on_discovery: DiscoveryLint,
    ) -> bool {
        match self {
            Self::NoLog => on_discovery.is_immediate(),
            Self::Passed(_) | Self::Failed(_) => match (last_started_at, max_source_mtime) {
                (Some(started), Some(mtime)) => source_is_newer(mtime, started),
                _ => false,
            },
        }
    }
}

/// True when `mtime` is at least one whole second newer than `last_started_at`.
/// Comparing at second granularity keeps sub-second mtime / RFC3339 rounding
/// noise from re-triggering a lint that already covered the change.
fn source_is_newer(mtime: SystemTime, last_started_at: DateTime<FixedOffset>) -> bool {
    let Ok(since_epoch) = mtime.duration_since(SystemTime::UNIX_EPOCH) else {
        return false;
    };
    let mtime_secs = i64::try_from(since_epoch.as_secs()).unwrap_or(i64::MAX);
    mtime_secs > last_started_at.timestamp()
}

pub fn read_status_under(cache_root: &Path, project_root: &Path) -> LintStatus {
    read_status_from_path(&paths::latest_path_under(cache_root, project_root))
}

fn read_status_from_path(path: &Path) -> LintStatus {
    let Some(run) = read_write::read_latest_file(path) else {
        return LintStatus::NoLog;
    };
    parse_run(&run)
}

pub(crate) fn parse_timestamp(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value.trim()).ok()
}

pub(super) fn parse_run(run: &LintRun) -> LintStatus {
    let timestamp = run
        .finished_at
        .as_deref()
        .and_then(parse_timestamp)
        .or_else(|| parse_timestamp(&run.started_at));
    let Some(ts) = timestamp else {
        return LintStatus::NoLog;
    };

    match run.status {
        LintRunStatus::Passed => LintStatus::Passed(ts),
        LintRunStatus::Failed => LintStatus::Failed(ts),
        LintRunStatus::Running => {
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
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;
    use std::time::SystemTime;

    use chrono::DateTime;
    use chrono::FixedOffset;
    use chrono::Utc;

    use super::*;
    use crate::config::DiscoveryLint;
    use crate::lint::history;
    use crate::lint::read_write;

    fn run(status: LintRunStatus) -> LintRun {
        LintRun {
            run_id: "run-1".to_string(),
            started_at: "2026-03-30T14:22:01-05:00".to_string(),
            finished_at: Some("2026-03-30T14:22:18-05:00".to_string()),
            duration_ms: Some(17_000),
            status,
            commands: Vec::new(),
            archive_bytes: 0,
        }
    }

    #[test]
    fn parse_run_cases() {
        let mut running = run(LintRunStatus::Running);
        running.started_at = Utc::now().format("%+").to_string();
        running.finished_at = None;

        let mut stale = run(LintRunStatus::Running);
        stale.started_at = "2020-01-01T00:00:00+00:00".to_string();
        stale.finished_at = None;

        let mut garbage = run(LintRunStatus::Passed);
        garbage.started_at = "not a valid timestamp".to_string();
        garbage.finished_at = Some("not a valid timestamp".to_string());

        let mut empty = run(LintRunStatus::Passed);
        empty.started_at.clear();
        empty.finished_at = None;

        let cases = [
            ("passed", run(LintRunStatus::Passed)),
            ("failed", run(LintRunStatus::Failed)),
            ("running", running),
            ("stale", stale),
            ("garbage", garbage),
            ("empty", empty),
        ];

        for (name, run) in cases {
            let status = parse_run(&run);
            match name {
                "passed" => assert!(matches!(status, LintStatus::Passed(_)), "{name}"),
                "failed" => assert!(matches!(status, LintStatus::Failed(_)), "{name}"),
                "running" => assert!(matches!(status, LintStatus::Running(_)), "{name}"),
                "stale" => assert!(matches!(status, LintStatus::Stale), "{name}"),
                "garbage" | "empty" => assert!(matches!(status, LintStatus::NoLog), "{name}"),
                _ => panic!("unexpected case"),
            }
        }
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

    fn write_latest(cache_root: &Path, project_root: &Path, run: &LintRun) {
        read_write::write_latest_under(cache_root, project_root, run).expect("write latest");
    }

    #[test]
    fn read_status_reads_latest_and_reports_missing_log() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let dir = tempfile::tempdir().expect("tempdir");
        write_latest(cache_dir.path(), dir.path(), &run(LintRunStatus::Passed));
        assert!(matches!(
            read_status_under(cache_dir.path(), dir.path()),
            LintStatus::Passed(_)
        ));

        let missing = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            read_status_under(cache_dir.path(), missing.path()),
            LintStatus::NoLog
        ));
    }

    #[test]
    fn read_status_uses_latest_over_history() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let dir = tempfile::tempdir().expect("tempdir");
        history::append_history_under(
            cache_dir.path(),
            dir.path(),
            &run(LintRunStatus::Failed),
            None,
        )
        .expect("append history");
        write_latest(cache_dir.path(), dir.path(), &run(LintRunStatus::Passed));
        assert!(
            matches!(
                read_status_under(cache_dir.path(), dir.path()),
                LintStatus::Passed(_)
            ),
            "should read latest.json, not older history"
        );
    }

    #[test]
    fn should_lint_on_startup_gates_nolog_by_discovery_and_relints_stale_terminal() {
        // NoLog (never linted) is the discovery case — gated by config, and
        // independent of any source mtime.
        assert!(CachedLintStatus::NoLog.should_lint_on_startup(
            None,
            None,
            DiscoveryLint::Immediate
        ));
        assert!(!CachedLintStatus::NoLog.should_lint_on_startup(
            None,
            None,
            DiscoveryLint::Deferred
        ));

        // A terminal result re-lints only when a source mtime post-dates the run
        // start, regardless of discovery config.
        let started: DateTime<FixedOffset> =
            DateTime::parse_from_rfc3339("2026-03-30T14:22:01-05:00").expect("parse start");
        let run_epoch = SystemTime::UNIX_EPOCH
            + Duration::from_secs(u64::try_from(started.timestamp()).expect("non-negative epoch"));
        let passed = CachedLintStatus::Passed(started);

        assert!(passed.should_lint_on_startup(
            Some(started),
            Some(run_epoch + Duration::from_secs(5)),
            DiscoveryLint::Deferred,
        ));
        assert!(!passed.should_lint_on_startup(
            Some(started),
            Some(run_epoch - Duration::from_secs(5)),
            DiscoveryLint::Deferred,
        ));
        // Same whole second is not "newer" — the second-granularity guard.
        assert!(!passed.should_lint_on_startup(
            Some(started),
            Some(run_epoch),
            DiscoveryLint::Immediate,
        ));
        // No source mtime collected → a terminal result cannot be stale.
        assert!(!passed.should_lint_on_startup(Some(started), None, DiscoveryLint::Immediate));
    }
}
