use nucleo_matcher::Matcher;
use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::Atom;
use nucleo_matcher::pattern::AtomKind;
use nucleo_matcher::pattern::CaseMatching;
use nucleo_matcher::pattern::Normalization;

use super::App;
use super::DetailCache;
use super::ExpandKey;
use super::ResolvedWidths;
use super::SearchMode;
use super::VisibleRow;
use super::build_visible_rows;
use crate::constants::WORKTREE;
use crate::project::RustProject;
use crate::scan::ProjectNode;
use crate::tui;
use crate::tui::columns::COL_NAME;
use crate::tui::render::PREFIX_ROOT_COLLAPSED;

impl App {
    pub(super) fn ensure_visible_rows_cached_impl(&mut self) {
        if !self.dirty.rows.is_dirty() {
            return;
        }
        self.dirty.rows.mark_clean();
        self.cached_visible_rows = build_visible_rows(&self.nodes, &self.expanded);
    }

    /// Return the cached visible rows. Must call `ensure_visible_rows_cached()` first.
    pub(super) fn visible_rows_impl(&self) -> &[VisibleRow] { &self.cached_visible_rows }

    /// Keep fit-to-content widths rebuilding in the background, never inline on the UI thread.
    pub(super) fn ensure_fit_widths_cached_impl(&mut self) { self.request_fit_widths_build(); }

    /// Iterate all group members in a node, including those nested under worktree entries.
    pub(super) fn all_group_members(node: &ProjectNode) -> impl Iterator<Item = &RustProject> {
        let direct = node.groups.iter().flat_map(|g| g.members.iter());
        let wt = node
            .worktrees
            .iter()
            .flat_map(|wt| wt.groups.iter().flat_map(|g| g.members.iter()));
        direct.chain(wt)
    }

    pub(super) fn all_vendored_projects(node: &ProjectNode) -> impl Iterator<Item = &RustProject> {
        let direct = node.vendored.iter();
        let wt = node
            .worktrees
            .iter()
            .flat_map(|worktree| worktree.vendored.iter());
        direct.chain(wt)
    }

    pub(super) fn observe_name_width(widths: &mut ResolvedWidths, content_width: usize) {
        use COL_NAME;

        widths.observe(COL_NAME, Self::name_width_with_gutter(content_width));
    }

    pub(super) const fn name_width_with_gutter(content_width: usize) -> usize {
        content_width.saturating_add(1)
    }

    pub(super) fn fit_name_for_node(node: &ProjectNode, live_worktrees: usize) -> usize {
        let dw = tui::columns::display_width;
        let mut name = node.project.display_name();
        if live_worktrees > 0 {
            name = format!("{name} {WORKTREE}:{live_worktrees}");
        }
        dw(PREFIX_ROOT_COLLAPSED) + dw(&name)
    }

    /// Keep disk sort caches rebuilding in the background, never inline on the UI thread.
    pub(super) fn ensure_disk_cache_impl(&mut self) { self.request_disk_cache_build(); }

    /// Ensure the cached `DetailInfo` is up to date for the selected project.
    /// The cache is valid only when the generation AND path both match.
    pub(super) fn ensure_detail_cached_impl(&mut self) {
        let current_selection = self.current_detail_selection_key();

        if let Some(ref cache) = self.cached_detail
            && cache.generation == self.detail_generation
            && cache.selection == current_selection
        {
            return;
        }

        self.cached_detail = self.selected_project().map(|p| DetailCache {
            generation: self.detail_generation,
            selection:  current_selection,
            info:       tui::detail::build_detail_info(self, p),
        });
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
                .selected_project()
                .map(|project| format!("search:{}", project.path))
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

    /// Returns the `ProjectNode` when a root row is selected (not a member or worktree).
    pub(super) fn selected_node_impl(&self) -> Option<&ProjectNode> {
        match self.selected_row()? {
            VisibleRow::Root { node_index } => self.nodes.get(node_index),
            _ => None,
        }
    }

    pub(super) fn selected_project_impl(&self) -> Option<&RustProject> {
        if self.is_searching() && !self.search_query.is_empty() {
            let selected = self.list_state.selected()?;
            let flat_idx = *self.filtered.get(selected)?;
            let entry = self.flat_entries.get(flat_idx)?;
            self.project_by_path(&entry.path)
        } else {
            let rows = self.visible_rows();
            let selected = self.list_state.selected()?;
            match rows.get(selected)? {
                VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                    Some(&self.nodes.get(*node_index)?.project)
                },
                VisibleRow::Member {
                    node_index,
                    group_index,
                    member_index,
                } => {
                    let node = self.nodes.get(*node_index)?;
                    let group = node.groups.get(*group_index)?;
                    group.members.get(*member_index)
                },
                VisibleRow::Vendored {
                    node_index,
                    vendored_index,
                } => self.nodes.get(*node_index)?.vendored.get(*vendored_index),
                VisibleRow::WorktreeEntry {
                    node_index,
                    worktree_index,
                }
                | VisibleRow::WorktreeGroupHeader {
                    node_index,
                    worktree_index,
                    ..
                } => {
                    let node = self.nodes.get(*node_index)?;
                    let wt = node.worktrees.get(*worktree_index)?;
                    Some(&wt.project)
                },
                VisibleRow::WorktreeMember {
                    node_index,
                    worktree_index,
                    group_index,
                    member_index,
                } => {
                    let wt = self
                        .nodes
                        .get(*node_index)?
                        .worktrees
                        .get(*worktree_index)?;
                    let group = wt.groups.get(*group_index)?;
                    group.members.get(*member_index)
                },
                VisibleRow::WorktreeVendored {
                    node_index,
                    worktree_index,
                    vendored_index,
                } => self
                    .nodes
                    .get(*node_index)?
                    .worktrees
                    .get(*worktree_index)?
                    .vendored
                    .get(*vendored_index),
            }
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
            Some(VisibleRow::Root { node_index }) => self.nodes[*node_index].has_children(),
            Some(VisibleRow::GroupHeader { .. } | VisibleRow::WorktreeGroupHeader { .. }) => true,
            Some(VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }) => self.nodes[*node_index].worktrees[*worktree_index].has_children(),
            _ => false,
        }
    }

    pub(super) fn expand_key_for_row(&self, row: VisibleRow) -> Option<ExpandKey> {
        match row {
            VisibleRow::Root { node_index } => self.nodes[node_index]
                .has_children()
                .then_some(ExpandKey::Node(node_index)),
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => Some(ExpandKey::Group(node_index, group_index)),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => self.nodes[node_index].worktrees[worktree_index]
                .has_children()
                .then_some(ExpandKey::Worktree(node_index, worktree_index)),
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

    pub(super) fn expand_impl(&mut self) -> bool {
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

    pub(super) fn collapse_impl(&mut self) -> bool {
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
                if self.nodes[ni].groups[gi].name.is_empty() {
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
                if self.nodes[ni].worktrees[wi].groups[gi].name.is_empty() {
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

    pub(super) fn row_count_impl(&self) -> usize {
        if self.is_searching() && !self.search_query.is_empty() {
            self.filtered.len()
        } else {
            self.visible_rows().len()
        }
    }

    pub(super) fn move_up_impl(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        if current > 0 {
            self.list_state.select(Some(current - 1));
        }
    }

    pub(super) fn move_down_impl(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        if current < count - 1 {
            self.list_state.select(Some(current + 1));
        }
    }

    pub(super) fn move_to_top_impl(&mut self) {
        if self.row_count() > 0 {
            self.list_state.select(Some(0));
        }
    }

    pub(super) fn move_to_bottom_impl(&mut self) {
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

    pub(super) fn expand_all_impl(&mut self) {
        let selected_path = self
            .selection_paths
            .collapsed_selected
            .take()
            .or_else(|| self.selected_project().map(|project| project.path.clone()));
        self.selection_paths.collapsed_anchor = None;
        for (node_index, node) in self.nodes.iter().enumerate() {
            if node.has_children() {
                self.expanded.insert(ExpandKey::Node(node_index));
            }
            for (group_index, group) in node.groups.iter().enumerate() {
                if !group.name.is_empty() {
                    self.expanded
                        .insert(ExpandKey::Group(node_index, group_index));
                }
            }
            for (worktree_index, worktree) in node.worktrees.iter().enumerate() {
                if worktree.has_children() {
                    self.expanded
                        .insert(ExpandKey::Worktree(node_index, worktree_index));
                }
                for (group_index, group) in worktree.groups.iter().enumerate() {
                    if !group.name.is_empty() {
                        self.expanded.insert(ExpandKey::WorktreeGroup(
                            node_index,
                            worktree_index,
                            group_index,
                        ));
                    }
                }
            }
        }
        self.dirty.rows.mark_dirty();
        if let Some(path) = selected_path {
            self.select_project_in_tree(&path);
        }
    }

    pub(super) fn collapse_all_impl(&mut self) {
        let selected_path = self.selected_project().map(|project| project.path.clone());
        let anchor = self.selected_row().map(Self::collapse_anchor_row);
        self.expanded.clear();
        self.dirty.rows.mark_dirty();
        self.ensure_visible_rows_cached();
        if let Some(anchor) = anchor
            && let Some(pos) = self.visible_rows().iter().position(|row| *row == anchor)
        {
            self.list_state.select(Some(pos));
        }
        let anchor_path = self.selected_project().map(|project| project.path.clone());
        if selected_path == anchor_path {
            self.selection_paths.collapsed_selected = None;
            self.selection_paths.collapsed_anchor = None;
        } else {
            self.selection_paths.collapsed_selected = selected_path;
            self.selection_paths.collapsed_anchor = anchor_path;
        }
    }

    pub(super) fn scan_log_scroll_up_impl(&mut self) {
        if self.scan_log.is_empty() {
            return;
        }
        let current = self.scan_log_state.selected().unwrap_or(0);
        if current > 0 {
            self.scan_log_state.select(Some(current - 1));
        }
    }

    pub(super) fn scan_log_scroll_down_impl(&mut self) {
        if self.scan_log.is_empty() {
            return;
        }
        let current = self.scan_log_state.selected().unwrap_or(0);
        if current < self.scan_log.len() - 1 {
            self.scan_log_state.select(Some(current + 1));
        }
    }

    pub(super) const fn scan_log_to_top_impl(&mut self) {
        if !self.scan_log.is_empty() {
            self.scan_log_state.select(Some(0));
        }
    }

    pub(super) const fn scan_log_to_bottom_impl(&mut self) {
        if !self.scan_log.is_empty() {
            self.scan_log_state
                .select(Some(self.scan_log.len().saturating_sub(1)));
        }
    }

    pub(super) fn cancel_search_impl(&mut self) {
        self.end_search();
        self.search_query.clear();
        self.filtered.clear();
        self.dirty.rows.mark_dirty();
        self.close_overlay();
        if !self.nodes.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    pub(super) fn confirm_search_impl(&mut self) {
        let project_path = self.selected_project().map(|p| p.path.clone());
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
        for (ni, node) in self.nodes.iter().enumerate() {
            for (gi, group) in node.groups.iter().enumerate() {
                for member in &group.members {
                    if member.path == target_path {
                        self.expanded.insert(ExpandKey::Node(ni));
                        if !group.name.is_empty() {
                            self.expanded.insert(ExpandKey::Group(ni, gi));
                        }
                    }
                }
            }
            for vendored in &node.vendored {
                if vendored.path == target_path {
                    self.expanded.insert(ExpandKey::Node(ni));
                }
            }
            for (wi, wt) in node.worktrees.iter().enumerate() {
                if wt.project.path == target_path {
                    self.expanded.insert(ExpandKey::Node(ni));
                }
                for (gi, group) in wt.groups.iter().enumerate() {
                    for member in &group.members {
                        if member.path == target_path {
                            self.expanded.insert(ExpandKey::Node(ni));
                            self.expanded.insert(ExpandKey::Worktree(ni, wi));
                            if !group.name.is_empty() {
                                self.expanded.insert(ExpandKey::WorktreeGroup(ni, wi, gi));
                            }
                        }
                    }
                }
                for vendored in &wt.vendored {
                    if vendored.path == target_path {
                        self.expanded.insert(ExpandKey::Node(ni));
                        self.expanded.insert(ExpandKey::Worktree(ni, wi));
                    }
                }
            }
        }
    }

    pub(super) fn row_matches_project_path(&self, row: VisibleRow, target_path: &str) -> bool {
        match row {
            VisibleRow::Root { node_index } => self.nodes[node_index].project.path == target_path,
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                self.nodes[node_index].groups[group_index].members[member_index].path == target_path
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => {
                self.nodes[node_index].worktrees[worktree_index]
                    .project
                    .path
                    == target_path
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                self.nodes[node_index].worktrees[worktree_index].groups[group_index].members
                    [member_index]
                    .path
                    == target_path
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => self.nodes[node_index].vendored[vendored_index].path == target_path,
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                self.nodes[node_index].worktrees[worktree_index].vendored[vendored_index].path
                    == target_path
            },
            VisibleRow::GroupHeader { .. } | VisibleRow::WorktreeGroupHeader { .. } => false,
        }
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

    pub(super) fn select_project_in_tree_impl(&mut self, target_path: &str) {
        self.expand_path_in_tree(target_path);
        self.dirty.rows.mark_dirty();
        self.select_matching_visible_row(target_path);
    }

    pub(super) fn update_search_impl(&mut self, query: &str) {
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
            .flat_entries
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
