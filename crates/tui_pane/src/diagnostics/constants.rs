//! Domain strings for the diagnostics module's perf log.

// perf log
pub(super) const DEFAULT_PERF_LOG_FILTER: &str = "info";
pub(super) const PERF_LOG_ENV: &str = "TUI_PANE_LOG";
/// Tracing target used for framework performance diagnostics.
pub const PERF_LOG_TARGET: &str = "tui_pane::perf";

// tui_pane src diagnostics cpu
/// How many poll samples a [`crate::diagnostics::RollingMean`] window averages.
///
/// A workload running in ~1 s bursts aliases against a 1 s poll cadence
/// — the instantaneous sample swings wildly with phase alignment. At the
/// 1 s cadence this window reads as the average over the last 5 s.
pub const CPU_SMOOTHING_WINDOW_POLLS: usize = 5;
/// Wildcard PDH counter path summing 3-D engine utilization across
/// every GPU engine instance. Uses the English (non-localized) counter
/// names so the query resolves regardless of the system language.
#[cfg(target_os = "windows")]
pub(super) const GPU_COUNTER_PATH: &str = "\\GPU Engine(*engtype_3D)\\Utilization Percentage";
/// `kern_return_t` success code shared by the mach and `IOKit` calls below.
#[cfg(target_os = "macos")]
pub(super) const KERN_SUCCESS: i32 = 0;
/// A per-item `CStatus` indicating a freshly cooked sample.
#[cfg(target_os = "windows")]
pub(super) const PDH_CSTATUS_NEW_DATA: u32 = 0x0000_0001;
/// Request cooked counter values formatted as `f64`.
#[cfg(target_os = "windows")]
pub(super) const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
/// `PdhGetFormattedCounterArrayW` needs a larger buffer (sizing pass).
#[cfg(target_os = "windows")]
pub(super) const PDH_MORE_DATA: u32 = 0x8000_07D2;
/// `ERROR_SUCCESS` / `PDH_CSTATUS_VALID_DATA`.
#[cfg(target_os = "windows")]
pub(super) const PDH_SUCCESS: u32 = 0x0000_0000;
pub(super) const PERCENT_PER_CELL: usize = 10;

// tui_pane src diagnostics perf_log
/// Background-message batch processing exceeding this is logged (ms).
pub const SLOW_BG_BATCH_MS: u128 = 50;
/// Frame paint exceeding this is logged as a slow frame (ms).
pub const SLOW_FRAME_MS: u128 = 30;
/// Per-input-event handling exceeding this is logged (ms).
pub const SLOW_INPUT_EVENT_MS: u128 = 10;
