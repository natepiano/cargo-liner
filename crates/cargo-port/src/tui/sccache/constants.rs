// The popup reserves the same padding on both sides of its body.
pub(super) const CONTENT_RIGHT_PADDING: &str = tui_pane::SECTION_ITEM_INDENT;
pub(super) const POPUP_BORDER_HEIGHT: u16 = 2;
pub(super) const POPUP_HORIZONTAL_MARGIN: u16 = 4;
pub(super) const POPUP_MIN_WIDTH: u16 = 56;
pub(super) const POPUP_VERTICAL_MARGIN: u16 = 4;

// `NOTE_LABEL` labels the sccache status-line segment; `REFRESH_INTERVAL_SECS` controls its polling
// interval.
pub(super) const NOTE_LABEL: &str = " sccache: ";
/// Gap between two `sccache --show-stats` polls for the status-line
/// segment.
pub(super) const REFRESH_INTERVAL_SECS: u64 = 10;
