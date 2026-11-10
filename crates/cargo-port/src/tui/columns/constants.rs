// `COL_*` constants index the project table columns; `NUM_COLS` fixes its column count.
use std::ops::RangeInclusive;
pub(super) const COL_CI: usize = 2;
pub(in crate::tui) const COL_DISK: usize = 7;
pub(super) const COL_GIT_PATH: usize = 4;
pub(super) const COL_LANG: usize = 3;
pub(super) const COL_LINT: usize = 1;
pub(in crate::tui) const COL_MAIN: usize = 6;
pub(in crate::tui) const COL_NAME: usize = 0;
pub(in crate::tui) const COL_SYNC: usize = 5;
pub(super) const NUM_COLS: usize = 8;

// Keep complete headers and readable names beside the detail panes.
/// Give the name column the cells that decimal storage values used.
pub(super) const COMPACT_DISK_MAX_WIDTH: usize = 7;
/// Keep every header inside the Projects frame at narrow widths.
pub(in crate::tui) const COMPLETE_PROJECT_HEADERS_WIDTH: u16 = 45;
/// Keep the cursor prefix, one name letter and an ellipsis.
pub(super) const CUT_PROJECT_NAME_MIN_WIDTH: usize = 5;
/// Separate the Lint heading from Projects and preserve a name letter.
pub(super) const LINT_PROJECT_HEADER_RESERVE: u16 = 2;
/// Reserve the common Main value before its comparison arrives.
pub(super) const MAIN_COMPARISON_VALUE_RESERVE: usize = 5;
/// Reduce names before columns when the window has little room.
pub(super) const NARROW_PROJECT_WINDOWS: RangeInclusive<u16> = 60..=80;
/// Keep both detail titles whole beside the Projects frame.
pub(super) const WHOLE_DETAIL_TITLES_WIDTH: u16 = 35;
