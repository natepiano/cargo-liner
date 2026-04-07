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
use super::project::GitInfo;
use super::project::GitPathState;
use super::project::GitRepoPresence;
use super::project::LegacyProject;
use super::project::MemberGroup;
use super::project::NonRust;
use super::project::Package;
use super::project::Project;
use super::project::ProjectLanguage;
use super::project::ProjectListItem;
use super::project::Workspace;
use super::project::WorkspaceStatus;
use super::project::WorktreeGroup;

/// Members within a workspace are organized into groups by their first subdirectory.
/// The "inline" group (empty name) contains members directly under the workspace root
/// or under the primary `crates/` directory -- these are shown without a folder header.
#[derive(Clone)]
pub(crate) struct LegacyMemberGroup {
    pub name:    String,
    pub members: Vec<LegacyProject>,
}

#[derive(Clone)]
pub(crate) struct ProjectEntry {
    pub project:   LegacyProject,
    pub groups:    Vec<LegacyMemberGroup>,
    pub worktrees: Vec<Self>,
    pub vendored:  Vec<LegacyProject>,
}

impl ProjectEntry {
    pub(crate) fn has_members(&self) -> bool { self.groups.iter().any(|g| !g.members.is_empty()) }
}

/// A flattened entry for fuzzy search.
pub(crate) struct FlatEntry {
    pub path: String,
    pub name: String,
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
        project: LegacyProject,
    },
    ProjectRefreshed {
        project: LegacyProject,
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
            Self::ProjectDiscovered { project } | Self::ProjectRefreshed { project } => {
                Some(&project.path)
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

pub(crate) fn build_tree(projects: &[LegacyProject], inline_dirs: &[String]) -> Vec<ProjectEntry> {
    let workspace_paths: Vec<String> = projects
        .iter()
        .filter(|p| p.is_workspace())
        .map(|p| p.path.clone())
        .collect();

    let mut nodes: Vec<ProjectEntry> = Vec::new();
    let mut consumed: HashSet<usize> = HashSet::new();

    let top_level_workspaces: HashSet<usize> = projects
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.is_workspace()
                && !workspace_paths
                    .iter()
                    .any(|ws| *ws != p.path && p.path.starts_with(&format!("{ws}/")))
        })
        .map(|(i, _)| i)
        .collect();

    for (i, project) in projects.iter().enumerate() {
        if top_level_workspaces.contains(&i) {
            let member_paths = workspace_member_paths(project, projects);
            let mut all_members: Vec<LegacyProject> = projects
                .iter()
                .enumerate()
                .filter(|(j, p)| {
                    *j != i && !top_level_workspaces.contains(j) && member_paths.contains(&p.path)
                })
                .map(|(j, p)| {
                    consumed.insert(j);
                    p.clone()
                })
                .collect();

            all_members.sort_by(|a, b| {
                let name_a = a.name.as_deref().unwrap_or(&a.path);
                let name_b = b.name.as_deref().unwrap_or(&b.path);
                name_a.cmp(name_b)
            });

            let groups = group_members(&project.path, all_members, inline_dirs);

            consumed.insert(i);
            nodes.push(ProjectEntry {
                project: project.clone(),
                groups,
                worktrees: Vec::new(),
                vendored: Vec::new(),
            });
        }
    }

    for (i, project) in projects.iter().enumerate() {
        if consumed.contains(&i) {
            continue;
        }
        nodes.push(ProjectEntry {
            project:   project.clone(),
            groups:    Vec::new(),
            worktrees: Vec::new(),
            vendored:  Vec::new(),
        });
    }

    nodes.sort_by(|a, b| a.project.path.cmp(&b.project.path));

    // Detect vendored crates first, before worktree merging.
    // This catches crates like clay-layout that live inside worktree directories.
    extract_vendored(&mut nodes);

    // Merge worktree nodes into their primary project.
    // A worktree has `worktree_name = Some(...)`, the primary has `None`.
    merge_worktrees(&mut nodes);

    nodes
}

fn workspace_member_paths(
    workspace: &LegacyProject,
    projects: &[LegacyProject],
) -> HashSet<String> {
    let manifest = Path::new(&workspace.abs_path).join("Cargo.toml");
    let Some((members, excludes)) = workspace_member_patterns(&manifest) else {
        return projects
            .iter()
            .filter(|project| project.path.starts_with(&format!("{}/", workspace.path)))
            .map(|project| project.path.clone())
            .collect();
    };

    projects
        .iter()
        .filter(|project| project.path.starts_with(&format!("{}/", workspace.path)))
        .filter_map(|project| {
            workspace_relative_path(workspace, project).and_then(|relative| {
                let included = members
                    .iter()
                    .any(|pattern| workspace_pattern_matches(pattern, &relative));
                let is_excluded = excludes
                    .iter()
                    .any(|pattern| workspace_pattern_matches(pattern, &relative));
                if included && !is_excluded {
                    Some(project.path.clone())
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

fn workspace_relative_path(workspace: &LegacyProject, project: &LegacyProject) -> Option<String> {
    Path::new(&project.abs_path)
        .strip_prefix(&workspace.abs_path)
        .ok()
        .map(normalize_workspace_path)
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

/// Group worktree nodes under their primary (non-worktree) project.
/// Projects match when they share the same `worktree_primary_abs_path` (git repo identity).
/// The primary itself is also listed as a worktree entry (using its directory name).
fn merge_worktrees(nodes: &mut Vec<ProjectEntry>) {
    let mut primary_indices: HashMap<String, usize> = HashMap::new();
    let mut worktree_indices: Vec<usize> = Vec::new();

    for (i, node) in nodes.iter().enumerate() {
        let Some(identity) = &node.project.worktree_primary_abs_path else {
            continue;
        };
        if node.project.worktree_name.is_some() {
            worktree_indices.push(i);
        } else {
            primary_indices.insert(identity.clone(), i);
        }
    }

    // Identities that actually have worktrees
    let identities_with_worktrees: HashSet<String> = worktree_indices
        .iter()
        .filter_map(|&wi| nodes[wi].project.worktree_primary_abs_path.clone())
        .filter(|id| primary_indices.contains_key(id))
        .collect();

    // Collect worktree nodes to move (highest index first to preserve lower indices)
    let mut moves: Vec<(usize, String)> = worktree_indices
        .iter()
        .filter_map(|&wi| {
            let id = nodes[wi].project.worktree_primary_abs_path.clone()?;
            primary_indices.get(&id)?;
            Some((wi, id))
        })
        .collect();
    moves.sort_by(|a, b| b.0.cmp(&a.0));

    let mut extracted: Vec<(ProjectEntry, String)> = Vec::new();
    for (wi, id) in moves {
        let wt_node = nodes.remove(wi);
        extracted.push((wt_node, id));
    }

    // Insert worktree nodes into their primaries
    for (wt_node, id) in extracted {
        if let Some(primary) = nodes.iter_mut().find(|n| {
            n.project
                .worktree_primary_abs_path
                .as_ref()
                .is_some_and(|p| *p == id)
                && n.project.worktree_name.is_none()
        }) {
            primary.worktrees.push(wt_node);
        }
    }

    // Add the primary directory itself as the first worktree entry,
    // transferring the primary's groups so they appear under the worktree entry.
    for id in &identities_with_worktrees {
        if let Some(primary) = nodes.iter_mut().find(|n| {
            n.project
                .worktree_primary_abs_path
                .as_ref()
                .is_some_and(|p| p == id)
                && n.project.worktree_name.is_none()
        }) {
            let dir_name = primary
                .project
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&primary.project.path)
                .to_string();
            let mut primary_as_wt = primary.project.clone();
            primary_as_wt.worktree_name = Some(dir_name);
            let primary_groups = std::mem::take(&mut primary.groups);
            primary.worktrees.insert(
                0,
                ProjectEntry {
                    project:   primary_as_wt,
                    groups:    primary_groups,
                    worktrees: Vec::new(),
                    vendored:  Vec::new(),
                },
            );
        }
    }
}

/// Find standalone nodes whose path lives inside another node's directory
/// (or inside a worktree's directory) and move them into that node's `vendored` list.
fn extract_vendored(nodes: &mut Vec<ProjectEntry>) {
    // Collect abs_paths of all nodes and their worktrees
    let mut parent_paths: Vec<(usize, Option<usize>, String)> = Vec::new();
    for (ni, node) in nodes.iter().enumerate() {
        parent_paths.push((ni, None, node.project.abs_path.clone()));
        for (wi, wt) in node.worktrees.iter().enumerate() {
            parent_paths.push((ni, Some(wi), wt.project.abs_path.clone()));
        }
    }

    // Find which top-level nodes are vendored inside another
    let mut vendored_map: Vec<(usize, usize, Option<usize>)> = Vec::new(); // (vendored_node_idx, parent_node_idx, parent_wt_idx)

    for (vi, vnode) in nodes.iter().enumerate() {
        // Skip nodes that have workspace members or worktrees — they're real projects
        if vnode.has_members() || !vnode.worktrees.is_empty() {
            continue;
        }
        for &(ni, wt_idx, ref parent_abs) in &parent_paths {
            if ni == vi {
                continue;
            }
            if vnode
                .project
                .abs_path
                .starts_with(&format!("{parent_abs}/"))
            {
                vendored_map.push((vi, ni, wt_idx));
                break;
            }
        }
    }

    if vendored_map.is_empty() {
        return;
    }

    // Extract vendored projects (iterate in reverse to preserve indices)
    let mut vendored_projects: Vec<(usize, Option<usize>, LegacyProject)> = Vec::new();
    let mut remove_indices: Vec<usize> = vendored_map.iter().map(|&(vi, _, _)| vi).collect();
    remove_indices.sort_unstable();
    remove_indices.dedup();

    for &(vi, ni, wt_idx) in &vendored_map {
        vendored_projects.push((ni, wt_idx, nodes[vi].project.clone()));
    }

    // Remove vendored nodes from the top level (reverse order)
    for &idx in remove_indices.iter().rev() {
        nodes.remove(idx);
    }

    // Adjust parent indices after removal
    for (ni, wt_idx, project) in vendored_projects {
        let adjusted_ni = remove_indices.iter().filter(|&&r| r < ni).count();
        let target_ni = ni - adjusted_ni;
        if let Some(node) = nodes.get_mut(target_ni) {
            if let Some(wi) = wt_idx {
                if let Some(wt) = node.worktrees.get_mut(wi) {
                    wt.vendored.push(project);
                }
            } else {
                node.vendored.push(project);
            }
        }
    }

    // Sort vendored lists
    for node in nodes {
        node.vendored.sort_by(|a, b| a.path.cmp(&b.path));
        for wt in &mut node.worktrees {
            wt.vendored.sort_by(|a, b| a.path.cmp(&b.path));
        }
    }
}

pub(crate) fn group_members(
    workspace_path: &str,
    members: Vec<LegacyProject>,
    inline_dirs: &[String],
) -> Vec<LegacyMemberGroup> {
    let prefix = format!("{workspace_path}/");

    let mut group_map: HashMap<String, Vec<LegacyProject>> = HashMap::new();

    for member in members {
        let relative = member.path.strip_prefix(&prefix).unwrap_or(&member.path);
        let subdir = relative.split('/').next().unwrap_or("").to_string();

        // Members in configured inline dirs or directly in the workspace root are shown inline.
        // Everything else gets grouped by first subdirectory.
        let group_name = if inline_dirs.contains(&subdir) || !relative.contains('/') {
            String::new()
        } else {
            subdir
        };

        group_map.entry(group_name).or_default().push(member);
    }

    let mut groups: Vec<LegacyMemberGroup> = group_map
        .into_iter()
        .map(|(name, members)| LegacyMemberGroup { name, members })
        .collect();

    // Sort: named directories first (alphabetically), then inline group last
    groups.sort_by(|a, b| {
        let a_inline = a.name.is_empty();
        let b_inline = b.name.is_empty();
        match (a_inline, b_inline) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => a.name.cmp(&b.name),
        }
    });

    groups
}

pub(crate) fn build_flat_entries(nodes: &[ProjectEntry]) -> Vec<FlatEntry> {
    let mut entries = Vec::new();
    for node in nodes {
        entries.push(FlatEntry {
            path: node.project.path.clone(),
            name: node.project.display_name(),
        });

        for group in &node.groups {
            for member in &group.members {
                entries.push(FlatEntry {
                    path: member.path.clone(),
                    name: member.display_name(),
                });
            }
        }

        for vendored in &node.vendored {
            entries.push(FlatEntry {
                path: vendored.path.clone(),
                name: format!("{} (vendored)", vendored.display_name()),
            });
        }

        for worktree in &node.worktrees {
            if worktree.project.path != node.project.path {
                entries.push(FlatEntry {
                    path: worktree.project.path.clone(),
                    name: worktree
                        .project
                        .worktree_name
                        .clone()
                        .unwrap_or_else(|| worktree.project.display_name()),
                });
            }

            for group in &worktree.groups {
                for member in &group.members {
                    entries.push(FlatEntry {
                        path: member.path.clone(),
                        name: member.display_name(),
                    });
                }
            }

            for vendored in &worktree.vendored {
                entries.push(FlatEntry {
                    path: vendored.path.clone(),
                    name: format!("{} (vendored)", vendored.display_name()),
                });
            }
        }
    }
    entries
}

// ── New type system: build_project_list ────────────────────────────

/// Convert an old `LegacyProject` into a `Project<Package>`.
fn old_project_to_package(p: &LegacyProject) -> Project<Package> {
    Project::<Package>::new(
        PathBuf::from(&p.abs_path),
        p.name.clone(),
        Cargo::new(
            p.version.clone(),
            p.description.clone(),
            p.types.clone(),
            p.examples.clone(),
            p.benches.clone(),
            p.test_count,
            p.local_dependency_paths.clone(),
        ),
        Vec::new(),
        p.worktree_name.clone(),
        p.worktree_primary_abs_path.as_ref().map(PathBuf::from),
    )
}

/// Convert an old `LegacyProject` into a `Project<Workspace>`.
fn old_project_to_workspace(
    p: &LegacyProject,
    groups: Vec<MemberGroup>,
    vendored: Vec<Project<Package>>,
) -> Project<Workspace> {
    Project::<Workspace>::new(
        PathBuf::from(&p.abs_path),
        p.name.clone(),
        Cargo::new(
            p.version.clone(),
            p.description.clone(),
            p.types.clone(),
            p.examples.clone(),
            p.benches.clone(),
            p.test_count,
            p.local_dependency_paths.clone(),
        ),
        groups,
        vendored,
        p.worktree_name.clone(),
        p.worktree_primary_abs_path.as_ref().map(PathBuf::from),
    )
}

/// Convert an old `LegacyProject` into a `Project<NonRust>`.
fn old_project_to_nonrust(p: &LegacyProject) -> Project<NonRust> {
    Project::<NonRust>::new(
        PathBuf::from(&p.abs_path),
        p.name.clone(),
        p.worktree_name.clone(),
        p.worktree_primary_abs_path.as_ref().map(PathBuf::from),
    )
}

/// Convert old `LegacyMemberGroup`s to new `MemberGroup`s.
fn convert_member_groups(groups: &[LegacyMemberGroup]) -> Vec<MemberGroup> {
    groups
        .iter()
        .map(|g| {
            let members: Vec<Project<Package>> =
                g.members.iter().map(old_project_to_package).collect();
            if g.name.is_empty() {
                MemberGroup::Inline { members }
            } else {
                MemberGroup::Named {
                    name: g.name.clone(),
                    members,
                }
            }
        })
        .collect()
}

/// Build `Vec<ProjectListItem>` from existing `ProjectEntry` tree.
/// This bridges the old and new type systems during migration.
pub(crate) fn build_project_list(nodes: &[ProjectEntry]) -> Vec<ProjectListItem> {
    let mut items = Vec::new();
    for node in nodes {
        let is_rust = matches!(node.project.is_rust, ProjectLanguage::Rust);
        let is_workspace = matches!(node.project.is_workspace, WorkspaceStatus::Workspace)
            || !node.groups.is_empty()
            || node.worktrees.iter().any(|wt| !wt.groups.is_empty());
        let has_worktrees = !node.worktrees.is_empty();

        if !is_rust {
            // NonRust doesn't have worktrees in the new model, skip worktree grouping.
            // Just emit the primary as NonRust.
            items.push(ProjectListItem::NonRust(old_project_to_nonrust(
                &node.project,
            )));
        } else if has_worktrees {
            // Build worktree group. The first worktree entry is the primary.
            let primary_wt = &node.worktrees[0];
            if is_workspace {
                let primary = old_project_to_workspace(
                    &primary_wt.project,
                    convert_member_groups(&primary_wt.groups),
                    primary_wt
                        .vendored
                        .iter()
                        .map(old_project_to_package)
                        .collect(),
                );
                let linked: Vec<Project<Workspace>> = node.worktrees[1..]
                    .iter()
                    .map(|wt| {
                        old_project_to_workspace(
                            &wt.project,
                            convert_member_groups(&wt.groups),
                            wt.vendored.iter().map(old_project_to_package).collect(),
                        )
                    })
                    .collect();
                items.push(ProjectListItem::WorkspaceWorktrees(WorktreeGroup::new(
                    primary, linked,
                )));
            } else {
                let primary = old_project_to_package(&primary_wt.project);
                let linked: Vec<Project<Package>> = node.worktrees[1..]
                    .iter()
                    .map(|wt| old_project_to_package(&wt.project))
                    .collect();
                items.push(ProjectListItem::PackageWorktrees(WorktreeGroup::new(
                    primary, linked,
                )));
            }
        } else if is_workspace {
            let groups = convert_member_groups(&node.groups);
            let vendored: Vec<Project<Package>> =
                node.vendored.iter().map(old_project_to_package).collect();
            items.push(ProjectListItem::Workspace(old_project_to_workspace(
                &node.project,
                groups,
                vendored,
            )));
        } else {
            let mut pkg = old_project_to_package(&node.project);
            if !node.vendored.is_empty() {
                *pkg.vendored_mut() = node.vendored.iter().map(old_project_to_package).collect();
            }
            items.push(ProjectListItem::Package(pkg));
        }
    }
    items
}

/// Shared network context passed to `fetch_project_details`.
pub(crate) struct FetchContext {
    pub client:     HttpClient,
    pub repo_cache: RepoCache,
}

pub(crate) struct ProjectDetailRequest<'a> {
    pub tx:            &'a mpsc::Sender<BackgroundMsg>,
    pub ctx:           &'a FetchContext,
    pub project_path:  &'a str,
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
    let project_path = req.project_path;
    let abs_path = req.abs_path;
    let project_name = req.project_name;
    let repo_presence = req.repo_presence;
    let ci_run_count = req.ci_run_count;
    let client = &ctx.client;
    let repo_cache = &ctx.repo_cache;
    let _ = tx.send(BackgroundMsg::GitPathState {
        path:  project_path.to_string(),
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
            path: project_path.to_string(),
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
            path: project_path.to_string(),
            runs: data.runs,
        });
        if let Some(meta) = data.meta {
            let _ = tx.send(BackgroundMsg::RepoMeta {
                path:        project_path.to_string(),
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
                path:      project_path.to_string(),
                version:   info.version,
                downloads: info.downloads,
            });
        }
    }

    // Disk usage last — walking large `target/` dirs is the slowest
    // local operation and doesn't block anything else.
    let bytes = dir_size(abs_path);
    let _ = tx.send(BackgroundMsg::DiskUsage {
        path: project_path.to_string(),
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
    let project = LegacyProject::from_git_dir(entry_path);
    let path = project.path.clone();
    let abs_path = PathBuf::from(&project.abs_path);
    stats.projects += 1;
    stats.non_rust_projects += 1;

    let _ = scan_context
        .tx
        .send(BackgroundMsg::ProjectDiscovered { project });

    let discovered = DiscoveredProject {
        path,
        abs_path: abs_path.clone(),
        name: None,
        repo_url: None,
        owner_repo: None,
        branch: None,
    };
    let disk_path = discovered.path.clone();
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
                let Ok(project) = LegacyProject::from_cargo_toml(entry.path()) else {
                    continue;
                };
                perf_log::log_duration(
                    "phase1_manifest_parse",
                    manifest_started.elapsed(),
                    &format!("manifest={}", entry.path().display()),
                    0,
                );
                stats.projects += 1;
                let abs_path = PathBuf::from(&project.abs_path);
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
                        project.path,
                        repo_presence.is_in_repo()
                    ),
                    0,
                );

                let _ = scan_context.tx.send(BackgroundMsg::ProjectDiscovered {
                    project: project.clone(),
                });

                let discovered = DiscoveredProject {
                    path: project.path.clone(),
                    abs_path,
                    name: project.name.clone(),
                    repo_url: None,
                    owner_repo: None,
                    branch: None,
                };
                spawn_project_local_work(scan_context, discovered.clone(), repo_presence);
                disk_entries.push((discovered.path.clone(), discovered.abs_path.clone()));
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
        match register_repo_path(&scan_context.repo_dispatch, &key, &project.path) {
            RepoDispatchRegistration::Cached(data) => {
                send_repo_data(&scan_context.tx, std::slice::from_ref(&project.path), &data);
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
            &project.path,
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
            path: project.path.clone(),
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
    use crate::project::ProjectLanguage;
    use crate::project::WorkspaceStatus;

    fn make_project(
        name: Option<&str>,
        path: &str,
        abs_path: &str,
        worktree_name: Option<&str>,
        primary_abs: Option<&str>,
        is_workspace: WorkspaceStatus,
    ) -> LegacyProject {
        LegacyProject {
            path: path.to_string(),
            abs_path: abs_path.to_string(),
            name: name.map(String::from),
            version: None,
            description: None,
            worktree_name: worktree_name.map(String::from),
            worktree_primary_abs_path: primary_abs.map(String::from),
            is_workspace,
            types: Vec::new(),
            examples: Vec::new(),
            benches: Vec::new(),
            test_count: 0,
            is_rust: ProjectLanguage::Rust,
            local_dependency_paths: Vec::new(),
        }
    }

    fn make_node(project: LegacyProject) -> ProjectEntry {
        ProjectEntry {
            project,
            groups: Vec::new(),
            worktrees: Vec::new(),
            vendored: Vec::new(),
        }
    }

    fn make_node_with_groups(
        project: LegacyProject,
        groups: Vec<LegacyMemberGroup>,
    ) -> ProjectEntry {
        ProjectEntry {
            project,
            groups,
            worktrees: Vec::new(),
            vendored: Vec::new(),
        }
    }

    #[test]
    fn merge_virtual_workspace() {
        let primary = make_project(
            None,
            "~/rust/ws",
            "/home/ws",
            None,
            Some("/home/ws"),
            WorkspaceStatus::Workspace,
        );
        let worktree = make_project(
            None,
            "~/rust/ws_feat",
            "/home/ws_feat",
            Some("ws_feat"),
            Some("/home/ws"),
            WorkspaceStatus::Workspace,
        );
        let mut nodes = vec![make_node(primary), make_node(worktree)];
        merge_worktrees(&mut nodes);

        assert_eq!(nodes.len(), 1, "worktree should be merged into primary");
        assert_eq!(nodes[0].worktrees.len(), 2, "primary-as-wt + worktree");
        assert_eq!(
            nodes[0].worktrees[0].project.worktree_name.as_deref(),
            Some("ws"),
            "first entry is primary-as-worktree"
        );
        assert_eq!(
            nodes[0].worktrees[1].project.worktree_name.as_deref(),
            Some("ws_feat"),
        );
    }

    #[test]
    fn merge_named_workspace() {
        let primary = make_project(
            Some("my-ws"),
            "~/rust/ws",
            "/home/ws",
            None,
            Some("/home/ws"),
            WorkspaceStatus::Workspace,
        );
        let worktree = make_project(
            Some("my-ws"),
            "~/rust/ws_feat",
            "/home/ws_feat",
            Some("ws_feat"),
            Some("/home/ws"),
            WorkspaceStatus::Workspace,
        );
        let mut nodes = vec![make_node(primary), make_node(worktree)];
        merge_worktrees(&mut nodes);

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].worktrees.len(), 2);
    }

    #[test]
    fn ci_cache_dir_scopes_runs_by_branch() {
        let main_dir = ci_cache_dir_pub("acme", "demo", Some("main"));
        let feature_dir = ci_cache_dir_pub("acme", "demo", Some("feat/demo"));

        assert_ne!(main_dir, feature_dir);
        assert!(feature_dir.ends_with("branches/feat_2fdemo"));
    }

    #[test]
    fn merge_groups_transfer() {
        let primary = make_project(
            None,
            "~/rust/ws",
            "/home/ws",
            None,
            Some("/home/ws"),
            WorkspaceStatus::Workspace,
        );
        let member_a = make_project(
            Some("crate-a"),
            "~/rust/ws/crates/a",
            "/home/ws/crates/a",
            None,
            Some("/home/ws"),
            WorkspaceStatus::Standalone,
        );
        let member_b = make_project(
            Some("crate-b"),
            "~/rust/ws/crates/b",
            "/home/ws/crates/b",
            None,
            Some("/home/ws"),
            WorkspaceStatus::Standalone,
        );
        let groups = vec![LegacyMemberGroup {
            name:    String::new(),
            members: vec![member_a, member_b],
        }];

        let worktree = make_project(
            None,
            "~/rust/ws_feat",
            "/home/ws_feat",
            Some("ws_feat"),
            Some("/home/ws"),
            WorkspaceStatus::Workspace,
        );

        let mut nodes = vec![make_node_with_groups(primary, groups), make_node(worktree)];
        merge_worktrees(&mut nodes);

        assert!(
            nodes[0].groups.is_empty(),
            "primary's groups should be moved to worktrees[0]"
        );
        assert_eq!(
            nodes[0].worktrees[0].groups.len(),
            1,
            "primary-as-wt should have the groups"
        );
        assert_eq!(nodes[0].worktrees[0].groups[0].members.len(), 2);
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

        let workspace = make_project(
            Some("hana"),
            "~/rust/hana",
            &workspace_dir.to_string_lossy(),
            None,
            Some(&workspace_dir.to_string_lossy()),
            WorkspaceStatus::Workspace,
        );
        let included = make_project(
            Some("hana-node-api"),
            "~/rust/hana/crates/hana",
            &included_dir.to_string_lossy(),
            None,
            Some(&workspace_dir.to_string_lossy()),
            WorkspaceStatus::Standalone,
        );
        let vendored = make_project(
            Some("clay-layout"),
            "~/rust/hana/crates/clay-layout",
            &vendored_dir.to_string_lossy(),
            None,
            Some(&workspace_dir.to_string_lossy()),
            WorkspaceStatus::Standalone,
        );

        let nodes = build_tree(
            &[workspace.clone(), included.clone(), vendored.clone()],
            &["crates".to_string()],
        );

        let workspace_node = nodes
            .iter()
            .find(|node| node.project.path == workspace.path)
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(workspace_node.groups.len(), 1);
        assert_eq!(workspace_node.groups[0].members.len(), 1);
        assert_eq!(workspace_node.groups[0].members[0].path, included.path);
        assert!(
            workspace_node
                .groups
                .iter()
                .flat_map(|group| group.members.iter())
                .all(|member| member.path != vendored.path),
            "non-member crate should not be grouped as a workspace member"
        );
        assert_eq!(workspace_node.vendored.len(), 1);
        assert_eq!(workspace_node.vendored[0].path, vendored.path);
    }

    #[test]
    fn merge_standalone_project() {
        let primary = make_project(
            Some("app"),
            "~/rust/app",
            "/home/app",
            None,
            Some("/home/app"),
            WorkspaceStatus::Standalone,
        );
        let worktree = make_project(
            Some("app"),
            "~/rust/app_feat",
            "/home/app_feat",
            Some("app_feat"),
            Some("/home/app"),
            WorkspaceStatus::Standalone,
        );
        let mut nodes = vec![make_node(primary), make_node(worktree)];
        merge_worktrees(&mut nodes);

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].worktrees.len(), 2);
        assert!(
            !nodes[0].worktrees[0].has_members(),
            "standalone worktrees have no groups"
        );
    }

    #[test]
    fn no_merge_different_repos() {
        let a = make_project(
            Some("a"),
            "~/rust/a",
            "/home/a",
            None,
            Some("/home/a"),
            WorkspaceStatus::Standalone,
        );
        let b = make_project(
            Some("b"),
            "~/rust/b",
            "/home/b",
            Some("b"),
            Some("/home/b"),
            WorkspaceStatus::Standalone,
        );
        let mut nodes = vec![make_node(a), make_node(b)];
        merge_worktrees(&mut nodes);

        assert_eq!(nodes.len(), 2, "different repos should remain separate");
    }

    #[test]
    fn no_merge_none_identity() {
        let a = make_project(
            Some("x"),
            "~/rust/x",
            "/home/x",
            None,
            None,
            WorkspaceStatus::Standalone,
        );
        let b = make_project(
            Some("x"),
            "~/rust/x2",
            "/home/x2",
            Some("x2"),
            None,
            WorkspaceStatus::Standalone,
        );
        let mut nodes = vec![make_node(a), make_node(b)];
        merge_worktrees(&mut nodes);

        assert_eq!(
            nodes.len(),
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
