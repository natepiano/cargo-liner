// tui_pane src toasts format
pub(super) const ELLIPSIS: &str = "…";
pub(super) const ELLIPSIS_WIDTH: usize = 1;
/// All items included in the body — the toast renderer truncates based on
/// allocated space and shows (+N more) as needed.
pub(super) const MAX_VISIBLE_ITEMS: usize = usize::MAX;
