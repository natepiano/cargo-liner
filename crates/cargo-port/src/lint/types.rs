use chrono::DateTime;
use chrono::FixedOffset;
use serde::Deserialize;
use serde::Serialize;

use crate::constants::LINT_FAILED;
use crate::constants::LINT_NO_LOG;
use crate::constants::LINT_PASSED;
use crate::constants::LINT_STALE;
use crate::tui::Icon;
use crate::tui::LINT_SPINNER;

/// Lint status derived from the latest lint run record.
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
pub enum LintRunStatus {
    Running,
    Passed,
    Failed,
}

impl LintRunStatus {
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
    pub run_id:      String,
    pub started_at:  String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub status:      LintRunStatus,
    pub commands:    Vec<LintCommand>,
}
