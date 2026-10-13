use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result as FmtResult;
use std::time::Duration;
use std::time::Instant;

/// Stable key for a tracked task item.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TrackedItemKey(String);

impl TrackedItemKey {
    /// Create a tracked-item key.
    pub fn new(value: impl Into<String>) -> Self { Self(value.into()) }

    /// Return the key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl From<String> for TrackedItemKey {
    fn from(value: String) -> Self { Self(value) }
}

impl From<&str> for TrackedItemKey {
    fn from(value: &str) -> Self { Self(value.to_owned()) }
}

impl Display for TrackedItemKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult { f.write_str(&self.0) }
}

/// Whether a tracked item is making progress or is stalled waiting on
/// something outside its control. `Stalled` renders the item's spinner in the
/// palette's error color instead of its accent color.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TrackedItemActivity {
    /// The work is progressing.
    #[default]
    Progressing,
    /// The work is stalled and cannot proceed.
    Stalled,
}

/// Observed lifetime of a tracked task item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackedItemTiming {
    /// The item is running, but its start was not observed.
    StartUnknown,
    /// The item is running from a known start.
    Running {
        /// Time the item started or restarted.
        started_at: Instant,
    },
    /// The item finished without an observed start.
    CompletedWithoutStart {
        /// Time the item completed.
        completed_at: Instant,
    },
    /// The item finished after an observed start.
    Completed {
        /// Time the item started or restarted.
        started_at:   Instant,
        /// Time the item completed.
        completed_at: Instant,
    },
}

impl TrackedItemTiming {
    /// Return whether the item has completed.
    #[must_use]
    pub const fn is_completed(self) -> bool {
        matches!(
            self,
            Self::CompletedWithoutStart { .. } | Self::Completed { .. }
        )
    }

    pub(super) fn elapsed(self, now: Instant) -> TrackedItemElapsed {
        match self {
            Self::StartUnknown | Self::CompletedWithoutStart { .. } => {
                TrackedItemElapsed::StartUnknown
            },
            Self::Running { started_at } => {
                TrackedItemElapsed::Measured(now.saturating_duration_since(started_at))
            },
            Self::Completed {
                started_at,
                completed_at,
            } => TrackedItemElapsed::Measured(completed_at.saturating_duration_since(started_at)),
        }
    }
}

/// Rendered lifecycle of a tracked task item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackedItemLifecycle {
    /// The item is still running.
    Running,
    /// The item completed with its completion fade disabled.
    CompletedWithoutFade,
    /// The completed item fades during its linger period.
    CompletedFading {
        /// Fade progress from 0.0 to 1.0.
        progress: f64,
    },
}

/// Whether an elapsed duration can be displayed for a tracked item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackedItemElapsed {
    /// The start was not observed, so no duration can be displayed.
    StartUnknown,
    /// Duration from the start to now, or to completion for a finished item.
    Measured(Duration),
}

/// One item tracked by a task toast.
#[derive(Clone, Debug)]
pub struct TrackedItem {
    /// Display label for the tracked item.
    pub label:    String,
    /// Stable key used to update the tracked item.
    pub key:      TrackedItemKey,
    /// Observed start and completion state.
    pub timing:   TrackedItemTiming,
    /// Whether the item is progressing or stalled.
    pub activity: TrackedItemActivity,
}

impl TrackedItem {
    /// Create a running tracked item starting now.
    pub fn new(label: impl Into<String>, key: impl Into<TrackedItemKey>) -> Self {
        Self {
            label:    label.into(),
            key:      key.into(),
            timing:   TrackedItemTiming::Running {
                started_at: Instant::now(),
            },
            activity: TrackedItemActivity::Progressing,
        }
    }

    /// Return the display label.
    #[must_use]
    pub fn label(&self) -> &str { &self.label }

    /// Return the stable key.
    #[must_use]
    pub const fn key(&self) -> &TrackedItemKey { &self.key }

    /// Return the item's observed start and completion state.
    #[must_use]
    pub const fn timing(&self) -> TrackedItemTiming { self.timing }

    /// Mark the item completed at `now`.
    pub const fn mark_completed(&mut self, now: Instant) {
        self.timing = match self.timing {
            TrackedItemTiming::StartUnknown | TrackedItemTiming::CompletedWithoutStart { .. } => {
                TrackedItemTiming::CompletedWithoutStart { completed_at: now }
            },
            TrackedItemTiming::Running { started_at }
            | TrackedItemTiming::Completed { started_at, .. } => TrackedItemTiming::Completed {
                started_at,
                completed_at: now,
            },
        };
    }
}

/// Render-ready view of one tracked task item.
#[derive(Clone, Debug)]
pub struct TrackedItemView {
    /// Display label.
    pub label:     String,
    /// Running or completed state, including the completion fade policy.
    pub lifecycle: TrackedItemLifecycle,
    /// Elapsed time since the item started, if known.
    pub elapsed:   TrackedItemElapsed,
    /// Whether the item is progressing or stalled.
    pub activity:  TrackedItemActivity,
}

impl TrackedItemView {
    /// Return the display label.
    #[must_use]
    pub fn label(&self) -> &str { &self.label }
}
