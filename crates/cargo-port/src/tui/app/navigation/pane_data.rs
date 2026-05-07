use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::tui;
use crate::tui::app::App;
use crate::tui::app::VisibleRow;
use crate::tui::panes::DetailPaneData;

impl App {
    /// Build per-pane data for the currently selected row, resolving through
    /// the `project_list_items` hierarchy.
    pub(super) fn build_selected_pane_data(&self) -> Option<DetailPaneData> {
        let row = self.project_list.selected_row()?;
        match row {
            VisibleRow::Root { node_index } => {
                let item = self.project_list.get(node_index)?;
                Some(tui::panes::build_pane_data(self, item))
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.project_list.get(node_index)?;
                let pkg = item.resolve_member(group_index, member_index)?;
                Some(tui::panes::build_pane_data_for_member(self, pkg))
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => {
                let item = self.project_list.get(node_index)?;
                let vendored = item.resolve_vendored(vendored_index)?;
                Some(tui::panes::build_pane_data_for_vendored(self, vendored))
            },
            VisibleRow::GroupHeader { node_index, .. } => {
                // Group headers show the parent project's detail
                let item = self.project_list.get(node_index)?;
                Some(tui::panes::build_pane_data(self, item))
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            } => {
                let item = self.project_list.get(node_index)?;
                self.build_worktree_detail(item, worktree_index)
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                let item = self.project_list.get(node_index)?;
                let RootItem::Worktrees(wtg) = &**item else {
                    return None;
                };
                let pkg = wtg.member_ref(worktree_index, group_index, member_index)?;
                Some(tui::panes::build_pane_data_for_member(self, pkg))
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.project_list.get(node_index)?;
                let RootItem::Worktrees(wtg) = &**item else {
                    return None;
                };
                let vendored = wtg.vendored_ref(worktree_index, vendored_index)?;
                Some(tui::panes::build_pane_data_for_vendored(self, vendored))
            },
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => {
                let item = self.project_list.get(node_index)?;
                let submodule = item.submodules().get(submodule_index)?;
                Some(tui::panes::build_pane_data_for_submodule(self, submodule))
            },
        }
    }

    /// Build pane data for a worktree entry (a linked workspace or package).
    fn build_worktree_detail(
        &self,
        item: &RootItem,
        worktree_index: usize,
    ) -> Option<DetailPaneData> {
        match item {
            RootItem::Worktrees(group) => match group.entry(worktree_index)? {
                RustProject::Workspace(ws) => {
                    let display_path = ws.display_path();
                    Some(tui::panes::build_pane_data_for_workspace_ref(
                        self,
                        ws,
                        display_path.as_str(),
                    ))
                },
                RustProject::Package(pkg) => {
                    Some(tui::panes::build_pane_data_for_member(self, pkg))
                },
            },
            _ => None,
        }
    }
}
