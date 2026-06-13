use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanPhase {
    #[default]
    Running,
    Complete,
}

impl ScanPhase {
    pub const fn is_complete(self) -> bool { matches!(self, Self::Complete) }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RetrySpawnMode {
    #[default]
    Enabled,
    Disabled,
}

#[cfg(test)]
impl RetrySpawnMode {
    pub const fn is_enabled(self) -> bool { matches!(self, Self::Enabled) }
}

#[derive(Debug)]
pub struct ScanState {
    pub phase:      ScanPhase,
    pub started_at: Instant,
    pub run_count:  u64,
}

impl ScanState {
    pub const fn new(started_at: Instant) -> Self {
        Self {
            phase: ScanPhase::Running,
            started_at,
            run_count: 1,
        }
    }
}
