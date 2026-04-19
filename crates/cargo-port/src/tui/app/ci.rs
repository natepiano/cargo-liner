use std::collections::HashSet;
use std::path::Path;

use super::App;
use super::types::CiRunDisplayMode;
use crate::ci;
use crate::ci::CiRun;
use crate::project::AbsolutePath;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::scan;
use crate::scan::CiFetchResult;
use crate::tui::panes::CiFetchKind;
use crate::tui::panes::PaneId;

impl App {
    pub(super) fn owner_repo_for_path_inner(&self, path: &Path) -> Option<ci::OwnerRepo> {
        let entry_path = self.projects.entry_containing(path)?.item.path().clone();
        self.primary_url_for(entry_path.as_path())
            .and_then(ci::parse_owner_repo)
    }

    /// Insert CI runs from the initial scan for the entry containing `path`.
    pub(super) fn insert_ci_runs(&mut self, path: &Path, runs: Vec<CiRun>, github_total: u32) {
        let exhausted = self
            .primary_url_for(path)
            .and_then(ci::parse_owner_repo)
            .is_some_and(|owner_repo| scan::is_exhausted(owner_repo.owner(), owner_repo.repo()));
        if let Some(entry) = self.projects.entry_containing_mut(path) {
            let repo = entry.git_repo.get_or_insert_with(Default::default);
            repo.ci_data = ProjectCiData::Loaded(ProjectCiInfo {
                runs,
                github_total,
                exhausted,
            });
        } else {
            self.ci_fetch_tracker.complete(path);
        }
    }

    /// Process a completed CI fetch: merge runs and detect exhaustion.
    pub(super) fn handle_ci_fetch_complete(
        &mut self,
        path: &str,
        result: CiFetchResult,
        kind: CiFetchKind,
    ) {
        let abs = AbsolutePath::from(Path::new(path));

        let prev_info = self.ci_info_for(abs.as_path());
        let prev_count = prev_info.map_or(0, |info| info.runs.len());
        let prev_exhausted = prev_info.is_some_and(|info| info.exhausted);
        let prev_github_total = prev_info.map_or(0, |info| info.github_total);

        // Only Sync returns an unfiltered total_count from GitHub.
        // FetchOlder uses created=<{date} which returns a filtered count,
        // and CacheOnly means the network failed.  In both cases, keep
        // the previous total.
        let github_total = match (&result, kind) {
            (CiFetchResult::Loaded { github_total, .. }, CiFetchKind::Sync) => *github_total,
            _ => prev_github_total,
        };
        let new_runs = result.into_runs();
        let existing = prev_info.map_or_else(Vec::new, |info| info.runs.clone());

        let mut seen = HashSet::new();
        let mut merged = Vec::new();
        for run in new_runs {
            if seen.insert(run.run_id) {
                merged.push(run);
            }
        }
        for run in existing {
            if seen.insert(run.run_id) {
                merged.push(run);
            }
        }
        merged.sort_by_key(|run| std::cmp::Reverse(run.run_id));

        let found_new = merged.len() > prev_count;
        // Only FetchOlder marks/clears exhaustion.  Sync clears it when
        // new runs appear but never marks it — we don't want a routine
        // refresh to block future FetchOlder requests.
        let exhausted = match kind {
            CiFetchKind::Sync => {
                if found_new {
                    if let Some(url) = self.primary_url_for(&abs)
                        && let Some(owner_repo) = ci::parse_owner_repo(url)
                    {
                        scan::clear_exhausted(owner_repo.owner(), owner_repo.repo());
                    }
                    false
                } else {
                    self.status_flash =
                        Some(("no new runs found".to_string(), std::time::Instant::now()));
                    self.show_timed_toast("CI", "No new runs found".to_string());
                    // Preserve current exhaustion state.
                    prev_exhausted
                }
            },
            CiFetchKind::FetchOlder => {
                if found_new {
                    if let Some(url) = self.primary_url_for(&abs)
                        && let Some(owner_repo) = ci::parse_owner_repo(url)
                    {
                        scan::clear_exhausted(owner_repo.owner(), owner_repo.repo());
                    }
                    false
                } else {
                    if let Some(url) = self.primary_url_for(&abs)
                        && let Some(owner_repo) = ci::parse_owner_repo(url)
                    {
                        scan::mark_exhausted(owner_repo.owner(), owner_repo.repo());
                    }
                    true
                }
            },
        };

        self.pane_manager
            .pane_mut(PaneId::CiRuns)
            .set_pos(merged.len());
        if let Some(repo) = self.owner_repo_for_path_inner(&abs) {
            let meta = crate::scan::load_cached_repo_data(&self.repo_fetch_cache, &repo)
                .and_then(|cached| cached.meta);
            crate::scan::store_cached_repo_data(
                &self.repo_fetch_cache,
                &repo,
                crate::scan::CachedRepoData {
                    runs: merged.clone(),
                    meta,
                    github_total,
                },
            );
        }
        self.ci_fetch_tracker.complete(abs.as_path());
        if let Some(entry) = self.projects.entry_containing_mut(abs.as_path()) {
            let repo = entry.git_repo.get_or_insert_with(Default::default);
            repo.ci_data = ProjectCiData::Loaded(ProjectCiInfo {
                runs: merged,
                github_total,
                exhausted,
            });
        }
        self.data_generation += 1;
    }

    pub(super) fn ci_display_mode_for(&self, path: &Path) -> CiRunDisplayMode {
        self.ci_display_modes.get(path).copied().unwrap_or_default()
    }

    pub(super) fn ci_display_mode_label_for_inner(&self, path: &Path) -> &'static str {
        match self.ci_display_mode_for(path) {
            CiRunDisplayMode::BranchOnly => "branch",
            CiRunDisplayMode::All => "all",
        }
    }

    fn current_branch_for(&self, path: &Path) -> Option<&str> {
        self.git_info_for(path)?.branch.as_deref()
    }

    pub(super) fn ci_toggle_available_for_inner(&self, path: &Path) -> bool {
        self.current_branch_for(path).is_some()
    }

    pub(super) fn toggle_ci_display_mode_for_inner(&mut self, path: &Path) {
        if !self.ci_toggle_available_for_inner(path) {
            self.ci_display_modes.remove(path);
            return;
        }
        let new_mode = match self.ci_display_mode_for(path) {
            CiRunDisplayMode::BranchOnly => CiRunDisplayMode::All,
            CiRunDisplayMode::All => CiRunDisplayMode::BranchOnly,
        };
        self.ci_display_modes
            .insert(AbsolutePath::from(path), new_mode);
        self.pane_manager.pane_mut(PaneId::CiRuns).home();
        self.data_generation += 1;
    }

    pub(super) fn ci_runs_for_display_inner(&self, path: &Path) -> Vec<CiRun> {
        let Some(info) = self.ci_info_for(path) else {
            return Vec::new();
        };
        let Some(branch) = self.current_branch_for(path) else {
            return info.runs.clone();
        };
        if self.ci_display_mode_for(path) == CiRunDisplayMode::All {
            return info.runs.clone();
        }
        info.runs
            .iter()
            .filter(|run| run.branch == branch)
            .cloned()
            .collect()
    }

    pub(super) fn latest_ci_run_for_path(&self, path: &Path) -> Option<&CiRun> {
        let info = self.ci_info_for(path)?;
        let runs = info.runs.as_slice();
        let Some(branch) = self.current_branch_for(path) else {
            return runs.first();
        };
        if self.ci_display_mode_for(path) == CiRunDisplayMode::All {
            return runs.first();
        }
        runs.iter().find(|run| run.branch == branch)
    }
}
