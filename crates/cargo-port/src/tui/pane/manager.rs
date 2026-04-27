use super::Pane;
use crate::tui::panes::PaneId;

pub struct PaneManager {
    panes: Vec<Pane>,
}

impl PaneManager {
    pub fn pane(&self, id: PaneId) -> &Pane { &self.panes[id.index()] }

    pub fn pane_mut(&mut self, id: PaneId) -> &mut Pane { &mut self.panes[id.index()] }

    pub fn new() -> Self {
        Self {
            panes: vec![Pane::new(); PaneId::pane_count()],
        }
    }

    pub fn clear_hover(&mut self) {
        for pane in &mut self.panes {
            pane.set_hovered(None);
        }
    }
}
