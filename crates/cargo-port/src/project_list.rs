use std::collections::HashMap;
use std::ops::Deref;
use std::ops::DerefMut;
use std::path::Path;

use crate::lint::LintRuns;
use crate::project::AbsolutePath;
use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::ProjectFields;
use crate::project::ProjectInfo;
use crate::project::RootItem;
use crate::project::RustInfo;
use crate::project::RustProject;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorktreeGroup;

/// Owning wrapper around the project hierarchy.
///
/// `ProjectList` is the single source of truth for all project data.
/// Mutations go through its methods; derived state is computed from it on
/// demand.
#[derive(Clone, Default)]
pub(crate) struct ProjectList {
    root_items: Vec<RootItem>,
}

impl ProjectList {
    pub(crate) const fn new(items: Vec<RootItem>) -> Self { Self { root_items: items } }

    pub(crate) fn resolved_root_labels(&self, include_non_rust: bool) -> Vec<String> {
        let mut labels: Vec<String> = self
            .root_items
            .iter()
            .map(|item| item.root_directory_name().into_string())
            .collect();
        let mut collision_sets: HashMap<String, Vec<usize>> = HashMap::new();

        for (index, item) in self.root_items.iter().enumerate() {
            if matches!(item.visibility(), Visibility::Dismissed) {
                continue;
            }
            if !include_non_rust && !item.is_rust() {
                continue;
            }
            collision_sets
                .entry(item.root_directory_name().into_string())
                .or_default()
                .push(index);
        }

        for indices in collision_sets
            .into_values()
            .filter(|indices| indices.len() > 1)
        {
            let suffixes = shortest_unique_suffixes(
                &indices
                    .iter()
                    .map(|&index| self.root_items[index].display_path().into_string())
                    .collect::<Vec<_>>(),
            );
            for (index, suffix) in indices.into_iter().zip(suffixes) {
                labels[index] = format!("{} [{suffix}]", labels[index]);
            }
        }

        for (label, item) in labels.iter_mut().zip(&self.root_items) {
            if let Some(suffix) = item.worktree_badge_suffix() {
                label.push_str(&suffix);
            }
        }

        labels
    }

    pub(crate) fn git_directories(&self) -> Vec<AbsolutePath> {
        self.root_items
            .iter()
            .filter_map(RootItem::git_directory)
            .collect()
    }

    // -- Leaf iteration ---------------------------------------------------

    /// Iterate all leaf-level projects from the hierarchy.
    ///
    /// For `Rust`, `NonRust`: yields the item directly.
    /// For worktree groups: yields primary and each linked entry wrapped as
    /// `Rust(Workspace(..))` or `Rust(Package(..))`.
    pub(crate) fn for_each_leaf(&self, mut f: impl FnMut(&RootItem)) {
        for item in &self.root_items {
            match item {
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    f(&RootItem::Rust(RustProject::Workspace(primary.clone())));
                    for l in linked {
                        f(&RootItem::Rust(RustProject::Workspace(l.clone())));
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    f(&RootItem::Rust(RustProject::Package(primary.clone())));
                    for l in linked {
                        f(&RootItem::Rust(RustProject::Package(l.clone())));
                    }
                },
                other => f(other),
            }
        }
    }

    /// Zero-allocation leaf path iteration. Yields `(path, is_rust)` for
    /// every leaf project without cloning any `RootItem`.
    pub(crate) fn for_each_leaf_path(&self, mut f: impl FnMut(&Path, bool)) {
        for item in &self.root_items {
            match item {
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    for ws in std::iter::once(primary).chain(linked) {
                        f(ws.path(), true);
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    for pkg in std::iter::once(primary).chain(linked) {
                        f(pkg.path(), true);
                    }
                },
                other => f(other.path(), other.is_rust()),
            }
        }
    }

    pub(crate) fn at_path(&self, target: &Path) -> Option<&ProjectInfo> {
        self.root_items.iter().find_map(|item| item.at_path(target))
    }

    pub(crate) fn at_path_mut(&mut self, target: &Path) -> Option<&mut ProjectInfo> {
        self.root_items
            .iter_mut()
            .find_map(|item| item.at_path_mut(target))
    }

    /// Whether `target` is the path of a submodule under any root item.
    /// CI fetches and GitHub repo metadata for submodules belong to the
    /// upstream repository and are suppressed at the parent project's
    /// level — see the `BackgroundMsg::GitInfo` handler.
    pub(crate) fn is_submodule_path(&self, target: &Path) -> bool {
        self.root_items
            .iter()
            .any(|item| item.submodules().iter().any(|s| s.path.as_path() == target))
    }

    pub(crate) fn rust_info_at_path(&self, target: &Path) -> Option<&RustInfo> {
        self.root_items
            .iter()
            .find_map(|item| item.rust_info_at_path(target))
    }

    pub(crate) fn rust_info_at_path_mut(&mut self, target: &Path) -> Option<&mut RustInfo> {
        self.root_items
            .iter_mut()
            .find_map(|item| item.rust_info_at_path_mut(target))
    }

    pub(crate) fn lint_at_path(&self, target: &Path) -> Option<&LintRuns> {
        self.root_items
            .iter()
            .find_map(|item| item.lint_at_path(target))
    }

    pub(crate) fn lint_at_path_mut(&mut self, target: &Path) -> Option<&mut LintRuns> {
        self.root_items
            .iter_mut()
            .find_map(|item| item.lint_at_path_mut(target))
    }

    // -- Hierarchy mutations ----------------------------------------------

    /// Find a leaf item by absolute path and replace it, returning the old
    /// item. Descends into worktree groups to find matching entries.
    pub(crate) fn replace_leaf_by_path(
        &mut self,
        path: &Path,
        mut replacement: RootItem,
    ) -> Option<RootItem> {
        for item in &mut self.root_items {
            match item {
                RootItem::Rust(_) | RootItem::NonRust(_) => {
                    if item.path() == path {
                        std::mem::swap(item, &mut replacement);
                        return Some(replacement);
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    if primary.path() == path
                        && let RootItem::Rust(RustProject::Workspace(ws)) = replacement
                    {
                        let old = primary.clone();
                        *primary = ws;
                        return Some(RootItem::Rust(RustProject::Workspace(old)));
                    }
                    for l in linked {
                        if l.path() == path
                            && let RootItem::Rust(RustProject::Workspace(ws)) = replacement
                        {
                            let old = l.clone();
                            *l = ws;
                            return Some(RootItem::Rust(RustProject::Workspace(old)));
                        }
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    if primary.path() == path
                        && let RootItem::Rust(RustProject::Package(pkg)) = replacement
                    {
                        let old = primary.clone();
                        *primary = pkg;
                        return Some(RootItem::Rust(RustProject::Package(old)));
                    }
                    for l in linked {
                        if l.path() == path
                            && let RootItem::Rust(RustProject::Package(pkg)) = replacement
                        {
                            let old = l.clone();
                            *l = pkg;
                            return Some(RootItem::Rust(RustProject::Package(old)));
                        }
                    }
                },
            }
        }
        None
    }

    /// Insert a discovered item into the hierarchy. If the item is a package
    /// whose path falls inside an existing workspace, it is added as an
    /// inline member of that workspace. Otherwise it is pushed as a
    /// top-level peer.
    ///
    /// Returns `true` if the item was inserted into an existing workspace.
    pub(crate) fn insert_into_hierarchy(&mut self, item: RootItem) -> bool {
        let item_path = item.path().to_path_buf();
        for existing in &mut self.root_items {
            if try_attach_worktree(existing, &item) {
                return false;
            }

            let inserted = match existing {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    try_insert_member(ws, &item_path, &item)
                },
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    try_insert_member(primary, &item_path, &item)
                        || linked
                            .iter_mut()
                            .any(|ws| try_insert_member(ws, &item_path, &item))
                },
                _ => false,
            };
            if inserted {
                return true;
            }
        }
        // No parent workspace found — add as top-level peer.
        let insert_index = self
            .root_items
            .binary_search_by(|existing| existing.path().cmp(item_path.as_path()))
            .unwrap_or_else(|index| index);
        self.root_items.insert(insert_index, item);
        false
    }

    // -- Config-driven regrouping -------------------------------------------

    /// Regroup workspace members based on `inline_dirs` config. Walks all
    /// workspaces (including inside worktree groups) and re-sorts their
    /// members into `Named` / `Inline` groups.
    pub(crate) fn regroup_members(&mut self, inline_dirs: &[String]) {
        for item in &mut self.root_items {
            match item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    regroup_workspace(ws, inline_dirs);
                },
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    regroup_workspace(primary, inline_dirs);
                    for l in linked {
                        regroup_workspace(l, inline_dirs);
                    }
                },
                _ => {},
            }
        }
    }

    pub(crate) fn regroup_top_level_worktrees(&mut self) {
        let mut index = 0;
        while index < self.root_items.len() {
            let Some(identity) = linked_worktree_identity(&self.root_items[index]).cloned() else {
                index += 1;
                continue;
            };
            let Some(mut target_index) =
                find_matching_worktree_container(&self.root_items, index, &identity)
            else {
                index += 1;
                continue;
            };

            let linked_item = self.root_items.remove(index);
            if target_index > index {
                target_index -= 1;
            }
            let attached = try_attach_worktree(&mut self.root_items[target_index], &linked_item);
            debug_assert!(
                attached,
                "linked worktree regroup should attach after container match"
            );
            if target_index >= index {
                index += 1;
            }
        }
    }

    // -- Vec-like operations -------------------------------------------------

    pub(crate) fn clear(&mut self) { self.root_items.clear(); }

    #[cfg(test)]
    pub(crate) fn push(&mut self, item: RootItem) { self.root_items.push(item); }
}

fn shortest_unique_suffixes(paths: &[String]) -> Vec<String> {
    let segments: Vec<Vec<&str>> = paths
        .iter()
        .map(|path| display_path_segments(path))
        .collect();
    let mut suffix_len = vec![1usize; paths.len()];

    loop {
        let mut collisions: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, path_segments) in segments.iter().enumerate() {
            collisions
                .entry(join_suffix(path_segments, suffix_len[index]))
                .or_default()
                .push(index);
        }

        let mut changed = false;
        for indices in collisions.into_values().filter(|indices| indices.len() > 1) {
            for index in indices {
                if suffix_len[index] < segments[index].len() {
                    suffix_len[index] += 1;
                    changed = true;
                }
            }
        }

        if !changed {
            break;
        }
    }

    segments
        .iter()
        .enumerate()
        .map(|(index, path_segments)| join_suffix(path_segments, suffix_len[index]))
        .collect()
}

fn display_path_segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn join_suffix(segments: &[&str], suffix_len: usize) -> String {
    let len = suffix_len.min(segments.len());
    segments[segments.len().saturating_sub(len)..].join("/")
}

fn try_attach_worktree(existing: &mut RootItem, item: &RootItem) -> bool {
    let existing_identity = item_worktree_identity(existing).cloned();

    if let RootItem::Rust(RustProject::Workspace(linked)) = item
        && linked.worktree_name().is_some()
    {
        match existing {
            RootItem::Rust(RustProject::Workspace(primary))
                if linked.worktree_primary_abs_path() == existing_identity.as_ref() =>
            {
                let primary = primary.clone();
                *existing = RootItem::Worktrees(WorktreeGroup::new_workspaces(
                    primary,
                    vec![linked.clone()],
                ));
                return true;
            },
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                linked: group_linked,
                ..
            }) if linked.worktree_primary_abs_path() == existing_identity.as_ref() => {
                group_linked.push(linked.clone());
                return true;
            },
            _ => {},
        }
    }

    if let RootItem::Rust(RustProject::Package(linked)) = item
        && linked.worktree_name().is_some()
    {
        match existing {
            RootItem::Rust(RustProject::Package(primary))
                if linked.worktree_primary_abs_path() == existing_identity.as_ref() =>
            {
                let primary = primary.clone();
                *existing =
                    RootItem::Worktrees(WorktreeGroup::new_packages(primary, vec![linked.clone()]));
                return true;
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                linked: group_linked,
                ..
            }) if linked.worktree_primary_abs_path() == existing_identity.as_ref() => {
                group_linked.push(linked.clone());
                return true;
            },
            _ => {},
        }
    }

    false
}

fn item_worktree_identity(item: &RootItem) -> Option<&AbsolutePath> {
    match item {
        RootItem::Rust(p) => p.worktree_primary_abs_path(),
        RootItem::Worktrees(WorktreeGroup::Workspaces { primary, .. }) => {
            primary.worktree_primary_abs_path()
        },
        RootItem::Worktrees(WorktreeGroup::Packages { primary, .. }) => {
            primary.worktree_primary_abs_path()
        },
        RootItem::NonRust(_) => None,
    }
}

fn linked_worktree_identity(item: &RootItem) -> Option<&AbsolutePath> {
    match item {
        RootItem::Rust(p) if p.worktree_name().is_some() => p.worktree_primary_abs_path(),
        _ => None,
    }
}

fn find_matching_worktree_container(
    items: &[RootItem],
    linked_index: usize,
    identity: &AbsolutePath,
) -> Option<usize> {
    items.iter().enumerate().find_map(|(index, item)| {
        if index == linked_index {
            return None;
        }
        (item_worktree_identity(item) == Some(identity)).then_some(index)
    })
}

// -- Deref to slice for read access ---------------------------------------

impl Deref for ProjectList {
    type Target = [RootItem];

    fn deref(&self) -> &[RootItem] { &self.root_items }
}

impl DerefMut for ProjectList {
    fn deref_mut(&mut self) -> &mut [RootItem] { &mut self.root_items }
}

// -- IntoIterator for `for item in &projects` / `for item in &mut projects`

impl<'a> IntoIterator for &'a ProjectList {
    type IntoIter = std::slice::Iter<'a, RootItem>;
    type Item = &'a RootItem;

    fn into_iter(self) -> Self::IntoIter { self.root_items.iter() }
}

impl<'a> IntoIterator for &'a mut ProjectList {
    type IntoIter = std::slice::IterMut<'a, RootItem>;
    type Item = &'a mut RootItem;

    fn into_iter(self) -> Self::IntoIter { self.root_items.iter_mut() }
}

// -- Helpers --------------------------------------------------------------

fn regroup_workspace(ws: &mut Workspace, inline_dirs: &[String]) {
    // Collect all members from all existing groups.
    let members: Vec<Package> = ws
        .groups_mut()
        .drain(..)
        .flat_map(MemberGroup::into_members)
        .collect();

    // Re-sort into groups based on subdirectory and inline_dirs.
    let mut group_map: std::collections::HashMap<String, Vec<Package>> =
        std::collections::HashMap::new();
    for member in members {
        let relative = member
            .path()
            .strip_prefix(ws.path())
            .ok()
            .map(crate::scan::normalize_workspace_path)
            .unwrap_or_default();
        let subdir = relative.split('/').next().unwrap_or("").to_string();
        let group_name = if inline_dirs.contains(&subdir) || !relative.contains('/') {
            String::new()
        } else {
            subdir
        };
        group_map.entry(group_name).or_default().push(member);
    }

    let mut groups: Vec<MemberGroup> = group_map
        .into_iter()
        .map(|(name, members)| {
            if name.is_empty() {
                MemberGroup::Inline { members }
            } else {
                MemberGroup::Named { name, members }
            }
        })
        .collect();
    groups.sort_by(|a, b| {
        let a_inline = a.group_name().is_empty();
        let b_inline = b.group_name().is_empty();
        match (a_inline, b_inline) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => a.group_name().cmp(b.group_name()),
        }
    });

    *ws.groups_mut() = groups;
}

fn try_insert_member(ws: &mut Workspace, item_path: &Path, item: &RootItem) -> bool {
    if !item_path.starts_with(ws.path()) || item_path == ws.path() {
        return false;
    }
    let RootItem::Rust(RustProject::Package(pkg)) = item else {
        return false;
    };
    // Add to the first inline group, or create one.
    let inline = ws.groups_mut().iter_mut().find(|g| g.is_inline());
    if let Some(group) = inline {
        group.members_mut().push(pkg.clone());
        group
            .members_mut()
            .sort_by(|a, b| a.package_name().as_str().cmp(b.package_name().as_str()));
    } else {
        ws.groups_mut().push(MemberGroup::Inline {
            members: vec![pkg.clone()],
        });
    }
    true
}
