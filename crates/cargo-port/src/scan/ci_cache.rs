use std::cmp::Reverse;
use std::collections::HashSet;

use super::background;
use super::cache_dir;
use super::discovery::RepoMetaInfo;
use crate::ci;
use crate::ci::CiRun;
use crate::ci::GhRun;
use crate::constants::NO_MORE_RUNS_MARKER;
use crate::http::HttpClient;
use crate::http::ServiceSignal;
use crate::project::AbsolutePath;
use crate::project::CiPagination;

/// What a CI fetch function returns. Forces callers to handle the
/// "network failed but cache exists" case explicitly -- the compiler won't
/// let you silently discard cached runs.
pub(crate) enum CiFetchResult {
    /// Fresh runs (network succeeded), merged with cache.
    Loaded {
        runs:         Vec<CiRun>,
        github_total: u32,
    },
    /// Network failed; returning whatever the disk cache had.
    CacheOnly(Vec<CiRun>),
}

impl CiFetchResult {
    pub(crate) fn into_runs(self) -> Vec<CiRun> {
        match self {
            Self::Loaded { runs, .. } | Self::CacheOnly(runs) => runs,
        }
    }
}

/// Repo-keyed cache directory: `{cache_dir}/{owner}/{repo}`.
fn repo_cache_dir(owner: &str, repo: &str) -> AbsolutePath {
    cache_dir().join(owner).join(repo).into()
}

fn ci_cache_dir(owner: &str, repo: &str) -> AbsolutePath { repo_cache_dir(owner, repo) }

pub(crate) fn ci_cache_dir_pub(owner: &str, repo: &str) -> AbsolutePath {
    ci_cache_dir(owner, repo)
}

/// `pagination` returns `CiPagination::Exhausted` when the repo's "no more runs"
/// marker exists, and `CiPagination::HasMore` otherwise.
pub(crate) fn pagination(owner: &str, repo: &str) -> CiPagination {
    crate::project::CiPagination::from(ci_cache_dir(owner, repo).join(NO_MORE_RUNS_MARKER).exists())
}

/// Save the "no more runs" marker for a repo.
pub(crate) fn mark_exhausted(owner: &str, repo: &str) {
    let dir = ci_cache_dir(owner, repo);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(NO_MORE_RUNS_MARKER), "");
}

/// Remove the "no more runs" marker so fresh runs can be discovered.
pub(crate) fn clear_exhausted(owner: &str, repo: &str) {
    let dir = ci_cache_dir(owner, repo);
    let _ = std::fs::remove_file(dir.join(NO_MORE_RUNS_MARKER));
}

fn save_cached_run(owner: &str, repo: &str, ci_run: &CiRun) {
    let dir = ci_cache_dir(owner, repo);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", ci_run.run_id));
    if let Ok(json) = serde_json::to_string(ci_run) {
        let _ = std::fs::write(&path, json);
    }
}

fn load_cached_run(owner: &str, repo: &str, run_id: u64) -> Option<CiRun> {
    let dir = ci_cache_dir(owner, repo);
    let path = dir.join(format!("{run_id}.json"));
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Load all cached CI runs for a given repo.
fn load_all_cached_runs(owner: &str, repo: &str) -> Vec<CiRun> {
    let dir = ci_cache_dir(owner, repo);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|e| {
            let contents = std::fs::read_to_string(e.path()).ok()?;
            serde_json::from_str::<CiRun>(&contents).ok()
        })
        .collect()
}

/// Fetch recent CI runs and repo metadata: serve cached runs when
/// possible, batch-fetch jobs for uncached runs + repo stars/description
/// in a single GraphQL call.
fn fetch_recent_runs(
    client: &HttpClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    gh_runs: &[GhRun],
) -> (Vec<CiRun>, Option<RepoMetaInfo>, Option<ServiceSignal>) {
    let mut result: Vec<CiRun> = Vec::with_capacity(gh_runs.len());

    // `load_cached_run` partitions cached `CiRun` hits from `uncached` misses.
    // Failed runs whose `updated_at` differs from REST's `GhRun::updated_at`
    // reenter `uncached`, because that change indicates a GitHub rerun.
    let mut uncached: Vec<&GhRun> = Vec::new();
    for gh_run in gh_runs {
        match load_cached_run(owner, repo, gh_run.id) {
            Some(cached)
                if cached.ci_status.is_failure()
                    && cached.updated_at.as_deref() != Some(&gh_run.updated_at) =>
            {
                uncached.push(gh_run);
            },
            Some(cached) => result.push(cached),
            None => uncached.push(gh_run),
        }
    }

    // `client.batch_fetch_jobs_and_meta` fetches uncached jobs and
    // `RepoMetaInfo` together in one GraphQL call.
    let (batch, signal) = client.batch_fetch_jobs_and_meta(owner, repo, &uncached);
    let (jobs_map, meta) = batch.unwrap_or_default();
    for gh_run in &uncached {
        if let Some(check_runs) = jobs_map.get(&gh_run.id) {
            let ci_run = ci::build_ci_run(gh_run, check_runs.clone(), repo_url);
            save_cached_run(owner, repo, &ci_run);
            result.push(ci_run);
        }
    }

    (result, meta, signal)
}

/// Merge fetched + cached runs, deduplicated by `run_id`, sorted descending.
fn merge_runs(fetched: Vec<CiRun>, cached: Vec<CiRun>) -> Vec<CiRun> {
    let mut seen = HashSet::new();
    let mut merged: Vec<CiRun> = Vec::new();

    // `merge_runs` inserts `fetched` into `seen` before `cached`, so fresh
    // `CiRun` values win for duplicate `run_id` values.
    for run in fetched {
        if seen.insert(run.run_id) {
            merged.push(run);
        }
    }
    for run in cached {
        if seen.insert(run.run_id) {
            merged.push(run);
        }
    }

    merged.sort_by_key(|run| Reverse(run.run_id));
    merged
}

/// Fetch CI runs, using the repo-keyed cache. Merges freshly fetched runs
/// with all previously cached runs for this repo, deduplicated and sorted by `run_id` descending.
///
/// Accepts `(repo_url, owner, repo)` derived from the *local* git remote so that
/// cache loading never depends on network availability.
pub(crate) fn fetch_ci_runs_cached(
    client: &HttpClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    count: u32,
) -> (CiFetchResult, Option<RepoMetaInfo>, Option<ServiceSignal>) {
    let (gh_list, list_signal) = client.list_runs(owner, repo, None, count, None);
    let (result, meta, detail_signal) = complete_ci_fetch(
        gh_list.map(|list| (list.runs, list.total_count)),
        load_all_cached_runs(owner, repo),
        |runs| fetch_recent_runs(client, repo_url, owner, repo, runs),
    );
    (
        result,
        meta,
        background::combine_service_signal(list_signal, detail_signal),
    )
}

/// Fetch CI runs older than the oldest cached run, using the
/// `created=<{date}` filter so the request size is always `count`.
pub(crate) fn fetch_older_runs(
    client: &HttpClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    oldest_created_at: &str,
    count: u32,
) -> (CiFetchResult, Option<ServiceSignal>) {
    let (gh_list, list_signal) =
        client.list_runs(owner, repo, None, count, Some(oldest_created_at));
    let (result, _, detail_signal) = complete_ci_fetch(
        gh_list.map(|list| (list.runs, list.total_count)),
        Vec::new(),
        |runs| fetch_recent_runs(client, repo_url, owner, repo, runs),
    );
    (
        result,
        background::combine_service_signal(list_signal, detail_signal),
    )
}

/// Classify the list reply before loading details and merging the cache.
fn complete_ci_fetch(
    reply: Option<(Vec<GhRun>, u32)>,
    cached: Vec<CiRun>,
    fetch_details: impl FnOnce(&[GhRun]) -> (Vec<CiRun>, Option<RepoMetaInfo>, Option<ServiceSignal>),
) -> (CiFetchResult, Option<RepoMetaInfo>, Option<ServiceSignal>) {
    let (listed, total) = match reply {
        Some((runs, total)) => (runs, Some(total)),
        None => (Vec::new(), None),
    };
    let (fetched, meta, signal) = fetch_details(&listed);
    let runs = merge_runs(fetched, cached);
    let result = match total {
        Some(github_total) => CiFetchResult::Loaded { runs, github_total },
        None => CiFetchResult::CacheOnly(runs),
    };
    (result, meta, signal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ci::CiStatus;
    use crate::ci::FetchStatus;

    #[test]
    fn ci_cache_dir_scopes_runs_by_repo() {
        let main_dir = ci_cache_dir_pub("acme", "demo");
        let feature_dir = ci_cache_dir_pub("acme", "demo");

        assert_eq!(main_dir, feature_dir);
        assert!(feature_dir.ends_with("acme/demo"));
    }

    fn run(id: u64) -> CiRun {
        CiRun {
            run_id:          id,
            created_at:      "2026-11-07T10:00:00Z".to_string(),
            branch:          "main".to_string(),
            url:             format!("https://github.com/example/demo/actions/runs/{id}"),
            ci_status:       CiStatus::Passed,
            jobs:            Vec::new(),
            wall_clock_secs: None,
            commit_title:    None,
            updated_at:      None,
            fetched:         FetchStatus::Fetched,
        }
    }

    #[test]
    fn empty_ci_success_keeps_zero_and_cached_runs() {
        for cached in [vec![run(8), run(9)], Vec::new()] {
            let expected_ids = cached
                .iter()
                .rev()
                .map(|run| run.run_id)
                .collect::<Vec<_>>();
            let (result, _, _) =
                complete_ci_fetch(Some((Vec::new(), 0)), cached, |_| (Vec::new(), None, None));
            assert_eq!(
                match result {
                    CiFetchResult::Loaded { runs, github_total } => Some((
                        github_total,
                        runs.iter().map(|run| run.run_id).collect::<Vec<_>>()
                    )),
                    CiFetchResult::CacheOnly(_) => None,
                },
                Some((0, expected_ids))
            );
        }
    }

    #[test]
    fn failed_ci_request_keeps_cached_runs_without_a_count() {
        for cached in [vec![run(8)], Vec::new()] {
            let expected_ids = cached.iter().map(|run| run.run_id).collect::<Vec<_>>();
            let (result, _, _) = complete_ci_fetch(None, cached, |_| (Vec::new(), None, None));
            assert_eq!(
                match result {
                    CiFetchResult::CacheOnly(runs) =>
                        Some(runs.iter().map(|run| run.run_id).collect::<Vec<_>>()),
                    CiFetchResult::Loaded { .. } => None,
                },
                Some(expected_ids)
            );
        }
    }

    #[test]
    fn ci_success_keeps_the_api_count_when_run_details_are_empty() {
        for cached in [vec![run(8)], Vec::new()] {
            let expected_ids = cached.iter().map(|run| run.run_id).collect::<Vec<_>>();
            let (result, _, _) = complete_ci_fetch(Some((Vec::new(), 2104)), cached, |_| {
                (Vec::new(), None, None)
            });
            assert_eq!(
                match result {
                    CiFetchResult::Loaded { runs, github_total } => Some((
                        github_total,
                        runs.iter().map(|run| run.run_id).collect::<Vec<_>>()
                    )),
                    CiFetchResult::CacheOnly(_) => None,
                },
                Some((2104, expected_ids))
            );
        }
    }
}
