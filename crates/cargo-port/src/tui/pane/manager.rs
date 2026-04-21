use super::Pane;
use crate::tui::panes::PaneId;

pub(in super::super) struct PaneManager {
    panes: Vec<Pane>,
}

impl PaneManager {
    pub(in super::super) fn pane(&self, id: PaneId) -> &Pane { &self.panes[id.index()] }

    pub(in super::super) fn pane_mut(&mut self, id: PaneId) -> &mut Pane {
        &mut self.panes[id.index()]
    }

    pub(in super::super) fn new() -> Self {
        Self {
            panes: vec![Pane::new(); PaneId::pane_count()],
        }
    }

    pub(in super::super) fn clear_hover(&mut self) {
        for pane in &mut self.panes {
            pane.set_hovered(None);
        }
    }
}
