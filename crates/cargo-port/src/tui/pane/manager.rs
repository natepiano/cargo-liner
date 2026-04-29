use super::Viewport;
use crate::tui::panes::PaneId;

/// Vestigial `PaneId`-keyed Viewport array. After Phase 9.7a every
/// pane owns its own `Viewport`; the only remaining caller is
/// `Panes::apply_hovered_pane_row` which clears hover via
/// `clear_hover`. Phase 9.8 deletes the type entirely.
pub struct PaneManager {
    panes: Vec<Viewport>,
}

impl PaneManager {
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
