use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;

use toml::Table;
use toml::Value;
use walkdir::WalkDir;

use super::cache_paths;
use super::ci;
use super::ci::CiRun;
use super::ci::GhRun;
use super::config::NonRustInclusion;
use super::constants::NO_MORE_RUNS_MARKER;
use super::constants::OLDER_RUNS_FETCH_INCREMENT;
use super::constants::SCAN_DISK_CONCURRENCY;
use super::constants::SCAN_HTTP_CONCURRENCY;
use super::constants::SCAN_LOCAL_CONCURRENCY;
use super::http::HttpClient;
use super::http::ServiceKind;
use super::http::ServiceSignal;
use super::lint::LintStatus;
use super::perf_log;
use super::project::Cargo;
use super::project::CargoProject;
use super::project::GitInfo;
use super::project::GitPathState;
use super::project::GitRepoPresence;
use super::project::MemberGroup;
use super::project::Package;
use super::project::Project;
use super::project::ProjectListItem;
use super::project::Workspace;
use super::project::WorktreeGroup;

/// A flattened entry for fuzzy search.
pub(crate) struct FlatEntry {
    pub path:     String,
    pub abs_path: PathBuf,
    pub name:     String,
}

pub(crate) enum BackgroundMsg {
    DiskUsage {
        path:  String,
        bytes: u64,
    },
    DiskUsageBatch {
        root_path: String,
        entries:   Vec<(String, u64)>,
    },
    LocalGitQueued {
        path: String,
    },
    CiRuns {
        path: String,
        runs: Vec<CiRun>,
    },
    RepoFetchQueued {
        key: String,
    },
    RepoFetchComplete {
        key: String,
    },
    GitInfo {
        path: String,
        info: GitInfo,
    },
    GitFirstCommit {
        path:         String,
        first_commit: Option<String>,
    },
    GitPathState {
        path:  String,
        state: GitPathState,
    },
    CratesIoVersion {
        path:      String,
        version:   String,
        downloads: u64,
    },
    RepoMeta {
        path:        String,
        stars:       u64,
        description: Option<String>,
    },
    ProjectDiscovered {
        item: ProjectListItem,
    },
    ProjectRefreshed {
        item: ProjectListItem,
    },
    LintStatus {
        path:   String,
        status: LintStatus,
    },
    LintCachePruned {
        runs_evicted:    usize,
        bytes_reclaimed: u64,
    },
    ScanComplete,
    ServiceReachable {
        service: ServiceKind,
    },
    ServiceRecovered {
        service: ServiceKind,
    },
    ServiceUnreachable {
        service: ServiceKind,
    },
}

impl BackgroundMsg {
    /// Returns the project path this message relates to, if any.
    pub(crate) fn path(&self) -> Option<&str> {
        match self {
            Self::DiskUsage { path, .. }
            | Self::LocalGitQueued { path }
            | Self::CiRuns { path, .. }
            | Self::GitInfo { path, .. }
            | Self::GitFirstCommit { path, .. }
            | Self::GitPathState { path, .. }
            | Self::CratesIoVersion { path, .. }
            | Self::RepoMeta { path, .. }
            | Self::LintStatus { path, .. } => Some(path),
            Self::ProjectDiscovered { item } | Self::ProjectRefreshed { item } => {
                Some(item.path().to_str().unwrap_or(""))
            },
            Self::DiskUsageBatch { .. }
            | Self::RepoFetchQueued { .. }
            | Self::RepoFetchComplete { .. }
            | Self::LintCachePruned { .. }
            | Self::ScanComplete
            | Self::ServiceReachable { .. }
            | Self::ServiceRecovered { .. }
            | Self::ServiceUnreachable { .. } => None,
        }
    }
}

const fn combine_service_signal(
    left: Option<ServiceSignal>,
    right: Option<ServiceSignal>,
) -> Option<ServiceSignal> {
    match (left, right) {
        (Some(ServiceSignal::Unreachable(service)), _)
        | (_, Some(ServiceSignal::Unreachable(service))) => {
            Some(ServiceSignal::Unreachable(service))
        },
        (Some(ServiceSignal::Reachable(service)), _)
        | (_, Some(ServiceSignal::Reachable(service))) => Some(ServiceSignal::Reachable(service)),
        (None, None) => None,
    }
}

pub(crate) fn emit_service_signal(tx: &mpsc::Sender<BackgroundMsg>, signal: Option<ServiceSignal>) {
    let msg = match signal {
        Some(ServiceSignal::Reachable(service)) => BackgroundMsg::ServiceReachable { service },
        Some(ServiceSignal::Unreachable(service)) => BackgroundMsg::ServiceUnreachable { service },
        None => return,
    };
    let _ = tx.send(msg);
}

pub(crate) fn emit_service_recovered(tx: &mpsc::Sender<BackgroundMsg>, service: ServiceKind) {
    let _ = tx.send(BackgroundMsg::ServiceRecovered { service });
}

/// What a CI fetch function returns. Forces callers to handle the
/// "network failed but cache exists" case explicitly -- the compiler won't
/// let you silently discard cached runs.
pub(crate) enum CiFetchResult {
    /// Fresh runs (network succeeded), merged with cache.
    Loaded(Vec<CiRun>),
    /// Network failed; returning whatever the disk cache had.
    CacheOnly(Vec<CiRun>),
}

/// Base cache directory for CI metadata.
pub(crate) fn cache_dir() -> PathBuf { cache_paths::ci_cache_root() }

/// Repo-keyed cache directory: `{cache_dir}/{owner}/{repo}`.
fn repo_cache_dir(owner: &str, repo: &str) -> PathBuf { cache_dir().join(owner).join(repo) }

fn branch_cache_component(branch: &str) -> String {
    let mut encoded = String::with_capacity(branch.len());
    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            encoded.push(ch);
        } else {
            let mut buf = [0_u8; 4];
            for byte in ch.encode_utf8(&mut buf).as_bytes() {
                use std::fmt::Write;

                let _ = write!(&mut encoded, "_{byte:02x}");
            }
        }
    }
    encoded
}

fn ci_cache_dir(owner: &str, repo: &str, branch: Option<&str>) -> PathBuf {
    branch.map_or_else(
        || repo_cache_dir(owner, repo),
        |branch| {
            repo_cache_dir(owner, repo)
                .join("branches")
                .join(branch_cache_component(branch))
        },
    )
}

pub(crate) fn ci_cache_dir_pub(owner: &str, repo: &str, branch: Option<&str>) -> PathBuf {
    ci_cache_dir(owner, repo, branch)
}

/// Check if the "no more runs" marker exists for a repo.
pub(crate) fn is_exhausted(owner: &str, repo: &str, branch: Option<&str>) -> bool {
    ci_cache_dir(owner, repo, branch)
        .join(NO_MORE_RUNS_MARKER)
        .exists()
}

/// Save the "no more runs" marker for a repo.
pub(crate) fn mark_exhausted(owner: &str, repo: &str, branch: Option<&str>) {
    let dir = ci_cache_dir(owner, repo, branch);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(NO_MORE_RUNS_MARKER), "");
}

/// Remove the "no more runs" marker so fresh runs can be discovered.
pub(crate) fn clear_exhausted(owner: &str, repo: &str, branch: Option<&str>) {
    let dir = ci_cache_dir(owner, repo, branch);
    let _ = std::fs::remove_file(dir.join(NO_MORE_RUNS_MARKER));
}

fn save_cached_run(owner: &str, repo: &str, branch: Option<&str>, ci_run: &CiRun) {
    let dir = ci_cache_dir(owner, repo, branch);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", ci_run.run_id));
    if let Ok(json) = serde_json::to_string(ci_run) {
        let _ = std::fs::write(&path, json);
    }
}

fn load_cached_run(owner: &str, repo: &str, branch: Option<&str>, run_id: u64) -> Option<CiRun> {
    let dir = ci_cache_dir(owner, repo, branch);
    let path = dir.join(format!("{run_id}.json"));
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Count the number of cached CI run files on disk for a given repo.
pub(crate) fn count_cached_runs(owner: &str, repo: &str, branch: Option<&str>) -> usize {
    let dir = ci_cache_dir(owner, repo, branch);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .count()
}

/// Load all cached CI runs for a given repo.
pub(crate) fn load_all_cached_runs(owner: &str, repo: &str, branch: Option<&str>) -> Vec<CiRun> {
    let dir = ci_cache_dir(owner, repo, branch);
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
    branch: Option<&str>,
    gh_runs: &[GhRun],
) -> (Vec<CiRun>, Option<RepoMetaInfo>, Option<ServiceSignal>) {
    let mut result: Vec<CiRun> = Vec::with_capacity(gh_runs.len());

    // Partition into cached hits and misses.
    let mut uncached: Vec<&GhRun> = Vec::new();
    for gh_run in gh_runs {
        if let Some(cached) = load_cached_run(owner, repo, branch, gh_run.id) {
            result.push(cached);
        } else {
            uncached.push(gh_run);
        }
    }

    // Single GraphQL call: jobs for uncached runs + repo metadata.
    let (batch, signal) = client.batch_fetch_jobs_and_meta(owner, repo, &uncached);
    let (jobs_map, meta) = batch.unwrap_or_default();
    for gh_run in &uncached {
        if let Some(check_runs) = jobs_map.get(&gh_run.id) {
            let ci_run = ci::build_ci_run(gh_run, check_runs.clone(), repo_url);
            save_cached_run(owner, repo, branch, &ci_run);
            result.push(ci_run);
        }
    }

    (result, meta, signal)
}

/// Async version of `fetch_recent_runs` for the concurrent scan phase.
async fn fetch_recent_runs_async(
    client: &HttpClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    branch: Option<&str>,
    gh_runs: &[GhRun],
) -> (Vec<CiRun>, Option<RepoMetaInfo>, Option<ServiceSignal>) {
    let mut result: Vec<CiRun> = Vec::with_capacity(gh_runs.len());

    let mut uncached: Vec<&GhRun> = Vec::new();
    for gh_run in gh_runs {
        if let Some(cached) = load_cached_run(owner, repo, branch, gh_run.id) {
            result.push(cached);
        } else {
            uncached.push(gh_run);
        }
    }

    let (batch, signal) = client
        .batch_fetch_jobs_and_meta_async(owner, repo, &uncached)
        .await;
    let (jobs_map, meta) = batch.unwrap_or_default();
    for gh_run in &uncached {
        if let Some(check_runs) = jobs_map.get(&gh_run.id) {
            let ci_run = ci::build_ci_run(gh_run, check_runs.clone(), repo_url);
            save_cached_run(owner, repo, branch, &ci_run);
            result.push(ci_run);
        }
    }

    (result, meta, signal)
}

/// Async version of `fetch_ci_runs_cached` for the concurrent scan phase.
async fn fetch_ci_runs_cached_async(
    client: &HttpClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    branch: Option<&str>,
    count: u32,
) -> (CiFetchResult, Option<RepoMetaInfo>, Option<ServiceSignal>) {
    let (gh_runs, list_signal) = client.list_runs_async(owner, repo, branch, count).await;
    let gh_runs = gh_runs.unwrap_or_default();
    let (fetched, meta, detail_signal) =
        fetch_recent_runs_async(client, repo_url, owner, repo, branch, &gh_runs).await;
    let cached = load_all_cached_runs(owner, repo, branch);
    let merged = merge_runs(fetched, cached);

    let result = if gh_runs.is_empty() {
        CiFetchResult::CacheOnly(merged)
    } else {
        CiFetchResult::Loaded(merged)
    };
    (
        result,
        meta,
        combine_service_signal(list_signal, detail_signal),
    )
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

    merged.sort_by(|a, b| b.run_id.cmp(&a.run_id));
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
    branch: Option<&str>,
    count: u32,
) -> (CiFetchResult, Option<RepoMetaInfo>, Option<ServiceSignal>) {
    let (gh_runs, list_signal) = client.list_runs(owner, repo, branch, count);
    let gh_runs = gh_runs.unwrap_or_default();
    let (fetched, meta, detail_signal) =
        fetch_recent_runs(client, repo_url, owner, repo, branch, &gh_runs);
    let cached = load_all_cached_runs(owner, repo, branch);
    let merged = merge_runs(fetched, cached);

    let result = if gh_runs.is_empty() {
        CiFetchResult::CacheOnly(merged)
    } else {
        CiFetchResult::Loaded(merged)
    };
    (
        result,
        meta,
        combine_service_signal(list_signal, detail_signal),
    )
}

/// Fetch older CI runs beyond what we currently have, by requesting a
/// larger limit and returning any newly discovered runs.
pub(crate) fn fetch_older_runs(
    client: &HttpClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    branch: Option<&str>,
    current_count: u32,
) -> (CiFetchResult, Option<ServiceSignal>) {
    let fetch_count = current_count + OLDER_RUNS_FETCH_INCREMENT;
    let (gh_runs, list_signal) = client.list_runs(owner, repo, branch, fetch_count);
    let gh_runs = gh_runs.unwrap_or_default();
    let (fetched, _meta, detail_signal) =
        fetch_recent_runs(client, repo_url, owner, repo, branch, &gh_runs);

    let mut result = fetched;
    result.sort_by(|a, b| b.run_id.cmp(&a.run_id));

    let result = if gh_runs.is_empty() {
        CiFetchResult::CacheOnly(result)
    } else {
        CiFetchResult::Loaded(result)
    };
    (result, combine_service_signal(list_signal, detail_signal))
}

/// Re-fetch at the current count to pick up newly created runs without
/// requesting deeper history.
pub(crate) fn fetch_newer_runs(
    client: &HttpClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    branch: Option<&str>,
    current_count: u32,
) -> (CiFetchResult, Option<ServiceSignal>) {
    let (gh_runs, list_signal) = client.list_runs(owner, repo, branch, current_count);
    let gh_runs = gh_runs.unwrap_or_default();
    let (mut result, _meta, detail_signal) =
        fetch_recent_runs(client, repo_url, owner, repo, branch, &gh_runs);
    result.sort_by(|a, b| b.run_id.cmp(&a.run_id));

    let result = if gh_runs.is_empty() {
        CiFetchResult::CacheOnly(result)
    } else {
        CiFetchResult::Loaded(result)
    };
    (result, combine_service_signal(list_signal, detail_signal))
}

pub(crate) struct CratesIoInfo {
    pub version:   String,
    pub downloads: u64,
}

pub(crate) fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Build a project tree from a flat list of discovered `ProjectListItem`s.
///
/// The input must contain only `Workspace`, `Package`, and `NonRust` variants
/// (discovery does not produce worktree groups). This function:
/// 1. Nests workspace members into their parent workspace's `groups`
/// 2. Detects vendored crates nested inside other projects
/// 3. Merges worktree checkouts into `WorktreeGroup` variants
pub(crate) fn build_tree(
    items: &[ProjectListItem],
    inline_dirs: &[String],
) -> Vec<ProjectListItem> {
    let workspace_paths: Vec<PathBuf> = items
        .iter()
        .filter(|item| matches!(item, ProjectListItem::Workspace(_)))
        .map(|item| item.path().to_path_buf())
        .collect();

    let mut result: Vec<ProjectListItem> = Vec::new();
    let mut consumed: HashSet<usize> = HashSet::new();

    // Identify top-level workspaces (not nested inside another workspace).
    let top_level_workspaces: HashSet<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(item, ProjectListItem::Workspace(_))
                && !workspace_paths
                    .iter()
                    .any(|ws| *ws != item.path() && item.path().starts_with(ws))
        })
        .map(|(i, _)| i)
        .collect();

    for (i, item) in items.iter().enumerate() {
        if !top_level_workspaces.contains(&i) {
            continue;
        }
        let ProjectListItem::Workspace(ws) = item else {
            continue;
        };
        let ws_path = ws.path().to_path_buf();
        let member_paths = workspace_member_paths_new(&ws_path, items);

        let mut all_members: Vec<Project<Package>> = items
            .iter()
            .enumerate()
            .filter(|(j, candidate)| {
                *j != i
                    && !top_level_workspaces.contains(j)
                    && member_paths.contains(&candidate.path().to_path_buf())
            })
            .filter_map(|(j, candidate)| {
                consumed.insert(j);
                if let ProjectListItem::Package(pkg) = candidate {
                    Some(pkg.clone())
                } else if let ProjectListItem::Workspace(nested_ws) = candidate {
                    // Nested workspace treated as a package member
                    Some(Project::<Package>::new(
                        nested_ws.path().to_path_buf(),
                        nested_ws.name().map(str::to_string),
                        nested_ws.cargo().clone(),
                        Vec::new(),
                        nested_ws.worktree_name().map(str::to_string),
                        nested_ws.worktree_primary_abs_path().map(Path::to_path_buf),
                    ))
                } else {
                    None
                }
            })
            .collect();

        all_members.sort_by(|a, b| {
            let name_a = a.name().unwrap_or_else(|| a.path().to_str().unwrap_or(""));
            let name_b = b.name().unwrap_or_else(|| b.path().to_str().unwrap_or(""));
            name_a.cmp(name_b)
        });

        let groups = group_members_new(&ws_path, all_members, inline_dirs);

        let mut new_ws = ws.clone();
        *new_ws.groups_mut() = groups;
        consumed.insert(i);
        result.push(ProjectListItem::Workspace(new_ws));
    }

    for (i, item) in items.iter().enumerate() {
        if consumed.contains(&i) {
            continue;
        }
        result.push(item.clone());
    }

    result.sort_by(|a, b| a.path().cmp(b.path()));

    extract_vendored_new(&mut result);
    merge_worktrees_new(&mut result);

    result
}

fn workspace_member_paths_new(ws_path: &Path, items: &[ProjectListItem]) -> HashSet<PathBuf> {
    let manifest = ws_path.join("Cargo.toml");
    let Some((members, excludes)) = workspace_member_patterns(&manifest) else {
        return items
            .iter()
            .filter(|item| item.path().starts_with(ws_path) && item.path() != ws_path)
            .map(|item| item.path().to_path_buf())
            .collect();
    };

    items
        .iter()
        .filter(|item| item.path().starts_with(ws_path) && item.path() != ws_path)
        .filter_map(|item| {
            item.path().strip_prefix(ws_path).ok().and_then(|relative| {
                let relative_str = normalize_workspace_path(relative);
                let included = members
                    .iter()
                    .any(|pattern| workspace_pattern_matches(pattern, &relative_str));
                let is_excluded = excludes
                    .iter()
                    .any(|pattern| workspace_pattern_matches(pattern, &relative_str));
                if included && !is_excluded {
                    Some(item.path().to_path_buf())
                } else {
                    None
                }
            })
        })
        .collect()
}

fn workspace_member_patterns(manifest_path: &Path) -> Option<(Vec<String>, Vec<String>)> {
    let contents = std::fs::read_to_string(manifest_path).ok()?;
    let table: Table = contents.parse().ok()?;
    let workspace = table.get("workspace")?.as_table()?;

    let members = workspace
        .get("members")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(std::string::ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    let excludes = workspace
        .get("exclude")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(std::string::ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    Some((members, excludes))
}

fn normalize_workspace_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(segment) => Some(segment.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn workspace_pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let path_segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    workspace_pattern_matches_segments(&pattern_segments, &path_segments)
}

fn workspace_pattern_matches_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            workspace_pattern_matches_segments(rest, path)
                || (!path.is_empty() && workspace_pattern_matches_segments(pattern, &path[1..]))
        },
        Some((segment, rest)) => {
            !path.is_empty()
                && workspace_pattern_matches_segment(segment, path[0])
                && workspace_pattern_matches_segments(rest, &path[1..])
        },
    }
}

fn workspace_pattern_matches_segment(pattern: &str, value: &str) -> bool {
    fn matches(pattern: &[u8], value: &[u8]) -> bool {
        match pattern.split_first() {
            None => value.is_empty(),
            Some((b'*', rest)) => {
                matches(rest, value) || (!value.is_empty() && matches(pattern, &value[1..]))
            },
            Some((b'?', rest)) => !value.is_empty() && matches(rest, &value[1..]),
            Some((head, rest)) => {
                !value.is_empty() && *head == value[0] && matches(rest, &value[1..])
            },
        }
    }

    matches(pattern.as_bytes(), value.as_bytes())
}

/// Group worktree checkouts under their primary project.
///
/// Projects sharing the same `worktree_primary_abs_path` are grouped.
/// Workspaces → `WorkspaceWorktrees`, Packages → `PackageWorktrees`.
/// `NonRust` projects are not grouped into worktree variants.
fn item_worktree_identity(item: &ProjectListItem) -> Option<&Path> {
    match item {
        ProjectListItem::Workspace(p) => p.worktree_primary_abs_path(),
        ProjectListItem::Package(p) => p.worktree_primary_abs_path(),
        ProjectListItem::NonRust(p) => p.worktree_primary_abs_path(),
        _ => None,
    }
}

fn item_is_linked(item: &ProjectListItem) -> bool {
    match item {
        ProjectListItem::Workspace(p) => p.worktree_name().is_some(),
        ProjectListItem::Package(p) => p.worktree_name().is_some(),
        ProjectListItem::NonRust(p) => p.worktree_name().is_some(),
        _ => false,
    }
}

fn merge_worktrees_new(items: &mut Vec<ProjectListItem>) {
    let mut primary_indices: HashMap<PathBuf, usize> = HashMap::new();
    let mut worktree_indices: Vec<usize> = Vec::new();

    for (i, item) in items.iter().enumerate() {
        let Some(identity) = item_worktree_identity(item) else {
            continue;
        };
        let is_linked = item_is_linked(item);
        if is_linked {
            worktree_indices.push(i);
        } else {
            primary_indices.insert(identity.to_path_buf(), i);
        }
    }

    let identities_with_worktrees: HashSet<PathBuf> = worktree_indices
        .iter()
        .filter_map(|&wi| {
            item_worktree_identity(&items[wi])
                .filter(|id| primary_indices.contains_key(*id))
                .map(Path::to_path_buf)
        })
        .collect();

    if identities_with_worktrees.is_empty() {
        return;
    }

    // Extract worktree items (highest index first to preserve lower indices)
    let mut moves: Vec<(usize, PathBuf)> = worktree_indices
        .iter()
        .filter_map(|&wi| {
            let id = item_worktree_identity(&items[wi])?.to_path_buf();
            primary_indices.get(&id)?;
            Some((wi, id))
        })
        .collect();
    moves.sort_by(|a, b| b.0.cmp(&a.0));

    let mut extracted: Vec<(ProjectListItem, PathBuf)> = Vec::new();
    for (wi, id) in moves {
        let item = items.remove(wi);
        extracted.push((item, id));
    }

    // Rebuild primary_indices after removals
    let mut primary_map: HashMap<PathBuf, usize> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        if let Some(id) = item_worktree_identity(item)
            .filter(|id| identities_with_worktrees.contains(*id))
            .filter(|_| !item_is_linked(item))
        {
            primary_map.insert(id.to_path_buf(), i);
        }
    }

    // Group linked worktrees by identity, preserving order
    let mut linked_by_id: HashMap<PathBuf, Vec<ProjectListItem>> = HashMap::new();
    for (item, id) in extracted {
        linked_by_id.entry(id).or_default().push(item);
    }

    // Replace each primary with its WorktreeGroup variant
    // Process in reverse to avoid index shifting
    let mut replacements: Vec<(usize, ProjectListItem)> = Vec::new();
    for (id, idx) in &primary_map {
        let linked = linked_by_id.remove(id).unwrap_or_default();
        let primary_item = &items[*idx];
        let replacement = match primary_item {
            ProjectListItem::Workspace(ws) => {
                let linked_ws: Vec<Project<Workspace>> = linked
                    .into_iter()
                    .filter_map(|item| {
                        if let ProjectListItem::Workspace(linked_ws) = item {
                            Some(linked_ws)
                        } else {
                            None
                        }
                    })
                    .collect();
                ProjectListItem::WorkspaceWorktrees(WorktreeGroup::new(ws.clone(), linked_ws))
            },
            ProjectListItem::Package(pkg) => {
                let linked_pkg: Vec<Project<Package>> = linked
                    .into_iter()
                    .filter_map(|item| {
                        if let ProjectListItem::Package(linked_pkg) = item {
                            Some(linked_pkg)
                        } else {
                            None
                        }
                    })
                    .collect();
                ProjectListItem::PackageWorktrees(WorktreeGroup::new(pkg.clone(), linked_pkg))
            },
            _ => continue,
        };
        replacements.push((*idx, replacement));
    }

    for (idx, replacement) in replacements {
        items[idx] = replacement;
    }
}

/// Find standalone items whose path lives inside another item's directory
/// and move them into that item's `vendored` list.
fn extract_vendored_new(items: &mut Vec<ProjectListItem>) {
    let parent_paths: Vec<(usize, PathBuf)> = items
        .iter()
        .enumerate()
        .map(|(i, item)| (i, item.path().to_path_buf()))
        .collect();

    let mut vendored_map: Vec<(usize, usize)> = Vec::new();

    for (vi, vitem) in items.iter().enumerate() {
        let has_structure = match vitem {
            ProjectListItem::Workspace(ws) => ws.groups().iter().any(|g| !g.members().is_empty()),
            ProjectListItem::WorkspaceWorktrees(_) | ProjectListItem::PackageWorktrees(_) => true,
            _ => false,
        };
        if has_structure {
            continue;
        }
        for &(ni, ref parent_path) in &parent_paths {
            if ni == vi {
                continue;
            }
            if vitem.path().starts_with(parent_path) && vitem.path() != parent_path {
                vendored_map.push((vi, ni));
                break;
            }
        }
    }

    if vendored_map.is_empty() {
        return;
    }

    let mut remove_indices: Vec<usize> = vendored_map.iter().map(|&(vi, _)| vi).collect();
    remove_indices.sort_unstable();
    remove_indices.dedup();

    // Convert vendored items to Project<Package>
    let mut vendored_projects: Vec<(usize, Project<Package>)> = Vec::new();
    for &(vi, ni) in &vendored_map {
        let pkg = match &items[vi] {
            ProjectListItem::Package(p) => p.clone(),
            ProjectListItem::Workspace(ws) => Project::<Package>::new(
                ws.path().to_path_buf(),
                ws.name().map(str::to_string),
                ws.cargo().clone(),
                Vec::new(),
                ws.worktree_name().map(str::to_string),
                ws.worktree_primary_abs_path().map(Path::to_path_buf),
            ),
            ProjectListItem::NonRust(nr) => Project::<Package>::new(
                nr.path().to_path_buf(),
                nr.name().map(str::to_string),
                Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
                Vec::new(),
                nr.worktree_name().map(str::to_string),
                nr.worktree_primary_abs_path().map(Path::to_path_buf),
            ),
            _ => continue,
        };
        vendored_projects.push((ni, pkg));
    }

    for &idx in remove_indices.iter().rev() {
        items.remove(idx);
    }

    for (ni, pkg) in vendored_projects {
        let adjusted_ni = remove_indices.iter().filter(|&&r| r < ni).count();
        let target_ni = ni - adjusted_ni;
        if let Some(item) = items.get_mut(target_ni) {
            match item {
                ProjectListItem::Workspace(ws) => ws.vendored_mut().push(pkg),
                ProjectListItem::Package(p) => p.vendored_mut().push(pkg),
                _ => {},
            }
        }
    }

    // Sort vendored lists
    for item in items {
        match item {
            ProjectListItem::Workspace(ws) => {
                ws.vendored_mut().sort_by(|a, b| a.path().cmp(b.path()));
            },
            ProjectListItem::Package(pkg) => {
                pkg.vendored_mut().sort_by(|a, b| a.path().cmp(b.path()));
            },
            _ => {},
        }
    }
}

fn group_members_new(
    workspace_path: &Path,
    members: Vec<Project<Package>>,
    inline_dirs: &[String],
) -> Vec<MemberGroup> {
    let mut group_map: HashMap<String, Vec<Project<Package>>> = HashMap::new();

    for member in members {
        let relative = member
            .path()
            .strip_prefix(workspace_path)
            .ok()
            .map(normalize_workspace_path)
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
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => a.group_name().cmp(b.group_name()),
        }
    });

    groups
}

/// Build a flat list of entries for fuzzy search from the project tree.
pub(crate) fn build_flat_entries(items: &[ProjectListItem]) -> Vec<FlatEntry> {
    let mut entries = Vec::new();
    for item in items {
        entries.push(FlatEntry {
            path:     item.display_path(),
            abs_path: item.path().to_path_buf(),
            name:     item.display_name(),
        });

        match item {
            ProjectListItem::Workspace(ws) => {
                for group in ws.groups() {
                    for member in group.members() {
                        entries.push(FlatEntry {
                            path:     member.display_path(),
                            abs_path: member.path().to_path_buf(),
                            name:     member.display_name(),
                        });
                    }
                }
                for vendored in ws.vendored() {
                    entries.push(FlatEntry {
                        path:     vendored.display_path(),
                        abs_path: vendored.path().to_path_buf(),
                        name:     format!("{} (vendored)", vendored.display_name()),
                    });
                }
            },
            ProjectListItem::Package(pkg) => {
                for vendored in pkg.vendored() {
                    entries.push(FlatEntry {
                        path:     vendored.display_path(),
                        abs_path: vendored.path().to_path_buf(),
                        name:     format!("{} (vendored)", vendored.display_name()),
                    });
                }
            },
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                for linked in wtg.linked() {
                    entries.push(FlatEntry {
                        path:     linked.display_path(),
                        abs_path: linked.path().to_path_buf(),
                        name:     linked
                            .worktree_name()
                            .unwrap_or_else(|| linked.name().unwrap_or(""))
                            .to_string(),
                    });
                    for group in linked.groups() {
                        for member in group.members() {
                            entries.push(FlatEntry {
                                path:     member.display_path(),
                                abs_path: member.path().to_path_buf(),
                                name:     member.display_name(),
                            });
                        }
                    }
                }
                // Also emit primary's groups
                for group in wtg.primary().groups() {
                    for member in group.members() {
                        entries.push(FlatEntry {
                            path:     member.display_path(),
                            abs_path: member.path().to_path_buf(),
                            name:     member.display_name(),
                        });
                    }
                }
                for vendored in wtg.primary().vendored() {
                    entries.push(FlatEntry {
                        path:     vendored.display_path(),
                        abs_path: vendored.path().to_path_buf(),
                        name:     format!("{} (vendored)", vendored.display_name()),
                    });
                }
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                for linked in wtg.linked() {
                    entries.push(FlatEntry {
                        path:     linked.display_path(),
                        abs_path: linked.path().to_path_buf(),
                        name:     linked
                            .worktree_name()
                            .unwrap_or_else(|| linked.name().unwrap_or(""))
                            .to_string(),
                    });
                }
            },
            ProjectListItem::NonRust(_) => {},
        }
    }
    entries
}

/// Convert a `CargoProject` (from `from_cargo_toml()`) into a `ProjectListItem`.
pub(crate) fn cargo_project_to_item(cp: CargoProject) -> ProjectListItem {
    match cp {
        CargoProject::Workspace(ws) => ProjectListItem::Workspace(ws),
        CargoProject::Package(pkg) => ProjectListItem::Package(pkg),
    }
}

/// Shared network context passed to `fetch_project_details`.
pub(crate) struct FetchContext {
    pub client:     HttpClient,
    pub repo_cache: RepoCache,
}

pub(crate) struct ProjectDetailRequest<'a> {
    pub tx:            &'a mpsc::Sender<BackgroundMsg>,
    pub ctx:           &'a FetchContext,
    pub _project_path: &'a str,
    pub abs_path:      &'a Path,
    pub project_name:  Option<&'a str>,
    pub repo_presence: GitRepoPresence,
    pub ci_run_count:  u32,
}

/// Fetch all details (disk, git, crates.io, CI) for a single project and send
/// results through the provided channel. Used by both the main scan and priority fetch.
pub(crate) fn fetch_project_details(req: &ProjectDetailRequest<'_>) {
    let tx = req.tx;
    let ctx = req.ctx;
    let abs_path = req.abs_path;
    let project_name = req.project_name;
    let repo_presence = req.repo_presence;
    let ci_run_count = req.ci_run_count;
    let client = &ctx.client;
    let repo_cache = &ctx.repo_cache;
    let _ = tx.send(BackgroundMsg::GitPathState {
        path:  abs_path.to_string_lossy().into_owned(),
        state: super::project::detect_git_path_state(abs_path),
    });
    // Git info first (local, instant) — also provides the repo URL for CI cache lookup
    let git_info = if repo_presence.is_in_repo() {
        GitInfo::detect(abs_path)
    } else {
        None
    };
    if let Some(ref info) = git_info {
        let _ = tx.send(BackgroundMsg::GitInfo {
            path: abs_path.to_string_lossy().into_owned(),
            info: info.clone(),
        });
    }

    // CI runs + repo metadata — deduplicated across worktrees of the
    // same repo. First thread to reach a given `owner/repo` does the
    // HTTP calls; subsequent threads reuse the cached result.
    if let Some(ref repo_url) = git_info.as_ref().and_then(|g| g.url.clone())
        && let Some((owner, repo)) = ci::parse_owner_repo(repo_url)
    {
        let branch = git_info.as_ref().and_then(|git| git.branch.as_deref());
        let cache_key = repo_dispatch_key(&owner, &repo, branch);
        let cached = repo_cache
            .lock()
            .ok()
            .and_then(|c| c.get(&cache_key).cloned());

        let data = cached.unwrap_or_else(|| {
            let (result, meta, signal) =
                fetch_ci_runs_cached(client, repo_url, &owner, &repo, branch, ci_run_count);
            emit_service_signal(tx, signal);
            let runs = match result {
                CiFetchResult::Loaded(r) | CiFetchResult::CacheOnly(r) => r,
            };
            let data = CachedRepoData { runs, meta };
            if let Ok(mut c) = repo_cache.lock() {
                c.insert(cache_key, data.clone());
            }
            data
        });

        let _ = tx.send(BackgroundMsg::CiRuns {
            path: abs_path.to_string_lossy().into_owned(),
            runs: data.runs,
        });
        if let Some(meta) = data.meta {
            let _ = tx.send(BackgroundMsg::RepoMeta {
                path:        abs_path.to_string_lossy().into_owned(),
                stars:       meta.stars,
                description: meta.description,
            });
        }
    }

    // Crates.io version + downloads (network)
    if let Some(name) = project_name {
        let (info, signal) = client.fetch_crates_io_info(name);
        emit_service_signal(tx, signal);
        if let Some(info) = info {
            let _ = tx.send(BackgroundMsg::CratesIoVersion {
                path:      abs_path.to_string_lossy().into_owned(),
                version:   info.version,
                downloads: info.downloads,
            });
        }
    }

    // Disk usage last — walking large `target/` dirs is the slowest
    // local operation and doesn't block anything else.
    let bytes = dir_size(abs_path);
    let _ = tx.send(BackgroundMsg::DiskUsage {
        path: abs_path.to_string_lossy().into_owned(),
        bytes,
    });
}

#[derive(Clone)]
pub(crate) struct RepoMetaInfo {
    pub stars:       u64,
    pub description: Option<String>,
}

/// Cached CI + metadata results keyed by `"owner/repo"`. Shared across
/// rayon threads so worktrees on the same repo+branch don't make duplicate
/// HTTP calls.
#[derive(Clone)]
pub(crate) struct CachedRepoData {
    runs: Vec<CiRun>,
    meta: Option<RepoMetaInfo>,
}

pub(crate) type RepoCache = Arc<Mutex<HashMap<String, CachedRepoData>>>;

pub(crate) fn new_repo_cache() -> RepoCache { Arc::new(Mutex::new(HashMap::new())) }

/// Resolve include-dir entries to absolute paths. Relative entries are
/// joined to `scan_root`; absolute entries are used as-is. An empty
/// list falls back to `[scan_root]` so the whole tree is walked.
pub(crate) fn resolve_include_dirs(scan_root: &Path, include_dirs: &[String]) -> Vec<PathBuf> {
    if include_dirs.is_empty() {
        return vec![scan_root.to_path_buf()];
    }
    include_dirs
        .iter()
        .map(|dir| {
            let expanded = expand_home_path(dir);
            let path = expanded.as_path();
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                scan_root.join(path)
            }
        })
        .collect()
}

fn expand_home_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return dirs::home_dir().map_or_else(|| PathBuf::from(raw), |home| home.join(rest));
    }
    PathBuf::from(raw)
}

/// Information collected in phase 1 (local work) for a single project,
/// used to drive async HTTP dispatch and disk usage.
#[derive(Clone)]
struct DiscoveredProject {
    path:       String,
    abs_path:   PathBuf,
    name:       Option<String>,
    repo_url:   Option<String>,
    owner_repo: Option<(String, String)>,
    branch:     Option<String>,
}

enum RepoDispatchState {
    Pending(Vec<String>),
    Ready(CachedRepoData),
}

enum RepoDispatchRegistration {
    Cached(CachedRepoData),
    SpawnFetch,
    Pending,
}

type RepoDispatchMap = Arc<Mutex<HashMap<String, RepoDispatchState>>>;
type GitInfoCache = Arc<Mutex<HashMap<PathBuf, Option<GitInfo>>>>;

#[derive(Clone)]
struct StreamingScanContext {
    client:         HttpClient,
    tx:             mpsc::Sender<BackgroundMsg>,
    ci_run_count:   u32,
    disk_limit:     Arc<tokio::sync::Semaphore>,
    http_limit:     Arc<tokio::sync::Semaphore>,
    local_limit:    Arc<tokio::sync::Semaphore>,
    repo_dispatch:  RepoDispatchMap,
    git_info_cache: GitInfoCache,
}

struct RepoFetchRequest {
    key:          String,
    project_path: String,
    repo_url:     String,
    owner:        String,
    repo:         String,
    branch:       Option<String>,
}

/// Spawn a streaming scan using a hybrid approach:
///
/// - **Discovery (scan thread):** Walk the directory tree, discover projects, and emit rows
///   quickly.
/// - **Local enrichment (tokio blocking pool):** Git info runs behind its own semaphore so it does
///   not block discovery.
/// - **Disk usage (tokio blocking pool):** `dir_size()` runs behind its own semaphore so disk walks
///   cannot monopolize startup.
/// - **HTTP (tokio):** CI runs, repo metadata, crates.io info, and connectivity checks run on the
///   async runtime behind a shared semaphore.
///
/// `ScanComplete` is sent after discovery/local work has finished. Disk and HTTP results may
/// continue to stream in afterward.
pub(crate) fn spawn_streaming_scan(
    scan_root: &Path,
    ci_run_count: u32,
    include_dirs: &[String],
    non_rust: NonRustInclusion,
    client: HttpClient,
) -> (mpsc::Sender<BackgroundMsg>, Receiver<BackgroundMsg>) {
    let (tx, rx) = mpsc::channel();
    let root = scan_root.to_path_buf();
    let scan_dirs = resolve_include_dirs(&root, include_dirs);

    let scan_tx = tx.clone();
    thread::spawn(move || {
        let scan_context = StreamingScanContext {
            client,
            tx: scan_tx.clone(),
            ci_run_count,
            disk_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_DISK_CONCURRENCY)),
            http_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_HTTP_CONCURRENCY)),
            local_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_LOCAL_CONCURRENCY)),
            repo_dispatch: Arc::new(Mutex::new(HashMap::new())),
            git_info_cache: Arc::new(Mutex::new(HashMap::new())),
        };

        let phase1_started = std::time::Instant::now();
        let phase1 = phase1_discover(&scan_dirs, non_rust, &scan_context);
        perf_log::log_duration(
            "phase1_discover_total",
            phase1_started.elapsed(),
            &format!(
                "scan_dirs={} visited_dirs={} manifests={} projects={} non_rust_projects={} disk_entries={}",
                scan_dirs.len(),
                phase1.stats.visited_dirs,
                phase1.stats.manifests,
                phase1.stats.projects,
                phase1.stats.non_rust_projects,
                phase1.disk_entries.len()
            ),
            0,
        );
        let _ = scan_tx.send(BackgroundMsg::ScanComplete);
        spawn_initial_disk_usage(&scan_context, &phase1.disk_entries);
    });

    (tx, rx)
}

/// Walk `scan_dirs`, discover projects, and stream per-project work immediately. Discovery and
/// local metadata collection stay on the dedicated scan thread, while disk and network work are
/// dispatched onto bounded background queues.
struct Phase1DiscoverStats {
    visited_dirs:      usize,
    manifests:         usize,
    projects:          usize,
    non_rust_projects: usize,
}

struct Phase1DiscoverResult {
    disk_entries: Vec<(String, PathBuf)>,
    stats:        Phase1DiscoverStats,
}

fn discover_non_rust_project(
    scan_context: &StreamingScanContext,
    entry_path: &Path,
    disk_entries: &mut Vec<(String, PathBuf)>,
    stats: &mut Phase1DiscoverStats,
) {
    let project = super::project::from_git_dir(entry_path);
    let abs_path = project.path().to_path_buf();
    let display_path = project.display_path();
    stats.projects += 1;
    stats.non_rust_projects += 1;

    let item = ProjectListItem::NonRust(project);
    let _ = scan_context
        .tx
        .send(BackgroundMsg::ProjectDiscovered { item });

    let discovered = DiscoveredProject {
        path:       display_path,
        abs_path:   abs_path.clone(),
        name:       None,
        repo_url:   None,
        owner_repo: None,
        branch:     None,
    };
    let disk_path = abs_path.to_string_lossy().into_owned();
    spawn_project_local_work(scan_context, discovered, GitRepoPresence::InRepo);
    disk_entries.push((disk_path, abs_path));
}

fn phase1_discover(
    scan_dirs: &[PathBuf],
    non_rust: NonRustInclusion,
    scan_context: &StreamingScanContext,
) -> Phase1DiscoverResult {
    let mut disk_entries = Vec::new();
    let mut stats = Phase1DiscoverStats {
        visited_dirs:      0,
        manifests:         0,
        projects:          0,
        non_rust_projects: 0,
    };
    for dir in scan_dirs {
        if !dir.is_dir() {
            continue;
        }
        let mut iter = WalkDir::new(dir).into_iter();
        while let Some(Ok(entry)) = iter.next() {
            if entry.file_type().is_dir() {
                stats.visited_dirs += 1;
                let name = entry.file_name();
                if name == "target" || name == ".git" {
                    iter.skip_current_dir();
                    continue;
                }

                if non_rust.includes_non_rust()
                    && entry.path().join(".git").is_dir()
                    && !entry.path().join("Cargo.toml").exists()
                {
                    iter.skip_current_dir();
                    discover_non_rust_project(
                        scan_context,
                        entry.path(),
                        &mut disk_entries,
                        &mut stats,
                    );
                    continue;
                }
            }
            if entry.file_type().is_file() && entry.file_name() == "Cargo.toml" {
                stats.manifests += 1;
                let manifest_started = std::time::Instant::now();
                let Ok(cargo_project) = super::project::from_cargo_toml(entry.path()) else {
                    continue;
                };
                perf_log::log_duration(
                    "phase1_manifest_parse",
                    manifest_started.elapsed(),
                    &format!("manifest={}", entry.path().display()),
                    0,
                );
                stats.projects += 1;
                let item = cargo_project_to_item(cargo_project);
                let abs_path = item.path().to_path_buf();
                let display_path = item.display_path();
                let project_name = item.name().map(str::to_string);
                let repo_presence_started = std::time::Instant::now();
                let repo_presence = if super::project::git_repo_root(&abs_path).is_some() {
                    GitRepoPresence::InRepo
                } else {
                    GitRepoPresence::OutsideRepo
                };
                perf_log::log_duration(
                    "phase1_repo_presence",
                    repo_presence_started.elapsed(),
                    &format!(
                        "path={} in_repo={}",
                        display_path,
                        repo_presence.is_in_repo()
                    ),
                    0,
                );

                let _ = scan_context
                    .tx
                    .send(BackgroundMsg::ProjectDiscovered { item });

                let discovered = DiscoveredProject {
                    path:       display_path,
                    abs_path:   abs_path.clone(),
                    name:       project_name,
                    repo_url:   None,
                    owner_repo: None,
                    branch:     None,
                };
                spawn_project_local_work(scan_context, discovered.clone(), repo_presence);
                disk_entries.push((abs_path.to_string_lossy().into_owned(), abs_path));
            }
        }
    }
    Phase1DiscoverResult {
        disk_entries,
        stats,
    }
}

fn spawn_initial_disk_usage(
    scan_context: &StreamingScanContext,
    disk_entries: &[(String, PathBuf)],
) {
    for tree in group_disk_usage_trees(disk_entries) {
        spawn_disk_usage_tree(scan_context, tree);
    }
}

fn spawn_project_http(scan_context: &StreamingScanContext, project: &DiscoveredProject) {
    if let Some((owner, repo)) = &project.owner_repo {
        let key = repo_dispatch_key(owner, repo, project.branch.as_deref());
        let abs_path_str = project.abs_path.to_string_lossy().into_owned();
        match register_repo_path(&scan_context.repo_dispatch, &key, &abs_path_str) {
            RepoDispatchRegistration::Cached(data) => {
                send_repo_data(&scan_context.tx, std::slice::from_ref(&abs_path_str), &data);
            },
            RepoDispatchRegistration::SpawnFetch => {
                let _ = scan_context
                    .tx
                    .send(BackgroundMsg::RepoFetchQueued { key: key.clone() });
                spawn_repo_fetch(
                    scan_context,
                    RepoFetchRequest {
                        key,
                        project_path: project.path.clone(),
                        repo_url: project.repo_url.clone().unwrap_or_default(),
                        owner: owner.clone(),
                        repo: repo.clone(),
                        branch: project.branch.clone(),
                    },
                );
            },
            RepoDispatchRegistration::Pending => {},
        }
    }

    if let Some(name) = &project.name {
        spawn_crates_fetch(
            &scan_context.client,
            &scan_context.tx,
            &scan_context.http_limit,
            &project.abs_path.to_string_lossy(),
            name,
        );
    }
}

fn repo_dispatch_key(owner: &str, repo: &str, branch: Option<&str>) -> String {
    branch.map_or_else(
        || format!("{owner}/{repo}"),
        |branch| format!("{owner}/{repo}@{branch}"),
    )
}

fn spawn_project_local_work(
    scan_context: &StreamingScanContext,
    project: DiscoveredProject,
    repo_presence: GitRepoPresence,
) {
    let handle = scan_context.client.handle.clone();
    let tx = scan_context.tx.clone();
    let git_info_cache = Arc::clone(&scan_context.git_info_cache);
    let local_limit = Arc::clone(&scan_context.local_limit);
    let scan_context = scan_context.clone();
    if repo_presence.is_in_repo() {
        let _ = tx.send(BackgroundMsg::LocalGitQueued {
            path: project.abs_path.to_string_lossy().into_owned(),
        });
    }

    handle.spawn(async move {
        let queue_started = std::time::Instant::now();
        let Ok(_permit) = local_limit.acquire_owned().await else {
            return;
        };
        perf_log::log_duration(
            "tokio_local_queue_wait",
            queue_started.elapsed(),
            &format!(
                "path={} abs_path={}",
                project.path,
                project.abs_path.display()
            ),
            0,
        );
        let run_started = std::time::Instant::now();
        let tx_for_work = tx.clone();
        let git_info_cache_for_work = Arc::clone(&git_info_cache);
        let project_for_work = project.clone();
        let Ok(discovered) = tokio::task::spawn_blocking(move || {
            phase1_local_work(
                &tx_for_work,
                &git_info_cache_for_work,
                project_for_work,
                repo_presence,
            )
        })
        .await
        else {
            return;
        };
        perf_log::log_duration(
            "tokio_local_work",
            run_started.elapsed(),
            &format!(
                "path={} abs_path={}",
                discovered.path,
                discovered.abs_path.display()
            ),
            0,
        );
        spawn_project_http(&scan_context, &discovered);
    });
}

#[derive(Clone)]
struct DiskUsageTree {
    root_path:     String,
    root_abs_path: PathBuf,
    entries:       Vec<(String, PathBuf)>,
}

fn group_disk_usage_trees(disk_entries: &[(String, PathBuf)]) -> Vec<DiskUsageTree> {
    let mut sorted = disk_entries.to_vec();
    sorted.sort_by(|(_, left), (_, right)| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });

    let mut trees: Vec<DiskUsageTree> = Vec::new();
    for (path, abs_path) in sorted {
        if let Some(tree) = trees
            .iter_mut()
            .find(|tree| abs_path.starts_with(&tree.root_abs_path))
        {
            tree.entries.push((path, abs_path));
        } else {
            trees.push(DiskUsageTree {
                root_path:     path.clone(),
                root_abs_path: abs_path.clone(),
                entries:       vec![(path, abs_path)],
            });
        }
    }
    trees
}

fn spawn_disk_usage_tree(scan_context: &StreamingScanContext, tree: DiskUsageTree) {
    let handle = scan_context.client.handle.clone();
    let tx = scan_context.tx.clone();
    let disk_limit = Arc::clone(&scan_context.disk_limit);

    handle.spawn(async move {
        let queue_started = std::time::Instant::now();
        let Ok(_permit) = disk_limit.acquire_owned().await else {
            return;
        };
        let queue_elapsed = queue_started.elapsed();
        perf_log::log_duration(
            "tokio_disk_queue_wait",
            queue_elapsed,
            &format!(
                "path={} abs_path={} rows={}",
                tree.root_path,
                tree.root_abs_path.display(),
                tree.entries.len()
            ),
            0,
        );
        let run_started = std::time::Instant::now();
        let tree_for_walk = tree.clone();
        let Ok(results) =
            tokio::task::spawn_blocking(move || dir_sizes_for_tree(&tree_for_walk)).await
        else {
            return;
        };
        perf_log::log_duration(
            "tokio_disk_usage",
            run_started.elapsed(),
            &format!(
                "path={} abs_path={} rows={}",
                tree.root_path,
                tree.root_abs_path.display(),
                tree.entries.len()
            ),
            0,
        );
        let _ = tx.send(BackgroundMsg::DiskUsageBatch {
            root_path: tree.root_path,
            entries:   results,
        });
    });
}

fn dir_sizes_for_tree(tree: &DiskUsageTree) -> Vec<(String, u64)> {
    let mut totals: HashMap<String, u64> = tree
        .entries
        .iter()
        .map(|(path, _)| (path.clone(), 0))
        .collect();
    let entry_paths_by_abs: HashMap<PathBuf, Vec<String>> =
        tree.entries
            .iter()
            .fold(HashMap::new(), |mut acc, (path, abs_path)| {
                acc.entry(abs_path.clone()).or_default().push(path.clone());
                acc
            });

    for entry in WalkDir::new(&tree.root_abs_path).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let bytes = metadata.len();
        let mut current = entry.path().parent();
        while let Some(dir) = current {
            if let Some(paths) = entry_paths_by_abs.get(dir) {
                for path in paths {
                    *totals.entry(path.clone()).or_default() += bytes;
                }
            }
            if dir == tree.root_abs_path {
                break;
            }
            current = dir.parent();
        }
    }

    tree.entries
        .iter()
        .map(|(path, _)| (path.clone(), totals.get(path).copied().unwrap_or(0)))
        .collect()
}

fn register_repo_path(
    repo_dispatch: &RepoDispatchMap,
    key: &str,
    path: &str,
) -> RepoDispatchRegistration {
    let Ok(mut dispatch) = repo_dispatch.lock() else {
        return RepoDispatchRegistration::SpawnFetch;
    };
    let state = dispatch
        .entry(key.to_string())
        .or_insert_with(|| RepoDispatchState::Pending(vec![path.to_string()]));

    match state {
        RepoDispatchState::Pending(paths) => {
            if paths.iter().all(|known_path| known_path != path) {
                paths.push(path.to_string());
            }
            if paths.len() == 1 {
                RepoDispatchRegistration::SpawnFetch
            } else {
                RepoDispatchRegistration::Pending
            }
        },
        RepoDispatchState::Ready(data) => RepoDispatchRegistration::Cached(data.clone()),
    }
}

fn finish_repo_fetch(
    repo_dispatch: &RepoDispatchMap,
    key: &str,
    data: CachedRepoData,
) -> Vec<String> {
    let Ok(mut dispatch) = repo_dispatch.lock() else {
        return Vec::new();
    };
    let previous = dispatch.insert(key.to_string(), RepoDispatchState::Ready(data));
    match previous {
        Some(RepoDispatchState::Pending(paths)) => paths,
        Some(RepoDispatchState::Ready(_)) | None => Vec::new(),
    }
}

fn send_repo_data(tx: &mpsc::Sender<BackgroundMsg>, paths: &[String], data: &CachedRepoData) {
    for path in paths {
        let _ = tx.send(BackgroundMsg::CiRuns {
            path: path.clone(),
            runs: data.runs.clone(),
        });
        if let Some(meta) = &data.meta {
            let _ = tx.send(BackgroundMsg::RepoMeta {
                path:        path.clone(),
                stars:       meta.stars,
                description: meta.description.clone(),
            });
        }
    }
}

fn spawn_repo_fetch(scan_context: &StreamingScanContext, request: RepoFetchRequest) {
    let client = scan_context.client.clone();
    let handle = client.handle.clone();
    let tx = scan_context.tx.clone();
    let http_limit = Arc::clone(&scan_context.http_limit);
    let repo_dispatch = Arc::clone(&scan_context.repo_dispatch);
    let ci_run_count = scan_context.ci_run_count;

    handle.spawn(async move {
        let queue_started = std::time::Instant::now();
        let Ok(_permit) = http_limit.acquire_owned().await else {
            return;
        };
        perf_log::log_duration(
            "tokio_repo_fetch_queue_wait",
            queue_started.elapsed(),
            &format!(
                "path={} repo={}/{} branch={}",
                request.project_path,
                request.owner,
                request.repo,
                request.branch.as_deref().unwrap_or("-")
            ),
            0,
        );
        let fetch_started = std::time::Instant::now();
        let (result, meta, signal) = fetch_ci_runs_cached_async(
            &client,
            &request.repo_url,
            &request.owner,
            &request.repo,
            request.branch.as_deref(),
            ci_run_count,
        )
        .await;
        emit_service_signal(&tx, signal);
        let data = CachedRepoData {
            runs: match result {
                CiFetchResult::Loaded(runs) | CiFetchResult::CacheOnly(runs) => runs,
            },
            meta,
        };
        perf_log::log_duration(
            "tokio_repo_fetch",
            fetch_started.elapsed(),
            &format!(
                "path={} repo={}/{} branch={} runs={}",
                request.project_path,
                request.owner,
                request.repo,
                request.branch.as_deref().unwrap_or("-"),
                data.runs.len()
            ),
            0,
        );
        let paths = finish_repo_fetch(&repo_dispatch, &request.key, data.clone());
        send_repo_data(&tx, &paths, &data);
        let _ = tx.send(BackgroundMsg::RepoFetchComplete { key: request.key });
    });
}

fn spawn_crates_fetch(
    client: &HttpClient,
    tx: &mpsc::Sender<BackgroundMsg>,
    http_limit: &Arc<tokio::sync::Semaphore>,
    project_path: &str,
    crate_name: &str,
) {
    let client = client.clone();
    let handle = client.handle.clone();
    let tx = tx.clone();
    let http_limit = Arc::clone(http_limit);
    let project_path = project_path.to_string();
    let crate_name = crate_name.to_string();

    handle.spawn(async move {
        let queue_started = std::time::Instant::now();
        let Ok(_permit) = http_limit.acquire_owned().await else {
            return;
        };
        perf_log::log_duration(
            "tokio_crates_fetch_queue_wait",
            queue_started.elapsed(),
            &format!("path={project_path} crate={crate_name}"),
            0,
        );
        let fetch_started = std::time::Instant::now();
        let (info, signal) = client.fetch_crates_io_info_async(&crate_name).await;
        emit_service_signal(&tx, signal);
        perf_log::log_duration(
            "tokio_crates_fetch",
            fetch_started.elapsed(),
            &format!(
                "path={project_path} crate={crate_name} found={}",
                info.is_some()
            ),
            0,
        );
        if let Some(info) = info {
            let _ = tx.send(BackgroundMsg::CratesIoVersion {
                path:      project_path,
                version:   info.version,
                downloads: info.downloads,
            });
        }
    });
}

/// Phase 1 local work: git info for a single project.
/// Returns the discovered repo metadata needed for async HTTP dispatch.
fn phase1_local_work(
    tx: &mpsc::Sender<BackgroundMsg>,
    git_info_cache: &GitInfoCache,
    mut project: DiscoveredProject,
    repo_presence: GitRepoPresence,
) -> DiscoveredProject {
    let started = std::time::Instant::now();
    let git_info = if repo_presence.is_in_repo() {
        cached_git_info(git_info_cache, &project.abs_path)
    } else {
        None
    };
    if let Some(ref info) = git_info {
        let _ = tx.send(BackgroundMsg::GitInfo {
            path: project.path.clone(),
            info: info.clone(),
        });
    }

    project.repo_url = git_info.as_ref().and_then(|g| g.url.clone());
    project.owner_repo = project
        .repo_url
        .as_ref()
        .and_then(|url| ci::parse_owner_repo(url));
    project.branch = git_info.as_ref().and_then(|g| g.branch.clone());
    perf_log::log_duration(
        "phase1_local_work",
        started.elapsed(),
        &format!(
            "path={} in_repo={} has_git_info={} branch={}",
            project.path,
            repo_presence.is_in_repo(),
            git_info.is_some(),
            project.branch.as_deref().unwrap_or("-")
        ),
        0,
    );
    project
}

fn cached_git_info(git_info_cache: &GitInfoCache, project_dir: &Path) -> Option<GitInfo> {
    let started = std::time::Instant::now();
    let repo_root = super::project::git_repo_root(project_dir)?;
    let Ok(mut cache) = git_info_cache.lock() else {
        let info = GitInfo::detect_fast(&repo_root);
        perf_log::log_duration(
            "phase1_cached_git_info",
            started.elapsed(),
            &format!("repo_root={} cache=poisoned hit=false", repo_root.display()),
            0,
        );
        return info;
    };
    if let Some(info) = cache.get(&repo_root) {
        perf_log::log_duration(
            "phase1_cached_git_info",
            started.elapsed(),
            &format!("repo_root={} cache=ok hit=true", repo_root.display()),
            0,
        );
        return info.clone();
    }
    let info = GitInfo::detect_fast(&repo_root);
    cache.insert(repo_root.clone(), info.clone());
    perf_log::log_duration(
        "phase1_cached_git_info",
        started.elapsed(),
        &format!("repo_root={} cache=ok hit=false", repo_root.display()),
        0,
    );
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Cargo;

    fn make_workspace(
        name: Option<&str>,
        abs_path: &str,
        worktree_name: Option<&str>,
        primary_abs: Option<&str>,
    ) -> ProjectListItem {
        ProjectListItem::Workspace(Project::<Workspace>::new(
            PathBuf::from(abs_path),
            name.map(String::from),
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
            Vec::new(),
            Vec::new(),
            worktree_name.map(String::from),
            primary_abs.map(PathBuf::from),
        ))
    }

    fn make_package(
        name: Option<&str>,
        abs_path: &str,
        worktree_name: Option<&str>,
        primary_abs: Option<&str>,
    ) -> ProjectListItem {
        ProjectListItem::Package(Project::<Package>::new(
            PathBuf::from(abs_path),
            name.map(String::from),
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
            Vec::new(),
            worktree_name.map(String::from),
            primary_abs.map(PathBuf::from),
        ))
    }

    #[test]
    fn merge_virtual_workspace() {
        let primary = make_workspace(None, "/home/ws", None, Some("/home/ws"));
        let worktree = make_workspace(None, "/home/ws_feat", Some("ws_feat"), Some("/home/ws"));
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1, "worktree should be merged into primary");
        let ProjectListItem::WorkspaceWorktrees(ref wtg) = items[0] else {
            std::process::abort()
        };
        assert_eq!(wtg.linked().len(), 1, "should have one linked worktree");
    }

    #[test]
    fn merge_named_workspace() {
        let primary = make_workspace(Some("my-ws"), "/home/ws", None, Some("/home/ws"));
        let worktree = make_workspace(
            Some("my-ws"),
            "/home/ws_feat",
            Some("ws_feat"),
            Some("/home/ws"),
        );
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1);
        let ProjectListItem::WorkspaceWorktrees(ref wtg) = items[0] else {
            std::process::abort()
        };
        assert_eq!(wtg.linked().len(), 1);
    }

    #[test]
    fn ci_cache_dir_scopes_runs_by_branch() {
        let main_dir = ci_cache_dir_pub("acme", "demo", Some("main"));
        let feature_dir = ci_cache_dir_pub("acme", "demo", Some("feat/demo"));

        assert_ne!(main_dir, feature_dir);
        assert!(feature_dir.ends_with("branches/feat_2fdemo"));
    }

    #[test]
    fn build_tree_only_nests_manifest_members() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let workspace_dir = tmp.path().join("hana");
        let included_dir = workspace_dir.join("crates").join("hana");
        let vendored_dir = workspace_dir.join("crates").join("clay-layout");

        std::fs::create_dir_all(&included_dir).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&vendored_dir).unwrap_or_else(|_| std::process::abort());
        std::fs::write(
            workspace_dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/hana\"]\n",
        )
        .unwrap_or_else(|_| std::process::abort());

        let workspace = make_workspace(Some("hana"), &workspace_dir.to_string_lossy(), None, None);
        let included = make_package(
            Some("hana-node-api"),
            &included_dir.to_string_lossy(),
            None,
            None,
        );
        let vendored = make_package(
            Some("clay-layout"),
            &vendored_dir.to_string_lossy(),
            None,
            None,
        );

        let items = build_tree(&[workspace, included, vendored], &["crates".to_string()]);

        let ws_item = items
            .iter()
            .find(|item| item.path() == workspace_dir.as_path())
            .unwrap_or_else(|| std::process::abort());
        let ProjectListItem::Workspace(ws) = ws_item else {
            std::process::abort()
        };
        assert_eq!(ws.groups().len(), 1);
        assert_eq!(ws.groups()[0].members().len(), 1);
        assert_eq!(ws.groups()[0].members()[0].path(), included_dir.as_path());
        assert!(
            ws.groups()
                .iter()
                .flat_map(|group| group.members().iter())
                .all(|member| member.path() != vendored_dir.as_path()),
            "non-member crate should not be grouped as a workspace member"
        );
        assert_eq!(ws.vendored().len(), 1);
        assert_eq!(ws.vendored()[0].path(), vendored_dir.as_path());
    }

    #[test]
    fn merge_standalone_project() {
        let primary = make_package(Some("app"), "/home/app", None, Some("/home/app"));
        let worktree = make_package(
            Some("app"),
            "/home/app_feat",
            Some("app_feat"),
            Some("/home/app"),
        );
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1);
        let ProjectListItem::PackageWorktrees(ref wtg) = items[0] else {
            std::process::abort()
        };
        assert_eq!(wtg.linked().len(), 1);
    }

    #[test]
    fn no_merge_different_repos() {
        let a = make_package(Some("a"), "/home/a", None, Some("/home/a"));
        let b = make_package(Some("b"), "/home/b", Some("b"), Some("/home/b"));
        let mut items = vec![a, b];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 2, "different repos should remain separate");
    }

    #[test]
    fn no_merge_none_identity() {
        let a = make_package(Some("x"), "/home/x", None, None);
        let b = make_package(Some("x"), "/home/x2", Some("x2"), None);
        let mut items = vec![a, b];
        merge_worktrees_new(&mut items);

        assert_eq!(
            items.len(),
            2,
            "nodes without identity should not be merged"
        );
    }

    #[test]
    fn group_disk_usage_trees_merges_nested_projects_under_one_root() {
        let trees = group_disk_usage_trees(&[
            (
                "~/rust/bevy".to_string(),
                PathBuf::from("/home/user/rust/bevy"),
            ),
            (
                "~/rust/bevy/crates/bevy_ecs".to_string(),
                PathBuf::from("/home/user/rust/bevy/crates/bevy_ecs"),
            ),
            (
                "~/rust/bevy/tools/ci".to_string(),
                PathBuf::from("/home/user/rust/bevy/tools/ci"),
            ),
            (
                "~/rust/hana".to_string(),
                PathBuf::from("/home/user/rust/hana"),
            ),
        ]);

        assert_eq!(trees.len(), 2);
        assert_eq!(trees[0].root_path, "~/rust/bevy");
        assert_eq!(trees[0].entries.len(), 3);
        assert_eq!(trees[1].root_path, "~/rust/hana");
        assert_eq!(trees[1].entries.len(), 1);
    }

    #[test]
    fn dir_sizes_for_tree_accumulates_root_and_child_sizes_from_one_walk() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let root = tmp.path().join("bevy");
        let child = root.join("crates").join("bevy_ecs");
        std::fs::create_dir_all(&child).unwrap_or_else(|_| std::process::abort());
        std::fs::write(root.join("root.txt"), vec![0_u8; 5])
            .unwrap_or_else(|_| std::process::abort());
        std::fs::write(child.join("child.txt"), vec![0_u8; 7])
            .unwrap_or_else(|_| std::process::abort());

        let sizes = dir_sizes_for_tree(&DiskUsageTree {
            root_path:     "~/rust/bevy".to_string(),
            root_abs_path: root.clone(),
            entries:       vec![
                ("~/rust/bevy".to_string(), root),
                ("~/rust/bevy/crates/bevy_ecs".to_string(), child),
            ],
        });
        let sizes: HashMap<String, u64> = sizes.into_iter().collect();

        assert_eq!(sizes.get("~/rust/bevy"), Some(&12));
        assert_eq!(sizes.get("~/rust/bevy/crates/bevy_ecs"), Some(&7));
    }
}
