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

/// One item tracked by a task toast.
#[derive(Clone, Debug)]
pub struct TrackedItem {
    /// Display label for the tracked item.
    pub label:        String,
    /// Stable key used to update the tracked item.
    pub key:          TrackedItemKey,
    /// Time the item started or restarted.
    pub started_at:   Option<Instant>,
    /// Time the item completed.
    pub completed_at: Option<Instant>,
}

impl TrackedItem {
    /// Create a tracked item with `started_at` set to now.
    pub fn new(label: impl Into<String>, key: impl Into<TrackedItemKey>) -> Self {
        Self {
            label:        label.into(),
            key:          key.into(),
            started_at:   Some(Instant::now()),
            completed_at: None,
        }
    }

    /// Return the display label.
    #[must_use]
    pub fn label(&self) -> &str { &self.label }

    /// Return the stable key.
    #[must_use]
    pub const fn key(&self) -> &TrackedItemKey { &self.key }

    /// Return the completion timestamp, if present.
    #[must_use]
    pub const fn completed_at(&self) -> Option<Instant> { self.completed_at }

    /// Mark the item completed at `now`.
    pub const fn mark_completed(&mut self, now: Instant) { self.completed_at = Some(now); }
}

/// Render-ready view of one tracked task item.
#[derive(Clone, Debug)]
pub struct TrackedItemView {
    /// Display label.
    pub label:           String,
    /// Completion linger progress from 0.0 to 1.0, if completed.
    pub linger_progress: Option<f64>,
    /// Elapsed time since the item started, if known.
    pub elapsed:         Option<Duration>,
}

impl TrackedItemView {
    /// Return the display label.
    #[must_use]
    pub fn label(&self) -> &str { &self.label }
}
