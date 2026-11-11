// popup
pub(super) const MIN_POPUP_WIDTH: u16 = 60;

// table
pub(in crate::tui) const FINDER_COLUMN_COUNT: usize = 5;
pub(super) const FINDER_DIR_COLUMN: usize = 3;
pub(super) const FINDER_HEADERS: [&str; FINDER_COLUMN_COUNT] =
    ["Name", "Project", "Branch", "Dir", "Type"];
