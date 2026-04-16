use std::path::Path;

use super::App;
use super::snapshots;
use super::types::ExpandKey;
use super::types::VisibleRow;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::DisplayPath;
use crate::project::PackageProject;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorktreeGroup;
use crate::tui;
use crate::tui::columns::COL_NAME;
use crate::tui::columns::ResolvedWidths;
use crate::tui::types::PaneId;

impl App {
    pub(in super::super) fn ensure_visible_rows_cached(&mut self) {
        if !self.dirty.rows.is_dirty() {
            return;
        }
        self.dirty.rows.mark_clean();
        self.cached_visible_rows = snapshots::build_visible_rows(
            &self.projects,
            &self.expanded,
            self.include_non_rust().includes_non_rust(),
        );
    }

    /// Return the cached visible rows. Must call `ensure_visible_rows_cached()` first.
    pub(in super::super) fn visible_rows(&self) -> &[VisibleRow] { &self.cached_visible_rows }

    /// Keep fit-to-content widths rebuilding in the background, never inline on the UI thread.
    pub(in super::super) fn ensure_fit_widths_cached(&mut self) { self.request_fit_widths_build(); }

    pub(in super::super) fn observe_name_width(widths: &mut ResolvedWidths, content_width: usize) {
        use COL_NAME;

        widths.observe(COL_NAME, Self::name_width_with_gutter(content_width));
    }

    pub(in super::super) const fn name_width_with_gutter(content_width: usize) -> usize {
        content_width.saturating_add(1)
    }

    /// Keep disk sort caches rebuilding in the background, never inline on the UI thread.
    pub(in super::super) fn ensure_disk_cache(&mut self) { self.request_disk_cache_build(); }

    /// Ensure per-pane data on `PaneManager` is up to date for the selected
    /// project. The cache key tracks generation + selection to avoid
    /// redundant rebuilds.
    pub(in super::super) fn ensure_detail_cached(&mut self) {
        let current_selection = self.current_detail_selection_key();

        if let Some(ref key) = self.detail_cache_key
            && key.generation == self.detail_generation
            && key.selection == current_selection
        {
            return;
        }

        if let Some(data) = self.build_selected_pane_data() {
            self.pane_manager.set_detail_data(
                data.package,
                data.git,
                data.targets,
                tui::detail::build_ci_data(self),
                tui::detail::build_lints_data(self),
            );
            self.detail_cache_key = Some(super::types::DetailCacheKey {
                generation: self.detail_generation,
                selection:  current_selection,
            });
        } else {
            self.pane_manager.clear_detail_data();
            self.detail_cache_key = None;
        }
    }

    /// Build per-pane data for the currently selected row, resolving through
    /// the `project_list_items` hierarchy.
    fn build_selected_pane_data(&self) -> Option<tui::detail::DetailPaneData> {
        let row = self.selected_row()?;
        match row {
            VisibleRow::Root { node_index } => {
                let item = self.projects.get(node_index)?;
                Some(tui::detail::build_pane_data(self, item))
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
                let pkg = Self::resolve_member(item, group_index, member_index)?;
                Some(tui::detail::build_pane_data_for_member(self, pkg))
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => {
                let item = self.projects.get(node_index)?;
                let pkg = Self::resolve_vendored(item, vendored_index)?;
                Some(tui::detail::build_pane_data_for_member(self, pkg))
            },
            VisibleRow::GroupHeader { node_index, .. } => {
                // Group headers show the parent project's detail
                let item = self.projects.get(node_index)?;
                Some(tui::detail::build_pane_data(self, item))
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
                let item = self.projects.get(node_index)?;
                self.build_worktree_detail(item, worktree_index)
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
                let pkg =
                    Self::worktree_member_ref(item, worktree_index, group_index, member_index)?;
                Some(tui::detail::build_pane_data_for_member(self, pkg))
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.projects.get(node_index)?;
                let pkg = Self::worktree_vendored_ref(item, worktree_index, vendored_index)?;
                Some(tui::detail::build_pane_data_for_member(self, pkg))
            },
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => {
                let item = self.projects.get(node_index)?;
                let submodule = item.submodules().get(submodule_index)?;
                Some(tui::detail::build_pane_data_for_submodule(self, submodule))
            },
        }
    }

    /// Resolve a member `PackageProject` from a `RootItem`.
    fn resolve_member(
        item: &RootItem,
        group_index: usize,
        member_index: usize,
    ) -> Option<&PackageProject> {
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

    /// Resolve a vendored `PackageProject` from a `RootItem`.
    fn resolve_vendored(item: &RootItem, vendored_index: usize) -> Option<&PackageProject> {
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
    ) -> Option<&PackageProject> {
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
    ) -> Option<&PackageProject> {
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
    ) -> Option<tui::detail::DetailPaneData> {
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
                Some(tui::detail::build_pane_data_for_workspace_ref(
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
                Some(tui::detail::build_pane_data_for_member(self, pkg))
            },
            _ => None,
        }
    }

    pub(in super::super) fn selected_row(&self) -> Option<VisibleRow> {
        let rows = self.visible_rows();
        let selected = self.pane_manager.pane(PaneId::ProjectList).pos();
        rows.get(selected).copied()
    }

    pub(in super::super) fn current_detail_selection_key(&self) -> String {
        match self.selected_row() {
            Some(VisibleRow::Root { node_index }) => format!("root:{node_index}"),
            Some(VisibleRow::GroupHeader {
                node_index,
                group_index,
            }) => format!("group:{node_index}:{group_index}"),
            Some(VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            }) => format!("member:{node_index}:{group_index}:{member_index}"),
            Some(VisibleRow::Vendored {
                node_index,
                vendored_index,
            }) => format!("vendored:{node_index}:{vendored_index}"),
            Some(VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }) => format!("worktree:{node_index}:{worktree_index}"),
            Some(VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                group_index,
            }) => format!("worktree-group:{node_index}:{worktree_index}:{group_index}"),
            Some(VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            }) => format!(
                "worktree-member:{node_index}:{worktree_index}:{group_index}:{member_index}"
            ),
            Some(VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            }) => format!("worktree-vendored:{node_index}:{worktree_index}:{vendored_index}"),
            Some(VisibleRow::Submodule {
                node_index,
                submodule_index,
            }) => format!("submodule:{node_index}:{submodule_index}"),
            None => String::new(),
        }
    }

    /// Returns the `RootItem` when a root row is selected.
    pub(in super::super) fn selected_item(&self) -> Option<&RootItem> {
        match self.selected_row()? {
            VisibleRow::Root { node_index } => self.projects.get(node_index),
            _ => None,
        }
    }

    /// Returns the absolute path of the currently selected project, borrowed
    /// from the visible tree rows.
    pub(in super::super) fn selected_project_path(&self) -> Option<&Path> {
        let row = self.selected_row()?;
        self.path_for_row(row)
    }

    /// Given a `VisibleRow`, resolve the absolute `&Path` borrowed from
    /// `project_list_items`.
    pub(in super::super) fn path_for_row(&self, row: VisibleRow) -> Option<&Path> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                let item = self.projects.get(node_index)?;
                Some(item.path().as_path())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
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
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => {
                let item = self.projects.get(node_index)?;
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
                let item = self.projects.get(node_index)?;
                Self::worktree_path_ref(item, worktree_index)
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
                Self::worktree_member_path_ref(item, worktree_index, group_index, member_index)
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.projects.get(node_index)?;
                Self::worktree_vendored_path_ref(item, worktree_index, vendored_index)
            },
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => {
                let item = self.projects.get(node_index)?;
                item.submodules()
                    .get(submodule_index)
                    .map(|s| s.path.as_path())
            },
        }
    }

    /// Resolve the display path of the currently selected row using `project_list_items`.
    pub(in super::super) fn selected_display_path(&self) -> Option<DisplayPath> {
        let rows = self.visible_rows();
        let selected = self.pane_manager.pane(PaneId::ProjectList).pos();
        let row = rows.get(selected)?;
        self.display_path_for_row(*row)
    }

    /// Given a `VisibleRow`, resolve the display path from `project_list_items`.
    pub(in super::super) fn display_path_for_row(&self, row: VisibleRow) -> Option<DisplayPath> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                let item = self.projects.get(node_index)?;
                Some(item.display_path())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
                match item {
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
                let item = self.projects.get(node_index)?;
                match item {
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
                let item = self.projects.get(node_index)?;
                Self::worktree_display_path(item, worktree_index)
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
                Self::worktree_member_display_path(item, worktree_index, group_index, member_index)
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.projects.get(node_index)?;
                Self::worktree_vendored_display_path(item, worktree_index, vendored_index)
            },
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => {
                let item = self.projects.get(node_index)?;
                let submodule = item.submodules().get(submodule_index)?;
                Some(DisplayPath::new(project::home_relative_path(
                    &submodule.path,
                )))
            },
        }
    }

    /// Given a `VisibleRow`, resolve the absolute path from `project_list_items`.
    pub(in super::super) fn abs_path_for_row(&self, row: VisibleRow) -> Option<AbsolutePath> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                let item = self.projects.get(node_index)?;
                Some(item.path().clone())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
                match item {
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
                let item = self.projects.get(node_index)?;
                match item {
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
                let item = self.projects.get(node_index)?;
                Self::worktree_abs_path(item, worktree_index)
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
                Self::worktree_member_abs_path(item, worktree_index, group_index, member_index)
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.projects.get(node_index)?;
                Self::worktree_vendored_abs_path(item, worktree_index, vendored_index)
            },
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => {
                let item = self.projects.get(node_index)?;
                item.submodules()
                    .get(submodule_index)
                    .map(|s| s.path.clone())
            },
        }
    }

    /// Check if a group at the given indices is an inline (unnamed) group.
    fn is_inline_group(&self, ni: usize, gi: usize) -> bool {
        let Some(item) = self.projects.get(ni) else {
            return true;
        };
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.groups().get(gi).is_some_and(|g| !g.is_named())
            },
            _ => true,
        }
    }

    /// Check if a worktree group at the given indices is an inline (unnamed) group.
    fn is_worktree_inline_group(&self, ni: usize, wi: usize, gi: usize) -> bool {
        let Some(item) = self.projects.get(ni) else {
            return true;
        };
        match item {
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

    fn worktree_display_path(item: &RootItem, wi: usize) -> Option<DisplayPath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.display_path())
                } else {
                    linked.get(wi - 1).map(ProjectFields::display_path)
                }
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.display_path())
                } else {
                    linked.get(wi - 1).map(ProjectFields::display_path)
                }
            },
            _ => None,
        }
    }

    fn worktree_member_display_path(
        item: &RootItem,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<DisplayPath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                let group = ws.groups().get(gi)?;
                group.members().get(mi).map(ProjectFields::display_path)
            },
            _ => None,
        }
    }

    fn worktree_vendored_display_path(
        item: &RootItem,
        wi: usize,
        vi: usize,
    ) -> Option<DisplayPath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                ws.vendored().get(vi).map(ProjectFields::display_path)
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                let pkg = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                pkg.vendored().get(vi).map(ProjectFields::display_path)
            },
            _ => None,
        }
    }

    fn worktree_abs_path(item: &RootItem, wi: usize) -> Option<AbsolutePath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.path().clone())
                } else {
                    linked.get(wi - 1).map(|p| p.path().clone())
                }
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.path().clone())
                } else {
                    linked.get(wi - 1).map(|p| p.path().clone())
                }
            },
            _ => None,
        }
    }

    fn worktree_member_abs_path(
        item: &RootItem,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<AbsolutePath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                let group = ws.groups().get(gi)?;
                group.members().get(mi).map(|p| p.path().clone())
            },
            _ => None,
        }
    }

    fn worktree_vendored_abs_path(item: &RootItem, wi: usize, vi: usize) -> Option<AbsolutePath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                ws.vendored().get(vi).map(|p| p.path().clone())
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                let pkg = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                pkg.vendored().get(vi).map(|p| p.path().clone())
            },
            _ => None,
        }
    }

    fn worktree_path_ref(item: &RootItem, wi: usize) -> Option<&Path> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.path().as_path())
                } else {
                    linked.get(wi - 1).map(|p| p.path().as_path())
                }
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.path().as_path())
                } else {
                    linked.get(wi - 1).map(|p| p.path().as_path())
                }
            },
            _ => None,
        }
    }

    fn worktree_member_path_ref(item: &RootItem, wi: usize, gi: usize, mi: usize) -> Option<&Path> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                let group = ws.groups().get(gi)?;
                group.members().get(mi).map(|p| p.path().as_path())
            },
            _ => None,
        }
    }

    fn worktree_vendored_path_ref(item: &RootItem, wi: usize, vi: usize) -> Option<&Path> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                ws.vendored().get(vi).map(|p| p.path().as_path())
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                let pkg = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                pkg.vendored().get(vi).map(|p| p.path().as_path())
            },
            _ => None,
        }
    }

    pub(in super::super) fn selected_is_expandable(&self) -> bool {
        let selected = self.pane_manager.pane(PaneId::ProjectList).pos();
        self.visible_rows()
            .get(selected)
            .copied()
            .and_then(|row| self.expand_key_for_row(row))
            .is_some()
    }

    pub(in super::super) fn expand_key_for_row(&self, row: VisibleRow) -> Option<ExpandKey> {
        match row {
            VisibleRow::Root { node_index } => self
                .projects
                .get(node_index)?
                .has_children()
                .then_some(ExpandKey::Node(node_index)),
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => Some(ExpandKey::Group(node_index, group_index)),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => {
                // In the new model, worktree entries don't expand themselves.
                // But we keep the expand key for backward compat with workspace worktrees.
                let item = self.projects.get(node_index)?;
                match item {
                    RootItem::Worktrees(WorktreeGroup::Workspaces {
                        primary, linked, ..
                    }) => {
                        let ws = if worktree_index == 0 {
                            primary
                        } else {
                            linked.get(worktree_index - 1)?
                        };
                        ws.has_members()
                            .then_some(ExpandKey::Worktree(node_index, worktree_index))
                    },
                    _ => None,
                }
            },
            VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                group_index,
            } => Some(ExpandKey::WorktreeGroup(
                node_index,
                worktree_index,
                group_index,
            )),
            VisibleRow::Member { .. }
            | VisibleRow::Vendored { .. }
            | VisibleRow::Submodule { .. }
            | VisibleRow::WorktreeMember { .. }
            | VisibleRow::WorktreeVendored { .. } => None,
        }
    }

    pub(in super::super) fn expand(&mut self) -> bool {
        if !self.selected_is_expandable() {
            return false;
        }
        let selected = self.pane_manager.pane(PaneId::ProjectList).pos();
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return false;
        };
        let Some(key) = self.expand_key_for_row(row) else {
            return false;
        };
        if self.expanded.insert(key) {
            self.dirty.rows.mark_dirty();
            true
        } else {
            false
        }
    }

    /// Remove `key` from expanded, recompute rows, and move cursor to `target`.
    pub(in super::super) fn collapse_to(&mut self, key: &ExpandKey, target: VisibleRow) {
        self.expanded.remove(key);
        self.dirty.rows.mark_dirty();
        self.ensure_visible_rows_cached();
        if let Some(pos) = self.visible_rows().iter().position(|r| *r == target) {
            self.pane_manager.pane_mut(PaneId::ProjectList).set_pos(pos);
        }
    }

    /// Try to remove `key` from expanded. If present, mark dirty and return `true`.
    /// Otherwise return `false` (caller should cascade to parent).
    pub(in super::super) fn try_collapse(&mut self, key: &ExpandKey) -> bool {
        if self.expanded.remove(key) {
            self.dirty.rows.mark_dirty();
            true
        } else {
            false
        }
    }

    pub(in super::super) fn collapse(&mut self) -> bool {
        let selected = self.pane_manager.pane(PaneId::ProjectList).pos();
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return false;
        };
        let expanded_before = self.expanded.len();
        let selected_before = self.pane_manager.pane(PaneId::ProjectList).pos();
        self.collapse_row(row);
        self.expanded.len() != expanded_before
            || self.pane_manager.pane(PaneId::ProjectList).pos() != selected_before
            || self.dirty.rows.is_dirty()
    }

    pub(in super::super) fn collapse_row(&mut self, row: VisibleRow) {
        match row {
            VisibleRow::Root { node_index: ni } => {
                self.try_collapse(&ExpandKey::Node(ni));
            },
            VisibleRow::GroupHeader {
                node_index: ni,
                group_index: gi,
            } => {
                if !self.try_collapse(&ExpandKey::Group(ni, gi)) {
                    self.collapse_to(&ExpandKey::Node(ni), VisibleRow::Root { node_index: ni });
                }
            },
            VisibleRow::Member {
                node_index: ni,
                group_index: gi,
                ..
            } => {
                if self.is_inline_group(ni, gi) {
                    self.collapse_to(&ExpandKey::Node(ni), VisibleRow::Root { node_index: ni });
                } else {
                    self.collapse_to(
                        &ExpandKey::Group(ni, gi),
                        VisibleRow::GroupHeader {
                            node_index:  ni,
                            group_index: gi,
                        },
                    );
                }
            },
            VisibleRow::Vendored { node_index: ni, .. }
            | VisibleRow::Submodule { node_index: ni, .. } => {
                self.collapse_to(&ExpandKey::Node(ni), VisibleRow::Root { node_index: ni });
            },
            VisibleRow::WorktreeEntry {
                node_index: ni,
                worktree_index: wi,
            } => {
                if !self.try_collapse(&ExpandKey::Worktree(ni, wi)) {
                    self.collapse_to(&ExpandKey::Node(ni), VisibleRow::Root { node_index: ni });
                }
            },
            VisibleRow::WorktreeGroupHeader {
                node_index: ni,
                worktree_index: wi,
                group_index: gi,
            } => {
                if !self.try_collapse(&ExpandKey::WorktreeGroup(ni, wi, gi)) {
                    self.collapse_to(
                        &ExpandKey::Worktree(ni, wi),
                        VisibleRow::WorktreeEntry {
                            node_index:     ni,
                            worktree_index: wi,
                        },
                    );
                }
            },
            VisibleRow::WorktreeMember {
                node_index: ni,
                worktree_index: wi,
                group_index: gi,
                ..
            } => {
                if self.is_worktree_inline_group(ni, wi, gi) {
                    self.collapse_to(
                        &ExpandKey::Worktree(ni, wi),
                        VisibleRow::WorktreeEntry {
                            node_index:     ni,
                            worktree_index: wi,
                        },
                    );
                } else {
                    self.collapse_to(
                        &ExpandKey::WorktreeGroup(ni, wi, gi),
                        VisibleRow::WorktreeGroupHeader {
                            node_index:     ni,
                            worktree_index: wi,
                            group_index:    gi,
                        },
                    );
                }
            },
            VisibleRow::WorktreeVendored {
                node_index: ni,
                worktree_index: wi,
                ..
            } => {
                self.collapse_to(
                    &ExpandKey::Worktree(ni, wi),
                    VisibleRow::WorktreeEntry {
                        node_index:     ni,
                        worktree_index: wi,
                    },
                );
            },
        }
    }

    pub(in super::super) fn row_count(&self) -> usize { self.visible_rows().len() }

    pub(in super::super) fn move_up(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.pane_manager.pane(PaneId::ProjectList).pos();
        if current > 0 {
            self.pane_manager
                .pane_mut(PaneId::ProjectList)
                .set_pos(current - 1);
        }
    }

    pub(in super::super) fn move_down(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.pane_manager.pane(PaneId::ProjectList).pos();
        if current < count - 1 {
            self.pane_manager
                .pane_mut(PaneId::ProjectList)
                .set_pos(current + 1);
        }
    }

    pub(in super::super) fn move_to_top(&mut self) {
        if self.row_count() > 0 {
            self.pane_manager.pane_mut(PaneId::ProjectList).set_pos(0);
        }
    }

    pub(in super::super) fn move_to_bottom(&mut self) {
        let count = self.row_count();
        if count > 0 {
            self.pane_manager
                .pane_mut(PaneId::ProjectList)
                .set_pos(count - 1);
        }
    }

    pub(in super::super) const fn collapse_anchor_row(row: VisibleRow) -> VisibleRow {
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

    pub(in super::super) fn expand_all(&mut self) {
        let selected_path = self
            .selection_paths
            .collapsed_selected
            .take()
            .or_else(|| self.selected_project_path().map(AbsolutePath::from));
        self.selection_paths.collapsed_anchor = None;
        for (ni, item) in self.projects.iter().enumerate() {
            if item.has_children() {
                self.expanded.insert(ExpandKey::Node(ni));
            }
            match item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    for (gi, group) in ws.groups().iter().enumerate() {
                        if group.is_named() {
                            self.expanded.insert(ExpandKey::Group(ni, gi));
                        }
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    for (wi, ws) in std::iter::once(primary).chain(linked.iter()).enumerate() {
                        if ws.has_members() {
                            self.expanded.insert(ExpandKey::Worktree(ni, wi));
                        }
                        for (gi, group) in ws.groups().iter().enumerate() {
                            if group.is_named() {
                                self.expanded.insert(ExpandKey::WorktreeGroup(ni, wi, gi));
                            }
                        }
                    }
                },
                _ => {},
            }
        }
        self.dirty.rows.mark_dirty();
        if let Some(path) = selected_path {
            self.select_project_in_tree(path.as_path());
        }
    }

    pub(in super::super) fn collapse_all(&mut self) {
        let selected_path = self.selected_project_path().map(AbsolutePath::from);
        let anchor = self.selected_row().map(Self::collapse_anchor_row);
        self.expanded.clear();
        self.dirty.rows.mark_dirty();
        self.ensure_visible_rows_cached();
        if let Some(anchor) = anchor
            && let Some(pos) = self.visible_rows().iter().position(|row| *row == anchor)
        {
            self.pane_manager.pane_mut(PaneId::ProjectList).set_pos(pos);
        }
        let anchor_path = self.selected_project_path().map(AbsolutePath::from);
        if selected_path == anchor_path {
            self.selection_paths.collapsed_selected = None;
            self.selection_paths.collapsed_anchor = None;
        } else {
            self.selection_paths.collapsed_selected = selected_path;
            self.selection_paths.collapsed_anchor = anchor_path;
        }
    }

    pub(in super::super) fn expand_path_in_tree(&mut self, target_path: &Path) {
        for (ni, item) in self.projects.iter().enumerate() {
            match item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    for (gi, group) in ws.groups().iter().enumerate() {
                        for member in group.members() {
                            if member.path() == target_path {
                                self.expanded.insert(ExpandKey::Node(ni));
                                if group.is_named() {
                                    self.expanded.insert(ExpandKey::Group(ni, gi));
                                }
                            }
                        }
                    }
                    for vendored in ws.vendored() {
                        if vendored.path() == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
                        }
                    }
                },
                RootItem::Rust(RustProject::Package(pkg)) => {
                    for vendored in pkg.vendored() {
                        if vendored.path() == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
                        }
                    }
                },
                RootItem::NonRust(_) => {},
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    for (wi, ws) in std::iter::once(primary).chain(linked.iter()).enumerate() {
                        if ws.path() == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
                        }
                        for (gi, group) in ws.groups().iter().enumerate() {
                            for member in group.members() {
                                if member.path() == target_path {
                                    self.expanded.insert(ExpandKey::Node(ni));
                                    self.expanded.insert(ExpandKey::Worktree(ni, wi));
                                    if group.is_named() {
                                        self.expanded.insert(ExpandKey::WorktreeGroup(ni, wi, gi));
                                    }
                                }
                            }
                        }
                        for vendored in ws.vendored() {
                            if vendored.path() == target_path {
                                self.expanded.insert(ExpandKey::Node(ni));
                                self.expanded.insert(ExpandKey::Worktree(ni, wi));
                            }
                        }
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    for (wi, pkg) in std::iter::once(primary).chain(linked.iter()).enumerate() {
                        if pkg.path() == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
                        }
                        for vendored in pkg.vendored() {
                            if vendored.path() == target_path {
                                self.expanded.insert(ExpandKey::Node(ni));
                                self.expanded.insert(ExpandKey::Worktree(ni, wi));
                            }
                        }
                    }
                },
            }
        }
    }

    pub(in super::super) fn row_matches_project_path(
        &self,
        row: VisibleRow,
        target_path: &Path,
    ) -> bool {
        self.path_for_row(row)
            .is_some_and(|path| path == target_path)
    }

    pub(in super::super) fn select_matching_visible_row(&mut self, target_path: &Path) {
        self.ensure_visible_rows_cached();
        let selected_index = self
            .visible_rows()
            .iter()
            .position(|row| self.row_matches_project_path(*row, target_path));
        if let Some(selected_index) = selected_index {
            self.pane_manager
                .pane_mut(PaneId::ProjectList)
                .set_pos(selected_index);
        }
    }

    pub(in super::super) fn select_project_in_tree(&mut self, target_path: &Path) {
        self.expand_path_in_tree(target_path);
        self.dirty.rows.mark_dirty();
        self.select_matching_visible_row(target_path);
    }
}
