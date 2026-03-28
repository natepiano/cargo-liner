//! Watches project `target/` directories for filesystem changes and
//! recalculates disk usage after a debounce period.
//!
//! Each project gets two non-recursive watches:
//! - **Project root** — detects `target/` creation and deletion.
//! - **`target/`** (when present) — detects build activity via files cargo touches directly in
//!   `target/` (e.g. `.rustc_info.json`).
//!
//! On macOS (FSEvents) the non-recursive watches still report subtree
//! events, so build activity is caught even without a recursive watch.
//! On Linux (inotify) the `target/`-level watch catches the direct
//! children that cargo modifies on every build.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use notify::RecursiveMode;
use notify::Watcher;

use crate::scan;
use crate::scan::BackgroundMsg;

/// Wait for build/clean activity to settle before recalculating.
const DEBOUNCE_DURATION: Duration = Duration::from_secs(3);

/// How often the watcher thread checks for expired debounce timers.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Request to start watching a project's `target/` directory.
pub struct WatchRequest {
    /// Display path (e.g. `~/foo/bar`).
    pub project_path: String,
    /// Absolute filesystem path to the project root.
    pub abs_path:     PathBuf,
}

/// Spawn a background thread that watches project `target/` directories
/// for filesystem changes. Returns a sender for registering new projects.
///
/// When changes are detected (with debouncing), the thread recalculates
/// `dir_size` and sends `BackgroundMsg::DiskUsage` through `bg_tx`.
pub fn spawn_disk_watcher(bg_tx: mpsc::Sender<BackgroundMsg>) -> mpsc::Sender<WatchRequest> {
    let (watch_tx, watch_rx) = mpsc::channel();

    thread::spawn(move || {
        watcher_loop(bg_tx, watch_rx);
    });

    watch_tx
}

/// Per-project tracking state.
struct ProjectWatch {
    project_path:   String,
    abs_path:       PathBuf,
    target_watched: bool,
}

fn watcher_loop(bg_tx: mpsc::Sender<BackgroundMsg>, watch_rx: mpsc::Receiver<WatchRequest>) {
    let (notify_tx, notify_rx) = mpsc::channel();
    let handler = move |res| {
        let _ = notify_tx.send(res);
    };
    let Ok(mut watcher) = notify::recommended_watcher(handler) else {
        return;
    };

    // project abs_path → watch state
    let mut projects: HashMap<PathBuf, ProjectWatch> = HashMap::new();
    // project_path → (deadline, abs_path) for debouncing
    let mut pending: HashMap<String, (Instant, PathBuf)> = HashMap::new();

    loop {
        // Drain new registrations (exit when the sender is dropped).
        loop {
            match watch_rx.try_recv() {
                Ok(req) => register(&mut watcher, &mut projects, req),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }

        // Drain filesystem events.
        while let Ok(result) = notify_rx.try_recv() {
            if let Ok(event) = result {
                handle_event(&event, &mut watcher, &mut projects, &mut pending);
            }
        }

        // Fire any expired debounce timers.
        let now = Instant::now();
        let ready: Vec<String> = pending
            .iter()
            .filter(|(_, (deadline, _))| now >= *deadline)
            .map(|(key, _)| key.clone())
            .collect();

        for project_path in ready {
            if let Some((_, abs_path)) = pending.remove(&project_path) {
                let bytes = scan::dir_size(&abs_path);
                if bg_tx
                    .send(BackgroundMsg::DiskUsage {
                        path: project_path,
                        bytes,
                    })
                    .is_err()
                {
                    return; // main app disconnected
                }
            }
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn register(
    watcher: &mut impl Watcher,
    projects: &mut HashMap<PathBuf, ProjectWatch>,
    req: WatchRequest,
) {
    // Watch project root (catches target/ creation/deletion).
    let _ = watcher.watch(&req.abs_path, RecursiveMode::NonRecursive);

    // Watch target/ if it already exists (catches build activity).
    let target_dir = req.abs_path.join("target");
    let target_watched = target_dir.is_dir()
        && watcher
            .watch(&target_dir, RecursiveMode::NonRecursive)
            .is_ok();

    projects.insert(
        req.abs_path.clone(),
        ProjectWatch {
            project_path: req.project_path,
            abs_path: req.abs_path,
            target_watched,
        },
    );
}

fn handle_event(
    event: &notify::Event,
    watcher: &mut impl Watcher,
    projects: &mut HashMap<PathBuf, ProjectWatch>,
    pending: &mut HashMap<String, (Instant, PathBuf)>,
) {
    let deadline = Instant::now() + DEBOUNCE_DURATION;

    for event_path in &event.paths {
        let Some((_, project)) = projects
            .iter_mut()
            .find(|(root, _)| event_path.starts_with(root))
        else {
            continue;
        };

        // Only care about target-related changes.
        let relative = event_path
            .strip_prefix(&project.abs_path)
            .unwrap_or(event_path);
        let is_target = relative
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == "target");
        if !is_target {
            continue;
        }

        // If target/ just appeared, start watching it.
        let target_dir = project.abs_path.join("target");
        if !project.target_watched && target_dir.is_dir() {
            if watcher
                .watch(&target_dir, RecursiveMode::NonRecursive)
                .is_ok()
            {
                project.target_watched = true;
            }
        }

        // If target/ was removed, mark the watch as stale.
        if project.target_watched && !target_dir.is_dir() {
            project.target_watched = false;
        }

        pending.insert(
            project.project_path.clone(),
            (deadline, project.abs_path.clone()),
        );
    }
}
