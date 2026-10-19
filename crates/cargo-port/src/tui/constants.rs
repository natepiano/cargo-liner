// Cargo-port role colors live in `crate::tui::theme_roles`.

use std::time::Duration;

// `FINDER_POPUP_HEIGHT`, `SETTINGS_POPUP_WIDTH` and `CONFIRM_DIALOG_HEIGHT` size
// dialogs; `CI_TIMESTAMP_WIDTH` sizes CI timestamps.
pub(super) const FINDER_POPUP_HEIGHT: u16 = 28;
pub(super) const SETTINGS_POPUP_WIDTH: u16 = 90;
pub(super) const CONFIRM_DIALOG_HEIGHT: u16 = 3;
pub(super) const CI_TIMESTAMP_WIDTH: u16 = 16;

// `TOGGLE_ON` and `TOGGLE_OFF` supply the values displayed by settings toggle rows.
/// A toggle row's value while the setting is on.
pub(super) const TOGGLE_ON: &str = "ON";
/// A toggle row's value while the setting is off.
pub(super) const TOGGLE_OFF: &str = "OFF";

pub(super) const MAX_FINDER_RESULTS: usize = 50;

// `CARGO_*_FLAG` and `CARGO_*_SUBCOMMAND` constants supply Cargo invocation arguments.
pub(super) const CARGO_BENCH_FLAG: &str = "--bench";
pub(super) const CARGO_BENCH_SUBCOMMAND: &str = "bench";
pub(super) const CARGO_CLEAN_SUBCOMMAND: &str = "clean";
pub(super) const CARGO_COLOR_ALWAYS_FLAG: &str = "--color=always";
pub(super) const CARGO_EXAMPLE_FLAG: &str = "--example";
pub(super) const CARGO_FEATURES_FLAG: &str = "--features";
pub(super) const CARGO_PACKAGE_FLAG: &str = "-p";
pub(super) const CARGO_RELEASE_FLAG: &str = "--release";
pub(super) const CARGO_RUN_SUBCOMMAND: &str = "run";

// `TARGET_KIND_BENCH_LABEL`, `TARGET_KIND_BIN_LABEL` and `TARGET_KIND_EXAMPLE_LABEL`
// identify Cargo target kinds.
pub(super) const TARGET_KIND_BENCH_LABEL: &str = "bench";
pub(super) const TARGET_KIND_BIN_LABEL: &str = "bin";
pub(super) const TARGET_KIND_EXAMPLE_LABEL: &str = "example";

// `PERF_LOG_FILE` and `PREVIOUS_PERF_LOG_FILE` name current and previous TUI performance logs.
pub(super) const PERF_LOG_FILE: &str = "cargo-port-tui-perf.log";
pub(super) const PREVIOUS_PERF_LOG_FILE: &str = "cargo-port-tui-perf.prev.log";

// `STARTUP_BAR_EMPTY`, `STARTUP_BAR_FILLED` and `STARTUP_BAR_WIDTH` define the startup progress
// bar.
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

// `STARTUP_PHASE_*` constants label startup panel rows in display order within each group.
pub(super) const STARTUP_PHASE_CRATES_IO: &str = "crates.io";
pub(super) const STARTUP_PHASE_DISK: &str = "Disk usage";
pub(super) const STARTUP_PHASE_GIT: &str = "Local git repos";
pub(super) const STARTUP_PHASE_GITHUB: &str = "GitHub repos";
pub(super) const STARTUP_PHASE_LANGUAGES: &str = "Languages";
pub(super) const STARTUP_PHASE_LINT: &str = "Lint history";
pub(super) const STARTUP_PHASE_METADATA: &str = "Cargo metadata";
pub(super) const STARTUP_PHASE_TESTS: &str = "Test counts";

// `render` uses `AFFECTED_EXTRAS_VISIBLE_CAP` to limit confirmation details and
// `STRIP_SLOT_COUNT` to size the status strip's global-shortcut slots.
/// Maximum affected-checkout paths shown explicitly in the confirm
/// dialog before collapsing the tail into a `+N more` line (design
/// plan → "Confirm dialog → uniform rule").
pub(super) const AFFECTED_EXTRAS_VISIBLE_CAP: usize = 5;
pub(super) const STRIP_SLOT_COUNT: usize = 1;

// `theme_roles` maps the following role-key constants, collected in `ROLE_KEYS`,
// to application colors.
pub(super) const COLUMN_HEADER: &str = "cargo-port.column.header";
pub(super) const DISCOVERY_SHIMMER: &str = "cargo-port.discovery.shimmer";
pub(super) const GIT_IGNORED: &str = "cargo-port.git.ignored";
pub(super) const GIT_MODIFIED: &str = "cargo-port.git.modified";
pub(super) const GIT_UNTRACKED: &str = "cargo-port.git.untracked";
pub(super) const LANGUAGE_SUBTOTAL: &str = "cargo-port.language.subtotal";
pub(super) const ROLE_KEYS: [&str; 7] = [
    COLUMN_HEADER,
    DISCOVERY_SHIMMER,
    GIT_IGNORED,
    GIT_MODIFIED,
    GIT_UNTRACKED,
    LANGUAGE_SUBTOTAL,
    TARGET_BENCH,
];
pub(super) const TARGET_BENCH: &str = "cargo-port.target.bench";
