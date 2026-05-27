use std::collections::HashSet;
use std::path::Path;
use std::thread;
use std::time::Instant;

use tui_pane::TrackedItem;

use crate::ci;
use crate::ci::OwnerRepo;
use crate::http::PullRequestFetch;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::GitStatus;
use crate::project::LocalGitState;
use crate::project::ProjectPrData;
use crate::project::ProjectPrInfo;
use crate::project::ProjectPrUnavailable;
use crate::project::PullRequestInfo;
use crate::project::PullRequestUnavailableReason;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::CachedRepoData;
use crate::scan::CiFetchResult;
use crate::tui::app::App;
use crate::tui::constants::STARTUP_PHASE_GITHUB;
use crate::tui::integration;
use crate::tui::state;

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
            .github
            .repo_fetch_in_flight_mut()
            .insert(owner_repo.clone())
        {
            return;
        }

        let tx = self.background.background_sender();
        let client = self.net.http_client();
        let repo_cache = self.net.github.fetch_cache.clone();
        let path: AbsolutePath = AbsolutePath::from(path);
        let repo_url = repo_url.to_string();
        let ci_run_count = self.config.ci_run_count();
        thread::spawn(move || {
            let mut data =
                scan::load_cached_repo_data(&repo_cache, &owner_repo).unwrap_or_else(|| {
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
                        pr_data: ProjectPrData::Unfetched,
                    };
                    scan::store_cached_repo_data(&repo_cache, &owner_repo, data.clone());
                    data
                });
            if data.pr_data.needs_fetch() {
                let _ = tx.send(BackgroundMsg::RepoFetchQueued {
                    repo: owner_repo.clone(),
                });
                let _ = tx.send(BackgroundMsg::PullRequests {
                    repo: owner_repo.clone(),
                    data: ProjectPrData::Loading,
                });
                let stale = data.pr_data.info().cloned();
                let (pr_fetch, signal) = client.fetch_open_pull_requests(owner_repo.clone());
                scan::emit_service_signal(&tx, signal);
                data.pr_data = match pr_fetch {
                    Some(PullRequestFetch::Loaded(info)) => ProjectPrData::Loaded(info),
                    Some(PullRequestFetch::Unavailable(reason)) => {
                        ProjectPrData::Unavailable(ProjectPrUnavailable {
                            reason,
                            stale,
                            fetched_at: None,
                        })
                    },
                    None => ProjectPrData::Unavailable(ProjectPrUnavailable {
                        reason: PullRequestUnavailableReason::Network,
                        stale,
                        fetched_at: None,
                    }),
                };
                scan::store_cached_repo_data(&repo_cache, &owner_repo, data.clone());
            }

            let _ = tx.send(BackgroundMsg::CiRuns {
                path:         path.clone(),
                runs:         data.runs,
                github_total: data.github_total,
            });
            let _ = tx.send(BackgroundMsg::PullRequests {
                repo: owner_repo.clone(),
                data: data.pr_data,
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

        let status = info.status;
        if let Some(project) = self.project_list.at_path_mut(path) {
            project.local_git_state = LocalGitState::Detected(Box::new(info));
        }
        // Detected git state implies the entry (or submodule) is in a
        // git repo. Ensure a `GitRepo` slot exists for `path` so per-repo
        // writes (CI, GitHub meta, RepoInfo) can land on it.
        self.project_list.ensure_git_repo_for(path);

        if self.scan.is_complete() {
            let git_dir = self
                .startup_git_directory_for_path(path)
                .unwrap_or_else(|| AbsolutePath::from(path));
            self.startup.git.seen.insert(git_dir.clone());
            if let Some(git_toast) = self.startup.git.toast {
                let key = integration::path_key(&git_dir);
                self.framework.toasts.mark_item_completed(git_toast, &key);
            }
            self.maybe_log_startup_phase_completions();
        }

        self.record_git_status_observation(path, status);
        self.maybe_trigger_repo_fetch(path);
    }

    /// Diff the current `GitStatus` for `path` against the tracker's
    /// baseline; on transition, push or extend the "Git status changes"
    /// task toast.
    fn record_git_status_observation(&mut self, path: &Path, status: GitStatus) {
        let Some(transition) = self
            .git_status_tracker
            .observe(AbsolutePath::from(path), status)
        else {
            return;
        };
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        let label = state::format_git_status_transition(&name, &transition);
        let seq = self.git_status_tracker.next_item_seq();
        let key = format!("{}#{seq}", path.display());
        let item = TrackedItem {
            label,
            key: key.into(),
            started_at: None,
            completed_at: Some(Instant::now()),
        };

        let reuse = self
            .git_status_tracker
            .current_toast()
            .filter(|id| self.framework.toasts.tracked_item_count(*id) > 0);
        let toast_id = reuse.unwrap_or_else(|| {
            let id = self.framework.toasts.push_task("Git status changes", "", 1);
            self.git_status_tracker.set_current_toast(Some(id));
            id
        });
        self.framework
            .toasts
            .add_new_tracked_items(toast_id, &[item]);
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

        // Submodules write to their own `git_repo`; for top-level
        // entries we still apply the primary-only-write policy below.
        let is_submodule_target = self.project_list.is_submodule_path(path);
        if is_submodule_target {
            if let Some(git_repo) = self.project_list.ensure_git_repo_for(path) {
                git_repo.repo_info = Some(info);
            }
        } else if let Some(entry) = self.project_list.entry_containing_mut(path) {
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
            && !self.net.github.contains_in_flight(&owner_repo)
        {
            scan::invalidate_cached_repo_data(&self.net.github.fetch_cache, &owner_repo);
        }

        self.record_sync_observation(path);
        self.maybe_trigger_repo_fetch(path);
    }

    /// Diff the current sync state for `path` against the tracker's
    /// baseline; on transition, push or extend the "Sync changes" task
    /// toast.
    fn record_sync_observation(&mut self, path: &Path) {
        let current = self.project_list.primary_ahead_behind_for(path);
        let Some(transition) = self.sync_tracker.observe(AbsolutePath::from(path), current) else {
            return;
        };
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        let label = state::format_sync_transition(&name, &transition);
        let seq = self.sync_tracker.next_item_seq();
        let key = format!("{}#{seq}", path.display());
        let item = TrackedItem {
            label,
            key: key.into(),
            started_at: None,
            completed_at: Some(Instant::now()),
        };

        let reuse = self
            .sync_tracker
            .current_toast()
            .filter(|id| self.framework.toasts.tracked_item_count(*id) > 0);
        let toast_id = reuse.unwrap_or_else(|| {
            let id = self.framework.toasts.push_task("Sync changes", "", 1);
            self.sync_tracker.set_current_toast(Some(id));
            id
        });
        self.framework
            .toasts
            .add_new_tracked_items(toast_id, &[item]);
    }
    /// Shared between `handle_repo_info` and `handle_checkout_info`:
    /// kick a GitHub fetch for this path's repo if we have a parseable
    /// remote URL. The dedup set in `App.net.github.repo_fetch_in_flight`
    /// keys on `OwnerRepo`, so a submodule sharing its `OwnerRepo` with
    /// the parent won't cause a duplicate fetch. Cache invalidation is
    /// gated inside `handle_repo_info` by `last_fetched` advance.
    pub(super) fn maybe_trigger_repo_fetch(&mut self, path: &Path) {
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
        self.net.github.running_mut().insert(repo, Instant::now());
        if first_repo {
            // First repo queued — add the "GitHub repos" tracked item
            // to the startup toast and reset completion so the phase
            // is re-evaluated now that there's actual work to track.
            self.startup.repo.complete_at = None;
            self.startup.complete_at = None;
            if let Some(toast) = self.startup.toast {
                self.framework.toasts.add_new_tracked_items(
                    toast,
                    &[TrackedItem {
                        label:        STARTUP_PHASE_GITHUB.to_string(),
                        key:          STARTUP_PHASE_GITHUB.into(),
                        started_at:   Some(Instant::now()),
                        completed_at: None,
                    }],
                );
            }
        }
        self.sync_running_repo_fetch_toast();
    }
    pub(super) fn handle_repo_fetch_complete(&mut self, repo: OwnerRepo) {
        self.net.github.repo_fetch_in_flight_mut().remove(&repo);
        self.net.github.running_mut().remove(&repo);
        self.mark_sync_eligible_for(&repo);
        self.startup.repo.seen.insert(repo);
        self.maybe_log_startup_phase_completions();
        self.sync_running_repo_fetch_toast();
    }

    pub(super) fn handle_pull_requests(&mut self, repo: &OwnerRepo, data: &ProjectPrData) {
        let prior = self.project_list.pr_info_for_repo(repo).cloned();
        let selected_matches = self
            .project_list
            .selected_project_path()
            .and_then(|path| self.project_list.fetch_url_for(path))
            .and_then(|url| ci::parse_owner_repo(&url))
            .as_ref()
            == Some(repo);
        self.maybe_toast_deleted_pull_requests(repo, prior.as_ref(), data);
        self.project_list.replace_pr_data_for_repo(repo, data);
        if selected_matches {
            self.scan.bump_generation();
        }
    }

    fn maybe_toast_deleted_pull_requests(
        &mut self,
        repo: &OwnerRepo,
        prior: Option<&ProjectPrInfo>,
        data: &ProjectPrData,
    ) {
        let deleted = deleted_pull_requests(prior, data);
        if deleted.is_empty() {
            return;
        }
        let title = if deleted.len() == 1 {
            "Pull request deleted".to_string()
        } else {
            format!("{} pull requests deleted", deleted.len())
        };
        let body = format!(
            "{repo}: {}",
            deleted
                .iter()
                .map(|pull_request| format!("#{} {}", pull_request.number, pull_request.title))
                .collect::<Vec<_>>()
                .join(", ")
        );
        self.framework.toasts.push_status(title, body);
    }

    /// Flip the sync-toast eligibility flag for every project that
    /// resolves to `repo` via its fetch URL, seeding each baseline with
    /// the current ahead/behind so the next change toasts.
    fn mark_sync_eligible_for(&mut self, repo: &OwnerRepo) {
        let mut targets: Vec<(AbsolutePath, Option<(usize, usize)>)> = Vec::new();
        self.project_list.for_each_leaf_path(|path, _| {
            let Some(url) = self.project_list.fetch_url_for(path) else {
                return;
            };
            if ci::parse_owner_repo(&url).as_ref() != Some(repo) {
                return;
            }
            targets.push((
                AbsolutePath::from(path),
                self.project_list.primary_ahead_behind_for(path),
            ));
        });
        for (path, current) in targets {
            self.sync_tracker.mark_eligible(path, current);
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

        self.background.register_item_background_services(&item);
        self.register_lint_project_if_eligible(&item);
        // Insert into the hierarchy directly — under a parent workspace if
        // one exists, otherwise as a top-level peer.
        let discovered_path = item.path().to_path_buf();
        let inline_dirs = self.config.current().tui.inline_dirs.clone();
        let dispatch = self.metadata_dispatch();
        {
            let mut tree = self.mutate_tree();
            tree.insert_into_hierarchy(item, &dispatch);
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
        let dispatch = self.metadata_dispatch();
        {
            let mut tree = self.mutate_tree();
            // The probe-side leaf comes back with a default `Cargo`, so
            // each replace re-dispatches `cargo metadata`; the first
            // (transient extraction) dispatch is superseded by the
            // second via the store's per-root generation counter.
            let Some(old) = tree.replace_leaf_by_path(&path, item.clone(), &dispatch) else {
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
            tree.replace_leaf_by_path(&path, item, &dispatch);
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

fn deleted_pull_requests(
    prior: Option<&ProjectPrInfo>,
    data: &ProjectPrData,
) -> Vec<PullRequestInfo> {
    let Some(previous) = prior else {
        return Vec::new();
    };
    let ProjectPrData::Loaded(current) = data else {
        return Vec::new();
    };
    if previous.viewer_login != current.viewer_login {
        return Vec::new();
    }
    let current_numbers: HashSet<u32> = current
        .open
        .iter()
        .map(|pull_request| pull_request.number)
        .collect();
    previous
        .open
        .iter()
        .filter(|pull_request| !current_numbers.contains(&pull_request.number))
        .cloned()
        .collect()
}
