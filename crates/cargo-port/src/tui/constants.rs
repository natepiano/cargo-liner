// Cargo-port role colors live in `crate::tui::theme_roles`.

use std::time::Duration;

// cargo arguments
pub(super) const CARGO_BENCH_FLAG: &str = "--bench";
pub(super) const CARGO_BENCH_SUBCOMMAND: &str = "bench";
pub(super) const CARGO_BIN_FLAG: &str = "--bin";
pub(super) const CARGO_CLEAN_SUBCOMMAND: &str = "clean";
pub(super) const CARGO_COLOR_ALWAYS_FLAG: &str = "--color=always";
pub(super) const CARGO_EXAMPLE_FLAG: &str = "--example";
pub(super) const CARGO_FEATURES_FLAG: &str = "--features";
pub(super) const CARGO_PACKAGE_FLAG: &str = "-p";
pub(super) const CARGO_RELEASE_FLAG: &str = "--release";
pub(super) const CARGO_RUN_SUBCOMMAND: &str = "run";

// dialogs
pub(super) const CONFIRM_DIALOG_HEIGHT: u16 = 3;
pub(super) const FINDER_POPUP_HEIGHT: u16 = 28;
pub(super) const SETTINGS_POPUP_WIDTH: u16 = 90;

// finder
pub(super) const MAX_FINDER_RESULTS: usize = 50;

// performance logs
pub(super) const PERF_LOG_FILE: &str = "cargo-port-tui-perf.log";
pub(super) const PREVIOUS_PERF_LOG_FILE: &str = "cargo-port-tui-perf.prev.log";

// render
/// Show at most this many affected paths in the confirm dialog before a
/// `+N more` row (design plan → "Confirm dialog → uniform rule").
pub(super) const AFFECTED_EXTRAS_VISIBLE_CAP: usize = 5;
/// Separators a cut value never keeps directly before its `…`.
pub(super) const CUT_TRAILING_SEPARATORS: [&str; 8] = [" ·", " —", " –", " -", "-", "_", "/", "|"];
pub(super) const STRIP_SLOT_COUNT: usize = 1;

// settings
/// A toggle row's value while the setting is off.
pub(super) const TOGGLE_OFF: &str = "OFF";
/// A toggle row's value while the setting is on.
pub(super) const TOGGLE_ON: &str = "ON";

// startup
pub(super) const STARTUP_BAR_EMPTY: &str = "░";
pub(super) const STARTUP_BAR_FILLED: &str = "▓";
pub(super) const STARTUP_BAR_WIDTH: usize = 8;
pub(super) const STARTUP_PHASE_CRATES_IO: &str = "crates.io";
pub(super) const STARTUP_PHASE_DISK: &str = "Disk usage";
pub(super) const STARTUP_PHASE_GIT: &str = "Local git repos";
pub(super) const STARTUP_PHASE_GITHUB: &str = "GitHub repos";
pub(super) const STARTUP_PHASE_LANGUAGES: &str = "Languages";
pub(super) const STARTUP_PHASE_LINT: &str = "Lint history";
pub(super) const STARTUP_PHASE_METADATA: &str = "Cargo metadata";
pub(super) const STARTUP_PHASE_TESTS: &str = "Test counts";
/// Show the current item after this delay. A row that finishes faster never
/// shows it, so quick phases stay short.
pub(super) const STARTUP_ROW_DETAIL_DELAY: Duration = Duration::from_secs(1);
/// Keep each startup row visible for this duration after its first progress.
/// Without it, a fast row would flash from 0 to 100%.
pub(super) const STARTUP_ROW_MIN_VISIBLE: Duration = Duration::from_millis(400);
/// Mark a startup phase failed when it exceeds this duration. A hung fetch,
/// `cargo metadata` or tokei run then cannot hold the panel open.
pub(super) const STARTUP_ROW_TIMEOUT: Duration = Duration::from_mins(2);

// table
pub(super) const CI_TIMESTAMP_WIDTH: u16 = 16;

// target kinds
pub(super) const TARGET_KIND_BENCH_LABEL: &str = "bench";
pub(super) const TARGET_KIND_BIN_LABEL: &str = "binary";
pub(super) const TARGET_KIND_EXAMPLE_LABEL: &str = "example";

// theme roles
pub(super) const COLUMN_HEADER: &str = "cargo-port.column.header";
pub(super) const CPU_BAR_LABEL: &str = "cargo-port.cpu.bar-label";
pub(super) const DISCOVERY_SHIMMER: &str = "cargo-port.discovery.shimmer";
pub(super) const GIT_IGNORED: &str = "cargo-port.git.ignored";
pub(super) const GIT_MODIFIED: &str = "cargo-port.git.modified";
pub(super) const GIT_UNTRACKED: &str = "cargo-port.git.untracked";
pub(super) const LANGUAGE_SUBTOTAL: &str = "cargo-port.language.subtotal";
pub(super) const ROLE_KEYS: [&str; 10] = [
    tui_pane::DIM_VALUE_ROLE,
    COLUMN_HEADER,
    CPU_BAR_LABEL,
    DISCOVERY_SHIMMER,
    GIT_IGNORED,
    GIT_MODIFIED,
    GIT_UNTRACKED,
    LANGUAGE_SUBTOTAL,
    SELECTION_TEXT,
    TARGET_BENCH,
];
pub(super) const SELECTION_TEXT: &str = "cargo-port.selection.text";
pub(super) const TARGET_BENCH: &str = "cargo-port.target.bench";
