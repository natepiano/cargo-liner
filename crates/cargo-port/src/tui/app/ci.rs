use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::Path;

use super::App;
use crate::ci;
use crate::ci::CiRun;
use crate::project::AbsolutePath;
use crate::project::CiPagination;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::project::ProjectPrData;
use crate::scan;
use crate::scan::CachedRepoData;
use crate::scan::CiFetchResult;
use crate::tui::panes::CiFetchKind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::tui) enum CiRunDisplayMode {
    #[default]
    BranchOnly,
    All,
}

impl App {
    /// Insert CI runs from the initial scan for the entry containing `path`.
    pub(super) fn insert_ci_runs(&mut self, path: &Path, runs: Vec<CiRun>, github_total: u32) {
        let ci_pagination = CiPagination::from(
            self.project_list
                .primary_url_for(path)
                .and_then(ci::parse_owner_repo)
                .is_some_and(|owner_repo| {
                    scan::is_exhausted(owner_repo.owner(), owner_repo.repo())
                }),
        );
        if let Some(project_entry) = self.project_list.entry_containing_mut(path) {
            let repo = project_entry.git_repo.get_or_insert_with(Default::default);
            repo.ci_data = ProjectCiData::Loaded(ProjectCiInfo {
                runs,
                github_total,
                ci_pagination,
            });
        } else {
            self.ci.fetch_tracker.complete(path);
        }
    }

    /// Process a completed CI fetch: merge runs and detect exhaustion.
    /// Returns `true` when the caller should chain a `CiFetchKind::Older`
    /// request (`CiFetchKind::Sync` surfaced no new runs but a cached cursor
    /// exists to look further back). The caller preserves the toast across
    /// the chained fetch.
    pub(super) fn handle_ci_fetch_complete(
        &mut self,
        path: &str,
        ci_fetch_result: CiFetchResult,
        ci_fetch_kind: CiFetchKind,
    ) -> bool {
        let absolute_path = AbsolutePath::from(Path::new(path));

        let previous_project_ci_info = self.project_list.ci_info_for(absolute_path.as_path());
        let prev_count =
            previous_project_ci_info.map_or(0, |project_ci_info| project_ci_info.runs.len());
        let prev_pagination = previous_project_ci_info
            .map_or(CiPagination::HasMore, |project_ci_info| {
                project_ci_info.ci_pagination
            });
        let prev_github_total =
            previous_project_ci_info.map_or(0, |project_ci_info| project_ci_info.github_total);

        // Only `CiFetchKind::Sync` returns GitHub's unfiltered total_count. `CiFetchKind::Older`
        // uses created=<{date} and gets a filtered count; `CiFetchResult::CacheOnly` means the
        // network failed. Preserve the previous `github_total` in either case.
        let github_total = match (&ci_fetch_result, ci_fetch_kind) {
            (CiFetchResult::Loaded { github_total, .. }, CiFetchKind::Sync) => *github_total,
            _ => prev_github_total,
        };
        let new_runs = ci_fetch_result.into_runs();
        let existing = previous_project_ci_info
            .map_or_else(Vec::new, |project_ci_info| project_ci_info.runs.clone());

        let merged = merge_ci_runs(new_runs, existing);

        let found_new = merged.len() > prev_count;
        // Chain `CiFetchKind::Sync` to `CiFetchKind::Older` when
        // `CiFetchKind::Sync` surfaced nothing and a cached cursor exists.
        // The caller schedules the `CiFetchKind::Older` request and
        // preserves the toast across the chained fetch.
        let chain_older =
            matches!(ci_fetch_kind, CiFetchKind::Sync) && !found_new && merged.last().is_some();
        // Only `CiFetchKind::Older` marks/clears exhaustion.
        // `CiFetchKind::Sync` clears it when new runs appear but never
        // marks it — we don't want a routine refresh to block future
        // `CiFetchKind::Older` requests.
        let ci_pagination = match ci_fetch_kind {
            CiFetchKind::Sync => {
                if found_new {
                    if let Some(url) = self.project_list.primary_url_for(&absolute_path)
                        && let Some(owner_repo) = ci::parse_owner_repo(url)
                    {
                        scan::clear_exhausted(owner_repo.owner(), owner_repo.repo());
                    }
                    CiPagination::HasMore
                } else {
                    // Skip the status flash when chaining; the chained
                    // `CiFetchKind::Older` will produce its own outcome
                    // message.
                    if !chain_older {
                        self.overlays.set_status_flash(
                            "no new runs found".to_string(),
                            std::time::Instant::now(),
                        );
                    }
                    // Keep `prev_pagination` so a sync with no new runs preserves the current
                    // `CiPagination`.
                    prev_pagination
                }
            },
            CiFetchKind::Older => {
                if found_new {
                    if let Some(url) = self.project_list.primary_url_for(&absolute_path)
                        && let Some(owner_repo) = ci::parse_owner_repo(url)
                    {
                        scan::clear_exhausted(owner_repo.owner(), owner_repo.repo());
                    }
                    CiPagination::HasMore
                } else {
                    if let Some(url) = self.project_list.primary_url_for(&absolute_path)
                        && let Some(owner_repo) = ci::parse_owner_repo(url)
                    {
                        scan::mark_exhausted(owner_repo.owner(), owner_repo.repo());
                    }
                    self.overlays.set_status_flash(
                        "no older runs found".to_string(),
                        std::time::Instant::now(),
                    );
                    CiPagination::Exhausted
                }
            },
        };

        self.ci.viewport.set_pos(merged.len());
        if let Some(repo) = self.project_list.owner_repo_for_path_inner(&absolute_path) {
            let cached_repo_data = scan::load_cached_repo_data(&self.net.github.fetch_cache, &repo);
            let repo_meta_info = cached_repo_data
                .as_ref()
                .and_then(|cached| cached.meta.clone());
            let project_pr_data =
                cached_repo_data.map_or(ProjectPrData::Unfetched, |cached| cached.pr_data);
            scan::store_cached_repo_data(
                &self.net.github.fetch_cache,
                &repo,
                CachedRepoData {
                    runs: merged.clone(),
                    meta: repo_meta_info,
                    github_total,
                    pr_data: project_pr_data,
                },
            );
        }
        self.ci.fetch_tracker.complete(absolute_path.as_path());
        if let Some(project_entry) = self
            .project_list
            .entry_containing_mut(absolute_path.as_path())
        {
            let repo = project_entry.git_repo.get_or_insert_with(Default::default);
            repo.ci_data = ProjectCiData::Loaded(ProjectCiInfo {
                runs: merged,
                github_total,
                ci_pagination,
            });
        }
        self.scan.bump_generation();
        chain_older
    }

    pub(super) fn ci_display_mode_for(&self, path: &Path) -> CiRunDisplayMode {
        self.ci.display_mode_for(path)
    }

    pub(super) fn set_ci_display_mode_for_inner(
        &mut self,
        path: &Path,
        ci_run_display_mode: CiRunDisplayMode,
    ) {
        // Workspace members share `CiRunDisplayMode` with their checkout root, so
        // `ci_branch_owner_path` supplies the `owner` path under which it is stored and read.
        let owner = self.project_list.ci_branch_owner_path(path);
        if !self
            .project_list
            .ci_toggle_available_for_inner(owner.as_path())
        {
            self.ci.remove_display_mode(owner.as_path());
            return;
        }
        if self.ci_display_mode_for(owner.as_path()) == ci_run_display_mode {
            return;
        }
        self.ci.viewport.home();
        self.ci.set_display_mode(owner, ci_run_display_mode);
        self.scan.bump_generation();
    }
}

fn merge_ci_runs(new_runs: Vec<CiRun>, existing: Vec<CiRun>) -> Vec<CiRun> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for ci_run in new_runs {
        if seen.insert(ci_run.run_id) {
            merged.push(ci_run);
        }
    }
    for ci_run in existing {
        if seen.insert(ci_run.run_id) {
            merged.push(ci_run);
        }
    }
    merged.sort_by_key(|ci_run| Reverse(ci_run.run_id));
    merged
}
