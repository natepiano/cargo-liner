//! Scheduling tolerance for tests that race a competing mutation-lock owner.

use std::time::Duration;

pub(super) const LOCK_CONTENTION_TOLERANCE: Duration = Duration::from_millis(300);
