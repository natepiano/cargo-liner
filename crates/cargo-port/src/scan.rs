use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;

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
use super::http::HttpClient;
use super::http::ServiceKind;
use super::http::ServiceSignal;
use super::port_report;
use super::port_report::LintStatus;
use super::project::GitInfo;
use super::project::GitTracking;
use super::project::RustProject;

/// Members within a workspace are organized into groups by their first subdirectory.
/// The "inline" group (empty name) contains members directly under the workspace root
/// or under the primary `crates/` directory -- these are shown without a folder header.
#[derive(Clone)]
pub struct MemberGroup {
    pub name: String,
    pub members: Vec<RustProject>,
}

#[derive(Clone)]
pub struct ProjectNode {
    pub project: RustProject,
    pub groups: Vec<MemberGroup>,
    pub worktrees: Vec<Self>,
    pub vendored: Vec<RustProject>,
}

impl ProjectNode {
    pub fn has_members(&self) -> bool {
        self.groups.iter().any(|g| !g.members.is_empty())
    }

    pub fn has_children(&self) -> bool {
        self.has_members() || !self.worktrees.is_empty()
    }
}

/// A flattened entry for fuzzy search.
pub struct FlatEntry {
    pub node_index: usize,
    pub group_index: usize,
    pub member_index: usize,
    pub name: String,
}

pub enum BackgroundMsg {
    DiskUsage {
        path: String,
        bytes: u64,
    },
    CiRuns {
        path: String,
        runs: Vec<CiRun>,
    },
    GitInfo {
        path: String,
        info: GitInfo,
    },
    CratesIoVersion {
        path: String,
        version: String,
        downloads: u64,
    },
    RepoMeta {
        path: String,
        stars: u64,
        description: Option<String>,
    },
    ProjectDiscovered {
        project: RustProject,
    },
    ScanActivity {
        path: String,
    },
    LintStatus {
        path: String,
        status: LintStatus,
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
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::DiskUsage { path, .. }
            | Self::CiRuns { path, .. }
            | Self::GitInfo { path, .. }
            | Self::CratesIoVersion { path, .. }
            | Self::RepoMeta { path, .. }
            | Self::LintStatus { path, .. } => Some(path),
            Self::ProjectDiscovered { project } => Some(&project.path),
            Self::ScanActivity { .. }
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

pub fn emit_service_signal(tx: &mpsc::Sender<BackgroundMsg>, signal: Option<ServiceSignal>) {
    let msg = match signal {
        Some(ServiceSignal::Reachable(service)) => BackgroundMsg::ServiceReachable { service },
        Some(ServiceSignal::Unreachable(service)) => BackgroundMsg::ServiceUnreachable { service },
        None => return,
    };
    let _ = tx.send(msg);
}

pub fn emit_service_recovered(tx: &mpsc::Sender<BackgroundMsg>, service: ServiceKind) {
    let _ = tx.send(BackgroundMsg::ServiceRecovered { service });
}

/// What a CI fetch function returns. Forces callers to handle the
/// "network failed but cache exists" case explicitly -- the compiler won't
/// let you silently discard cached runs.
pub enum CiFetchResult {
    /// Fresh runs (network succeeded), merged with cache.
    Loaded(Vec<CiRun>),
    /// Network failed; returning whatever the disk cache had.
    CacheOnly(Vec<CiRun>),
}

/// Base cache directory for CI metadata.
pub fn cache_dir() -> PathBuf {
    cache_paths::ci_cache_root()
}

/// Repo-keyed cache directory: `{cache_dir}/{owner}/{repo}`.
fn repo_cache_dir(owner: &str, repo: &str) -> PathBuf {
    cache_dir().join(owner).join(repo)
}

/// Public accessor for clearing the cache directory.
pub fn repo_cache_dir_pub(owner: &str, repo: &str) -> PathBuf {
    repo_cache_dir(owner, repo)
}

/// Check if the "no more runs" marker exists for a repo.
pub fn is_exhausted(owner: &str, repo: &str) -> bool {
    repo_cache_dir(owner, repo)
        .join(NO_MORE_RUNS_MARKER)
        .exists()
}

/// Save the "no more runs" marker for a repo.
pub fn mark_exhausted(owner: &str, repo: &str) {
    let dir = repo_cache_dir(owner, repo);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(NO_MORE_RUNS_MARKER), "");
}

/// Remove the "no more runs" marker so fresh runs can be discovered.
pub fn clear_exhausted(owner: &str, repo: &str) {
    let dir = repo_cache_dir(owner, repo);
    let _ = std::fs::remove_file(dir.join(NO_MORE_RUNS_MARKER));
}

fn save_cached_run(owner: &str, repo: &str, ci_run: &CiRun) {
    let dir = repo_cache_dir(owner, repo);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", ci_run.run_id));
    if let Ok(json) = serde_json::to_string(ci_run) {
        let _ = std::fs::write(&path, json);
    }
}

fn load_cached_run(owner: &str, repo: &str, run_id: u64) -> Option<CiRun> {
    let dir = repo_cache_dir(owner, repo);
    let path = dir.join(format!("{run_id}.json"));
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Count the number of cached CI run files on disk for a given repo.
pub fn count_cached_runs(owner: &str, repo: &str) -> usize {
    let dir = repo_cache_dir(owner, repo);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .count()
}

/// Load all cached CI runs for a given repo.
pub fn load_all_cached_runs(owner: &str, repo: &str) -> Vec<CiRun> {
    let dir = repo_cache_dir(owner, repo);
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

    // Partition into cached hits and misses.
    let mut uncached: Vec<&GhRun> = Vec::new();
    for gh_run in gh_runs {
        if let Some(cached) = load_cached_run(owner, repo, gh_run.id) {
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
            save_cached_run(owner, repo, &ci_run);
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
    gh_runs: &[GhRun],
) -> (Vec<CiRun>, Option<RepoMetaInfo>, Option<ServiceSignal>) {
    let mut result: Vec<CiRun> = Vec::with_capacity(gh_runs.len());

    let mut uncached: Vec<&GhRun> = Vec::new();
    for gh_run in gh_runs {
        if let Some(cached) = load_cached_run(owner, repo, gh_run.id) {
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
            save_cached_run(owner, repo, &ci_run);
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
    count: u32,
) -> (CiFetchResult, Option<RepoMetaInfo>, Option<ServiceSignal>) {
    let (gh_runs, list_signal) = client.list_runs_async(owner, repo, None, count).await;
    let gh_runs = gh_runs.unwrap_or_default();
    let (fetched, meta, detail_signal) =
        fetch_recent_runs_async(client, repo_url, owner, repo, &gh_runs).await;
    let cached = load_all_cached_runs(owner, repo);
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
pub fn fetch_ci_runs_cached(
    client: &HttpClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    count: u32,
) -> (CiFetchResult, Option<RepoMetaInfo>, Option<ServiceSignal>) {
    let (gh_runs, list_signal) = client.list_runs(owner, repo, None, count);
    let gh_runs = gh_runs.unwrap_or_default();
    let (fetched, meta, detail_signal) = fetch_recent_runs(client, repo_url, owner, repo, &gh_runs);
    let cached = load_all_cached_runs(owner, repo);
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
pub fn fetch_older_runs(
    client: &HttpClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    current_count: u32,
) -> (CiFetchResult, Option<ServiceSignal>) {
    let fetch_count = current_count + OLDER_RUNS_FETCH_INCREMENT;
    let (gh_runs, list_signal) = client.list_runs(owner, repo, None, fetch_count);
    let gh_runs = gh_runs.unwrap_or_default();
    let (fetched, _meta, detail_signal) =
        fetch_recent_runs(client, repo_url, owner, repo, &gh_runs);

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
pub fn fetch_newer_runs(
    client: &HttpClient,
    repo_url: &str,
    owner: &str,
    repo: &str,
    current_count: u32,
) -> (CiFetchResult, Option<ServiceSignal>) {
    let (gh_runs, list_signal) = client.list_runs(owner, repo, None, current_count);
    let gh_runs = gh_runs.unwrap_or_default();
    let (mut result, _meta, detail_signal) =
        fetch_recent_runs(client, repo_url, owner, repo, &gh_runs);
    result.sort_by(|a, b| b.run_id.cmp(&a.run_id));

    let result = if gh_runs.is_empty() {
        CiFetchResult::CacheOnly(result)
    } else {
        CiFetchResult::Loaded(result)
    };
    (result, combine_service_signal(list_signal, detail_signal))
}

pub struct CratesIoInfo {
    pub version: String,
    pub downloads: u64,
}

pub fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

pub fn build_tree(projects: &[RustProject], inline_dirs: &[String]) -> Vec<ProjectNode> {
    let workspace_paths: Vec<String> = projects
        .iter()
        .filter(|p| p.is_workspace())
        .map(|p| p.path.clone())
        .collect();

    let mut nodes: Vec<ProjectNode> = Vec::new();
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
            let mut all_members: Vec<RustProject> = projects
                .iter()
                .enumerate()
                .filter(|(j, p)| {
                    *j != i
                        && !top_level_workspaces.contains(j)
                        && p.path.starts_with(&format!("{}/", project.path))
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
            nodes.push(ProjectNode {
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
        let under_workspace = workspace_paths
            .iter()
            .any(|ws| project.path.starts_with(&format!("{ws}/")));
        if !under_workspace {
            nodes.push(ProjectNode {
                project: project.clone(),
                groups: Vec::new(),
                worktrees: Vec::new(),
                vendored: Vec::new(),
            });
        }
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

/// Group worktree nodes under their primary (non-worktree) project.
/// Projects match when they share the same `worktree_primary_abs_path` (git repo identity).
/// The primary itself is also listed as a worktree entry (using its directory name).
fn merge_worktrees(nodes: &mut Vec<ProjectNode>) {
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

    let mut extracted: Vec<(ProjectNode, String)> = Vec::new();
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
                ProjectNode {
                    project: primary_as_wt,
                    groups: primary_groups,
                    worktrees: Vec::new(),
                    vendored: Vec::new(),
                },
            );
        }
    }
}

/// Find standalone nodes whose path lives inside another node's directory
/// (or inside a worktree's directory) and move them into that node's `vendored` list.
fn extract_vendored(nodes: &mut Vec<ProjectNode>) {
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
    let mut vendored_projects: Vec<(usize, Option<usize>, RustProject)> = Vec::new();
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

pub fn group_members(
    workspace_path: &str,
    members: Vec<RustProject>,
    inline_dirs: &[String],
) -> Vec<MemberGroup> {
    let prefix = format!("{workspace_path}/");

    let mut group_map: HashMap<String, Vec<RustProject>> = HashMap::new();

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

    let mut groups: Vec<MemberGroup> = group_map
        .into_iter()
        .map(|(name, members)| MemberGroup { name, members })
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

pub fn build_flat_entries(nodes: &[ProjectNode]) -> Vec<FlatEntry> {
    let mut entries = Vec::new();
    for (ni, node) in nodes.iter().enumerate() {
        // Add workspace root itself
        let name = node.project.name.as_deref().unwrap_or(&node.project.path);
        entries.push(FlatEntry {
            node_index: ni,
            group_index: 0,
            member_index: 0,
            name: name.to_string(),
        });
        // Add all members
        for (gi, group) in node.groups.iter().enumerate() {
            for (mi, member) in group.members.iter().enumerate() {
                let name = member.name.as_deref().unwrap_or(&member.path);
                entries.push(FlatEntry {
                    node_index: ni,
                    group_index: gi,
                    member_index: mi,
                    name: name.to_string(),
                });
            }
        }
    }
    entries
}

/// Shared network context passed to `fetch_project_details`.
pub struct FetchContext {
    pub client: HttpClient,
    pub repo_cache: RepoCache,
}

/// Fetch all details (disk, git, crates.io, CI) for a single project and send
/// results through the provided channel. Used by both the main scan and priority fetch.
#[allow(
    clippy::too_many_arguments,
    reason = "priority fetch shares the same fully-expanded project detail path as discovery"
)]
pub fn fetch_project_details(
    tx: &mpsc::Sender<BackgroundMsg>,
    ctx: &FetchContext,
    project_path: &str,
    abs_path: &Path,
    project_name: Option<&String>,
    git_tracking: GitTracking,
    ci_run_count: u32,
    lint_enabled: bool,
) {
    let client = &ctx.client;
    let repo_cache = &ctx.repo_cache;
    // Git info first (local, instant) — also provides the repo URL for CI cache lookup
    let git_info = if git_tracking.is_tracked() {
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
        let cache_key = format!("{owner}/{repo}");
        let cached = repo_cache
            .lock()
            .ok()
            .and_then(|c| c.get(&cache_key).cloned());

        let data = cached.unwrap_or_else(|| {
            let _ = tx.send(BackgroundMsg::ScanActivity {
                path: format!("CI: {project_path}"),
            });
            let (result, meta, signal) =
                fetch_ci_runs_cached(client, repo_url, &owner, &repo, ci_run_count);
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
                path: project_path.to_string(),
                stars: meta.stars,
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
                path: project_path.to_string(),
                version: info.version,
                downloads: info.downloads,
            });
        }
    }

    if lint_enabled {
        // Lint status (cheap local file read).
        let lint = port_report::read_status(abs_path);
        if !matches!(lint, LintStatus::NoLog) {
            let _ = tx.send(BackgroundMsg::LintStatus {
                path: project_path.to_string(),
                status: lint,
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
pub struct RepoMetaInfo {
    pub stars: u64,
    pub description: Option<String>,
}

/// Cached CI + metadata results keyed by `"owner/repo"`. Shared across
/// rayon threads so worktrees of the same repo don't make duplicate
/// HTTP calls.
#[derive(Clone)]
pub struct CachedRepoData {
    runs: Vec<CiRun>,
    meta: Option<RepoMetaInfo>,
}

pub type RepoCache = Arc<Mutex<HashMap<String, CachedRepoData>>>;

pub fn new_repo_cache() -> RepoCache {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Resolve include-dir entries to absolute paths. Relative entries are
/// joined to `scan_root`; absolute entries are used as-is. An empty
/// list falls back to `[scan_root]` so the whole tree is walked.
pub fn resolve_include_dirs(scan_root: &Path, include_dirs: &[String]) -> Vec<PathBuf> {
    if include_dirs.is_empty() {
        return vec![scan_root.to_path_buf()];
    }
    include_dirs
        .iter()
        .map(|dir| {
            let path = Path::new(dir);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                scan_root.join(path)
            }
        })
        .collect()
}

/// Information collected in phase 1 (local work) for a single project,
/// used to drive async HTTP dispatch and disk usage.
struct DiscoveredProject {
    path: String,
    abs_path: PathBuf,
    name: Option<String>,
    repo_url: Option<String>,
    owner_repo: Option<(String, String)>,
    lint_enabled: bool,
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

#[derive(Clone)]
struct StreamingScanContext {
    client: HttpClient,
    tx: mpsc::Sender<BackgroundMsg>,
    ci_run_count: u32,
    lint_enabled: bool,
    disk_limit: Arc<tokio::sync::Semaphore>,
    http_limit: Arc<tokio::sync::Semaphore>,
    repo_dispatch: RepoDispatchMap,
}

struct RepoFetchRequest {
    key: String,
    project_path: String,
    repo_url: String,
    owner: String,
    repo: String,
}

/// Spawn a streaming scan using a hybrid approach:
///
/// - **Discovery + local work (rayon):** Walk the directory tree, discover projects, collect git
///   info and lint status, and dispatch background work.
/// - **Disk usage (tokio blocking pool):** `dir_size()` runs behind its own semaphore so disk walks
///   cannot monopolize startup.
/// - **HTTP (tokio):** CI runs, repo metadata, crates.io info, and connectivity checks run on the
///   async runtime behind a shared semaphore.
///
/// `ScanComplete` is sent after discovery/local work has finished. Disk and HTTP results may
/// continue to stream in afterward.
pub fn spawn_streaming_scan(
    scan_root: &Path,
    ci_run_count: u32,
    include_dirs: &[String],
    non_rust: NonRustInclusion,
    lint_enabled: bool,
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
            lint_enabled,
            disk_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_DISK_CONCURRENCY)),
            http_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_HTTP_CONCURRENCY)),
            repo_dispatch: Arc::new(Mutex::new(HashMap::new())),
        };

        phase1_discover(&root, &scan_dirs, non_rust, &scan_context);
        let _ = scan_tx.send(BackgroundMsg::ScanComplete);
    });

    (tx, rx)
}

/// Walk `scan_dirs`, discover projects, and stream per-project work immediately. Discovery and
/// local metadata collection stay on the dedicated scan thread, while disk and network work are
/// dispatched onto bounded background queues.
fn phase1_discover(
    root: &Path,
    scan_dirs: &[PathBuf],
    non_rust: NonRustInclusion,
    scan_context: &StreamingScanContext,
) {
    for dir in scan_dirs {
        if !dir.is_dir() {
            continue;
        }
        let mut iter = WalkDir::new(dir).into_iter();
        while let Some(Ok(entry)) = iter.next() {
            if entry.file_type().is_dir() {
                let name = entry.file_name();
                if name == "target" || name == ".git" {
                    iter.skip_current_dir();
                    continue;
                }

                let rel = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap_or_else(|_| entry.path())
                    .display()
                    .to_string();
                let _ = scan_context.tx.send(BackgroundMsg::ScanActivity {
                    path: if rel.is_empty() { ".".to_string() } else { rel },
                });

                if non_rust.includes_non_rust()
                    && entry.path().join(".git").is_dir()
                    && !entry.path().join("Cargo.toml").exists()
                {
                    iter.skip_current_dir();

                    let project = RustProject::from_git_dir(entry.path());
                    let abs_path = PathBuf::from(&project.abs_path);

                    let _ = scan_context.tx.send(BackgroundMsg::ProjectDiscovered {
                        project: project.clone(),
                    });

                    let discovered = phase1_local_work(
                        &scan_context.tx,
                        DiscoveredProject {
                            path: project.path.clone(),
                            abs_path,
                            name: None,
                            repo_url: None,
                            owner_repo: None,
                            lint_enabled: scan_context.lint_enabled,
                        },
                        GitTracking::Tracked,
                    );
                    spawn_project_http(scan_context, &discovered);
                    spawn_disk_usage(scan_context, &discovered.abs_path, &discovered.path);
                    continue;
                }
            }
            if entry.file_type().is_file()
                && entry.file_name() == "Cargo.toml"
                && let Ok(project) = RustProject::from_cargo_toml(entry.path())
            {
                let abs_path = PathBuf::from(&project.abs_path);
                let git_tracking = if abs_path.join(".git").exists() {
                    GitTracking::Tracked
                } else {
                    GitTracking::Untracked
                };

                let _ = scan_context.tx.send(BackgroundMsg::ProjectDiscovered {
                    project: project.clone(),
                });

                let discovered = phase1_local_work(
                    &scan_context.tx,
                    DiscoveredProject {
                        path: project.path.clone(),
                        abs_path,
                        name: project.name.clone(),
                        repo_url: None,
                        owner_repo: None,
                        lint_enabled: scan_context.lint_enabled,
                    },
                    git_tracking,
                );
                spawn_project_http(scan_context, &discovered);
                spawn_disk_usage(scan_context, &discovered.abs_path, &discovered.path);
            }
        }
    }
}

fn spawn_project_http(scan_context: &StreamingScanContext, project: &DiscoveredProject) {
    if let Some((owner, repo)) = &project.owner_repo {
        let key = format!("{owner}/{repo}");
        match register_repo_path(&scan_context.repo_dispatch, &key, &project.path) {
            RepoDispatchRegistration::Cached(data) => {
                send_repo_data(&scan_context.tx, std::slice::from_ref(&project.path), &data);
            },
            RepoDispatchRegistration::SpawnFetch => {
                spawn_repo_fetch(
                    scan_context,
                    RepoFetchRequest {
                        key,
                        project_path: project.path.clone(),
                        repo_url: project.repo_url.clone().unwrap_or_default(),
                        owner: owner.clone(),
                        repo: repo.clone(),
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

fn spawn_disk_usage(scan_context: &StreamingScanContext, abs_path: &Path, project_path: &str) {
    let handle = scan_context.client.handle.clone();
    let tx = scan_context.tx.clone();
    let disk_limit = Arc::clone(&scan_context.disk_limit);
    let abs_path = abs_path.to_path_buf();
    let project_path = project_path.to_string();

    handle.spawn(async move {
        let Ok(_permit) = disk_limit.acquire_owned().await else {
            return;
        };
        let Ok(bytes) = tokio::task::spawn_blocking(move || dir_size(&abs_path)).await else {
            return;
        };
        let _ = tx.send(BackgroundMsg::DiskUsage {
            path: project_path,
            bytes,
        });
    });
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
                path: path.clone(),
                stars: meta.stars,
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
        let Ok(_permit) = http_limit.acquire_owned().await else {
            return;
        };
        let _ = tx.send(BackgroundMsg::ScanActivity {
            path: format!("CI: {}", request.project_path),
        });
        let (result, meta, signal) = fetch_ci_runs_cached_async(
            &client,
            &request.repo_url,
            &request.owner,
            &request.repo,
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
        let paths = finish_repo_fetch(&repo_dispatch, &request.key, data.clone());
        send_repo_data(&tx, &paths, &data);
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
        let Ok(_permit) = http_limit.acquire_owned().await else {
            return;
        };
        let (info, signal) = client.fetch_crates_io_info_async(&crate_name).await;
        emit_service_signal(&tx, signal);
        if let Some(info) = info {
            let _ = tx.send(BackgroundMsg::CratesIoVersion {
                path: project_path,
                version: info.version,
                downloads: info.downloads,
            });
        }
    });
}

/// Phase 1 local work: git info + lint status for a single project.
/// Returns the discovered repo metadata needed for async HTTP dispatch.
fn phase1_local_work(
    tx: &mpsc::Sender<BackgroundMsg>,
    mut project: DiscoveredProject,
    git_tracking: GitTracking,
) -> DiscoveredProject {
    let git_info = if git_tracking.is_tracked() {
        GitInfo::detect(&project.abs_path)
    } else {
        None
    };
    if let Some(ref info) = git_info {
        let _ = tx.send(BackgroundMsg::GitInfo {
            path: project.path.clone(),
            info: info.clone(),
        });
    }

    if project.lint_enabled {
        // Lint status (cheap local file read).
        let lint = port_report::read_status(&project.abs_path);
        if !matches!(lint, LintStatus::NoLog) {
            let _ = tx.send(BackgroundMsg::LintStatus {
                path: project.path.clone(),
                status: lint,
            });
        }
    }

    project.repo_url = git_info.as_ref().and_then(|g| g.url.clone());
    project.owner_repo = project
        .repo_url
        .as_ref()
        .and_then(|url| ci::parse_owner_repo(url));
    project
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
    ) -> RustProject {
        RustProject {
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
        }
    }

    fn make_node(project: RustProject) -> ProjectNode {
        ProjectNode {
            project,
            groups: Vec::new(),
            worktrees: Vec::new(),
            vendored: Vec::new(),
        }
    }

    fn make_node_with_groups(project: RustProject, groups: Vec<MemberGroup>) -> ProjectNode {
        ProjectNode {
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
        let groups = vec![MemberGroup {
            name: String::new(),
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
}
