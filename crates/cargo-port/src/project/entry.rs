use std::ops::Deref;
use std::ops::DerefMut;

use super::git;
use super::git::RepoInfo;
use super::info::GitHubInfo;
use super::info::ProjectCiData;
use super::info::ProjectPrData;
use super::root_item::RootItem;

/// The Git probe distinguishes pending data from a completed probe outside Git.
#[derive(Clone, Default)]
pub(crate) enum RepositoryProbe {
    #[default]
    Awaiting,
    NoRepository,
    Repository(RepoInfo),
}

impl RepositoryProbe {
    pub(crate) const fn as_ref(&self) -> Option<&RepoInfo> {
        match self {
            Self::Repository(info) => Some(info),
            Self::Awaiting | Self::NoRepository => None,
        }
    }

    pub(crate) const fn as_mut(&mut self) -> Option<&mut RepoInfo> {
        match self {
            Self::Repository(info) => Some(info),
            Self::Awaiting | Self::NoRepository => None,
        }
    }
}

/// Store the repository probe and shared metadata on the containing entry.
#[derive(Clone, Default)]
pub(crate) struct GitRepo {
    pub repo_info:   RepositoryProbe,
    pub github_info: Option<GitHubInfo>,
    pub ci_data:     ProjectCiData,
    pub pr_data:     ProjectPrData,
}

impl GitRepo {
    pub(crate) fn without_repository() -> Self {
        Self {
            repo_info: RepositoryProbe::NoRepository,
            ..Self::default()
        }
    }
}

/// A top-level entry in the project list. Wraps a `RootItem` with the
/// repo-level data that belongs to the entire entry (one repo, possibly
/// multiple checkouts).
///
/// `git_repo` can be absent before the background probe completes.
/// A completed probe outside Git stores `RepositoryProbe::NoRepository`.
#[derive(Clone)]
pub(crate) struct ProjectEntry {
    pub root_item: RootItem,
    pub git_repo:  Option<GitRepo>,
}

impl ProjectEntry {
    /// Wrap a `RootItem`, auto-computing whether it lives inside a git
    /// repo. The detection here matches the same `git_repo_root`
    /// probe used elsewhere in the tree.
    pub(crate) fn new(root_item: RootItem) -> Self {
        let git_repo = git_repo_for(&root_item);
        Self {
            root_item,
            git_repo,
        }
    }

    /// Construct with explicit repo data — used when a caller has just
    /// computed `git_repo` (e.g. preserving across refresh).
    pub(crate) const fn with_repo(root_item: RootItem, git_repo: Option<GitRepo>) -> Self {
        Self {
            root_item,
            git_repo,
        }
    }

    #[cfg(test)]
    #[expect(dead_code, reason = "Reserved for later-stage test helpers")]
    pub(crate) fn for_tests(root_item: RootItem) -> Self { Self::new(root_item) }
}

/// `ProjectEntry` derefs transparently to its contained `RootItem` so
/// call sites that only need tree-structure data (paths, names, visibility,
/// ...) don't have to rewrite `entry.foo()` as `entry.root_item.foo()`.
/// Pattern-matching on variants still needs explicit `&entry.root_item`
/// because Deref coercion doesn't apply there — that's fine, those
/// sites already read the tree's structural data.
impl Deref for ProjectEntry {
    type Target = RootItem;

    fn deref(&self) -> &RootItem { &self.root_item }
}

impl DerefMut for ProjectEntry {
    fn deref_mut(&mut self) -> &mut RootItem { &mut self.root_item }
}

fn git_repo_for(item: &RootItem) -> Option<GitRepo> {
    git::git_repo_root(item.path()).map(|_| GitRepo::default())
}
