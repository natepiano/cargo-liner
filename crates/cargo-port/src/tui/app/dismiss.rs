use super::App;
use crate::lint;
use crate::project::Visibility::Dismissed;
use crate::tui::pane::DismissTarget;
use crate::tui::panes::PaneId;

// ── Resolution + dispatch ───────────────────────────────────────

impl App {
    /// Resolve the currently focused pane into a dismiss target, if one exists.
    pub fn focused_dismiss_target(&self) -> Option<DismissTarget> {
        match self.focused_pane_id() {
            PaneId::Toasts => self
                .framework
                .toasts
                .focused_toast_id()
                .map(DismissTarget::Toast),
            PaneId::ProjectList => self
                .project_list
                .selected_row()
                .and_then(|row| self.project_list.dismiss_target_for_row_inner(row)),
            _ => None,
        }
    }

    /// Perform the dismiss for the given target.
    pub fn dismiss(&mut self, target: DismissTarget) {
        match target {
            DismissTarget::Toast(id) => self.dismiss_toast(id),
            DismissTarget::DeletedProject(path) => {
                // The project at `path` is gone from disk and the
                // user has confirmed dismissal. Reclaim its lint
                // cache so a future worktree/branch reusing this
                // exact path starts clean. CI cache is keyed by
                // (owner, repo) and shared across sibling worktrees
                // — left alone here.
                lint::reclaim_project_cache(path.as_path());
                let parent_node_index = self.project_list.worktree_parent_node_index(&path);
                if let Some(project) = self.project_list.at_path_mut(&path) {
                    project.visibility = Dismissed;
                }
                self.ensure_visible_rows_cached();
                if let Some(ni) = parent_node_index {
                    self.project_list.select_root_row(ni);
                } else {
                    let count = self.project_list.row_count();
                    let selected = self.project_list.cursor();
                    if selected >= count {
                        self.project_list.set_cursor(count.saturating_sub(1));
                    }
                }
            },
        }
    }
}
