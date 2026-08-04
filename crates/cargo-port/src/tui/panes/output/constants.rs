// monitor columns
/// Rows a column header occupies: command, checkout, profile/pid/elapsed, state.
pub(super) const COLUMN_HEADER_HEIGHT: usize = 4;

/// The one-cell rule drawn between adjacent columns. A single column takes the
/// full width and draws none.
pub(super) const COLUMN_DIVIDER_WIDTH: u16 = 1;

/// Narrowest column that still fits a header field and an activity row; below
/// this the monitor windows a subset of columns instead of squeezing them all.
pub(super) const MINIMUM_READABLE_COLUMN_WIDTH: u16 = 28;

/// The `Build monitor on` indicator row above the columns.
pub(super) const MONITOR_INDICATOR_HEIGHT: u16 = 1;

/// Rows the column strip keeps for itself whatever else wants the monitor area,
/// so a short pane never draws a section where the columns should be.
pub(super) const COLUMN_STRIP_MINIMUM_HEIGHT: u16 = 1;

// owned body
/// The `── Output ──` separator between the owned run's activity rows and its
/// captured output.
pub(super) const OWNED_OUTPUT_SEPARATOR_HEIGHT: usize = 1;

/// The pinned owned column's caption row, naming the run and how it ended.
pub(super) const OWNED_PIN_CAPTION_HEIGHT: usize = 1;

// pane chrome
/// Columns the pane border takes off the left and right of the pane area.
pub(super) const PANE_BORDER_COLUMNS: u16 = 2;

/// Rows the pane border takes off the top and bottom of the pane area.
pub(super) const PANE_CHROME_ROWS: u16 = 2;

// unattributed section
/// Largest share of the monitor area the scope-level unattributed section may
/// take while columns are on screen beside it.
pub(super) const UNATTRIBUTED_SECTION_AREA_DIVISOR: u16 = 3;

/// The unattributed section's own caption row.
pub(super) const UNATTRIBUTED_SECTION_CAPTION_HEIGHT: u16 = 1;

/// Smallest useful unattributed section — its caption plus one activity row —
/// so a short pane narrows the section instead of dropping it silently.
pub(super) const UNATTRIBUTED_SECTION_MINIMUM_HEIGHT: u16 = 2;
