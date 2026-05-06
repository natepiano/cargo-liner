use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Index;
use std::path::Path;

use indexmap::IndexMap;
use indexmap::map::Values;
use indexmap::map::ValuesMut;

use super::app::FinderState;
use super::app::SelectionPaths;
use super::app::SelectionSync;
use super::columns::ProjectListWidths;
use crate::ci;
use crate::ci::OwnerRepo;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::lint::LintRuns;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::DisplayPath;
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

/// Owning wrapper around the project hierarchy plus all project-list
/// navigation state (cursor, expansion set, finder, sort/width caches).
///
/// `ProjectList` is the single source of truth for project data and the
/// per-pane state that navigates that data. Mutations go through its
/// methods; derived state (e.g. `cached_visible_rows`) is computed from
/// it on demand or refreshed by the [`SelectionMutation`] guard.
///
/// The underlying store is `IndexMap<AbsolutePath, ProjectEntry>` keyed by
/// each root's absolute path. The map preserves insertion order so
/// iteration stays deterministic, and gives O(1) root-path lookups via
/// `get` without a separate index that would have to be kept in sync by
/// convention. Every mutation site updates keys and values together, so
/// the "key matches the root's own path" invariant cannot silently drift.
#[derive(Default)]
pub(crate) struct ProjectList {
    roots:               IndexMap<AbsolutePath, ProjectEntry>,
    paths:               SelectionPaths,
    sync:                SelectionSync,
    expanded:            HashSet<ExpandKey>,
    finder:              FinderState,
    cached_visible_rows: Vec<VisibleRow>,
    cached_root_sorted:  Vec<u64>,
    cached_child_sorted: HashMap<usize, Vec<u64>>,
    cached_fit_widths:   ProjectListWidths,
    cursor:              usize,
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
            ..Self::default()
        }
    }

    /// Production-only seeding: load `last_selected` from the terminal-state
    /// file and stamp the lint-enabled state into the column-width cache.
    /// Tests build via `ProjectList::new` / `ProjectList::default` and skip
    /// this side-effecting initialization.
    pub(super) fn init_runtime_state(&mut self, lint_enabled: bool) {
        self.paths = SelectionPaths::new();
        self.cached_fit_widths = ProjectListWidths::new(lint_enabled);
    }

    // -- Slice-like read surface ------------------------------------------

    pub(crate) fn len(&self) -> usize { self.roots.len() }

    pub(crate) fn is_empty(&self) -> bool { self.roots.is_empty() }

    pub(crate) fn iter(&self) -> Values<'_, AbsolutePath, ProjectEntry> { self.roots.values() }

    /// Replace only the project hierarchy, keeping the selection-cluster
    /// state (cursor, expansion set, finder, sort/width caches) intact.
    /// Used by tree-rebuild paths that hand-build a fresh `ProjectList`
    /// for whole-tree replacement.
    pub(super) fn replace_roots_from(&mut self, replacement: Self) {
        self.roots = replacement.roots;
    }

    /// Split-borrow accessor for bulk-expansion paths that need to inspect
    /// the tree structure while mutating the expansion set. Used by
    /// `App::expand_all`, `App::expand_path_in_tree`, and the legacy-root
    /// migration in `async_tasks::tree`.
    pub(super) fn iter_with_expanded_mut(
        &mut self,
    ) -> (
        Values<'_, AbsolutePath, ProjectEntry>,
        &mut HashSet<ExpandKey>,
    ) {
        let Self {
            roots, expanded, ..
        } = self;
        (roots.values(), expanded)
    }

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
    pub(crate) fn compute_visible_rows(
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

// ── Selection-cluster surface ───────────────────────────────────────
//
// Project-list navigation state (cursor, expansion set, finder, sort and
// width caches) lives directly on `ProjectList` because every field is
// project-list scoped — none of it is shared with other panes. The
// `SelectionMutation` guard at the bottom recomputes `cached_visible_rows`
// on drop so visibility-changing mutations stay in sync with derived
// rows. Mutation guard (RAII) — self-only flavor; see
// `src/tui/app/mod.rs` § "Recurring patterns".

impl ProjectList {
    // ── project-list cursor ─────────────────────────────────────────

    pub(super) const fn cursor(&self) -> usize { self.cursor }

    pub(super) const fn set_cursor(&mut self, cursor: usize) { self.cursor = cursor; }

    // ── path tracking ───────────────────────────────────────────────

    pub(super) const fn paths(&self) -> &SelectionPaths { &self.paths }

    pub(super) const fn paths_mut(&mut self) -> &mut SelectionPaths { &mut self.paths }

    // ── sync flag ───────────────────────────────────────────────────

    pub(super) const fn sync(&self) -> SelectionSync { self.sync }

    pub(super) const fn mark_sync_changed(&mut self) { self.sync = SelectionSync::Changed; }

    pub(super) const fn mark_sync_stable(&mut self) { self.sync = SelectionSync::Stable; }

    // ── expansion set ───────────────────────────────────────────────

    pub(super) const fn expanded(&self) -> &HashSet<ExpandKey> { &self.expanded }

    /// Mutable access to the expansion set. Most callers (rebuild paths
    /// in `tui::app::async_tasks` and `tui::app::navigation`) populate
    /// the set in bulk and don't want the per-mutation recompute the
    /// `SelectionMutation` guard fires; the guard covers single-key
    /// toggle paths where the recompute is the whole point.
    pub(super) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> { &mut self.expanded }

    // ── finder state ────────────────────────────────────────────────

    pub(super) const fn finder(&self) -> &FinderState { &self.finder }

    pub(super) const fn finder_mut(&mut self) -> &mut FinderState { &mut self.finder }

    // ── cached visible rows ─────────────────────────────────────────

    pub(super) fn visible_rows(&self) -> &[VisibleRow] { &self.cached_visible_rows }

    pub(super) const fn row_count(&self) -> usize { self.cached_visible_rows.len() }

    // ── cursor movement ─────────────────────────────────────────────

    pub(super) const fn move_up(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.cursor;
        if current > 0 {
            self.cursor = current - 1;
        }
    }

    pub(super) const fn move_down(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.cursor;
        if current < count - 1 {
            self.cursor = current + 1;
        }
    }

    pub(super) const fn move_to_top(&mut self) {
        if self.row_count() > 0 {
            self.cursor = 0;
        }
    }

    pub(super) const fn move_to_bottom(&mut self) {
        let count = self.row_count();
        if count > 0 {
            self.cursor = count - 1;
        }
    }

    /// Recompute `cached_visible_rows` from the current `expanded`
    /// set. Called by [`SelectionMutation::drop`] and (via App) from
    /// `TreeMutation::drop` so externally-driven tree mutations also
    /// keep the visible-rows cache fresh.
    pub(super) fn recompute_visibility(&mut self, include_non_rust: bool) {
        self.cached_visible_rows = self.compute_visible_rows(&self.expanded, include_non_rust);
        let len = self.cached_visible_rows.len();
        if len == 0 {
            self.cursor = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    // ── disk-sort caches ────────────────────────────────────────────

    pub(super) fn cached_root_sorted(&self) -> &[u64] { &self.cached_root_sorted }

    pub(super) const fn cached_child_sorted(&self) -> &HashMap<usize, Vec<u64>> {
        &self.cached_child_sorted
    }

    pub(super) fn set_disk_caches(
        &mut self,
        root_sorted: Vec<u64>,
        child_sorted: HashMap<usize, Vec<u64>>,
    ) {
        self.cached_root_sorted = root_sorted;
        self.cached_child_sorted = child_sorted;
    }

    // ── fit widths ──────────────────────────────────────────────────

    pub(super) const fn fit_widths(&self) -> &ProjectListWidths { &self.cached_fit_widths }

    /// Test-only — production paths replace the whole `ProjectListWidths`
    /// via [`Self::set_fit_widths`] and never observe individual columns
    /// after seeding.
    #[cfg(test)]
    pub(super) const fn fit_widths_mut(&mut self) -> &mut ProjectListWidths {
        &mut self.cached_fit_widths
    }

    pub(super) fn set_fit_widths(&mut self, widths: ProjectListWidths) {
        self.cached_fit_widths = widths;
    }

    pub(super) fn reset_fit_widths(&mut self, lint_enabled: bool) {
        self.cached_fit_widths = ProjectListWidths::new(lint_enabled);
    }

    // ── mutation guard entry point ──────────────────────────────────

    /// Borrow `self` for a visibility-changing mutation.
    ///
    /// Type-level invariant: the guard's mutating methods
    /// (`toggle_expand`, `expand`, `collapse`, `expanded_mut`,
    /// `finder_mut`) are only callable through the returned guard.
    /// The guard's `Drop` recomputes `cached_visible_rows`, so
    /// visibility-affecting mutations cannot drift out of sync with
    /// their derived rows.
    #[allow(
        dead_code,
        reason = "tui::app::navigation (try_expand / try_collapse) still calls \
                  expanded_mut directly because it recomputes via a separate \
                  ensure_visible_rows_cached() call in the same code path."
    )]
    pub(super) const fn mutate(&mut self, include_non_rust: bool) -> SelectionMutation<'_> {
        SelectionMutation {
            project_list: self,
            include_non_rust,
        }
    }
}

/// RAII guard for visibility-changing [`ProjectList`] mutations.
/// Obtained via [`ProjectList::mutate`]; `Drop` recomputes
/// `cached_visible_rows`. Mutation guard (RAII) — self-only flavor.
#[allow(
    dead_code,
    reason = "guard ships alongside ProjectList so the type is in place \
              while call sites still use the direct accessors"
)]
pub(super) struct SelectionMutation<'a> {
    project_list:     &'a mut ProjectList,
    include_non_rust: bool,
}

#[allow(
    dead_code,
    reason = "guard methods ship alongside the type while call sites \
              still use the direct accessors"
)]
impl SelectionMutation<'_> {
    /// Toggle membership of `key` in the expansion set. Returns `true`
    /// if the key was newly inserted.
    pub(super) fn toggle_expand(&mut self, key: ExpandKey) -> bool {
        if self.project_list.expanded.contains(&key) {
            self.project_list.expanded.remove(&key);
            false
        } else {
            self.project_list.expanded.insert(key);
            true
        }
    }

    /// Insert `key` into the expansion set. Returns `true` if the key
    /// was newly inserted.
    pub fn expand(&mut self, key: ExpandKey) -> bool { self.project_list.expanded.insert(key) }

    /// Remove `key` from the expansion set. Returns `true` if the key
    /// was present.
    pub fn collapse(&mut self, key: &ExpandKey) -> bool { self.project_list.expanded.remove(key) }

    /// Mutable access to the underlying expansion set, for bulk
    /// operations (e.g. `clear`, multi-key inserts) that still want
    /// the drop-recompute to fire afterward.
    pub(super) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> {
        &mut self.project_list.expanded
    }

    /// Mutable access to the finder state, for callers that update
    /// the finder query / results inline. The drop-recompute fires
    /// on guard release.
    pub(super) const fn finder_mut(&mut self) -> &mut FinderState { &mut self.project_list.finder }
}

impl Drop for SelectionMutation<'_> {
    fn drop(&mut self) {
        self.project_list
            .recompute_visibility(self.include_non_rust);
    }
}

// ── Phase 11 row-navigation read-side ────────────────────────────────────
//
// Pure ProjectList queries: row → path resolution, expand-key lookup,
// dismiss-target lookup, CI/branch lookups that don't cross into Ci/panes
// state. Cross-subsystem methods (build_selected_pane_data,
// latest_ci_run_for_path, ci_runs_for_display_inner) remain on App.
impl ProjectList {
    pub(super) fn selected_row(&self) -> Option<VisibleRow> {
        let rows = self.visible_rows();
        let selected = self.cursor();
        rows.get(selected).copied()
    }

    pub(super) fn selected_project_path(&self) -> Option<&Path> {
        let row = self.selected_row()?;
        self.path_for_row(row)
    }

    pub(super) fn path_for_row(&self, row: VisibleRow) -> Option<&Path> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                Some(self.get(node_index)?.path().as_path())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => Self::member_path_ref(&self.get(node_index)?.item, group_index, member_index),
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => Self::vendored_path_ref(&self.get(node_index)?.item, vendored_index),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            } => Self::worktree_path_ref(&self.get(node_index)?.item, worktree_index),
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => Self::worktree_member_path_ref(
                &self.get(node_index)?.item,
                worktree_index,
                group_index,
                member_index,
            ),
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => Self::worktree_vendored_path_ref(
                &self.get(node_index)?.item,
                worktree_index,
                vendored_index,
            ),
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => self
                .get(node_index)?
                .submodules()
                .get(submodule_index)
                .map(|s| s.path.as_path()),
        }
    }

    pub(super) fn display_path_for_row(&self, row: VisibleRow) -> Option<DisplayPath> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                let item = self.get(node_index)?;
                Some(item.display_path())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.get(node_index)?;
                match &item.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => {
                        let group = ws.groups().get(group_index)?;
                        let member = group.members().get(member_index)?;
                        Some(member.display_path())
                    },
                    RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                        if !wtg.renders_as_group() =>
                    {
                        let group = wtg.single_live_workspace()?.groups().get(group_index)?;
                        let member = group.members().get(member_index)?;
                        Some(member.display_path())
                    },
                    _ => None,
                }
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => {
                let item = self.get(node_index)?;
                match &item.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => ws
                        .vendored()
                        .get(vendored_index)
                        .map(ProjectFields::display_path),
                    RootItem::Rust(RustProject::Package(pkg)) => pkg
                        .vendored()
                        .get(vendored_index)
                        .map(ProjectFields::display_path),
                    RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                        if !wtg.renders_as_group() =>
                    {
                        wtg.single_live_workspace()?
                            .vendored()
                            .get(vendored_index)
                            .map(ProjectFields::display_path)
                    },
                    RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. })
                        if !wtg.renders_as_group() =>
                    {
                        wtg.single_live_package()?
                            .vendored()
                            .get(vendored_index)
                            .map(ProjectFields::display_path)
                    },
                    _ => None,
                }
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            } => {
                let item = self.get(node_index)?;
                Self::worktree_display_path(item, worktree_index)
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                let item = self.get(node_index)?;
                Self::worktree_member_display_path(item, worktree_index, group_index, member_index)
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.get(node_index)?;
                Self::worktree_vendored_display_path(item, worktree_index, vendored_index)
            },
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => {
                let item = self.get(node_index)?;
                let submodule = item.submodules().get(submodule_index)?;
                Some(DisplayPath::new(project::home_relative_path(
                    &submodule.path,
                )))
            },
        }
    }

    pub(super) fn abs_path_for_row(&self, row: VisibleRow) -> Option<AbsolutePath> {
        match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                let item = self.get(node_index)?;
                Some(item.path().clone())
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let item = self.get(node_index)?;
                match &item.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => {
                        let group = ws.groups().get(group_index)?;
                        let member = group.members().get(member_index)?;
                        Some(member.path().clone())
                    },
                    RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                        if !wtg.renders_as_group() =>
                    {
                        let group = wtg.single_live_workspace()?.groups().get(group_index)?;
                        let member = group.members().get(member_index)?;
                        Some(member.path().clone())
                    },
                    _ => None,
                }
            },
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => {
                let item = self.get(node_index)?;
                match &item.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => {
                        ws.vendored().get(vendored_index).map(|p| p.path().clone())
                    },
                    RootItem::Rust(RustProject::Package(pkg)) => {
                        pkg.vendored().get(vendored_index).map(|p| p.path().clone())
                    },
                    RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                        if !wtg.renders_as_group() =>
                    {
                        wtg.single_live_workspace()?
                            .vendored()
                            .get(vendored_index)
                            .map(|p| p.path().clone())
                    },
                    RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. })
                        if !wtg.renders_as_group() =>
                    {
                        wtg.single_live_package()?
                            .vendored()
                            .get(vendored_index)
                            .map(|p| p.path().clone())
                    },
                    _ => None,
                }
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            } => {
                let item = self.get(node_index)?;
                Self::worktree_abs_path(item, worktree_index)
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => {
                let item = self.get(node_index)?;
                Self::worktree_member_abs_path(item, worktree_index, group_index, member_index)
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => {
                let item = self.get(node_index)?;
                Self::worktree_vendored_abs_path(item, worktree_index, vendored_index)
            },
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => {
                let item = self.get(node_index)?;
                item.submodules()
                    .get(submodule_index)
                    .map(|s| s.path.clone())
            },
        }
    }

    pub(super) fn expand_key_for_row(&self, row: VisibleRow) -> Option<ExpandKey> {
        match row {
            VisibleRow::Root { node_index } => self
                .get(node_index)?
                .has_children()
                .then_some(ExpandKey::Node(node_index)),
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => Some(ExpandKey::Group(node_index, group_index)),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => {
                let item = self.get(node_index)?;
                match &item.item {
                    RootItem::Worktrees(WorktreeGroup::Workspaces {
                        primary, linked, ..
                    }) => {
                        let ws = if worktree_index == 0 {
                            primary
                        } else {
                            linked.get(worktree_index - 1)?
                        };
                        ws.has_members()
                            .then_some(ExpandKey::Worktree(node_index, worktree_index))
                    },
                    _ => None,
                }
            },
            VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                group_index,
            } => Some(ExpandKey::WorktreeGroup(
                node_index,
                worktree_index,
                group_index,
            )),
            VisibleRow::Member { .. }
            | VisibleRow::Vendored { .. }
            | VisibleRow::Submodule { .. }
            | VisibleRow::WorktreeMember { .. }
            | VisibleRow::WorktreeVendored { .. } => None,
        }
    }

    pub(super) fn try_collapse(&mut self, key: &ExpandKey) -> bool {
        self.expanded_mut().remove(key)
    }

    pub(super) fn dismiss_target_for_row_inner(
        &self,
        row: VisibleRow,
    ) -> Option<crate::tui::app::DismissTarget> {
        use super::app::DismissTarget;
        let dismiss_path = match row {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                self.get(node_index).map(|item| item.path().clone())
            },
            VisibleRow::Member { node_index, .. }
            | VisibleRow::Vendored { node_index, .. }
            | VisibleRow::Submodule { node_index, .. } => {
                self.get(node_index).map(|item| item.path().clone())
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            }
            | VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                ..
            }
            | VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                ..
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    if worktree_index == 0 {
                        Some(primary.path().clone())
                    } else {
                        linked.get(worktree_index - 1).map(|ws| ws.path().clone())
                    }
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    if worktree_index == 0 {
                        Some(primary.path().clone())
                    } else {
                        linked.get(worktree_index - 1).map(|pkg| pkg.path().clone())
                    }
                },
                _ => None,
            },
        }?;

        if self.is_deleted(&dismiss_path) {
            Some(DismissTarget::DeletedProject(dismiss_path))
        } else {
            None
        }
    }

    pub(super) fn worktree_parent_node_index(&self, path: &Path) -> Option<usize> {
        self.iter()
            .enumerate()
            .find_map(|(ni, item)| match &item.item {
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    let has_match =
                        primary.path() == path || linked.iter().any(|l| l.path() == path);
                    has_match.then_some(ni)
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    let has_match =
                        primary.path() == path || linked.iter().any(|l| l.path() == path);
                    has_match.then_some(ni)
                },
                _ => None,
            })
    }

    pub(super) fn row_matches_project_path(&self, row: VisibleRow, target_path: &Path) -> bool {
        self.path_for_row(row)
            .is_some_and(|path| path == target_path)
    }

    pub(super) const fn last_selected_path(&self) -> Option<&AbsolutePath> {
        self.paths().last_selected.as_ref()
    }

    pub(super) fn current_branch_for(&self, path: &Path) -> Option<&str> {
        self.git_info_for(path)?.branch.as_deref()
    }

    pub(super) fn ci_toggle_available_for_inner(&self, path: &Path) -> bool {
        self.current_branch_for(path).is_some()
    }

    pub(super) fn owner_repo_for_path_inner(&self, path: &Path) -> Option<OwnerRepo> {
        let entry_path = self.entry_containing(path)?.item.path().clone();
        self.primary_url_for(entry_path.as_path())
            .and_then(ci::parse_owner_repo)
    }

    // ── helper static methods for path / display_path / abs_path ─────

    pub(super) fn member_path_ref(
        item: &RootItem,
        group_index: usize,
        member_index: usize,
    ) -> Option<&Path> {
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                let group = ws.groups().get(group_index)?;
                let member = group.members().get(member_index)?;
                Some(member.path().as_path())
            },
            RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                if !wtg.renders_as_group() =>
            {
                let group = wtg.single_live_workspace()?.groups().get(group_index)?;
                let member = group.members().get(member_index)?;
                Some(member.path().as_path())
            },
            _ => None,
        }
    }

    pub(super) fn vendored_path_ref(item: &RootItem, vendored_index: usize) -> Option<&Path> {
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => ws
                .vendored()
                .get(vendored_index)
                .map(|p| p.path().as_path()),
            RootItem::Rust(RustProject::Package(pkg)) => pkg
                .vendored()
                .get(vendored_index)
                .map(|p| p.path().as_path()),
            RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. })
                if !wtg.renders_as_group() =>
            {
                wtg.single_live_workspace()?
                    .vendored()
                    .get(vendored_index)
                    .map(|p| p.path().as_path())
            },
            RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. })
                if !wtg.renders_as_group() =>
            {
                wtg.single_live_package()?
                    .vendored()
                    .get(vendored_index)
                    .map(|p| p.path().as_path())
            },
            _ => None,
        }
    }

    pub(super) fn worktree_display_path(item: &RootItem, wi: usize) -> Option<DisplayPath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.display_path())
                } else {
                    linked.get(wi - 1).map(ProjectFields::display_path)
                }
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.display_path())
                } else {
                    linked.get(wi - 1).map(ProjectFields::display_path)
                }
            },
            _ => None,
        }
    }

    pub(super) fn worktree_member_display_path(
        item: &RootItem,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<DisplayPath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                let group = ws.groups().get(gi)?;
                group.members().get(mi).map(ProjectFields::display_path)
            },
            _ => None,
        }
    }

    pub(super) fn worktree_vendored_display_path(
        item: &RootItem,
        wi: usize,
        vi: usize,
    ) -> Option<DisplayPath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                ws.vendored().get(vi).map(ProjectFields::display_path)
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                let pkg = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                pkg.vendored().get(vi).map(ProjectFields::display_path)
            },
            _ => None,
        }
    }

    pub(super) fn worktree_abs_path(item: &RootItem, wi: usize) -> Option<AbsolutePath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.path().clone())
                } else {
                    linked.get(wi - 1).map(|p| p.path().clone())
                }
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.path().clone())
                } else {
                    linked.get(wi - 1).map(|p| p.path().clone())
                }
            },
            _ => None,
        }
    }

    pub(super) fn worktree_member_abs_path(
        item: &RootItem,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<AbsolutePath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                let group = ws.groups().get(gi)?;
                group.members().get(mi).map(|p| p.path().clone())
            },
            _ => None,
        }
    }

    pub(super) fn worktree_vendored_abs_path(
        item: &RootItem,
        wi: usize,
        vi: usize,
    ) -> Option<AbsolutePath> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                ws.vendored().get(vi).map(|p| p.path().clone())
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                let pkg = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                pkg.vendored().get(vi).map(|p| p.path().clone())
            },
            _ => None,
        }
    }

    pub(super) fn worktree_path_ref(item: &RootItem, wi: usize) -> Option<&Path> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.path().as_path())
                } else {
                    linked.get(wi - 1).map(|p| p.path().as_path())
                }
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                if wi == 0 {
                    Some(primary.path().as_path())
                } else {
                    linked.get(wi - 1).map(|p| p.path().as_path())
                }
            },
            _ => None,
        }
    }

    pub(super) fn worktree_member_path_ref(
        item: &RootItem,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<&Path> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                let group = ws.groups().get(gi)?;
                group.members().get(mi).map(|p| p.path().as_path())
            },
            _ => None,
        }
    }

    pub(super) fn worktree_vendored_path_ref(
        item: &RootItem,
        wi: usize,
        vi: usize,
    ) -> Option<&Path> {
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                let ws = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                ws.vendored().get(vi).map(|p| p.path().as_path())
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                let pkg = if wi == 0 {
                    primary
                } else {
                    linked.get(wi - 1)?
                };
                pkg.vendored().get(vi).map(|p| p.path().as_path())
            },
            _ => None,
        }
    }
}
