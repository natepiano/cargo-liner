use crate::project::Package;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::WorktreeGroup;
use crate::tui;
use crate::tui::app::App;
use crate::tui::app::VisibleRow;
use crate::tui::panes::DetailPaneData;

impl App {
    /// Build per-pane data for the currently selected row, resolving through
    /// the `project_list_items` hierarchy.
    pub(super) fn build_selected_pane_data(&self) -> Option<DetailPaneData> {
        let row = self.selected_row()?;
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
                let pkg = Self::resolve_member(item, group_index, member_index)?;
                Some(tui::panes::build_pane_data_for_member(self, pkg))
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => {
                let item = self.project_list.get(node_index)?;
                let vendored = Self::resolve_vendored(item, vendored_index)?;
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
                let pkg =
                    Self::worktree_member_ref(item, worktree_index, group_index, member_index)?;
                Some(tui::panes::build_pane_data_for_member(self, pkg))
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.project_list.get(node_index)?;
                let vendored = Self::worktree_vendored_ref(item, worktree_index, vendored_index)?;
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

    /// Resolve a member `Package` from a `RootItem`.
    fn resolve_member(
        item: &RootItem,
        group_index: usize,
        member_index: usize,
    ) -> Option<&Package> {
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.groups().get(group_index)?.members().get(member_index)
            },
            RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                if !wtg.renders_as_group() =>
            {
                wtg.single_live_workspace()?
                    .groups()
                    .get(group_index)?
                    .members()
                    .get(member_index)
            },
            _ => None,
        }
    }

    /// Resolve a vendored package from a `RootItem`.
    fn resolve_vendored(item: &RootItem, vendored_index: usize) -> Option<&VendoredPackage> {
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => ws.vendored().get(vendored_index),
            RootItem::Rust(RustProject::Package(pkg)) => pkg.vendored().get(vendored_index),
            RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                if !wtg.renders_as_group() =>
            {
                wtg.single_live_workspace()?.vendored().get(vendored_index)
            },
            RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. })
                if !wtg.renders_as_group() =>
            {
                wtg.single_live_package()?.vendored().get(vendored_index)
            },
            _ => None,
        }
    }

    /// Resolve a member inside a worktree entry.
    fn worktree_member_ref(
        item: &RootItem,
        worktree_index: usize,
        group_index: usize,
        member_index: usize,
    ) -> Option<&Package> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if worktree_index == 0 {
                    primary
                } else {
                    linked.get(worktree_index - 1)?
                };
                ws.groups().get(group_index)?.members().get(member_index)
            },
            _ => None,
        }
    }

    /// Resolve a vendored package inside a worktree entry.
    fn worktree_vendored_ref(
        item: &RootItem,
        worktree_index: usize,
        vendored_index: usize,
    ) -> Option<&VendoredPackage> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if worktree_index == 0 {
                    primary
                } else {
                    linked.get(worktree_index - 1)?
                };
                ws.vendored().get(vendored_index)
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                let pkg = if worktree_index == 0 {
                    primary
                } else {
                    linked.get(worktree_index - 1)?
                };
                pkg.vendored().get(vendored_index)
            },
            _ => None,
        }
    }

    /// Build pane data for a worktree entry (a linked workspace or package).
    fn build_worktree_detail(
        &self,
        item: &RootItem,
        worktree_index: usize,
    ) -> Option<DetailPaneData> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if worktree_index == 0 {
                    primary
                } else {
                    linked.get(worktree_index - 1)?
                };
                let display_path = ws.display_path();
                Some(tui::panes::build_pane_data_for_workspace_ref(
                    self,
                    ws,
                    display_path.as_str(),
                ))
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                let pkg = if worktree_index == 0 {
                    primary
                } else {
                    linked.get(worktree_index - 1)?
                };
                Some(tui::panes::build_pane_data_for_member(self, pkg))
            },
            _ => None,
        }
    }
}
