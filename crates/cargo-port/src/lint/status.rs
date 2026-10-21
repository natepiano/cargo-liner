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

/// Whether a live lint command is making progress or is stopped waiting for a
/// cargo file lock. Detected from command output by
/// `crate::lint::runtime::command`; consumed by the spinner color in
/// `crate::tui::state::lint_cell_for`.
///
/// `Ord` places `Blocked` above `Executing` so [`LintStatus::combine`] makes one
/// blocked worktree turn its whole group's rollup row red.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum LintRunPhase {
    #[default]
    Executing,
    Blocked,
}

/// Display-agnostic discriminant of [`LintStatus`]. The TUI integration
/// layer (`crate::tui::integration::lint_display`) maps this to the
/// concrete `tui_pane::Icon` used at render time, keeping `lint/` free
/// of UI-framework imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintStatusKind {
    Running(LintRunPhase),
    Passed,
    Failed,
    EnvUnavailable,
    Stale,
    Waiting,
    NoLog,
}

/// Lint status derived from the latest lint run record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LintStatus {
    Running(DateTime<FixedOffset>, LintRunPhase),
    Passed(DateTime<FixedOffset>),
    Failed(DateTime<FixedOffset>),
    /// The last run could not load the project's environment, so no lint
    /// command ran. Held apart from `Failed` so a `direnv` problem never
    /// reads as a finding about the code.
    EnvUnavailable(DateTime<FixedOffset>),
    Stale,
    /// A lint command deferred the last run and its retry is pending. The
    /// prior result stays in `latest.json`. Published by the worker only and
    /// never held in [`CachedLintStatus`].
    Waiting,
    #[default]
    NoLog,
}

impl LintStatus {
    /// Returns the display-agnostic [`LintStatusKind`] discriminant.
    pub const fn kind(&self) -> LintStatusKind {
        match self {
            Self::Running(_, phase) => LintStatusKind::Running(*phase),
            Self::Passed(_) => LintStatusKind::Passed,
            Self::Failed(_) => LintStatusKind::Failed,
            Self::EnvUnavailable(_) => LintStatusKind::EnvUnavailable,
            Self::Stale => LintStatusKind::Stale,
            Self::Waiting => LintStatusKind::Waiting,
            Self::NoLog => LintStatusKind::NoLog,
        }
    }

    /// `EnvUnavailable` outranks `Running` so a group rollup surfaces a
    /// project whose environment is broken instead of hiding it behind a
    /// sibling's spinner, and sits below `Failed` because a finding about the
    /// code is the more specific result of the two. `Waiting` sits below
    /// `Running`, so a sibling's live run shows over a pending retry.
    const fn severity_rank(&self) -> u8 {
        match self {
            Self::NoLog => 0,
            Self::Passed(_) => 1,
            Self::Stale => 2,
            Self::Waiting => 3,
            Self::Running(..) => 4,
            Self::EnvUnavailable(_) => 5,
            Self::Failed(_) => 6,
        }
    }

    pub fn combine(self, other: Self) -> Self {
        match self.severity_rank().cmp(&other.severity_rank()) {
            Ordering::Greater => self,
            Ordering::Less => other,
            Ordering::Equal => match (self, other) {
                (Self::Passed(lhs), Self::Passed(rhs)) => Self::Passed(lhs.max(rhs)),
                (Self::Running(lhs_at, lhs_phase), Self::Running(rhs_at, rhs_phase)) => {
                    Self::Running(lhs_at.max(rhs_at), lhs_phase.max(rhs_phase))
                },
                (Self::Failed(lhs), Self::Failed(rhs)) => Self::Failed(lhs.max(rhs)),
                (Self::EnvUnavailable(lhs), Self::EnvUnavailable(rhs)) => {
                    Self::EnvUnavailable(lhs.max(rhs))
                },
                (Self::Stale, Self::Stale) => Self::Stale,
                (Self::Waiting, Self::Waiting) => Self::Waiting,
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

/// Outcome of a terminal lint run, independent of its timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachedLintOutcome {
    Passed,
    Failed,
    EnvUnavailable,
}

impl CachedLintOutcome {
    const fn into_lint_status(self, timestamp: DateTime<FixedOffset>) -> LintStatus {
        match self {
            Self::Passed => LintStatus::Passed(timestamp),
            Self::Failed => LintStatus::Failed(timestamp),
            Self::EnvUnavailable => LintStatus::EnvUnavailable(timestamp),
        }
    }
}

/// Lint status hydrated from a historical run during startup/sync.
///
/// `Terminal` owns the validated start of the same run as its outcome and
/// display timestamp. `StartTimeUnavailable` names a terminal result with no
/// validated start: a persisted record whose start is missing or invalid, or a
/// display-only update (`CachedLintStatus::without_start_time`) held until
/// `LintRuns::set_hydrated_runs` supplies the record. Its optional finish
/// timestamp preserves whatever display information could be parsed. No variant can
/// represent a live worker's `LintStatus::Running` state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CachedLintStatus {
    Terminal {
        outcome:    CachedLintOutcome,
        timestamp:  DateTime<FixedOffset>,
        started_at: DateTime<FixedOffset>,
    },
    StartTimeUnavailable {
        outcome:   CachedLintOutcome,
        timestamp: Option<DateTime<FixedOffset>>,
    },
    #[default]
    NoLog,
}

impl CachedLintStatus {
    /// Hydrate a single run, pairing its outcome with its own validated start.
    /// `Running` records are not historical terminal results.
    pub fn from_run(run: &LintRun) -> Option<Self> {
        let outcome = match run.status {
            LintRunStatus::Passed => CachedLintOutcome::Passed,
            LintRunStatus::Failed => CachedLintOutcome::Failed,
            LintRunStatus::EnvUnavailable => CachedLintOutcome::EnvUnavailable,
            LintRunStatus::Running => return None,
        };
        let finished_at = run.finished_at.as_deref().and_then(parse_timestamp);
        Some(parse_timestamp(&run.started_at).map_or(
            Self::StartTimeUnavailable {
                outcome,
                timestamp: finished_at,
            },
            |started_at| Self::Terminal {
                outcome,
                timestamp: finished_at.unwrap_or(started_at),
                started_at,
            },
        ))
    }

    /// A display-only update cannot supply a run's start. Keep its result
    /// explicit until `LintRuns::set_hydrated_runs` supplies the actual record.
    pub(super) const fn without_start_time(status: &LintStatus) -> Option<Self> {
        let (outcome, timestamp) = match status {
            LintStatus::Passed(timestamp) => (CachedLintOutcome::Passed, *timestamp),
            LintStatus::Failed(timestamp) => (CachedLintOutcome::Failed, *timestamp),
            LintStatus::EnvUnavailable(timestamp) => {
                (CachedLintOutcome::EnvUnavailable, *timestamp)
            },
            LintStatus::NoLog => return Some(Self::NoLog),
            LintStatus::Running(..) | LintStatus::Stale | LintStatus::Waiting => return None,
        };
        Some(Self::StartTimeUnavailable {
            outcome,
            timestamp: Some(timestamp),
        })
    }

    pub const fn into_lint_status(self) -> LintStatus {
        match self {
            Self::Terminal {
                outcome, timestamp, ..
            }
            | Self::StartTimeUnavailable {
                outcome,
                timestamp: Some(timestamp),
            } => outcome.into_lint_status(timestamp),
            Self::StartTimeUnavailable {
                timestamp: None, ..
            }
            | Self::NoLog => LintStatus::NoLog,
        }
    }

    /// Should this project be linted as the startup phase closes?
    ///
    /// - `NoLog` is the discovery case, gated by `on_discovery` so turning discovery linting off
    ///   does not lint every project on the first launch.
    /// - `Terminal` re-lints only when `max_source_mtime` is newer than its `started_at`,
    ///   independently of `on_discovery`: an edited project always re-lints.
    /// - `StartTimeUnavailable` cannot establish staleness and skips startup re-lint, preserving
    ///   the policy for an unparseable run start.
    pub fn should_lint_on_startup(
        &self,
        max_source_mtime: Option<SystemTime>,
        on_discovery: DiscoveryLint,
    ) -> bool {
        match self {
            Self::NoLog => on_discovery.is_immediate(),
            Self::Terminal { started_at, .. } => {
                max_source_mtime.is_some_and(|mtime| source_is_newer(mtime, *started_at))
            },
            Self::StartTimeUnavailable { .. } => false,
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
        LintRunStatus::EnvUnavailable => LintStatus::EnvUnavailable(ts),
        LintRunStatus::Running => {
            let elapsed = Utc::now().signed_duration_since(ts);
            if elapsed > chrono::Duration::from_std(STALE_TIMEOUT).unwrap_or_default() {
                LintStatus::Stale
            } else {
                // `LintRun` deliberately does not persist `LintRunPhase`; `parse_run`
                // uses `LintRunPhase::Executing` because only the worker's output
                // reader publishes live `LintRunPhase::Blocked` state.
                LintStatus::Running(ts, LintRunPhase::Executing)
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
            ("env_unavailable", run(LintRunStatus::EnvUnavailable)),
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
                "env_unavailable" => {
                    assert!(matches!(status, LintStatus::EnvUnavailable(_)), "{name}");
                },
                "running" => assert!(
                    matches!(status, LintStatus::Running(_, LintRunPhase::Executing)),
                    "{name}"
                ),
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
            LintStatus::Running(ts, LintRunPhase::Executing),
            LintStatus::Failed(ts),
        ]);
        assert!(matches!(status, LintStatus::Failed(_)));
    }

    /// A group rollup must show a broken environment over a sibling still
    /// running, and a real finding over a broken environment.
    #[test]
    fn aggregate_places_env_unavailable_between_running_and_failed() {
        let ts = DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("timestamp");
        assert!(matches!(
            LintStatus::aggregate([
                LintStatus::Running(ts, LintRunPhase::Executing),
                LintStatus::EnvUnavailable(ts),
            ]),
            LintStatus::EnvUnavailable(_)
        ));
        assert!(matches!(
            LintStatus::aggregate([LintStatus::EnvUnavailable(ts), LintStatus::Failed(ts)]),
            LintStatus::Failed(_)
        ));
    }

    #[test]
    fn aggregate_running_prefers_blocked_phase() {
        let ts = DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("timestamp");
        let status = LintStatus::aggregate([
            LintStatus::Running(ts, LintRunPhase::Executing),
            LintStatus::Running(ts, LintRunPhase::Blocked),
        ]);
        assert_eq!(status, LintStatus::Running(ts, LintRunPhase::Blocked));
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
    fn hydrated_terminal_uses_its_start_instead_of_its_finish_for_staleness() {
        for outcome in [
            LintRunStatus::Passed,
            LintRunStatus::Failed,
            LintRunStatus::EnvUnavailable,
        ] {
            let record = run(outcome);
            let started = parse_timestamp(&record.started_at).expect("start");
            let finished = parse_timestamp(record.finished_at.as_deref().expect("finish text"))
                .expect("finish");
            let source_mtime = SystemTime::UNIX_EPOCH
                + Duration::from_secs(u64::try_from(started.timestamp() + 5).expect("epoch"));
            let cached = CachedLintStatus::from_run(&record).expect("terminal run");
            assert!(
                matches!(cached, CachedLintStatus::Terminal { started_at, timestamp, .. }
                if started_at == started && timestamp == finished)
            );
            assert!(cached.should_lint_on_startup(Some(source_mtime), DiscoveryLint::Deferred));
            assert_eq!(cached.into_lint_status(), parse_run(&record));
        }
    }

    #[test]
    fn unavailable_start_is_named_and_skips_startup_lint_without_losing_the_display_result() {
        for outcome in [
            LintRunStatus::Passed,
            LintRunStatus::Failed,
            LintRunStatus::EnvUnavailable,
        ] {
            for started_at in ["", "not a timestamp"] {
                let mut record = run(outcome.clone());
                record.started_at = started_at.to_string();
                let cached = CachedLintStatus::from_run(&record).expect("terminal run");
                assert!(matches!(
                    cached,
                    CachedLintStatus::StartTimeUnavailable { .. }
                ));
                assert!(
                    !cached
                        .should_lint_on_startup(Some(SystemTime::now()), DiscoveryLint::Immediate)
                );
                assert_eq!(cached.into_lint_status(), parse_run(&record));

                record.finished_at = None;
                let cached = CachedLintStatus::from_run(&record).expect("terminal run");
                assert!(matches!(
                    cached,
                    CachedLintStatus::StartTimeUnavailable {
                        timestamp: None,
                        ..
                    }
                ));
                assert!(
                    !cached
                        .should_lint_on_startup(Some(SystemTime::now()), DiscoveryLint::Immediate)
                );
                assert_eq!(cached.into_lint_status(), LintStatus::NoLog);
            }
        }
    }

    #[test]
    fn should_lint_on_startup_gates_nolog_by_discovery_and_relints_stale_terminal() {
        // NoLog (never linted) is the discovery case — gated by config, and
        // independent of any source mtime.
        assert!(CachedLintStatus::NoLog.should_lint_on_startup(None, DiscoveryLint::Immediate));
        assert!(!CachedLintStatus::NoLog.should_lint_on_startup(None, DiscoveryLint::Deferred));

        // `passed.should_lint_on_startup` re-lints a terminal result only when the
        // source mtime post-dates `started`, independently of `DiscoveryLint`.
        let started: DateTime<FixedOffset> =
            DateTime::parse_from_rfc3339("2026-03-30T14:22:01-05:00").expect("parse start");
        let run_epoch = SystemTime::UNIX_EPOCH
            + Duration::from_secs(u64::try_from(started.timestamp()).expect("non-negative epoch"));
        let passed = CachedLintStatus::from_run(&run(LintRunStatus::Passed)).expect("terminal run");

        assert!(passed.should_lint_on_startup(
            Some(run_epoch + Duration::from_secs(5)),
            DiscoveryLint::Deferred,
        ));
        assert!(!passed.should_lint_on_startup(
            Some(run_epoch - Duration::from_secs(5)),
            DiscoveryLint::Deferred,
        ));
        // `source_is_newer` rejects a source mtime in the same whole second as
        // `started`, preserving the rounding-noise guard.
        assert!(!passed.should_lint_on_startup(Some(run_epoch), DiscoveryLint::Immediate,));
        // `should_lint_on_startup` treats `max_source_mtime: None` as insufficient
        // evidence to re-lint a terminal result.
        assert!(!passed.should_lint_on_startup(None, DiscoveryLint::Immediate));
    }
}
