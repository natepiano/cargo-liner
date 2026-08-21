//! Constants for `cargo-tile`.

// configuration
/// Directory under the OS config root holding `config.toml`,
/// `keymap.toml`, and `themes/`.
pub(crate) const CONFIG_DIRNAME: &str = "cargo-tile";
/// App configuration file, read at startup for its `[appearance]` section.
pub(crate) const CONFIG_FILENAME: &str = "config.toml";
/// Keymap overrides loaded by [`tui_pane::KeymapBuilder::load_toml`].
pub(crate) const KEYMAP_FILENAME: &str = "keymap.toml";
/// Per-user theme directory scanned by
/// [`tui_pane::ThemeRegistry::from_dir_with_builtins`].
pub(crate) const THEMES_DIRNAME: &str = "themes";

// settings overlay
/// Values `appearance.mode` cycles through, in stepper order.
pub(crate) const APPEARANCE_MODES: [&str; 3] = ["auto", "light", "dark"];
/// Rows of popup border above and below the settings body.
pub(crate) const POPUP_CHROME_HEIGHT: u16 = 2;
/// Columns of popup border left and right of the settings body.
pub(crate) const POPUP_CHROME_WIDTH: u16 = 2;
/// Cells the selection cursor occupies to the left of a row label.
pub(crate) const CURSOR_WIDTH: usize = 2;
/// Cells between a row label and its value.
pub(crate) const LABEL_VALUE_GAP: usize = 2;
/// Cells `< ` and ` >` add around a stepper row's value.
pub(crate) const STEPPER_DECORATION_WIDTH: usize = 4;
/// Minimum width of the settings popup in cells. Long rows widen it, a
/// narrow terminal caps it.
pub(crate) const SETTINGS_POPUP_WIDTH: u16 = 64;

// lifecycle
/// Fallback executable name when the running binary's path cannot be
/// resolved for a restart.
pub(crate) const BINARY_NAME: &str = "cargo-tile";

// startup
/// Shown in the settings overlay when a path cannot be resolved on this
/// platform.
pub(crate) const UNRESOLVED_PATH: &str = "unavailable";

// status line and overlays
/// Section heading the keymap overlay gives this app's globals scope.
pub(crate) const APP_GLOBALS_SECTION: &str = "App Shortcuts";
/// Rows the status line occupies along the bottom of the terminal.
pub(crate) const STATUS_LINE_HEIGHT: u16 = 1;

/// Comment block the keymap editor writes above the generated tables.
pub(crate) const KEYMAP_TOML_HEADER: &str = "\
# cargo-tile keymap configuration\n\
# Edit bindings below. Format: action = \"key\" or \"modifier-key\"\n\
# Modifiers: ctrl, alt, shift.  Examples: \"ctrl-k\", \"shift-tab\", \"q\"\n\
# Chord steps are space-separated, e.g. \"g g\".\n\n";
// running-cargo table
/// Process names that are the genuine cargo binary.
///
/// Matching on the process's own name rather than on its arguments keeps
/// the wrappers around an invocation — a shim shell script, `script`, a
/// login shell — out of the table, so each command appears exactly once.
/// `cargo-tile-real` is here because a toolchain hook that interposes a
/// shim at `~/.rustup/toolchains/*/bin/cargo` renames the binary it
/// wraps, and the renamed process is still the cargo doing the work.
pub(crate) const CARGO_PROCESS_NAMES: [&str; 2] = ["cargo", "cargo-tile-real"];
/// What a cargo binary is called in the `command` column, whatever the
/// name it happens to be installed under.
pub(crate) const CARGO_DISPLAY_NAME: &str = "cargo";
/// Compiler driver names counted under each cargo invocation, in
/// reporting priority order: with a wrapper in use every `rustc` is a
/// child of one, so `sccache` wins to avoid counting a compile twice.
pub(crate) const COMPILER_PROCESS_NAMES: [&str; 2] = ["sccache", "rustc"];
/// How far up a parent chain to look for the cargo process that owns a
/// compiler, bounding the walk against a reparented cycle.
pub(crate) const PARENT_WALK_LIMIT: usize = 32;
/// Delay between process scans. Each scan reads the costly per-process
/// fields for cargo processes only, so a quarter second stays cheap while
/// keeping a freshly started build visible almost immediately.
pub(crate) const PROCESS_POLL_MILLIS: u64 = 250;
/// `chrono` format for the `start` column.
pub(crate) const START_TIME_FORMAT: &str = "%H:%M";
/// Seconds in a minute, for splitting a run time into its parts.
pub(crate) const SECONDS_PER_MINUTE: u64 = 60;
/// Seconds in an hour, past which `dur` widens to `hh:mm:ss`.
pub(crate) const SECONDS_PER_HOUR: u64 = 3600;
/// Shown in `start` when a process's timestamp cannot be interpreted.
pub(crate) const UNRESOLVED_TIME: &str = "--:--";
/// Home directory stand-in in the `path` column.
pub(crate) const HOME_ALIAS: &str = "~";
/// Column headers, in table order.
pub(crate) const TABLE_HEADERS: [&str; 6] = ["path", "pid", "start", "dur", "compiler", "command"];
/// Index of the `path` column in [`TABLE_HEADERS`].
pub(crate) const PATH_COLUMN: usize = 0;
/// Index of the `pid` column in [`TABLE_HEADERS`].
pub(crate) const PID_COLUMN: usize = 1;
/// Index of the `start` column in [`TABLE_HEADERS`].
pub(crate) const START_COLUMN: usize = 2;
/// Index of the `dur` column in [`TABLE_HEADERS`].
pub(crate) const DURATION_COLUMN: usize = 3;
/// Index of the `compiler` column in [`TABLE_HEADERS`].
pub(crate) const COMPILER_COLUMN: usize = 4;
/// Index of the `command` column in [`TABLE_HEADERS`]. It is last, and
/// absorbs whatever width the fitted columns leave.
pub(crate) const COMMAND_COLUMN: usize = 5;
/// Cells the `\u{d7}` separator occupies in a `compiler` cell.
pub(crate) const COMPILER_SEPARATOR_WIDTH: usize = 1;
/// Blank cells between table columns.
pub(crate) const TABLE_COLUMN_SPACING: u16 = 2;
/// Shown in place of the table when no cargo is running.
pub(crate) const NO_PROCESSES_NOTE: &str = "no cargo processes running";
