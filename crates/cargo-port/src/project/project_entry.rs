use std::ops::Deref;
use std::ops::DerefMut;
use std::path::Path;

use super::git::RepoInfo;
use super::info::GitHubInfo;
use super::info::ProjectCiData;
use super::root_item::RootItem;

/// Repo-level metadata shared by every checkout of the same git repo.
///
/// Stored on the containing `ProjectEntry`, not on each `ProjectInfo`, so
/// the fields below cannot drift between sibling worktrees of the same
/// repo. `repo_info` is `None` while the repo is known but the background
/// `LocalGitInfo::get` call hasn't completed yet — the UI can use that
/// distinction to show "loading" instead of empty values.
#[derive(Clone, Default)]
pub(crate) struct GitRepo {
    pub repo_info:   Option<RepoInfo>,
    pub github_info: Option<GitHubInfo>,
    pub ci_data:     ProjectCiData,
}

/// A top-level entry in the project list. Wraps a `RootItem` with the
/// repo-level data that belongs to the entire entry (one repo, possibly
/// multiple checkouts).
///
/// `git_repo` is `None` when the entry is not inside a git repo (e.g. a
/// non-Rust project outside any repo, or an uninitialized directory).
#[derive(Clone)]
pub(crate) struct ProjectEntry {
    pub item:     RootItem,
    pub git_repo: Option<GitRepo>,
}

impl ProjectEntry {
    /// Wrap a `RootItem`, auto-computing whether it lives inside a git
    /// repo. The detection here matches the same `git_repo_root`
    /// probe used elsewhere in the tree.
    pub(crate) fn new(item: RootItem) -> Self {
        let git_repo = git_repo_for(&item);
        Self { item, git_repo }
    }

    /// Construct with explicit repo data — used when a caller has just
    /// computed `git_repo` (e.g. preserving across refresh).
    pub(crate) const fn with_repo(item: RootItem, git_repo: Option<GitRepo>) -> Self {
        Self { item, git_repo }
    }

    #[cfg(test)]
    #[expect(dead_code, reason = "Reserved for later-stage test helpers")]
    pub(crate) fn for_tests(item: RootItem) -> Self { Self::new(item) }
}

/// `ProjectEntry` derefs transparently to its contained `RootItem` so
/// call sites that only need tree-shape data (paths, names, visibility,
/// ...) don't have to rewrite `entry.foo()` as `entry.item.foo()`.
/// Pattern-matching on variants still needs explicit `&entry.item`
/// because Deref coercion doesn't apply there — that's fine, those
/// sites already read tree-shape structure.
impl Deref for ProjectEntry {
    type Target = RootItem;

    fn deref(&self) -> &RootItem { &self.item }
}

impl DerefMut for ProjectEntry {
    fn deref_mut(&mut self) -> &mut RootItem { &mut self.item }
}

fn git_repo_for(item: &RootItem) -> Option<GitRepo> {
    super::git::git_repo_root(item.path()).map(|_| GitRepo::default())
}

/// True if `target` is within (or equal to) the entry's hierarchy. Walks
/// the contained `RootItem`'s paths via `at_path` — the same lookup the
/// read-side accessors already use.
pub(crate) fn entry_contains(entry: &ProjectEntry, target: &Path) -> bool {
    entry.item.at_path(target).is_some()
        || entry
            .item
            .submodules()
            .iter()
            .any(|s| s.path.as_path() == target)
}
