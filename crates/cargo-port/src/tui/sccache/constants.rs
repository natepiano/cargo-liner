// src tui sccache render
pub(super) const CONTENT_WIDTH_PADDING: usize = 2;
pub(super) const POPUP_BORDER_HEIGHT: u16 = 2;
pub(super) const POPUP_HORIZONTAL_MARGIN: u16 = 4;
pub(super) const POPUP_MIN_WIDTH: u16 = 56;
pub(super) const POPUP_VERTICAL_MARGIN: u16 = 4;

// src tui sccache status_line
pub(super) const NOTE_LABEL: &str = " sccache: ";
/// Gap between two `sccache --show-stats` polls for the status-line
/// segment.
pub(super) const REFRESH_INTERVAL_SECS: u64 = 10;
