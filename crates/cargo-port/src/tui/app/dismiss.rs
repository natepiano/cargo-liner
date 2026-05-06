use super::App;
use super::VisibleRow;
use crate::project::AbsolutePath;
use crate::project::Visibility::Dismissed;
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
    /// Resolve the currently focused pane into a dismiss target, if one exists.
    pub fn focused_dismiss_target(&self) -> Option<DismissTarget> {
        match self.focus.current() {
            PaneId::Toasts => self.focused_toast_id().map(DismissTarget::Toast),
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
                let parent_node_index = self.project_list.worktree_parent_node_index(&path);
                if let Some(project) = self.project_list.at_path_mut(&path) {
                    project.visibility = Dismissed;
                }
                self.ensure_visible_rows_cached();
                if let Some(ni) = parent_node_index {
                    self.select_root_row(ni);
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

    /// Select the `Root` row for the given node index.
    fn select_root_row(&mut self, node_index: usize) {
        let rows = self.visible_rows();
        if let Some(pos) = rows
            .iter()
            .position(|row| matches!(row, VisibleRow::Root { node_index: ni } if *ni == node_index))
        {
            self.project_list.set_cursor(pos);
        }
    }
}
