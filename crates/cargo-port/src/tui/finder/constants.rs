// src tui finder index
/// Column width metrics cached at index build time so the popup renders at a
/// stable size regardless of the current query results.
pub const FINDER_COLUMN_COUNT: usize = 5;
pub const FINDER_HEADERS: [&str; FINDER_COLUMN_COUNT] =
    ["Name", "Project", "Branch", "Dir", "Type"];
pub(super) const MIN_POPUP_WIDTH: u16 = 60;
