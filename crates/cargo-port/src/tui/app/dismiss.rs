use ratatui::layout::Rect;

use super::types::App;
use super::types::VisibleRow;
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
                let parent_node_index = self.worktree_parent_node_index(&path);
                self.dismissed_projects.insert(path.clone());
                self.deleted_projects.remove(&path);
                self.dirty.rows.mark_dirty();
                self.ensure_visible_rows_cached();
                if let Some(ni) = parent_node_index {
                    self.select_root_row(ni);
                } else {
                    let count = self.row_count();
                    if let Some(selected) = self.list_state.selected()
                        && selected >= count
                    {
                        self.list_state.select(Some(count.saturating_sub(1)));
                    }
                }
            },
        }
    }

    /// If `path` is a worktree entry's project path, return the parent
    /// node index so the selection can jump to the Root row after dismiss.
    fn worktree_parent_node_index(&self, path: &str) -> Option<usize> {
        self.project_list_items
            .iter()
            .enumerate()
            .find_map(|(ni, item)| match item {
                crate::project::ProjectListItem::WorkspaceWorktrees(wtg) => {
                    let has_match = wtg.primary().display_path() == path
                        || wtg.linked().iter().any(|l| l.display_path() == path);
                    has_match.then_some(ni)
                },
                crate::project::ProjectListItem::PackageWorktrees(wtg) => {
                    let has_match = wtg.primary().display_path() == path
                        || wtg.linked().iter().any(|l| l.display_path() == path);
                    has_match.then_some(ni)
                },
                _ => None,
            })
    }

    /// Select the `Root` row for the given node index.
    fn select_root_row(&mut self, node_index: usize) {
        let rows = self.visible_rows();
        if let Some(pos) = rows
            .iter()
            .position(|row| matches!(row, VisibleRow::Root { node_index: ni } if *ni == node_index))
        {
            self.list_state.select(Some(pos));
        }
    }
}
