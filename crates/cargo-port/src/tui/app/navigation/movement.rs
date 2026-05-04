use crate::tui::app::App;
use crate::tui::app::VisibleRow;

impl App {
    pub fn move_up(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.panes().project_list().viewport().pos();
        if current > 0 {
            self.panes_mut()
                .project_list_mut()
                .viewport_mut()
                .set_pos(current - 1);
        }
    }

    pub fn move_down(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.panes().project_list().viewport().pos();
        if current < count - 1 {
            self.panes_mut()
                .project_list_mut()
                .viewport_mut()
                .set_pos(current + 1);
        }
    }

    pub fn move_to_top(&mut self) {
        if self.row_count() > 0 {
            self.panes_mut()
                .project_list_mut()
                .viewport_mut()
                .set_pos(0);
        }
    }

    pub fn move_to_bottom(&mut self) {
        let count = self.row_count();
        if count > 0 {
            self.panes_mut()
                .project_list_mut()
                .viewport_mut()
                .set_pos(count - 1);
        }
    }

    pub(super) const fn collapse_anchor_row(row: VisibleRow) -> VisibleRow {
        match row {
            VisibleRow::GroupHeader { node_index, .. }
            | VisibleRow::Member { node_index, .. }
            | VisibleRow::Vendored { node_index, .. }
            | VisibleRow::Submodule { node_index, .. } => VisibleRow::Root { node_index },
            VisibleRow::Root { .. } | VisibleRow::WorktreeEntry { .. } => row,
            VisibleRow::WorktreeGroupHeader {
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
            } => VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            },
        }
    }
}
