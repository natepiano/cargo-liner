//! Watches the scan root recursively for filesystem changes and maps
//! events to discovered projects for disk-usage and git-sync updates.
//!
//! A single `notify` subscription covers the entire scan root. Events are
//! matched to projects by prefix, debounced, and result in both
//! `BackgroundMsg::DiskUsage` and `BackgroundMsg::GitInfo` updates. New project directories are
//! detected automatically; removed directories trigger a zero-byte update so the
//! app can mark them as deleted.
//!
//! On macOS (`FSEvents`) this is a small fixed set of kernel subscriptions
//! regardless of tree size: one for the scan roots. Linux / Windows may want
//! a different approach in the future to avoid inotify watch limits.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
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
use super::constants::WATCHER_DISK_CONCURRENCY;
use super::constants::WATCHER_GIT_CONCURRENCY;
use super::http::HttpClient;
use super::project;
use super::project::GitInfo;
use super::project::GitRepoPresence;
use super::scan;
use super::scan::BackgroundMsg;
use crate::project::RootItem;
use crate::project::RootItem::NonRust;

/// Request to register an already-known project with the watcher.
pub(crate) struct WatchRequest {
    /// Display path (e.g. `~/foo/bar`).
    pub project_path: String,
    /// Absolute filesystem path to the project root.
    pub abs_path:     PathBuf,
    /// Absolute path of the containing git repo root when known.
    pub repo_root:    Option<PathBuf>,
}

/// Spawn a unified background watcher thread. Watches the include
/// directories recursively and handles disk-usage updates,
/// new-project detection, and deleted-project detection.
pub(crate) fn spawn_watcher(
    scan_root: PathBuf,
    bg_tx: mpsc::Sender<BackgroundMsg>,
    ci_run_count: u32,
    non_rust: NonRustInclusion,
    include_dirs: Vec<String>,
    client: HttpClient,
) -> mpsc::Sender<WatchRequest> {
    let (watch_tx, watch_rx) = mpsc::channel();
    let ctx = WatcherLoopContext {
        scan_root,
        bg_tx,
        ci_run_count,
        non_rust,
        include_dirs,
        client,
    };

    thread::spawn(move || {
        watcher_loop(&ctx, &watch_rx);
    });

    watch_tx
}

struct WatcherLoopContext {
    scan_root:    PathBuf,
    bg_tx:        mpsc::Sender<BackgroundMsg>,
    ci_run_count: u32,
    non_rust:     NonRustInclusion,
    include_dirs: Vec<String>,
    client:       HttpClient,
}

/// Per-project tracking state.
struct ProjectEntry {
    project_path: String,
    abs_path:     PathBuf,
    repo_root:    Option<PathBuf>,
    /// The resolved on-disk git directory. For normal repos this is
    /// `repo_root/.git`; for worktrees it follows the `.git` file to the
    /// real directory (e.g. `<main-repo>/.git/worktrees/<name>`).
    git_dir:      Option<PathBuf>,
}

enum DiskState {
    Pending {
        debounce_deadline: Instant,
        max_deadline:      Instant,
    },
    Running {
        dirty_since_start: bool,
    },
}

enum GitState {
    Pending {
        debounce_deadline: Instant,
        max_deadline:      Instant,
        refresh_info:      bool,
    },
    Running {
        dirty_since_start: bool,
        refresh_info:      bool,
    },
}

#[derive(Clone, Copy)]
enum GitRefreshKind {
    PathStateOnly,
    FullMetadata,
}

impl GitRefreshKind {
    const fn refresh_info(self) -> bool { matches!(self, Self::FullMetadata) }
}

fn watcher_loop(ctx: &WatcherLoopContext, watch_rx: &mpsc::Receiver<WatchRequest>) {
    let watch_dirs = scan::resolve_include_dirs(&ctx.scan_root, &ctx.include_dirs);
    let (notify_tx, notify_rx) = mpsc::channel();
    let handler = move |res| {
        let _ = notify_tx.send(res);
    };
    let Ok(mut watcher) = notify::recommended_watcher(handler) else {
        return;
    };
    register_watch_roots(&mut watcher, &watch_dirs);

    // `abs_path` → project tracking state
    let mut projects: HashMap<PathBuf, ProjectEntry> = HashMap::new();
    // Directories that contain at least one known project (e.g. `~/rust/`).
    let mut project_parents: HashSet<PathBuf> = HashSet::new();
    // project_path → disk refresh state
    let mut pending_disk: HashMap<String, DiskState> = HashMap::new();
    // repo_root → git refresh state
    let mut pending_git: HashMap<PathBuf, GitState> = HashMap::new();
    // Directories that might be new projects → probe deadline
    let mut pending_new: HashMap<PathBuf, Instant> = HashMap::new();
    // Directories already discovered as new projects by this watcher.
    let mut discovered: HashSet<PathBuf> = HashSet::new();
    let mut watched_git_metadata: HashSet<PathBuf> = HashSet::new();
    let (disk_done_tx, disk_done_rx) = mpsc::channel::<String>();
    let (git_done_tx, git_done_rx) = mpsc::channel::<PathBuf>();
    let disk_limit = Arc::new(tokio::sync::Semaphore::new(WATCHER_DISK_CONCURRENCY));
    let git_limit = Arc::new(tokio::sync::Semaphore::new(WATCHER_GIT_CONCURRENCY));

    loop {
        if drain_watch_requests(
            &mut watcher,
            watch_rx,
            &mut projects,
            &mut project_parents,
            &mut watched_git_metadata,
        ) {
            return;
        }

        let dispatch = WatcherDispatchContext {
            event: EventContext {
                scan_root:       &ctx.scan_root,
                projects:        &projects,
                project_parents: &project_parents,
                discovered:      &discovered,
            },
            bg_tx: &ctx.bg_tx,
        };
        drain_notify_events(
            &notify_rx,
            &dispatch,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );
        drain_completed_refreshes(
            &disk_done_rx,
            &git_done_rx,
            &mut pending_disk,
            &mut pending_git,
        );

        // Fire git refreshes whose debounce has expired.
        fire_git_updates(
            &ctx.client.handle,
            &git_limit,
            &git_done_tx,
            &ctx.bg_tx,
            &projects,
            &mut pending_git,
        );

        // Fire disk recalculations whose debounce has expired.
        fire_disk_updates(
            &ctx.client.handle,
            &disk_limit,
            &disk_done_tx,
            &ctx.bg_tx,
            &projects,
            &mut pending_disk,
        );

        // Probe new-project candidates whose debounce has expired.
        probe_new_projects(
            &ctx.bg_tx,
            &mut pending_new,
            &mut discovered,
            ctx.ci_run_count,
            ctx.non_rust,
            &ctx.client,
        );

        thread::sleep(POLL_INTERVAL);
    }
}

fn register_watch_roots(watcher: &mut impl Watcher, watch_dirs: &[PathBuf]) {
    for dir in watch_dirs {
        if dir.is_dir() {
            let _ = watcher.watch(dir, RecursiveMode::Recursive);
        }
    }
}

fn drain_watch_requests(
    watcher: &mut impl Watcher,
    watch_rx: &mpsc::Receiver<WatchRequest>,
    projects: &mut HashMap<PathBuf, ProjectEntry>,
    project_parents: &mut HashSet<PathBuf>,
    watched_git_metadata: &mut HashSet<PathBuf>,
) -> bool {
    loop {
        match watch_rx.try_recv() {
            Ok(req) => {
                if let Some(parent) = req.abs_path.parent() {
                    project_parents.insert(parent.to_path_buf());
                }
                let git_dir = req.repo_root.as_deref().and_then(project::resolve_git_dir);
                watch_git_metadata_paths(watcher, &req, git_dir.as_deref(), watched_git_metadata);
                projects.insert(
                    req.abs_path.clone(),
                    ProjectEntry {
                        project_path: req.project_path,
                        abs_path: req.abs_path.clone(),
                        repo_root: req.repo_root,
                        git_dir,
                    },
                );
            },
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => return true,
        }
    }
}

fn drain_notify_events(
    notify_rx: &mpsc::Receiver<notify::Result<notify::Event>>,
    ctx: &WatcherDispatchContext<'_>,
    pending_disk: &mut HashMap<String, DiskState>,
    pending_git: &mut HashMap<PathBuf, GitState>,
    pending_new: &mut HashMap<PathBuf, Instant>,
) {
    while let Ok(result) = notify_rx.try_recv() {
        let Ok(event) = result else {
            continue;
        };
        for event_path in &event.paths {
            handle_event(
                event_path,
                &ctx.event,
                ctx.bg_tx,
                pending_disk,
                pending_git,
                pending_new,
            );
        }
    }
}

fn drain_completed_refreshes(
    disk_done_rx: &mpsc::Receiver<String>,
    git_done_rx: &mpsc::Receiver<PathBuf>,
    pending_disk: &mut HashMap<String, DiskState>,
    pending_git: &mut HashMap<PathBuf, GitState>,
) {
    while let Ok(project_path) = disk_done_rx.try_recv() {
        handle_disk_completion(pending_disk, &project_path);
    }

    while let Ok(repo_root) = git_done_rx.try_recv() {
        handle_git_completion(pending_git, repo_root);
    }
}

fn watch_git_metadata_paths(
    watcher: &mut impl Watcher,
    req: &WatchRequest,
    git_dir: Option<&Path>,
    watched_git_metadata: &mut HashSet<PathBuf>,
) {
    let started = Instant::now();
    let Some(repo_root) = req.repo_root.as_deref() else {
        return;
    };

    let metadata_paths = git_metadata_watch_paths(repo_root, git_dir);
    let mut added = 0;
    for path in metadata_paths {
        if watched_git_metadata.insert(path.clone()) {
            let _ = watcher.watch(&path, RecursiveMode::NonRecursive);
            added += 1;
        }
    }
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        repo_root = %repo_root.display(),
        request_path = %req.project_path,
        added,
        "watcher_watch_git_metadata"
    );
}

fn git_metadata_watch_paths(repo_root: &Path, git_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = vec![repo_root.join(".gitignore")];
    if let Some(git_dir) = git_dir {
        paths.push(git_dir.join("HEAD"));
        paths.push(git_dir.join("index"));
        let info = git_dir.join("info");
        paths.push(info.join("exclude"));
        paths.push(info);
    }
    paths
}

/// Immutable state needed to classify a filesystem event.
struct EventContext<'a> {
    scan_root:       &'a Path,
    projects:        &'a HashMap<PathBuf, ProjectEntry>,
    project_parents: &'a HashSet<PathBuf>,
    discovered:      &'a HashSet<PathBuf>,
}

struct WatcherDispatchContext<'a> {
    event: EventContext<'a>,
    bg_tx: &'a mpsc::Sender<BackgroundMsg>,
}

fn handle_event(
    event_path: &Path,
    ctx: &EventContext<'_>,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    pending_disk: &mut HashMap<String, DiskState>,
    pending_git: &mut HashMap<PathBuf, GitState>,
    pending_new: &mut HashMap<PathBuf, Instant>,
) {
    let now = Instant::now();

    if let Some((entry, refresh_kind)) = ctx.projects.values().find_map(|entry| {
        classify_fast_git_event(event_path, entry).map(|refresh_kind| (entry, refresh_kind))
    }) {
        if let Some(repo_root) = &entry.repo_root {
            tracing::info!(
                repo_root = %repo_root.display(),
                event_path = %event_path.display(),
                refresh_info = %refresh_kind.refresh_info(),
                "watcher_fast_git_metadata_event"
            );
            emit_root_git_path_refresh(bg_tx, ctx.projects, repo_root);
            enqueue_git_refresh(
                pending_git,
                repo_root.clone(),
                now,
                false,
                refresh_kind.refresh_info(),
                if refresh_kind.refresh_info() {
                    "fast_git_metadata"
                } else {
                    "fast_git_path_state"
                },
            );
        }
        return;
    }

    // Try to match the event to a known project.
    if let Some((_, entry)) = ctx
        .projects
        .iter()
        .find(|(root, _)| event_path.starts_with(root))
    {
        if is_target_metadata_event(event_path, entry.abs_path.as_path()) {
            spawn_project_refresh(bg_tx.clone(), entry.abs_path.clone());
        }
        if is_internal_git_path(event_path, entry) {
            if let Some(repo_root) = &entry.repo_root
                && let Some(refresh_kind) = classify_internal_git_event(event_path, entry)
            {
                enqueue_git_refresh(
                    pending_git,
                    repo_root.clone(),
                    now,
                    false,
                    refresh_kind.refresh_info(),
                    if refresh_kind.refresh_info() {
                        "git_internal"
                    } else {
                        "git_internal_path_state"
                    },
                );
            }
            return;
        }
        let is_target_event = event_path.starts_with(entry.abs_path.join("target"));
        schedule_disk_refresh(pending_disk, &entry.project_path, now);
        if !is_target_event && let Some(repo_root) = &entry.repo_root {
            enqueue_git_refresh(
                pending_git,
                repo_root.clone(),
                now,
                false,
                classify_internal_git_event(event_path, entry)
                    .is_some_and(GitRefreshKind::refresh_info),
                "project_event",
            );
        }
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

fn schedule_disk_refresh(
    pending_disk: &mut HashMap<String, DiskState>,
    project_path: &str,
    now: Instant,
) {
    match pending_disk.get_mut(project_path) {
        Some(DiskState::Pending {
            debounce_deadline, ..
        }) => {
            *debounce_deadline = now + DEBOUNCE_DURATION;
        },
        Some(DiskState::Running { dirty_since_start }) => {
            *dirty_since_start = true;
        },
        None => {
            pending_disk.insert(
                project_path.to_string(),
                DiskState::Pending {
                    debounce_deadline: now + DEBOUNCE_DURATION,
                    max_deadline:      now + MAX_WAIT,
                },
            );
        },
    }
}

fn handle_disk_completion(pending_disk: &mut HashMap<String, DiskState>, project_path: &str) {
    let now = Instant::now();
    let Some(state) = pending_disk.remove(project_path) else {
        return;
    };
    if let DiskState::Running { dirty_since_start } = state
        && dirty_since_start
    {
        pending_disk.insert(
            project_path.to_string(),
            DiskState::Pending {
                debounce_deadline: now + DEBOUNCE_DURATION,
                max_deadline:      now + MAX_WAIT,
            },
        );
    }
}

fn handle_git_completion(pending_git: &mut HashMap<PathBuf, GitState>, repo_root: PathBuf) {
    let now = Instant::now();
    let Some(state) = pending_git.remove(&repo_root) else {
        return;
    };
    if let GitState::Running {
        dirty_since_start,
        refresh_info,
    } = state
        && dirty_since_start
    {
        pending_git.insert(
            repo_root,
            GitState::Pending {
                debounce_deadline: now + DEBOUNCE_DURATION,
                max_deadline: now + MAX_WAIT,
                refresh_info,
            },
        );
    }
}

fn classify_fast_git_event(event_path: &Path, entry: &ProjectEntry) -> Option<GitRefreshKind> {
    let repo_root = entry.repo_root.as_deref()?;
    let git_dir = entry.git_dir.as_deref()?;
    if event_path == repo_root.join(".gitignore")
        || event_path == git_dir.join("index")
        || event_path == git_dir.join("info").join("exclude")
    {
        Some(GitRefreshKind::PathStateOnly)
    } else if event_path == git_dir.join("HEAD") {
        Some(GitRefreshKind::FullMetadata)
    } else {
        None
    }
}

fn is_internal_git_path(event_path: &Path, entry: &ProjectEntry) -> bool {
    let repo_root = entry.repo_root.as_deref();
    let git_dir = entry.git_dir.as_deref();
    // Match events under the resolved git dir (handles worktrees) or
    // under repo_root/.git (handles normal repos where git_dir ==
    // repo_root/.git, but also catches events like refs/heads updates
    // that live in the common git dir rather than the worktree git dir).
    git_dir.is_some_and(|d| event_path.starts_with(d))
        || repo_root.is_some_and(|r| event_path.starts_with(r.join(".git")))
}

fn classify_internal_git_event(event_path: &Path, entry: &ProjectEntry) -> Option<GitRefreshKind> {
    let git_dir = entry.git_dir.as_deref()?;
    let repo_root = entry.repo_root.as_deref()?;
    if event_path == repo_root.join(".gitignore")
        || event_path == git_dir.join("index")
        || event_path == git_dir.join("index.lock")
        || event_path == git_dir.join("info").join("exclude")
    {
        Some(GitRefreshKind::PathStateOnly)
    } else if event_path == git_dir.join("HEAD")
        || event_path == git_dir.join("FETCH_HEAD")
        || event_path == git_dir.join("ORIG_HEAD")
        || event_path == git_dir.join("config")
        || event_path == git_dir.join("packed-refs")
        || event_path.starts_with(git_dir.join("refs").join("heads"))
        || event_path.starts_with(git_dir.join("refs").join("remotes"))
    {
        Some(GitRefreshKind::FullMetadata)
    } else {
        None
    }
}

fn is_target_metadata_event(event_path: &Path, project_root: &Path) -> bool {
    let cargo_toml = project_root.join("Cargo.toml");
    let build_rs = project_root.join("build.rs");
    let src_main = project_root.join("src").join("main.rs");
    let src_bin = project_root.join("src").join("bin");
    let examples = project_root.join("examples");
    let benches = project_root.join("benches");
    let tests = project_root.join("tests");

    event_path == cargo_toml
        || event_path == build_rs
        || event_path == src_main
        || event_path.starts_with(src_bin)
        || event_path.starts_with(examples)
        || event_path.starts_with(benches)
        || event_path.starts_with(tests)
}

fn spawn_project_refresh(bg_tx: mpsc::Sender<BackgroundMsg>, project_root: PathBuf) {
    rayon::spawn(move || {
        let cargo_toml = project_root.join("Cargo.toml");
        let Ok(cargo_project) = crate::project::from_cargo_toml(&cargo_toml) else {
            return;
        };
        let item = scan::cargo_project_to_item(cargo_project);
        let _ = bg_tx.send(BackgroundMsg::ProjectRefreshed { item });
    });
}

fn enqueue_git_refresh(
    pending_git: &mut HashMap<PathBuf, GitState>,
    repo_root: PathBuf,
    now: Instant,
    immediate: bool,
    refresh_info: bool,
    cause: &str,
) {
    let pending_count = pending_git
        .iter()
        .filter(|(path, _)| path.as_path() != repo_root.as_path())
        .filter(|(_, state)| matches!(state, GitState::Pending { .. }))
        .count()
        + usize::from(!matches!(
            pending_git.get(&repo_root),
            Some(GitState::Pending { .. })
        ));
    tracing::info!(
        repo_root = %repo_root.display(),
        immediate,
        refresh_info,
        cause,
        pending_git = pending_count,
        "watcher_enqueue_git_refresh"
    );
    match pending_git.get_mut(&repo_root) {
        Some(GitState::Pending {
            debounce_deadline,
            refresh_info: pending_refresh_info,
            ..
        }) => {
            *debounce_deadline = if immediate {
                now
            } else {
                now + DEBOUNCE_DURATION
            };
            *pending_refresh_info |= refresh_info;
        },
        Some(GitState::Running {
            dirty_since_start,
            refresh_info: pending_refresh_info,
        }) => {
            *dirty_since_start = true;
            *pending_refresh_info |= refresh_info;
        },
        None => {
            pending_git.insert(
                repo_root,
                GitState::Pending {
                    debounce_deadline: if immediate {
                        now
                    } else {
                        now + DEBOUNCE_DURATION
                    },
                    max_deadline: now + MAX_WAIT,
                    refresh_info,
                },
            );
        },
    }
}

fn emit_root_git_path_refresh(
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    projects: &HashMap<PathBuf, ProjectEntry>,
    repo_root: &Path,
) {
    let started = Instant::now();
    let Some(root_entry) = projects
        .values()
        .find(|entry| entry.abs_path.as_path() == repo_root)
    else {
        return;
    };
    let state = project::detect_git_path_state(repo_root);
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        repo_root = %repo_root.display(),
        path = %root_entry.project_path,
        state = %state.label(),
        "watcher_root_git_path_refresh"
    );
    let _ = bg_tx.send(BackgroundMsg::GitPathState {
        path: root_entry.abs_path.clone().into(),
        state,
    });
}

fn fire_git_updates(
    handle: &tokio::runtime::Handle,
    git_limit: &Arc<tokio::sync::Semaphore>,
    git_done_tx: &mpsc::Sender<PathBuf>,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    projects: &HashMap<PathBuf, ProjectEntry>,
    pending_git: &mut HashMap<PathBuf, GitState>,
) {
    let now = Instant::now();
    let ready: Vec<(PathBuf, bool)> = pending_git
        .iter()
        .filter_map(|(repo_root, state)| match state {
            GitState::Pending {
                debounce_deadline,
                max_deadline,
                refresh_info,
            } if now >= *debounce_deadline || now >= *max_deadline => {
                Some((repo_root.clone(), *refresh_info))
            },
            GitState::Pending { .. } | GitState::Running { .. } => None,
        })
        .collect();

    for (repo_root, refresh_info) in ready {
        let affected: Vec<(String, String)> = projects
            .values()
            .filter(|entry| entry.repo_root.as_deref() == Some(repo_root.as_path()))
            .map(|entry| {
                let abs = entry.abs_path.to_string_lossy().to_string();
                (abs.clone(), abs)
            })
            .collect();
        if affected.is_empty() {
            pending_git.remove(&repo_root);
            continue;
        }
        pending_git.insert(
            repo_root.clone(),
            GitState::Running {
                dirty_since_start: false,
                refresh_info:      false,
            },
        );
        spawn_git_refresh(
            handle,
            git_limit,
            git_done_tx.clone(),
            bg_tx.clone(),
            repo_root,
            affected,
            refresh_info,
        );
    }
}

fn spawn_git_refresh(
    handle: &tokio::runtime::Handle,
    git_limit: &Arc<tokio::sync::Semaphore>,
    git_done_tx: mpsc::Sender<PathBuf>,
    bg_tx: mpsc::Sender<BackgroundMsg>,
    repo_root: PathBuf,
    affected: Vec<(String, String)>,
    refresh_info: bool,
) {
    let handle = handle.clone();
    let git_limit = Arc::clone(git_limit);
    handle.spawn(async move {
        let queue_started = Instant::now();
        let Ok(_permit) = git_limit.acquire_owned().await else {
            return;
        };
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(queue_started.elapsed().as_millis()),
            repo_root = %repo_root.display(),
            affected_rows = affected.len(),
            "watcher_git_queue_wait"
        );

        let started = Instant::now();
        let git_info_elapsed_ms = if refresh_info {
            let repo_root_for_git_info = repo_root.clone();
            let git_info_started = Instant::now();
            let git_info =
                tokio::task::spawn_blocking(move || GitInfo::detect_fast(&repo_root_for_git_info))
                    .await
                    .ok()
                    .flatten();
            let git_info_elapsed_ms = git_info_started.elapsed().as_millis();
            tracing::info!(
                elapsed_ms = crate::perf_log::ms(git_info_started.elapsed().as_millis()),
                repo_root = %repo_root.display(),
                affected_rows = affected.len(),
                refresh_info,
                "watcher_git_info_detect"
            );
            if let Some(info) = git_info {
                for (path, _) in &affected {
                    let _ = bg_tx.send(BackgroundMsg::GitInfo {
                        path: PathBuf::from(path).into(),
                        info: info.clone(),
                    });
                }
            }
            git_info_elapsed_ms
        } else {
            0
        };

        let git_projects = affected.clone();
        let state_started = Instant::now();
        let git_path_states = tokio::task::spawn_blocking(move || {
            project::detect_git_path_states_batch(&git_projects)
        })
        .await
        .ok();
        let git_path_states_elapsed_ms = state_started.elapsed().as_millis();
        if let Some(git_path_states) = git_path_states {
            for (path, state) in git_path_states {
                let _ = bg_tx.send(BackgroundMsg::GitPathState {
                    path: PathBuf::from(path).into(),
                    state,
                });
            }
        }
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            repo_root = %repo_root.display(),
            affected_rows = affected.len(),
            refresh_info,
            git_info_ms = git_info_elapsed_ms,
            git_path_states_ms = git_path_states_elapsed_ms,
            "watcher_git_refresh"
        );
        let _ = git_done_tx.send(repo_root);
    });
}

fn fire_disk_updates(
    handle: &tokio::runtime::Handle,
    disk_limit: &Arc<tokio::sync::Semaphore>,
    disk_done_tx: &mpsc::Sender<String>,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    projects: &HashMap<PathBuf, ProjectEntry>,
    pending_disk: &mut HashMap<String, DiskState>,
) {
    let now = Instant::now();
    let ready: Vec<String> = pending_disk
        .iter()
        .filter_map(|(key, state)| match state {
            DiskState::Pending {
                debounce_deadline,
                max_deadline,
            } if now >= *debounce_deadline || now >= *max_deadline => Some(key.clone()),
            DiskState::Pending { .. } | DiskState::Running { .. } => None,
        })
        .collect();

    for project_path in ready {
        let Some(state) = pending_disk.get_mut(&project_path) else {
            continue;
        };
        *state = DiskState::Running {
            dirty_since_start: false,
        };
        let Some(entry) = projects.values().find(|e| e.project_path == project_path) else {
            continue;
        };
        spawn_disk_update(
            handle,
            disk_limit,
            disk_done_tx.clone(),
            bg_tx.clone(),
            project_path.clone(),
            entry.abs_path.clone(),
        );
    }
}

fn spawn_disk_update(
    handle: &tokio::runtime::Handle,
    disk_limit: &Arc<tokio::sync::Semaphore>,
    disk_done_tx: mpsc::Sender<String>,
    bg_tx: mpsc::Sender<BackgroundMsg>,
    project_path: String,
    abs_path: PathBuf,
) {
    let handle = handle.clone();
    let disk_limit = Arc::clone(disk_limit);
    handle.spawn(async move {
        let queue_started = Instant::now();
        let Ok(_permit) = disk_limit.acquire_owned().await else {
            return;
        };
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(queue_started.elapsed().as_millis()),
            path = %project_path,
            abs_path = %abs_path.display(),
            "watcher_disk_queue_wait"
        );

        let started = Instant::now();
        let abs_for_msg = abs_path.clone().into();
        let bytes = tokio::task::spawn_blocking(move || scan::dir_size(&abs_path))
            .await
            .ok()
            .unwrap_or(0);
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            path = %project_path,
            bytes,
            "watcher_disk_usage"
        );
        let _ = bg_tx.send(BackgroundMsg::DiskUsage {
            path: abs_for_msg,
            bytes,
        });
        let _ = disk_done_tx.send(project_path);
    });
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
            let _ = bg_tx.send(BackgroundMsg::DiskUsage {
                path:  dir.into(),
                bytes: 0,
            });
            continue;
        }

        if discovered.contains(&dir) {
            continue;
        }
        if let Some(item) = probe_project(&dir, non_rust) {
            discovered.insert(dir.clone());
            let abs_path = item.path().to_path_buf();
            let display_path = item.display_path();
            let project_name = item.name().map(str::to_string);
            let repo_presence = if project::git_repo_root(&abs_path).is_some() {
                GitRepoPresence::InRepo
            } else {
                GitRepoPresence::OutsideRepo
            };
            let _ = bg_tx.send(BackgroundMsg::ProjectDiscovered { item });
            let tx = bg_tx.clone();
            let task_ctx = scan::FetchContext {
                client:     client.clone(),
                repo_cache: scan::new_repo_cache(),
            };
            rayon::spawn(move || {
                let request = scan::ProjectDetailRequest {
                    tx: &tx,
                    ctx: &task_ctx,
                    _project_path: &display_path,
                    abs_path: &abs_path,
                    project_name: project_name.as_deref(),
                    repo_presence,
                    ci_run_count,
                };
                scan::fetch_project_details(&request);
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
fn probe_project(dir: &Path, non_rust: NonRustInclusion) -> Option<RootItem> {
    let cargo_toml = dir.join("Cargo.toml");
    if cargo_toml.exists() {
        return crate::project::from_cargo_toml(&cargo_toml)
            .ok()
            .map(scan::cargo_project_to_item);
    }
    if non_rust.includes_non_rust() && dir.join(".git").is_dir() {
        return Some(NonRust(crate::project::from_git_dir(dir)));
    }
    None
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::OnceLock;
    use std::time::Duration;

    use super::*;
    use crate::lint;

    fn test_runtime() -> &'static tokio::runtime::Runtime {
        static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        TEST_RT.get_or_init(|| {
            tokio::runtime::Runtime::new().unwrap_or_else(|_| std::process::abort())
        })
    }

    fn wait_for_completion<T>(rx: &mpsc::Receiver<T>) {
        rx.recv_timeout(Duration::from_secs(1))
            .unwrap_or_else(|_| panic!("timed out waiting for background completion"));
    }

    fn collect_messages_until(
        rx: &mpsc::Receiver<BackgroundMsg>,
        predicate: impl Fn(&BackgroundMsg) -> bool,
    ) -> Vec<BackgroundMsg> {
        let first = rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or_else(|_| panic!("timed out waiting for background message"));
        let mut messages = vec![first];
        while !messages.iter().any(&predicate) {
            let next = rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap_or_else(|_| panic!("timed out waiting for expected background message"));
            messages.push(next);
        }
        messages
    }

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
                repo_root:    None,
                git_dir:      None,
            },
        )
    }

    fn assert_pending_disk(states: &HashMap<String, DiskState>, project_path: &str) {
        assert!(matches!(
            states.get(project_path),
            Some(DiskState::Pending { .. })
        ));
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
        let (bg_tx, _bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            Path::new("/home/user/rust/bevy/src/lib.rs"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert_pending_disk(&pending_disk, "~/rust/bevy");
        assert!(pending_git.is_empty());
        assert!(pending_new.is_empty());
    }

    #[test]
    fn target_event_refreshes_project_metadata() {
        let project_root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(project_root.path().join("examples")).expect("create examples dir");
        std::fs::write(
            project_root.path().join("Cargo.toml"),
            r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024"
"#,
        )
        .expect("write Cargo.toml");
        std::fs::write(
            project_root.path().join("examples").join("new_target.rs"),
            "fn main() {}\n",
        )
        .expect("write example");

        let mut projects = HashMap::new();
        let (key, entry) = make_project_entry("~/rust/demo", project_root.path());
        projects.insert(key, entry);
        let scan_root = project_root.path().to_path_buf();
        let project_parents = HashSet::new();
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            &project_root.path().join("examples").join("new_target.rs"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );
        let BackgroundMsg::ProjectRefreshed { item: refreshed } = bg_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("project refresh message")
        else {
            panic!("unexpected background message");
        };
        assert_eq!(refreshed.path(), project_root.path());
        // Verify examples were parsed from the refreshed Cargo.toml
        let example_count = match &refreshed {
            crate::project::RootItem::Package(pkg) => pkg
                .cargo()
                .examples()
                .iter()
                .map(|g| g.names.len())
                .sum::<usize>(),
            crate::project::RootItem::Workspace(ws) => ws
                .cargo()
                .examples()
                .iter()
                .map(|g| g.names.len())
                .sum::<usize>(),
            _ => 0,
        };
        assert_eq!(example_count, 1);
        assert_pending_disk(&pending_disk, "~/rust/demo");
        assert!(pending_git.is_empty());
        assert!(pending_new.is_empty());
    }

    #[test]
    fn git_exclude_event_refreshes_git_immediately() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("my_project");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        init_git_repo(&project_dir);
        let member_dir = project_dir.join("crates").join("member");
        std::fs::create_dir_all(&member_dir).expect("create member dir");

        let mut projects = HashMap::new();
        projects.insert(
            project_dir.clone(),
            ProjectEntry {
                project_path: "~/my_project".to_string(),
                abs_path:     project_dir.clone(),
                repo_root:    Some(project_dir.clone()),
                git_dir:      Some(project_dir.join(".git")),
            },
        );
        let member_key = member_dir.clone();
        projects.insert(
            member_key,
            ProjectEntry {
                project_path: "~/my_project/crates/member".to_string(),
                abs_path:     member_dir.clone(),
                repo_root:    Some(project_dir.clone()),
                git_dir:      Some(project_dir.join(".git")),
            },
        );
        let scan_root = tmp.path().to_path_buf();
        let project_parents = HashSet::from([scan_root.clone()]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();
        let (git_done_tx, _git_done_rx) = mpsc::channel();

        handle_event(
            &project_dir.join(".git").join("info").join("exclude"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        let git_limit = Arc::new(tokio::sync::Semaphore::new(1));
        fire_git_updates(
            test_runtime().handle(),
            &git_limit,
            &git_done_tx,
            &bg_tx,
            &projects,
            &mut pending_git,
        );
        let messages = collect_messages_until(
            &bg_rx,
            |msg| matches!(msg, BackgroundMsg::GitPathState { path, .. } if path.as_path() == project_dir),
        );

        let mut got_git_info = false;
        let mut got_root_git_state = false;
        let mut got_member_git_state = false;
        for msg in messages {
            match msg {
                BackgroundMsg::GitInfo { .. } => got_git_info = true,
                BackgroundMsg::GitPathState { path, .. } if path.as_path() == project_dir => {
                    got_root_git_state = true;
                },
                BackgroundMsg::GitPathState { path, .. } if path.as_path() == member_dir => {
                    got_member_git_state = true;
                },
                _ => {},
            }
        }

        assert!(
            !got_git_info,
            "repo-wide GitInfo should not block the fast path"
        );
        assert!(
            got_root_git_state,
            "expected immediate root GitPathState refresh"
        );
        assert!(
            !got_member_git_state,
            "member rows should wait for the background repo refresh"
        );
        assert!(
            pending_disk.is_empty(),
            "exclude edits should bypass disk queue"
        );
        assert!(
            pending_git.contains_key(&project_dir),
            "full repo refresh should stay queued for children"
        );
        assert!(pending_new.is_empty());
    }

    #[test]
    fn git_internal_noise_is_ignored() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("my_project");
        std::fs::create_dir_all(project_dir.join(".git").join("objects")).expect("create git dir");

        let mut projects = HashMap::new();
        projects.insert(
            project_dir.clone(),
            ProjectEntry {
                project_path: "~/my_project".to_string(),
                abs_path:     project_dir.clone(),
                repo_root:    Some(project_dir.clone()),
                git_dir:      Some(project_dir.join(".git")),
            },
        );
        let scan_root = tmp.path().to_path_buf();
        let project_parents = HashSet::from([scan_root.clone()]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, _bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            &project_dir.join(".git").join("objects").join("pack.tmp"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(pending_disk.is_empty());
        assert!(pending_git.is_empty());
        assert!(pending_new.is_empty());
    }

    #[test]
    fn git_index_event_refreshes_git_path_immediately() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("my_project");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        init_git_repo(&project_dir);
        let member_dir = project_dir.join("crates").join("member");
        std::fs::create_dir_all(&member_dir).expect("create member dir");

        let mut projects = HashMap::new();
        projects.insert(
            project_dir.clone(),
            ProjectEntry {
                project_path: "~/my_project".to_string(),
                abs_path:     project_dir.clone(),
                repo_root:    Some(project_dir.clone()),
                git_dir:      Some(project_dir.join(".git")),
            },
        );
        let member_key = member_dir.clone();
        projects.insert(
            member_key,
            ProjectEntry {
                project_path: "~/my_project/crates/member".to_string(),
                abs_path:     member_dir.clone(),
                repo_root:    Some(project_dir.clone()),
                git_dir:      Some(project_dir.join(".git")),
            },
        );
        let scan_root = tmp.path().to_path_buf();
        let project_parents = HashSet::from([scan_root.clone()]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();
        let (git_done_tx, _git_done_rx) = mpsc::channel();

        handle_event(
            &project_dir.join(".git").join("index"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        let git_limit = Arc::new(tokio::sync::Semaphore::new(1));
        fire_git_updates(
            test_runtime().handle(),
            &git_limit,
            &git_done_tx,
            &bg_tx,
            &projects,
            &mut pending_git,
        );
        let messages = collect_messages_until(
            &bg_rx,
            |msg| matches!(msg, BackgroundMsg::GitPathState { path, .. } if path.as_path() == project_dir),
        );

        let mut got_git_info = false;
        let mut got_root_git_state = false;
        let mut got_member_git_state = false;
        for msg in messages {
            match msg {
                BackgroundMsg::GitInfo { .. } => got_git_info = true,
                BackgroundMsg::GitPathState { path, .. } if path.as_path() == project_dir => {
                    got_root_git_state = true;
                },
                BackgroundMsg::GitPathState { path, .. } if path.as_path() == member_dir => {
                    got_member_git_state = true;
                },
                _ => {},
            }
        }

        assert!(
            !got_git_info,
            "index writes should refresh path state without a full GitInfo refresh"
        );
        assert!(
            got_root_git_state,
            "expected immediate root GitPathState refresh"
        );
        assert!(
            !got_member_git_state,
            "member rows should wait for the background repo refresh"
        );
        assert!(pending_disk.is_empty());
        assert!(
            pending_git.contains_key(&project_dir),
            "repo refresh should stay queued for child rows"
        );
        assert!(pending_new.is_empty());
    }

    /// Worktree projects have `.git` as a file (not a directory) that
    /// points to a git dir elsewhere. Commit events fire under that
    /// real git dir, not under `repo_root/.git`. Verify the watcher
    /// recognises these events and enqueues a git refresh.
    #[test]
    fn worktree_index_event_enqueues_git_refresh() {
        let tmp = tempfile::tempdir().expect("tempdir");

        // Worktree git dir (simulates <main-repo>/.git/worktrees/<name>).
        let wt_git_dir = tmp
            .path()
            .join("main_repo_git")
            .join("worktrees")
            .join("wt");
        std::fs::create_dir_all(&wt_git_dir).expect("create worktree git dir");
        std::fs::write(wt_git_dir.join("HEAD"), "ref: refs/heads/wt-branch\n").expect("write HEAD");
        std::fs::write(wt_git_dir.join("index"), "fake-index").expect("write index");

        // Worktree project root — `.git` is a file pointing to wt_git_dir.
        let wt_root = tmp.path().join("main_repo_style_fix");
        std::fs::create_dir_all(&wt_root).expect("create worktree root");
        std::fs::write(
            wt_root.join(".git"),
            format!("gitdir: {}\n", wt_git_dir.display()),
        )
        .expect("write .git file");

        let mut projects = HashMap::new();
        projects.insert(
            wt_root.clone(),
            ProjectEntry {
                project_path: "~/main_repo_style_fix".to_string(),
                abs_path:     wt_root.clone(),
                repo_root:    Some(wt_root.clone()),
                git_dir:      Some(wt_git_dir.clone()),
            },
        );
        let scan_root = tmp.path().to_path_buf();
        let project_parents = HashSet::from([scan_root.clone()]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, _bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        // Simulate the index write that happens during a commit.
        // The event fires under the real git dir, not under wt_root/.git.
        handle_event(
            &wt_git_dir.join("index"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(
            pending_git.contains_key(&wt_root),
            "worktree index event should enqueue a git refresh for the worktree project"
        );
    }

    #[test]
    fn worktree_noise_under_real_git_dir_is_ignored() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let wt_git_dir = tmp
            .path()
            .join("main_repo_git")
            .join("worktrees")
            .join("wt");
        std::fs::create_dir_all(&wt_git_dir).expect("create worktree git dir");

        let wt_root = tmp.path().join("main_repo_style_fix");
        std::fs::create_dir_all(&wt_root).expect("create worktree root");
        std::fs::write(
            wt_root.join(".git"),
            format!("gitdir: {}\n", wt_git_dir.display()),
        )
        .expect("write .git file");

        let mut projects = HashMap::new();
        let wt_key = wt_root.clone();
        let wt_abs = wt_root.clone();
        projects.insert(
            wt_key,
            ProjectEntry {
                project_path: "~/main_repo_style_fix".to_string(),
                abs_path:     wt_abs,
                repo_root:    Some(wt_root),
                git_dir:      Some(wt_git_dir.clone()),
            },
        );
        let scan_root = tmp.path().to_path_buf();
        let project_parents = HashSet::from([scan_root.clone()]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, _bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        // An event for objects/pack.tmp under the worktree git dir
        // should not enqueue a git refresh or disk refresh.
        handle_event(
            &wt_git_dir.join("objects").join("pack.tmp"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(
            pending_git.is_empty(),
            "objects noise should not enqueue git refresh"
        );
        assert!(
            pending_disk.is_empty(),
            "objects noise should not enqueue disk refresh"
        );
    }

    #[test]
    fn cache_lint_event_is_ignored_by_project_watcher() {
        let project_root = tempfile::tempdir().expect("tempdir");
        let project_path = "~/rust/demo";
        let mut projects = HashMap::new();
        let (key, entry) = make_project_entry(project_path, project_root.path());
        let latest_path = lint::latest_path_under(&lint::cache_root(), project_root.path());
        projects.insert(key, entry);

        std::fs::create_dir_all(latest_path.parent().expect("latest file has parent"))
            .expect("create cache lint-runs dir");
        std::fs::write(
            &latest_path,
            r#"{"run_id":"run-1","started_at":"2026-03-30T14:22:01-05:00","finished_at":"2026-03-30T14:22:18-05:00","duration_ms":17000,"status":"passed","commands":[]}"#,
        )
        .expect("write latest");

        let scan_root = project_root.path().to_path_buf();
        let project_parents = HashSet::new();
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            &latest_path,
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(bg_rx.try_recv().is_err());
        assert!(pending_disk.is_empty());
        assert!(pending_git.is_empty());
        assert!(pending_new.is_empty());
    }

    #[test]
    fn cache_lint_child_event_is_ignored_by_project_watcher() {
        let project_root = tempfile::tempdir().expect("tempdir");
        let project_path = "~/rust/demo";
        let mut projects = HashMap::new();
        let (key, entry) = make_project_entry(project_path, project_root.path());
        let lint_cache_dir = lint::project_dir(project_root.path());
        let latest_path = lint::latest_path_under(&lint::cache_root(), project_root.path());
        let child_path = lint_cache_dir.join("clippy-latest.log");
        projects.insert(key, entry);

        std::fs::create_dir_all(child_path.parent().expect("child file has parent"))
            .expect("create cache lint-runs child dir");
        std::fs::write(
            &latest_path,
            r#"{"run_id":"run-1","started_at":"2026-03-30T14:22:01-05:00","finished_at":"2026-03-30T14:22:18-05:00","duration_ms":17000,"status":"failed","commands":[]}"#,
        )
        .expect("write latest");
        std::fs::write(&child_path, "warning: example\n").expect("write child file");

        let scan_root = project_root.path().to_path_buf();
        let project_parents = HashSet::new();
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            &child_path,
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(bg_rx.try_recv().is_err());
        assert!(pending_disk.is_empty());
        assert!(pending_git.is_empty());
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
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        let (bg_tx, _bg_rx) = mpsc::channel();
        let event_path = new_project.join("src/main.rs");
        handle_event(
            &event_path,
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(pending_disk.is_empty());
        assert!(pending_git.is_empty());
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
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        let (bg_tx, _bg_rx) = mpsc::channel();
        handle_event(
            &project_dir.join("Cargo.toml"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(pending_git.is_empty());
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
        let (bg_tx, _bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            &new_wt.join("src/main.rs"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
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
    fn resolve_include_dirs_cases() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/user"));
        let cases = [
            (
                "empty_uses_scan_root",
                PathBuf::from("/home/user"),
                Vec::<String>::new(),
                vec![PathBuf::from("/home/user")],
            ),
            (
                "relative_under_scan_root",
                PathBuf::from("/home/user"),
                vec!["rust".to_string(), ".claude".to_string()],
                vec![
                    PathBuf::from("/home/user/rust"),
                    PathBuf::from("/home/user/.claude"),
                ],
            ),
            (
                "tilde_expands_to_home",
                PathBuf::from("/home/user/rust"),
                vec!["~/rust".to_string(), "~/.claude".to_string()],
                vec![home.join("rust"), home.join(".claude")],
            ),
            (
                "absolute_used_as_is",
                PathBuf::from("/home/user"),
                vec!["/opt/projects".to_string()],
                vec![PathBuf::from("/opt/projects")],
            ),
        ];

        for (name, scan_root, include_dirs, expected) in cases {
            let dirs = scan::resolve_include_dirs(&scan_root, &include_dirs);
            assert_eq!(dirs, expected, "{name}");
        }
    }

    // ── fire_disk_updates ───────────────────────────────────────────

    /// Helper: create a git repo in `dir` with one commit so
    /// `GitInfo::detect` returns `Some`.
    fn git_binary() -> &'static str {
        if Path::new("/usr/bin/git").is_file() {
            "/usr/bin/git"
        } else {
            "git"
        }
    }

    fn init_git_repo(dir: &Path) {
        Command::new(git_binary())
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("git init");
        Command::new(git_binary())
            .args(["config", "user.name", "cargo-port-tests"])
            .current_dir(dir)
            .output()
            .expect("git config user.name");
        Command::new(git_binary())
            .args(["config", "user.email", "cargo-port-tests@example.com"])
            .current_dir(dir)
            .output()
            .expect("git config user.email");
        Command::new(git_binary())
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .expect("git add");
        Command::new(git_binary())
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(dir)
            .output()
            .expect("git commit");
    }

    fn manifest_contents(name: &str, workspace: bool) -> String {
        let workspace_section = if workspace { "\n[workspace]\n" } else { "" };
        format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
{workspace_section}
"#
        )
    }

    fn init_cargo_git_repo(dir: &Path, name: &str, workspace: bool) {
        std::fs::create_dir_all(dir.join("src")).expect("create src");
        std::fs::write(dir.join("Cargo.toml"), manifest_contents(name, workspace))
            .expect("write Cargo.toml");
        std::fs::write(dir.join("src").join("main.rs"), "fn main() {}\n").expect("write main.rs");
        init_git_repo(dir);
    }

    fn add_git_worktree(primary_dir: &Path, worktree_dir: &Path, branch: &str) {
        let status = Command::new(git_binary())
            .args([
                "worktree",
                "add",
                worktree_dir.to_str().expect("utf-8 worktree path"),
                "-b",
                branch,
            ])
            .current_dir(primary_dir)
            .status()
            .expect("git worktree add");
        assert!(status.success(), "git worktree add should succeed");
    }

    #[test]
    fn disk_update_only_sends_disk_usage_for_tracked_project() {
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
                abs_path:     project_dir.clone(),
                repo_root:    Some(project_dir.clone()),
                git_dir:      Some(project_dir.join(".git")),
            },
        );

        // Deadline already expired → fires immediately.
        let past = Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .expect("1s subtraction should not underflow");
        let mut pending = HashMap::from([(
            "~/my_project".to_string(),
            DiskState::Pending {
                debounce_deadline: past,
                max_deadline:      past,
            },
        )]);

        let disk_limit = Arc::new(tokio::sync::Semaphore::new(1));
        let (disk_done_tx, disk_done_rx) = mpsc::channel();
        fire_disk_updates(
            test_runtime().handle(),
            &disk_limit,
            &disk_done_tx,
            &tx,
            &projects,
            &mut pending,
        );
        wait_for_completion(&disk_done_rx);

        let mut got_disk = false;
        let mut got_git = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BackgroundMsg::DiskUsage { path, .. } if path.as_path() == project_dir => {
                    got_disk = true;
                },
                BackgroundMsg::GitInfo { path, .. } if path.as_path() == project_dir => {
                    got_git = true;
                },
                _ => {},
            }
        }
        assert!(got_disk, "expected DiskUsage message");
        assert!(!got_git, "disk updates should no longer emit GitInfo");
        assert!(matches!(
            pending.get("~/my_project"),
            Some(DiskState::Running { .. })
        ));
    }

    #[test]
    fn disk_update_skips_git_info_for_untracked_project() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("no_git");
        std::fs::create_dir_all(&project_dir).expect("create dir");

        let (tx, rx) = mpsc::channel();
        let mut projects = HashMap::new();
        let dir_key = project_dir.clone();
        projects.insert(
            dir_key,
            ProjectEntry {
                project_path: "~/no_git".to_string(),
                abs_path:     project_dir.clone(),
                repo_root:    None,
                git_dir:      None,
            },
        );

        let past = Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .expect("1s subtraction should not underflow");
        let mut pending = HashMap::from([(
            "~/no_git".to_string(),
            DiskState::Pending {
                debounce_deadline: past,
                max_deadline:      past,
            },
        )]);

        let disk_limit = Arc::new(tokio::sync::Semaphore::new(1));
        let (disk_done_tx, disk_done_rx) = mpsc::channel();
        fire_disk_updates(
            test_runtime().handle(),
            &disk_limit,
            &disk_done_tx,
            &tx,
            &projects,
            &mut pending,
        );
        wait_for_completion(&disk_done_rx);

        let mut got_disk = false;
        let mut got_git = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BackgroundMsg::DiskUsage { path, .. } if path.as_path() == project_dir => {
                    got_disk = true;
                },
                BackgroundMsg::GitInfo { .. } => got_git = true,
                _ => {},
            }
        }
        assert!(got_disk, "expected DiskUsage message");
        assert!(!got_git, "should not send GitInfo for untracked project");
    }

    #[test]
    fn disk_completion_requeues_once_when_project_changed_while_running() {
        let mut pending = HashMap::from([(
            "~/rust/bevy".to_string(),
            DiskState::Running {
                dirty_since_start: true,
            },
        )]);

        handle_disk_completion(&mut pending, "~/rust/bevy");

        assert_pending_disk(&pending, "~/rust/bevy");
    }

    #[test]
    fn probe_new_package_worktree_emits_discovered_item() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let primary_dir = tmp.path().join("app");
        let linked_dir = tmp.path().join("app_test");
        init_cargo_git_repo(&primary_dir, "app", false);
        add_git_worktree(&primary_dir, &linked_dir, "test/app");

        let (bg_tx, bg_rx) = mpsc::channel();
        let past = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("1s subtraction should not underflow");
        let mut pending_new = HashMap::from([(linked_dir.clone(), past)]);
        let mut discovered = HashSet::new();

        probe_new_projects(
            &bg_tx,
            &mut pending_new,
            &mut discovered,
            5,
            NonRustInclusion::default(),
            &crate::http::HttpClient::new(test_runtime().handle().clone()).expect("http client"),
        );

        let BackgroundMsg::ProjectDiscovered { item } = bg_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("project discovered message")
        else {
            panic!("unexpected message");
        };
        let RootItem::Package(pkg) = item else {
            panic!("expected package worktree item");
        };
        assert_eq!(pkg.path(), linked_dir.as_path());
        assert_eq!(pkg.worktree_name(), Some("app_test"));
        assert_eq!(
            pkg.worktree_primary_abs_path(),
            Some(
                primary_dir
                    .canonicalize()
                    .expect("canonical primary")
                    .as_path()
            )
        );
    }

    #[test]
    fn probe_new_workspace_worktree_emits_discovered_item() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let primary_dir = tmp.path().join("obsidian_knife");
        let linked_dir = tmp.path().join("obsidian_knife_test");
        init_cargo_git_repo(&primary_dir, "obsidian_knife", true);
        add_git_worktree(&primary_dir, &linked_dir, "test/obsidian");

        let (bg_tx, bg_rx) = mpsc::channel();
        let past = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("1s subtraction should not underflow");
        let mut pending_new = HashMap::from([(linked_dir.clone(), past)]);
        let mut discovered = HashSet::new();

        probe_new_projects(
            &bg_tx,
            &mut pending_new,
            &mut discovered,
            5,
            NonRustInclusion::default(),
            &crate::http::HttpClient::new(test_runtime().handle().clone()).expect("http client"),
        );

        let BackgroundMsg::ProjectDiscovered { item } = bg_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("project discovered message")
        else {
            panic!("unexpected message");
        };
        let RootItem::Workspace(ws) = item else {
            panic!("expected workspace worktree item");
        };
        assert_eq!(ws.path(), linked_dir.as_path());
        assert_eq!(ws.worktree_name(), Some("obsidian_knife_test"));
        assert_eq!(
            ws.worktree_primary_abs_path(),
            Some(
                primary_dir
                    .canonicalize()
                    .expect("canonical primary")
                    .as_path()
            )
        );
    }

    #[test]
    fn removed_package_worktree_emits_zero_disk_usage() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let primary_dir = tmp.path().join("app");
        let linked_dir = tmp.path().join("app_test");
        init_cargo_git_repo(&primary_dir, "app", false);
        add_git_worktree(&primary_dir, &linked_dir, "test/app");

        let mut projects = HashMap::new();
        projects.insert(
            linked_dir.clone(),
            ProjectEntry {
                project_path: "~/app_test".to_string(),
                abs_path:     linked_dir.clone(),
                repo_root:    None,
                git_dir:      None,
            },
        );
        let scan_root = tmp.path().to_path_buf();
        let project_parents = HashSet::from([scan_root.clone()]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        std::fs::remove_dir_all(&linked_dir).expect("remove linked worktree");
        handle_event(
            &linked_dir.join("Cargo.toml"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        let past = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("1s subtraction should not underflow");
        pending_disk.insert(
            "~/app_test".to_string(),
            DiskState::Pending {
                debounce_deadline: past,
                max_deadline:      past,
            },
        );
        let disk_limit = Arc::new(tokio::sync::Semaphore::new(1));
        let (disk_done_tx, disk_done_rx) = mpsc::channel();
        fire_disk_updates(
            test_runtime().handle(),
            &disk_limit,
            &disk_done_tx,
            &bg_tx,
            &projects,
            &mut pending_disk,
        );
        wait_for_completion(&disk_done_rx);

        let mut got_zero = false;
        while let Ok(msg) = bg_rx.try_recv() {
            if let BackgroundMsg::DiskUsage { path, bytes } = msg
                && path.as_path() == linked_dir
                && bytes == 0
            {
                got_zero = true;
            }
        }
        assert!(
            got_zero,
            "expected zero-byte disk usage for removed package worktree"
        );
    }

    #[test]
    fn removed_workspace_worktree_emits_zero_disk_usage() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let primary_dir = tmp.path().join("obsidian_knife");
        let linked_dir = tmp.path().join("obsidian_knife_test");
        init_cargo_git_repo(&primary_dir, "obsidian_knife", true);
        add_git_worktree(&primary_dir, &linked_dir, "test/obsidian");

        let mut projects = HashMap::new();
        projects.insert(
            linked_dir.clone(),
            ProjectEntry {
                project_path: "~/obsidian_knife_test".to_string(),
                abs_path:     linked_dir.clone(),
                repo_root:    None,
                git_dir:      None,
            },
        );
        let scan_root = tmp.path().to_path_buf();
        let project_parents = HashSet::from([scan_root.clone()]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            scan_root:       &scan_root,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        std::fs::remove_dir_all(&linked_dir).expect("remove linked worktree");
        handle_event(
            &linked_dir.join("Cargo.toml"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        let past = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("1s subtraction should not underflow");
        pending_disk.insert(
            "~/obsidian_knife_test".to_string(),
            DiskState::Pending {
                debounce_deadline: past,
                max_deadline:      past,
            },
        );
        let disk_limit = Arc::new(tokio::sync::Semaphore::new(1));
        let (disk_done_tx, disk_done_rx) = mpsc::channel();
        fire_disk_updates(
            test_runtime().handle(),
            &disk_limit,
            &disk_done_tx,
            &bg_tx,
            &projects,
            &mut pending_disk,
        );
        wait_for_completion(&disk_done_rx);

        let mut got_zero = false;
        while let Ok(msg) = bg_rx.try_recv() {
            if let BackgroundMsg::DiskUsage { path, bytes } = msg
                && path.as_path() == linked_dir
                && bytes == 0
            {
                got_zero = true;
            }
        }
        assert!(
            got_zero,
            "expected zero-byte disk usage for removed workspace worktree"
        );
    }
}
