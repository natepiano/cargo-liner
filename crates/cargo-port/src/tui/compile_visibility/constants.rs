//! Values the compile-monitor scope lifetime and its refresh cadence share.

use std::time::Duration;

// monitor refresh
/// How often an enabled compile monitor asks the shared process-refresh
/// executor for a freshly classified cycle. A disabled monitor owns no
/// schedule at all, so this value is only ever read through
/// `ActiveMonitorState`.
pub(super) const COMPILE_MONITOR_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
