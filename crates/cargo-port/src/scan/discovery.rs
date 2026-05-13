use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::Sender;

use walkdir::WalkDir;

use super::BackgroundMsg;
use super::emit_git_info;
use super::emit_service_signal;
use super::tree;
use crate::ci::OwnerRepo;
use crate::config::NonRustInclusion;
use crate::enrichment;
use crate::http::HttpClient;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::GitRepoPresence;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::ci::CiRun;

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
            items.push(tree::cargo_project_to_item(parsed));
        }
    }

    if items.is_empty() {
        return None;
    }

    tree::build_tree(&items, &[])
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
    let bytes = tree::dir_size(abs_path);
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

/// Walk `scan_dirs`, discover projects, and stream per-project work immediately. Discovery and
/// local metadata collection stay on the dedicated scan thread, while disk and network work are
/// dispatched onto bounded background queues.
pub(super) struct Phase1DiscoverStats {
    pub(super) visited_dirs:      usize,
    pub(super) manifests:         usize,
    pub(super) projects:          usize,
    pub(super) non_rust_projects: usize,
}

pub(super) struct Phase1DiscoverResult {
    pub(super) items:        Vec<RootItem>,
    pub(super) disk_entries: Vec<(String, AbsolutePath)>,
    pub(super) stats:        Phase1DiscoverStats,
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

pub(super) fn phase1_discover(
    scan_dirs: &[AbsolutePath],
    non_rust: NonRustInclusion,
) -> Phase1DiscoverResult {
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
                let item = tree::cargo_project_to_item(cargo_project);
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
