use std::ops::Deref;
use std::ops::DerefMut;
use std::path::Path;

use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::ProjectListItem;
use crate::project::RustProject;
use crate::project::Workspace;

/// Owning wrapper around the project hierarchy.
///
/// `ProjectList` is the single source of truth for all project data.
/// Mutations go through its methods; derived state (flat entries, visible
/// rows) is computed from it on demand.
#[derive(Clone, Default)]
pub(crate) struct ProjectList(Vec<ProjectListItem>);

impl ProjectList {
    pub(crate) fn new(items: Vec<ProjectListItem>) -> Self { Self(items) }

    pub(crate) fn into_inner(self) -> Vec<ProjectListItem> { self.0 }

    // -- Leaf iteration ---------------------------------------------------

    /// Iterate all leaf-level projects from the hierarchy.
    ///
    /// For `Workspace`, `Package`, `NonRust`: yields the item directly.
    /// For worktree groups: yields primary and each linked entry wrapped as
    /// `Workspace` or `Package`.
    pub(crate) fn for_each_leaf(&self, mut f: impl FnMut(&ProjectListItem)) {
        for item in &self.0 {
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
        for item in &self.0 {
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

    // -- Hierarchy mutations ----------------------------------------------

    /// Find a leaf item by absolute path and replace it, returning the old
    /// item. Descends into worktree groups to find matching entries.
    pub(crate) fn replace_leaf_by_path(
        &mut self,
        path: &Path,
        mut replacement: ProjectListItem,
    ) -> Option<ProjectListItem> {
        for item in self.0.iter_mut() {
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
        for existing in self.0.iter_mut() {
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
        self.0.push(item);
        false
    }

    // -- Vec-like operations -------------------------------------------------

    pub(crate) fn clear(&mut self) { self.0.clear(); }

    pub(crate) fn push(&mut self, item: ProjectListItem) { self.0.push(item); }
}

// -- Deref to slice for read access ---------------------------------------

impl Deref for ProjectList {
    type Target = [ProjectListItem];

    fn deref(&self) -> &[ProjectListItem] { &self.0 }
}

impl DerefMut for ProjectList {
    fn deref_mut(&mut self) -> &mut [ProjectListItem] { &mut self.0 }
}

// -- IntoIterator for `for item in &projects` / `for item in &mut projects`

impl<'a> IntoIterator for &'a ProjectList {
    type IntoIter = std::slice::Iter<'a, ProjectListItem>;
    type Item = &'a ProjectListItem;

    fn into_iter(self) -> Self::IntoIter { self.0.iter() }
}

impl<'a> IntoIterator for &'a mut ProjectList {
    type IntoIter = std::slice::IterMut<'a, ProjectListItem>;
    type Item = &'a mut ProjectListItem;

    fn into_iter(self) -> Self::IntoIter { self.0.iter_mut() }
}

// -- Helpers --------------------------------------------------------------

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
