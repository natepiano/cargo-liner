//! Watches the scan root recursively for filesystem changes and maps
//! events to discovered projects for disk-usage recalculation.
//!
//! A single `notify` subscription covers the entire scan root. Events are
//! matched to projects by prefix, debounced, and result in a
//! `BackgroundMsg::DiskUsage` update. New project directories are detected
//! automatically; removed directories trigger a zero-byte update so the
//! app can mark them as deleted.
//!
//! On macOS (`FSEvents`) this is a single kernel subscription regardless of
//! tree size. Linux / Windows may want a per-project approach in the
//! future to avoid inotify watch limits.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use notify::RecursiveMode;
use notify::Watcher;

use super::config::NonRustInclusion;
use super::constants::DEBOUNCE_DURATION;
use super::constants::MAX_WAIT;
use super::constants::NEW_PROJECT_DEBOUNCE;
use super::constants::POLL_INTERVAL;
use super::project;
use super::project::GitTracking;
use super::project::RustProject;
use super::scan;
use super::scan::BackgroundMsg;

/// Request to register an already-known project with the watcher.
pub struct WatchRequest {
    /// Display path (e.g. `~/foo/bar`).
    pub project_path: String,
    /// Absolute filesystem path to the project root.
    pub abs_path:     PathBuf,
}

/// Spawn a unified background watcher thread. Watches the scan root
/// recursively and handles disk-usage updates, new-project detection,
/// and deleted-project detection through a single `notify` subscription.
pub fn spawn_watcher(
    scan_root: PathBuf,
    bg_tx: mpsc::Sender<BackgroundMsg>,
    ci_run_count: u32,
    non_rust: NonRustInclusion,
) -> mpsc::Sender<WatchRequest> {
    let (watch_tx, watch_rx) = mpsc::channel();

    thread::spawn(move || {
        watcher_loop(scan_root, bg_tx, watch_rx, ci_run_count, non_rust);
    });

    watch_tx
}

/// Per-project tracking state.
struct ProjectEntry {
    project_path: String,
    abs_path:     PathBuf,
}

#[allow(clippy::needless_pass_by_value)]
fn watcher_loop(
    scan_root: PathBuf,
    bg_tx: mpsc::Sender<BackgroundMsg>,
    watch_rx: mpsc::Receiver<WatchRequest>,
    ci_run_count: u32,
    non_rust: NonRustInclusion,
) {
    let (notify_tx, notify_rx) = mpsc::channel();
    let handler = move |res| {
        let _ = notify_tx.send(res);
    };
    let Ok(mut watcher) = notify::recommended_watcher(handler) else {
        return;
    };
    if watcher.watch(&scan_root, RecursiveMode::Recursive).is_err() {
        return;
    }

    // `abs_path` → project tracking state
    let mut projects: HashMap<PathBuf, ProjectEntry> = HashMap::new();
    // project_path → (debounce_deadline, max_deadline)
    let mut pending_disk: HashMap<String, (Instant, Instant)> = HashMap::new();
    // Directories that might be new projects → probe deadline
    let mut pending_new: HashMap<PathBuf, Instant> = HashMap::new();
    // Directories already discovered as new projects by this watcher.
    let mut discovered: HashSet<PathBuf> = HashSet::new();

    loop {
        // Drain new registrations (exit when the app disconnects).
        loop {
            match watch_rx.try_recv() {
                Ok(req) => {
                    projects.insert(
                        req.abs_path.clone(),
                        ProjectEntry {
                            project_path: req.project_path,
                            abs_path:     req.abs_path,
                        },
                    );
                },
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }

        // Drain filesystem events.
        while let Ok(result) = notify_rx.try_recv() {
            let Ok(event) = result else {
                continue;
            };
            for event_path in &event.paths {
                handle_event(
                    event_path,
                    &scan_root,
                    &projects,
                    &discovered,
                    &mut pending_disk,
                    &mut pending_new,
                );
            }
        }

        // Fire disk recalculations whose debounce has expired.
        fire_disk_updates(&bg_tx, &projects, &mut pending_disk);

        // Probe new-project candidates whose debounce has expired.
        probe_new_projects(
            &bg_tx,
            &mut pending_new,
            &mut discovered,
            ci_run_count,
            non_rust,
        );

        thread::sleep(POLL_INTERVAL);
    }
}

fn handle_event(
    event_path: &Path,
    scan_root: &Path,
    projects: &HashMap<PathBuf, ProjectEntry>,
    discovered: &HashSet<PathBuf>,
    pending_disk: &mut HashMap<String, (Instant, Instant)>,
    pending_new: &mut HashMap<PathBuf, Instant>,
) {
    let now = Instant::now();

    // Try to match the event to a known project.
    if let Some((_, entry)) = projects
        .iter()
        .find(|(root, _)| event_path.starts_with(root))
    {
        let debounce_deadline = now + DEBOUNCE_DURATION;
        let max_deadline = pending_disk
            .get(&entry.project_path)
            .map_or(now + MAX_WAIT, |(_, max)| *max);
        pending_disk.insert(
            entry.project_path.clone(),
            (debounce_deadline, max_deadline),
        );
        return;
    }

    // Not a known project — check if this is a direct child of the scan
    // root (potential new project or deleted project).
    let Some(parent) = event_path.parent() else {
        return;
    };
    if parent != scan_root {
        return;
    }
    // Always enqueue removals (dir gone); for creations, skip already-discovered.
    if !event_path.is_dir() || !discovered.contains(event_path) {
        pending_new
            .entry(event_path.to_path_buf())
            .or_insert_with(|| now + NEW_PROJECT_DEBOUNCE);
    }
}

fn fire_disk_updates(
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    projects: &HashMap<PathBuf, ProjectEntry>,
    pending_disk: &mut HashMap<String, (Instant, Instant)>,
) {
    let now = Instant::now();
    let ready: Vec<String> = pending_disk
        .iter()
        .filter(|(_, (debounce, max))| now >= *debounce || now >= *max)
        .map(|(key, _)| key.clone())
        .collect();

    for project_path in ready {
        pending_disk.remove(&project_path);
        let Some(entry) = projects.values().find(|e| e.project_path == project_path) else {
            continue;
        };
        let bytes = scan::dir_size(&entry.abs_path);
        if bg_tx
            .send(BackgroundMsg::DiskUsage {
                path: project_path,
                bytes,
            })
            .is_err()
        {
            return;
        }
    }
}

fn probe_new_projects(
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    pending_new: &mut HashMap<PathBuf, Instant>,
    discovered: &mut HashSet<PathBuf>,
    ci_run_count: u32,
    non_rust: NonRustInclusion,
) {
    let now = Instant::now();
    let ready: Vec<PathBuf> = pending_new
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
            let display_path = project::home_relative_path(&dir);
            let _ = bg_tx.send(BackgroundMsg::DiskUsage {
                path:  display_path,
                bytes: 0,
            });
            continue;
        }

        if discovered.contains(&dir) {
            continue;
        }
        if let Some(project) = probe_project(&dir, non_rust) {
            discovered.insert(dir.clone());
            let abs_path = PathBuf::from(&project.abs_path);
            let git_tracking = if abs_path.join(".git").exists() {
                GitTracking::Tracked
            } else {
                GitTracking::Untracked
            };
            let _ = bg_tx.send(BackgroundMsg::ProjectDiscovered {
                project: project.clone(),
            });
            let tx = bg_tx.clone();
            let path = project.path.clone();
            let name = project.name.clone();
            rayon::spawn(move || {
                scan::fetch_project_details(
                    &tx,
                    &path,
                    &abs_path,
                    name.as_ref(),
                    git_tracking,
                    ci_run_count,
                );
            });
        }
    }
}

/// Check if a directory is a project (has `Cargo.toml`, or `.git` when
/// `include_non_rust` is enabled).
fn probe_project(dir: &Path, non_rust: NonRustInclusion) -> Option<RustProject> {
    let cargo_toml = dir.join("Cargo.toml");
    if cargo_toml.exists() {
        return RustProject::from_cargo_toml(&cargo_toml).ok();
    }
    if non_rust.includes_non_rust() && dir.join(".git").is_dir() {
        return Some(RustProject::from_git_dir(dir));
    }
    None
}
