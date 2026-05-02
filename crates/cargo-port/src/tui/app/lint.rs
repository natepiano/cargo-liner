use std::path::Path;

use super::App;
use super::types::VisibleRow;
use crate::constants::LINT_NO_LOG;
use crate::lint::LintStatus;

impl App {
    /// Whether the currently selected row is a lint-owning node.
    ///
    /// Only roots and worktree entries own lint state. Members, vendored
    /// packages, and group headers do not — the match is exhaustive so new
    /// variants must be classified.
    pub fn selected_row_owns_lint(&self) -> bool {
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
                | VisibleRow::Submodule { .. }
                | VisibleRow::WorktreeMember { .. }
                | VisibleRow::WorktreeVendored { .. },
            )
            | None => false,
        }
    }

    /// Frame `status` to a static icon string for the current
    /// animation tick, or `LINT_NO_LOG` when lint is disabled.
    /// Phase 11.2 — replaces the per-row `lint_icon*` bodies that
    /// each duplicated the "find a status, then frame an icon"
    /// pattern.
    pub fn frame_lint_icon(&self, status: &LintStatus) -> &'static str {
        if !self.lint_enabled() {
            return LINT_NO_LOG;
        }
        status.icon().frame_at(self.animation_elapsed())
    }

    /// Lint icon for a project at `path`. Pass-through to
    /// [`Self::frame_lint_icon`] over [`Lint::status_for_path`].
    pub fn lint_icon(&self, path: &Path) -> &'static str {
        let status = crate::tui::lint_state::Lint::status_for_path(self.projects(), path);
        self.frame_lint_icon(&status)
    }

    /// Lint icon for the root project at `node_index`, aggregated
    /// across worktree-group entries when applicable.
    pub fn lint_icon_for_root(&self, node_index: usize) -> &'static str {
        let Some(entry) = self.projects().get(node_index) else {
            return LINT_NO_LOG;
        };
        let status = crate::tui::lint_state::Lint::status_for_root(&entry.item);
        self.frame_lint_icon(&status)
    }

    /// Lint icon for a worktree entry within a worktree group;
    /// `worktree_index` 0 is the primary checkout.
    pub fn lint_icon_for_worktree(&self, node_index: usize, worktree_index: usize) -> &'static str {
        let Some(entry) = self.projects().get(node_index) else {
            return LINT_NO_LOG;
        };
        let status = crate::tui::lint_state::Lint::status_for_worktree(&entry.item, worktree_index);
        self.frame_lint_icon(&status)
    }

    /// Lint icon for the currently selected row, used by the
    /// Package detail's worktree-group title.
    ///
    /// Phase 11.3 deletes this — `resolve_lint_display`, the
    /// only consumer, gets replaced by `Lint::package_display`,
    /// which branches on `is_worktree_group` directly. Until
    /// then this delegates to `Lint`.
    pub fn selected_lint_icon(&self, path: &Path) -> Option<&'static str> {
        if !self.lint_enabled() {
            return None;
        }
        match self.selected_row() {
            Some(VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. }) => {
                self.projects()
                    .get(node_index)
                    .map(|entry| crate::tui::lint_state::Lint::status_for_root(&entry.item))
                    .map(|status| status.icon().frame_at(self.animation_elapsed()))
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
            ) => self
                .projects()
                .get(node_index)
                .map(|entry| {
                    crate::tui::lint_state::Lint::status_for_worktree(&entry.item, worktree_index)
                })
                .map(|status| status.icon().frame_at(self.animation_elapsed())),
            Some(
                VisibleRow::Member { .. }
                | VisibleRow::Vendored { .. }
                | VisibleRow::Submodule { .. }
                | VisibleRow::WorktreeMember { .. }
                | VisibleRow::WorktreeVendored { .. },
            )
            | None => self
                .projects()
                .lint_at_path(path)
                .or_else(|| self.projects().vendored_owner_lint(path))
                .map(|lr| lr.status().icon().frame_at(self.animation_elapsed())),
        }
    }
}
