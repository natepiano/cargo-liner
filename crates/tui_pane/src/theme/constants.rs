use std::time::Duration;

// tui_pane src theme poller
pub(super) const BACKOFF_INTERVAL: Duration = Duration::from_secs(30);
pub(super) const BACKOFF_THRESHOLD: u32 = 10;
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(1500);

// tui_pane src theme registry
/// Name of the built-in dark variant. Stable identifier used by config.
pub const BUILTIN_DARK_NAME: &str = "Default Dark";
/// Name of the built-in high-contrast dark variant.
pub const BUILTIN_HC_DARK_NAME: &str = "High Contrast Dark";
/// Name of the built-in high-contrast light variant.
pub const BUILTIN_HC_LIGHT_NAME: &str = "High Contrast Light";
/// Name of the built-in light variant. Stable identifier used by config.
pub const BUILTIN_LIGHT_NAME: &str = "Default Light";
