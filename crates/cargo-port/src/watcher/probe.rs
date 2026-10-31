use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use super::runtime::WatchRequest;
use crate::channel::Sender;
use crate::config::NonRustInclusion;
use crate::constants::CARGO_TOML;
use crate::constants::GIT_DIR;
use crate::constants::NEW_PROJECT_DEBOUNCE;
use crate::enrichment;
use crate::http::HttpClient;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::GitRepoPresence;
use crate::project::RootItem;
use crate::project::RootItem::NonRust;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::ExcludeDirs;
use crate::scan::FetchContext;
use crate::scan::ProjectDetailRequest;

pub(super) fn spawn_project_refresh(
    background_tx: Sender<BackgroundMsg>,
    project_root: AbsolutePath,
) {
    rayon::spawn(move || {
        let Some(item) = scan::discover_project_item(&project_root).or_else(|| {
            let cargo_toml = project_root.join(CARGO_TOML);
            project::from_cargo_toml(&cargo_toml)
                .ok()
                .map(scan::cargo_parse_result_to_item)
        }) else {
            return;
        };
        let disk_entries = scan::disk_usage_batch_for_item(&item);
        let root_path = AbsolutePath::from(item.path().to_path_buf());
        let _ = background_tx.send(BackgroundMsg::ProjectRefreshed { item });
        let _ = background_tx.send(BackgroundMsg::DiskUsageBatch {
            root_path,
            entries: disk_entries,
        });
    });
}

pub(super) fn spawn_project_refresh_after(
    background_tx: Sender<BackgroundMsg>,
    project_root: AbsolutePath,
    delay: Duration,
) {
    rayon::spawn(move || {
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        spawn_project_refresh(background_tx, project_root);
    });
}
pub(super) enum FolderProbeOutcome {
    Project(WatchRequest),
    NotProject(AbsolutePath),
}

/// Classify one folder through the production parser and outcome builder,
/// without launching enrichment or delayed refresh work.
#[cfg(test)]
pub(super) fn probe_folder(dir: &AbsolutePath, non_rust: NonRustInclusion) -> FolderProbeOutcome {
    probe_project(dir, non_rust).map_or_else(
        || FolderProbeOutcome::NotProject(dir.clone()),
        |item| project_probe_outcome(&item),
    )
}

fn project_probe_outcome(item: &RootItem) -> FolderProbeOutcome {
    FolderProbeOutcome::Project(WatchRequest {
        project_label: item.path().to_string(),
        abs_path:      item.path().clone(),
        repo_root:     project::git_repo_root(item.path()),
    })
}

/// Probe and publish discovery data; the caller applies each watch-role result.
pub(super) fn probe_new_projects(
    background_tx: &Sender<BackgroundMsg>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
    discovered: &mut HashSet<AbsolutePath>,
    _: u32,
    non_rust: NonRustInclusion,
    exclude_dirs: &ExcludeDirs,
    client: &HttpClient,
) -> Vec<FolderProbeOutcome> {
    let mut outcomes = Vec::new();
    let now = Instant::now();
    let ready: Vec<AbsolutePath> = pending_new
        .iter()
        .filter(|(_, deadline)| now >= **deadline)
        .map(|(path, _)| path.clone())
        .collect();

    for dir in ready {
        pending_new.remove(&dir);

        // `exclude_dirs.excludes` rejects names before the missing-directory branch: excluded
        // directories never became projects, so there is no tracked row to retire.
        if dir
            .file_name()
            .is_some_and(|name| exclude_dirs.excludes(name))
        {
            outcomes.push(FolderProbeOutcome::NotProject(dir));
            continue;
        }

        if !dir.is_dir() {
            // The shared outcome application publishes zero-byte DiskUsage and
            // removes candidate subscriptions without removing the app's row.
            discovered.remove(&dir);
            outcomes.push(FolderProbeOutcome::NotProject(dir));
            continue;
        }

        if discovered.contains(&dir) {
            continue;
        }
        if let Some(item) = probe_project(&dir, non_rust) {
            discovered.insert(dir.clone());
            let abs_path = AbsolutePath::from(item.path().to_path_buf());
            outcomes.push(project_probe_outcome(&item));
            let project_name = item.name().map(str::to_string);
            let repo_presence = if project::git_repo_root(&abs_path).is_some() {
                GitRepoPresence::InRepo
            } else {
                GitRepoPresence::OutsideRepo
            };
            let disk_entries = scan::disk_usage_batch_for_item(&item);
            let _ = background_tx.send(BackgroundMsg::ProjectDiscovered { item });
            let _ = background_tx.send(BackgroundMsg::DiskUsageBatch {
                root_path: abs_path.clone(),
                entries:   disk_entries,
            });
            if abs_path.join(CARGO_TOML).exists() {
                // `spawn_project_refresh_after` normalizes `abs_path` after `NEW_PROJECT_DEBOUNCE`:
                // new Rust worktrees may be discovered before nested workspace members are visible,
                // so the delayed refresh repairs partial state once the checkout settles.
                spawn_project_refresh_after(
                    background_tx.clone(),
                    abs_path.clone(),
                    NEW_PROJECT_DEBOUNCE,
                );
            }
            let sender = background_tx.clone();
            let fetch_context = FetchContext {
                client: client.clone(),
            };
            enrichment::spawn_language_scan(abs_path.clone(), background_tx.clone());
            rayon::spawn(move || {
                let request = ProjectDetailRequest {
                    sender: &sender,
                    fetch_context: &fetch_context,
                    abs_path: &abs_path,
                    name: project_name.as_deref(),
                    repo_presence,
                };
                scan::fetch_project_details(&request);
            });
        } else {
            outcomes.push(FolderProbeOutcome::NotProject(dir));
        }
    }
    outcomes
}

/// Walk up from `event_path` toward `scan_root`, returning the first
/// directory whose parent is a known project-parent directory or one of
/// the watch roots. This finds the directory at the same nesting level as
/// existing projects regardless of how deep the watch roots are.
///
/// When the walk-up doesn't find a known project parent, a filesystem
/// check for `Cargo.toml` or `.git` identifies project roots that
/// aren't yet registered (new projects added during or after the scan).
pub(super) fn project_level_dir(
    event_path: &Path,
    watch_roots: &[AbsolutePath],
    project_parents: &HashSet<AbsolutePath>,
) -> Option<AbsolutePath> {
    if let Some(root) = watch_roots.iter().find(|root| {
        event_path == root.as_path()
            || event_path.parent() == Some(root.as_path())
                && event_path
                    .file_name()
                    .is_some_and(|name| name == CARGO_TOML || name == GIT_DIR)
    }) {
        return Some(root.clone());
    }
    let mut path = event_path.to_path_buf();
    let mut marker_candidate: Option<AbsolutePath> = None;
    loop {
        let parent = path.parent()?;
        if path.join(CARGO_TOML).exists() || path.join(GIT_DIR).exists() {
            marker_candidate = Some(AbsolutePath::from(path.clone()));
        }
        if watch_roots.iter().any(|r| parent == r.as_path()) || project_parents.contains(parent) {
            // Return the outermost `marker_candidate` beneath the known project-parent boundary,
            // falling back to `path`; choosing an outer marker avoids discovering workspace members
            // as standalone projects while a new workspace worktree emits nested file events.
            return Some(marker_candidate.unwrap_or_else(|| AbsolutePath::from(path)));
        }
        if !watch_roots.iter().any(|r| path.starts_with(r.as_path())) {
            return None;
        }
        path = parent.to_path_buf();
    }
}

/// Check if a directory is a project (has `Cargo.toml`, or `.git` when
/// `include_non_rust` is enabled).
pub(super) fn probe_project(dir: &Path, non_rust: NonRustInclusion) -> Option<RootItem> {
    let cargo_toml = dir.join(CARGO_TOML);
    if cargo_toml.exists() {
        return scan::discover_project_item(dir);
    }
    if non_rust.includes_non_rust() && dir.join(GIT_DIR).is_dir() {
        return Some(NonRust(project::from_git_dir(dir)));
    }
    None
}
