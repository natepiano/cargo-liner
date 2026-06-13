use crate::tui::panes::PaneId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoveredPaneRow {
    pub pane: PaneId,
    pub row:  usize,
}
