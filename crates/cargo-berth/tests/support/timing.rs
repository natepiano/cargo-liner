//! Timing allowances shared by the cargo-berth integration test binaries.

use std::time::Duration;

pub(super) const LOCK_CONTENTION_TOLERANCE: Duration = Duration::from_millis(300);
pub(super) const LOCK_CONTENTION_TOLERANCE_ENVIRONMENT: &str =
    "CARGO_BERTH_TEST_LOCK_CONTENTION_TOLERANCE_MS";
/// How often a test rechecks a condition it is waiting on.
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// CI scheduling headroom, kept below the production ten-second deadline.
pub(super) const SCHEDULING_ALLOWANCE: Duration = Duration::from_secs(5);
