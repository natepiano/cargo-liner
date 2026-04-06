use ratatui::layout::Rect;

use super::types::App;
use crate::tui::types::PaneId;

// ── Dismiss target ──────────────────────────────────────────────

/// Identifies what is being dismissed by a `GlobalAction::Dismiss`.
#[derive(Clone, Debug)]
pub enum DismissTarget {
    Toast(u64),
    DeletedProject(String),
}

/// A clickable dismiss affordance registered during rendering.
/// Stored in `LayoutCache` so the input handler can hit-test mouse clicks.
#[derive(Clone, Debug)]
pub struct ClickAction {
    pub rect:   Rect,
    pub target: DismissTarget,
}

// ── Resolution + dispatch ───────────────────────────────────────

impl App {
    /// Resolve the currently focused pane into a dismiss target, if one exists.
    pub fn focused_dismiss_target(&self) -> Option<DismissTarget> {
        match self.focused_pane {
            PaneId::Toasts => self.focused_toast_id().map(DismissTarget::Toast),
            PaneId::ProjectList => {
                let project = self.selected_project()?;
                if self.is_deleted(&project.path) {
                    Some(DismissTarget::DeletedProject(project.path.clone()))
                } else {
                    None
                }
            },
            _ => None,
        }
    }

    /// Perform the dismiss for the given target.
    pub fn dismiss(&mut self, target: DismissTarget) {
        match target {
            DismissTarget::Toast(id) => self.dismiss_toast(id),
            DismissTarget::DeletedProject(path) => {
                self.dismissed_projects.insert(path.clone());
                self.deleted_projects.remove(&path);
                self.dirty.rows.mark_dirty();
                self.ensure_visible_rows_cached();
                let count = self.row_count();
                if let Some(selected) = self.list_state.selected()
                    && selected >= count
                {
                    self.list_state.select(Some(count.saturating_sub(1)));
                }
            },
        }
    }
}
