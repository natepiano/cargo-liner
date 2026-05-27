use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::Path;

use super::App;
use super::types::CiRunDisplayMode;
use crate::ci;
use crate::ci::CiRun;
use crate::project::AbsolutePath;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::project::ProjectPrData;
use crate::scan;
use crate::scan::CachedRepoData;
use crate::scan::CiFetchResult;
use crate::tui::panes::CiFetchKind;

impl App {
    /// Insert CI runs from the initial scan for the entry containing `path`.
    pub(super) fn insert_ci_runs(&mut self, path: &Path, runs: Vec<CiRun>, github_total: u32) {
        let exhausted = self
            .project_list
            .primary_url_for(path)
            .and_then(ci::parse_owner_repo)
            .is_some_and(|owner_repo| scan::is_exhausted(owner_repo.owner(), owner_repo.repo()));
        if let Some(entry) = self.project_list.entry_containing_mut(path) {
            let repo = entry.git_repo.get_or_insert_with(Default::default);
            repo.ci_data = ProjectCiData::Loaded(ProjectCiInfo {
                runs,
                github_total,
                exhausted,
            });
        } else {
            self.ci.fetch_tracker.complete(path);
        }
    }

    /// Process a completed CI fetch: merge runs and detect exhaustion.
    /// Returns `true` when the caller should chain a `FetchOlder` follow-up
    /// (Sync surfaced no new runs but a cached cursor exists to look further
    /// back). The caller preserves the toast across the chained fetch.
    pub(super) fn handle_ci_fetch_complete(
        &mut self,
        path: &str,
        result: CiFetchResult,
        kind: CiFetchKind,
    ) -> bool {
        let abs = AbsolutePath::from(Path::new(path));

        let prev_info = self.project_list.ci_info_for(abs.as_path());
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
        merged.sort_by_key(|run| Reverse(run.run_id));

        let found_new = merged.len() > prev_count;
        // Chain Sync → FetchOlder when Sync surfaced nothing and we still
        // have a cached cursor to look further back. The caller schedules
        // the FetchOlder and preserves the toast across the chained fetch.
        let chain_older =
            matches!(kind, CiFetchKind::Sync) && !found_new && merged.last().is_some();
        // Only FetchOlder marks/clears exhaustion.  Sync clears it when
        // new runs appear but never marks it — we don't want a routine
        // refresh to block future FetchOlder requests.
        let exhausted = match kind {
            CiFetchKind::Sync => {
                if found_new {
                    if let Some(url) = self.project_list.primary_url_for(&abs)
                        && let Some(owner_repo) = ci::parse_owner_repo(url)
                    {
                        scan::clear_exhausted(owner_repo.owner(), owner_repo.repo());
                    }
                    false
                } else {
                    // Skip the status flash when chaining; the chained
                    // FetchOlder will produce its own outcome message.
                    if !chain_older {
                        self.overlays.set_status_flash(
                            "no new runs found".to_string(),
                            std::time::Instant::now(),
                        );
                    }
                    // Preserve current exhaustion state.
                    prev_exhausted
                }
            },
            CiFetchKind::FetchOlder => {
                if found_new {
                    if let Some(url) = self.project_list.primary_url_for(&abs)
                        && let Some(owner_repo) = ci::parse_owner_repo(url)
                    {
                        scan::clear_exhausted(owner_repo.owner(), owner_repo.repo());
                    }
                    false
                } else {
                    if let Some(url) = self.project_list.primary_url_for(&abs)
                        && let Some(owner_repo) = ci::parse_owner_repo(url)
                    {
                        scan::mark_exhausted(owner_repo.owner(), owner_repo.repo());
                    }
                    self.overlays.set_status_flash(
                        "no older runs found".to_string(),
                        std::time::Instant::now(),
                    );
                    true
                }
            },
        };

        self.ci.viewport.set_pos(merged.len());
        if let Some(repo) = self.project_list.owner_repo_for_path_inner(&abs) {
            let cached = scan::load_cached_repo_data(&self.net.github.fetch_cache, &repo);
            let meta = cached.as_ref().and_then(|cached| cached.meta.clone());
            let pr_data = cached.map_or(ProjectPrData::Unfetched, |cached| cached.pr_data);
            scan::store_cached_repo_data(
                &self.net.github.fetch_cache,
                &repo,
                CachedRepoData {
                    runs: merged.clone(),
                    meta,
                    github_total,
                    pr_data,
                },
            );
        }
        self.ci.fetch_tracker.complete(abs.as_path());
        if let Some(entry) = self.project_list.entry_containing_mut(abs.as_path()) {
            let repo = entry.git_repo.get_or_insert_with(Default::default);
            repo.ci_data = ProjectCiData::Loaded(ProjectCiInfo {
                runs: merged,
                github_total,
                exhausted,
            });
        }
        self.scan.bump_generation();
        chain_older
    }

    pub(super) fn ci_display_mode_for(&self, path: &Path) -> CiRunDisplayMode {
        self.ci.display_mode_for(path)
    }

    pub(super) fn set_ci_display_mode_for_inner(&mut self, path: &Path, mode: CiRunDisplayMode) {
        if !self.project_list.ci_toggle_available_for_inner(path) {
            self.ci.remove_display_mode(path);
            return;
        }
        if self.ci_display_mode_for(path) == mode {
            return;
        }
        self.ci.set_display_mode(AbsolutePath::from(path), mode);
        self.ci.viewport.home();
        self.scan.bump_generation();
    }
}
