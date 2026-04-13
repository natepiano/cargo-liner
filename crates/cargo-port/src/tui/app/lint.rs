use std::path::Path;

use super::App;
use super::types::VisibleRow;
use crate::project::RootItem;

impl App {
    /// Whether the currently selected row is a lint-owning node.
    ///
    /// Only roots and worktree entries own lint state. Members, vendored
    /// packages, and group headers do not — the match is exhaustive so new
    /// variants must be classified.
    pub(in super::super) fn selected_row_owns_lint(&self) -> bool {
        match self.selected_row() {
            Some(
                VisibleRow::Root { .. }
                | VisibleRow::WorktreeEntry { .. }
                | VisibleRow::WorktreeGroupHeader { .. },
            ) => true,
            Some(
                VisibleRow::GroupHeader { .. }
                | VisibleRow::Member { .. }
                | VisibleRow::Vendored { .. }
                | VisibleRow::WorktreeMember { .. }
                | VisibleRow::WorktreeVendored { .. },
            )
            | None => false,
        }
    }

    /// Lint icon frame for the current animation state, or a blank space if lint is
    /// disabled or no log exists.
    pub(in super::super) fn lint_icon(&self, path: &Path) -> &'static str {
        use crate::constants::LINT_NO_LOG;

        if !self.lint_enabled() {
            return LINT_NO_LOG;
        }
        let Some(lr) = self.projects.lint_at_path(path) else {
            return LINT_NO_LOG;
        };
        lr.status().icon().frame_at(self.animation_elapsed())
    }

    pub(in super::super) fn lint_icon_for_root(&self, node_index: usize) -> &'static str {
        use crate::constants::LINT_NO_LOG;

        if !self.lint_enabled() {
            return LINT_NO_LOG;
        }
        let Some(item) = self.projects.get(node_index) else {
            return LINT_NO_LOG;
        };
        let status = item.lint_rollup_status();
        status.icon().frame_at(self.animation_elapsed())
    }

    pub(in super::super) fn lint_icon_for_worktree(
        &self,
        node_index: usize,
        worktree_index: usize,
    ) -> &'static str {
        use crate::constants::LINT_NO_LOG;

        if !self.lint_enabled() {
            return LINT_NO_LOG;
        }
        let Some(RootItem::Worktrees(g)) = self.projects.get(node_index) else {
            return LINT_NO_LOG;
        };
        let status = g.lint_status_for_worktree(worktree_index);
        status.icon().frame_at(self.animation_elapsed())
    }

    pub(in super::super) fn selected_lint_icon(&self, path: &Path) -> Option<&'static str> {
        if !self.lint_enabled() {
            return None;
        }
        match self.selected_row() {
            Some(VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. }) => {
                self.projects.get(node_index).map(|item| {
                    item.lint_rollup_status()
                        .icon()
                        .frame_at(self.animation_elapsed())
                })
            },
            Some(
                VisibleRow::WorktreeEntry {
                    node_index,
                    worktree_index,
                }
                | VisibleRow::WorktreeGroupHeader {
                    node_index,
                    worktree_index,
                    ..
                },
            ) => {
                let RootItem::Worktrees(g) = self.projects.get(node_index)? else {
                    return None;
                };
                let status = g.lint_status_for_worktree(worktree_index);
                Some(status.icon().frame_at(self.animation_elapsed()))
            },
            Some(
                VisibleRow::Member { .. }
                | VisibleRow::Vendored { .. }
                | VisibleRow::WorktreeMember { .. }
                | VisibleRow::WorktreeVendored { .. },
            )
            | None => self
                .projects
                .lint_at_path(path)
                .map(|lr| lr.status().icon().frame_at(self.animation_elapsed())),
        }
    }
}
