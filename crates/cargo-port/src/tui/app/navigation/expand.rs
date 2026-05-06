use crate::tui::app::App;
use crate::tui::app::ExpandKey;
use crate::tui::app::VisibleRow;

impl App {
    pub(super) fn selected_is_expandable(&self) -> bool {
        let selected = self.project_list.cursor();
        self.visible_rows()
            .get(selected)
            .copied()
            .and_then(|row| self.project_list.expand_key_for_row(row))
            .is_some()
    }

    pub fn expand(&mut self) -> bool {
        if !self.selected_is_expandable() {
            return false;
        }
        let selected = self.project_list.cursor();
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return false;
        };
        let Some(key) = self.project_list.expand_key_for_row(row) else {
            return false;
        };
        self.project_list.expanded_mut().insert(key)
    }

    /// Remove `key` from expanded, recompute rows, and move cursor to `target`.
    pub(super) fn collapse_to(&mut self, key: &ExpandKey, target: VisibleRow) {
        self.project_list.expanded_mut().remove(key);
        self.ensure_visible_rows_cached();
        if let Some(pos) = self.visible_rows().iter().position(|r| *r == target) {
            self.project_list.set_cursor(pos);
        }
    }

    pub fn collapse(&mut self) -> bool {
        let selected = self.project_list.cursor();
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return false;
        };
        let expanded_before = self.project_list.expanded().len();
        let selected_before = self.project_list.cursor();
        self.collapse_row(row);
        self.project_list.expanded().len() != expanded_before
            || self.project_list.cursor() != selected_before
    }

    pub(super) fn collapse_row(&mut self, row: VisibleRow) {
        match row {
            VisibleRow::Root { node_index: ni } => {
                self.project_list.try_collapse(&ExpandKey::Node(ni));
            },
            VisibleRow::GroupHeader {
                node_index: ni,
                group_index: gi,
            } => {
                if !self.project_list.try_collapse(&ExpandKey::Group(ni, gi)) {
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
                if !self.project_list.try_collapse(&ExpandKey::Worktree(ni, wi)) {
                    self.collapse_to(&ExpandKey::Node(ni), VisibleRow::Root { node_index: ni });
                }
            },
            VisibleRow::WorktreeGroupHeader {
                node_index: ni,
                worktree_index: wi,
                group_index: gi,
            } => {
                if !self
                    .project_list
                    .try_collapse(&ExpandKey::WorktreeGroup(ni, wi, gi))
                {
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
}
