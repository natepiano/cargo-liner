use std::path::Path;

use crate::project::AbsolutePath;
use crate::project::DisplayPath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorktreeGroup;
use crate::tui::app::App;

impl App {
    /// Check if a group at the given indices is an inline (unnamed) group.
    pub(super) fn is_inline_group(&self, ni: usize, gi: usize) -> bool {
        let Some(item) = self.projects().get(ni) else {
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
        let Some(item) = self.projects().get(ni) else {
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

    pub(super) fn worktree_display_path(item: &RootItem, wi: usize) -> Option<DisplayPath> {
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

    pub(super) fn worktree_member_display_path(
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

    pub(super) fn worktree_vendored_display_path(
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

    pub(super) fn worktree_abs_path(item: &RootItem, wi: usize) -> Option<AbsolutePath> {
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

    pub(super) fn worktree_member_abs_path(
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

    pub(super) fn worktree_vendored_abs_path(
        item: &RootItem,
        wi: usize,
        vi: usize,
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

    pub(super) fn worktree_path_ref(item: &RootItem, wi: usize) -> Option<&Path> {
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

    pub(super) fn worktree_member_path_ref(
        item: &RootItem,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<&Path> {
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

    pub(super) fn worktree_vendored_path_ref(
        item: &RootItem,
        wi: usize,
        vi: usize,
    ) -> Option<&Path> {
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
}
