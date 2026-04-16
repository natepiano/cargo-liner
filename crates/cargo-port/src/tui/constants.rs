use ratatui::style::Color;

// Byte sizes
pub(super) const BYTES_PER_KIB: u64 = 1024;
pub(super) const BYTES_PER_MIB: u64 = 1024 * 1024;
pub(super) const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Block border costs 2 horizontal characters (left + right).
pub(super) const BLOCK_BORDER_WIDTH: usize = 2;

pub(super) const FRAME_POLL_MILLIS: u64 = 16;

pub(super) const FINDER_POPUP_HEIGHT: u16 = 28;
pub(super) const SETTINGS_POPUP_WIDTH: u16 = 90;
pub(super) const CONFIRM_DIALOG_HEIGHT: u16 = 3;
pub(super) const CI_TIMESTAMP_WIDTH: u16 = 16;
pub(super) const TOAST_WIDTH: u16 = 50;
pub(super) const TOAST_GAP: u16 = 0;
/// Milliseconds between each line reveal/collapse during toast animation.
pub(super) const TOAST_LINE_REVEAL_MS: u64 = 150;

pub(super) const MAX_FINDER_RESULTS: usize = 50;

// Popup section layout
pub(super) const SECTION_HEADER_INDENT: &str = "  ";
pub(super) const SECTION_ITEM_INDENT: &str = "    ";

// Semantic colors

/// Spinners, shortcut key hints, running/in-progress indicators, finder
/// query input cursor.
pub(super) const ACCENT_COLOR: Color = Color::Cyan;
/// Border color for the currently focused pane (matches `TITLE_COLOR`).
pub(super) const ACTIVE_BORDER_COLOR: Color = Color::Yellow;
/// Background highlight for the currently focused pane row.
pub(super) const ACTIVE_FOCUS_COLOR: Color = Color::Rgb(125, 125, 125);
/// Background highlight for the row currently under the mouse in a
/// scrollable pane.
pub(super) const HOVER_FOCUS_COLOR: Color = Color::Rgb(80, 80, 80);
/// Project list column headers ("Name", "Disk", "Sync", etc.).
pub(super) const COLUMN_HEADER_COLOR: Color = Color::Rgb(150, 190, 180);
/// Shimmer animation on newly discovered projects.
pub(super) const DISCOVERY_SHIMMER_COLOR: Color = Color::Rgb(150, 210, 255);
/// Error text, failure icons, broken worktree backgrounds, error toasts.
pub(super) const ERROR_COLOR: Color = Color::Red;
/// Inline errors shown on selected settings rows where `ERROR_COLOR`
/// clashes with the selection highlight background.
pub(super) const INLINE_ERROR_COLOR: Color = Color::Yellow;
/// Unfocused pane borders and titles ("No Lint Runs", "No Targets",
/// "Not a git repo", empty CI panels).
pub(super) const INACTIVE_BORDER_COLOR: Color = Color::DarkGray;
/// Unfocused pane titles for populated panes.
pub(super) const INACTIVE_TITLE_COLOR: Color = Color::White;
/// Detail panel field labels ("Path", "Branch", "Disk"), stat labels,
/// settings labels, toast countdowns, finder hints, chevron arrows.
pub(super) const LABEL_COLOR: Color = COLUMN_HEADER_COLOR;
/// Background highlight showing the previously focused row when a pane
/// loses focus.
pub(super) const REMEMBERED_FOCUS_COLOR: Color = Color::Rgb(40, 40, 40);
/// Dimmed secondary text: shortcut descriptions, scan progress, ignored
/// git paths in the shimmer view.
pub(super) const SECONDARY_TEXT_COLOR: Color = Color::Gray;
/// Bottom status bar background.
pub(super) const STATUS_BAR_COLOR: Color = Color::DarkGray;
/// Clean/passed/synced states: lint passed, CI success, git in-sync,
/// settings toggle "on" state.
pub(super) const SUCCESS_COLOR: Color = Color::Green;
/// Bench target type in the targets panel.
pub(super) const TARGET_BENCH_COLOR: Color = Color::Magenta;
/// Active pane titles, section headers, group header labels, stat
/// numbers, confirm dialog prompts, popup titles, summary row.
pub(super) const TITLE_COLOR: Color = Color::Yellow;
/// Background tint on fuzzy-matched characters in finder results.
pub(super) const FINDER_MATCH_BG: Color = Color::Rgb(0, 90, 100);

// Startup toast phase labels — used as both the tracked-item label and
// key. Completion matches by key via `mark_tracked_item_completed`.
pub(super) const STARTUP_PHASE_DISK: &str = "Disk usage";
pub(super) const STARTUP_PHASE_GIT: &str = "Local git repos";
pub(super) const STARTUP_PHASE_GITHUB: &str = "GitHub repos";
pub(super) const STARTUP_PHASE_LINT: &str = "Lint cache";
