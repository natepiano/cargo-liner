use std::path::Path;

use super::App;
use super::VisibleRow;
use crate::project::AbsolutePath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::Visibility::Dismissed;
use crate::project::WorktreeGroup;
use crate::tui::panes::PaneId;

// ── Dismiss target ──────────────────────────────────────────────

/// Identifies what is being dismissed by a `GlobalAction::Dismiss`.
#[derive(Clone, Debug)]
pub enum DismissTarget {
    Toast(u64),
    DeletedProject(AbsolutePath),
}

// ── Resolution + dispatch ───────────────────────────────────────

impl App {
    pub(super) fn dismiss_target_for_row_inner(&self, row: VisibleRow) -> Option<DismissTarget> {
        let dismiss_path = match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => self
                .projects()
                .get(node_index)
                .map(|item| item.path().clone()),
            VisibleRow::Member { node_index, .. }
            | VisibleRow::Vendored { node_index, .. }
            | VisibleRow::Submodule { node_index, .. } => self
                .projects()
                .get(node_index)
                .map(|item| item.path().clone()),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            }
            | VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                ..
            }
            | VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                ..
            } => match &self.projects().get(node_index)?.item {
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    if worktree_index == 0 {
                        Some(primary.path().clone())
                    } else {
                        linked.get(worktree_index - 1).map(|ws| ws.path().clone())
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    if worktree_index == 0 {
                        Some(primary.path().clone())
                    } else {
                        linked.get(worktree_index - 1).map(|pkg| pkg.path().clone())
                    }
                },
                _ => None,
            },
        }?;

        if self.projects().is_deleted(&dismiss_path) {
            Some(DismissTarget::DeletedProject(dismiss_path))
        } else {
            None
        }
    }

    /// Resolve the currently focused pane into a dismiss target, if one exists.
    pub fn focused_dismiss_target(&self) -> Option<DismissTarget> {
        match self.focus.current() {
            PaneId::Toasts => self.focused_toast_id().map(DismissTarget::Toast),
            PaneId::ProjectList => self
                .selected_row()
                .and_then(|row| self.dismiss_target_for_row_inner(row)),
            _ => None,
        }
    }

    /// Perform the dismiss for the given target.
    pub fn dismiss(&mut self, target: DismissTarget) {
        match target {
            DismissTarget::Toast(id) => self.dismiss_toast(id),
            DismissTarget::DeletedProject(path) => {
                let parent_node_index = self.worktree_parent_node_index(&path);
                if let Some(project) = self.projects_mut().at_path_mut(&path) {
                    project.visibility = Dismissed;
                }
                self.ensure_visible_rows_cached();
                if let Some(ni) = parent_node_index {
                    self.select_root_row(ni);
                } else {
                    let count = self.row_count();
                    let selected = self.panes().project_list().viewport().pos();
                    if selected >= count {
                        self.panes_mut()
                            .project_list_mut()
                            .viewport_mut()
                            .set_pos(count.saturating_sub(1));
                    }
                }
            },
        }
    }

    /// If `path` is a worktree entry's project path, return the parent
    /// node index so the selection can jump to the Root row after dismiss.
    fn worktree_parent_node_index(&self, path: &Path) -> Option<usize> {
        self.projects()
            .iter()
            .enumerate()
            .find_map(|(ni, item)| match &item.item {
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    let has_match =
                        primary.path() == path || linked.iter().any(|l| l.path() == path);
                    has_match.then_some(ni)
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    let has_match =
                        primary.path() == path || linked.iter().any(|l| l.path() == path);
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
            self.panes_mut()
                .project_list_mut()
                .viewport_mut()
                .set_pos(pos);
        }
    }
}
