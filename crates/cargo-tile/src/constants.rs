//! Constants for `cargo-tile`.

use ratatui::style::Modifier;

// configuration
/// Directory under the OS config root holding `config.toml`,
/// `keymap.toml`, and `themes/`.
pub(crate) const CONFIG_DIRNAME: &str = "cargo-tile";
/// App configuration file, read at startup for its `[appearance]` section.
pub(crate) const CONFIG_FILENAME: &str = "config.toml";
/// Id of the built-in dark variant, and the `appearance.dark_theme`
/// default. Defined in [`crate::theme`], not in `tui_pane`: theme
/// content belongs to the app.
pub(crate) const DEFAULT_DARK_THEME: &str = "Default Dark";
/// Id of the built-in high-contrast dark variant.
pub(crate) const DEFAULT_HC_DARK_THEME: &str = "High Contrast Dark";
/// Id of the built-in high-contrast light variant.
pub(crate) const DEFAULT_HC_LIGHT_THEME: &str = "High Contrast Light";
/// Id of the built-in light variant, and the `appearance.light_theme`
/// default.
pub(crate) const DEFAULT_LIGHT_THEME: &str = "Default Light";
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

// iterm2
/// Environment variable naming the terminal emulator in use.
pub(crate) const TERM_PROGRAM_ENV: &str = "TERM_PROGRAM";
/// Value [`TERM_PROGRAM_ENV`] carries inside iTerm2.
pub(crate) const ITERM2_TERM_PROGRAM: &str = "iTerm.app";
/// Environment variable iTerm2 sets to the name of the profile the
/// session started on.
pub(crate) const ITERM2_PROFILE_ENV: &str = "ITERM_PROFILE";
/// iTerm2 profile the app adopts while it runs, when the user has made
/// one by that name. Sharing the binary's name keeps the pairing
/// obvious from the iTerm2 side.
pub(crate) const DEFAULT_ITERM2_PROFILE: &str = BINARY_NAME;

// startup
/// Shown in the settings overlay when a path cannot be resolved on this
/// platform.
pub(crate) const UNRESOLVED_PATH: &str = "unavailable";

// status line and overlays
/// Section heading the keymap overlay gives this app's globals scope.
pub(crate) const APP_GLOBALS_SECTION: &str = "App Shortcuts";
/// Label leading the status line's version note. The spaces around it
/// are its padding -- the framework adds none.
pub(crate) const APP_NAME: &str = " cargo-tile ";
/// Version shown beside [`APP_NAME`], read from the manifest at compile
/// time so a running instance always says which build it is.
pub(crate) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Comment block the keymap editor writes above the generated tables.
pub(crate) const KEYMAP_TOML_HEADER: &str = "\
# cargo-tile keymap configuration\n\
# Edit bindings below. Format: action = \"key\" or \"modifier-key\"\n\
# Modifiers: ctrl, alt, shift.  Examples: \"ctrl-k\", \"shift-tab\", \"q\"\n\
# Chord steps are space-separated, e.g. \"g g\".\n\n";
/// Rows the status line occupies along the bottom of the terminal.
pub(crate) const STATUS_LINE_HEIGHT: u16 = 1;

// tiles
/// Rows the first column fills before a second column opens, when
/// `config.toml` says nothing.
pub(crate) const DEFAULT_INITIAL_ROWS: usize = 4;
/// Ceiling the settings stepper walks `tiles.initial_rows` up to.
pub(crate) const MAX_INITIAL_ROWS: usize = 8;
/// Seconds a finished row stays on screen, greyed, before it goes, when
/// `config.toml` says nothing.
pub(crate) const DEFAULT_FADE_SECONDS: u64 = 3;
/// Floor on `tiles.fade_seconds`. At zero a finished row goes on the
/// scan that notices, which is a legitimate choice rather than a
/// mistake -- some developers want the display to hold only what runs.
pub(crate) const MIN_FADE_SECONDS: u64 = 0;
/// Ceiling the settings stepper walks `tiles.fade_seconds` up to.
pub(crate) const MAX_FADE_SECONDS: u64 = 30;
/// Floor on `tiles.initial_rows`. At one, a second cell opens a second
/// column rather than stacking into a second row.
pub(crate) const MIN_INITIAL_ROWS: usize = 1;
/// Rows one cell standing alone needs: a border line, a line of
/// content, and a border line. A cell with a neighbour below costs one
/// less, because the two share that line.
pub(crate) const MIN_TILE_HEIGHT: u16 = 3;
/// Columns one cell standing alone needs, its two border lines
/// included. A cell with a neighbour to its right costs one less.
pub(crate) const MIN_TILE_WIDTH: u16 = 8;
/// Laid over the comparison buffer to make every cell differ from
/// anything a frame can render, which is what turns the next draw into
/// a full repaint.
///
/// The difference is carried by modifiers rather than by the symbol.
/// An unrenderable symbol would have been the obvious choice, but
/// ratatui measures every symbol's display width and rejects control
/// characters on the way, so the only symbols it accepts are ones a
/// frame could legitimately hold. Nothing in this app blinks, so the
/// combination below is one no rendered cell ever carries.
pub(crate) const REPAINT_SENTINEL: Modifier = Modifier::SLOW_BLINK
    .union(Modifier::RAPID_BLINK)
    .union(Modifier::CROSSED_OUT);
/// How often the screen is redrawn cell for cell rather than by
/// difference.
///
/// ratatui writes only the cells that changed since the last frame, so
/// anything put on this terminal by something other than this app --
/// a pane manager splitting the window, a stray line landing on the
/// same tty -- stays where it is for good: both buffers agree those
/// cells already hold what they should, and nothing ever writes over
/// them. A redraw on this cadence is what repairs that, and it is far
/// enough apart to cost nothing while being well inside the time it
/// takes to notice a smear.
pub(crate) const FULL_REPAINT_SECONDS: u64 = 2;
/// Fixed-point scale a transition's progress is measured on, so the
/// animation needs no floating point.
pub(crate) const PROGRESS_SCALE: u32 = 1000;
/// Written on the summary cell's top border, so the one cell listing
/// every command is named rather than told apart by its contents. A
/// manager's own cell reads much like the summary -- one row per cargo
/// invocation it runs -- and this is what separates them at a glance.
/// It is the only titled cell: a command's cell already says which
/// command it is on every row it draws.
pub(crate) const SUMMARY_CELL_TITLE: &str = "summary";
/// The cell holding the running-cargo table. Cells are numbered from
/// one and fill column by column, so the table is always the first.
pub(crate) const TABLE_CELL: usize = 1;
/// How long one change to the grid takes, however many single-cell
/// steps it propagates through: one step takes all of it, and a longer
/// ripple divides it up between them.
pub(crate) const TILE_ANIMATION_MILLIS: u64 = 720;
/// Floor on one step of a ripple, so a long one still reads as cells
/// moving rather than flickering past.
pub(crate) const MIN_STEP_MILLIS: u64 = 60;
/// Steps the grid queues before it gives up propagating and settles the
/// rest in one move. A whole test suite finishing at once would take
/// longer to walk through cell by cell than anyone would watch.
pub(crate) const MAX_PENDING_STEPS: usize = 64;
/// Kept between a cell's left border and the number it carries, so the
/// number is not flush against the line.
pub(crate) const TILE_NUMBER_INDENT: &str = " ";

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
/// Prefix the binary behind an external subcommand carries.
///
/// `cargo nextest run` does not stay a `cargo` process: cargo replaces
/// itself with `cargo-nextest` rather than spawning it, so the command
/// that was typed is running under a name of its own with no `cargo`
/// left above it. Every tool installed as a cargo subcommand -- mend,
/// clippy, nextest -- reaches the table this way.
pub(crate) const CARGO_SUBCOMMAND_PREFIX: &str = "cargo-";
/// The one `cargo-` binary left out of the table: this one. cargo-tile
/// watching the builds is not one of the builds.
pub(crate) const SELF_PROCESS_NAME: &str = BINARY_NAME;
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
/// Home directory stand-in in the working-directory header.
pub(crate) const HOME_ALIAS: &str = "~";
/// The argument the summary leaves out, in either of the two spellings
/// cargo accepts -- `--manifest-path <path>` and `--manifest-path=<path>`.
pub(crate) const MANIFEST_PATH_FLAG: &str = "--manifest-path";
/// Column headers, in table order. The working directory is not among
/// them: it heads the group of invocations that share it rather than
/// repeating on every row.
pub(crate) const TABLE_HEADERS: [&str; 6] = ["pid", "start", "dur", "compiler", "sub", "command"];
/// Index of the `pid` column in [`TABLE_HEADERS`].
pub(crate) const PID_COLUMN: usize = 0;
/// Index of the `start` column in [`TABLE_HEADERS`].
pub(crate) const START_COLUMN: usize = 1;
/// Index of the `dur` column in [`TABLE_HEADERS`].
pub(crate) const DURATION_COLUMN: usize = 2;
/// Index of the `compiler` column in [`TABLE_HEADERS`].
pub(crate) const COMPILER_COLUMN: usize = 3;
/// Index of the `sub` column in [`TABLE_HEADERS`], which carries how
/// many cargo invocations a command is managing. Blank on the rows that
/// manage nothing, which is most of them.
pub(crate) const MANAGED_COLUMN: usize = 4;
/// Index of the `command` column in [`TABLE_HEADERS`]. It is last, and
/// absorbs whatever width the fitted columns leave.
pub(crate) const COMMAND_COLUMN: usize = 5;
/// Rows the working-directory header above each group's table occupies.
pub(crate) const GROUP_HEADER_HEIGHT: u16 = 1;
/// Rows the column-label row at the top of the pane occupies. There is
/// one for the whole table, not one per working-directory group.
pub(crate) const TABLE_HEADER_HEIGHT: u16 = 1;
/// Blank rows between one working directory's table and the next.
pub(crate) const GROUP_GAP_HEIGHT: u16 = 1;
/// Cells the `\u{d7}` separator occupies in a `compiler` cell.
pub(crate) const COMPILER_SEPARATOR_WIDTH: usize = 1;
/// Blank cells between table columns.
pub(crate) const TABLE_COLUMN_SPACING: u16 = 2;
/// Shown in place of the table when no cargo is running.
pub(crate) const NO_PROCESSES_NOTE: &str = "no cargo processes running";
