use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Deref;
use std::ops::DerefMut;
use std::path::Path;

use crate::lint::LintRuns;
use crate::project::AbsolutePath;
use crate::project::MemberGroup;
use crate::project::NonRustProject;
use crate::project::PackageProject;
use crate::project::ProjectFields;
use crate::project::ProjectInfo;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Visibility;
use crate::project::WorkspaceProject;
use crate::project::WorktreeGroup;
use crate::project::WorktreeHealth;

/// Owning wrapper around the project hierarchy.
///
/// `ProjectList` is the single source of truth for all project data.
/// Mutations go through its methods; derived state is computed from it on
/// demand.
#[derive(Clone, Default)]
pub(crate) struct ProjectList {
    root_items: Vec<RootItem>,
}

pub(crate) struct SearchableItem<'a> {
    pub abs_path:         &'a Path,
    pub display_path:     Cow<'a, str>,
    pub name:             Cow<'a, str>,
    /// Cargo package name when it differs from the visible label. Not displayed,
    /// only used for search matching.
    pub cargo_name:       Option<Cow<'a, str>>,
    pub is_rust:          bool,
    pub visibility:       Visibility,
    pub disk_usage_bytes: Option<u64>,
    pub worktree_health:  WorktreeHealth,
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

    /// Iterate every project-like item that search should match directly from
    /// the hierarchy without maintaining a synchronized flat cache.
    pub(crate) fn visit_searchables(&self, mut f: impl FnMut(SearchableItem<'_>)) {
        for item in &self.root_items {
            match item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    visit_workspace_searchables(ws, &mut f);
                },
                RootItem::Rust(RustProject::Package(pkg)) => {
                    visit_package_searchables(pkg, &mut f);
                },
                RootItem::NonRust(nr) => f(non_rust_searchable(nr)),
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    visit_workspace_searchables(primary, &mut f);
                    for l in linked {
                        let worktree_label = l.worktree_name().map_or_else(
                            || Cow::Owned(l.root_directory_name().into_string()),
                            Cow::Borrowed,
                        );
                        let cargo_name = ws_root_cargo_name(l, worktree_label.as_ref());
                        visit_workspace_searchables_with_root_name(
                            l,
                            worktree_label,
                            cargo_name,
                            &mut f,
                        );
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    visit_package_searchables(primary, &mut f);
                    for l in linked {
                        let worktree_label = l.worktree_name().map_or_else(
                            || Cow::Owned(l.root_directory_name().into_string()),
                            Cow::Borrowed,
                        );
                        let cargo_name = pkg_root_cargo_name(l, worktree_label.as_ref());
                        visit_package_searchables_with_root_name(
                            l,
                            worktree_label,
                            cargo_name,
                            &mut f,
                        );
                    }
                },
            }
        }
    }

    pub(crate) fn find_searchable_by_abs_path(&self, target: &Path) -> Option<SearchableItem<'_>> {
        for item in &self.root_items {
            let found = match item {
                RootItem::Rust(RustProject::Workspace(ws)) => find_workspace_searchable(ws, target),
                RootItem::Rust(RustProject::Package(pkg)) => find_package_searchable(pkg, target),
                RootItem::NonRust(nr) => (nr.path() == target).then(|| non_rust_searchable(nr)),
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => find_workspace_searchable(primary, target).or_else(|| {
                    linked
                        .iter()
                        .find_map(|ws| find_workspace_searchable(ws, target))
                }),
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => find_package_searchable(primary, target).or_else(|| {
                    linked
                        .iter()
                        .find_map(|pkg| find_package_searchable(pkg, target))
                }),
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }

    pub(crate) fn at_path(&self, target: &Path) -> Option<&ProjectInfo> {
        self.root_items.iter().find_map(|item| item.at_path(target))
    }

    pub(crate) fn at_path_mut(&mut self, target: &Path) -> Option<&mut ProjectInfo> {
        self.root_items
            .iter_mut()
            .find_map(|item| item.at_path_mut(target))
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
            let Some(identity) =
                linked_worktree_identity(&self.root_items[index]).map(Path::to_path_buf)
            else {
                index += 1;
                continue;
            };
            let Some(mut target_index) =
                find_matching_worktree_container(&self.root_items, index, identity.as_path())
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
    let existing_identity = item_worktree_identity(existing).map(Path::to_path_buf);

    if let RootItem::Rust(RustProject::Workspace(linked)) = item
        && linked.worktree_name().is_some()
    {
        match existing {
            RootItem::Rust(RustProject::Workspace(primary))
                if linked.worktree_primary_abs_path() == existing_identity.as_deref() =>
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
            }) if linked.worktree_primary_abs_path() == existing_identity.as_deref() => {
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
                if linked.worktree_primary_abs_path() == existing_identity.as_deref() =>
            {
                let primary = primary.clone();
                *existing =
                    RootItem::Worktrees(WorktreeGroup::new_packages(primary, vec![linked.clone()]));
                return true;
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                linked: group_linked,
                ..
            }) if linked.worktree_primary_abs_path() == existing_identity.as_deref() => {
                group_linked.push(linked.clone());
                return true;
            },
            _ => {},
        }
    }

    false
}

fn item_worktree_identity(item: &RootItem) -> Option<&Path> {
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

fn linked_worktree_identity(item: &RootItem) -> Option<&Path> {
    match item {
        RootItem::Rust(p) if p.worktree_name().is_some() => p.worktree_primary_abs_path(),
        _ => None,
    }
}

fn find_matching_worktree_container(
    items: &[RootItem],
    linked_index: usize,
    identity: &Path,
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

fn regroup_workspace(ws: &mut WorkspaceProject, inline_dirs: &[String]) {
    // Collect all members from all existing groups.
    let members: Vec<PackageProject> = ws
        .groups_mut()
        .drain(..)
        .flat_map(MemberGroup::into_members)
        .collect();

    // Re-sort into groups based on subdirectory and inline_dirs.
    let mut group_map: std::collections::HashMap<String, Vec<PackageProject>> =
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

fn non_rust_searchable(project: &NonRustProject) -> SearchableItem<'_> {
    SearchableItem {
        abs_path:         project.path(),
        display_path:     Cow::Owned(project.display_path().into_string()),
        name:             Cow::Owned(project.root_directory_name().into_string()),
        cargo_name:       None,
        is_rust:          false,
        visibility:       project.visibility(),
        disk_usage_bytes: project.disk_usage_bytes(),
        worktree_health:  project.worktree_health(),
    }
}

fn package_searchable<'a>(
    project: &'a PackageProject,
    name: Cow<'a, str>,
    cargo_name: Option<Cow<'a, str>>,
) -> SearchableItem<'a> {
    SearchableItem {
        abs_path: project.path(),
        display_path: Cow::Owned(project.display_path().into_string()),
        name,
        cargo_name,
        is_rust: true,
        visibility: project.visibility(),
        disk_usage_bytes: project.disk_usage_bytes(),
        worktree_health: project.worktree_health(),
    }
}

fn workspace_searchable<'a>(
    project: &'a WorkspaceProject,
    name: Cow<'a, str>,
    cargo_name: Option<Cow<'a, str>>,
) -> SearchableItem<'a> {
    SearchableItem {
        abs_path: project.path(),
        display_path: Cow::Owned(project.display_path().into_string()),
        name,
        cargo_name,
        is_rust: true,
        visibility: project.visibility(),
        disk_usage_bytes: project.disk_usage_bytes(),
        worktree_health: project.worktree_health(),
    }
}

fn vendored_searchable(project: &PackageProject) -> SearchableItem<'_> {
    SearchableItem {
        abs_path:         project.path(),
        display_path:     Cow::Owned(project.display_path().into_string()),
        name:             Cow::Owned(format!("{} (vendored)", project.package_name())),
        cargo_name:       None,
        is_rust:          true,
        visibility:       project.visibility(),
        disk_usage_bytes: project.disk_usage_bytes(),
        worktree_health:  project.worktree_health(),
    }
}

fn visit_package_searchables(pkg: &PackageProject, f: &mut impl FnMut(SearchableItem<'_>)) {
    let root_name = pkg.root_directory_name().into_string();
    let cargo_name = pkg_root_cargo_name(pkg, &root_name);
    visit_package_searchables_with_root_name(pkg, Cow::Owned(root_name), cargo_name, f);
}

fn visit_package_searchables_with_root_name<'a>(
    pkg: &'a PackageProject,
    root_name: Cow<'a, str>,
    cargo_name: Option<Cow<'a, str>>,
    f: &mut impl FnMut(SearchableItem<'a>),
) {
    f(package_searchable(pkg, root_name, cargo_name));
    for vendored in pkg.vendored() {
        f(vendored_searchable(vendored));
    }
}

fn find_package_searchable<'a>(
    pkg: &'a PackageProject,
    target: &Path,
) -> Option<SearchableItem<'a>> {
    if pkg.path() == target {
        let root_name = pkg.root_directory_name().into_string();
        let cargo_name = pkg_root_cargo_name(pkg, &root_name);
        return Some(package_searchable(pkg, Cow::Owned(root_name), cargo_name));
    }
    pkg.vendored()
        .iter()
        .find(|vendored| vendored.path() == target)
        .map(vendored_searchable)
}

fn visit_workspace_searchables(ws: &WorkspaceProject, f: &mut impl FnMut(SearchableItem<'_>)) {
    let root_name = ws.root_directory_name().into_string();
    let cargo_name = ws_root_cargo_name(ws, &root_name);
    visit_workspace_searchables_with_root_name(ws, Cow::Owned(root_name), cargo_name, f);
}

fn visit_workspace_searchables_with_root_name<'a>(
    ws: &'a WorkspaceProject,
    root_name: Cow<'a, str>,
    cargo_name: Option<Cow<'a, str>>,
    f: &mut impl FnMut(SearchableItem<'a>),
) {
    f(workspace_searchable(ws, root_name, cargo_name));
    for group in ws.groups() {
        for member in group.members() {
            f(package_searchable(
                member,
                Cow::Owned(member.package_name().into_string()),
                None,
            ));
            for vendored in member.vendored() {
                f(vendored_searchable(vendored));
            }
        }
    }
    for vendored in ws.vendored() {
        f(vendored_searchable(vendored));
    }
}

fn find_workspace_searchable<'a>(
    ws: &'a WorkspaceProject,
    target: &Path,
) -> Option<SearchableItem<'a>> {
    if ws.path() == target {
        let root_name = ws.root_directory_name().into_string();
        let cargo_name = ws_root_cargo_name(ws, &root_name);
        return Some(workspace_searchable(ws, Cow::Owned(root_name), cargo_name));
    }
    for group in ws.groups() {
        for member in group.members() {
            if member.path() == target {
                return Some(package_searchable(
                    member,
                    Cow::Owned(member.package_name().into_string()),
                    None,
                ));
            }
            if let Some(vendored) = member
                .vendored()
                .iter()
                .find(|vendored| vendored.path() == target)
            {
                return Some(vendored_searchable(vendored));
            }
        }
    }
    ws.vendored()
        .iter()
        .find(|vendored| vendored.path() == target)
        .map(vendored_searchable)
}

/// Return the Cargo package name as a search-only token when it differs from the
/// visible label. Returns `None` when both names match (no alias needed).
fn ws_root_cargo_name<'a>(
    project: &'a WorkspaceProject,
    visible_name: &str,
) -> Option<Cow<'a, str>> {
    let cargo_name = project.package_name();
    if cargo_name.as_str() == visible_name {
        None
    } else {
        Some(Cow::Owned(cargo_name.into_string()))
    }
}

fn pkg_root_cargo_name<'a>(
    project: &'a PackageProject,
    visible_name: &str,
) -> Option<Cow<'a, str>> {
    let cargo_name = project.package_name();
    if cargo_name.as_str() == visible_name {
        None
    } else {
        Some(Cow::Owned(cargo_name.into_string()))
    }
}

fn try_insert_member(ws: &mut WorkspaceProject, item_path: &Path, item: &RootItem) -> bool {
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
