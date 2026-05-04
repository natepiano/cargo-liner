use std::path::Path;

use crate::project;
use crate::project::AbsolutePath;
use crate::project::DisplayPath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorktreeGroup;
use crate::tui::app::App;
use crate::tui::app::VisibleRow;
use crate::tui::app::target_index::CleanSelection;

impl App {
    pub fn selected_row(&self) -> Option<VisibleRow> {
        let rows = self.visible_rows();
        let selected = self.panes().project_list().viewport().pos();
        rows.get(selected).copied()
    }

    /// Returns the `RootItem` when a root row is selected.
    pub fn selected_item(&self) -> Option<&RootItem> {
        match self.selected_row()? {
            VisibleRow::Root { node_index } => {
                self.projects().get(node_index).map(|entry| &entry.item)
            },
            _ => None,
        }
    }

    /// Map the currently selected row to a [`CleanSelection`] when the
    /// Clean shortcut should be enabled on it.
    pub fn clean_selection(&self) -> Option<CleanSelection> {
        let row = self.selected_row()?;
        match row {
            VisibleRow::Root { node_index } => {
                let entry = self.projects().get(node_index)?;
                match &entry.item {
                    RootItem::Rust(rust) => Some(CleanSelection::Project {
                        root: rust.path().clone(),
                    }),
                    RootItem::Worktrees(group) => Some(worktree_group_selection(group)),
                    RootItem::NonRust(_) => None,
                }
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => {
                let entry = self.projects().get(node_index)?;
                Self::worktree_path_ref(&entry.item, worktree_index).map(|path| {
                    CleanSelection::Project {
                        root: AbsolutePath::from(path),
                    }
                })
            },
            _ => None,
        }
    }

    /// Returns the absolute path of the currently selected project, borrowed
    /// from the visible tree rows.
    pub fn selected_project_path(&self) -> Option<&Path> {
        let row = self.selected_row()?;
        self.path_for_row(row)
    }

    /// Given a `VisibleRow`, resolve the absolute `&Path` borrowed from
    /// `project_list_items`.
    pub(super) fn path_for_row(&self, row: VisibleRow) -> Option<&Path> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                Some(self.projects().get(node_index)?.path().as_path())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => Self::member_path_ref(
                &self.projects().get(node_index)?.item,
                group_index,
                member_index,
            ),
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => Self::vendored_path_ref(&self.projects().get(node_index)?.item, vendored_index),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            } => Self::worktree_path_ref(&self.projects().get(node_index)?.item, worktree_index),
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => Self::worktree_member_path_ref(
                &self.projects().get(node_index)?.item,
                worktree_index,
                group_index,
                member_index,
            ),
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => Self::worktree_vendored_path_ref(
                &self.projects().get(node_index)?.item,
                worktree_index,
                vendored_index,
            ),
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => self
                .projects()
                .get(node_index)?
                .submodules()
                .get(submodule_index)
                .map(|s| s.path.as_path()),
        }
    }

    fn member_path_ref(item: &RootItem, group_index: usize, member_index: usize) -> Option<&Path> {
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                let group = ws.groups().get(group_index)?;
                let member = group.members().get(member_index)?;
                Some(member.path().as_path())
            },
            RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                if !wtg.renders_as_group() =>
            {
                let group = wtg.single_live_workspace()?.groups().get(group_index)?;
                let member = group.members().get(member_index)?;
                Some(member.path().as_path())
            },
            _ => None,
        }
    }

    fn vendored_path_ref(item: &RootItem, vendored_index: usize) -> Option<&Path> {
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => ws
                .vendored()
                .get(vendored_index)
                .map(|p| p.path().as_path()),
            RootItem::Rust(RustProject::Package(pkg)) => pkg
                .vendored()
                .get(vendored_index)
                .map(|p| p.path().as_path()),
            RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                if !wtg.renders_as_group() =>
            {
                wtg.single_live_workspace()?
                    .vendored()
                    .get(vendored_index)
                    .map(|p| p.path().as_path())
            },
            RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. })
                if !wtg.renders_as_group() =>
            {
                wtg.single_live_package()?
                    .vendored()
                    .get(vendored_index)
                    .map(|p| p.path().as_path())
            },
            _ => None,
        }
    }

    /// Resolve the display path of the currently selected row using `project_list_items`.
    pub fn selected_display_path(&self) -> Option<DisplayPath> {
        let rows = self.visible_rows();
        let selected = self.panes().project_list().viewport().pos();
        let row = rows.get(selected)?;
        self.display_path_for_row(*row)
    }

    /// Given a `VisibleRow`, resolve the display path from `project_list_items`.
    pub fn display_path_for_row(&self, row: VisibleRow) -> Option<DisplayPath> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                let item = self.projects().get(node_index)?;
                Some(item.display_path())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.projects().get(node_index)?;
                match &item.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => {
                        let group = ws.groups().get(group_index)?;
                        let member = group.members().get(member_index)?;
                        Some(member.display_path())
                    },
                    RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                        if !wtg.renders_as_group() =>
                    {
                        let group = wtg.single_live_workspace()?.groups().get(group_index)?;
                        let member = group.members().get(member_index)?;
                        Some(member.display_path())
                    },
                    _ => None,
                }
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => {
                let item = self.projects().get(node_index)?;
                match &item.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => ws
                        .vendored()
                        .get(vendored_index)
                        .map(ProjectFields::display_path),
                    RootItem::Rust(RustProject::Package(pkg)) => pkg
                        .vendored()
                        .get(vendored_index)
                        .map(ProjectFields::display_path),
                    RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                        if !wtg.renders_as_group() =>
                    {
                        wtg.single_live_workspace()?
                            .vendored()
                            .get(vendored_index)
                            .map(ProjectFields::display_path)
                    },
                    RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. })
                        if !wtg.renders_as_group() =>
                    {
                        wtg.single_live_package()?
                            .vendored()
                            .get(vendored_index)
                            .map(ProjectFields::display_path)
                    },
                    _ => None,
                }
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
                let item = self.projects().get(node_index)?;
                Self::worktree_display_path(item, worktree_index)
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                let item = self.projects().get(node_index)?;
                Self::worktree_member_display_path(item, worktree_index, group_index, member_index)
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.projects().get(node_index)?;
                Self::worktree_vendored_display_path(item, worktree_index, vendored_index)
            },
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => {
                let item = self.projects().get(node_index)?;
                let submodule = item.submodules().get(submodule_index)?;
                Some(DisplayPath::new(project::home_relative_path(
                    &submodule.path,
                )))
            },
        }
    }

    /// Given a `VisibleRow`, resolve the absolute path from `project_list_items`.
    pub fn abs_path_for_row(&self, row: VisibleRow) -> Option<AbsolutePath> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                let item = self.projects().get(node_index)?;
                Some(item.path().clone())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.projects().get(node_index)?;
                match &item.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => {
                        let group = ws.groups().get(group_index)?;
                        let member = group.members().get(member_index)?;
                        Some(member.path().clone())
                    },
                    RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                        if !wtg.renders_as_group() =>
                    {
                        let group = wtg.single_live_workspace()?.groups().get(group_index)?;
                        let member = group.members().get(member_index)?;
                        Some(member.path().clone())
                    },
                    _ => None,
                }
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => {
                let item = self.projects().get(node_index)?;
                match &item.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => {
                        ws.vendored().get(vendored_index).map(|p| p.path().clone())
                    },
                    RootItem::Rust(RustProject::Package(pkg)) => {
                        pkg.vendored().get(vendored_index).map(|p| p.path().clone())
                    },
                    RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                        if !wtg.renders_as_group() =>
                    {
                        wtg.single_live_workspace()?
                            .vendored()
                            .get(vendored_index)
                            .map(|p| p.path().clone())
                    },
                    RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. })
                        if !wtg.renders_as_group() =>
                    {
                        wtg.single_live_package()?
                            .vendored()
                            .get(vendored_index)
                            .map(|p| p.path().clone())
                    },
                    _ => None,
                }
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
                let item = self.projects().get(node_index)?;
                Self::worktree_abs_path(item, worktree_index)
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                let item = self.projects().get(node_index)?;
                Self::worktree_member_abs_path(item, worktree_index, group_index, member_index)
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.projects().get(node_index)?;
                Self::worktree_vendored_abs_path(item, worktree_index, vendored_index)
            },
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => {
                let item = self.projects().get(node_index)?;
                item.submodules()
                    .get(submodule_index)
                    .map(|s| s.path.clone())
            },
        }
    }
}

/// Build a `CleanSelection::WorktreeGroup` from a live
/// [`WorktreeGroup`]. Enum-agnostic (works for both Workspaces and
/// Packages variants) so the caller doesn't have to match twice.
fn worktree_group_selection(group: &WorktreeGroup) -> CleanSelection {
    match group {
        WorktreeGroup::Workspaces { primary, linked } => CleanSelection::WorktreeGroup {
            primary: primary.path().clone(),
            linked:  linked.iter().map(|ws| ws.path().clone()).collect(),
        },
        WorktreeGroup::Packages { primary, linked } => CleanSelection::WorktreeGroup {
            primary: primary.path().clone(),
            linked:  linked.iter().map(|pkg| pkg.path().clone()).collect(),
        },
    }
}
