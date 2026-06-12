//! Restart-stable identity for tree expansion state.
//!
//! [`ExpandKey`] is positional — it indexes into the live tree — so it cannot
//! survive a restart or a rebuild that re-orders projects. [`ExpandTarget`]
//! projects each expandable container onto a path-based identity that does: a
//! root by its path, a named group by its owner path plus group name, a
//! worktree checkout by its path, and a named group under a checkout by both.
//! The variant tag keeps a worktree group's primary `Node` distinct from its
//! primary `Worktree` entry — both resolve to the same `primary_path`, so a
//! bare path could not tell them apart.

use super::ProjectList;
use super::visible_rows::ExpandKey;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::project::RustProject;

/// Path-based projection of an [`ExpandKey`] — the form persisted to disk and
/// re-resolved against the tree on the next launch.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ExpandTarget {
    /// A top-level root node (workspace, package, worktree group, or non-Rust
    /// repo), identified by the root's path.
    Root(AbsolutePath),
    /// A named member group directly under a top-level workspace, identified by
    /// the owner root's path and the group name.
    Group(AbsolutePath, String),
    /// A single checkout inside a worktree group, identified by the checkout's
    /// path. The primary shares the root's path, so the distinct variant — not
    /// the path — separates it from [`ExpandTarget::Root`].
    Worktree(AbsolutePath),
    /// A named member group under a worktree checkout, identified by the
    /// checkout's path and the group name.
    WorktreeGroup(AbsolutePath, String),
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
    for (ni, entry) in list.iter().enumerate() {
        let item = &entry.root_item;
        if entry.has_children() {
            out.push((ExpandKey::Node(ni), ExpandTarget::Root(item.path().clone())));
        }
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                for (gi, group) in ws.groups().iter().enumerate() {
                    if group.is_named() {
                        out.push((
                            ExpandKey::Group(ni, gi),
                            ExpandTarget::Group(
                                item.path().clone(),
                                group.group_name().to_string(),
                            ),
                        ));
                    }
                }
            },
            RootItem::Worktrees(wtg) if wtg.renders_as_group() => {
                for (wi, wentry) in wtg.iter_entries().enumerate() {
                    let RustProject::Workspace(ws) = wentry else {
                        continue;
                    };
                    if ws.has_members() {
                        out.push((
                            ExpandKey::Worktree(ni, wi),
                            ExpandTarget::Worktree(wentry.path().clone()),
                        ));
                    }
                    for (gi, group) in ws.groups().iter().enumerate() {
                        if group.is_named() {
                            out.push((
                                ExpandKey::WorktreeGroup(ni, wi, gi),
                                ExpandTarget::WorktreeGroup(
                                    wentry.path().clone(),
                                    group.group_name().to_string(),
                                ),
                            ));
                        }
                    }
                }
            },
            // A single live worktree renders like a plain workspace — its named
            // groups are keyed `Group`, not `WorktreeGroup` (see
            // `compute_visible_rows`).
            RootItem::Worktrees(wtg) => {
                if let Some(RustProject::Workspace(ws)) = wtg.single_live() {
                    for (gi, group) in ws.groups().iter().enumerate() {
                        if group.is_named() {
                            out.push((
                                ExpandKey::Group(ni, gi),
                                ExpandTarget::Group(
                                    item.path().clone(),
                                    group.group_name().to_string(),
                                ),
                            ));
                        }
                    }
                }
            },
            RootItem::Rust(RustProject::Package(_)) | RootItem::NonRust(_) => {},
        }
    }
    out
}
