use std::collections::HashSet;

use super::ProjectList;
use crate::project::AbsolutePath;
use crate::project::GitStatus;
use crate::project::MemberGroup;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Submodule;
use crate::project::VendoredPackage;
use crate::project::Visibility;
use crate::project::WorktreeGroup;

// ── Visible-rows flattening ──────────────────────────────────────────
//
// The project tree is nested; the renderer wants a flat list. The
// types and walker below produce that flat list, expanding /
// collapsing groups based on user state.

/// User-driven expansion state key. Identifies which of the
/// nested containers (root nodes, named groups, worktree
/// entries, worktree groups) the user has toggled open.
#[derive(Hash, Eq, PartialEq, Clone)]
pub enum ExpandKey {
    Node(usize),
    Group(usize, usize),
    Worktree(usize, usize),
    WorktreeGroup(usize, usize, usize),
}

/// What a visible row represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibleRow {
    /// A top-level project/workspace root.
    Root { node_index: usize },
    /// A group header (e.g., "examples").
    GroupHeader {
        node_index:  usize,
        group_index: usize,
    },
    /// An actual project member.
    Member {
        node_index:   usize,
        group_index:  usize,
        member_index: usize,
    },
    /// A vendored crate nested directly under the root project.
    Vendored {
        node_index:     usize,
        vendored_index: usize,
    },
    /// A worktree entry shown directly under the parent node.
    WorktreeEntry {
        node_index:     usize,
        worktree_index: usize,
    },
    /// A group header inside an expanded worktree entry.
    WorktreeGroupHeader {
        node_index:     usize,
        worktree_index: usize,
        group_index:    usize,
    },
    /// A member inside an expanded worktree entry.
    WorktreeMember {
        node_index:     usize,
        worktree_index: usize,
        group_index:    usize,
        member_index:   usize,
    },
    /// A vendored crate nested under a worktree entry.
    WorktreeVendored {
        node_index:     usize,
        worktree_index: usize,
        vendored_index: usize,
    },
    /// A git submodule nested under the root project.
    Submodule {
        node_index:      usize,
        submodule_index: usize,
    },
}

impl ProjectList {
    /// Flatten the nested project tree into the linear list of
    /// rows the renderer walks. Expansion state controls which
    /// nested containers are walked into; `include_non_rust`
    /// gates whether non-Rust roots are emitted; `Dismissed`
    /// roots are always filtered out.
    pub fn compute_visible_rows(
        &self,
        expanded: &HashSet<ExpandKey>,
        include_non_rust: bool,
    ) -> Vec<VisibleRow> {
        let mut rows = Vec::new();
        for (ni, entry) in self.iter().enumerate() {
            let item = &entry.item;
            if matches!(item.visibility(), Visibility::Dismissed) {
                continue;
            }
            if !include_non_rust && !item.is_rust() {
                continue;
            }
            rows.push(VisibleRow::Root { node_index: ni });
            if !expanded.contains(&ExpandKey::Node(ni)) {
                continue;
            }
            match item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    emit_groups(&mut rows, ni, ws.groups(), expanded);
                    emit_vendored_rows(&mut rows, ni, ws.vendored());
                },
                RootItem::Rust(RustProject::Package(pkg)) => {
                    emit_vendored_rows(&mut rows, ni, pkg.vendored());
                },
                RootItem::NonRust(_) => {},
                RootItem::Worktrees(wtg) => {
                    if wtg.renders_as_group() {
                        emit_worktree_group(&mut rows, ni, wtg, expanded);
                    } else if let Some(entry) = wtg.single_live() {
                        if let RustProject::Workspace(ws) = entry {
                            emit_groups(&mut rows, ni, ws.groups(), expanded);
                        }
                        emit_vendored_rows(&mut rows, ni, entry.rust_info().vendored());
                    }
                },
            }
            emit_submodule_rows(&mut rows, ni, item.submodules());
        }
        rows
    }
}

fn emit_groups(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    groups: &[MemberGroup],
    expanded: &HashSet<ExpandKey>,
) {
    for (gi, group) in groups.iter().enumerate() {
        match group {
            MemberGroup::Inline { members } => {
                for (mi, _) in members.iter().enumerate() {
                    rows.push(VisibleRow::Member {
                        node_index:   ni,
                        group_index:  gi,
                        member_index: mi,
                    });
                }
            },
            MemberGroup::Named { members, .. } => {
                rows.push(VisibleRow::GroupHeader {
                    node_index:  ni,
                    group_index: gi,
                });
                if expanded.contains(&ExpandKey::Group(ni, gi)) {
                    for (mi, _) in members.iter().enumerate() {
                        rows.push(VisibleRow::Member {
                            node_index:   ni,
                            group_index:  gi,
                            member_index: mi,
                        });
                    }
                }
            },
        }
    }
}

fn emit_vendored_rows(rows: &mut Vec<VisibleRow>, ni: usize, vendored: &[VendoredPackage]) {
    for (vi, _) in vendored.iter().enumerate() {
        rows.push(VisibleRow::Vendored {
            node_index:     ni,
            vendored_index: vi,
        });
    }
}

fn emit_submodule_rows(rows: &mut Vec<VisibleRow>, ni: usize, submodules: &[Submodule]) {
    for (si, _) in submodules.iter().enumerate() {
        rows.push(VisibleRow::Submodule {
            node_index:      ni,
            submodule_index: si,
        });
    }
}

fn emit_worktree_group(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    wtg: &WorktreeGroup,
    expanded: &HashSet<ExpandKey>,
) {
    for (wi, entry) in wtg.iter_entries().enumerate() {
        if matches!(entry.visibility(), Visibility::Dismissed) {
            continue;
        }
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
        if let RustProject::Workspace(ws) = entry
            && ws.has_members()
            && expanded.contains(&ExpandKey::Worktree(ni, wi))
        {
            emit_worktree_children(rows, ni, wi, ws.groups(), ws.vendored(), expanded);
        }
    }
}

fn emit_worktree_children(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    wi: usize,
    groups: &[MemberGroup],
    vendored: &[VendoredPackage],
    expanded: &HashSet<ExpandKey>,
) {
    for (gi, group) in groups.iter().enumerate() {
        match group {
            MemberGroup::Inline { members } => {
                for (mi, _) in members.iter().enumerate() {
                    rows.push(VisibleRow::WorktreeMember {
                        node_index:     ni,
                        worktree_index: wi,
                        group_index:    gi,
                        member_index:   mi,
                    });
                }
            },
            MemberGroup::Named { members, .. } => {
                rows.push(VisibleRow::WorktreeGroupHeader {
                    node_index:     ni,
                    worktree_index: wi,
                    group_index:    gi,
                });
                if expanded.contains(&ExpandKey::WorktreeGroup(ni, wi, gi)) {
                    for (mi, _) in members.iter().enumerate() {
                        rows.push(VisibleRow::WorktreeMember {
                            node_index:     ni,
                            worktree_index: wi,
                            group_index:    gi,
                            member_index:   mi,
                        });
                    }
                }
            },
        }
    }

    for (vi, _) in vendored.iter().enumerate() {
        rows.push(VisibleRow::WorktreeVendored {
            node_index:     ni,
            worktree_index: wi,
            vendored_index: vi,
        });
    }
}

/// Return the most severe git path state from an iterator.
/// Severity: `Modified` > `Untracked` > `Clean` > `Ignored`.
pub(super) fn worst_git_status(
    states: impl Iterator<Item = Option<GitStatus>>,
) -> Option<GitStatus> {
    const fn severity(state: GitStatus) -> u8 {
        match state {
            GitStatus::Modified => 4,
            GitStatus::Untracked => 3,
            GitStatus::Clean => 2,
            GitStatus::Ignored => 1,
        }
    }
    states.flatten().max_by_key(|s| severity(*s))
}

/// Snapshot of a top-level expansion captured before a tree rebuild
/// reorders node indices. Used to re-apply the same logical expansions
/// to the new layout.
#[derive(Clone)]
pub struct LegacyRootExpansion {
    pub(super) root_path:      AbsolutePath,
    pub(super) old_node_index: usize,
    pub(super) had_children:   bool,
    pub(super) named_groups:   Vec<usize>,
}

impl VisibleRow {
    /// Anchor row to fall back to when collapsing this row — the parent
    /// row that should receive the cursor after the collapse.
    pub(super) const fn collapse_anchor(self) -> Self {
        match self {
            Self::GroupHeader { node_index, .. }
            | Self::Member { node_index, .. }
            | Self::Vendored { node_index, .. }
            | Self::Submodule { node_index, .. } => Self::Root { node_index },
            Self::Root { .. } | Self::WorktreeEntry { .. } => self,
            Self::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            }
            | Self::WorktreeMember {
                node_index,
                worktree_index,
                ..
            }
            | Self::WorktreeVendored {
                node_index,
                worktree_index,
                ..
            } => Self::WorktreeEntry {
                node_index,
                worktree_index,
            },
        }
    }
}
