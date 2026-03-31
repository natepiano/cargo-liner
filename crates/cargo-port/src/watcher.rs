//! Watches the scan root recursively for filesystem changes and maps
//! events to discovered projects for disk-usage and git-sync updates.
//!
//! A single `notify` subscription covers the entire scan root. Events are
//! matched to projects by prefix, debounced, and result in both
//! `BackgroundMsg::DiskUsage` and `BackgroundMsg::GitInfo` updates. New project directories are
//! detected automatically; removed directories trigger a zero-byte update so the
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
use super::http::HttpClient;
use super::project;
use super::project::GitInfo;
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
    /// Whether this project has git tracking.
    pub git_tracking: GitTracking,
}

/// Spawn a unified background watcher thread. Watches the include
/// directories recursively and handles disk-usage updates,
/// new-project detection, and deleted-project detection.
pub fn spawn_watcher(
    scan_root: PathBuf,
    bg_tx: mpsc::Sender<BackgroundMsg>,
    ci_run_count: u32,
    non_rust: NonRustInclusion,
    include_dirs: Vec<String>,
    client: HttpClient,
) -> mpsc::Sender<WatchRequest> {
    let (watch_tx, watch_rx) = mpsc::channel();

    thread::spawn(move || {
        watcher_loop(
            &scan_root,
            &bg_tx,
            &watch_rx,
            ci_run_count,
            non_rust,
            &include_dirs,
            &client,
        );
    });

    watch_tx
}

/// Per-project tracking state.
struct ProjectEntry {
    project_path: String,
    abs_path:     PathBuf,
    git_tracking: GitTracking,
}

fn watcher_loop(
    scan_root: &Path,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    watch_rx: &mpsc::Receiver<WatchRequest>,
    ci_run_count: u32,
    non_rust: NonRustInclusion,
    include_dirs: &[String],
    client: &HttpClient,
) {
    let watch_dirs = scan::resolve_include_dirs(scan_root, include_dirs);
    let (notify_tx, notify_rx) = mpsc::channel();
    let handler = move |res| {
        let _ = notify_tx.send(res);
    };
    let Ok(mut watcher) = notify::recommended_watcher(handler) else {
        return;
    };
    for dir in &watch_dirs {
        if dir.is_dir() {
            let _ = watcher.watch(dir, RecursiveMode::Recursive);
        }
    }

    // `abs_path` → project tracking state
    let mut projects: HashMap<PathBuf, ProjectEntry> = HashMap::new();
    // Directories that contain at least one known project (e.g. `~/rust/`).
    let mut project_parents: HashSet<PathBuf> = HashSet::new();
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
                    if let Some(parent) = req.abs_path.parent() {
                        project_parents.insert(parent.to_path_buf());
                    }
                    projects.insert(
                        req.abs_path.clone(),
                        ProjectEntry {
                            project_path: req.project_path,
                            abs_path:     req.abs_path,
                            git_tracking: req.git_tracking,
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
            let ctx = EventContext {
                scan_root,
                projects: &projects,
                project_parents: &project_parents,
                discovered: &discovered,
            };
            for event_path in &event.paths {
                handle_event(event_path, &ctx, &mut pending_disk, &mut pending_new);
            }
        }

        // Fire disk recalculations whose debounce has expired.
        fire_disk_updates(bg_tx, &projects, &mut pending_disk);

        // Probe new-project candidates whose debounce has expired.
        probe_new_projects(
            bg_tx,
            &mut pending_new,
            &mut discovered,
            ci_run_count,
            non_rust,
            client,
        );

        thread::sleep(POLL_INTERVAL);
    }
}

/// Immutable state needed to classify a filesystem event.
struct EventContext<'a> {
    scan_root:       &'a Path,
    projects:        &'a HashMap<PathBuf, ProjectEntry>,
    project_parents: &'a HashSet<PathBuf>,
    discovered:      &'a HashSet<PathBuf>,
}

fn handle_event(
    event_path: &Path,
    ctx: &EventContext<'_>,
    pending_disk: &mut HashMap<String, (Instant, Instant)>,
    pending_new: &mut HashMap<PathBuf, Instant>,
) {
    let now = Instant::now();

    // Try to match the event to a known project.
    if let Some((_, entry)) = ctx
        .projects
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

    // Not a known project — walk up from the event path to find the
    // directory at the same level as existing projects. A "project parent"
    // is any directory that already contains a known project (e.g. `~/rust/`).
    let Some(candidate) = project_level_dir(event_path, ctx.scan_root, ctx.project_parents) else {
        return;
    };
    // Always enqueue removals (dir gone); for creations, skip already-discovered.
    if !candidate.is_dir() || !ctx.discovered.contains(&candidate) {
        pending_new
            .entry(candidate)
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
                path: project_path.clone(),
                bytes,
            })
            .is_err()
        {
            return;
        }

        // Re-detect git sync status so the project list and git panel
        // reflect commits, pulls, branch switches, etc.
        if entry.git_tracking.is_tracked()
            && let Some(info) = GitInfo::detect(&entry.abs_path)
        {
            let _ = bg_tx.send(BackgroundMsg::GitInfo {
                path: project_path,
                info,
            });
        }
    }
}

fn probe_new_projects(
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    pending_new: &mut HashMap<PathBuf, Instant>,
    discovered: &mut HashSet<PathBuf>,
    ci_run_count: u32,
    non_rust: NonRustInclusion,
    client: &HttpClient,
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
            let task_client = client.clone();
            let path = project.path.clone();
            let name = project.name.clone();
            rayon::spawn(move || {
                scan::fetch_project_details(
                    &tx,
                    &task_client,
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

/// Walk up from `event_path` toward `scan_root`, returning the first
/// directory whose parent is a known project-parent directory or the scan
/// root itself. This finds the directory at the same nesting level as
/// existing projects regardless of how deep the scan root is.
///
/// When the walk-up doesn't find a known project parent, a filesystem
/// check for `Cargo.toml` or `.git` identifies project roots that
/// aren't yet registered (new projects added during or after the scan).
fn project_level_dir(
    event_path: &Path,
    scan_root: &Path,
    project_parents: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let mut path = event_path.to_path_buf();
    loop {
        let parent = path.parent()?;
        if parent == scan_root || project_parents.contains(parent) {
            // `path` is at the same level as known projects.
            return Some(path);
        }
        // Check for project markers on disk so we resolve to the actual
        // project root even when its parent isn't in `project_parents`.
        if path.join("Cargo.toml").exists() || path.join(".git").exists() {
            return Some(path);
        }
        if !path.starts_with(scan_root) {
            return None;
        }
        path = parent.to_path_buf();
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::process::Command;

    use super::*;

    // ── project_level_dir ────────────────────────────────────────────

    #[test]
    fn sibling_of_known_project() {
        // scan_root = /home/user, known project at /home/user/rust/bevy
        // → event inside /home/user/rust/bevy_style_fix/ should yield that dir
        let scan_root = Path::new("/home/user");
        let parents = HashSet::from([PathBuf::from("/home/user/rust")]);

        let event = Path::new("/home/user/rust/bevy_style_fix/src/main.rs");
        let result = project_level_dir(event, scan_root, &parents);
        assert_eq!(
            result.as_deref(),
            Some(Path::new("/home/user/rust/bevy_style_fix"))
        );
    }

    #[test]
    fn direct_child_of_scan_root() {
        // scan_root = /home/user/rust, no project_parents needed
        // → event inside /home/user/rust/new_project/ falls back to scan_root
        let scan_root = Path::new("/home/user/rust");
        let parents = HashSet::new();

        let event = Path::new("/home/user/rust/new_project/Cargo.toml");
        let result = project_level_dir(event, scan_root, &parents);
        assert_eq!(
            result.as_deref(),
            Some(Path::new("/home/user/rust/new_project"))
        );
    }

    #[test]
    fn event_is_the_new_directory_itself() {
        let scan_root = Path::new("/home/user");
        let parents = HashSet::from([PathBuf::from("/home/user/rust")]);

        let event = Path::new("/home/user/rust/new_wt");
        let result = project_level_dir(event, scan_root, &parents);
        assert_eq!(result.as_deref(), Some(Path::new("/home/user/rust/new_wt")));
    }

    #[test]
    fn deeply_nested_event_resolves_to_project_dir() {
        let scan_root = Path::new("/home/user");
        let parents = HashSet::from([PathBuf::from("/home/user/rust")]);

        let event = Path::new("/home/user/rust/cargo-port_wt/src/tui/render.rs");
        let result = project_level_dir(event, scan_root, &parents);
        assert_eq!(
            result.as_deref(),
            Some(Path::new("/home/user/rust/cargo-port_wt"))
        );
    }

    #[test]
    fn event_at_scan_root_returns_none() {
        let scan_root = Path::new("/home/user");
        let parents = HashSet::from([PathBuf::from("/home/user/rust")]);

        let result = project_level_dir(scan_root, scan_root, &parents);
        assert_eq!(result, None);
    }

    #[test]
    fn event_outside_scan_root_returns_none() {
        let scan_root = Path::new("/home/user/rust");
        let parents = HashSet::new();

        let event = Path::new("/tmp/other/file.rs");
        let result = project_level_dir(event, scan_root, &parents);
        assert_eq!(result, None);
    }

    #[test]
    fn multiple_parent_levels() {
        // Projects at different depths: ~/code/rust/foo and ~/code/python/bar
        let scan_root = Path::new("/home/user");
        let parents = HashSet::from([
            PathBuf::from("/home/user/code/rust"),
            PathBuf::from("/home/user/code/python"),
        ]);

        let rust_event = Path::new("/home/user/code/rust/new_crate/src/lib.rs");
        assert_eq!(
            project_level_dir(rust_event, scan_root, &parents).as_deref(),
            Some(Path::new("/home/user/code/rust/new_crate"))
        );

        let py_event = Path::new("/home/user/code/python/new_pkg/setup.py");
        assert_eq!(
            project_level_dir(py_event, scan_root, &parents).as_deref(),
            Some(Path::new("/home/user/code/python/new_pkg"))
        );
    }

    /// Synthetic paths with no filesystem markers fall back to the
    /// nearest `scan_root` or `project_parents` boundary.
    #[test]
    fn synthetic_paths_resolve_to_scan_root_child() {
        let scan_root = Path::new("/home/user");
        let parents = HashSet::new();

        let event = Path::new("/home/user/rust/bevy/src/lib.rs");
        let result = project_level_dir(event, scan_root, &parents);
        assert_eq!(result.as_deref(), Some(Path::new("/home/user/rust")));
    }

    /// Filesystem markers (`Cargo.toml`) are detected regardless of
    /// whether `project_parents` is empty or populated.
    #[test]
    fn filesystem_fallback_finds_cargo_toml() {
        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let scan_root = tmp.path();
        let project_dir = scan_root.join("rust").join("new_project");
        std::fs::create_dir_all(&project_dir).expect("create dirs");
        std::fs::write(project_dir.join("Cargo.toml"), b"[package]").expect("write Cargo.toml");

        let parents = HashSet::new();
        let event = project_dir.join("src/main.rs");
        let result = project_level_dir(&event, scan_root, &parents);
        assert_eq!(result, Some(project_dir));
    }

    /// A new project under a parent directory that isn't in
    /// `project_parents` is still found via `Cargo.toml` on disk.
    /// This covers: scan already passed `~/python/`, `project_parents`
    /// only has `~/rust/`, new project appears at `~/python/new_thing/`.
    #[test]
    fn new_project_in_unknown_parent_found_via_filesystem() {
        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let scan_root = tmp.path();

        // Existing project parent — only ~/rust/ is known
        let parents = HashSet::from([scan_root.join("rust")]);

        // New project under ~/python/ — not in project_parents
        let new_project = scan_root.join("python").join("new_thing");
        std::fs::create_dir_all(&new_project).expect("create dirs");
        std::fs::write(new_project.join("Cargo.toml"), b"[package]").expect("write Cargo.toml");

        let event = new_project.join("src/lib.rs");
        let result = project_level_dir(&event, scan_root, &parents);
        assert_eq!(result, Some(new_project));
    }

    // ── handle_event ─────────────────────────────────────────────────

    fn make_project_entry(project_path: &str, abs_path: &Path) -> (PathBuf, ProjectEntry) {
        (
            abs_path.to_path_buf(),
            ProjectEntry {
                project_path: project_path.to_string(),
                abs_path:     abs_path.to_path_buf(),
                git_tracking: GitTracking::Untracked,
            },
        )
    }

    #[test]
    fn known_project_event_goes_to_pending_disk() {
        let scan_root = PathBuf::from("/home/user");
        let mut projects = HashMap::new();
        let (key, entry) = make_project_entry("~/rust/bevy", Path::new("/home/user/rust/bevy"));
        projects.insert(key, entry);
        let project_parents = HashSet::from([PathBuf::from("/home/user/rust")]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let mut pending_disk = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            Path::new("/home/user/rust/bevy/src/lib.rs"),
            &ctx,
            &mut pending_disk,
            &mut pending_new,
        );

        assert!(pending_disk.contains_key("~/rust/bevy"));
        assert!(pending_new.is_empty());
    }

    #[test]
    fn unknown_sibling_event_goes_to_pending_new() {
        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let scan_root = tmp.path().to_path_buf();

        // Create the new project directory (handle_event checks is_dir)
        let new_project = scan_root.join("new_project");
        std::fs::create_dir_all(&new_project).expect("failed to create new_project dir");

        // Register an existing sibling so project_parents is populated
        let existing = scan_root.join("existing_project");
        let mut projects = HashMap::new();
        let (key, entry) = make_project_entry("~/existing_project", &existing);
        projects.insert(key, entry);
        let project_parents = HashSet::from([scan_root.clone()]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let mut pending_disk = HashMap::new();
        let mut pending_new = HashMap::new();

        let event_path = new_project.join("src/main.rs");
        handle_event(&event_path, &ctx, &mut pending_disk, &mut pending_new);

        assert!(pending_disk.is_empty());
        assert!(pending_new.contains_key(&new_project));
    }

    #[test]
    fn already_discovered_directory_not_re_enqueued() {
        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let scan_root = tmp.path().to_path_buf();

        let project_dir = scan_root.join("my_project");
        std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

        let projects = HashMap::new();
        let project_parents = HashSet::from([scan_root.clone()]);
        let discovered = HashSet::from([project_dir.clone()]);
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let mut pending_disk = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            &project_dir.join("Cargo.toml"),
            &ctx,
            &mut pending_disk,
            &mut pending_new,
        );

        assert!(pending_new.is_empty());
    }

    /// Simulates the full race: `scan_root` is two levels above projects,
    /// `project_parents` is empty (early scan), and a new project dir
    /// appears. The filesystem fallback finds `Cargo.toml` and
    /// `handle_event` enqueues the correct project directory.
    #[test]
    fn new_project_enqueued_during_early_scan() {
        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let scan_root = tmp.path().to_path_buf();

        // ~/rust/new_wt — two levels below scan root, no siblings registered
        let new_wt = scan_root.join("rust").join("new_wt");
        std::fs::create_dir_all(&new_wt).expect("create dirs");
        std::fs::write(new_wt.join("Cargo.toml"), b"[package]").expect("write Cargo.toml");

        let projects = HashMap::new();
        let project_parents = HashSet::new(); // empty — early scan
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let mut pending_disk = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            &new_wt.join("src/main.rs"),
            &ctx,
            &mut pending_disk,
            &mut pending_new,
        );

        // Must enqueue the project dir, not its grandparent
        assert!(
            pending_new.contains_key(&new_wt),
            "expected pending_new to contain {}, got: {:?}",
            new_wt.display(),
            pending_new.keys().collect::<Vec<_>>()
        );
    }

    // ── resolve_include_dirs ────────────────────────────────────────

    #[test]
    fn empty_include_dirs_falls_back_to_scan_root() {
        let dirs = scan::resolve_include_dirs(Path::new("/home/user"), &[]);
        assert_eq!(dirs, vec![PathBuf::from("/home/user")]);
    }

    #[test]
    fn relative_include_dirs_resolve_to_scan_root() {
        let dirs = scan::resolve_include_dirs(
            Path::new("/home/user"),
            &["rust".to_string(), ".claude".to_string()],
        );
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/home/user/rust"),
                PathBuf::from("/home/user/.claude"),
            ]
        );
    }

    #[test]
    fn absolute_include_dirs_used_as_is() {
        let dirs =
            scan::resolve_include_dirs(Path::new("/home/user"), &["/opt/projects".to_string()]);
        assert_eq!(dirs, vec![PathBuf::from("/opt/projects")]);
    }

    // ── fire_disk_updates ───────────────────────────────────────────

    /// Helper: create a git repo in `dir` with one commit so
    /// `GitInfo::detect` returns `Some`.
    fn init_git_repo(dir: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("git init");
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir)
            .output()
            .expect("git commit");
    }

    #[test]
    fn disk_update_sends_git_info_for_tracked_project() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("my_project");
        std::fs::create_dir_all(&project_dir).expect("create dir");
        init_git_repo(&project_dir);

        let (tx, rx) = mpsc::channel();
        let mut projects = HashMap::new();
        projects.insert(
            project_dir.clone(),
            ProjectEntry {
                project_path: "~/my_project".to_string(),
                abs_path:     project_dir,
                git_tracking: GitTracking::Tracked,
            },
        );

        // Deadline already expired → fires immediately.
        let past = Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .expect("1s subtraction should not underflow");
        let mut pending = HashMap::from([("~/my_project".to_string(), (past, past))]);

        fire_disk_updates(&tx, &projects, &mut pending);

        let mut got_disk = false;
        let mut got_git = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BackgroundMsg::DiskUsage { path, .. } if path == "~/my_project" => got_disk = true,
                BackgroundMsg::GitInfo { path, .. } if path == "~/my_project" => got_git = true,
                _ => {},
            }
        }
        assert!(got_disk, "expected DiskUsage message");
        assert!(got_git, "expected GitInfo message for tracked project");
        assert!(pending.is_empty(), "pending entry should be consumed");
    }

    #[test]
    fn disk_update_skips_git_info_for_untracked_project() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("no_git");
        std::fs::create_dir_all(&project_dir).expect("create dir");

        let (tx, rx) = mpsc::channel();
        let mut projects = HashMap::new();
        projects.insert(
            project_dir.clone(),
            ProjectEntry {
                project_path: "~/no_git".to_string(),
                abs_path:     project_dir,
                git_tracking: GitTracking::Untracked,
            },
        );

        let past = Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .expect("1s subtraction should not underflow");
        let mut pending = HashMap::from([("~/no_git".to_string(), (past, past))]);

        fire_disk_updates(&tx, &projects, &mut pending);

        let mut got_disk = false;
        let mut got_git = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BackgroundMsg::DiskUsage { path, .. } if path == "~/no_git" => got_disk = true,
                BackgroundMsg::GitInfo { .. } => got_git = true,
                _ => {},
            }
        }
        assert!(got_disk, "expected DiskUsage message");
        assert!(!got_git, "should not send GitInfo for untracked project");
    }
}
