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
