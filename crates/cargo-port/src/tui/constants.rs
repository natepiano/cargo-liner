// Git colors moved to the theme system: see `tui_pane::git_modified_color`
// / `git_untracked_color` / `git_ignored_color`.

// App-specific popup dimensions.
pub(super) const FINDER_POPUP_HEIGHT: u16 = 28;
pub(super) const SETTINGS_POPUP_WIDTH: u16 = 90;
pub(super) const CONFIRM_DIALOG_HEIGHT: u16 = 3;
pub(super) const CI_TIMESTAMP_WIDTH: u16 = 16;

pub(super) const MAX_FINDER_RESULTS: usize = 50;

// perf log
pub(super) const PERF_LOG_FILE: &str = "cargo-port-tui-perf.log";
pub(super) const PREVIOUS_PERF_LOG_FILE: &str = "cargo-port-tui-perf.prev.log";

// Startup toast phase labels — used as both the tracked-item label and
// key. Completion matches by key via `mark_tracked_item_completed`.
pub(super) const STARTUP_PHASE_DISK: &str = "Disk usage";
pub(super) const STARTUP_PHASE_GIT: &str = "Local git repos";
pub(super) const STARTUP_PHASE_GITHUB: &str = "GitHub repos";
pub(super) const STARTUP_PHASE_LINT: &str = "Lint history";
pub(super) const STARTUP_PHASE_METADATA: &str = "Cargo metadata";
