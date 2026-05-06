use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorktreeGroup;
use crate::tui::app::App;

impl App {
    /// Check if a group at the given indices is an inline (unnamed) group.
    pub(super) fn is_inline_group(&self, ni: usize, gi: usize) -> bool {
        let Some(item) = self.project_list.get(ni) else {
            return true;
        };
        match &item.item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.groups().get(gi).is_some_and(|g| !g.is_named())
            },
            _ => true,
        }
    }

    /// Check if a worktree group at the given indices is an inline (unnamed) group.
    pub(super) fn is_worktree_inline_group(&self, ni: usize, wi: usize, gi: usize) -> bool {
        let Some(item) = self.project_list.get(ni) else {
            return true;
        };
        match &item.item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    match linked.get(wi - 1) {
                        Some(ws) => ws,
                        None => return true,
                    }
                };
                ws.groups().get(gi).is_some_and(|g| !g.is_named())
            },
            _ => true,
        }
    }
}
