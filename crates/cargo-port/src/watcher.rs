//! Watches project directories recursively for `target/` changes and
//! recalculates disk usage after a debounce period.
//!
//! Each project gets a single recursive watch at the project root.
//! Events outside `target/` are ignored. The 3-second debounce ensures
//! disk recalculation waits for build/clean activity to settle.
//!
//! On Linux (inotify) this creates one watch per subdirectory. The
//! default limit of 8192 handles ~10-25 projects; developers using
//! VS Code or JetBrains typically have this raised already.

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
const DEBOUNCE_DURATION: Duration = Duration::from_millis(500);

/// Maximum time before forcing a recalc even if events keep arriving.
const MAX_WAIT: Duration = Duration::from_secs(1);

/// How often the watcher thread checks for expired timers.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Request to start watching a project directory.
pub struct WatchRequest {
    /// Display path (e.g. `~/foo/bar`).
    pub project_path: String,
    /// Absolute filesystem path to the project root.
    pub abs_path:     PathBuf,
}

/// Spawn a background thread that watches project directories for
/// `target/` changes. Returns a sender for registering new projects.
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
    project_path: String,
    abs_path:     PathBuf,
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
    // project_path → (debounce_deadline, max_deadline, abs_path)
    let mut pending: HashMap<String, (Instant, Instant, PathBuf)> = HashMap::new();

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
                handle_event(&event, &projects, &mut pending);
            }
        }

        // Fire when debounce expires or max wait is reached.
        let now = Instant::now();
        let ready: Vec<String> = pending
            .iter()
            .filter(|(_, (debounce, max, _))| now >= *debounce || now >= *max)
            .map(|(key, _)| key.clone())
            .collect();

        for project_path in ready {
            if let Some((_, _, abs_path)) = pending.remove(&project_path) {
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
    let _ = watcher.watch(&req.abs_path, RecursiveMode::Recursive);

    projects.insert(
        req.abs_path.clone(),
        ProjectWatch {
            project_path: req.project_path,
            abs_path:     req.abs_path,
        },
    );
}

fn handle_event(
    event: &notify::Event,
    projects: &HashMap<PathBuf, ProjectWatch>,
    pending: &mut HashMap<String, (Instant, Instant, PathBuf)>,
) {
    let now = Instant::now();
    let debounce_deadline = now + DEBOUNCE_DURATION;

    for event_path in &event.paths {
        let Some((_, project)) = projects
            .iter()
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

        // Preserve the max deadline from the first event; reset the debounce.
        let max_deadline = pending
            .get(&project.project_path)
            .map_or(now + MAX_WAIT, |(_, max, _)| *max);

        pending.insert(
            project.project_path.clone(),
            (debounce_deadline, max_deadline, project.abs_path.clone()),
        );
    }
}
