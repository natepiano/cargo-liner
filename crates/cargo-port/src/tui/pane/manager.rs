use super::Viewport;
use crate::tui::panes::PaneId;

pub struct PaneManager {
    panes: Vec<Viewport>,
}

impl PaneManager {
    pub fn pane(&self, id: PaneId) -> &Viewport { &self.panes[id.index()] }

    pub fn pane_mut(&mut self, id: PaneId) -> &mut Viewport { &mut self.panes[id.index()] }

    pub fn new() -> Self {
        Self {
            panes: vec![Viewport::new(); PaneId::pane_count()],
        }
    }

    pub fn clear_hover(&mut self) {
        for pane in &mut self.panes {
            pane.set_hovered(None);
        }
    }
}
