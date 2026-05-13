use std::path::Path;
use std::sync::mpsc::Sender;

use crate::cache_paths;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::ManifestFingerprint;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::Submodule;
use crate::project::WorkspaceMetadata;

mod cargo_metadata;
mod ci_cache;
mod discovery;
mod disk_usage;
mod language_stats;
mod tree;

pub(crate) use cargo_metadata::CargoMetadataError;
pub(crate) use cargo_metadata::MetadataDispatchContext;
pub(crate) use cargo_metadata::spawn_cargo_metadata_refresh;
pub(crate) use cargo_metadata::spawn_out_of_tree_target_walk;
pub(crate) use cargo_metadata::spawn_streaming_scan;
pub(crate) use ci_cache::CiFetchResult;
pub(crate) use ci_cache::CratesIoInfo;
pub(crate) use ci_cache::ci_cache_dir_pub;
pub(crate) use ci_cache::clear_exhausted;
pub(crate) use ci_cache::fetch_ci_runs_cached;
pub(crate) use ci_cache::fetch_older_runs;
pub(crate) use ci_cache::is_exhausted;
pub(crate) use ci_cache::mark_exhausted;
pub(crate) use discovery::CachedRepoData;
pub(crate) use discovery::FetchContext;
pub(crate) use discovery::ProjectDetailRequest;
pub(crate) use discovery::RepoCache;
pub(crate) use discovery::RepoMetaInfo;
pub(crate) use discovery::discover_project_item;
pub(crate) use discovery::fetch_project_details;
pub(crate) use discovery::invalidate_cached_repo_data;
pub(crate) use discovery::load_cached_repo_data;
pub(crate) use discovery::new_repo_cache;
pub(crate) use discovery::resolve_include_dirs;
pub(crate) use discovery::store_cached_repo_data;
pub(crate) use disk_usage::DirSizes;
pub(crate) use disk_usage::disk_usage_batch_for_item;
pub(crate) use language_stats::collect_language_stats_single;
pub(crate) use tree::build_tree;
pub(crate) use tree::cargo_project_to_item;
pub(crate) use tree::dir_size;
pub(crate) use tree::normalize_workspace_path;

/// Messages sent from background threads to the main event loop.
pub(crate) enum BackgroundMsg {
    /// Disk usage (bytes) computed for a single project path.
    DiskUsage { path: AbsolutePath, bytes: u64 },
    /// Batch of disk usage results for projects under a common root.
    /// Each entry carries both the total and the in-target /
    /// non-target split used by the detail-pane breakdown.
    DiskUsageBatch {
        root_path: AbsolutePath,
        entries:   Vec<(AbsolutePath, DirSizes)>,
    },
    /// GitHub Actions CI runs fetched for a project.
    CiRuns {
        path:         AbsolutePath,
        runs:         Vec<crate::ci::CiRun>,
        github_total: u32,
    },
    /// A GitHub repo fetch has been queued (for startup tracking).
    RepoFetchQueued { repo: crate::ci::OwnerRepo },
    /// A `spawn_repo_fetch_for_git_info` thread has finished. Sent
    /// regardless of whether the spawn hit the network or returned a
    /// cached result. Drives both the startup "GitHub repos" toast
    /// progress (no-op on cache hit, since no `RepoFetchQueued` was
    /// sent) and the `repo_fetch_in_flight` dedup set.
    RepoFetchComplete { repo: crate::ci::OwnerRepo },
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
        status: crate::lint::LintStatus,
    },
    /// Startup lint cache check result. Sent once per registered project
    /// when the lint runtime reads cached lint results from disk during
    /// initialization. Distinct from `LintStatus` so the app can track
    /// when all startup cache checks are complete.
    LintStartupStatus {
        path:   AbsolutePath,
        status: crate::lint::LintStatus,
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
        entries: Vec<(AbsolutePath, crate::project::LanguageStats)>,
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

pub(super) const fn combine_service_signal(
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

/// Base cache directory for CI metadata.
pub(crate) fn cache_dir() -> AbsolutePath { cache_paths::ci_cache_root() }
