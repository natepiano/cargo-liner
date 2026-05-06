use std::collections::HashSet;
use std::path::Path;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::ci;
use crate::ci::OwnerRepo;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::GitHubInfo;
use crate::project::LocalGitState;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::CachedRepoData;
use crate::scan::CiFetchResult;
use crate::tui::app::App;
use crate::tui::constants::STARTUP_PHASE_GITHUB;
use crate::tui::toasts::TrackedItem;

impl App {
    pub(super) fn spawn_repo_fetch_for_git_info(&mut self, path: &Path, repo_url: &str) {
        let Some(owner_repo) = ci::parse_owner_repo(repo_url) else {
            return;
        };
        // Dedup by `OwnerRepo`: a fetch for this repo is either already
        // running or queued. The `RepoFetchComplete` background message
        // removes the entry, so a later spawn after completion is not
        // blocked.
        if !self
            .net
            .github_mut()
            .repo_fetch_in_flight_mut()
            .insert(owner_repo.clone())
        {
            return;
        }

        let tx = self.background.bg_sender();
        let client = self.net.http_client();
        let repo_cache = self.net.github().fetch_cache().clone();
        let path: AbsolutePath = AbsolutePath::from(path);
        let repo_url = repo_url.to_string();
        let ci_run_count = self.config.ci_run_count();
        thread::spawn(move || {
            let data = scan::load_cached_repo_data(&repo_cache, &owner_repo).unwrap_or_else(|| {
                let _ = tx.send(BackgroundMsg::RepoFetchQueued {
                    repo: owner_repo.clone(),
                });
                let (result, meta, signal) = scan::fetch_ci_runs_cached(
                    &client,
                    &repo_url,
                    owner_repo.owner(),
                    owner_repo.repo(),
                    ci_run_count,
                );
                scan::emit_service_signal(&tx, signal);
                let (runs, github_total) = match result {
                    CiFetchResult::Loaded { runs, github_total } => (runs, github_total),
                    CiFetchResult::CacheOnly(runs) => (runs, 0),
                };
                let data = CachedRepoData {
                    runs,
                    meta,
                    github_total,
                };
                scan::store_cached_repo_data(&repo_cache, &owner_repo, data.clone());
                data
            });

            let _ = tx.send(BackgroundMsg::CiRuns {
                path:         path.clone(),
                runs:         data.runs,
                github_total: data.github_total,
            });
            if let Some(meta) = data.meta {
                let _ = tx.send(BackgroundMsg::RepoMeta {
                    path,
                    stars: meta.stars,
                    description: meta.description,
                });
            }
            // Fire `RepoFetchComplete` from the always-runs tail so the
            // dedup set clears on cache hits too. The startup toast
            // handler is a no-op for repos that were never queued.
            let _ = tx.send(BackgroundMsg::RepoFetchComplete { repo: owner_repo });
        });
    }
    /// Handle a per-checkout git state update. Writes to the
    /// `ProjectInfo.local_git_state` for `path`, runs startup tracking
    /// hooks, and triggers a repo-level fetch if applicable. The repo
    /// fetch trigger is here because either a `RepoInfo` or
    /// `CheckoutInfo` arrival can signal "this repo's state changed";
    /// the dedup set absorbs N attempts for the same `OwnerRepo`.
    pub fn handle_checkout_info(&mut self, path: &Path, info: CheckoutInfo) {
        tracing::info!(
            path = %path.display(),
            git_status = %info.status.label(),
            "checkout_info_applied"
        );

        if let Some(project) = self.project_list.at_path_mut(path) {
            project.local_git_state = LocalGitState::Detected(Box::new(info));
        }
        // Detected git state implies the entry is in a git repo. Ensure
        // the entry has a `git_repo` slot so per-repo writes (CI,
        // GitHub meta, RepoInfo) can land on it.
        if let Some(entry) = self.project_list.entry_containing_mut(path) {
            entry.git_repo.get_or_insert_with(Default::default);
        }

        if self.scan.is_complete() {
            let git_dir = self
                .startup_git_directory_for_path(path)
                .unwrap_or_else(|| AbsolutePath::from(path));
            self.startup.git.seen.insert(git_dir.clone());
            if let Some(git_toast) = self.startup.git.toast {
                self.mark_tracked_item_completed(git_toast, &git_dir.to_string());
            }
            self.maybe_log_startup_phase_completions();
        }

        self.maybe_trigger_repo_fetch(path);
    }
    /// Handle a per-repo git state update. Only the primary checkout
    /// writes `RepoInfo` (linked worktrees share the primary's
    /// `.git/config` by design; admitting last-writer-wins from any
    /// checkout would produce silent arbitration if they ever
    /// diverged). The `path` is the primary's path — the emitter is
    /// responsible for that contract.
    pub fn handle_repo_info(&mut self, path: &Path, mut info: RepoInfo) {
        // Preserve a previously-fetched `first_commit` across refresh.
        // `RepoInfo::get` always returns `None` for it; the value is
        // filled in either by a prior `handle_git_first_commit` write
        // or via the `pending_git_first_commit` map below.
        let preserved_first_commit = self
            .project_list
            .repo_info_for(path)
            .and_then(|existing| existing.first_commit.clone());
        if info.first_commit.is_none() {
            info.first_commit = preserved_first_commit
                .or_else(|| self.scan.pending_git_first_commit_mut().remove(path));
        }

        // Gate GitHub cache invalidation on `FETCH_HEAD` mtime actually
        // advancing. Without this, every watcher tick / commit / branch
        // switch would invalidate the cache and trigger a refetch,
        // burning REST quota. ISO 8601 strings compare lexically in
        // chronological order, so `!=` captures advance reliably.
        let previous_last_fetched = self
            .project_list
            .repo_info_for(path)
            .and_then(|existing| existing.last_fetched.clone());
        let fetch_head_advanced =
            info.last_fetched.is_some() && info.last_fetched != previous_last_fetched;

        if let Some(entry) = self.project_list.entry_containing_mut(path) {
            if entry.item.path().as_path() != path {
                // Non-primary write — discard per the policy above.
                return;
            }
            let git_repo = entry.git_repo.get_or_insert_with(Default::default);
            git_repo.repo_info = Some(info);
        }

        if fetch_head_advanced
            && self.scan.is_complete()
            && let Some(url) = self.project_list.fetch_url_for(path)
            && let Some(owner_repo) = ci::parse_owner_repo(&url)
            && !self.net.github().contains_in_flight(&owner_repo)
        {
            scan::invalidate_cached_repo_data(self.net.github().fetch_cache(), &owner_repo);
        }

        self.maybe_trigger_repo_fetch(path);
    }
    /// Shared between `handle_repo_info` and `handle_checkout_info`:
    /// kick a GitHub fetch for this path's repo if we have a parseable
    /// remote URL. The dedup set absorbs concurrent attempts for the
    /// same `OwnerRepo`; cache invalidation is gated inside
    /// `handle_repo_info` by `last_fetched` advance. Submodule paths are
    /// excluded — submodule CI/metadata is shown on the parent project.
    pub(super) fn maybe_trigger_repo_fetch(&mut self, path: &Path) {
        if self.project_list.is_submodule_path(path) {
            return;
        }
        let Some(url) = self.project_list.fetch_url_for(path) else {
            return;
        };
        self.spawn_repo_fetch_for_git_info(path, &url);
    }
    pub(super) fn handle_git_first_commit(&mut self, path: &Path, first_commit: Option<&str>) {
        let first_commit = first_commit.map(String::from);
        // first_commit is per-repo, so it lands on the entry's
        // `RepoInfo`. If the entry's `repo_info` slot doesn't exist yet
        // (`RepoInfo::get` hasn't completed), stash the value in
        // `pending_git_first_commit` and `handle_repo_info` will fold
        // it in when repo info arrives.
        let applied = self
            .project_list
            .entry_containing_mut(path)
            .and_then(|entry| entry.git_repo.as_mut()?.repo_info.as_mut())
            .map(|repo| repo.first_commit.clone_from(&first_commit))
            .is_some();
        if applied {
            self.scan.pending_git_first_commit_mut().remove(path);
        } else if let Some(first_commit) = first_commit {
            self.scan
                .pending_git_first_commit_mut()
                .insert(AbsolutePath::from(path), first_commit);
        } else {
            self.scan.pending_git_first_commit_mut().remove(path);
        }
    }
    pub(super) fn handle_repo_fetch_queued(&mut self, repo: OwnerRepo) {
        let first_repo = self
            .startup
            .repo
            .expected
            .as_ref()
            .is_none_or(HashSet::is_empty);
        self.startup.repo.ensure_expected().insert(repo.clone());
        self.net
            .github_mut()
            .running_mut()
            .insert(repo, Instant::now());
        if first_repo {
            // First repo queued — add the "GitHub repos" tracked item
            // to the startup toast and reset completion so the phase
            // is re-evaluated now that there's actual work to track.
            self.startup.repo.complete_at = None;
            self.startup.complete_at = None;
            if let Some(toast) = self.startup.toast {
                let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
                self.toasts.add_new_tracked_items(
                    toast,
                    &[TrackedItem {
                        label:        STARTUP_PHASE_GITHUB.to_string(),
                        key:          STARTUP_PHASE_GITHUB.into(),
                        started_at:   Some(Instant::now()),
                        completed_at: None,
                    }],
                    linger,
                );
                let toast_len = self.toasts.active_now().len();
                self.toasts.viewport_mut().set_len(toast_len);
            }
        }
        self.sync_running_repo_fetch_toast();
    }
    pub(super) fn handle_repo_fetch_complete(&mut self, repo: OwnerRepo) {
        self.net
            .github_mut()
            .repo_fetch_in_flight_mut()
            .remove(&repo);
        self.net.github_mut().running_mut().remove(&repo);
        self.startup.repo.seen.insert(repo);
        self.maybe_log_startup_phase_completions();
        self.sync_running_repo_fetch_toast();
    }
    pub fn handle_repo_meta(&mut self, path: &Path, stars: u64, description: Option<String>) {
        if let Some(entry) = self.project_list.entry_containing_mut(path) {
            let repo = entry.git_repo.get_or_insert_with(Default::default);
            repo.github_info = Some(GitHubInfo { stars, description });
        }
    }
    pub fn handle_project_discovered(&mut self, item: RootItem) -> bool {
        let legacy_expansions = self.project_list.capture_legacy_root_expansions();
        let discovered_path = item.path().to_path_buf();
        let mut already_exists = false;
        self.project_list.for_each_leaf_path(|path, _| {
            if path == discovered_path {
                already_exists = true;
            }
        });
        if already_exists {
            return false;
        }

        self.register_item_background_services(&item);
        // Insert into the hierarchy directly — under a parent workspace if
        // one exists, otherwise as a top-level peer.
        let discovered_path = item.path().to_path_buf();
        let inline_dirs = self.config.current().tui.inline_dirs.clone();
        {
            let mut tree = self.mutate_tree();
            tree.insert_into_hierarchy(item);
            tree.regroup_members(&inline_dirs);
        }
        self.register_discovery_shimmer(discovered_path.as_path());
        self.project_list
            .migrate_legacy_root_expansions(&legacy_expansions);
        self.rebuild_visible_rows_now();
        // Signal that derived state and caches need refresh.
        // The caller batches multiple discoveries before refreshing once.
        true
    }
    pub fn handle_project_refreshed(&mut self, item: RootItem) -> bool {
        let legacy_expansions = self.project_list.capture_legacy_root_expansions();
        let path = item.path().to_path_buf();

        // Replace the leaf in project_list_items, transferring runtime data
        // from the old item to the incoming one. `worktree_health` is
        // filesystem-detected at refresh time and must survive the info copy.
        // `worktree_status` is no longer on `ProjectInfo` — it lives directly
        // on `Workspace` / `Package` / `NonRustProject` — so this copy cannot
        // clobber it.
        let inline_dirs = self.config.current().tui.inline_dirs.clone();
        {
            let mut tree = self.mutate_tree();
            let Some(old) = tree.replace_leaf_by_path(&path, item.clone()) else {
                return false;
            };
            let mut item = item;
            for (project_path, info) in old.collect_project_info() {
                if let Some(project) = item.at_path_mut(&project_path) {
                    let fresh_worktree_health = project.worktree_health;
                    *project = info;
                    project.worktree_health = fresh_worktree_health;
                }
            }
            // Re-replace with the runtime-data-enriched version.
            tree.replace_leaf_by_path(&path, item);
            tree.regroup_members(&inline_dirs);
            tree.regroup_top_level_worktrees();
        }
        self.reload_lint_history(&path);
        self.project_list
            .migrate_legacy_root_expansions(&legacy_expansions);
        self.rebuild_visible_rows_now();
        self.ci.clear_content();
        self.lint.clear_content();
        self.panes.clear_detail_data(None);
        // Signal that derived state needs refresh (batched by caller).
        true
    }

    fn startup_git_directory_for_path(&self, path: &Path) -> Option<AbsolutePath> {
        self.project_list
            .iter()
            .find(|entry| entry.item.at_path(path).is_some())
            .and_then(|entry| entry.item.git_directory())
    }
}
