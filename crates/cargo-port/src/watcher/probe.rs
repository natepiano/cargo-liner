use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::time::Instant;

use crate::config::NonRustInclusion;
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
use crate::scan::FetchContext;
use crate::scan::ProjectDetailRequest;

pub(super) fn spawn_project_refresh(bg_tx: Sender<BackgroundMsg>, project_root: AbsolutePath) {
    rayon::spawn(move || {
        let Some(item) = scan::discover_project_item(&project_root).or_else(|| {
            let cargo_toml = project_root.join("Cargo.toml");
            project::from_cargo_toml(&cargo_toml)
                .ok()
                .map(scan::cargo_project_to_item)
        }) else {
            return;
        };
        let disk_entries = scan::disk_usage_batch_for_item(&item);
        let root_path = AbsolutePath::from(item.path().to_path_buf());
        let _ = bg_tx.send(BackgroundMsg::ProjectRefreshed { item });
        let _ = bg_tx.send(BackgroundMsg::DiskUsageBatch {
            root_path,
            entries: disk_entries,
        });
    });
}

pub(super) fn spawn_project_refresh_after(
    bg_tx: Sender<BackgroundMsg>,
    project_root: AbsolutePath,
    delay: Duration,
) {
    rayon::spawn(move || {
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        spawn_project_refresh(bg_tx, project_root);
    });
}
pub(super) fn probe_new_projects(
    bg_tx: &Sender<BackgroundMsg>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
    discovered: &mut HashSet<AbsolutePath>,
    _ci_run_count: u32,
    non_rust: NonRustInclusion,
    client: &HttpClient,
) {
    let now = Instant::now();
    let ready: Vec<AbsolutePath> = pending_new
        .iter()
        .filter(|(_, deadline)| now >= **deadline)
        .map(|(path, _)| path.clone())
        .collect();

    for dir in ready {
        pending_new.remove(&dir);

        if !dir.is_dir() {
            // Directory was removed — send a zero-byte update so the app
            // can mark it as deleted if it was a tracked project.
            discovered.remove(&dir);
            let _ = bg_tx.send(BackgroundMsg::DiskUsage {
                path:  dir,
                bytes: 0,
            });
            continue;
        }

        if discovered.contains(&dir) {
            continue;
        }
        if let Some(item) = probe_project(&dir, non_rust) {
            discovered.insert(dir.clone());
            let abs_path = AbsolutePath::from(item.path().to_path_buf());
            let display_path = item.display_path();
            let project_name = item.name().map(str::to_string);
            let repo_presence = if project::git_repo_root(&abs_path).is_some() {
                GitRepoPresence::InRepo
            } else {
                GitRepoPresence::OutsideRepo
            };
            let disk_entries = scan::disk_usage_batch_for_item(&item);
            let _ = bg_tx.send(BackgroundMsg::ProjectDiscovered { item });
            let _ = bg_tx.send(BackgroundMsg::DiskUsageBatch {
                root_path: abs_path.clone(),
                entries:   disk_entries,
            });
            if abs_path.join("Cargo.toml").exists() {
                // Newly created Rust worktrees can be discovered before all
                // nested workspace members are visible. A delayed normalized
                // refresh repairs that initial partial state once the checkout
                // settles.
                spawn_project_refresh_after(bg_tx.clone(), abs_path.clone(), NEW_PROJECT_DEBOUNCE);
            }
            let tx = bg_tx.clone();
            let fetch_context = FetchContext {
                client: client.clone(),
            };
            enrichment::spawn_language_scan(abs_path.clone(), bg_tx.clone());
            rayon::spawn(move || {
                let request = ProjectDetailRequest {
                    tx: &tx,
                    fetch_context: &fetch_context,
                    _project_path: display_path.as_str(),
                    abs_path: &abs_path,
                    project_name: project_name.as_deref(),
                    repo_presence,
                };
                scan::fetch_project_details(&request);
            });
        }
    }
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
    let mut path = event_path.to_path_buf();
    let mut marker_candidate: Option<AbsolutePath> = None;
    loop {
        let parent = path.parent()?;
        if path.join("Cargo.toml").exists() || path.join(".git").exists() {
            marker_candidate = Some(AbsolutePath::from(path.clone()));
        }
        if watch_roots.iter().any(|r| parent == r.as_path()) || project_parents.contains(parent) {
            // Prefer the outermost directory under the known project-parent
            // boundary that carries project markers. This avoids discovering
            // workspace members as standalone projects when a new workspace
            // worktree is still emitting nested file events.
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
    let cargo_toml = dir.join("Cargo.toml");
    if cargo_toml.exists() {
        return scan::discover_project_item(dir);
    }
    if non_rust.includes_non_rust() && dir.join(".git").is_dir() {
        return Some(NonRust(project::from_git_dir(dir)));
    }
    None
}
