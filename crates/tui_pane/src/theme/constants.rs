use std::time::Duration;

// tui_pane src theme poller
pub(super) const BACKOFF_INTERVAL: Duration = Duration::from_secs(30);
pub(super) const BACKOFF_THRESHOLD: u32 = 10;
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(1500);
