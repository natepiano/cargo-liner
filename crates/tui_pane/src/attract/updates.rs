//! Whether the display is live or paused.

/// Whether the display accepts updates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Updates {
    /// Apply scans and animation steps.
    Live,
    /// Keep the displayed observations and animation progress.
    Paused,
}

impl Updates {
    /// Toggle between live and paused.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Live => Self::Paused,
            Self::Paused => Self::Live,
        }
    }
}
