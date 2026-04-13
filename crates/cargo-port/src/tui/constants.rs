use ratatui::style::Color;

// Byte sizes
pub(super) const BYTES_PER_KIB: u64 = 1024;
pub(super) const BYTES_PER_MIB: u64 = 1024 * 1024;
pub(super) const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Block border costs 2 horizontal characters (left + right).
pub(super) const BLOCK_BORDER_WIDTH: usize = 2;

pub(super) const FRAME_POLL_MILLIS: u64 = 16;
pub(super) const CI_FETCH_DISPLAY_COUNT: u32 = 5;

pub(super) const SEARCH_BAR_HEIGHT: u16 = 3;
pub(super) const DETAIL_PANEL_HEIGHT: u16 = 14;
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

pub(super) const CI_EXTRA_ROWS: usize = 1;

// Semantic colors

/// Spinners, shortcut key hints, running/in-progress indicators, finder
/// query input cursor.
pub(super) const ACCENT_COLOR: Color = Color::Cyan;
/// Background highlight for the currently focused pane row.
pub(super) const ACTIVE_FOCUS_COLOR: Color = Color::Rgb(0, 156, 156);
/// Project list column headers ("Name", "Disk", "Sync", etc.).
pub(super) const COLUMN_HEADER_COLOR: Color = Color::Rgb(150, 190, 180);
/// Shimmer animation on newly discovered projects.
pub(super) const DISCOVERY_SHIMMER_COLOR: Color = Color::Rgb(150, 210, 255);
/// Error text, failure icons, broken worktree backgrounds, error toasts.
pub(super) const ERROR_COLOR: Color = Color::Red;
/// Unfocused pane borders and titles ("No Lint Runs", "No Targets",
/// "Not a git repo", empty CI panels).
pub(super) const INACTIVE_BORDER_COLOR: Color = Color::DarkGray;
/// Detail panel field labels ("Path", "Branch", "Disk"), stat labels,
/// settings labels, toast countdowns, finder hints, chevron arrows.
pub(super) const LABEL_COLOR: Color = COLUMN_HEADER_COLOR;
/// Background highlight showing the previously focused row when a pane
/// loses focus.
pub(super) const REMEMBERED_FOCUS_COLOR: Color = Color::DarkGray;
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
