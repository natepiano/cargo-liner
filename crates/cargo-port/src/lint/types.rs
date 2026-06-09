use std::cmp::Ordering;
use std::time::SystemTime;

use chrono::DateTime;
use chrono::FixedOffset;
use serde::Deserialize;
use serde::Serialize;

use crate::config::DiscoveryLint;

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

/// Origin of a live lint run as reported by the runtime.
///
/// This is intentionally separate from [`LintStatus`]: cached/project status
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LintRunStatus {
    Running,
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LintCommandStatus {
    Pending,
    Passed,
    Failed,
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
