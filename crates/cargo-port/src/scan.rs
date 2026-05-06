use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::thread;

use cargo_metadata::Error;
use cargo_metadata::Metadata;
use tokei::Config;
use tokei::Languages;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use toml::Table;
use toml::Value;
use walkdir::WalkDir;

use super::cache_paths;
use super::ci;
use super::ci::CiRun;
use super::ci::GhRun;
use super::ci::OwnerRepo;
use super::config::NonRustInclusion;
use super::constants::CARGO_METADATA_TIMEOUT;
use super::constants::NO_MORE_RUNS_MARKER;
use super::constants::SCAN_DISK_CONCURRENCY;
use super::constants::SCAN_METADATA_CONCURRENCY;
use super::http::HttpClient;
use super::http::ServiceKind;
use super::http::ServiceSignal;
use super::lint::LintStatus;
use super::project;
use super::project::AbsolutePath;
use super::project::CargoParseResult;
use super::project::CheckoutInfo;
use super::project::GitRepoPresence;
use super::project::LangEntry;
use super::project::LanguageStats;
use super::project::ManifestFingerprint;
use super::project::MemberGroup;
use super::project::Package;
use super::project::PackageRecord;
use super::project::ProjectFields;
use super::project::PublishPolicy;
use super::project::RepoInfo;
use super::project::RootItem;
use super::project::RustInfo;
use super::project::RustProject;
use super::project::Submodule;
use super::project::TargetRecord;
use super::project::VendoredPackage;
use super::project::Workspace;
use super::project::WorkspaceMetadata;
use super::project::WorkspaceMetadataStore;
use super::project::WorktreeGroup;
use crate::enrichment;

/// Messages sent from background threads to the main event loop.
pub(crate) enum BackgroundMsg {
    /// Disk usage (bytes) computed for a single project path.
    DiskUsage { path: AbsolutePath, bytes: u64 },
    /// Batch of disk usage results for projects under a common root.
    /// Step 5: each entry carries both the total and the in-target /
    /// non-target split used by the detail-pane breakdown.
    DiskUsageBatch {
        root_path: AbsolutePath,
        entries:   Vec<(AbsolutePath, DirSizes)>,
    },
    /// GitHub Actions CI runs fetched for a project.
    CiRuns {
        path:         AbsolutePath,
        runs:         Vec<CiRun>,
        github_total: u32,
    },
    /// A GitHub repo fetch has been queued (for startup tracking).
    RepoFetchQueued { repo: OwnerRepo },
    /// A `spawn_repo_fetch_for_git_info` thread has finished. Sent
    /// regardless of whether the spawn hit the network or returned a
    /// cached result. Drives both the startup "GitHub repos" toast
    /// progress (no-op on cache hit, since no `RepoFetchQueued` was
    /// sent) and the `repo_fetch_in_flight` dedup set.
    RepoFetchComplete { repo: OwnerRepo },
    /// Per-checkout git state for a project (branch, status, ahead/
    /// behind, `last_commit`, `primary_tracked_ref`). Sent by
    /// `CheckoutInfo::get` for every affected checkout — primary AND
    /// each linked worktree on a refresh — since each working tree has
    /// its own HEAD/index/branch.
    CheckoutInfo {
        path: AbsolutePath,
        info: CheckoutInfo,
    },
    /// Per-repo git state (remotes, workflows, default branch, last
    /// fetched, etc.). Sent by `RepoInfo::get` once per repo refresh.
    /// `path` is the primary checkout's path so `handle_repo_info` can
    /// enforce the "only the primary writes `RepoInfo`" policy.
    RepoInfo { path: AbsolutePath, info: RepoInfo },
    /// First commit date detected for a project (deferred post-scan,
    /// batched by repo root to avoid redundant `git log` calls).
    GitFirstCommit {
        path:         AbsolutePath,
        first_commit: Option<String>,
    },
    /// Crates.io version and download count fetched for a project.
    CratesIoVersion {
        path:      AbsolutePath,
        version:   String,
        downloads: u64,
    },
    /// GitHub repo metadata (stars, description) fetched.
    RepoMeta {
        path:        AbsolutePath,
        stars:       u64,
        description: Option<String>,
    },
    /// Complete project tree from the streaming scan, plus disk entry
    /// paths for background disk usage computation.
    ScanResult {
        projects:     Vec<RootItem>,
        disk_entries: Vec<(String, AbsolutePath)>,
    },
    /// A new project discovered by the watcher after the initial scan.
    ProjectDiscovered { item: RootItem },
    /// An existing project re-scanned by the watcher (e.g. after a
    /// Cargo.toml change adds/removes workspace members).
    ProjectRefreshed { item: RootItem },
    /// Git submodules detected for a project.
    Submodules {
        path:       AbsolutePath,
        submodules: Vec<Submodule>,
    },
    /// Live lint status update from the lint runtime (a lint run started,
    /// passed, failed, etc.). Sent during normal operation when files
    /// change and the lint runtime re-checks a project.
    LintStatus {
        path:   AbsolutePath,
        status: LintStatus,
    },
    /// Startup lint cache check result. Sent once per registered project
    /// when the lint runtime reads cached lint results from disk during
    /// initialization. Distinct from `LintStatus` so the app can track
    /// when all startup cache checks are complete.
    LintStartupStatus {
        path:   AbsolutePath,
        status: LintStatus,
    },
    /// Lint cache pruned — old runs evicted to stay within the configured
    /// cache size limit.
    LintCachePruned {
        runs_evicted:    usize,
        bytes_reclaimed: u64,
    },
    /// An external service (GitHub, crates.io) is reachable.
    ServiceReachable { service: ServiceKind },
    /// An external service recovered after being unreachable or
    /// rate-limited.
    ServiceRecovered { service: ServiceKind },
    /// Network failure reaching the service (DNS, connection, timeout,
    /// 5xx).
    ServiceUnreachable { service: ServiceKind },
    /// Service is reachable but currently rate-limited.
    ServiceRateLimited { service: ServiceKind },
    /// Language statistics (file counts + LOC by language) computed by tokei.
    LanguageStatsBatch {
        entries: Vec<(AbsolutePath, LanguageStats)>,
    },
    /// `cargo metadata --no-deps --offline` result for one workspace root.
    /// The `fingerprint` was captured *before* the spawn; callers recompute
    /// at merge time and discard the result on mismatch. `generation`
    /// coalesces rapid re-dispatches — arrivals stamped with an older
    /// generation are dropped rather than merged.
    CargoMetadata {
        workspace_root: AbsolutePath,
        generation:     u64,
        fingerprint:    ManifestFingerprint,
        result:         Result<WorkspaceMetadata, CargoMetadataError>,
    },
    /// Disk walk result for an out-of-tree `target_directory`. Emitted by
    /// [`spawn_out_of_tree_target_walk`] when workspace metadata whose
    /// `target_directory` sits outside its `workspace_root` lands. The
    /// receiver stamps `bytes` onto the cached metadata so the detail pane
    /// can surface sharer target sizes that the per-project walker can't see.
    OutOfTreeTargetSize {
        workspace_root: AbsolutePath,
        target_dir:     AbsolutePath,
        bytes:          u64,
    },
}

/// Structured failure for a `cargo metadata` invocation. Held inside
/// [`BackgroundMsg::CargoMetadata`] so the main loop can raise a keyed
/// error toast and leave the affected rows in fallback state.
///
/// Variant chosen deliberately so the handler dispatches on cause rather
/// than string-matching: `WorkspaceMissing` is the expected race when the
/// user just deleted a worktree (no toast — the workspace is gone), and
/// `Other` is a real failure surface that needs to be visible.
#[derive(Clone, Debug)]
pub(crate) enum CargoMetadataError {
    /// Workspace root no longer exists on disk between dispatch and run.
    /// Common when a worktree is deleted while a refresh is in flight.
    /// Logged at debug; never shown to the user.
    WorkspaceMissing,
    /// All other failures: cargo subprocess errors, timeouts, parse
    /// failures. Shown verbatim in a timed error toast.
    Other(String),
}

impl CargoMetadataError {
    /// Message body for the error toast — only meaningful for `Other`.
    pub(crate) const fn user_facing_message(&self) -> Option<&str> {
        match self {
            Self::WorkspaceMissing => None,
            Self::Other(message) => Some(message.as_str()),
        }
    }
}

impl BackgroundMsg {
    /// If this message can change what the detail pane would render for a
    /// project at some path, return that path. Otherwise return `None`.
    ///
    /// This is exhaustive on every variant *by design* — adding a new
    /// `BackgroundMsg` without classifying it here is a compile error.
    /// That's the type-level guarantee: invalidation policy can't drift
    /// out of sync with the message catalog.
    ///
    /// "Affects detail" means the message could change a field in
    /// `PaneDataStore`'s built detail set (`package`, `git`, `targets`,
    /// `ci`, `lints`). Service-level signals, fetch lifecycle, and batch
    /// notifications that are processed via dedicated paths return
    /// `None` — they invalidate via their own routes (or don't need to).
    pub(crate) fn detail_relevance(&self) -> Option<&Path> {
        match self {
            // Per-project path bearing — each maps to a field rendered
            // inside the detail set.
            Self::DiskUsage { path, .. }              // package.disk
            | Self::CiRuns { path, .. }                // ci.runs
            | Self::CheckoutInfo { path, .. }          // git.branch / git.status
            | Self::RepoInfo { path, .. }              // git.remotes / git.workflows
            | Self::GitFirstCommit { path, .. }        // git.inception
            | Self::CratesIoVersion { path, .. }      // package.crates_version
            | Self::RepoMeta { path, .. }              // git.stars / git.description
            | Self::Submodules { path, .. }            // submodules detail
            | Self::LintStatus { path, .. }            // lints
            | Self::LintStartupStatus { path, .. } => Some(path.as_path()),

            // Discovery/refresh of an item is detail-relevant for that
            // item's path (ahead/behind cache, package fields, etc.).
            Self::ProjectDiscovered { item }
            | Self::ProjectRefreshed { item } => Some(item.path()),

            // Workspace-wide metadata feeds package + targets fields for
            // every member of the workspace, but the path we have is the
            // workspace root — `detail_path_is_affected` will widen the
            // match correctly.
            Self::CargoMetadata { workspace_root, .. }
            | Self::OutOfTreeTargetSize { workspace_root, .. } => Some(workspace_root.as_path()),

            // Wholesale tree replacement bumps `data_generation` via the
            // dedicated `apply_tree_build` / scan-result paths. No
            // per-message bump needed.
            Self::ScanResult { .. }
            // Batch arrivals are aggregated and the handler bumps
            // generation explicitly (see `handle_disk_usage_batch_msg`).
            | Self::DiskUsageBatch { .. }
            // Language stats live in `RustInfo`, not in the detail set.
            | Self::LanguageStatsBatch { .. }
            // Fetch lifecycle is reflected via toasts, not detail data.
            | Self::RepoFetchQueued { .. }
            | Self::RepoFetchComplete { .. }
            // Cache pruning is internal to the lint subsystem.
            | Self::LintCachePruned { .. }
            // Service availability is a separate UI surface.
            | Self::ServiceReachable { .. }
            | Self::ServiceRecovered { .. }
            | Self::ServiceUnreachable { .. }
            | Self::ServiceRateLimited { .. } => None,
        }
    }
}

const fn combine_service_signal(
    left: Option<ServiceSignal>,
    right: Option<ServiceSignal>,
) -> Option<ServiceSignal> {
    // Priority: Unreachable > RateLimited > Reachable — any bad signal
    // wins over a good one, and network failure trumps rate-limit when
    // both show up in the same batch.
    match (left, right) {
        (Some(ServiceSignal::Unreachable(service)), _)
        | (_, Some(ServiceSignal::Unreachable(service))) => {
            Some(ServiceSignal::Unreachable(service))
        },
        (Some(ServiceSignal::RateLimited(service)), _)
        | (_, Some(ServiceSignal::RateLimited(service))) => {
            Some(ServiceSignal::RateLimited(service))
        },
        (Some(ServiceSignal::Reachable(service)), _)
        | (_, Some(ServiceSignal::Reachable(service))) => Some(ServiceSignal::Reachable(service)),
        (None, None) => None,
    }
}

pub(crate) fn emit_service_signal(tx: &Sender<BackgroundMsg>, signal: Option<ServiceSignal>) {
    let msg = match signal {
        Some(ServiceSignal::Reachable(service)) => BackgroundMsg::ServiceReachable { service },
        Some(ServiceSignal::Unreachable(service)) => BackgroundMsg::ServiceUnreachable { service },
        Some(ServiceSignal::RateLimited(service)) => BackgroundMsg::ServiceRateLimited { service },
        None => return,
    };
    let _ = tx.send(msg);
}

pub(crate) fn emit_service_recovered(tx: &Sender<BackgroundMsg>, service: ServiceKind) {
    let _ = tx.send(BackgroundMsg::ServiceRecovered { service });
}

/// Probe per-repo + per-checkout git state for a single project and
/// emit them as two background messages. Used by the initial scan and
/// project-discovery enrichment paths, where each project is processed
/// independently. The watcher's refresh path uses a smarter
/// orchestration that probes `RepoInfo` once per repo and reuses it
/// across sibling worktrees.
pub(crate) fn emit_git_info(tx: &Sender<BackgroundMsg>, path: &AbsolutePath) {
    let Some(repo) = RepoInfo::get(path.as_path()) else {
        return;
    };
    let checkout = CheckoutInfo::get(path.as_path(), repo.local_main_branch.as_deref());
    let _ = tx.send(BackgroundMsg::RepoInfo {
        path: path.clone(),
        info: repo,
    });
    if let Some(checkout) = checkout {
        let _ = tx.send(BackgroundMsg::CheckoutInfo {
            path: path.clone(),
            info: checkout,
        });
    }
}

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

/// Base cache directory for CI metadata.
pub(crate) fn cache_dir() -> AbsolutePath { cache_paths::ci_cache_root() }

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

pub(crate) fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Build a project tree from a flat list of discovered `RootItem`s.
///
/// The input must contain only `Rust(Workspace)`, `Rust(Package)`, and `NonRust` variants
/// (discovery does not produce worktree groups). This function:
/// 1. Nests workspace members into their parent workspace's `groups`
/// 2. Detects vendored crates nested inside other projects
/// 3. Merges worktree checkouts into `WorktreeGroup` variants
pub(crate) fn build_tree(items: &[RootItem], inline_dirs: &[String]) -> Vec<RootItem> {
    let workspace_paths: Vec<&AbsolutePath> = items
        .iter()
        .filter(|item| matches!(item, RootItem::Rust(RustProject::Workspace(_))))
        .map(RootItem::path)
        .collect();

    let mut result: Vec<RootItem> = Vec::new();
    let mut consumed: HashSet<usize> = HashSet::new();

    // Identify top-level workspaces (not nested inside another workspace).
    let top_level_workspaces: HashSet<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(item, RootItem::Rust(RustProject::Workspace(_)))
                && !workspace_paths
                    .iter()
                    .any(|ws| *ws != item.path() && item.path().starts_with(ws.as_path()))
        })
        .map(|(i, _)| i)
        .collect();

    for (i, item) in items.iter().enumerate() {
        if !top_level_workspaces.contains(&i) {
            continue;
        }
        let RootItem::Rust(RustProject::Workspace(ws)) = item else {
            continue;
        };
        let ws_path = ws.path().to_path_buf();
        let member_paths = workspace_member_paths_new(&ws_path, items);

        let mut all_members: Vec<Package> = items
            .iter()
            .enumerate()
            .filter(|(j, candidate)| {
                *j != i
                    && !top_level_workspaces.contains(j)
                    && member_paths.contains(candidate.path())
            })
            .filter_map(|(j, candidate)| {
                consumed.insert(j);
                if let RootItem::Rust(RustProject::Package(pkg)) = candidate {
                    Some(pkg.clone())
                } else if let RootItem::Rust(RustProject::Workspace(nested_ws)) = candidate {
                    // Nested workspace treated as a package member
                    Some(Package {
                        path:            nested_ws.path().clone(),
                        name:            nested_ws.name().map(str::to_string),
                        worktree_status: nested_ws.worktree_status().clone(),
                        rust:            RustInfo {
                            cargo: nested_ws.cargo.clone(),
                            ..RustInfo::default()
                        },
                    })
                } else {
                    None
                }
            })
            .collect();

        all_members.sort_by(|a, b| a.package_name().as_str().cmp(b.package_name().as_str()));

        let groups = group_members_new(&ws_path, all_members, inline_dirs);

        let mut new_ws = ws.clone();
        *new_ws.groups_mut() = groups;
        consumed.insert(i);
        result.push(RootItem::Rust(RustProject::Workspace(new_ws)));
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

fn workspace_member_paths_new(ws_path: &Path, items: &[RootItem]) -> HashSet<AbsolutePath> {
    let manifest = ws_path.join("Cargo.toml");
    let Some((members, excludes)) = workspace_member_patterns(&manifest) else {
        return items
            .iter()
            .filter(|item| item.path().starts_with(ws_path) && item.path() != ws_path)
            .map(|item| item.path().clone())
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
                    Some(item.path().clone())
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

pub(crate) fn normalize_workspace_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(segment) => Some(segment.to_string_lossy().to_string()),
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
/// Projects sharing the same `worktree_primary_abs_path` are grouped
/// into `Worktrees(WorktreeGroup::Workspaces { .. })` or
/// `Worktrees(WorktreeGroup::Packages { .. })`.
/// `NonRust` projects are not grouped into worktree variants.
fn item_worktree_identity(item: &RootItem) -> Option<&AbsolutePath> {
    match item {
        RootItem::Rust(p) => p.worktree_status().primary_root(),
        _ => None,
    }
}

fn item_is_linked(item: &RootItem) -> bool {
    match item {
        RootItem::Rust(p) => p.worktree_status().is_linked_worktree(),
        _ => false,
    }
}

fn merge_worktrees_new(items: &mut Vec<RootItem>) {
    let mut primary_indices: HashMap<AbsolutePath, usize> = HashMap::new();
    let mut worktree_indices: Vec<usize> = Vec::new();

    for (i, item) in items.iter().enumerate() {
        let Some(identity) = item_worktree_identity(item) else {
            continue;
        };
        let is_linked = item_is_linked(item);
        if is_linked {
            worktree_indices.push(i);
        } else {
            primary_indices.insert(identity.clone(), i);
        }
    }

    let identities_with_worktrees: HashSet<AbsolutePath> = worktree_indices
        .iter()
        .filter_map(|&wi| {
            item_worktree_identity(&items[wi])
                .filter(|id| primary_indices.contains_key(id.as_path()))
                .cloned()
        })
        .collect();

    if identities_with_worktrees.is_empty() {
        return;
    }

    // Extract worktree items (highest index first to preserve lower indices)
    let mut moves: Vec<(usize, AbsolutePath)> = worktree_indices
        .iter()
        .filter_map(|&wi| {
            let id = item_worktree_identity(&items[wi])?.clone();
            primary_indices.get(id.as_path())?;
            Some((wi, id))
        })
        .collect();
    moves.sort_by_key(|entry| Reverse(entry.0));

    let mut extracted: Vec<(RootItem, AbsolutePath)> = Vec::new();
    for (wi, id) in moves {
        let item = items.remove(wi);
        extracted.push((item, id));
    }

    // Rebuild primary_indices after removals
    let mut primary_map: HashMap<AbsolutePath, usize> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        if let Some(id) = item_worktree_identity(item)
            .filter(|id| identities_with_worktrees.contains(*id))
            .filter(|_| !item_is_linked(item))
        {
            primary_map.insert(id.clone(), i);
        }
    }

    // Group linked worktrees by identity, preserving order
    let mut linked_by_id: HashMap<AbsolutePath, Vec<RootItem>> = HashMap::new();
    for (item, id) in extracted {
        linked_by_id.entry(id).or_default().push(item);
    }

    // Replace each primary with its WorktreeGroup variant
    // Process in reverse to avoid index shifting
    let mut replacements: Vec<(usize, RootItem)> = Vec::new();
    for (id, idx) in &primary_map {
        let linked = linked_by_id.remove(id).unwrap_or_default();
        let primary_item = &items[*idx];
        let replacement = match primary_item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                let linked_ws: Vec<Workspace> = linked
                    .into_iter()
                    .filter_map(|item| {
                        if let RootItem::Rust(RustProject::Workspace(linked_ws)) = item {
                            Some(linked_ws)
                        } else {
                            None
                        }
                    })
                    .collect();
                RootItem::Worktrees(WorktreeGroup::new_workspaces(ws.clone(), linked_ws))
            },
            RootItem::Rust(RustProject::Package(pkg)) => {
                let linked_pkg: Vec<Package> = linked
                    .into_iter()
                    .filter_map(|item| {
                        if let RootItem::Rust(RustProject::Package(linked_pkg)) = item {
                            Some(linked_pkg)
                        } else {
                            None
                        }
                    })
                    .collect();
                RootItem::Worktrees(WorktreeGroup::new_packages(pkg.clone(), linked_pkg))
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
fn extract_vendored_new(items: &mut Vec<RootItem>) {
    let parent_paths: Vec<(usize, AbsolutePath)> = items
        .iter()
        .enumerate()
        .map(|(i, item)| (i, item.path().clone()))
        .collect();

    let mut vendored_map: Vec<(usize, usize)> = Vec::new();

    for (vi, vitem) in items.iter().enumerate() {
        let has_structure = match vitem {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.groups().iter().any(|g| !g.members().is_empty())
            },
            RootItem::Worktrees(_) => true,
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

    // Convert vendored items to `VendoredPackage`
    let mut vendored_projects: Vec<(usize, VendoredPackage)> = Vec::new();
    for &(vi, ni) in &vendored_map {
        let vendored = match &items[vi] {
            RootItem::Rust(RustProject::Package(p)) => VendoredPackage {
                path:             p.path.clone(),
                name:             p.name.clone(),
                worktree_status:  p.worktree_status.clone(),
                info:             p.rust.info.clone(),
                cargo:            p.rust.cargo.clone(),
                crates_version:   p.rust.crates_version.clone(),
                crates_downloads: p.rust.crates_downloads,
            },
            RootItem::Rust(RustProject::Workspace(ws)) => VendoredPackage {
                path: ws.path().clone(),
                name: ws.name().map(String::from),
                worktree_status: ws.worktree_status().clone(),
                cargo: ws.cargo.clone(),
                ..VendoredPackage::default()
            },
            RootItem::NonRust(nr) => VendoredPackage {
                path: nr.path().clone(),
                name: nr.name().map(String::from),
                ..VendoredPackage::default()
            },
            _ => continue,
        };
        vendored_projects.push((ni, vendored));
    }

    for &idx in remove_indices.iter().rev() {
        items.remove(idx);
    }

    for (ni, vendored) in vendored_projects {
        let adjusted_ni = remove_indices.iter().filter(|&&r| r < ni).count();
        let target_ni = ni - adjusted_ni;
        if let Some(item) = items.get_mut(target_ni) {
            match item {
                RootItem::Rust(RustProject::Workspace(ws)) => ws.vendored_mut().push(vendored),
                RootItem::Rust(RustProject::Package(p)) => p.vendored_mut().push(vendored),
                _ => {},
            }
        }
    }

    // Sort vendored lists
    for item in items {
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.vendored_mut().sort_by(|a, b| a.path().cmp(b.path()));
            },
            RootItem::Rust(RustProject::Package(pkg)) => {
                pkg.vendored_mut().sort_by(|a, b| a.path().cmp(b.path()));
            },
            _ => {},
        }
    }
}

fn group_members_new(
    workspace_path: &Path,
    members: Vec<Package>,
    inline_dirs: &[String],
) -> Vec<MemberGroup> {
    let mut group_map: HashMap<String, Vec<Package>> = HashMap::new();

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
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => a.group_name().cmp(b.group_name()),
        }
    });

    groups
}

/// Convert a `CargoProject` (from `from_cargo_toml()`) into a `RootItem`.
pub(crate) fn cargo_project_to_item(cp: CargoParseResult) -> RootItem {
    match cp {
        CargoParseResult::Workspace(ws) => RootItem::Rust(RustProject::Workspace(ws)),
        CargoParseResult::Package(pkg) => RootItem::Rust(RustProject::Package(pkg)),
    }
}

/// Build a normalized project item for a discovered root directory.
///
/// Unlike `cargo_project_to_item(from_cargo_toml(...))`, this walks nested
/// manifests under the root and runs them through `build_tree()`, so a newly
/// discovered workspace arrives with its member groups already populated.
pub(crate) fn discover_project_item(root_dir: &Path) -> Option<RootItem> {
    let mut items = Vec::new();
    let mut iter = WalkDir::new(root_dir).into_iter();
    while let Some(Ok(entry)) = iter.next() {
        if entry.file_type().is_dir() {
            let name = entry.file_name();
            if name == "target" || name == ".git" {
                iter.skip_current_dir();
                continue;
            }
        }
        if entry.file_type().is_file() && entry.file_name() == "Cargo.toml" {
            let parsed = project::from_cargo_toml(entry.path()).ok()?;
            items.push(cargo_project_to_item(parsed));
        }
    }

    if items.is_empty() {
        return None;
    }

    build_tree(&items, &[])
        .into_iter()
        .find(|item| item.path() == root_dir)
}

/// Shared network context passed to `fetch_project_details`.
pub(crate) struct FetchContext {
    pub client: HttpClient,
}

pub(crate) struct ProjectDetailRequest<'a> {
    pub tx:            &'a Sender<BackgroundMsg>,
    pub fetch_context: &'a FetchContext,
    pub _project_path: &'a str,
    pub abs_path:      &'a Path,
    pub project_name:  Option<&'a str>,
    pub repo_presence: GitRepoPresence,
}

/// Fetch local project details for a single project and send results through
/// the provided channel. Used by both the main scan and project discovery paths.
pub(crate) fn fetch_project_details(req: &ProjectDetailRequest<'_>) {
    let tx = req.tx;
    let fetch_context = req.fetch_context;
    let abs_path = req.abs_path;
    let abs: AbsolutePath = abs_path.to_path_buf().into();
    let project_name = req.project_name;
    let repo_presence = req.repo_presence;
    let client = &fetch_context.client;
    // Local git info — includes git status but skips first_commit,
    // which is handled separately by
    // `schedule_git_first_commit_refreshes` (batched by repo root).
    if repo_presence.is_in_repo() {
        emit_git_info(tx, &abs);
    }

    // Crates.io version + downloads (network)
    if let Some(name) = project_name {
        let (info, signal) = client.fetch_crates_io_info(name);
        emit_service_signal(tx, signal);
        if let Some(info) = info {
            let _ = tx.send(BackgroundMsg::CratesIoVersion {
                path:      abs.clone(),
                version:   info.version,
                downloads: info.downloads,
            });
        }
    }

    // Submodules (local, fast — reads .gitmodules + one git ls-tree).
    // Send the Submodules message first so `at_path_mut` can resolve each
    // submodule before its per-entry enrichment messages arrive.
    if repo_presence.is_in_repo() {
        let submodules = project::get_submodules(abs_path);
        if !submodules.is_empty() {
            let _ = tx.send(BackgroundMsg::Submodules {
                path:       abs.clone(),
                submodules: submodules.clone(),
            });
            for sub in &submodules {
                enrichment::enrich(sub, tx, fetch_context);
            }
        }
    }

    // Disk usage last — walking large `target/` dirs is the slowest
    // local operation and doesn't block anything else.
    let bytes = dir_size(abs_path);
    let _ = tx.send(BackgroundMsg::DiskUsage { path: abs, bytes });
}

#[derive(Clone)]
pub(crate) struct RepoMetaInfo {
    pub stars:       u64,
    pub description: Option<String>,
}

/// Cached CI + metadata results keyed by `"owner/repo"`. Shared across
/// background tasks so worktrees on the same repo don't make duplicate
/// HTTP calls.
#[derive(Clone)]
pub(crate) struct CachedRepoData {
    pub(crate) runs:         Vec<CiRun>,
    pub(crate) meta:         Option<RepoMetaInfo>,
    pub(crate) github_total: u32,
}

pub(crate) type RepoCache = Arc<Mutex<HashMap<OwnerRepo, CachedRepoData>>>;

pub(crate) fn new_repo_cache() -> RepoCache { Arc::new(Mutex::new(HashMap::new())) }

pub(crate) fn load_cached_repo_data(
    repo_cache: &RepoCache,
    owner_repo: &OwnerRepo,
) -> Option<CachedRepoData> {
    repo_cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(owner_repo).cloned())
}

pub(crate) fn store_cached_repo_data(
    repo_cache: &RepoCache,
    owner_repo: &OwnerRepo,
    data: CachedRepoData,
) {
    if let Ok(mut cache) = repo_cache.lock() {
        cache.insert(owner_repo.clone(), data);
    }
}

pub(crate) fn invalidate_cached_repo_data(repo_cache: &RepoCache, owner_repo: &OwnerRepo) {
    if let Ok(mut cache) = repo_cache.lock() {
        cache.remove(owner_repo);
    }
}

/// Resolve include-dir entries to absolute paths. `~` and `~/…` entries
/// expand via the user's home directory; already-absolute entries are
/// used as-is; relative entries are joined to the home directory.
/// An empty list returns an empty vec (no fallback).
pub(crate) fn resolve_include_dirs(include_dirs: &[String]) -> Vec<AbsolutePath> {
    include_dirs
        .iter()
        .map(|dir| {
            let expanded = expand_home_path(dir);
            let resolved = if expanded.is_absolute() {
                expanded
            } else {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(&expanded)
            };
            AbsolutePath::from(resolved.canonicalize().unwrap_or(resolved))
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

#[derive(Clone)]
struct StreamingScanContext {
    client:         HttpClient,
    tx:             Sender<BackgroundMsg>,
    disk_limit:     Arc<Semaphore>,
    metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    metadata_limit: Arc<Semaphore>,
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
/// `ScanResult` is sent after discovery/local work has finished, containing the complete tree
/// and flat entries. Disk and HTTP results may continue to stream in afterward.
pub(crate) fn spawn_streaming_scan(
    scan_dirs: Vec<AbsolutePath>,
    inline_dirs: &[String],
    non_rust: NonRustInclusion,
    client: HttpClient,
    metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
) -> (Sender<BackgroundMsg>, Receiver<BackgroundMsg>) {
    let (tx, rx) = mpsc::channel();
    let inline_dirs = inline_dirs.to_vec();

    let scan_tx = tx.clone();
    thread::spawn(move || {
        let scan_context = StreamingScanContext {
            client,
            tx: scan_tx.clone(),
            disk_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_DISK_CONCURRENCY)),
            metadata_store,
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_METADATA_CONCURRENCY)),
        };

        let phase1_started = std::time::Instant::now();
        let phase1 = phase1_discover(&scan_dirs, non_rust);
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(phase1_started.elapsed().as_millis()),
            scan_dirs = scan_dirs.len(),
            visited_dirs = phase1.stats.visited_dirs,
            manifests = phase1.stats.manifests,
            projects = phase1.stats.projects,
            non_rust_projects = phase1.stats.non_rust_projects,
            disk_entries = phase1.disk_entries.len(),
            "phase1_discover_total"
        );

        let tree_started = std::time::Instant::now();
        let projects = build_tree(&phase1.items, &inline_dirs);
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(tree_started.elapsed().as_millis()),
            input_items = phase1.items.len(),
            tree_items = projects.len(),
            "scan_tree_build"
        );

        let workspace_roots = collect_cargo_metadata_roots(&projects);
        let _ = scan_tx.send(BackgroundMsg::ScanResult {
            projects,
            disk_entries: phase1.disk_entries.clone(),
        });
        spawn_initial_disk_usage(&scan_context, &phase1.disk_entries);
        spawn_initial_language_stats(&scan_context, &phase1.disk_entries);
        spawn_cargo_metadata_tree(&scan_context, workspace_roots);
    });

    (tx, rx)
}

/// Collect distinct workspace roots that warrant a `cargo metadata`
/// dispatch — every Rust leaf project (workspace or standalone package),
/// worktree members included. Non-Rust roots are skipped.
fn collect_cargo_metadata_roots(projects: &[RootItem]) -> Vec<AbsolutePath> {
    let mut seen: HashSet<AbsolutePath> = HashSet::new();
    let mut roots = Vec::new();
    for item in projects {
        for root in cargo_metadata_roots_for_item(item) {
            if seen.insert(root.clone()) {
                roots.push(root);
            }
        }
    }
    roots
}

fn cargo_metadata_roots_for_item(item: &RootItem) -> Vec<AbsolutePath> {
    match item {
        RootItem::Rust(rust) => vec![rust.path().clone()],
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => std::iter::once(primary.path().clone())
            .chain(linked.iter().map(|ws| ws.path().clone()))
            .collect(),
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => std::iter::once(primary.path().clone())
            .chain(linked.iter().map(|pkg| pkg.path().clone()))
            .collect(),
        RootItem::NonRust(_) => Vec::new(),
    }
}

fn spawn_cargo_metadata_tree(scan_context: &StreamingScanContext, roots: Vec<AbsolutePath>) {
    for workspace_root in roots {
        let dispatch = MetadataDispatchContext {
            handle:         scan_context.client.handle.clone(),
            tx:             scan_context.tx.clone(),
            metadata_store: Arc::clone(&scan_context.metadata_store),
            metadata_limit: Arc::clone(&scan_context.metadata_limit),
        };
        spawn_cargo_metadata_refresh(dispatch, workspace_root);
    }
}

/// Context shared by any caller that wants to kick off a
/// `cargo metadata --no-deps --offline` task for a single workspace root.
/// The scan thread uses this to do initial dispatch; the watcher uses it
/// to re-run on manifest/config edits.
#[derive(Clone)]
pub(crate) struct MetadataDispatchContext {
    pub handle:         Handle,
    pub tx:             Sender<BackgroundMsg>,
    pub metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    pub metadata_limit: Arc<Semaphore>,
}

impl MetadataDispatchContext {
    /// Lock the store briefly and clone the resolved `target_directory`
    /// for any `path` inside a known workspace. Callers that hold a live
    /// `App` should use `App::resolve_target_dir` instead; this shim
    /// exists for the watcher, which holds the dispatch context but has
    /// no direct App handle.
    pub(crate) fn resolved_target_dir(&self, path: &AbsolutePath) -> Option<AbsolutePath> {
        self.metadata_store
            .lock()
            .ok()
            .and_then(|store| store.resolved_target_dir(path).cloned())
    }
}

/// Queue a `cargo metadata` invocation for `workspace_root` on the shared
/// tokio handle. Captures the fingerprint and bumps the store's dispatch
/// generation before the blocking `exec()` fires; arrivals round-trip
/// through `BackgroundMsg::CargoMetadata` so the main loop can gate on
/// the latest generation.
pub(crate) fn spawn_cargo_metadata_refresh(
    dispatch: MetadataDispatchContext,
    workspace_root: AbsolutePath,
) {
    let MetadataDispatchContext {
        handle,
        tx,
        metadata_store: store,
        metadata_limit: limit,
    } = dispatch;

    handle.spawn(async move {
        let Ok(_permit) = limit.acquire_owned().await else {
            return;
        };

        let workspace_root_for_task = workspace_root.clone();
        let blocking = tokio::task::spawn_blocking(move || {
            run_cargo_metadata_for_root(&workspace_root_for_task, &store)
        });
        let task_result = match tokio::time::timeout(CARGO_METADATA_TIMEOUT, blocking).await {
            Ok(Ok(output)) => output,
            Ok(Err(_)) => {
                tracing::warn!(
                    workspace_root = %workspace_root.display(),
                    "cargo_metadata_task_join_failed"
                );
                return;
            },
            Err(_) => {
                let fingerprint = ManifestFingerprint::capture(workspace_root.as_path())
                    .unwrap_or_else(|_| synthetic_fingerprint());
                CargoMetadataTaskOutput {
                    generation: 0,
                    fingerprint,
                    result: Err(CargoMetadataError::Other(format!(
                        "cargo metadata timed out after {}s",
                        CARGO_METADATA_TIMEOUT.as_secs()
                    ))),
                }
            },
        };

        let CargoMetadataTaskOutput {
            generation,
            fingerprint,
            result,
        } = task_result;
        let _ = tx.send(BackgroundMsg::CargoMetadata {
            workspace_root,
            generation,
            fingerprint,
            result,
        });
    });
}

struct CargoMetadataTaskOutput {
    generation:  u64,
    fingerprint: ManifestFingerprint,
    result:      Result<WorkspaceMetadata, CargoMetadataError>,
}

/// Walk `target_dir` on a blocking thread and emit its total byte size via
/// [`BackgroundMsg::OutOfTreeTargetSize`]. Used when workspace metadata
/// reports a `target_directory` outside its `workspace_root`; the scan-time
/// walker's per-project breakdown doesn't reach there, so this fills in the
/// sharer target size for the detail pane.
pub(crate) fn spawn_out_of_tree_target_walk(
    handle: &Handle,
    tx: Sender<BackgroundMsg>,
    workspace_root: AbsolutePath,
    target_dir: AbsolutePath,
) {
    handle.spawn(async move {
        let walk_target = target_dir.clone();
        let bytes = tokio::task::spawn_blocking(move || sum_dir_bytes(walk_target.as_path())).await;
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(
                    workspace_root = %workspace_root.display(),
                    target_dir = %target_dir.display(),
                    error = %err,
                    "out_of_tree_target_walk_join_failed"
                );
                return;
            },
        };
        tracing::debug!(
            workspace_root = %workspace_root.display(),
            target_dir = %target_dir.display(),
            bytes,
            "out_of_tree_target_walk_done"
        );
        let _ = tx.send(BackgroundMsg::OutOfTreeTargetSize {
            workspace_root,
            target_dir,
            bytes,
        });
    });
}

fn sum_dir_bytes(dir: &Path) -> u64 {
    WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok().map(|meta| meta.len()))
        .sum()
}

fn run_cargo_metadata_for_root(
    workspace_root: &AbsolutePath,
    store: &Arc<Mutex<WorkspaceMetadataStore>>,
) -> CargoMetadataTaskOutput {
    let generation = store
        .lock()
        .map_or(0, |mut guard| guard.next_generation(workspace_root));
    let fingerprint = match ManifestFingerprint::capture(workspace_root.as_path()) {
        Ok(fp) => fp,
        Err(err) => {
            // `NotFound` here means the workspace root vanished between
            // dispatch and run — the user just deleted a worktree, or a
            // similar race. Classify it as `WorkspaceMissing` so the
            // handler can suppress the toast at the type level. All other
            // I/O errors (permissions, etc.) flow into `Other`.
            let result = if err.kind() == ErrorKind::NotFound {
                Err(CargoMetadataError::WorkspaceMissing)
            } else {
                Err(CargoMetadataError::Other(format!(
                    "fingerprint capture failed: {err}"
                )))
            };
            return CargoMetadataTaskOutput {
                generation,
                fingerprint: synthetic_fingerprint(),
                result,
            };
        },
    };

    let manifest_path = workspace_root.as_path().join("Cargo.toml");
    let started_at = std::time::Instant::now();
    let result = match execute_cargo_metadata(&manifest_path) {
        Ok(metadata) => Ok(build_workspace_metadata(
            workspace_root.clone(),
            &metadata,
            fingerprint.clone(),
        )),
        Err(err) => Err(err),
    };
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started_at.elapsed().as_millis()),
        workspace_root = %workspace_root.display(),
        ok = result.is_ok(),
        "cargo_metadata_exec"
    );

    CargoMetadataTaskOutput {
        generation,
        fingerprint,
        result,
    }
}

fn execute_cargo_metadata(manifest_path: &Path) -> Result<Metadata, CargoMetadataError> {
    // Wall-clock cap lives on the caller via `tokio::time::timeout`;
    // `MetadataCommand::exec` itself has no timeout knob.
    let mut cmd = cargo_metadata::MetadataCommand::new();
    cmd.manifest_path(manifest_path).no_deps();
    cmd.other_options(vec!["--offline".to_string()]);
    cmd.exec()
        .map_err(|err| CargoMetadataError::Other(format_cargo_metadata_error(&err)))
}

fn format_cargo_metadata_error(err: &Error) -> String {
    let text = err.to_string();
    text.lines().next().unwrap_or(&text).to_string()
}

fn synthetic_fingerprint() -> ManifestFingerprint {
    use std::collections::BTreeMap;

    use crate::project::FileStamp;
    let now = std::time::SystemTime::now();
    ManifestFingerprint {
        manifest:       FileStamp {
            mtime:        now,
            len:          0,
            content_hash: [0_u8; 32],
        },
        lockfile:       None,
        rust_toolchain: None,
        configs:        BTreeMap::new(),
    }
}

fn build_workspace_metadata(
    workspace_root: AbsolutePath,
    metadata: &Metadata,
    fingerprint: ManifestFingerprint,
) -> WorkspaceMetadata {
    let target_directory =
        AbsolutePath::from(PathBuf::from(metadata.target_directory.as_std_path()));
    let workspace_members = metadata.workspace_members.clone();
    let packages = metadata
        .packages
        .iter()
        .map(|pkg| {
            let record = PackageRecord {
                id:            pkg.id.clone(),
                name:          pkg.name.to_string(),
                version:       pkg.version.clone(),
                edition:       pkg.edition.to_string(),
                description:   pkg.description.clone(),
                license:       pkg.license.clone(),
                homepage:      pkg.homepage.clone(),
                repository:    pkg.repository.clone(),
                manifest_path: AbsolutePath::from(PathBuf::from(pkg.manifest_path.as_std_path())),
                targets:       pkg
                    .targets
                    .iter()
                    .map(|target| TargetRecord {
                        name:              target.name.clone(),
                        kinds:             target.kind.clone(),
                        src_path:          AbsolutePath::from(PathBuf::from(
                            target.src_path.as_std_path(),
                        )),
                        edition:           target.edition.to_string(),
                        required_features: target.required_features.clone(),
                    })
                    .collect(),
                publish:       PublishPolicy::from_cargo_publish(pkg.publish.as_deref()),
            };
            (pkg.id.clone(), record)
        })
        .collect();
    WorkspaceMetadata {
        workspace_root,
        target_directory,
        packages,
        workspace_members,
        fetched_at: std::time::SystemTime::now(),
        fingerprint,
        out_of_tree_target_bytes: None,
    }
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
    items:        Vec<RootItem>,
    disk_entries: Vec<(String, AbsolutePath)>,
    stats:        Phase1DiscoverStats,
}

fn discover_non_rust_project(
    entry_path: &Path,
    items: &mut Vec<RootItem>,
    disk_entries: &mut Vec<(String, AbsolutePath)>,
    stats: &mut Phase1DiscoverStats,
) {
    let project = project::from_git_dir(entry_path);
    let abs_path = project.path().clone();
    stats.projects += 1;
    stats.non_rust_projects += 1;

    items.push(RootItem::NonRust(project));
    let disk_path = abs_path.to_string_lossy().into_owned();
    disk_entries.push((disk_path, abs_path));
}

fn phase1_discover(scan_dirs: &[AbsolutePath], non_rust: NonRustInclusion) -> Phase1DiscoverResult {
    let mut items = Vec::new();
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
                        entry.path(),
                        &mut items,
                        &mut disk_entries,
                        &mut stats,
                    );
                    continue;
                }
            }
            if entry.file_type().is_file() && entry.file_name() == "Cargo.toml" {
                stats.manifests += 1;
                let manifest_started = std::time::Instant::now();
                let Ok(cargo_project) = project::from_cargo_toml(entry.path()) else {
                    continue;
                };
                tracing::info!(
                    elapsed_ms = crate::perf_log::ms(manifest_started.elapsed().as_millis()),
                    manifest = %entry.path().display(),
                    "phase1_manifest_parse"
                );
                stats.projects += 1;
                let item = cargo_project_to_item(cargo_project);
                let abs_path = item.path().clone();
                let repo_presence_started = std::time::Instant::now();
                let repo_presence = if project::git_repo_root(&abs_path).is_some() {
                    GitRepoPresence::InRepo
                } else {
                    GitRepoPresence::OutsideRepo
                };
                tracing::info!(
                    elapsed_ms = crate::perf_log::ms(repo_presence_started.elapsed().as_millis()),
                    path = %abs_path,
                    in_repo = repo_presence.is_in_repo(),
                    "phase1_repo_presence"
                );

                items.push(item);
                disk_entries.push((abs_path.to_string_lossy().into_owned(), abs_path));
            }
        }
    }
    Phase1DiscoverResult {
        items,
        disk_entries,
        stats,
    }
}

// ── Language statistics (tokei) ─────────────────────────────────────

fn spawn_initial_language_stats(
    scan_context: &StreamingScanContext,
    disk_entries: &[(String, AbsolutePath)],
) {
    for tree in group_disk_usage_trees(disk_entries) {
        spawn_language_stats_tree(scan_context, tree);
    }
}

fn spawn_language_stats_tree(scan_context: &StreamingScanContext, tree: DiskUsageTree) {
    let handle = scan_context.client.handle.clone();
    let tx = scan_context.tx.clone();

    handle.spawn(async move {
        let Ok(results) =
            tokio::task::spawn_blocking(move || collect_language_stats_for_tree(&tree)).await
        else {
            return;
        };
        if !results.is_empty() {
            let _ = tx.send(BackgroundMsg::LanguageStatsBatch { entries: results });
        }
    });
}

fn collect_language_stats_for_tree(tree: &DiskUsageTree) -> Vec<(AbsolutePath, LanguageStats)> {
    let config = Config {
        hidden: Some(false),
        ..tokei::Config::default()
    };
    let mut languages = tokei::Languages::new();
    languages.get_statistics(&[&tree.root_abs_path], &[], &config);

    // Build a single LanguageStats from all results — this covers the root.
    let stats = build_language_stats(&languages);

    // For each entry in the tree, run tokei on that specific subtree if it
    // differs from the root. For simple single-project trees, just reuse
    // the root stats.
    if tree.entries.len() == 1 {
        return vec![(tree.entries[0].clone(), stats)];
    }

    // Multi-entry tree: root gets the full stats, members get their own.
    let mut results = Vec::with_capacity(tree.entries.len());
    for entry in &tree.entries {
        if entry.as_path() == tree.root_abs_path.as_path() {
            results.push((entry.clone(), stats.clone()));
        } else {
            let mut member_langs = tokei::Languages::new();
            member_langs.get_statistics(&[entry.as_path()], &[], &config);
            results.push((entry.clone(), build_language_stats(&member_langs)));
        }
    }
    results
}

fn build_language_stats(languages: &Languages) -> LanguageStats {
    let mut entries: Vec<LangEntry> = languages
        .iter()
        .filter(|(_, lang)| lang.code > 0 || !lang.reports.is_empty())
        .map(|(lang_type, lang)| {
            // Merge children (e.g., doc comments classified as embedded
            // Markdown) back into the parent language's counts.
            let (child_code, child_comments, child_blanks) = lang
                .children
                .values()
                .flat_map(|reports| reports.iter())
                .fold((0, 0, 0), |(c, m, b), report| {
                    (
                        c + report.stats.code,
                        m + report.stats.comments,
                        b + report.stats.blanks,
                    )
                });
            LangEntry {
                language: lang_type.to_string(),
                files:    lang.reports.len(),
                code:     lang.code + child_code,
                comments: lang.comments + child_comments,
                blanks:   lang.blanks + child_blanks,
            }
        })
        .collect();
    entries.sort_by_key(|entry| Reverse(entry.code));
    LanguageStats { entries }
}

/// Collect language stats for a single project path (watcher discovery).
pub(crate) fn collect_language_stats_single(path: &Path) -> LanguageStats {
    let config = Config {
        hidden: Some(false),
        ..tokei::Config::default()
    };
    let mut languages = tokei::Languages::new();
    languages.get_statistics(&[path], &[], &config);
    build_language_stats(&languages)
}

// ── Disk usage ─────────────────────────────────────────────────────

fn spawn_initial_disk_usage(
    scan_context: &StreamingScanContext,
    disk_entries: &[(String, AbsolutePath)],
) {
    for tree in group_disk_usage_trees(disk_entries) {
        spawn_disk_usage_tree(scan_context, tree);
    }
}

#[derive(Clone)]
struct DiskUsageTree {
    root_abs_path: AbsolutePath,
    entries:       Vec<AbsolutePath>,
}

fn group_disk_usage_trees(disk_entries: &[(String, AbsolutePath)]) -> Vec<DiskUsageTree> {
    let mut sorted: Vec<AbsolutePath> = disk_entries.iter().map(|(_, p)| p.clone()).collect();
    sorted.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });

    let mut trees: Vec<DiskUsageTree> = Vec::new();
    for abs_path in sorted {
        if let Some(tree) = trees
            .iter_mut()
            .find(|tree| abs_path.starts_with(&tree.root_abs_path))
        {
            tree.entries.push(abs_path);
        } else {
            let root = abs_path.clone();
            trees.push(DiskUsageTree {
                root_abs_path: root,
                entries:       vec![abs_path],
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
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(queue_elapsed.as_millis()),
            abs_path = %tree.root_abs_path.display(),
            rows = tree.entries.len(),
            "tokio_disk_queue_wait"
        );
        let run_started = std::time::Instant::now();
        let tree_for_walk = tree.clone();
        let Ok(results) =
            tokio::task::spawn_blocking(move || dir_sizes_for_tree(&tree_for_walk)).await
        else {
            return;
        };
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(run_started.elapsed().as_millis()),
            abs_path = %tree.root_abs_path.display(),
            rows = tree.entries.len(),
            "tokio_disk_usage"
        );
        let _ = tx.send(BackgroundMsg::DiskUsageBatch {
            root_path: tree.root_abs_path,
            entries:   results,
        });
    });
}

/// Per-project disk size breakdown emitted by the tree walker.
///
/// `total = in_project_target + in_project_non_target` by construction —
/// preserves the `disk_usage_bytes` formula for every owner (target is
/// in-tree) and naturally shrinks for a sharer (its `in_project_target
/// == 0` because the real target lives elsewhere under the workspace's
/// redirected `target_directory`).
///
/// "Is this file inside a `target/` subtree?" uses the literal
/// basename heuristic (any ancestor path component named `target`).
/// A workspace that redirects via `CARGO_TARGET_DIR` /
/// `.cargo/config.toml` ends up with `in_project_target = 0` for its
/// members, matching the sharer semantics the design plan lays out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DirSizes {
    pub total:                 u64,
    pub in_project_target:     u64,
    pub in_project_non_target: u64,
}

impl DirSizes {
    fn add_file(&mut self, bytes: u64, file_path: &Path) {
        self.total += bytes;
        if file_lives_under_target(file_path) {
            self.in_project_target += bytes;
        } else {
            self.in_project_non_target += bytes;
        }
    }
}

fn file_lives_under_target(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "target")
}

fn dir_sizes_for_tree(tree: &DiskUsageTree) -> Vec<(AbsolutePath, DirSizes)> {
    let mut totals: HashMap<AbsolutePath, DirSizes> = tree
        .entries
        .iter()
        .map(|abs_path| (abs_path.clone(), DirSizes::default()))
        .collect();

    for entry in WalkDir::new(&tree.root_abs_path).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let bytes = metadata.len();
        let file_path = entry.path();
        let mut current = file_path.parent();
        while let Some(dir) = current {
            if let Some(sizes) = totals.get_mut(dir) {
                sizes.add_file(bytes, file_path);
            }
            if dir == tree.root_abs_path.as_path() {
                break;
            }
            current = dir.parent();
        }
    }

    tree.entries
        .iter()
        .map(|abs_path| {
            let sizes = totals.get(abs_path.as_path()).copied().unwrap_or_default();
            (abs_path.clone(), sizes)
        })
        .collect()
}

pub(crate) fn disk_usage_batch_for_item(item: &RootItem) -> Vec<(AbsolutePath, DirSizes)> {
    let entries = item
        .collect_project_info()
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    let tree = DiskUsageTree {
        root_abs_path: item.path().clone(),
        entries,
    };
    dir_sizes_for_tree(&tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::AbsolutePath;
    use crate::project::WorktreeStatus;

    fn status_for(is_linked_worktree: bool, primary_abs: Option<&str>) -> WorktreeStatus {
        match (is_linked_worktree, primary_abs) {
            (_, None) => WorktreeStatus::NotGit,
            (true, Some(p)) => WorktreeStatus::Linked {
                primary: AbsolutePath::from(p.to_string()),
            },
            (false, Some(p)) => WorktreeStatus::Primary {
                root: AbsolutePath::from(p.to_string()),
            },
        }
    }

    fn make_workspace(
        name: Option<&str>,
        abs_path: &str,
        is_linked_worktree: bool,
        primary_abs: Option<&str>,
    ) -> RootItem {
        RootItem::Rust(RustProject::Workspace(Workspace {
            path: AbsolutePath::from(abs_path),
            name: name.map(String::from),
            worktree_status: status_for(is_linked_worktree, primary_abs),
            ..Workspace::default()
        }))
    }

    fn make_package(
        name: Option<&str>,
        abs_path: &str,
        is_linked_worktree: bool,
        primary_abs: Option<&str>,
    ) -> RootItem {
        RootItem::Rust(RustProject::Package(Package {
            path: AbsolutePath::from(abs_path),
            name: name.map(String::from),
            worktree_status: status_for(is_linked_worktree, primary_abs),
            ..Package::default()
        }))
    }

    #[test]
    fn merge_virtual_workspace() {
        let primary = make_workspace(None, "/home/ws", false, Some("/home/ws"));
        let worktree = make_workspace(None, "/home/ws_feat", true, Some("/home/ws"));
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1, "worktree should be merged into primary");
        let RootItem::Worktrees(WorktreeGroup::Workspaces { ref linked, .. }) = items[0] else {
            std::process::abort()
        };
        assert_eq!(linked.len(), 1, "should have one linked worktree");
    }

    #[test]
    fn merge_named_workspace() {
        let primary = make_workspace(Some("my-ws"), "/home/ws", false, Some("/home/ws"));
        let worktree = make_workspace(Some("my-ws"), "/home/ws_feat", true, Some("/home/ws"));
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1);
        let RootItem::Worktrees(WorktreeGroup::Workspaces { ref linked, .. }) = items[0] else {
            std::process::abort()
        };
        assert_eq!(linked.len(), 1);
    }

    #[test]
    fn ci_cache_dir_scopes_runs_by_repo() {
        let main_dir = ci_cache_dir_pub("acme", "demo");
        let feature_dir = ci_cache_dir_pub("acme", "demo");

        assert_eq!(main_dir, feature_dir);
        assert!(feature_dir.ends_with("acme/demo"));
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

        let workspace = make_workspace(Some("hana"), &workspace_dir.to_string_lossy(), false, None);
        let included = make_package(
            Some("hana-node-api"),
            &included_dir.to_string_lossy(),
            false,
            None,
        );
        let vendored = make_package(
            Some("clay-layout"),
            &vendored_dir.to_string_lossy(),
            false,
            None,
        );

        let items = build_tree(&[workspace, included, vendored], &["crates".to_string()]);

        let ws_item = items
            .iter()
            .find(|item| item.path() == workspace_dir.as_path())
            .unwrap_or_else(|| std::process::abort());
        let RootItem::Rust(RustProject::Workspace(ws)) = ws_item else {
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
        let primary = make_package(Some("app"), "/home/app", false, Some("/home/app"));
        let worktree = make_package(Some("app"), "/home/app_feat", true, Some("/home/app"));
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1);
        let RootItem::Worktrees(WorktreeGroup::Packages { ref linked, .. }) = items[0] else {
            std::process::abort()
        };
        assert_eq!(linked.len(), 1);
    }

    #[test]
    fn no_merge_different_repos() {
        let a = make_package(Some("a"), "/home/a", false, Some("/home/a"));
        let b = make_package(Some("b"), "/home/b", true, Some("/home/b"));
        let mut items = vec![a, b];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 2, "different repos should remain separate");
    }

    #[test]
    fn no_merge_none_identity() {
        let a = make_package(Some("x"), "/home/x", false, None);
        let b = make_package(Some("x"), "/home/x2", true, None);
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
            ("~/rust/bevy".to_string(), "/home/user/rust/bevy".into()),
            (
                "~/rust/bevy/crates/bevy_ecs".to_string(),
                "/home/user/rust/bevy/crates/bevy_ecs".into(),
            ),
            (
                "~/rust/bevy/tools/ci".to_string(),
                "/home/user/rust/bevy/tools/ci".into(),
            ),
            ("~/rust/hana".to_string(), "/home/user/rust/hana".into()),
        ]);

        assert_eq!(trees.len(), 2);
        assert_eq!(trees[0].root_abs_path, *Path::new("/home/user/rust/bevy"));
        assert_eq!(trees[0].entries.len(), 3);
        assert_eq!(trees[1].root_abs_path, *Path::new("/home/user/rust/hana"));
        assert_eq!(trees[1].entries.len(), 1);
    }

    #[test]
    fn dir_sizes_for_tree_accumulates_root_and_child_sizes_from_one_walk() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let root: AbsolutePath = tmp.path().join("bevy").into();
        let child: AbsolutePath = root.join("crates").join("bevy_ecs").into();
        std::fs::create_dir_all(&*child).unwrap_or_else(|_| std::process::abort());
        std::fs::write(root.join("root.txt"), vec![0_u8; 5])
            .unwrap_or_else(|_| std::process::abort());
        std::fs::write(child.join("child.txt"), vec![0_u8; 7])
            .unwrap_or_else(|_| std::process::abort());

        let sizes = dir_sizes_for_tree(&DiskUsageTree {
            root_abs_path: root.clone(),
            entries:       vec![root.clone(), child.clone()],
        });
        let sizes: HashMap<AbsolutePath, DirSizes> = sizes.into_iter().collect();

        assert_eq!(sizes.get(root.as_path()).map(|s| s.total), Some(12));
        assert_eq!(sizes.get(child.as_path()).map(|s| s.total), Some(7));
    }

    #[test]
    fn dir_sizes_for_tree_splits_target_and_non_target_bytes_in_one_pass() {
        // Step 5: confirm the single-pass walker partitions bytes
        // between `in_project_target` and `in_project_non_target`
        // based on whether any ancestor path component is named
        // `target`. A file at `<root>/target/debug/foo` is counted
        // as in-target; one at `<root>/src/main.rs` is not.
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let root: AbsolutePath = tmp.path().join("proj").into();
        let src = root.join("src");
        let target_debug = root.join("target").join("debug");
        std::fs::create_dir_all(&src).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&target_debug).unwrap_or_else(|_| std::process::abort());
        std::fs::write(src.join("main.rs"), vec![0_u8; 3])
            .unwrap_or_else(|_| std::process::abort());
        std::fs::write(target_debug.join("proj"), vec![0_u8; 17])
            .unwrap_or_else(|_| std::process::abort());

        let sizes = dir_sizes_for_tree(&DiskUsageTree {
            root_abs_path: root.clone(),
            entries:       vec![root],
        });
        let (_, entry) = &sizes[0];
        assert_eq!(entry.total, 20, "total bytes = 3 (src) + 17 (target)");
        assert_eq!(entry.in_project_target, 17, "target bytes isolated");
        assert_eq!(
            entry.in_project_non_target, 3,
            "non-target bytes exclude the target/ subtree"
        );
        assert_eq!(
            entry.in_project_target + entry.in_project_non_target,
            entry.total,
            "breakdown always sums to total (design plan formula)"
        );
    }

    // ── collect_cargo_metadata_roots ─────────────────────────────────

    #[test]
    fn collect_cargo_metadata_roots_yields_one_root_per_rust_leaf() {
        let ws = make_workspace(Some("ws"), "/ws", false, Some("/ws"));
        let pkg = make_package(Some("pkg"), "/pkg", false, Some("/pkg"));
        let roots = collect_cargo_metadata_roots(&[ws, pkg]);

        assert_eq!(
            roots,
            vec![AbsolutePath::from("/ws"), AbsolutePath::from("/pkg"),],
            "each Rust leaf produces exactly one metadata root, preserving input order"
        );
    }

    #[test]
    fn collect_cargo_metadata_roots_skips_non_rust_projects() {
        let non_rust = RootItem::NonRust(crate::project::NonRustProject::new(
            AbsolutePath::from("/notes"),
            Some("notes".into()),
        ));
        let pkg = make_package(Some("pkg"), "/pkg", false, Some("/pkg"));

        let roots = collect_cargo_metadata_roots(&[non_rust, pkg]);

        assert_eq!(
            roots,
            vec![AbsolutePath::from("/pkg")],
            "non-rust leaves never receive a metadata dispatch"
        );
    }

    #[test]
    fn collect_cargo_metadata_roots_unions_primary_and_linked_worktrees() {
        // Merge a primary + two linked worktrees into a group, then assert
        // every worktree gets its own metadata root.
        let primary = make_workspace(Some("ws"), "/ws", false, Some("/ws"));
        let linked_a = make_workspace(Some("ws_feat"), "/ws_feat", true, Some("/ws"));
        let linked_b = make_workspace(Some("ws_bug"), "/ws_bug", true, Some("/ws"));
        let mut items = vec![primary, linked_a, linked_b];
        merge_worktrees_new(&mut items);
        assert_eq!(items.len(), 1, "merged into one worktree group");

        let mut roots = collect_cargo_metadata_roots(&items);
        roots.sort_by(|a, b| a.as_path().cmp(b.as_path()));
        assert_eq!(
            roots,
            vec![
                AbsolutePath::from("/ws"),
                AbsolutePath::from("/ws_bug"),
                AbsolutePath::from("/ws_feat"),
            ],
            "primary + every linked worktree gets its own metadata root"
        );
    }

    #[test]
    fn collect_cargo_metadata_roots_dedupes_repeated_paths() {
        // Shouldn't happen in practice (each project has a unique path),
        // but the deduping logic is cheap and catches any future caller
        // that accidentally feeds the same root twice.
        let pkg_a = make_package(Some("a"), "/pkg", false, Some("/pkg"));
        let pkg_b = make_package(Some("b"), "/pkg", false, Some("/pkg"));

        let roots = collect_cargo_metadata_roots(&[pkg_a, pkg_b]);
        assert_eq!(roots, vec![AbsolutePath::from("/pkg")]);
    }
}
