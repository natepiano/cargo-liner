// popup
/// The popup keeps the same padding on both sides of its body.
pub(super) const CONTENT_RIGHT_PADDING: &str = tui_pane::SECTION_ITEM_INDENT;
/// Cells between the hint text and each side border.
pub(super) const HINT_ROW_MARGIN: u16 = 1;
pub(super) const POPUP_BORDER_HEIGHT: u16 = 2;
pub(super) const POPUP_HORIZONTAL_MARGIN: u16 = 4;
pub(super) const POPUP_MIN_WIDTH: u16 = 56;
pub(super) const POPUP_VERTICAL_MARGIN: u16 = 4;

// status line
pub(super) const NOTE_LABEL: &str = " Disk cache: ";
/// Seconds between two `sccache --show-stats` polls for the status line.
pub(super) const REFRESH_INTERVAL_SECS: u64 = 10;
