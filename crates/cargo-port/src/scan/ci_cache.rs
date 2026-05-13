use std::cmp::Reverse;
use std::collections::HashSet;

use super::cache_dir;
use super::combine_service_signal;
use super::discovery::RepoMetaInfo;
use crate::ci;
use crate::ci::CiRun;
use crate::ci::GhRun;
use crate::constants::NO_MORE_RUNS_MARKER;
use crate::http::HttpClient;
use crate::http::ServiceSignal;
use crate::project::AbsolutePath;

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

/// Check if the "no more runs" marker exists for a repo.
pub(crate) fn is_exhausted(owner: &str, repo: &str) -> bool {
    ci_cache_dir(owner, repo).join(NO_MORE_RUNS_MARKER).exists()
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
pub(crate) fn load_all_cached_runs(owner: &str, repo: &str) -> Vec<CiRun> {
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

    // Partition into cached hits and misses.  Cached failures are
    // re-fetched when their `updated_at` differs from the REST response,
    // which indicates the run was re-run on GitHub.
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

    // Single GraphQL call: jobs for uncached runs + repo metadata.
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

    // Fetched runs take priority
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
    let (gh_runs, github_total) =
        gh_list.map_or_else(|| (Vec::new(), 0), |list| (list.runs, list.total_count));
    let (fetched, meta, detail_signal) = fetch_recent_runs(client, repo_url, owner, repo, &gh_runs);
    let cached = load_all_cached_runs(owner, repo);
    let merged = merge_runs(fetched, cached);

    let result = if gh_runs.is_empty() {
        CiFetchResult::CacheOnly(merged)
    } else {
        CiFetchResult::Loaded {
            runs: merged,
            github_total,
        }
    };
    (
        result,
        meta,
        combine_service_signal(list_signal, detail_signal),
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
    let (gh_runs, github_total) =
        gh_list.map_or_else(|| (Vec::new(), 0), |list| (list.runs, list.total_count));
    let (fetched, _meta, detail_signal) =
        fetch_recent_runs(client, repo_url, owner, repo, &gh_runs);

    let mut result = fetched;
    result.sort_by_key(|run| Reverse(run.run_id));

    let result = if gh_runs.is_empty() {
        CiFetchResult::CacheOnly(result)
    } else {
        CiFetchResult::Loaded {
            runs: result,
            github_total,
        }
    };
    (result, combine_service_signal(list_signal, detail_signal))
}

pub(crate) struct CratesIoInfo {
    pub version:   String,
    pub downloads: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ci_cache_dir_scopes_runs_by_repo() {
        let main_dir = ci_cache_dir_pub("acme", "demo");
        let feature_dir = ci_cache_dir_pub("acme", "demo");

        assert_eq!(main_dir, feature_dir);
        assert!(feature_dir.ends_with("acme/demo"));
    }
}
