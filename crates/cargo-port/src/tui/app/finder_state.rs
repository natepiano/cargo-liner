use crate::tui::finder::FINDER_COLUMN_COUNT;
use crate::tui::finder::FinderItem;

#[derive(Default)]
pub struct FinderState {
    pub query:      String,
    pub results:    Vec<usize>,
    pub total:      usize,
    pub index:      Vec<FinderItem>,
    pub col_widths: [usize; FINDER_COLUMN_COUNT],
}
