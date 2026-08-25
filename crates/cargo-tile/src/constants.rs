//! Constants for `cargo-tile`.

use std::time::Duration;

use ratatui::style::Modifier;

// attract screen
/// How far the attract screen's strip is carried toward or away from
/// full strength each frame. Divides the range exactly, so the fade
/// lands on either end rather than saturating out its last frame, and
/// at the frame poll's cadence it crosses the whole thing in a little
/// over half a second -- long enough to read as the screen changing
/// hands rather than as a cut.
pub(crate) const ATTRACT_FADE_STEP: u8 = 3;
/// TOML table the moving band's keys are read from and written back to.
/// Stable -- `keymap.toml` is hand-edited.
pub(crate) const ATTRACT_MOVING_BAND_SCOPE: &str = "attract_moving_band";
/// Section heading the keymap overlay gives the moving band's keys.
pub(crate) const ATTRACT_MOVING_BAND_SECTION: &str = "Attract: Moving Band";
/// TOML table the drifting text's keys are read from and written back
/// to. Stable -- `keymap.toml` is hand-edited.
pub(crate) const ATTRACT_MOVING_TEXT_SCOPE: &str = "attract_moving_text";
/// Section heading the keymap overlay gives the drifting text's keys.
pub(crate) const ATTRACT_MOVING_TEXT_SECTION: &str = "Attract: Moving Text";
/// Cells per second one step of the faster / slower keys moves the
/// band.
pub(crate) const BAND_SPEED_STEP: u32 = 2;
/// How much one step of the fray-faster / fray-slower keys moves the
/// band's trailing edge, on the per-second scale that speed is held in.
pub(crate) const BAND_TAIL_SPEED_STEP: u32 = 24;
/// Cells one step of the wider / thinner keys moves the band.
pub(crate) const BAND_WIDTH_STEP: u32 = 1;
/// Cells per second one step of the faster / slower keys moves the
/// drifting text. Smaller than the band's step because the range it
/// walks is half as long.
pub(crate) const TEXT_SPEED_STEP: u32 = 1;
/// Percentage points one step of the spread keys moves how far the
/// text's lines' own speeds stand from the field's.
pub(crate) const TEXT_SPREAD_STEP: u32 = 5;
/// Longest gap between two presses of the same steering key that still
/// reads as the key being held down. A terminal's own auto-repeat runs
/// at roughly twice this rate once it gets going, and a reader tapping
/// a key deliberately is slower than it.
pub(crate) const HELD_KEY_GAP: Duration = Duration::from_millis(200);
/// Most steps one press of a held steering key is ever worth.
pub(crate) const HELD_KEY_MAX_STEP: u32 = 8;
/// Presses a held steering key spends at each step size before it is
/// worth one more.
pub(crate) const HELD_KEY_PRESSES_PER_STEP: u32 = 4;

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
/// The `commands.hidden_when_idle` default: subcommands the grid gives a
/// cell of their own only while they are driving other cargo
/// invocations. The sibling terminal UI is the case it exists for -- it
/// is open all day and compiles nothing on its own, so an idle cell for
/// it is a cell no build is getting. The summary still carries it: one
/// line saying it is running is the whole of what it has to say.
pub(crate) const DEFAULT_HIDDEN_WHEN_IDLE: [&str; 1] = [SIBLING_SUBCOMMAND_NAME];
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
/// What separates a list setting's entries where the overlay reports
/// one. Config lists are edited in the file rather than stepped, so
/// this is for reading only.
pub(crate) const LIST_SEPARATOR: &str = ", ";
/// Shown in place of a list setting the user has emptied, an empty row
/// being indistinguishable from a broken one.
pub(crate) const EMPTY_LIST: &str = "none";
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
/// The binary's own name: what the command line calls itself in help
/// and in anything it reports going wrong, and the fallback executable
/// name when the running binary's path cannot be resolved for a
/// restart. Distinct from [`APP_NAME`], which is padded for the status
/// line.
pub(crate) const BINARY_NAME: &str = "cargo-tile";
/// The word cargo knows this tool by, which is the binary's name with
/// cargo's own prefix taken off. Cargo runs `cargo tile ...` by finding
/// `cargo-tile` on the path and handing it this word ahead of every
/// other argument, so the command line drops it before parsing.
pub(crate) const SUBCOMMAND_NAME: &str = "tile";
/// The sibling terminal UI in this workspace, reached as `cargo port`.
/// The capture shim passes it through for the same reason it passes the
/// grid through: capturing a terminal UI copies every redraw of it into
/// a log for as long as it stays open. It is also what
/// [`DEFAULT_HIDDEN_WHEN_IDLE`] names, being the command that runs all
/// day without compiling anything.
pub(crate) const SIBLING_SUBCOMMAND_NAME: &str = "port";

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
/// Section heading the keymap overlay gives the navigation scope.
pub(crate) const NAVIGATION_SECTION: &str = "Navigation";
/// Rows the status line occupies along the bottom of the terminal.
pub(crate) const STATUS_LINE_HEIGHT: u16 = 1;

// tiles
/// Rows the grid grows to in a single column before it starts
/// arranging itself into a square, when `config.toml` says nothing.
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
/// Floor on `tiles.initial_rows`. At one there is no single-column
/// stretch at all: the grid arranges itself into a square from the
/// second cell on.
pub(crate) const MIN_INITIAL_ROWS: usize = 1;
/// Rows one cell standing alone needs: a border line, a line of
/// content, and a border line. A cell with a neighbour below costs one
/// less, because the two share that line.
pub(crate) const MIN_TILE_HEIGHT: u16 = 3;
/// Columns one cell standing alone needs, its two border lines
/// included. A cell with a neighbour to its right costs one less.
pub(crate) const MIN_TILE_WIDTH: u16 = 8;
/// Rows a cell spends on its own border, which is what its contents
/// have to be given on top of. Two, though neighbours share one of
/// them, so a stacked cell costs less than this and the division is a
/// little conservative rather than a little short.
pub(crate) const TILE_BORDER_ROWS: u16 = 2;
/// Rows of content one step of demand is worth. A cell asks for its
/// content rounded up to a whole number of these, so a column
/// re-divides on the move from a quiet cell to a busy one rather than
/// on every scan that added a row or rewrapped a command line.
pub(crate) const TILE_DEMAND_STEP: usize = 3;
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

/// How long a frame has to have taken before [`crate::probe`] writes it
/// down, where the probe is switched on at all.
///
/// A little over three frames at the poll interval, which is where a
/// gap stops reading as one frame arriving late and starts reading as
/// the display having stopped.
pub(crate) const PROBE_THRESHOLD: Duration = Duration::from_millis(33);
/// Fixed-point scale a transition's progress is measured on, so the
/// animation needs no floating point.
pub(crate) const PROGRESS_SCALE: u32 = 1000;
/// Written on the summary cell's top border, so the one cell listing
/// every command is named rather than told apart by its contents. A
/// manager's own cell reads much like the summary -- one row per cargo
/// invocation it runs -- and this is what separates them at a glance.
/// It is the only titled cell: a command's cell already says which
/// command it is on every row it draws. The leading space holds the
/// word off the corner glyph the title is set against.
pub(crate) const SUMMARY_CELL_TITLE: &str = " summary";
/// Cells of the summary cell's top border the sccache label never
/// gets: the two corner glyphs, and one line cell kept between the
/// label and the right corner so the two do not run together.
pub(crate) const SUMMARY_LABEL_BORDER_RESERVE: u16 = 3;
/// Cells between the sccache label's last character and the summary
/// cell's top-right corner -- the corner glyph itself, and the line
/// cell kept clear in front of it.
pub(crate) const SUMMARY_LABEL_RIGHT_INSET: u16 = 2;
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
/// How long the resize a step of the focus ring asks for takes.
///
/// Far shorter than [`TILE_ANIMATION_MILLIS`], because the two are
/// different events: a cell opening or closing happens on its own and
/// wants watching, while this one is a key the developer just pressed
/// and is already pressing again. At the full travel, holding an arrow
/// down would leave the grid still settling from the first press.
pub(crate) const FOCUS_ANIMATION_MILLIS: u64 = 140;
/// Steps the grid queues before it gives up propagating and settles the
/// rest in one move. A whole test suite finishing at once would take
/// longer to walk through cell by cell than anyone would watch.
pub(crate) const MAX_PENDING_STEPS: usize = 64;
/// Kept between a cell's left border and the number it carries, so the
/// number is not flush against the line.
pub(crate) const TILE_NUMBER_INDENT: &str = " ";
/// Ahead of what a cell's contents ask for, in the readout along the
/// foot of every cell.
pub(crate) const TILE_ROWS_CONTENT_LABEL: &str = "content rows: ";
/// Ahead of the cell's own size in the same readout, written as rows
/// over columns.
pub(crate) const TILE_ROWS_CELL_LABEL: &str = "  r/c: ";
/// Between those two numbers.
pub(crate) const TILE_ROWS_CELL_SEPARATOR: &str = "/";
/// Ahead of the width the demand was measured at, which is written only
/// where it is not the width the cell was drawn at. The two agreeing is
/// the ordinary case and says nothing; the two disagreeing is the one
/// way the counts can differ without either being wrong on its own
/// terms, and is worth the room it takes.
pub(crate) const TILE_ROWS_WIDTH_LABEL: &str = " @ ";
/// Kept between that readout and the cell's right border, so it is not
/// flush against the line.
pub(crate) const TILE_ROWS_RIGHT_INSET: u16 = 1;
/// Rows the readout takes: it is one line along the foot of the cell.
pub(crate) const TILE_ROWS_READOUT_HEIGHT: u16 = 1;

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
/// What marks the toolchain selector cargo takes ahead of a subcommand,
/// as in `cargo +nightly build`. Reading a subcommand out of an argument
/// list means stepping over one of these first.
pub(crate) const CARGO_TOOLCHAIN_SELECTOR: char = '+';
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
pub(crate) const COMPILER_PROCESS_NAMES: [&str; 2] = [SCCACHE_BINARY, "rustc"];
/// The process every tree on this machine roots at -- `launchd` on
/// macOS, `init` on Linux. An ancestry walk stops short of it: a row
/// naming the one process everything descends from tells one command
/// from no other.
pub(crate) const ROOT_PROCESS_PID: u32 = 1;
/// Added to a cell's indent once per level of the ancestry block, so
/// the chain reads as a staircase down to the command.
pub(crate) const ANCESTRY_LEVEL_INDENT: &str = " ";
/// Blank rows between the ancestry block and the table under it.
pub(crate) const ANCESTRY_GAP_HEIGHT: u16 = 1;
/// Rows the ancestry block needs before it can drop levels out of the
/// middle rather than off the end: one for the top-level parent, one
/// for the elision, and one for the level nearest the command.
pub(crate) const ANCESTRY_MIN_ELIDED_ROWS: usize = 3;
/// How many times a cell's demand re-measures its ancestry block
/// against the cell its own last answer would have bought.
///
/// The block is given half the cell, so what it draws depends on a
/// height the demand is itself deciding. Each pass hands the block the
/// budget the pass before it worked out, and each answer is no larger
/// than the one before -- a smaller cell buys a smaller budget, which
/// fits no more levels -- so the sequence settles, usually on the second
/// pass. This bounds it anyway: a render path is no place to discover
/// that an assumption about convergence was wrong.
pub(crate) const ANCESTRY_DEMAND_PASSES: usize = 8;
/// What stands in the ancestry block for the levels a short cell has no
/// room to draw.
pub(crate) const ANCESTRY_ELISION: &str = "\u{2026}";
/// Process names an ancestry chain steps over: the shells a command is
/// typed into, and the login process a terminal session starts under.
/// None of them launched anything -- they passed a command through --
/// and a row for one says only that a terminal was involved, which the
/// row above it already said.
pub(crate) const TRANSPARENT_PROCESS_NAMES: [&str; 9] = [
    "login", "sh", "bash", "zsh", "dash", "fish", "ksh", "csh", "tcsh",
];
/// How far up a parent chain to look for the cargo process that owns a
/// compiler, bounding the walk against a reparented cycle.
pub(crate) const PARENT_WALK_LIMIT: usize = 32;
/// Delay between process scans. Each scan reads the costly per-process
/// fields for cargo processes only, so a quarter second stays cheap while
/// keeping a freshly started build visible almost immediately.
pub(crate) const PROCESS_POLL_MILLIS: u64 = 250;
/// How long a `cpu` reading is carried before the table takes a fresh
/// one, in milliseconds.
///
/// Separate from [`CPU_SMOOTHING_SECONDS`], which is how fast the
/// reading moves, and from [`PROCESS_POLL_MILLIS`], which is how often
/// it is sampled: every sample still moves the reading, and what this
/// settles is how often the column is allowed to say something new. A
/// number redrawn four times a second is unreadable however smooth the
/// figures behind it are -- the eye is given nothing to rest on.
pub(crate) const CPU_REPORT_MILLIS: u64 = 1000;
/// How long a `cpu` reading takes to travel most of the way to a share
/// that has changed, in seconds.
///
/// A quarter-second sample of a long-running command that works in
/// bursts -- a watcher waking to check something, then going back to
/// sleep -- is nearly all sampling artefact: the same steady command
/// reads nought, then twelve, then two, and a column of that reports
/// jitter rather than load. The reading is settled toward each sample
/// instead of taking it, over a window long enough to average a burst
/// out and short enough that a build ramping up is not left behind.
pub(crate) const CPU_SMOOTHING_SECONDS: f32 = 2.0;
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
const MANIFEST_PATH_FLAG: &str = "--manifest-path";
/// Flags the summary leaves out, with the word each of them takes.
/// Both spellings count: `--color always` and `--color=always`.
///
/// What they have in common is that none of them say anything about
/// what is being built. A `--manifest-path` names the directory already
/// heading the row, in full and in absolute form; a `--message-format`
/// or `--color` names how the caller wanted the output rendered, which
/// is the caller's business rather than the run's. Between them they
/// are long enough to push the subcommand -- the one word worth
/// reading -- off the edge of a narrow cell.
///
/// Only these. Everything else stands, however long: `-p` names which
/// member of a workspace is being built and `--all-targets` names how
/// much of it, which is exactly what the row is there to say.
pub(crate) const SUMMARY_HIDDEN_VALUED_FLAGS: [&str; 3] =
    [MANIFEST_PATH_FLAG, "--message-format", "--color"];
/// The bare `--` handing everything after it to whatever cargo runs.
/// Those arguments are the other program's, so the summary passes them
/// through untouched however they are spelled.
pub(crate) const ARGUMENT_SEPARATOR: &str = "--";
/// Column headers, in table order. The working directory is not among
/// them: it heads the group of invocations that share it rather than
/// repeating on every row.
pub(crate) const TABLE_HEADERS: [&str; 9] = [
    "pid", "parent", "start", "dur", "cpu", "state", "command", "compiler", "runs",
];
/// Index of the `pid` column in [`TABLE_HEADERS`].
pub(crate) const PID_COLUMN: usize = 0;
/// Index of the `parent` column in [`TABLE_HEADERS`], which names what
/// started an invocation: the cargo above it where there is one, and
/// otherwise the foot of the chain drawn over the table. It stands
/// beside `pid` because the two are read together -- the eye follows a
/// row's parent up to whatever carries that number.
pub(crate) const PARENT_COLUMN: usize = 1;
/// Index of the `start` column in [`TABLE_HEADERS`].
pub(crate) const START_COLUMN: usize = 2;
/// Index of the `dur` column in [`TABLE_HEADERS`].
pub(crate) const DURATION_COLUMN: usize = 3;
/// Index of the `cpu` column in [`TABLE_HEADERS`], which carries the
/// share of a core the invocation and everything under it are using. It
/// stands beside `dur` because the two answer the same question from
/// either end -- how long this has been going, and how hard.
pub(crate) const CPU_COLUMN: usize = 4;
/// Index of the `state` column in [`TABLE_HEADERS`], which says that a
/// command is waiting on another cargo's lock. How far along a command
/// is goes on the working-directory heading over it instead. It is the
/// one column that comes and goes, and joins only where a row is
/// waiting -- an empty column costs a narrow tile the width its command
/// line needs and reports nothing. Every row but the waiting one leaves
/// the cell blank, so the one word in the column is the only thing in
/// it.
pub(crate) const STATE_COLUMN: usize = 5;
/// Index of the `command` column in [`TABLE_HEADERS`]. It absorbs
/// whatever width the fitted columns leave, wherever it stands among
/// them, so it comes ahead of the two that describe an invocation
/// rather than after them.
pub(crate) const COMMAND_COLUMN: usize = 6;
/// Index of the `compiler` column in [`TABLE_HEADERS`].
pub(crate) const COMPILER_COLUMN: usize = 7;
/// Index of the `runs` column in [`TABLE_HEADERS`], which carries how
/// many cargo invocations a command is managing. Blank on the rows that
/// manage nothing, which is most of them.
pub(crate) const MANAGED_COLUMN: usize = 8;
/// Columns the summary leaves out. One row there stands for a whole
/// command rather than for a single invocation, and each of these
/// describes an invocation: what is compiling under it at this instant,
/// how many invocations it is managing, and which cargo started it.
/// The command's own cell has the room to say so, and the summary
/// spends that width on the command line instead.
///
/// `parent` has nothing to point at there. A pid in that column is only
/// worth reading because the screen shows it somewhere else -- another
/// row of the same table, or the ancestry chain over it -- and the
/// summary has neither: it draws one row per command, over every
/// directory at once, with no chain above it. What ties a summary row
/// to the command's own cell is the colour on `pid`, which it keeps.
pub(crate) const SUMMARY_HIDDEN_COLUMNS: [usize; 3] =
    [PARENT_COLUMN, COMPILER_COLUMN, MANAGED_COLUMN];
/// What the status line says while the display is held still. It stands
/// alone rather than labelling a value: there is nothing to report but
/// that nothing is being reported.
pub(crate) const FROZEN_NOTE_LABEL: &str = "frozen";
/// What the status line says while the attract screen is being shown
/// because it was asked for. Stands alone for the same reason
/// [`FROZEN_NOTE_LABEL`] does, and says the same kind of thing: the
/// grid is still there, it is being drawn over.
pub(crate) const ATTRACT_NOTE_LABEL: &str = "attract";
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

// capture shim
/// Name of the real cargo once the shim takes its place beside it. The
/// shim resolves it as a sibling, so a hardcoded-path invocation of a
/// toolchain's cargo is captured the same as one found through `PATH`.
pub(crate) const REAL_CARGO_NAME: &str = "cargo-tile-real";
/// The binary the shim stands in for, in each toolchain's `bin`.
pub(crate) const CARGO_NAME: &str = "cargo";
/// Line the installer recognises its own shim by. A `cargo` without it
/// is the real binary, whatever else it may be, and is moved aside
/// rather than overwritten.
pub(crate) const SHIM_MARKER: &str = "cargo-tile-capture-shim";
/// Permissions the shim is written with: readable and executable by
/// all, writable by its owner.
pub(crate) const SHIM_MODE: u32 = 0o755;
/// Directory under the rustup home holding one directory per toolchain.
pub(crate) const TOOLCHAINS_DIR: &str = "toolchains";
/// Directory under a toolchain holding its binaries.
pub(crate) const TOOLCHAIN_BIN_DIR: &str = "bin";
/// Where rustup keeps its toolchains, under the home directory.
pub(crate) const RUSTUP_DIRNAME: &str = ".rustup";
/// Environment variable moving the rustup home away from
/// [`RUSTUP_DIRNAME`].
pub(crate) const RUSTUP_HOME_ENV: &str = "RUSTUP_HOME";
/// How much of a `cargo` is read to look for [`SHIM_MARKER`]. The real
/// cargo is a thirty-megabyte binary and the marker is in the shim's
/// opening comment, so there is no reason to read further.
pub(crate) const SHIM_MARKER_SEARCH_BYTES: usize = 1024;

// build progress
/// Directory under [`CAPTURE_ROOT`], one file per run still in flight,
/// each named for the pid of the shim that captured it.
pub(crate) const CAPTURE_LIVE_RUNS_DIR: &str = "state/pids";
/// Where the cargo shim mirrors each run's output. Under `/tmp` rather
/// than the home directory because a sandboxed caller can write there.
pub(crate) const CAPTURE_ROOT: &str = "/tmp/cargo-tile";
/// Environment variable moving [`CAPTURE_ROOT`], which is what puts a
/// second grid on captures of its own.
pub(crate) const CAPTURE_ROOT_ENV: &str = "CARGO_TILE_ROOT";
/// What separates the pid at the end of a run log's name from the
/// timestamp in front of it.
pub(crate) const PID_SEPARATOR: char = '-';
/// Shown in `state` for a run waiting on another cargo to give up the
/// build directory. A word rather than a bar: there is no reading to
/// draw, which is the whole of what it says.
pub(crate) const STATE_BLOCKED: &str = "blocked";
/// Cells the reading itself takes at the end of a header's rule,
/// `100%` being the widest it goes.
pub(crate) const PROGRESS_READING_WIDTH: usize = 4;
/// Cells a reading carrying a tenth takes instead, `100.0%` being the
/// widest that one goes.
pub(crate) const PROGRESS_READING_TENTHS_WIDTH: usize = 6;
/// Units a plan has to hold before its reading is worth a tenth. Up to
/// a hundred, one unit is a whole percent or more and the number moves
/// every time the count does; past it a run climbs several units
/// between readings, and a tenth is what gives it somewhere to go.
pub(crate) const PROGRESS_TENTHS_MIN_TOTAL: usize = 100;
/// Unfilled glyph of the rule running along a working-directory header.
pub(crate) const PROGRESS_HEADING_EMPTY: char = '\u{254c}';
/// Filled glyph of the rule running along a working-directory header.
pub(crate) const PROGRESS_HEADING_FILLED: char = '\u{2501}';
/// Blank cells the header's rule keeps around itself: one after the
/// working directory and one before the reading at the end.
pub(crate) const PROGRESS_HEADING_MARGINS: u16 = 2;
/// Cells a header's rule needs before it is worth drawing at all. Below
/// this the header shows the directory alone, the way it always has.
pub(crate) const PROGRESS_HEADING_MIN_WIDTH: u16 = 4;
/// The blank cell between the working directory and the word naming
/// the phase, where the header has the room to carry one.
pub(crate) const PROGRESS_HEADING_PHASE_MARGIN: u16 = 1;
/// Bytes of a run log's end to read for the counter. Sized to hold the
/// bar's last redraw across a burst of diagnostics printed over it.
pub(crate) const RUN_LOG_TAIL_BYTES: u64 = 64 * 1024;
/// What a run log's name starts with, ahead of its timestamp and pid.
pub(crate) const RUN_LOG_PREFIX: &str = "run-";
/// What a run log's name ends with.
pub(crate) const RUN_LOG_SUFFIX: &str = ".log";
/// What closes the drawn bar in cargo's progress line, immediately
/// ahead of the counter: `[====>    ] 149/403: serde`.
pub(crate) const UNIT_COUNTER_LEAD: &str = "] ";
/// What divides the two numbers in cargo's counter.
pub(crate) const UNIT_COUNTER_SEPARATOR: &str = "/";
/// What closes cargo's counter, ahead of the crate names it is
/// currently building.
pub(crate) const UNIT_COUNTER_TRAILER: &str = ":";
/// First of the block-element glyphs a drawn bar is filled with, and
/// [`BAR_GLYPH_LAST`] the last. A test runner draws its bar between the
/// bracket closing its elapsed time and the counter beyond it, where
/// cargo puts nothing at all, so the two are told apart by what stands
/// between them.
pub(crate) const BAR_GLYPH_FIRST: char = '\u{2580}';
/// Last of the block-element glyphs described by [`BAR_GLYPH_FIRST`].
pub(crate) const BAR_GLYPH_LAST: char = '\u{259f}';
/// What opens a test runner's per-test tally and [`TALLY_CLOSE`] what
/// closes it: `PASS [   1.014s] (11/24) nxprobe t18`. That is where the
/// count goes when the runner has no bar to put it in, which is every
/// run whose output is not a terminal.
pub(crate) const TALLY_OPEN: &str = "(";
/// What closes the tally opened by [`TALLY_OPEN`].
pub(crate) const TALLY_CLOSE: &str = ")";
/// The status word a test runner draws its counter under. Cargo's own
/// bars say `Building` or `Downloading`; only a run of the tests says
/// this with a counter beside it. Matched on the word alone, the colour
/// codes around it leaving it whole.
pub(crate) const TEST_PHASE_MARKER: &str = "Running";
/// What a working-directory header calls the phase a run is in while
/// cargo compiles the units of its build plan.
pub(crate) const PHASE_BUILDING: &str = "building";
/// What a working-directory header calls the phase a run is in while a
/// test runner works through the tests it collected.
pub(crate) const PHASE_TESTING: &str = "testing";
/// What cargo says while it waits for another cargo to give up the
/// build directory. Matched on the phrase alone: the `Blocking` status
/// word ahead of it arrives wrapped in colour codes, and what it names
/// varies with which lock is held.
pub(crate) const LOCK_WAIT_MARKER: &str = "waiting for file lock";

// sccache
/// The sccache executable: what the stats read runs, and the process
/// name a scan recognises a running server by. `sccache --show-stats`
/// starts a server when none is up, so the scan's answer is what keeps
/// the read from creating the thing it reports on.
pub(crate) const SCCACHE_BINARY: &str = "sccache";
/// Word naming the cache span on the summary cell's border. The words
/// there are the border's own, not sccache's: they are set beside the
/// figures in a colour of their own and have to fit a top line.
pub(crate) const SCCACHE_CACHE_WORD: &str = "cache";
/// `sccache --show-stats` label for compiles served from the cache.
pub(crate) const SCCACHE_HITS_LABEL: &str = "Cache hits";
/// Word naming the hits figure on the summary cell's border.
pub(crate) const SCCACHE_HITS_WORD: &str = "hits";
/// `sccache --show-stats` label for the overall hit rate. The
/// per-language rates are printed under labels of their own, which is
/// why a field is matched on the whole label rather than a prefix.
pub(crate) const SCCACHE_HIT_RATE_LABEL: &str = "Cache hits rate";
/// Word naming the hit rate on the summary cell's border.
pub(crate) const SCCACHE_HIT_RATE_WORD: &str = "hit rate";
/// `sccache --show-stats` label for the cache's ceiling, past which it
/// evicts.
pub(crate) const SCCACHE_MAX_SIZE_LABEL: &str = "Max cache size";
/// `sccache --show-stats` label for compiles that had to run.
pub(crate) const SCCACHE_MISSES_LABEL: &str = "Cache misses";
/// Word naming the misses figure on the summary cell's border.
pub(crate) const SCCACHE_MISSES_WORD: &str = "misses";
/// Gap between two `sccache --show-stats` reads. The figures are
/// cumulative over a server's whole life, so they move slowly enough
/// that reading them at the process poll's cadence would spend a
/// process a quarter second to redraw the same line.
pub(crate) const SCCACHE_POLL_SECONDS: u64 = 10;
/// `sccache --show-stats` label for the disk the cache occupies.
pub(crate) const SCCACHE_SIZE_LABEL: &str = "Cache size";
/// The argument that makes sccache report its statistics.
pub(crate) const SCCACHE_STATS_ARG: &str = "--show-stats";
