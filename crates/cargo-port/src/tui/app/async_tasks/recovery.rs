use std::collections::HashSet;
use std::path::PathBuf;

use crate::ci;
use crate::ci::OwnerRepo;
use crate::http::ServiceKind;
use crate::scan;
use crate::tui::app::App;

impl App {
    /// Dispatch the per-service "refetch rows that didn't land during
    /// the outage" pass. Called from [`App::apply_recovery_outcome`]
    /// on every reachable transition (silent grace window or
    /// user-visible recovery), so background fetches that failed
    /// during the outage are retried without waiting for an
    /// unrelated trigger.
    pub(super) fn refetch_missing_after_recovery(&mut self, service: ServiceKind) {
        match service {
            ServiceKind::CratesIo => self.refetch_missing_crates_io_targets(),
            ServiceKind::GitHub => self.refetch_missing_github_repos(),
        }
    }
    /// Identify cached GitHub repo entries that look like failed
    /// fetches (`meta.is_none()` — a successful `batch_fetch_jobs_and_meta`
    /// always returns a meta payload, even for repos with zero CI
    /// runs), invalidate them, then re-fire
    /// [`App::spawn_repo_fetch_for_git_info`] for every leaf with a
    /// parseable URL. The existing in-flight dedup absorbs duplicate
    /// calls; cache hits for entries we left alone return immediately
    /// without spending a request.
    fn refetch_missing_github_repos(&mut self) {
        self.invalidate_failed_github_cache_entries();
        let leaves = self.collect_leaves_with_repo_urls();
        for (path, url) in leaves {
            self.spawn_repo_fetch_for_git_info(&path, &url);
        }
    }
    /// Walk the repo cache and drop entries whose `meta.is_none()` —
    /// the marker for a fetch that ran during the outage and stored
    /// empty data. Successful entries (any prior good fetch) keep
    /// their cached meta and are not refetched.
    fn invalidate_failed_github_cache_entries(&self) {
        let to_invalidate: Vec<OwnerRepo> = match self.net.github.fetch_cache.lock() {
            Ok(cache) => cache
                .iter()
                .filter(|(_, data)| data.meta.is_none())
                .map(|(repo, _)| repo.clone())
                .collect(),
            Err(_) => return,
        };
        for repo in &to_invalidate {
            scan::invalidate_cached_repo_data(&self.net.github.fetch_cache, repo);
        }
    }
    /// Collect `(path, url)` pairs for every leaf in the tree that
    /// resolves to a parseable GitHub repo. Deduped by `OwnerRepo` so
    /// linked worktrees on the same repo don't enqueue redundant
    /// fetches — the per-spawn dedup would catch them, but skipping
    /// the work earlier saves a few clones.
    fn collect_leaves_with_repo_urls(&self) -> Vec<(PathBuf, String)> {
        let mut seen: HashSet<OwnerRepo> = HashSet::new();
        let mut out: Vec<(PathBuf, String)> = Vec::new();
        self.project_list.for_each_leaf_path(|path, _| {
            let Some(url) = self.project_list.fetch_url_for(path) else {
                return;
            };
            let Some(repo) = ci::parse_owner_repo(&url) else {
                return;
            };
            if seen.insert(repo) {
                out.push((path.to_path_buf(), url));
            }
        });
        out
    }
}
