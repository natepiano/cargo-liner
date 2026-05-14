use ratatui::style::Color;

/// Bytes per kibibyte.
pub const BYTES_PER_KIB: u64 = 1024;
/// Bytes per mebibyte.
pub const BYTES_PER_MIB: u64 = 1024 * 1024;
/// Bytes per gibibyte.
pub const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Block border costs 2 horizontal characters (left + right).
pub const BLOCK_BORDER_WIDTH: usize = 2;

/// Tick interval between render polls.
pub const FRAME_POLL_MILLIS: u64 = 16;

/// Two-space indent for popup section headers.
pub const SECTION_HEADER_INDENT: &str = "  ";
/// Four-space indent for popup section items.
pub const SECTION_ITEM_INDENT: &str = "    ";

// Semantic colors

/// Spinners, shortcut key hints, running/in-progress indicators, finder
/// query input cursor.
pub const ACCENT_COLOR: Color = Color::Cyan;
/// Border color for the currently focused pane (matches `TITLE_COLOR`).
pub const ACTIVE_BORDER_COLOR: Color = Color::Yellow;
/// Background highlight for the currently focused pane row.
pub const ACTIVE_FOCUS_COLOR: Color = Color::Rgb(125, 125, 125);
/// Background highlight for the row currently under the mouse in a
/// scrollable pane.
pub const HOVER_FOCUS_COLOR: Color = Color::Rgb(80, 80, 80);
/// Project list column headers ("Name", "Disk", "Sync", etc.).
pub const COLUMN_HEADER_COLOR: Color = Color::Rgb(150, 190, 180);
/// Shimmer animation on newly discovered projects.
pub const DISCOVERY_SHIMMER_COLOR: Color = Color::Rgb(150, 210, 255);
/// Error text, failure icons, broken worktree backgrounds, error toasts.
pub const ERROR_COLOR: Color = Color::Red;
/// Inline errors shown on selected settings rows where `ERROR_COLOR`
/// clashes with the selection highlight background.
pub const INLINE_ERROR_COLOR: Color = Color::Yellow;
/// Unfocused pane borders and titles for empty/disabled panes.
pub const INACTIVE_BORDER_COLOR: Color = Color::DarkGray;
/// Unfocused pane titles for populated panes.
pub const INACTIVE_TITLE_COLOR: Color = Color::White;
/// Detail panel field labels, stat labels, settings labels, toast
/// countdowns, finder hints, chevron arrows.
pub const LABEL_COLOR: Color = COLUMN_HEADER_COLOR;
/// Background highlight showing the previously focused row when a pane
/// loses focus.
pub const REMEMBERED_FOCUS_COLOR: Color = Color::Rgb(40, 40, 40);
/// Dimmed secondary text: shortcut descriptions, scan progress, ignored
/// paths in shimmer views.
pub const SECONDARY_TEXT_COLOR: Color = Color::Gray;
/// Bottom status bar background.
pub const STATUS_BAR_COLOR: Color = Color::DarkGray;
/// Clean/passed/synced states.
pub const SUCCESS_COLOR: Color = Color::Green;
/// Bench target type accent.
pub const TARGET_BENCH_COLOR: Color = Color::Magenta;
/// Active pane titles, section headers, group header labels, stat
/// numbers, confirm dialog prompts, popup titles, summary row.
pub const TITLE_COLOR: Color = Color::Yellow;
/// Background tint on fuzzy-matched characters in finder results.
pub const FINDER_MATCH_BG: Color = Color::Rgb(0, 90, 100);
