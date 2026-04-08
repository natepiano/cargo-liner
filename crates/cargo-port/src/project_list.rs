use std::borrow::Cow;
use std::ops::Deref;
use std::ops::DerefMut;
use std::path::Path;

use crate::project::MemberGroup;
use crate::project::NonRustProject;
use crate::project::Package;
use crate::project::ProjectInfo;
use crate::project::ProjectListItem;
use crate::project::RustProject;
use crate::project::Visibility;
use crate::project::Workspace;

/// Owning wrapper around the project hierarchy.
///
/// `ProjectList` is the single source of truth for all project data.
/// Mutations go through its methods; derived state is computed from it on
/// demand.
#[derive(Clone, Default)]
pub(crate) struct ProjectList {
    items: Vec<ProjectListItem>,
}

pub(crate) struct SearchableItem<'a> {
    pub abs_path:         &'a Path,
    pub display_path:     Cow<'a, str>,
    pub name:             Cow<'a, str>,
    pub is_rust:          bool,
    pub visibility:       Visibility,
    pub disk_usage_bytes: Option<u64>,
}

impl ProjectList {
    pub(crate) const fn new(items: Vec<ProjectListItem>) -> Self { Self { items } }

    // -- Leaf iteration ---------------------------------------------------

    /// Iterate all leaf-level projects from the hierarchy.
    ///
    /// For `Workspace`, `Package`, `NonRust`: yields the item directly.
    /// For worktree groups: yields primary and each linked entry wrapped as
    /// `Workspace` or `Package`.
    pub(crate) fn for_each_leaf(&self, mut f: impl FnMut(&ProjectListItem)) {
        for item in &self.items {
            match item {
                ProjectListItem::WorkspaceWorktrees(g) => {
                    f(&ProjectListItem::Workspace(g.primary().clone()));
                    for linked in g.linked() {
                        f(&ProjectListItem::Workspace(linked.clone()));
                    }
                },
                ProjectListItem::PackageWorktrees(g) => {
                    f(&ProjectListItem::Package(g.primary().clone()));
                    for linked in g.linked() {
                        f(&ProjectListItem::Package(linked.clone()));
                    }
                },
                other => f(other),
            }
        }
    }

    /// Zero-allocation leaf path iteration. Yields `(path, is_rust)` for
    /// every leaf project without cloning any `ProjectListItem`.
    pub(crate) fn for_each_leaf_path(&self, mut f: impl FnMut(&Path, bool)) {
        for item in &self.items {
            match item {
                ProjectListItem::WorkspaceWorktrees(g) => {
                    for ws in std::iter::once(g.primary()).chain(g.linked()) {
                        f(ws.path(), true);
                    }
                },
                ProjectListItem::PackageWorktrees(g) => {
                    for pkg in std::iter::once(g.primary()).chain(g.linked()) {
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
        for item in &self.items {
            match item {
                ProjectListItem::Workspace(ws) => visit_workspace_searchables(ws, &mut f),
                ProjectListItem::Package(pkg) => visit_package_searchables(pkg, &mut f),
                ProjectListItem::NonRust(nr) => f(non_rust_searchable(nr)),
                ProjectListItem::WorkspaceWorktrees(g) => {
                    visit_workspace_searchables(g.primary(), &mut f);
                    for linked in g.linked() {
                        visit_workspace_searchables_with_root_name(
                            linked,
                            linked
                                .worktree_name()
                                .map(Cow::Borrowed)
                                .unwrap_or_else(|| Cow::Owned(linked.display_name())),
                            &mut f,
                        );
                    }
                },
                ProjectListItem::PackageWorktrees(g) => {
                    visit_package_searchables(g.primary(), &mut f);
                    for linked in g.linked() {
                        visit_package_searchables_with_root_name(
                            linked,
                            linked
                                .worktree_name()
                                .map(Cow::Borrowed)
                                .unwrap_or_else(|| Cow::Owned(linked.display_name())),
                            &mut f,
                        );
                    }
                },
            }
        }
    }

    pub(crate) fn find_searchable_by_abs_path(&self, target: &Path) -> Option<SearchableItem<'_>> {
        for item in &self.items {
            let found = match item {
                ProjectListItem::Workspace(ws) => find_workspace_searchable(ws, target),
                ProjectListItem::Package(pkg) => find_package_searchable(pkg, target),
                ProjectListItem::NonRust(nr) => {
                    (nr.path() == target).then(|| non_rust_searchable(nr))
                },
                ProjectListItem::WorkspaceWorktrees(g) => {
                    find_workspace_searchable(g.primary(), target).or_else(|| {
                        g.linked()
                            .iter()
                            .find_map(|ws| find_workspace_searchable(ws, target))
                    })
                },
                ProjectListItem::PackageWorktrees(g) => {
                    find_package_searchable(g.primary(), target).or_else(|| {
                        g.linked()
                            .iter()
                            .find_map(|pkg| find_package_searchable(pkg, target))
                    })
                },
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }

    // -- Hierarchy mutations ----------------------------------------------

    /// Find a leaf item by absolute path and replace it, returning the old
    /// item. Descends into worktree groups to find matching entries.
    pub(crate) fn replace_leaf_by_path(
        &mut self,
        path: &Path,
        mut replacement: ProjectListItem,
    ) -> Option<ProjectListItem> {
        for item in &mut self.items {
            match item {
                ProjectListItem::Workspace(_)
                | ProjectListItem::Package(_)
                | ProjectListItem::NonRust(_) => {
                    if item.path() == path {
                        std::mem::swap(item, &mut replacement);
                        return Some(replacement);
                    }
                },
                ProjectListItem::WorkspaceWorktrees(g) => {
                    if g.primary().path() == path
                        && let ProjectListItem::Workspace(ws) = replacement
                    {
                        let old = g.primary().clone();
                        *g.primary_mut() = ws;
                        return Some(ProjectListItem::Workspace(old));
                    }
                    for linked in g.linked_mut() {
                        if linked.path() == path
                            && let ProjectListItem::Workspace(ws) = replacement
                        {
                            let old = linked.clone();
                            *linked = ws;
                            return Some(ProjectListItem::Workspace(old));
                        }
                    }
                },
                ProjectListItem::PackageWorktrees(g) => {
                    if g.primary().path() == path
                        && let ProjectListItem::Package(pkg) = replacement
                    {
                        let old = g.primary().clone();
                        *g.primary_mut() = pkg;
                        return Some(ProjectListItem::Package(old));
                    }
                    for linked in g.linked_mut() {
                        if linked.path() == path
                            && let ProjectListItem::Package(pkg) = replacement
                        {
                            let old = linked.clone();
                            *linked = pkg;
                            return Some(ProjectListItem::Package(old));
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
    pub(crate) fn insert_into_hierarchy(&mut self, item: ProjectListItem) -> bool {
        let item_path = item.path().to_path_buf();
        for existing in &mut self.items {
            let inserted = match existing {
                ProjectListItem::Workspace(ws) => try_insert_member(ws, &item_path, &item),
                ProjectListItem::WorkspaceWorktrees(g) => {
                    try_insert_member(g.primary_mut(), &item_path, &item)
                        || g.linked_mut()
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
        self.items.push(item);
        false
    }

    // -- Config-driven regrouping -------------------------------------------

    /// Regroup workspace members based on `inline_dirs` config. Walks all
    /// workspaces (including inside worktree groups) and re-sorts their
    /// members into `Named` / `Inline` groups.
    pub(crate) fn regroup_members(&mut self, inline_dirs: &[String]) {
        for item in &mut self.items {
            match item {
                ProjectListItem::Workspace(ws) => {
                    regroup_workspace(ws, inline_dirs);
                },
                ProjectListItem::WorkspaceWorktrees(g) => {
                    regroup_workspace(g.primary_mut(), inline_dirs);
                    for linked in g.linked_mut() {
                        regroup_workspace(linked, inline_dirs);
                    }
                },
                _ => {},
            }
        }
    }

    // -- Vec-like operations -------------------------------------------------

    pub(crate) fn clear(&mut self) { self.items.clear(); }

    #[cfg(test)]
    pub(crate) fn push(&mut self, item: ProjectListItem) { self.items.push(item); }
}

// -- Deref to slice for read access ---------------------------------------

impl Deref for ProjectList {
    type Target = [ProjectListItem];

    fn deref(&self) -> &[ProjectListItem] { &self.items }
}

impl DerefMut for ProjectList {
    fn deref_mut(&mut self) -> &mut [ProjectListItem] { &mut self.items }
}

// -- IntoIterator for `for item in &projects` / `for item in &mut projects`

impl<'a> IntoIterator for &'a ProjectList {
    type IntoIter = std::slice::Iter<'a, ProjectListItem>;
    type Item = &'a ProjectListItem;

    fn into_iter(self) -> Self::IntoIter { self.items.iter() }
}

impl<'a> IntoIterator for &'a mut ProjectList {
    type IntoIter = std::slice::IterMut<'a, ProjectListItem>;
    type Item = &'a mut ProjectListItem;

    fn into_iter(self) -> Self::IntoIter { self.items.iter_mut() }
}

// -- Helpers --------------------------------------------------------------

fn regroup_workspace(ws: &mut RustProject<Workspace>, inline_dirs: &[String]) {
    // Collect all members from all existing groups.
    let members: Vec<RustProject<Package>> = ws
        .groups_mut()
        .drain(..)
        .flat_map(MemberGroup::into_members)
        .collect();

    // Re-sort into groups based on subdirectory and inline_dirs.
    let mut group_map: std::collections::HashMap<String, Vec<RustProject<Package>>> =
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
        display_path:     Cow::Owned(project.display_path()),
        name:             Cow::Owned(project.display_name()),
        is_rust:          false,
        visibility:       project.visibility(),
        disk_usage_bytes: project.disk_usage_bytes(),
    }
}

fn package_searchable<'a>(
    project: &'a RustProject<Package>,
    name: Cow<'a, str>,
) -> SearchableItem<'a> {
    SearchableItem {
        abs_path: project.path(),
        display_path: Cow::Owned(project.display_path()),
        name,
        is_rust: true,
        visibility: project.visibility(),
        disk_usage_bytes: project.disk_usage_bytes(),
    }
}

fn workspace_searchable<'a>(
    project: &'a RustProject<Workspace>,
    name: Cow<'a, str>,
) -> SearchableItem<'a> {
    SearchableItem {
        abs_path: project.path(),
        display_path: Cow::Owned(project.display_path()),
        name,
        is_rust: true,
        visibility: project.visibility(),
        disk_usage_bytes: project.disk_usage_bytes(),
    }
}

fn vendored_searchable(project: &RustProject<Package>) -> SearchableItem<'_> {
    SearchableItem {
        abs_path:         project.path(),
        display_path:     Cow::Owned(project.display_path()),
        name:             Cow::Owned(format!("{} (vendored)", project.display_name())),
        is_rust:          true,
        visibility:       project.visibility(),
        disk_usage_bytes: project.disk_usage_bytes(),
    }
}

fn visit_package_searchables(pkg: &RustProject<Package>, f: &mut impl FnMut(SearchableItem<'_>)) {
    visit_package_searchables_with_root_name(pkg, Cow::Owned(pkg.display_name()), f);
}

fn visit_package_searchables_with_root_name<'a>(
    pkg: &'a RustProject<Package>,
    root_name: Cow<'a, str>,
    f: &mut impl FnMut(SearchableItem<'a>),
) {
    f(package_searchable(pkg, root_name));
    for vendored in pkg.vendored() {
        f(vendored_searchable(vendored));
    }
}

fn find_package_searchable<'a>(
    pkg: &'a RustProject<Package>,
    target: &Path,
) -> Option<SearchableItem<'a>> {
    if pkg.path() == target {
        return Some(package_searchable(pkg, Cow::Owned(pkg.display_name())));
    }
    pkg.vendored()
        .iter()
        .find(|vendored| vendored.path() == target)
        .map(vendored_searchable)
}

fn visit_workspace_searchables(
    ws: &RustProject<Workspace>,
    f: &mut impl FnMut(SearchableItem<'_>),
) {
    visit_workspace_searchables_with_root_name(ws, Cow::Owned(ws.display_name()), f);
}

fn visit_workspace_searchables_with_root_name<'a>(
    ws: &'a RustProject<Workspace>,
    root_name: Cow<'a, str>,
    f: &mut impl FnMut(SearchableItem<'a>),
) {
    f(workspace_searchable(ws, root_name));
    for group in ws.groups() {
        for member in group.members() {
            f(package_searchable(
                member,
                Cow::Owned(member.display_name()),
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
    ws: &'a RustProject<Workspace>,
    target: &Path,
) -> Option<SearchableItem<'a>> {
    if ws.path() == target {
        return Some(workspace_searchable(ws, Cow::Owned(ws.display_name())));
    }
    for group in ws.groups() {
        for member in group.members() {
            if member.path() == target {
                return Some(package_searchable(
                    member,
                    Cow::Owned(member.display_name()),
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

fn try_insert_member(
    ws: &mut RustProject<Workspace>,
    item_path: &Path,
    item: &ProjectListItem,
) -> bool {
    if !item_path.starts_with(ws.path()) || item_path == ws.path() {
        return false;
    }
    let ProjectListItem::Package(pkg) = item else {
        return false;
    };
    // Add to the first inline group, or create one.
    let inline = ws.groups_mut().iter_mut().find(|g| g.is_inline());
    if let Some(group) = inline {
        group.members_mut().push(pkg.clone());
        group.members_mut().sort_by(|a, b| {
            let na = a.name().unwrap_or_else(|| a.path().to_str().unwrap_or(""));
            let nb = b.name().unwrap_or_else(|| b.path().to_str().unwrap_or(""));
            na.cmp(nb)
        });
    } else {
        ws.groups_mut().push(MemberGroup::Inline {
            members: vec![pkg.clone()],
        });
    }
    true
}
