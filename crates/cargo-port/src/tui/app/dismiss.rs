use std::path::Path;
use std::path::PathBuf;

use ratatui::layout::Rect;

use super::types::App;
use super::types::VisibleRow;
use crate::project::Visibility::Dismissed;
use crate::tui::types::PaneId;

// ── Dismiss target ──────────────────────────────────────────────

/// Identifies what is being dismissed by a `GlobalAction::Dismiss`.
#[derive(Clone, Debug)]
pub enum DismissTarget {
    Toast(u64),
    DeletedProject(PathBuf),
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
                let selected_path = self.selected_project_path()?;
                if self.is_deleted(selected_path) {
                    Some(DismissTarget::DeletedProject(selected_path.to_path_buf()))
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
                if let Some(project) = self.projects.at_path_mut(&path) {
                    project.visibility = Dismissed;
                }
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
    fn worktree_parent_node_index(&self, path: &Path) -> Option<usize> {
        self.projects
            .iter()
            .enumerate()
            .find_map(|(ni, item)| match item {
                crate::project::RootItem::WorkspaceWorktrees(wtg) => {
                    let has_match = wtg.primary().path() == path
                        || wtg.linked().iter().any(|l| l.path() == path);
                    has_match.then_some(ni)
                },
                crate::project::RootItem::PackageWorktrees(wtg) => {
                    let has_match = wtg.primary().path() == path
                        || wtg.linked().iter().any(|l| l.path() == path);
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
