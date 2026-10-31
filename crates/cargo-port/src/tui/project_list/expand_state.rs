//! Restart-stable identity for tree expansion state.
//!
//! [`ExpandKey`] is positional — it indexes into the live tree — so it cannot
//! survive a restart or a rebuild that re-orders projects. [`ExpandTarget`]
//! projects each expandable container onto a path-based identity that does: a
//! root by its path, a named group by its owner path plus group name, a member
//! with vendored children by the member path, a worktree checkout by its path,
//! and a named group or member under a checkout by its checkout-local path.
//! The variant tag keeps a worktree group's primary `Node` distinct from its
//! primary `Worktree` entry — both resolve to the same `primary_path`, so a
//! bare path could not tell them apart.

use super::visible_rows::ExpandKey;
use crate::project::AbsolutePath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::tui::project_list_state::ProjectList;

/// Path-based projection of an [`ExpandKey`] — the form persisted to disk and
/// re-resolved against the tree on the next launch.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::tui) enum ExpandTarget {
    /// A top-level root node (workspace, package, worktree group, or non-Rust
    /// repo), identified by the root's path.
    Root(AbsolutePath),
    /// A named member group directly under a top-level workspace, identified by
    /// the owner root's path and the group name.
    Group(AbsolutePath, String),
    /// A workspace member with vendored children, identified by the member path.
    Member(AbsolutePath),
    /// A single checkout inside a worktree group, identified by the checkout's
    /// path. The primary shares the root's path, so the distinct variant — not
    /// the path — separates it from [`ExpandTarget::Root`].
    Worktree(AbsolutePath),
    /// A named member group under a worktree checkout, identified by the
    /// checkout's path and the group name.
    WorktreeGroup(AbsolutePath, String),
    /// A worktree workspace member with vendored children, identified by the
    /// member path.
    WorktreeMember(AbsolutePath),
}

/// Every expandable container in the current tree paired with its
/// restart-stable identity. One pass serves both directions:
/// [`ProjectList::export_expanded`] projects the live `expanded` keys to
/// targets for saving, and [`ProjectList::apply_expanded`] resolves saved
/// targets back to keys for restoring. The gates mirror
/// [`ProjectList::expand_key_for_row`], so the keys produced here are exactly
/// the ones the expansion set can hold.
pub(super) fn collect_expandable_targets(list: &ProjectList) -> Vec<(ExpandKey, ExpandTarget)> {
    let mut out = Vec::new();
    for (node_index, entry) in list.iter().enumerate() {
        let item = &entry.root_item;
        if entry.has_children() {
            out.push((
                ExpandKey::Node(node_index),
                ExpandTarget::Root(item.path().clone()),
            ));
        }
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                for (gi, member_group) in ws.groups().iter().enumerate() {
                    if member_group.is_named() {
                        out.push((
                            ExpandKey::Group(node_index, gi),
                            ExpandTarget::Group(
                                item.path().clone(),
                                member_group.group_name().to_string(),
                            ),
                        ));
                    }
                    for (mi, package) in member_group.members().iter().enumerate() {
                        if !package.vendored().is_empty() {
                            out.push((
                                ExpandKey::Member(node_index, gi, mi),
                                ExpandTarget::Member(package.path().clone()),
                            ));
                        }
                    }
                }
            },
            RootItem::Worktrees(worktree_group) if worktree_group.renders_as_group() => {
                for (wi, rust_project) in worktree_group.iter_entries().enumerate() {
                    if worktree_entry_has_children(rust_project) {
                        out.push((
                            ExpandKey::Worktree(node_index, wi),
                            ExpandTarget::Worktree(rust_project.path().clone()),
                        ));
                    }
                    if let RustProject::Workspace(ws) = rust_project {
                        for (gi, member_group) in ws.groups().iter().enumerate() {
                            if member_group.is_named() {
                                out.push((
                                    ExpandKey::WorktreeGroup(node_index, wi, gi),
                                    ExpandTarget::WorktreeGroup(
                                        rust_project.path().clone(),
                                        member_group.group_name().to_string(),
                                    ),
                                ));
                            }
                            for (mi, package) in member_group.members().iter().enumerate() {
                                if !package.vendored().is_empty() {
                                    out.push((
                                        ExpandKey::WorktreeMember(node_index, wi, gi, mi),
                                        ExpandTarget::WorktreeMember(package.path().clone()),
                                    ));
                                }
                            }
                        }
                    }
                }
            },
            // `worktree_group.single_live()` renders as a plain workspace, so
            // `compute_visible_rows` keys its named groups with `ExpandKey::Group`, not
            // `ExpandKey::WorktreeGroup`.
            RootItem::Worktrees(worktree_group) => {
                if let Some(RustProject::Workspace(ws)) = worktree_group.single_live() {
                    for (gi, member_group) in ws.groups().iter().enumerate() {
                        if member_group.is_named() {
                            out.push((
                                ExpandKey::Group(node_index, gi),
                                ExpandTarget::Group(
                                    item.path().clone(),
                                    member_group.group_name().to_string(),
                                ),
                            ));
                        }
                        for (mi, package) in member_group.members().iter().enumerate() {
                            if !package.vendored().is_empty() {
                                out.push((
                                    ExpandKey::Member(node_index, gi, mi),
                                    ExpandTarget::Member(package.path().clone()),
                                ));
                            }
                        }
                    }
                }
            },
            RootItem::Rust(RustProject::Package(_)) | RootItem::NonRust(_) => {},
        }
    }
    out
}

fn worktree_entry_has_children(entry: &RustProject) -> bool {
    match entry {
        RustProject::Workspace(ws) => ws.has_members() || !ws.vendored().is_empty(),
        RustProject::Package(pkg) => !pkg.vendored().is_empty(),
    }
}
