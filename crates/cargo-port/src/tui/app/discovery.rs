use std::time::Duration;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiscoveryShimmer {
    pub started_at: Instant,
    pub duration:   Duration,
}

impl DiscoveryShimmer {
    pub const fn new(started_at: Instant, duration: Duration) -> Self {
        Self {
            started_at,
            duration,
        }
    }

    pub fn is_active_at(self, now: Instant) -> bool {
        now.duration_since(self.started_at) < self.duration
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryRowKind {
    Root,
    WorktreeEntry,
    PathOnly,
}

impl DiscoveryRowKind {
    pub(super) const fn allows_parent_kind(self, kind: Self) -> bool {
        matches!(
            (self, kind),
            (Self::Root, Self::Root)
                | (Self::WorktreeEntry, Self::WorktreeEntry)
                | (Self::PathOnly, Self::PathOnly)
        )
    }

    pub(super) const fn discriminant(self) -> u8 {
        match self {
            Self::Root => 0,
            Self::WorktreeEntry => 1,
            Self::PathOnly => 2,
        }
    }
}
