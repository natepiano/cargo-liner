use std::path::Path;
use std::path::PathBuf;

use nucleo_matcher::Matcher;
use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::Atom;
use nucleo_matcher::pattern::AtomKind;
use nucleo_matcher::pattern::CaseMatching;
use nucleo_matcher::pattern::Normalization;

use super::snapshots;
use super::types::App;
use super::types::DetailCache;
use super::types::ExpandKey;
use super::types::SearchMode;
use super::types::VisibleRow;
use crate::constants::WORKTREE;
use crate::project::Package;
use crate::project::ProjectListItem;
use crate::project::RustProject;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::tui;
use crate::tui::columns::COL_NAME;
use crate::tui::columns::ResolvedWidths;
use crate::tui::render::PREFIX_ROOT_COLLAPSED;

impl App {
    pub fn ensure_visible_rows_cached(&mut self) {
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
    pub fn visible_rows(&self) -> &[VisibleRow] { &self.cached_visible_rows }

    /// Keep fit-to-content widths rebuilding in the background, never inline on the UI thread.
    pub fn ensure_fit_widths_cached(&mut self) { self.request_fit_widths_build(); }

    pub(super) fn observe_name_width(widths: &mut ResolvedWidths, content_width: usize) {
        use COL_NAME;

        widths.observe(COL_NAME, Self::name_width_with_gutter(content_width));
    }

    pub(super) const fn name_width_with_gutter(content_width: usize) -> usize {
        content_width.saturating_add(1)
    }

    pub(super) fn fit_name_for_item(item: &ProjectListItem) -> usize {
        let dw = tui::columns::display_width;
        let mut name = item.display_name();
        let live = match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let count = std::iter::once(wtg.primary().visibility())
                    .chain(
                        wtg.linked()
                            .iter()
                            .map(crate::project::RustProject::visibility),
                    )
                    .filter(|v| !matches!(v, Visibility::Deleted | Visibility::Dismissed))
                    .count();
                if count <= 1 { 0 } else { count }
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                let count = std::iter::once(wtg.primary().visibility())
                    .chain(
                        wtg.linked()
                            .iter()
                            .map(crate::project::RustProject::visibility),
                    )
                    .filter(|v| !matches!(v, Visibility::Deleted | Visibility::Dismissed))
                    .count();
                if count <= 1 { 0 } else { count }
            },
            _ => 0,
        };
        if live > 0 {
            name = format!("{name} {WORKTREE}:{live}");
        }
        dw(PREFIX_ROOT_COLLAPSED) + dw(&name)
    }

    /// Keep disk sort caches rebuilding in the background, never inline on the UI thread.
    pub fn ensure_disk_cache(&mut self) { self.request_disk_cache_build(); }

    /// Ensure the cached `DetailInfo` is up to date for the selected project.
    /// The cache is valid only when the generation AND path both match.
    pub fn ensure_detail_cached(&mut self) {
        let current_selection = self.current_detail_selection_key();

        if let Some(ref cache) = self.cached_detail
            && cache.generation == self.detail_generation
            && cache.selection == current_selection
        {
            return;
        }

        self.cached_detail = self.build_selected_detail_info().map(|info| DetailCache {
            generation: self.detail_generation,
            selection: current_selection,
            info,
        });
    }

    /// Build `DetailInfo` for the currently selected row, resolving through
    /// the `project_list_items` hierarchy.
    fn build_selected_detail_info(&self) -> Option<tui::detail::DetailInfo> {
        let row = self.selected_row()?;
        match row {
            VisibleRow::Root { node_index } => {
                let item = self.projects.get(node_index)?;
                Some(tui::detail::build_detail_info(self, item))
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
                let pkg = Self::resolve_member(item, group_index, member_index)?;
                Some(tui::detail::build_detail_info_for_member(self, pkg))
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => {
                let item = self.projects.get(node_index)?;
                let pkg = Self::resolve_vendored(item, vendored_index)?;
                Some(tui::detail::build_detail_info_for_member(self, pkg))
            },
            VisibleRow::GroupHeader { node_index, .. } => {
                // Group headers show the parent project's detail
                let item = self.projects.get(node_index)?;
                Some(tui::detail::build_detail_info(self, item))
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
                Some(tui::detail::build_detail_info_for_member(self, pkg))
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.projects.get(node_index)?;
                let pkg = Self::worktree_vendored_ref(item, worktree_index, vendored_index)?;
                Some(tui::detail::build_detail_info_for_member(self, pkg))
            },
        }
    }

    /// Resolve a member `Project<Package>` from a `ProjectListItem`.
    fn resolve_member(
        item: &ProjectListItem,
        group_index: usize,
        member_index: usize,
    ) -> Option<&RustProject<Package>> {
        match item {
            ProjectListItem::Workspace(ws) => {
                ws.groups().get(group_index)?.members().get(member_index)
            },
            _ => None,
        }
    }

    /// Resolve a vendored `Project<Package>` from a `ProjectListItem`.
    fn resolve_vendored(
        item: &ProjectListItem,
        vendored_index: usize,
    ) -> Option<&RustProject<Package>> {
        match item {
            ProjectListItem::Workspace(ws) => ws.vendored().get(vendored_index),
            ProjectListItem::Package(pkg) => pkg.vendored().get(vendored_index),
            _ => None,
        }
    }

    /// Resolve a member inside a worktree entry.
    fn worktree_member_ref(
        item: &ProjectListItem,
        worktree_index: usize,
        group_index: usize,
        member_index: usize,
    ) -> Option<&RustProject<Package>> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let ws = wtg.linked().get(worktree_index)?;
                ws.groups().get(group_index)?.members().get(member_index)
            },
            _ => None,
        }
    }

    /// Resolve a vendored package inside a worktree entry.
    fn worktree_vendored_ref(
        item: &ProjectListItem,
        worktree_index: usize,
        vendored_index: usize,
    ) -> Option<&RustProject<Package>> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let ws = wtg.linked().get(worktree_index)?;
                ws.vendored().get(vendored_index)
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                let pkg = wtg.linked().get(worktree_index)?;
                pkg.vendored().get(vendored_index)
            },
            _ => None,
        }
    }

    /// Build detail info for a worktree entry (a linked workspace or package).
    fn build_worktree_detail(
        &self,
        item: &ProjectListItem,
        worktree_index: usize,
    ) -> Option<tui::detail::DetailInfo> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let ws = wtg.linked().get(worktree_index)?;
                let display_path = ws.display_path();
                Some(tui::detail::build_detail_info_for_workspace_ref(
                    self,
                    ws,
                    &display_path,
                ))
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                let pkg = wtg.linked().get(worktree_index)?;
                Some(tui::detail::build_detail_info_for_member(self, pkg))
            },
            _ => None,
        }
    }

    pub(super) fn selected_row(&self) -> Option<VisibleRow> {
        if self.is_searching() && !self.search_query.is_empty() {
            return None;
        }
        let rows = self.visible_rows();
        let selected = self.list_state.selected()?;
        rows.get(selected).copied()
    }

    pub(super) fn current_detail_selection_key(&self) -> String {
        if self.is_searching() && !self.search_query.is_empty() {
            return self
                .selected_project_path()
                .map(|path| format!("search:{}", path.display()))
                .unwrap_or_default();
        }
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
            None => String::new(),
        }
    }

    /// Returns the `ProjectListItem` when a root row is selected.
    pub fn selected_item(&self) -> Option<&ProjectListItem> {
        match self.selected_row()? {
            VisibleRow::Root { node_index } => self.projects.get(node_index),
            _ => None,
        }
    }

    /// Returns the absolute path of the currently selected project, borrowed
    /// from `project_list_items` (or `flat_entries` during search).
    pub fn selected_project_path(&self) -> Option<&Path> {
        if self.is_searching() && !self.search_query.is_empty() {
            let selected = self.list_state.selected()?;
            let flat_idx = *self.filtered.get(selected)?;
            let entry = self.projects.flat_entries().get(flat_idx)?;
            Some(entry.abs_path.as_path())
        } else {
            let row = self.selected_row()?;
            self.path_for_row(row)
        }
    }

    /// Given a `VisibleRow`, resolve the absolute `&Path` borrowed from
    /// `project_list_items`.
    pub fn path_for_row(&self, row: VisibleRow) -> Option<&Path> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                let item = self.projects.get(node_index)?;
                Some(item.path())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
                match item {
                    ProjectListItem::Workspace(ws) => {
                        let group = ws.groups().get(group_index)?;
                        let member = group.members().get(member_index)?;
                        Some(member.path())
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
                    ProjectListItem::Workspace(ws) => {
                        ws.vendored().get(vendored_index).map(RustProject::path)
                    },
                    ProjectListItem::Package(pkg) => {
                        pkg.vendored().get(vendored_index).map(RustProject::path)
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
        }
    }

    /// Resolve the display path of the currently selected row using `project_list_items`.
    pub(super) fn selected_display_path(&self) -> Option<String> {
        let rows = self.visible_rows();
        let selected = self.list_state.selected()?;
        let row = rows.get(selected)?;
        self.display_path_for_row(*row)
    }

    /// Given a `VisibleRow`, resolve the display path from `project_list_items`.
    pub fn display_path_for_row(&self, row: VisibleRow) -> Option<String> {
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
                    ProjectListItem::Workspace(ws) => {
                        let group = ws.groups().get(group_index)?;
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
                    ProjectListItem::Workspace(ws) => ws
                        .vendored()
                        .get(vendored_index)
                        .map(RustProject::display_path),
                    ProjectListItem::Package(pkg) => pkg
                        .vendored()
                        .get(vendored_index)
                        .map(RustProject::display_path),
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
        }
    }

    /// Given a `VisibleRow`, resolve the absolute path from `project_list_items`.
    pub fn abs_path_for_row(&self, row: VisibleRow) -> Option<PathBuf> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                let item = self.projects.get(node_index)?;
                Some(item.path().to_path_buf())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.projects.get(node_index)?;
                match item {
                    ProjectListItem::Workspace(ws) => {
                        let group = ws.groups().get(group_index)?;
                        let member = group.members().get(member_index)?;
                        Some(member.path().to_path_buf())
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
                    ProjectListItem::Workspace(ws) => ws
                        .vendored()
                        .get(vendored_index)
                        .map(|p| p.path().to_path_buf()),
                    ProjectListItem::Package(pkg) => pkg
                        .vendored()
                        .get(vendored_index)
                        .map(|p| p.path().to_path_buf()),
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
        }
    }

    /// Check if a group at the given indices is an inline (unnamed) group.
    fn is_inline_group(&self, ni: usize, gi: usize) -> bool {
        let Some(item) = self.projects.get(ni) else {
            return true;
        };
        match item {
            ProjectListItem::Workspace(ws) => ws.groups().get(gi).is_some_and(|g| !g.is_named()),
            _ => true,
        }
    }

    /// Check if a worktree group at the given indices is an inline (unnamed) group.
    fn is_worktree_inline_group(&self, ni: usize, wi: usize, gi: usize) -> bool {
        let Some(item) = self.projects.get(ni) else {
            return true;
        };
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let ws = if wi == 0 {
                    wtg.primary()
                } else {
                    match wtg.linked().get(wi - 1) {
                        Some(ws) => ws,
                        None => return true,
                    }
                };
                ws.groups().get(gi).is_some_and(|g| !g.is_named())
            },
            _ => true,
        }
    }

    fn worktree_display_path(item: &ProjectListItem, wi: usize) -> Option<String> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                if wi == 0 {
                    Some(wtg.primary().display_path())
                } else {
                    wtg.linked().get(wi - 1).map(RustProject::display_path)
                }
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                if wi == 0 {
                    Some(wtg.primary().display_path())
                } else {
                    wtg.linked().get(wi - 1).map(RustProject::display_path)
                }
            },
            _ => None,
        }
    }

    fn worktree_member_display_path(
        item: &ProjectListItem,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<String> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let ws = if wi == 0 {
                    wtg.primary()
                } else {
                    wtg.linked().get(wi - 1)?
                };
                let group = ws.groups().get(gi)?;
                group.members().get(mi).map(RustProject::display_path)
            },
            _ => None,
        }
    }

    fn worktree_vendored_display_path(
        item: &ProjectListItem,
        wi: usize,
        vi: usize,
    ) -> Option<String> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let ws = if wi == 0 {
                    wtg.primary()
                } else {
                    wtg.linked().get(wi - 1)?
                };
                ws.vendored().get(vi).map(RustProject::display_path)
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                let pkg = if wi == 0 {
                    wtg.primary()
                } else {
                    wtg.linked().get(wi - 1)?
                };
                pkg.vendored().get(vi).map(RustProject::display_path)
            },
            _ => None,
        }
    }

    fn worktree_abs_path(item: &ProjectListItem, wi: usize) -> Option<PathBuf> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                if wi == 0 {
                    Some(wtg.primary().path().to_path_buf())
                } else {
                    wtg.linked().get(wi - 1).map(|p| p.path().to_path_buf())
                }
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                if wi == 0 {
                    Some(wtg.primary().path().to_path_buf())
                } else {
                    wtg.linked().get(wi - 1).map(|p| p.path().to_path_buf())
                }
            },
            _ => None,
        }
    }

    fn worktree_member_abs_path(
        item: &ProjectListItem,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<PathBuf> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let ws = if wi == 0 {
                    wtg.primary()
                } else {
                    wtg.linked().get(wi - 1)?
                };
                let group = ws.groups().get(gi)?;
                group.members().get(mi).map(|p| p.path().to_path_buf())
            },
            _ => None,
        }
    }

    fn worktree_vendored_abs_path(item: &ProjectListItem, wi: usize, vi: usize) -> Option<PathBuf> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let ws = if wi == 0 {
                    wtg.primary()
                } else {
                    wtg.linked().get(wi - 1)?
                };
                ws.vendored().get(vi).map(|p| p.path().to_path_buf())
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                let pkg = if wi == 0 {
                    wtg.primary()
                } else {
                    wtg.linked().get(wi - 1)?
                };
                pkg.vendored().get(vi).map(|p| p.path().to_path_buf())
            },
            _ => None,
        }
    }

    fn worktree_path_ref(item: &ProjectListItem, wi: usize) -> Option<&Path> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                if wi == 0 {
                    Some(wtg.primary().path())
                } else {
                    wtg.linked().get(wi - 1).map(RustProject::path)
                }
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                if wi == 0 {
                    Some(wtg.primary().path())
                } else {
                    wtg.linked().get(wi - 1).map(RustProject::path)
                }
            },
            _ => None,
        }
    }

    fn worktree_member_path_ref(
        item: &ProjectListItem,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<&Path> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let ws = if wi == 0 {
                    wtg.primary()
                } else {
                    wtg.linked().get(wi - 1)?
                };
                let group = ws.groups().get(gi)?;
                group.members().get(mi).map(RustProject::path)
            },
            _ => None,
        }
    }

    fn worktree_vendored_path_ref(item: &ProjectListItem, wi: usize, vi: usize) -> Option<&Path> {
        match item {
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                let ws = if wi == 0 {
                    wtg.primary()
                } else {
                    wtg.linked().get(wi - 1)?
                };
                ws.vendored().get(vi).map(RustProject::path)
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                let pkg = if wi == 0 {
                    wtg.primary()
                } else {
                    wtg.linked().get(wi - 1)?
                };
                pkg.vendored().get(vi).map(RustProject::path)
            },
            _ => None,
        }
    }

    pub(super) fn selected_is_expandable(&self) -> bool {
        if self.is_searching() && !self.search_query.is_empty() {
            return false;
        }
        let rows = self.visible_rows();
        let Some(selected) = self.list_state.selected() else {
            return false;
        };
        match rows.get(selected) {
            Some(VisibleRow::Root { node_index }) => self
                .projects
                .get(*node_index)
                .is_some_and(ProjectListItem::has_children),
            Some(VisibleRow::GroupHeader { .. } | VisibleRow::WorktreeGroupHeader { .. }) => true,
            _ => false,
        }
    }

    pub(super) fn expand_key_for_row(&self, row: VisibleRow) -> Option<ExpandKey> {
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
                    ProjectListItem::WorkspaceWorktrees(wtg) => {
                        let ws = if worktree_index == 0 {
                            wtg.primary()
                        } else {
                            wtg.linked().get(worktree_index - 1)?
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
            | VisibleRow::WorktreeMember { .. }
            | VisibleRow::WorktreeVendored { .. } => None,
        }
    }

    pub fn expand(&mut self) -> bool {
        if !self.selected_is_expandable() {
            return false;
        }
        let Some(selected) = self.list_state.selected() else {
            return false;
        };
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
    pub(super) fn collapse_to(&mut self, key: &ExpandKey, target: VisibleRow) {
        self.expanded.remove(key);
        self.dirty.rows.mark_dirty();
        self.ensure_visible_rows_cached();
        if let Some(pos) = self.visible_rows().iter().position(|r| *r == target) {
            self.list_state.select(Some(pos));
        }
    }

    /// Try to remove `key` from expanded. If present, mark dirty and return `true`.
    /// Otherwise return `false` (caller should cascade to parent).
    pub(super) fn try_collapse(&mut self, key: &ExpandKey) -> bool {
        if self.expanded.remove(key) {
            self.dirty.rows.mark_dirty();
            true
        } else {
            false
        }
    }

    pub fn collapse(&mut self) -> bool {
        let Some(selected) = self.list_state.selected() else {
            return false;
        };
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return false;
        };
        let expanded_before = self.expanded.len();
        let selected_before = self.list_state.selected();
        self.collapse_row(row);
        self.expanded.len() != expanded_before
            || self.list_state.selected() != selected_before
            || self.dirty.rows.is_dirty()
    }

    pub(super) fn collapse_row(&mut self, row: VisibleRow) {
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
            VisibleRow::Vendored { node_index: ni, .. } => {
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

    pub fn row_count(&self) -> usize {
        if self.is_searching() && !self.search_query.is_empty() {
            self.filtered.len()
        } else {
            self.visible_rows().len()
        }
    }

    pub fn move_up(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        if current > 0 {
            self.list_state.select(Some(current - 1));
        }
    }

    pub fn move_down(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        if current < count - 1 {
            self.list_state.select(Some(current + 1));
        }
    }

    pub fn move_to_top(&mut self) {
        if self.row_count() > 0 {
            self.list_state.select(Some(0));
        }
    }

    pub fn move_to_bottom(&mut self) {
        let count = self.row_count();
        if count > 0 {
            self.list_state.select(Some(count - 1));
        }
    }

    pub(super) const fn collapse_anchor_row(row: VisibleRow) -> VisibleRow {
        match row {
            VisibleRow::GroupHeader { node_index, .. }
            | VisibleRow::Member { node_index, .. }
            | VisibleRow::Vendored { node_index, .. } => VisibleRow::Root { node_index },
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

    pub fn expand_all(&mut self) {
        let selected_path = self
            .selection_paths
            .collapsed_selected
            .take()
            .or_else(|| self.selected_display_path());
        self.selection_paths.collapsed_anchor = None;
        for (ni, item) in self.projects.iter().enumerate() {
            if item.has_children() {
                self.expanded.insert(ExpandKey::Node(ni));
            }
            match item {
                ProjectListItem::Workspace(ws) => {
                    for (gi, group) in ws.groups().iter().enumerate() {
                        if group.is_named() {
                            self.expanded.insert(ExpandKey::Group(ni, gi));
                        }
                    }
                },
                ProjectListItem::WorkspaceWorktrees(wtg) => {
                    let all_ws: Vec<&RustProject<Workspace>> = std::iter::once(wtg.primary())
                        .chain(wtg.linked().iter())
                        .collect();
                    for (wi, ws) in all_ws.iter().enumerate() {
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
            self.select_project_in_tree(&path);
        }
    }

    pub fn collapse_all(&mut self) {
        let selected_path = self.selected_display_path();
        let anchor = self.selected_row().map(Self::collapse_anchor_row);
        self.expanded.clear();
        self.dirty.rows.mark_dirty();
        self.ensure_visible_rows_cached();
        if let Some(anchor) = anchor
            && let Some(pos) = self.visible_rows().iter().position(|row| *row == anchor)
        {
            self.list_state.select(Some(pos));
        }
        let anchor_path = self.selected_display_path();
        if selected_path == anchor_path {
            self.selection_paths.collapsed_selected = None;
            self.selection_paths.collapsed_anchor = None;
        } else {
            self.selection_paths.collapsed_selected = selected_path;
            self.selection_paths.collapsed_anchor = anchor_path;
        }
    }

    pub fn cancel_search(&mut self) {
        self.end_search();
        self.search_query.clear();
        self.filtered.clear();
        self.dirty.rows.mark_dirty();
        self.close_overlay();
        if !self.projects.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    pub fn confirm_search(&mut self) {
        let project_path = self
            .list_state
            .selected()
            .and_then(|sel| self.filtered.get(sel).copied())
            .and_then(|flat_idx| self.projects.flat_entries().get(flat_idx))
            .map(|entry| entry.path.clone());
        self.end_search();
        self.search_query.clear();
        self.filtered.clear();
        self.dirty.rows.mark_dirty();
        self.close_overlay();

        if let Some(target_path) = project_path {
            self.select_project_in_tree(&target_path);
        }
    }

    pub(super) fn expand_path_in_tree(&mut self, target_path: &str) {
        for (ni, item) in self.projects.iter().enumerate() {
            match item {
                ProjectListItem::Workspace(ws) => {
                    for (gi, group) in ws.groups().iter().enumerate() {
                        for member in group.members() {
                            if member.display_path() == target_path {
                                self.expanded.insert(ExpandKey::Node(ni));
                                if group.is_named() {
                                    self.expanded.insert(ExpandKey::Group(ni, gi));
                                }
                            }
                        }
                    }
                    for vendored in ws.vendored() {
                        if vendored.display_path() == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
                        }
                    }
                },
                ProjectListItem::Package(pkg) => {
                    for vendored in pkg.vendored() {
                        if vendored.display_path() == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
                        }
                    }
                },
                ProjectListItem::NonRust(_) => {},
                ProjectListItem::WorkspaceWorktrees(wtg) => {
                    let all_ws: Vec<&RustProject<Workspace>> = std::iter::once(wtg.primary())
                        .chain(wtg.linked().iter())
                        .collect();
                    for (wi, ws) in all_ws.iter().enumerate() {
                        if ws.display_path() == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
                        }
                        for (gi, group) in ws.groups().iter().enumerate() {
                            for member in group.members() {
                                if member.display_path() == target_path {
                                    self.expanded.insert(ExpandKey::Node(ni));
                                    self.expanded.insert(ExpandKey::Worktree(ni, wi));
                                    if group.is_named() {
                                        self.expanded.insert(ExpandKey::WorktreeGroup(ni, wi, gi));
                                    }
                                }
                            }
                        }
                        for vendored in ws.vendored() {
                            if vendored.display_path() == target_path {
                                self.expanded.insert(ExpandKey::Node(ni));
                                self.expanded.insert(ExpandKey::Worktree(ni, wi));
                            }
                        }
                    }
                },
                ProjectListItem::PackageWorktrees(wtg) => {
                    let all_pkg: Vec<&RustProject<Package>> = std::iter::once(wtg.primary())
                        .chain(wtg.linked().iter())
                        .collect();
                    for (wi, pkg) in all_pkg.iter().enumerate() {
                        if pkg.display_path() == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
                        }
                        for vendored in pkg.vendored() {
                            if vendored.display_path() == target_path {
                                self.expanded.insert(ExpandKey::Node(ni));
                                self.expanded.insert(ExpandKey::Worktree(ni, wi));
                            }
                        }
                    }
                },
            }
        }
    }

    pub(super) fn row_matches_project_path(&self, row: VisibleRow, target_path: &str) -> bool {
        self.display_path_for_row(row)
            .is_some_and(|dp| dp == target_path)
    }

    pub(super) fn select_matching_visible_row(&mut self, target_path: &str) {
        self.ensure_visible_rows_cached();
        let selected_index = self
            .visible_rows()
            .iter()
            .position(|row| self.row_matches_project_path(*row, target_path));
        if let Some(selected_index) = selected_index {
            self.list_state.select(Some(selected_index));
        }
    }

    pub fn select_project_in_tree(&mut self, target_path: &str) {
        self.expand_path_in_tree(target_path);
        self.dirty.rows.mark_dirty();
        self.select_matching_visible_row(target_path);
    }

    pub fn update_search(&mut self, query: &str) {
        self.search_query = query.to_string();

        if query.is_empty() {
            self.end_search();
            self.filtered.clear();
            self.list_state.select(Some(0));
            return;
        }

        self.ui_modes.search = SearchMode::Active;

        let mut matcher = Matcher::default();
        let atom = Atom::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
            false,
        );

        let mut scored: Vec<(usize, u16)> = self
            .projects
            .flat_entries()
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let mut buf = Vec::new();
                let haystack = Utf32Str::new(&entry.name, &mut buf);
                atom.score(haystack, &mut matcher).map(|score| (i, score))
            })
            .collect();

        scored.sort_by(|a, b| b.1.cmp(&a.1));
        self.filtered = scored.into_iter().map(|(i, _)| i).collect();

        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }
}
