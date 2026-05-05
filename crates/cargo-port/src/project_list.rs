use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Index;
use std::path::Path;

use indexmap::IndexMap;
use indexmap::map::Values;
use indexmap::map::ValuesMut;

use crate::ci;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::lint::LintRuns;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::GitStatus;
use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::project::ProjectEntry;
use crate::project::ProjectFields;
use crate::project::ProjectInfo;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::RustInfo;
use crate::project::RustProject;
use crate::project::Submodule;
use crate::project::VendoredPackage;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::project::WorktreeStatus;
use crate::scan;

/// Owning wrapper around the project hierarchy.
///
/// `ProjectList` is the single source of truth for all project data.
/// Mutations go through its methods; derived state is computed from it on
/// demand.
///
/// The underlying store is `IndexMap<AbsolutePath, ProjectEntry>` keyed by
/// each root's absolute path. The map preserves insertion order so
/// iteration stays deterministic, and gives O(1) root-path lookups via
/// `get` without a separate index that would have to be kept in sync by
/// convention. Every mutation site updates keys and values together, so
/// the "key matches the root's own path" invariant cannot silently drift.
#[derive(Clone, Default)]
pub(crate) struct ProjectList {
    roots: IndexMap<AbsolutePath, ProjectEntry>,
}

impl ProjectList {
    pub(crate) fn new(items: Vec<RootItem>) -> Self {
        Self {
            roots: items
                .into_iter()
                .map(|item| {
                    let entry = ProjectEntry::new(item);
                    (entry.item.path().clone(), entry)
                })
                .collect(),
        }
    }

    // -- Slice-like read surface ------------------------------------------

    pub(crate) fn len(&self) -> usize { self.roots.len() }

    pub(crate) fn is_empty(&self) -> bool { self.roots.is_empty() }

    pub(crate) fn iter(&self) -> Values<'_, AbsolutePath, ProjectEntry> { self.roots.values() }

    #[cfg(test)]
    pub(crate) fn first(&self) -> Option<&ProjectEntry> {
        self.roots.first().map(|(_, entry)| entry)
    }

    pub(crate) fn get(&self, index: usize) -> Option<&ProjectEntry> {
        self.roots.get_index(index).map(|(_, entry)| entry)
    }

    pub(crate) fn resolved_root_labels(&self, include_non_rust: bool) -> Vec<String> {
        let mut labels: Vec<String> = self
            .roots
            .values()
            .map(|entry| entry.item.root_directory_name().into_string())
            .collect();
        let mut collision_sets: HashMap<String, Vec<usize>> = HashMap::new();

        for (index, entry) in self.roots.values().enumerate() {
            if matches!(entry.item.visibility(), Visibility::Dismissed) {
                continue;
            }
            if !include_non_rust && !entry.item.is_rust() {
                continue;
            }
            collision_sets
                .entry(entry.item.root_directory_name().into_string())
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
                    .map(|&index| self.roots[index].item.display_path().into_string())
                    .collect::<Vec<_>>(),
            );
            for (index, suffix) in indices.into_iter().zip(suffixes) {
                labels[index] = format!("{} [{suffix}]", labels[index]);
            }
        }

        for (label, entry) in labels.iter_mut().zip(self.roots.values()) {
            if let Some(suffix) = entry.item.worktree_badge_suffix() {
                label.push_str(&suffix);
            }
        }

        labels
    }

    pub(crate) fn git_directories(&self) -> Vec<AbsolutePath> {
        self.roots
            .values()
            .filter_map(|entry| entry.item.git_directory())
            .collect()
    }

    // -- Leaf iteration ---------------------------------------------------

    /// Iterate all leaf-level projects from the hierarchy.
    ///
    /// For `Rust`, `NonRust`: yields the entry directly.
    /// For worktree groups: yields primary and each linked leaf wrapped in
    /// a synthesized `ProjectEntry` whose `item` is `Rust(Workspace(..))`
    /// or `Rust(Package(..))`. The synthesized entries share the outer
    /// `git_repo` via clone so each leaf sees the same repo-level data.
    pub(crate) fn for_each_leaf(&self, mut f: impl FnMut(&ProjectEntry)) {
        for entry in self.roots.values() {
            match &entry.item {
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    let synth = |ws: &Workspace| {
                        ProjectEntry::with_repo(
                            RootItem::Rust(RustProject::Workspace(ws.clone())),
                            entry.git_repo.clone(),
                        )
                    };
                    f(&synth(primary));
                    for l in linked {
                        f(&synth(l));
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    let synth = |pkg: &Package| {
                        ProjectEntry::with_repo(
                            RootItem::Rust(RustProject::Package(pkg.clone())),
                            entry.git_repo.clone(),
                        )
                    };
                    f(&synth(primary));
                    for l in linked {
                        f(&synth(l));
                    }
                },
                _ => f(entry),
            }
        }
    }

    /// Zero-allocation leaf path iteration. Yields `(path, is_rust)` for
    /// every leaf project without cloning any `RootItem`.
    pub(crate) fn for_each_leaf_path(&self, mut f: impl FnMut(&Path, bool)) {
        for entry in self.roots.values() {
            match &entry.item {
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
        if let Some(entry) = self.roots.get(target) {
            return entry.item.at_path(target);
        }
        self.roots
            .values()
            .find_map(|entry| entry.item.at_path(target))
    }

    pub(crate) fn at_path_mut(&mut self, target: &Path) -> Option<&mut ProjectInfo> {
        // Split into two separate borrows to sidestep the NLL limitation on
        // returning a reference borrowed inside an if-let: first check if the
        // root key matches, then re-borrow mutably to return.
        if self.roots.contains_key(target) {
            return self
                .roots
                .get_mut(target)
                .and_then(|entry| entry.item.at_path_mut(target));
        }
        self.roots
            .values_mut()
            .find_map(|entry| entry.item.at_path_mut(target))
    }

    /// Whether `target` is the path of a submodule under any root entry.
    /// CI fetches and GitHub repo metadata for submodules belong to the
    /// upstream repository and are suppressed at the parent project's
    /// level — see the `BackgroundMsg::GitInfo` handler.
    pub(crate) fn is_submodule_path(&self, target: &Path) -> bool {
        self.roots.values().any(|entry| {
            entry
                .item
                .submodules()
                .iter()
                .any(|s| s.path.as_path() == target)
        })
    }

    pub(crate) fn rust_info_at_path(&self, target: &Path) -> Option<&RustInfo> {
        self.roots
            .values()
            .find_map(|entry| entry.item.rust_info_at_path(target))
    }

    pub(crate) fn rust_info_at_path_mut(&mut self, target: &Path) -> Option<&mut RustInfo> {
        self.roots
            .values_mut()
            .find_map(|entry| entry.item.rust_info_at_path_mut(target))
    }

    pub(crate) fn vendored_at_path(&self, target: &Path) -> Option<&VendoredPackage> {
        self.roots
            .values()
            .find_map(|entry| entry.item.vendored_at_path(target))
    }

    pub(crate) fn vendored_at_path_mut(&mut self, target: &Path) -> Option<&mut VendoredPackage> {
        self.roots
            .values_mut()
            .find_map(|entry| entry.item.vendored_at_path_mut(target))
    }

    /// For a vendored crate path, return the owning root's `LintRuns`.
    ///
    /// Used by the detail pane/icon to show parent lints when a vendored row
    /// is selected — the list-row icon stays blank because `lint_at_path`
    /// does not fall back.
    pub(crate) fn vendored_owner_lint(&self, target: &Path) -> Option<&LintRuns> {
        self.roots
            .values()
            .find_map(|entry| entry.item.vendored_owner_lint(target))
    }

    pub(crate) fn lint_at_path(&self, target: &Path) -> Option<&LintRuns> {
        self.roots
            .values()
            .find_map(|entry| entry.item.lint_at_path(target))
    }

    pub(crate) fn lint_at_path_mut(&mut self, target: &Path) -> Option<&mut LintRuns> {
        self.roots
            .values_mut()
            .find_map(|entry| entry.item.lint_at_path_mut(target))
    }

    /// Top-level entry whose hierarchy contains `target`. One-shot
    /// replacement for the per-field per-path lookups used elsewhere.
    pub(crate) fn entry_containing(&self, target: &Path) -> Option<&ProjectEntry> {
        self.roots
            .values()
            .find(|entry| project::entry_contains(entry, target))
    }

    pub(crate) fn entry_containing_mut(&mut self, target: &Path) -> Option<&mut ProjectEntry> {
        self.roots
            .values_mut()
            .find(|entry| project::entry_contains(entry, target))
    }

    /// Replace `git_repo.ci_data` on the entry containing `path`.
    /// Silently no-ops when no entry contains `path` or the entry
    /// has no git repo.
    pub(crate) fn replace_ci_data_for_path(&mut self, path: &Path, ci_data: ProjectCiData) {
        if let Some(repo) = self
            .entry_containing_mut(path)
            .and_then(|entry| entry.git_repo.as_mut())
        {
            repo.ci_data = ci_data;
        }
    }

    // -- Git/Repo reads (Phase 3) ----------------------------------------

    pub(crate) fn git_info_for(&self, path: &Path) -> Option<&CheckoutInfo> {
        self.at_path(path)
            .and_then(|project| project.local_git_state.info())
    }

    /// Per-repo info (remotes, workflows, default branch, ...) for the
    /// entry containing `path`. `None` means either the path isn't in a
    /// known entry, the entry isn't in a git repo, or the background
    /// `LocalGitInfo::get` call hasn't completed yet.
    pub(crate) fn repo_info_for(&self, path: &Path) -> Option<&RepoInfo> {
        self.entry_containing(path)
            .and_then(|entry| entry.git_repo.as_ref()?.repo_info.as_ref())
    }

    /// Convenience: the primary remote's URL for the checkout at `path`.
    pub(crate) fn primary_url_for(&self, path: &Path) -> Option<&str> {
        let checkout = self.git_info_for(path)?;
        let repo = self.repo_info_for(path)?;
        checkout.primary_url(repo)
    }

    /// Convenience: the primary remote's ahead/behind for the checkout
    /// at `path`.
    pub(crate) fn primary_ahead_behind_for(&self, path: &Path) -> Option<(usize, usize)> {
        let checkout = self.git_info_for(path)?;
        let repo = self.repo_info_for(path)?;
        checkout.primary_ahead_behind(repo)
    }

    /// Pick a remote URL to drive the GitHub fetch for the entry
    /// containing `path`. Independent of the current checkout's
    /// upstream tracking: a worktree on a branch without an upstream
    /// still belongs to the repo and should fetch repo-level metadata.
    /// Preference order: `upstream`, then `origin`, then the first
    /// remote with a parseable owner/repo URL.
    pub(crate) fn fetch_url_for(&self, path: &Path) -> Option<String> {
        let repo = self.repo_info_for(path)?;
        let parseable = |name: &str| {
            repo.remotes
                .iter()
                .find(|r| r.name == name)
                .and_then(|r| r.url.as_deref())
                .filter(|url| ci::parse_owner_repo(url).is_some())
        };
        parseable("upstream")
            .or_else(|| parseable("origin"))
            .or_else(|| {
                repo.remotes.iter().find_map(|r| {
                    let url = r.url.as_deref()?;
                    ci::parse_owner_repo(url).map(|_| url)
                })
            })
            .map(String::from)
    }

    pub(crate) fn git_status_for(&self, path: &Path) -> Option<GitStatus> {
        self.git_info_for(path).map(|info| info.status)
    }

    /// Roll up the worst git path state across all **visible** children of a
    /// `RootItem`. For worktree groups, checks primary + non-dismissed linked
    /// entries. For everything else, returns the state for the single path.
    pub(crate) fn git_status_for_item(&self, item: &RootItem) -> Option<GitStatus> {
        match item {
            RootItem::Worktrees(g) => {
                let states: Box<dyn Iterator<Item = Option<GitStatus>>> = match g {
                    WorktreeGroup::Workspaces {
                        primary, linked, ..
                    } => Box::new(
                        std::iter::once(self.git_status_for(primary.path())).chain(
                            linked
                                .iter()
                                .filter(|l| l.visibility() == Visibility::Visible)
                                .map(|l| self.git_status_for(l.path())),
                        ),
                    ),
                    WorktreeGroup::Packages {
                        primary, linked, ..
                    } => Box::new(
                        std::iter::once(self.git_status_for(primary.path())).chain(
                            linked
                                .iter()
                                .filter(|l| l.visibility() == Visibility::Visible)
                                .map(|l| self.git_status_for(l.path())),
                        ),
                    ),
                };
                worst_git_status(states)
            },
            _ => self.git_status_for(item.path()),
        }
    }

    /// Formatted ahead/behind sync status for the project list columns.
    pub(crate) fn git_sync(&self, path: &Path) -> String {
        let Some(info) = self.git_info_for(path) else {
            return String::new();
        };
        if matches!(info.status, GitStatus::Untracked | GitStatus::Ignored) {
            return String::new();
        }
        match self.primary_ahead_behind_for(path) {
            Some((0, 0)) => IN_SYNC.to_string(),
            Some((a, 0)) => format!("{SYNC_UP}{a}"),
            Some((0, b)) => format!("{SYNC_DOWN}{b}"),
            Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
            None => NO_REMOTE_SYNC.to_string(),
        }
    }

    pub(crate) fn ci_data_for(&self, path: &Path) -> Option<&ProjectCiData> {
        self.entry_containing(path)
            .and_then(|entry| entry.git_repo.as_ref())
            .map(|repo| &repo.ci_data)
    }

    pub(crate) fn ci_info_for(&self, path: &Path) -> Option<&ProjectCiInfo> {
        self.ci_data_for(path).and_then(ProjectCiData::info)
    }

    /// Branch name for a checkout whose CI cannot be inferred from the
    /// parent repo's default-branch runs: an unpushed (no-upstream) branch
    /// that also isn't the default. Used to suppress stale parent-repo CI
    /// status for unpublished worktree branches.
    pub(crate) fn unpublished_ci_branch_name(&self, path: &Path) -> Option<String> {
        let git = self.git_info_for(path)?;
        let default_branch = self
            .repo_info_for(path)
            .and_then(|repo| repo.default_branch.as_deref());
        (git.primary_tracked_ref().is_none() && git.branch.as_deref() != default_branch)
            .then(|| git.branch.clone())
            .flatten()
    }

    pub(crate) fn is_deleted(&self, path: &Path) -> bool {
        self.at_path(path)
            .is_some_and(|project| project.visibility == Visibility::Deleted)
    }

    pub(crate) fn is_rust_at_path(&self, path: &Path) -> bool {
        self.iter().any(|item| {
            if item
                .submodules()
                .iter()
                .any(|submodule| submodule.path.as_path() == path)
            {
                return false;
            }
            (item.path() == path || item.at_path(path).is_some()) && item.is_rust()
        })
    }

    pub(crate) fn is_vendored_path(&self, path: &Path) -> bool {
        self.iter().any(|item| match &item.item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.vendored().iter().any(|v| v.path() == path)
            },
            RootItem::Rust(RustProject::Package(pkg)) => {
                pkg.vendored().iter().any(|v| v.path() == path)
            },
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => std::iter::once(primary)
                .chain(linked.iter())
                .any(|ws| ws.vendored().iter().any(|v| v.path() == path)),
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => std::iter::once(primary)
                .chain(linked.iter())
                .any(|pkg| pkg.vendored().iter().any(|v| v.path() == path)),
            RootItem::NonRust(_) => false,
        })
    }

    pub(crate) fn is_workspace_member_path(&self, path: &Path) -> bool {
        self.iter().any(|item| match &item.item {
            RootItem::Rust(RustProject::Workspace(ws)) => ws
                .groups()
                .iter()
                .any(|g| g.members().iter().any(|m| m.path() == path)),
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => std::iter::once(primary).chain(linked.iter()).any(|ws| {
                ws.groups()
                    .iter()
                    .any(|g| g.members().iter().any(|m| m.path() == path))
            }),
            _ => false,
        })
    }

    pub(crate) fn git_main(&self, path: &Path) -> String {
        let Some(info) = self.git_info_for(path) else {
            return String::new();
        };
        if matches!(info.status, GitStatus::Untracked | GitStatus::Ignored) {
            return String::new();
        }
        match info.ahead_behind_local {
            Some((0, 0)) => IN_SYNC.to_string(),
            Some((a, 0)) => format!("{SYNC_UP}{a}"),
            Some((0, b)) => format!("{SYNC_DOWN}{b}"),
            Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
            None => String::new(),
        }
    }

    // -- Hierarchy mutations ----------------------------------------------

    /// Find a leaf item by absolute path and replace it, returning the old
    /// item. Descends into worktree groups to find matching leaves. The
    /// outer `ProjectEntry.git_repo` is preserved; callers that want to
    /// promote a leaf into a worktree group must use
    /// `promote_to_worktree_group` instead, so the intent is explicit.
    ///
    /// The caller must pass a `replacement` whose own path equals `path`.
    /// This preserves the `IndexMap` key invariant: no mutation here can
    /// change a root entry's primary path, so keys stay in sync with the
    /// entries they index.
    pub(crate) fn replace_leaf_by_path(
        &mut self,
        path: &Path,
        mut replacement: RootItem,
    ) -> Option<RootItem> {
        for entry in self.roots.values_mut() {
            match &mut entry.item {
                item @ (RootItem::Rust(_) | RootItem::NonRust(_)) => {
                    if item.path() == path {
                        debug_assert_eq!(
                            replacement.path().as_path(),
                            path,
                            "replacement path must match target path"
                        );
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

    /// Promote the top-level entry whose primary path matches `path` into a
    /// worktree group. Preserves the entry's `git_repo` — a worktree
    /// promotion never crosses repo boundaries, so the existing repo data
    /// carries over unchanged.
    ///
    /// Returns `true` if an entry was found and promoted.
    #[expect(dead_code, reason = "Stage 0 scaffolding; used in later stages")]
    pub(crate) fn promote_to_worktree_group(&mut self, path: &Path, group: WorktreeGroup) -> bool {
        let Some(entry) = self.roots.get_mut(path) else {
            return false;
        };
        debug_assert_eq!(
            group.primary_path().as_path(),
            path,
            "promoted group primary must retain the same root path"
        );
        entry.item = RootItem::Worktrees(group);
        true
    }

    /// Insert a discovered item into the hierarchy. If the item is a package
    /// whose path falls inside an existing workspace, it is added as an
    /// inline member of that workspace. Otherwise it is pushed as a
    /// top-level peer.
    ///
    /// Returns `true` if the item was inserted into an existing workspace.
    pub(crate) fn insert_into_hierarchy(&mut self, item: RootItem) -> bool {
        let item_path = item.path().to_path_buf();
        for entry in self.roots.values_mut() {
            if try_attach_worktree(&mut entry.item, &item) {
                return false;
            }

            let inserted = match &mut entry.item {
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
        // No parent workspace found — add as top-level peer, keeping the
        // `IndexMap` in ascending path order (matches the pre-IndexMap
        // binary-search behavior).
        let insert_index = self
            .roots
            .keys()
            .position(|existing| existing.as_path() > item_path.as_path())
            .unwrap_or(self.roots.len());
        let key = item.path().clone();
        self.roots
            .shift_insert(insert_index, key, ProjectEntry::new(item));
        false
    }

    // -- Config-driven regrouping -------------------------------------------

    /// Regroup workspace members based on `inline_dirs` config. Walks all
    /// workspaces (including inside worktree groups) and re-sorts their
    /// members into `Named` / `Inline` groups.
    pub(crate) fn regroup_members(&mut self, inline_dirs: &[String]) {
        for entry in self.roots.values_mut() {
            match &mut entry.item {
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
        while index < self.roots.len() {
            let Some(identity) = linked_worktree_identity(&self.roots[index].item).cloned() else {
                index += 1;
                continue;
            };
            let Some(mut target_index) =
                find_matching_worktree_container(&self.roots, index, &identity)
            else {
                index += 1;
                continue;
            };

            let Some((_key, linked_entry)) = self.roots.shift_remove_index(index) else {
                index += 1;
                continue;
            };
            if target_index > index {
                target_index -= 1;
            }
            let attached =
                try_attach_worktree(&mut self.roots[target_index].item, &linked_entry.item);
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

    pub(crate) fn clear(&mut self) { self.roots.clear(); }

    #[cfg(test)]
    pub(crate) fn push(&mut self, item: RootItem) {
        let key = item.path().clone();
        self.roots.insert(key, ProjectEntry::new(item));
    }
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
        && linked.worktree_status().is_linked_worktree()
    {
        match existing {
            RootItem::Rust(RustProject::Workspace(primary))
                if linked.worktree_status().primary_root() == existing_identity.as_ref() =>
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
            }) if linked.worktree_status().primary_root() == existing_identity.as_ref() => {
                group_linked.push(linked.clone());
                return true;
            },
            _ => {},
        }
    }

    if let RootItem::Rust(RustProject::Package(linked)) = item
        && linked.worktree_status().is_linked_worktree()
    {
        match existing {
            RootItem::Rust(RustProject::Package(primary))
                if linked.worktree_status().primary_root() == existing_identity.as_ref() =>
            {
                let primary = primary.clone();
                *existing =
                    RootItem::Worktrees(WorktreeGroup::new_packages(primary, vec![linked.clone()]));
                return true;
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                linked: group_linked,
                ..
            }) if linked.worktree_status().primary_root() == existing_identity.as_ref() => {
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
        RootItem::Rust(p) => p.worktree_status().primary_root(),
        RootItem::Worktrees(WorktreeGroup::Workspaces { primary, .. }) => {
            primary.worktree_status().primary_root()
        },
        RootItem::Worktrees(WorktreeGroup::Packages { primary, .. }) => {
            primary.worktree_status().primary_root()
        },
        RootItem::NonRust(_) => None,
    }
}

fn linked_worktree_identity(item: &RootItem) -> Option<&AbsolutePath> {
    match item {
        RootItem::Rust(p) => match p.worktree_status() {
            WorktreeStatus::Linked { primary } => Some(primary),
            _ => None,
        },
        _ => None,
    }
}

fn find_matching_worktree_container(
    roots: &IndexMap<AbsolutePath, ProjectEntry>,
    linked_index: usize,
    identity: &AbsolutePath,
) -> Option<usize> {
    roots.values().enumerate().find_map(|(index, entry)| {
        if index == linked_index {
            return None;
        }
        (item_worktree_identity(&entry.item) == Some(identity)).then_some(index)
    })
}

// -- Index<usize> so call sites can do `projects[i]` like a slice.

impl Index<usize> for ProjectList {
    type Output = ProjectEntry;

    fn index(&self, index: usize) -> &ProjectEntry { &self.roots[index] }
}

// -- IntoIterator for `for entry in &projects` / `for entry in &mut projects`

impl<'a> IntoIterator for &'a ProjectList {
    type IntoIter = Values<'a, AbsolutePath, ProjectEntry>;
    type Item = &'a ProjectEntry;

    fn into_iter(self) -> Self::IntoIter { self.roots.values() }
}

impl<'a> IntoIterator for &'a mut ProjectList {
    type IntoIter = ValuesMut<'a, AbsolutePath, ProjectEntry>;
    type Item = &'a mut ProjectEntry;

    fn into_iter(self) -> Self::IntoIter { self.roots.values_mut() }
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
    let mut group_map: HashMap<String, Vec<Package>> = std::collections::HashMap::new();
    for member in members {
        let relative = member
            .path()
            .strip_prefix(ws.path())
            .ok()
            .map(scan::normalize_workspace_path)
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
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
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

// ── Visible-rows flattening ──────────────────────────────────────────
//
// The project tree is nested; the renderer wants a flat list. The
// types and walker below produce that flat list, expanding /
// collapsing groups based on user state.

/// User-driven expansion state key. Identifies which of the
/// nested containers (root nodes, named groups, worktree
/// entries, worktree groups) the user has toggled open.
#[derive(Hash, Eq, PartialEq, Clone)]
pub(crate) enum ExpandKey {
    Node(usize),
    Group(usize, usize),
    Worktree(usize, usize),
    WorktreeGroup(usize, usize, usize),
}

/// What a visible row represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VisibleRow {
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
    pub(crate) fn visible_rows(
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
                RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. }) => {
                    if wtg.renders_as_group() {
                        emit_workspace_worktree_group(&mut rows, ni, wtg, expanded);
                    } else if let Some(workspace) = wtg.single_live_workspace() {
                        emit_groups(&mut rows, ni, workspace.groups(), expanded);
                        emit_vendored_rows(&mut rows, ni, workspace.vendored());
                    }
                },
                RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. }) => {
                    if wtg.renders_as_group() {
                        emit_package_worktree_group(&mut rows, ni, wtg, expanded);
                    } else if let Some(package) = wtg.single_live_package() {
                        emit_vendored_rows(&mut rows, ni, package.vendored());
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

fn emit_workspace_worktree_group(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    wtg: &WorktreeGroup,
    expanded: &HashSet<ExpandKey>,
) {
    let WorktreeGroup::Workspaces {
        primary, linked, ..
    } = wtg
    else {
        return;
    };
    if !matches!(primary.visibility(), Visibility::Dismissed) {
        let wi = 0;
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
        if primary.has_members() && expanded.contains(&ExpandKey::Worktree(ni, wi)) {
            emit_worktree_children(rows, ni, wi, primary.groups(), primary.vendored(), expanded);
        }
    }
    for (i, ws) in linked.iter().enumerate() {
        if matches!(ws.visibility(), Visibility::Dismissed) {
            continue;
        }
        let wi = i + 1;
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
        if ws.has_members() && expanded.contains(&ExpandKey::Worktree(ni, wi)) {
            emit_worktree_children(rows, ni, wi, ws.groups(), ws.vendored(), expanded);
        }
    }
}

fn emit_package_worktree_group(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    wtg: &WorktreeGroup,
    _expanded: &HashSet<ExpandKey>,
) {
    let WorktreeGroup::Packages {
        primary, linked, ..
    } = wtg
    else {
        return;
    };
    if !matches!(primary.visibility(), Visibility::Dismissed) {
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: 0,
        });
    }
    for (i, pkg) in linked.iter().enumerate() {
        if matches!(pkg.visibility(), Visibility::Dismissed) {
            continue;
        }
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: i + 1,
        });
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
fn worst_git_status(states: impl Iterator<Item = Option<GitStatus>>) -> Option<GitStatus> {
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
