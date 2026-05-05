use std::path::Path;

use crate::project::AbsolutePath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorktreeGroup;
use crate::tui::app::App;
use crate::tui::app::ExpandKey;
use crate::tui::app::VisibleRow;

impl App {
    pub fn expand_all(&mut self) {
        let selected_path = self
            .selection
            .paths_mut()
            .collapsed_selected
            .take()
            .or_else(|| self.selected_project_path().map(AbsolutePath::from));
        self.selection.paths_mut().collapsed_anchor = None;
        let Self {
            scan, selection, ..
        } = self;
        for (ni, entry) in scan.projects().iter().enumerate() {
            if entry.item.has_children() {
                selection.expanded_mut().insert(ExpandKey::Node(ni));
            }
            match &entry.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    for (gi, group) in ws.groups().iter().enumerate() {
                        if group.is_named() {
                            selection.expanded_mut().insert(ExpandKey::Group(ni, gi));
                        }
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    for (wi, ws) in std::iter::once(primary).chain(linked.iter()).enumerate() {
                        if ws.has_members() {
                            selection.expanded_mut().insert(ExpandKey::Worktree(ni, wi));
                        }
                        for (gi, group) in ws.groups().iter().enumerate() {
                            if group.is_named() {
                                selection
                                    .expanded_mut()
                                    .insert(ExpandKey::WorktreeGroup(ni, wi, gi));
                            }
                        }
                    }
                },
                _ => {},
            }
        }
        if let Some(path) = selected_path {
            self.select_project_in_tree(path.as_path());
        }
    }

    pub fn collapse_all(&mut self) {
        let selected_path = self.selected_project_path().map(AbsolutePath::from);
        let anchor = self.selected_row().map(Self::collapse_anchor_row);
        self.selection.expanded_mut().clear();
        self.ensure_visible_rows_cached();
        if let Some(anchor) = anchor
            && let Some(pos) = self.visible_rows().iter().position(|row| *row == anchor)
        {
            self.selection.set_cursor(pos);
        }
        let anchor_path = self.selected_project_path().map(AbsolutePath::from);
        if selected_path == anchor_path {
            self.selection.paths_mut().collapsed_selected = None;
            self.selection.paths_mut().collapsed_anchor = None;
        } else {
            self.selection.paths_mut().collapsed_selected = selected_path;
            self.selection.paths_mut().collapsed_anchor = anchor_path;
        }
    }

    pub(super) fn expand_path_in_tree(&mut self, target_path: &Path) {
        let Self {
            scan, selection, ..
        } = self;
        for (ni, entry) in scan.projects().iter().enumerate() {
            match &entry.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    for (gi, group) in ws.groups().iter().enumerate() {
                        for member in group.members() {
                            if member.path() == target_path {
                                selection.expanded_mut().insert(ExpandKey::Node(ni));
                                if group.is_named() {
                                    selection.expanded_mut().insert(ExpandKey::Group(ni, gi));
                                }
                            }
                        }
                    }
                    for vendored in ws.vendored() {
                        if vendored.path() == target_path {
                            selection.expanded_mut().insert(ExpandKey::Node(ni));
                        }
                    }
                },
                RootItem::Rust(RustProject::Package(pkg)) => {
                    for vendored in pkg.vendored() {
                        if vendored.path() == target_path {
                            selection.expanded_mut().insert(ExpandKey::Node(ni));
                        }
                    }
                },
                RootItem::NonRust(_) => {},
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    for (wi, ws) in std::iter::once(primary).chain(linked.iter()).enumerate() {
                        if ws.path() == target_path {
                            selection.expanded_mut().insert(ExpandKey::Node(ni));
                        }
                        for (gi, group) in ws.groups().iter().enumerate() {
                            for member in group.members() {
                                if member.path() == target_path {
                                    selection.expanded_mut().insert(ExpandKey::Node(ni));
                                    selection.expanded_mut().insert(ExpandKey::Worktree(ni, wi));
                                    if group.is_named() {
                                        selection
                                            .expanded_mut()
                                            .insert(ExpandKey::WorktreeGroup(ni, wi, gi));
                                    }
                                }
                            }
                        }
                        for vendored in ws.vendored() {
                            if vendored.path() == target_path {
                                selection.expanded_mut().insert(ExpandKey::Node(ni));
                                selection.expanded_mut().insert(ExpandKey::Worktree(ni, wi));
                            }
                        }
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    for (wi, pkg) in std::iter::once(primary).chain(linked.iter()).enumerate() {
                        if pkg.path() == target_path {
                            selection.expanded_mut().insert(ExpandKey::Node(ni));
                        }
                        for vendored in pkg.vendored() {
                            if vendored.path() == target_path {
                                selection.expanded_mut().insert(ExpandKey::Node(ni));
                                selection.expanded_mut().insert(ExpandKey::Worktree(ni, wi));
                            }
                        }
                    }
                },
            }
        }
    }

    pub(super) fn row_matches_project_path(&self, row: VisibleRow, target_path: &Path) -> bool {
        self.path_for_row(row)
            .is_some_and(|path| path == target_path)
    }

    pub(super) fn select_matching_visible_row(&mut self, target_path: &Path) {
        self.ensure_visible_rows_cached();
        let selected_index = self
            .visible_rows()
            .iter()
            .position(|row| self.row_matches_project_path(*row, target_path));
        if let Some(selected_index) = selected_index {
            self.selection.set_cursor(selected_index);
        }
    }

    pub fn select_project_in_tree(&mut self, target_path: &Path) {
        self.expand_path_in_tree(target_path);
        self.select_matching_visible_row(target_path);
    }
}
