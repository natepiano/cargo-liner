// Cargo-port role colors live in `crate::tui::theme_roles`.

use std::time::Duration;

// App-specific popup dimensions.
pub(super) const FINDER_POPUP_HEIGHT: u16 = 28;
pub(super) const SETTINGS_POPUP_WIDTH: u16 = 90;
pub(super) const CONFIRM_DIALOG_HEIGHT: u16 = 3;
pub(super) const CI_TIMESTAMP_WIDTH: u16 = 16;

pub(super) const MAX_FINDER_RESULTS: usize = 50;

// perf log
pub(super) const PERF_LOG_FILE: &str = "cargo-port-tui-perf.log";
pub(super) const PREVIOUS_PERF_LOG_FILE: &str = "cargo-port-tui-perf.prev.log";

// startup panel
pub(super) const STARTUP_BAR_EMPTY: &str = "░";
pub(super) const STARTUP_BAR_FILLED: &str = "▓";
pub(super) const STARTUP_BAR_WIDTH: usize = 8;
/// A row that completes faster than this would flash 0 → 100%; the panel
/// holds every row visible at least this long past its first progress.
pub(super) const STARTUP_ROW_MIN_VISIBLE: Duration = Duration::from_millis(400);
/// A still-in-progress row that has been visible at least this long appends
/// the item it is currently working on (or about to) after its percent. Rows
/// that finish faster never show it, so quick phases stay terse.
pub(super) const STARTUP_ROW_DETAIL_DELAY: Duration = Duration::from_secs(1);
/// A generous uniform backstop so a hung phase (stalled fetch, hung
/// `cargo metadata`, slow tokei) cannot hold the panel open forever. Sized
/// so legitimately slow work finishes well inside it; a phase that exceeds
/// it is marked failed and the panel finishes.
pub(super) const STARTUP_ROW_TIMEOUT: Duration = Duration::from_mins(2);

// Startup panel row labels, in display order within each group.
pub(super) const STARTUP_PHASE_CRATES_IO: &str = "crates.io";
pub(super) const STARTUP_PHASE_DISK: &str = "Disk usage";
pub(super) const STARTUP_PHASE_GIT: &str = "Local git repos";
pub(super) const STARTUP_PHASE_GITHUB: &str = "GitHub repos";
pub(super) const STARTUP_PHASE_LANGUAGES: &str = "Languages";
pub(super) const STARTUP_PHASE_LINT: &str = "Lint history";
pub(super) const STARTUP_PHASE_METADATA: &str = "Cargo metadata";
pub(super) const STARTUP_PHASE_TESTS: &str = "Test counts";
