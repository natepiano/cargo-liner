use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Index;
use std::path::Path;

use indexmap::IndexMap;
use indexmap::map::Values;
use indexmap::map::ValuesMut;

use super::app::CiRunDisplayMode;
use super::app::CleanSelection;
use super::app::FinderState;
use super::app::SelectionPaths;
use super::app::SelectionSync;
use super::columns::ProjectListWidths;
use super::state::Ci;
use super::state::CiStatusLookup;
use crate::ci;
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::ci::OwnerRepo;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::lint::LintRuns;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::Cargo;
use crate::project::CheckoutInfo;
use crate::project::DisplayPath;
use crate::project::GitHubInfo;
use crate::project::GitStatus;
use crate::project::LanguageStats;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::project::ProjectEntry;
use crate::project::ProjectFields;
use crate::project::ProjectInfo;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::RustInfo;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::Visibility;
use crate::project::WorkspaceMetadata;
use crate::project::WorktreeGroup;

mod grouping;
mod selection;
mod visible_rows;

use grouping::find_matching_worktree_container;
use grouping::linked_worktree_identity;
use grouping::regroup_workspace;
use grouping::shortest_unique_suffixes;
use grouping::try_attach_worktree;
use grouping::try_insert_member;
pub(super) use selection::SelectionMutation;
pub(super) use visible_rows::ExpandKey;
pub(super) use visible_rows::LegacyRootExpansion;
pub(super) use visible_rows::VisibleRow;
use visible_rows::worst_git_status;

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
    roots:                          IndexMap<AbsolutePath, ProjectEntry>,
    pub(super) paths:               SelectionPaths,
    sync:                           SelectionSync,
    pub(super) expanded:            HashSet<ExpandKey>,
    pub(super) finder:              FinderState,
    cached_visible_rows:            Vec<VisibleRow>,
    pub(super) cached_root_sorted:  Vec<u64>,
    pub(super) cached_child_sorted: HashMap<usize, Vec<u64>>,
    pub(super) cached_fit_widths:   ProjectListWidths,
    cursor:                         usize,
}

impl ProjectList {
    pub(super) fn new(items: Vec<RootItem>) -> Self {
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

    pub(super) fn len(&self) -> usize { self.roots.len() }

    pub(super) fn is_empty(&self) -> bool { self.roots.is_empty() }

    pub(super) fn iter(&self) -> Values<'_, AbsolutePath, ProjectEntry> { self.roots.values() }

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
    pub(super) fn first(&self) -> Option<&ProjectEntry> {
        self.roots.first().map(|(_, entry)| entry)
    }

    pub(super) fn get(&self, index: usize) -> Option<&ProjectEntry> {
        self.roots.get_index(index).map(|(_, entry)| entry)
    }

    pub(super) fn resolved_root_labels(&self, include_non_rust: bool) -> Vec<String> {
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

    pub(super) fn git_directories(&self) -> Vec<AbsolutePath> {
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
    pub(super) fn for_each_leaf(&self, mut f: impl FnMut(&ProjectEntry)) {
        for entry in self.roots.values() {
            match &entry.item {
                RootItem::Worktrees(group) => {
                    for project in group.iter_entries() {
                        f(&ProjectEntry::with_repo(
                            RootItem::Rust(project.clone()),
                            entry.git_repo.clone(),
                        ));
                    }
                },
                _ => f(entry),
            }
        }
    }

    /// Zero-allocation leaf path iteration. Yields `(path, is_rust)` for
    /// every leaf project without cloning any `RootItem`.
    pub(super) fn for_each_leaf_path(&self, mut f: impl FnMut(&Path, bool)) {
        for entry in self.roots.values() {
            match &entry.item {
                RootItem::Worktrees(group) => {
                    for project in group.iter_entries() {
                        f(project.path(), true);
                    }
                },
                other => f(other.path(), other.is_rust()),
            }
        }
    }

    pub(super) fn at_path(&self, target: &Path) -> Option<&ProjectInfo> {
        if let Some(entry) = self.roots.get(target) {
            return entry.item.at_path(target);
        }
        self.roots
            .values()
            .find_map(|entry| entry.item.at_path(target))
    }

    pub(super) fn at_path_mut(&mut self, target: &Path) -> Option<&mut ProjectInfo> {
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
    pub(super) fn is_submodule_path(&self, target: &Path) -> bool {
        self.roots.values().any(|entry| {
            entry
                .item
                .submodules()
                .iter()
                .any(|s| s.path.as_path() == target)
        })
    }

    pub(super) fn rust_info_at_path(&self, target: &Path) -> Option<&RustInfo> {
        self.roots
            .values()
            .find_map(|entry| entry.item.rust_info_at_path(target))
    }

    pub(super) fn rust_info_at_path_mut(&mut self, target: &Path) -> Option<&mut RustInfo> {
        self.roots
            .values_mut()
            .find_map(|entry| entry.item.rust_info_at_path_mut(target))
    }

    pub(super) fn vendored_at_path(&self, target: &Path) -> Option<&VendoredPackage> {
        self.roots
            .values()
            .find_map(|entry| entry.item.vendored_at_path(target))
    }

    pub(super) fn vendored_at_path_mut(&mut self, target: &Path) -> Option<&mut VendoredPackage> {
        self.roots
            .values_mut()
            .find_map(|entry| entry.item.vendored_at_path_mut(target))
    }

    /// For a vendored crate path, return the owning root's `LintRuns`.
    ///
    /// Used by the detail pane/icon to show parent lints when a vendored row
    /// is selected — the list-row icon stays blank because `lint_at_path`
    /// does not fall back.
    pub(super) fn vendored_owner_lint(&self, target: &Path) -> Option<&LintRuns> {
        self.roots
            .values()
            .find_map(|entry| entry.item.vendored_owner_lint(target))
    }

    pub(super) fn lint_at_path(&self, target: &Path) -> Option<&LintRuns> {
        self.roots
            .values()
            .find_map(|entry| entry.item.lint_at_path(target))
    }

    pub(super) fn lint_at_path_mut(&mut self, target: &Path) -> Option<&mut LintRuns> {
        self.roots
            .values_mut()
            .find_map(|entry| entry.item.lint_at_path_mut(target))
    }

    /// Top-level entry whose hierarchy contains `target`. One-shot
    /// replacement for the per-field per-path lookups used elsewhere.
    pub(super) fn entry_containing(&self, target: &Path) -> Option<&ProjectEntry> {
        self.roots
            .values()
            .find(|entry| project::entry_contains(entry, target))
    }

    pub(super) fn entry_containing_mut(&mut self, target: &Path) -> Option<&mut ProjectEntry> {
        self.roots
            .values_mut()
            .find(|entry| project::entry_contains(entry, target))
    }

    /// Replace `git_repo.ci_data` on the entry containing `path`.
    /// Silently no-ops when no entry contains `path` or the entry
    /// has no git repo.
    pub(super) fn replace_ci_data_for_path(&mut self, path: &Path, ci_data: ProjectCiData) {
        if let Some(repo) = self
            .entry_containing_mut(path)
            .and_then(|entry| entry.git_repo.as_mut())
        {
            repo.ci_data = ci_data;
        }
    }

    // -- Git/Repo reads --------------------------------------------------

    pub(super) fn git_info_for(&self, path: &Path) -> Option<&CheckoutInfo> {
        self.at_path(path)
            .and_then(|project| project.local_git_state.info())
    }

    /// Per-repo info (remotes, workflows, default branch, ...) for the
    /// entry containing `path`. `None` means either the path isn't in a
    /// known entry, the entry isn't in a git repo, or the background
    /// `LocalGitInfo::get` call hasn't completed yet.
    pub(super) fn repo_info_for(&self, path: &Path) -> Option<&RepoInfo> {
        self.entry_containing(path)
            .and_then(|entry| entry.git_repo.as_ref()?.repo_info.as_ref())
    }

    /// Convenience: the primary remote's URL for the checkout at `path`.
    pub(super) fn primary_url_for(&self, path: &Path) -> Option<&str> {
        let checkout = self.git_info_for(path)?;
        let repo = self.repo_info_for(path)?;
        checkout.primary_url(repo)
    }

    /// Convenience: the primary remote's ahead/behind for the checkout
    /// at `path`.
    pub(super) fn primary_ahead_behind_for(&self, path: &Path) -> Option<(usize, usize)> {
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
    pub(super) fn fetch_url_for(&self, path: &Path) -> Option<String> {
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

    pub(super) fn git_status_for(&self, path: &Path) -> Option<GitStatus> {
        self.git_info_for(path).map(|info| info.status)
    }

    /// Roll up the worst git path state across all **visible** children of a
    /// `RootItem`. For worktree groups, checks primary + non-dismissed linked
    /// entries. For everything else, returns the state for the single path.
    pub(super) fn git_status_for_item(&self, item: &RootItem) -> Option<GitStatus> {
        match item {
            RootItem::Worktrees(g) => worst_git_status(
                std::iter::once(self.git_status_for(g.primary.path())).chain(
                    g.linked
                        .iter()
                        .filter(|l| l.visibility() == Visibility::Visible)
                        .map(|l| self.git_status_for(l.path())),
                ),
            ),
            _ => self.git_status_for(item.path()),
        }
    }

    /// Formatted ahead/behind sync status for the project list columns.
    pub(super) fn git_sync(&self, path: &Path) -> String {
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

    pub(super) fn ci_data_for(&self, path: &Path) -> Option<&ProjectCiData> {
        self.entry_containing(path)
            .and_then(|entry| entry.git_repo.as_ref())
            .map(|repo| &repo.ci_data)
    }

    pub(super) fn ci_info_for(&self, path: &Path) -> Option<&ProjectCiInfo> {
        self.ci_data_for(path).and_then(ProjectCiData::info)
    }

    /// Branch name for a checkout whose CI cannot be inferred from the
    /// parent repo's default-branch runs: an unpushed (no-upstream) branch
    /// that also isn't the default. Used to suppress stale parent-repo CI
    /// status for unpublished worktree branches.
    pub(super) fn unpublished_ci_branch_name(&self, path: &Path) -> Option<String> {
        let git = self.git_info_for(path)?;
        let default_branch = self
            .repo_info_for(path)
            .and_then(|repo| repo.default_branch.as_deref());
        (git.primary_tracked_ref().is_none() && git.branch.as_deref() != default_branch)
            .then(|| git.branch.clone())
            .flatten()
    }

    /// Latest CI status at `path`, with display-mode resolved through
    /// the render-time [`CiStatusLookup`] snapshot. Suppressed for
    /// unpublished worktree branches whose parent-repo CI doesn't
    /// apply.
    pub(super) fn ci_status_using_lookup(
        &self,
        path: &Path,
        lookup: &CiStatusLookup,
    ) -> Option<CiStatus> {
        if self.unpublished_ci_branch_name(path).is_some() {
            return None;
        }
        let display_mode = lookup.display_mode_for(path);
        let info = self.ci_info_for(path)?;
        let runs = info.runs.as_slice();
        let latest = match self.current_branch_for(path) {
            None => runs.first(),
            Some(_) if display_mode == CiRunDisplayMode::All => runs.first(),
            Some(branch) => runs.iter().find(|run| run.branch == branch),
        };
        latest.map(|run| run.ci_status)
    }

    /// Latest CI status for a `RootItem`, dispatching to either the path-keyed
    /// lookup or `RootItem::ci_status`'s aggregator (for `WorktreeGroup`,
    /// which spans multiple checkouts and therefore can't be addressed by a
    /// single path).
    pub(super) fn ci_status_for_root_item_using_lookup(
        &self,
        item: &RootItem,
        lookup: &CiStatusLookup,
    ) -> Option<CiStatus> {
        item.ci_status(|p| self.ci_status_using_lookup(p, lookup))
    }

    pub(super) fn is_deleted(&self, path: &Path) -> bool {
        self.at_path(path)
            .is_some_and(|project| project.visibility == Visibility::Deleted)
    }

    pub(super) fn is_rust_at_path(&self, path: &Path) -> bool {
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

    pub(super) fn is_vendored_path(&self, path: &Path) -> bool {
        self.iter().any(|item| match &item.item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.vendored().iter().any(|v| v.path() == path)
            },
            RootItem::Rust(RustProject::Package(pkg)) => {
                pkg.vendored().iter().any(|v| v.path() == path)
            },
            RootItem::Worktrees(group) => group.iter_entries().any(|entry| {
                entry
                    .rust_info()
                    .vendored()
                    .iter()
                    .any(|v| v.path() == path)
            }),
            RootItem::NonRust(_) => false,
        })
    }

    pub(super) fn is_workspace_member_path(&self, path: &Path) -> bool {
        self.iter().any(|item| match &item.item {
            RootItem::Rust(RustProject::Workspace(ws)) => ws
                .groups()
                .iter()
                .any(|g| g.members().iter().any(|m| m.path() == path)),
            RootItem::Worktrees(group) => group.iter_entries().any(|entry| {
                if let RustProject::Workspace(ws) = entry {
                    ws.groups()
                        .iter()
                        .any(|g| g.members().iter().any(|m| m.path() == path))
                } else {
                    false
                }
            }),
            _ => false,
        })
    }

    pub(super) fn git_main(&self, path: &Path) -> String {
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
    pub(super) fn replace_leaf_by_path(
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
                RootItem::Worktrees(group) => {
                    if group.primary.path() == path
                        && let RootItem::Rust(rp) = replacement
                    {
                        let old = std::mem::replace(&mut group.primary, rp);
                        return Some(RootItem::Rust(old));
                    }
                    for l in &mut group.linked {
                        if l.path() == path
                            && let RootItem::Rust(rp) = replacement
                        {
                            let old = std::mem::replace(l, rp);
                            return Some(RootItem::Rust(old));
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
    #[expect(
        dead_code,
        reason = "kept for use by upcoming worktree promotion sites"
    )]
    pub(super) fn promote_to_worktree_group(&mut self, path: &Path, group: WorktreeGroup) -> bool {
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
    pub(super) fn insert_into_hierarchy(&mut self, item: RootItem) -> bool {
        let item_path = item.path().to_path_buf();
        for entry in self.roots.values_mut() {
            if try_attach_worktree(&mut entry.item, &item) {
                return false;
            }

            let inserted = match &mut entry.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    try_insert_member(ws, &item_path, &item)
                },
                RootItem::Worktrees(group) => std::iter::once(&mut group.primary)
                    .chain(group.linked.iter_mut())
                    .any(|entry| {
                        if let RustProject::Workspace(ws) = entry {
                            try_insert_member(ws, &item_path, &item)
                        } else {
                            false
                        }
                    }),
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
    pub(super) fn regroup_members(&mut self, inline_dirs: &[String]) {
        for entry in self.roots.values_mut() {
            match &mut entry.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    regroup_workspace(ws, inline_dirs);
                },
                RootItem::Worktrees(group) => {
                    for entry in std::iter::once(&mut group.primary).chain(group.linked.iter_mut())
                    {
                        if let RustProject::Workspace(ws) = entry {
                            regroup_workspace(ws, inline_dirs);
                        }
                    }
                },
                _ => {},
            }
        }
    }

    pub(super) fn regroup_top_level_worktrees(&mut self) {
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

    pub(super) fn clear(&mut self) { self.roots.clear(); }

    #[cfg(test)]
    pub(super) fn push(&mut self, item: RootItem) {
        let key = item.path().clone();
        self.roots.insert(key, ProjectEntry::new(item));
    }
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

    // ── sync flag ───────────────────────────────────────────────────

    pub(super) const fn sync(&self) -> SelectionSync { self.sync }

    pub(super) const fn mark_sync_changed(&mut self) { self.sync = SelectionSync::Changed; }

    pub(super) const fn mark_sync_stable(&mut self) { self.sync = SelectionSync::Stable; }

    // ── expansion set ───────────────────────────────────────────────

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

    pub(super) fn set_disk_caches(
        &mut self,
        root_sorted: Vec<u64>,
        child_sorted: HashMap<usize, Vec<u64>>,
    ) {
        self.cached_root_sorted = root_sorted;
        self.cached_child_sorted = child_sorted;
    }

    // ── fit widths ──────────────────────────────────────────────────

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

// ── Row-navigation read-side ─────────────────────────────────────────────
//
// Pure ProjectList queries: row → path resolution, expand-key lookup,
// dismiss-target lookup, CI/branch lookups that don't cross into Ci/panes
// state.
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
            } => self
                .get(node_index)?
                .item
                .member_path_ref(group_index, member_index),
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => self.get(node_index)?.item.vendored_path_ref(vendored_index),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(wtg) => wtg.worktree_path_ref(worktree_index),
                _ => None,
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(wtg) => {
                    wtg.worktree_member_path_ref(worktree_index, group_index, member_index)
                },
                _ => None,
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(wtg) => {
                    wtg.worktree_vendored_path_ref(worktree_index, vendored_index)
                },
                _ => None,
            },
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
            } => self
                .get(node_index)?
                .item
                .resolve_member(group_index, member_index)
                .map(ProjectFields::display_path),
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => self
                .get(node_index)?
                .item
                .resolve_vendored(vendored_index)
                .map(ProjectFields::display_path),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(wtg) => wtg.worktree_display_path(worktree_index),
                _ => None,
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(wtg) => {
                    wtg.worktree_member_display_path(worktree_index, group_index, member_index)
                },
                _ => None,
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(wtg) => {
                    wtg.worktree_vendored_display_path(worktree_index, vendored_index)
                },
                _ => None,
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
            } => self
                .get(node_index)?
                .item
                .resolve_member(group_index, member_index)
                .map(|p| p.path().clone()),
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => self
                .get(node_index)?
                .item
                .resolve_vendored(vendored_index)
                .map(|p| p.path().clone()),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(wtg) => wtg.worktree_abs_path(worktree_index),
                _ => None,
            },
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(wtg) => {
                    wtg.worktree_member_abs_path(worktree_index, group_index, member_index)
                },
                _ => None,
            },
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(wtg) => {
                    wtg.worktree_vendored_abs_path(worktree_index, vendored_index)
                },
                _ => None,
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
                    RootItem::Worktrees(group) => match group.entry(worktree_index)? {
                        RustProject::Workspace(ws) => ws
                            .has_members()
                            .then_some(ExpandKey::Worktree(node_index, worktree_index)),
                        RustProject::Package(_) => None,
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

    pub(super) fn try_collapse(&mut self, key: &ExpandKey) -> bool { self.expanded.remove(key) }

    pub(super) fn dismiss_target_for_row_inner(
        &self,
        row: VisibleRow,
    ) -> Option<crate::tui::pane::DismissTarget> {
        use super::pane::DismissTarget;
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
                RootItem::Worktrees(group) => group.entry(worktree_index).map(|p| p.path().clone()),
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
                RootItem::Worktrees(group) => {
                    group.iter_entries().any(|p| p.path() == path).then_some(ni)
                },
                _ => None,
            })
    }

    pub(super) fn row_matches_project_path(&self, row: VisibleRow, target_path: &Path) -> bool {
        self.path_for_row(row)
            .is_some_and(|path| path == target_path)
    }

    pub(super) const fn last_selected_path(&self) -> Option<&AbsolutePath> {
        self.paths.last_selected.as_ref()
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
}

impl ProjectList {
    /// Expand every node and named group, restoring selection after recompute.
    pub(super) fn expand_all(&mut self, include_non_rust: bool) {
        let selected_path = self
            .paths
            .collapsed_selected
            .take()
            .or_else(|| self.selected_project_path().map(AbsolutePath::from));
        self.paths.collapsed_anchor = None;
        let (roots, expanded) = self.iter_with_expanded_mut();
        for (ni, entry) in roots.enumerate() {
            if entry.item.has_children() {
                expanded.insert(ExpandKey::Node(ni));
            }
            match &entry.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    for (gi, group) in ws.groups().iter().enumerate() {
                        if group.is_named() {
                            expanded.insert(ExpandKey::Group(ni, gi));
                        }
                    }
                },
                RootItem::Worktrees(group) => {
                    for (wi, entry) in group.iter_entries().enumerate() {
                        if let RustProject::Workspace(ws) = entry {
                            if ws.has_members() {
                                expanded.insert(ExpandKey::Worktree(ni, wi));
                            }
                            for (gi, g) in ws.groups().iter().enumerate() {
                                if g.is_named() {
                                    expanded.insert(ExpandKey::WorktreeGroup(ni, wi, gi));
                                }
                            }
                        }
                    }
                },
                _ => {},
            }
        }
        if let Some(path) = selected_path {
            self.select_project_in_tree(path.as_path(), include_non_rust);
        }
    }

    /// Clear all expansions, then recompute and restore selection.
    pub(super) fn collapse_all(&mut self, include_non_rust: bool) {
        let selected_path = self.selected_project_path().map(AbsolutePath::from);
        let anchor = self.selected_row().map(VisibleRow::collapse_anchor);
        self.expanded.clear();
        self.recompute_visibility(include_non_rust);
        if let Some(anchor) = anchor
            && let Some(pos) = self.visible_rows().iter().position(|row| *row == anchor)
        {
            self.set_cursor(pos);
        }
        let anchor_path = self.selected_project_path().map(AbsolutePath::from);
        if selected_path == anchor_path {
            self.paths.collapsed_selected = None;
            self.paths.collapsed_anchor = None;
        } else {
            self.paths.collapsed_selected = selected_path;
            self.paths.collapsed_anchor = anchor_path;
        }
    }

    /// Mark all nodes containing `target_path` as expanded.
    pub(super) fn expand_path_in_tree(&mut self, target_path: &Path) {
        let (roots, expanded) = self.iter_with_expanded_mut();
        for (ni, entry) in roots.enumerate() {
            match &entry.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    for (gi, group) in ws.groups().iter().enumerate() {
                        for member in group.members() {
                            if member.path() == target_path {
                                expanded.insert(ExpandKey::Node(ni));
                                if group.is_named() {
                                    expanded.insert(ExpandKey::Group(ni, gi));
                                }
                            }
                        }
                    }
                    for vendored in ws.vendored() {
                        if vendored.path() == target_path {
                            expanded.insert(ExpandKey::Node(ni));
                        }
                    }
                },
                RootItem::Rust(RustProject::Package(pkg)) => {
                    for vendored in pkg.vendored() {
                        if vendored.path() == target_path {
                            expanded.insert(ExpandKey::Node(ni));
                        }
                    }
                },
                RootItem::NonRust(_) => {},
                RootItem::Worktrees(group) => {
                    for (wi, entry) in group.iter_entries().enumerate() {
                        if entry.path() == target_path {
                            expanded.insert(ExpandKey::Node(ni));
                        }
                        if let RustProject::Workspace(ws) = entry {
                            for (gi, g) in ws.groups().iter().enumerate() {
                                for member in g.members() {
                                    if member.path() == target_path {
                                        expanded.insert(ExpandKey::Node(ni));
                                        expanded.insert(ExpandKey::Worktree(ni, wi));
                                        if g.is_named() {
                                            expanded.insert(ExpandKey::WorktreeGroup(ni, wi, gi));
                                        }
                                    }
                                }
                            }
                        }
                        for vendored in entry.rust_info().vendored() {
                            if vendored.path() == target_path {
                                expanded.insert(ExpandKey::Node(ni));
                                expanded.insert(ExpandKey::Worktree(ni, wi));
                            }
                        }
                    }
                },
            }
        }
    }

    /// After expanding a path, find the visible row matching it and put
    /// the cursor on it (no-op if none match).
    pub(super) fn select_matching_visible_row(
        &mut self,
        target_path: &Path,
        include_non_rust: bool,
    ) {
        self.recompute_visibility(include_non_rust);
        let selected_index = self
            .visible_rows()
            .iter()
            .position(|row| self.row_matches_project_path(*row, target_path));
        if let Some(selected_index) = selected_index {
            self.set_cursor(selected_index);
        }
    }

    /// Composes `expand_path_in_tree` + `select_matching_visible_row`.
    pub(super) fn select_project_in_tree(&mut self, target_path: &Path, include_non_rust: bool) {
        self.expand_path_in_tree(target_path);
        self.select_matching_visible_row(target_path, include_non_rust);
    }

    /// Remove `key` from expanded, recompute rows, and move cursor to
    /// `target` if it appears in the new visible row set.
    pub(super) fn collapse_to(
        &mut self,
        key: &ExpandKey,
        target: VisibleRow,
        include_non_rust: bool,
    ) {
        self.expanded.remove(key);
        self.recompute_visibility(include_non_rust);
        if let Some(pos) = self.visible_rows().iter().position(|r| *r == target) {
            self.set_cursor(pos);
        }
    }

    /// Collapse the row at `row` to its nearest ancestor anchor.
    pub(super) fn collapse_row(&mut self, row: VisibleRow, include_non_rust: bool) {
        match row {
            VisibleRow::Root { node_index: ni } => {
                self.try_collapse(&ExpandKey::Node(ni));
            },
            VisibleRow::GroupHeader {
                node_index: ni,
                group_index: gi,
            } => {
                if !self.try_collapse(&ExpandKey::Group(ni, gi)) {
                    self.collapse_to_root(ni, include_non_rust);
                }
            },
            VisibleRow::Member {
                node_index: ni,
                group_index: gi,
                ..
            } => {
                if self.is_inline_group(ni, gi) {
                    self.collapse_to_root(ni, include_non_rust);
                } else {
                    self.collapse_to(
                        &ExpandKey::Group(ni, gi),
                        VisibleRow::GroupHeader {
                            node_index:  ni,
                            group_index: gi,
                        },
                        include_non_rust,
                    );
                }
            },
            VisibleRow::Vendored { node_index: ni, .. }
            | VisibleRow::Submodule { node_index: ni, .. } => {
                self.collapse_to_root(ni, include_non_rust);
            },
            VisibleRow::WorktreeEntry {
                node_index: ni,
                worktree_index: wi,
            } => {
                if !self.try_collapse(&ExpandKey::Worktree(ni, wi)) {
                    self.collapse_to_root(ni, include_non_rust);
                }
            },
            VisibleRow::WorktreeGroupHeader {
                node_index: ni,
                worktree_index: wi,
                group_index: gi,
            } => {
                if !self.try_collapse(&ExpandKey::WorktreeGroup(ni, wi, gi)) {
                    self.collapse_to_worktree_entry(ni, wi, include_non_rust);
                }
            },
            VisibleRow::WorktreeMember {
                node_index: ni,
                worktree_index: wi,
                group_index: gi,
                ..
            } => {
                if self.is_worktree_inline_group(ni, wi, gi) {
                    self.collapse_to_worktree_entry(ni, wi, include_non_rust);
                } else {
                    self.collapse_to(
                        &ExpandKey::WorktreeGroup(ni, wi, gi),
                        VisibleRow::WorktreeGroupHeader {
                            node_index:     ni,
                            worktree_index: wi,
                            group_index:    gi,
                        },
                        include_non_rust,
                    );
                }
            },
            VisibleRow::WorktreeVendored {
                node_index: ni,
                worktree_index: wi,
                ..
            } => {
                self.collapse_to_worktree_entry(ni, wi, include_non_rust);
            },
        }
    }

    fn collapse_to_root(&mut self, ni: usize, include_non_rust: bool) {
        self.collapse_to(
            &ExpandKey::Node(ni),
            VisibleRow::Root { node_index: ni },
            include_non_rust,
        );
    }

    fn collapse_to_worktree_entry(&mut self, ni: usize, wi: usize, include_non_rust: bool) {
        self.collapse_to(
            &ExpandKey::Worktree(ni, wi),
            VisibleRow::WorktreeEntry {
                node_index:     ni,
                worktree_index: wi,
            },
            include_non_rust,
        );
    }

    /// Public collapse entry point. Returns whether anything visibly changed.
    pub(super) fn collapse(&mut self, include_non_rust: bool) -> bool {
        let selected = self.cursor();
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return false;
        };
        let expanded_before = self.expanded.len();
        let selected_before = self.cursor();
        self.collapse_row(row, include_non_rust);
        self.expanded.len() != expanded_before || self.cursor() != selected_before
    }

    /// Whether the group at `(ni, gi)` is an inline (unnamed) group.
    pub(super) fn is_inline_group(&self, ni: usize, gi: usize) -> bool {
        let Some(item) = self.get(ni) else {
            return true;
        };
        match &item.item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.groups().get(gi).is_some_and(|g| !g.is_named())
            },
            _ => true,
        }
    }

    /// Whether the worktree group at `(ni, wi, gi)` is an inline (unnamed) group.
    pub(super) fn is_worktree_inline_group(&self, ni: usize, wi: usize, gi: usize) -> bool {
        let Some(item) = self.get(ni) else {
            return true;
        };
        match &item.item {
            RootItem::Worktrees(group) => match group.entry(wi) {
                Some(RustProject::Workspace(ws)) => {
                    ws.groups().get(gi).is_some_and(|g| !g.is_named())
                },
                _ => true,
            },
            _ => true,
        }
    }

    /// Map the currently selected row to a [`CleanSelection`] when the
    /// Clean shortcut should be enabled on it.
    pub(super) fn clean_selection(&self) -> Option<CleanSelection> {
        let row = self.selected_row()?;
        match row {
            VisibleRow::Root { node_index } => {
                let entry = self.get(node_index)?;
                match &entry.item {
                    RootItem::Rust(rust) => Some(CleanSelection::Project {
                        root: rust.path().clone(),
                    }),
                    RootItem::Worktrees(group) => Some(worktree_group_selection(group)),
                    RootItem::NonRust(_) => None,
                }
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => match &self.get(node_index)?.item {
                RootItem::Worktrees(wtg) => {
                    wtg.worktree_path_ref(worktree_index)
                        .map(|path| CleanSelection::Project {
                            root: AbsolutePath::from(path),
                        })
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// Move the cursor to the `Root` row for `node_index`, if visible.
    pub(super) fn select_root_row(&mut self, node_index: usize) {
        if let Some(pos) = self
            .visible_rows()
            .iter()
            .position(|row| matches!(row, VisibleRow::Root { node_index: ni } if *ni == node_index))
        {
            self.set_cursor(pos);
        }
    }

    /// Snapshot expansion of every top-level node so a tree rebuild can
    /// re-apply the same logical expansions to a re-indexed layout.
    pub(super) fn capture_legacy_root_expansions(&self) -> Vec<LegacyRootExpansion> {
        self.iter()
            .enumerate()
            .filter_map(|(ni, entry)| {
                if !self.expanded.contains(&ExpandKey::Node(ni)) {
                    return None;
                }
                match &entry.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => Some(LegacyRootExpansion {
                        root_path:      ws.path().clone(),
                        old_node_index: ni,
                        had_children:   ws.has_members() || !ws.vendored().is_empty(),
                        named_groups:   ws
                            .groups()
                            .iter()
                            .enumerate()
                            .filter_map(|(gi, group)| {
                                group
                                    .is_named()
                                    .then(|| self.expanded.contains(&ExpandKey::Group(ni, gi)))
                                    .filter(|expanded| *expanded)
                                    .map(|_| gi)
                            })
                            .collect(),
                    }),
                    RootItem::Rust(RustProject::Package(pkg)) => Some(LegacyRootExpansion {
                        root_path:      pkg.path().clone(),
                        old_node_index: ni,
                        had_children:   !pkg.vendored().is_empty(),
                        named_groups:   Vec::new(),
                    }),
                    _ => None,
                }
            })
            .collect()
    }

    /// Re-apply expansions captured by `capture_legacy_root_expansions`,
    /// adapting old node indices to the post-rebuild layout.
    pub(super) fn migrate_legacy_root_expansions(&mut self, legacy: &[LegacyRootExpansion]) {
        let (roots, expanded) = self.iter_with_expanded_mut();
        let entries: Vec<(usize, &RootItem)> = roots
            .enumerate()
            .map(|(idx, entry)| (idx, &entry.item))
            .collect();
        for legacy_root in legacy {
            let Some((current_index, item)) = entries
                .iter()
                .find(|(_, item)| item.path() == legacy_root.root_path.as_path())
                .map(|(idx, item)| (*idx, *item))
            else {
                continue;
            };
            match item {
                RootItem::Worktrees(group) if group.renders_as_group() => {
                    expanded.insert(ExpandKey::Node(current_index));
                    if legacy_root.had_children {
                        expanded.insert(ExpandKey::Worktree(current_index, 0));
                    }
                    if let RustProject::Workspace(ws) = &group.primary {
                        for &group_index in &legacy_root.named_groups {
                            if ws.groups().get(group_index).is_some() {
                                expanded.insert(ExpandKey::WorktreeGroup(
                                    current_index,
                                    0,
                                    group_index,
                                ));
                            }
                            expanded
                                .remove(&ExpandKey::Group(legacy_root.old_node_index, group_index));
                        }
                    }
                },
                _ => {},
            }
        }
    }

    /// Stamp each [`PackageRecord`]'s derived [`Cargo`] fields onto the
    /// matching package / workspace member / vendored package.
    pub(super) fn apply_cargo_fields_from_workspace_metadata(
        &mut self,
        metadata: &WorkspaceMetadata,
    ) {
        for record in metadata.packages.values() {
            let Some(manifest_dir) = record.manifest_path.as_path().parent() else {
                continue;
            };
            let cargo = Cargo::from_package_record(record);
            if let Some(rust_info) = self.rust_info_at_path_mut(manifest_dir) {
                rust_info.cargo = cargo.clone();
            }
            if let Some(vendored) = self.vendored_at_path_mut(manifest_dir) {
                vendored.cargo = cargo;
            }
        }
    }

    /// Apply a batch of `LanguageStats` to matching projects.
    pub(super) fn handle_language_stats_batch(
        &mut self,
        entries: Vec<(AbsolutePath, LanguageStats)>,
    ) {
        for (path, stats) in entries {
            if let Some(project) = self.at_path_mut(path.as_path()) {
                project.language_stats = Some(stats);
            }
        }
    }

    /// Stamp crates.io version+downloads onto the matching project.
    pub(super) fn handle_crates_io_version_msg(
        &mut self,
        path: &Path,
        version: String,
        downloads: u64,
    ) {
        if let Some(rust_info) = self.rust_info_at_path_mut(path) {
            rust_info.set_crates_io(version, downloads);
        } else if let Some(vendored) = self.vendored_at_path_mut(path) {
            vendored.set_crates_io(version, downloads);
        }
    }

    pub(super) fn handle_repo_meta(
        &mut self,
        path: &Path,
        stars: u64,
        description: Option<String>,
    ) {
        if let Some(entry) = self.entry_containing_mut(path) {
            let repo = entry.git_repo.get_or_insert_with(Default::default);
            repo.github_info = Some(GitHubInfo { stars, description });
        }
    }

    /// Collect root project paths and metadata for the lint runtime.
    pub(super) fn lint_runtime_root_entries(&self) -> Vec<(AbsolutePath, bool)> {
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        for entry in self {
            let items: Vec<(&AbsolutePath, bool)> = match &entry.item {
                RootItem::Worktrees(group) => {
                    group.iter_entries().map(|p| (p.path(), true)).collect()
                },
                _ => vec![(entry.item.path(), entry.item.is_rust())],
            };
            for (path, is_rust) in items {
                let owned = path.clone();
                if seen.insert(owned.clone()) {
                    entries.push((owned, is_rust));
                }
            }
        }
        entries
    }

    /// Whether any project in the current snapshot is non-Rust.
    pub(super) fn has_cached_non_rust_projects(&self) -> bool {
        let mut found = false;
        self.for_each_leaf(|item| {
            if !item.is_rust() {
                found = true;
            }
        });
        found
    }

    /// Whether the currently-selected project's path has been dismissed.
    pub(super) fn selected_project_is_deleted(&self) -> bool {
        self.selected_project_path()
            .is_some_and(|path| self.is_deleted(path))
    }

    /// Resolve the current selection to the absolute path of its
    /// containing root entry (Workspace, Package, or worktree primary).
    pub(super) fn selected_ci_path(&self) -> Option<AbsolutePath> {
        let path = self.selected_project_path()?;
        let entry = self.entry_containing(path)?;
        Some(entry.item.path().clone())
    }

    /// CI runs at `path`, with display-mode resolved against `ci`.
    /// Consumed by the CI-runs detail pane.
    pub(super) fn ci_runs_for_ci_pane(&self, path: &Path, ci: &Ci) -> Vec<CiRun> {
        let Some(info) = self.ci_info_for(path) else {
            return Vec::new();
        };
        let Some(branch) = self.current_branch_for(path) else {
            return info.runs.clone();
        };
        if ci.display_mode_for(path) == CiRunDisplayMode::All {
            return info.runs.clone();
        }
        info.runs
            .iter()
            .filter(|run| run.branch == branch)
            .cloned()
            .collect()
    }
}

/// Build a `CleanSelection::WorktreeGroup` from a live `WorktreeGroup`.
fn worktree_group_selection(group: &WorktreeGroup) -> CleanSelection {
    CleanSelection::WorktreeGroup {
        primary: group.primary.path().clone(),
        linked:  group.linked.iter().map(|p| p.path().clone()).collect(),
    }
}
