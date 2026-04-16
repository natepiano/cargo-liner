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
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
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
#[cfg(test)]
use super::project::ProjectFields;
use super::scan;
use super::scan::BackgroundMsg;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::project::RootItem::NonRust;

/// Request to register an already-known project with the watcher.
pub(crate) struct WatchRequest {
    /// Display path (e.g. `~/foo/bar`).
    pub project_label: String,
    /// Absolute filesystem path to the project root.
    pub abs_path:      AbsolutePath,
    /// Absolute path of the containing git repo root when known.
    pub repo_root:     Option<AbsolutePath>,
}

pub(crate) enum WatcherMsg {
    Register(WatchRequest),
    InitialRegistrationComplete,
}

/// Spawn a unified background watcher thread. Watches the include
/// directories recursively and handles disk-usage updates,
/// new-project detection, and deleted-project detection.
pub(crate) fn spawn_watcher(
    watch_roots: Vec<AbsolutePath>,
    bg_tx: mpsc::Sender<BackgroundMsg>,
    ci_run_count: u32,
    non_rust: NonRustInclusion,
    client: HttpClient,
) -> mpsc::Sender<WatcherMsg> {
    let (watch_tx, watch_rx) = mpsc::channel();
    let (notify_tx, notify_rx) = mpsc::channel();
    let handler = move |res| {
        let _ = notify_tx.send(res);
    };
    let Ok(mut watcher) = notify::recommended_watcher(handler) else {
        return watch_tx;
    };
    let started = Instant::now();
    register_watch_roots(&mut watcher, &watch_roots);
    tracing::info!(
        roots = watch_roots.len(),
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        "watcher_root_registration_complete"
    );
    let ctx = WatcherLoopContext {
        watch_roots,
        bg_tx,
        ci_run_count,
        non_rust,
        client,
    };

    spawn_watcher_thread(ctx, watch_rx, notify_rx, watcher);

    watch_tx
}

struct WatcherLoopContext {
    watch_roots:  Vec<AbsolutePath>,
    bg_tx:        mpsc::Sender<BackgroundMsg>,
    ci_run_count: u32,
    non_rust:     NonRustInclusion,
    client:       HttpClient,
}

fn spawn_watcher_thread<W: Send + 'static>(
    ctx: WatcherLoopContext,
    watch_rx: mpsc::Receiver<WatcherMsg>,
    notify_rx: mpsc::Receiver<notify::Result<notify::Event>>,
    watcher_guard: W,
) {
    thread::spawn(move || {
        watcher_loop(&ctx, &watch_rx, &notify_rx, watcher_guard);
    });
}

/// Per-project tracking state.
struct ProjectEntry {
    project_label:  String,
    abs_path:       AbsolutePath,
    repo_root:      Option<AbsolutePath>,
    /// The resolved on-disk git directory. For normal repos this is
    /// `repo_root/.git`; for worktrees it follows the `.git` file to the
    /// real directory (e.g. `<main-repo>/.git/worktrees/<name>`).
    git_dir:        Option<AbsolutePath>,
    /// The shared git directory that holds branch refs. For linked worktrees
    /// this points at the primary repo's `.git` directory.
    common_git_dir: Option<AbsolutePath>,
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

enum GitRefreshState {
    Pending {
        debounce_deadline: Instant,
        max_deadline:      Instant,
        refresh_scope:     GitRefreshKind,
    },
    Running {
        dirty_since_start: bool,
        refresh_scope:     GitRefreshKind,
    },
}

/// Classifies a filesystem event by what level of git detection it requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitRefreshKind {
    /// The event can only change per-project path state
    /// (clean/modified/untracked/ignored). Examples: `.gitignore` edits, git
    /// index updates, `info/exclude` changes.
    PathStateOnly,
    /// The event may have changed repo-level metadata (branch, remote,
    /// ahead/behind). Examples: `HEAD` changes, ref updates, `packed-refs`
    /// writes.
    FullMetadata,
}

impl GitRefreshKind {
    /// Widen scope: if either side is `FullMetadata`, the result is
    /// `FullMetadata`.
    const fn widen(&mut self, other: Self) {
        if matches!(other, Self::FullMetadata) {
            *self = Self::FullMetadata;
        }
    }
}

struct WatcherLoopState {
    projects:             HashMap<AbsolutePath, ProjectEntry>,
    project_parents:      HashSet<AbsolutePath>,
    pending_disk:         HashMap<String, DiskState>,
    pending_git:          HashMap<AbsolutePath, GitRefreshState>,
    pending_new:          HashMap<AbsolutePath, Instant>,
    discovered:           HashSet<AbsolutePath>,
    watched_git_metadata: HashSet<AbsolutePath>,
    initializing:         bool,
    buffered_events:      Vec<notify::Event>,
}

impl WatcherLoopState {
    fn new() -> Self {
        Self {
            projects:             HashMap::new(),
            project_parents:      HashSet::new(),
            pending_disk:         HashMap::new(),
            pending_git:          HashMap::new(),
            pending_new:          HashMap::new(),
            discovered:           HashSet::new(),
            watched_git_metadata: HashSet::new(),
            initializing:         true,
            buffered_events:      Vec::new(),
        }
    }
}

fn watcher_loop<W: Send + 'static>(
    ctx: &WatcherLoopContext,
    watch_rx: &mpsc::Receiver<WatcherMsg>,
    notify_rx: &mpsc::Receiver<notify::Result<notify::Event>>,
    _watcher: W,
) {
    let WatcherLoopContext {
        watch_roots,
        bg_tx,
        ci_run_count,
        non_rust,
        client,
    } = ctx;
    let mut state = WatcherLoopState::new();
    let (disk_done_tx, disk_done_rx) = mpsc::channel::<String>();
    let (git_done_tx, git_done_rx) = mpsc::channel::<AbsolutePath>();
    let disk_limit = Arc::new(tokio::sync::Semaphore::new(WATCHER_DISK_CONCURRENCY));
    let git_limit = Arc::new(tokio::sync::Semaphore::new(WATCHER_GIT_CONCURRENCY));

    let mut tick: u64 = 0;
    loop {
        tick += 1;
        let watch_drain = drain_watch_messages(
            watch_rx,
            &mut state.projects,
            &mut state.project_parents,
            &mut state.watched_git_metadata,
            &mut state.initializing,
        );
        if watch_drain.disconnected {
            tracing::info!(tick, "watcher_loop_exit_disconnected");
            return;
        }

        let notify_events = drain_notify_events(notify_rx);
        process_notify_events(
            tick,
            &watch_drain,
            notify_events,
            watch_roots,
            bg_tx,
            &mut state,
        );
        drain_completed_refreshes(
            &disk_done_rx,
            &git_done_rx,
            &mut state.pending_disk,
            &mut state.pending_git,
        );

        // Fire git refreshes whose debounce has expired.
        fire_git_updates(
            &client.handle,
            &git_limit,
            &git_done_tx,
            bg_tx,
            &state.projects,
            &mut state.pending_git,
        );

        // Fire disk recalculations whose debounce has expired.
        fire_disk_updates(
            &client.handle,
            &disk_limit,
            &disk_done_tx,
            bg_tx,
            &state.projects,
            &mut state.pending_disk,
        );

        // Probe new-project candidates whose debounce has expired.
        probe_new_projects(
            bg_tx,
            &mut state.pending_new,
            &mut state.discovered,
            *ci_run_count,
            *non_rust,
            client,
        );

        thread::sleep(POLL_INTERVAL);
    }
}

fn register_watch_roots(watcher: &mut impl Watcher, watch_dirs: &[AbsolutePath]) {
    for dir in watch_dirs {
        if dir.is_dir() {
            let _ = watcher.watch(dir, RecursiveMode::Recursive);
        }
    }
}

struct WatchDrainResult {
    disconnected:           bool,
    registration_completed: bool,
}

fn drain_watch_messages(
    watch_rx: &mpsc::Receiver<WatcherMsg>,
    projects: &mut HashMap<AbsolutePath, ProjectEntry>,
    project_parents: &mut HashSet<AbsolutePath>,
    watched_git_metadata: &mut HashSet<AbsolutePath>,
    initializing: &mut bool,
) -> WatchDrainResult {
    let mut result = WatchDrainResult {
        disconnected:           false,
        registration_completed: false,
    };
    loop {
        match watch_rx.try_recv() {
            Ok(WatcherMsg::Register(req)) => {
                apply_watch_request(req, projects, project_parents, watched_git_metadata);
            },
            Ok(WatcherMsg::InitialRegistrationComplete) => {
                *initializing = false;
                result.registration_completed = true;
            },
            Err(mpsc::TryRecvError::Empty) => return result,
            Err(mpsc::TryRecvError::Disconnected) => {
                result.disconnected = true;
                return result;
            },
        }
    }
}

fn apply_watch_request(
    req: WatchRequest,
    projects: &mut HashMap<AbsolutePath, ProjectEntry>,
    project_parents: &mut HashSet<AbsolutePath>,
    _watched_git_metadata: &mut HashSet<AbsolutePath>,
) {
    if let Some(parent) = req.abs_path.parent() {
        project_parents.insert(AbsolutePath::from(parent));
    }
    let git_dir = req.repo_root.as_deref().and_then(project::resolve_git_dir);
    let common_git_dir = req
        .repo_root
        .as_deref()
        .and_then(project::resolve_common_git_dir);
    projects.insert(
        req.abs_path.clone(),
        ProjectEntry {
            project_label: req.project_label,
            abs_path: req.abs_path.clone(),
            repo_root: req.repo_root,
            git_dir,
            common_git_dir,
        },
    );
}

fn process_notify_events(
    tick: u64,
    watch_drain: &WatchDrainResult,
    notify_events: Vec<notify::Event>,
    watch_roots: &[AbsolutePath],
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    state: &mut WatcherLoopState,
) {
    let notify_count = notify_events.len();
    if watch_drain.registration_completed {
        tracing::info!(
            tick,
            buffered = state.buffered_events.len(),
            notify_count,
            initializing = state.initializing,
            projects = state.projects.len(),
            "watcher_loop_registration_completed"
        );
        let dispatch = WatcherDispatchContext {
            event: EventContext {
                watch_roots,
                projects: &state.projects,
                project_parents: &state.project_parents,
                discovered: &state.discovered,
            },
            bg_tx,
        };
        replay_buffered_events(
            &state.buffered_events,
            &dispatch,
            &mut state.pending_disk,
            &mut state.pending_git,
            &mut state.pending_new,
        );
        state.buffered_events.clear();
    }
    if state.initializing {
        if notify_count > 0 {
            tracing::info!(
                tick,
                notify_count,
                buffered_total = state.buffered_events.len() + notify_count,
                "watcher_loop_buffering_while_initializing"
            );
        }
        state.buffered_events.extend(notify_events);
    } else {
        if notify_count > 0 {
            tracing::info!(tick, notify_count, "watcher_loop_processing_events");
        }
        let dispatch = WatcherDispatchContext {
            event: EventContext {
                watch_roots,
                projects: &state.projects,
                project_parents: &state.project_parents,
                discovered: &state.discovered,
            },
            bg_tx,
        };
        replay_buffered_events(
            &notify_events,
            &dispatch,
            &mut state.pending_disk,
            &mut state.pending_git,
            &mut state.pending_new,
        );
    }
}

fn drain_notify_events(
    notify_rx: &mpsc::Receiver<notify::Result<notify::Event>>,
) -> Vec<notify::Event> {
    let mut events = Vec::new();
    while let Ok(result) = notify_rx.try_recv() {
        match result {
            Ok(event) => events.push(event),
            Err(err) => {
                tracing::warn!(error = %err, "watcher_notify_error");
            },
        }
    }
    events
}

fn replay_buffered_events(
    events: &[notify::Event],
    ctx: &WatcherDispatchContext<'_>,
    pending_disk: &mut HashMap<String, DiskState>,
    pending_git: &mut HashMap<AbsolutePath, GitRefreshState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
) {
    for event in events {
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
    git_done_rx: &mpsc::Receiver<AbsolutePath>,
    pending_disk: &mut HashMap<String, DiskState>,
    pending_git: &mut HashMap<AbsolutePath, GitRefreshState>,
) {
    while let Ok(project_path) = disk_done_rx.try_recv() {
        handle_disk_completion(pending_disk, &project_path);
    }

    while let Ok(repo_root) = git_done_rx.try_recv() {
        handle_git_completion(pending_git, repo_root);
    }
}

/// Immutable state needed to classify a filesystem event.
struct EventContext<'a> {
    watch_roots:     &'a [AbsolutePath],
    projects:        &'a HashMap<AbsolutePath, ProjectEntry>,
    project_parents: &'a HashSet<AbsolutePath>,
    discovered:      &'a HashSet<AbsolutePath>,
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
    pending_git: &mut HashMap<AbsolutePath, GitRefreshState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
) {
    let now = Instant::now();

    let mut matched_fast_git = false;
    for entry in ctx.projects.values() {
        if let Some(refresh_kind) = classify_fast_git_event(event_path, entry)
            && let Some(repo_root) = &entry.repo_root
        {
            matched_fast_git = true;
            tracing::info!(
                repo_root = %repo_root.display(),
                event_path = %event_path.display(),
                refresh_scope = ?refresh_kind,
                "watcher_fast_git_metadata_event"
            );
            emit_root_git_info_refresh(bg_tx, ctx.projects, repo_root);
            enqueue_git_refresh(
                pending_git,
                repo_root.clone(),
                now,
                false,
                refresh_kind,
                match refresh_kind {
                    GitRefreshKind::FullMetadata => "fast_git_metadata",
                    GitRefreshKind::PathStateOnly => "fast_git_path_state",
                },
            );
        }
    }
    if matched_fast_git {
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
                    refresh_kind,
                    match refresh_kind {
                        GitRefreshKind::FullMetadata => "git_internal",
                        GitRefreshKind::PathStateOnly => "git_internal_path_state",
                    },
                );
            }
            return;
        }
        let is_target_event = event_path.starts_with(entry.abs_path.join("target"));
        schedule_disk_refresh(pending_disk, &entry.project_label, now);
        if !is_target_event && let Some(repo_root) = &entry.repo_root {
            let scope = classify_internal_git_event(event_path, entry)
                .unwrap_or(GitRefreshKind::PathStateOnly);
            enqueue_git_refresh(
                pending_git,
                repo_root.clone(),
                now,
                false,
                scope,
                "project_event",
            );
        }
        return;
    }

    // Not a known project — walk up from the event path to find the
    // directory at the same level as existing projects. A "project parent"
    // is any directory that already contains a known project (e.g. `~/rust/`).
    let Some(candidate) = project_level_dir(event_path, ctx.watch_roots, ctx.project_parents)
    else {
        return;
    };
    // Canonicalize so symlinked notify paths match existing project keys.
    let candidate = AbsolutePath::from(
        candidate
            .to_path_buf()
            .canonicalize()
            .unwrap_or_else(|_| candidate.to_path_buf()),
    );
    // Always enqueue removals (dir gone); for creations, skip already-discovered.
    if !candidate.is_dir() || !ctx.discovered.contains(&candidate) {
        pending_new
            .entry(candidate)
            .or_insert_with(|| now + NEW_PROJECT_DEBOUNCE);
    }
}

fn schedule_disk_refresh(
    pending_disk: &mut HashMap<String, DiskState>,
    project_label: &str,
    now: Instant,
) {
    match pending_disk.get_mut(project_label) {
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
                project_label.to_string(),
                DiskState::Pending {
                    debounce_deadline: now + DEBOUNCE_DURATION,
                    max_deadline:      now + MAX_WAIT,
                },
            );
        },
    }
}

fn handle_disk_completion(pending_disk: &mut HashMap<String, DiskState>, project_label: &str) {
    let now = Instant::now();
    let Some(state) = pending_disk.remove(project_label) else {
        return;
    };
    if let DiskState::Running { dirty_since_start } = state
        && dirty_since_start
    {
        pending_disk.insert(
            project_label.to_string(),
            DiskState::Pending {
                debounce_deadline: now + DEBOUNCE_DURATION,
                max_deadline:      now + MAX_WAIT,
            },
        );
    }
}

fn handle_git_completion(
    pending_git: &mut HashMap<AbsolutePath, GitRefreshState>,
    repo_root: AbsolutePath,
) {
    let now = Instant::now();
    let Some(state) = pending_git.remove(&repo_root) else {
        return;
    };
    if let GitRefreshState::Running {
        dirty_since_start,
        refresh_scope,
    } = state
        && dirty_since_start
    {
        pending_git.insert(
            repo_root,
            GitRefreshState::Pending {
                debounce_deadline: now + DEBOUNCE_DURATION,
                max_deadline: now + MAX_WAIT,
                refresh_scope,
            },
        );
    }
}

fn classify_fast_git_event(event_path: &Path, entry: &ProjectEntry) -> Option<GitRefreshKind> {
    let repo_root = entry.repo_root.as_deref()?;
    let git_dir = entry.git_dir.as_deref()?;
    let common_git_dir = entry.common_git_dir.as_deref()?;
    if event_path == repo_root.join(".gitignore")
        || event_path == git_dir.join("index")
        || event_path == git_dir.join("info").join("exclude")
    {
        Some(GitRefreshKind::PathStateOnly)
    } else if event_path == git_dir.join("HEAD")
        || event_path == common_git_dir.join("packed-refs")
        || event_path.starts_with(common_git_dir.join("refs").join("heads"))
        || event_path.starts_with(common_git_dir.join("refs").join("remotes"))
    {
        Some(GitRefreshKind::FullMetadata)
    } else {
        classify_worktree_git_fallback(event_path, git_dir)
    }
}

fn is_internal_git_path(event_path: &Path, entry: &ProjectEntry) -> bool {
    let repo_root = entry.repo_root.as_deref();
    let git_dir = entry.git_dir.as_deref();
    let common_git_dir = entry.common_git_dir.as_deref();
    // Match events under the resolved git dir (handles worktrees) or
    // under repo_root/.git (handles normal repos where git_dir ==
    // repo_root/.git, but also catches events like refs/heads updates
    // that live in the common git dir rather than the worktree git dir).
    git_dir.is_some_and(|d| event_path.starts_with(d))
        || common_git_dir.is_some_and(|d| event_path.starts_with(d))
        || repo_root.is_some_and(|r| event_path.starts_with(r.join(".git")))
}

fn classify_internal_git_event(event_path: &Path, entry: &ProjectEntry) -> Option<GitRefreshKind> {
    let git_dir = entry.git_dir.as_deref()?;
    let common_git_dir = entry.common_git_dir.as_deref()?;
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
        || event_path == common_git_dir.join("packed-refs")
        || event_path.starts_with(common_git_dir.join("refs").join("heads"))
        || event_path.starts_with(common_git_dir.join("refs").join("remotes"))
    {
        Some(GitRefreshKind::FullMetadata)
    } else {
        classify_worktree_git_fallback(event_path, git_dir)
    }
}

fn classify_worktree_git_fallback(event_path: &Path, git_dir: &Path) -> Option<GitRefreshKind> {
    let logs_dir = git_dir.join("logs");
    if event_path == git_dir || event_path == logs_dir || event_path.starts_with(&logs_dir) {
        Some(GitRefreshKind::PathStateOnly)
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

fn spawn_project_refresh(bg_tx: mpsc::Sender<BackgroundMsg>, project_root: AbsolutePath) {
    rayon::spawn(move || {
        let Some(item) = scan::discover_project_item(&project_root).or_else(|| {
            let cargo_toml = project_root.join("Cargo.toml");
            crate::project::from_cargo_toml(&cargo_toml)
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

fn spawn_project_refresh_after(
    bg_tx: mpsc::Sender<BackgroundMsg>,
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

fn enqueue_git_refresh(
    pending_git: &mut HashMap<AbsolutePath, GitRefreshState>,
    repo_root: AbsolutePath,
    now: Instant,
    immediate: bool,
    refresh_scope: GitRefreshKind,
    cause: &str,
) {
    let pending_count = pending_git
        .iter()
        .filter(|(path, _)| path.as_path() != repo_root.as_path())
        .filter(|(_, state)| matches!(state, GitRefreshState::Pending { .. }))
        .count()
        + usize::from(!matches!(
            pending_git.get(&repo_root),
            Some(GitRefreshState::Pending { .. })
        ));
    tracing::info!(
        repo_root = %repo_root.display(),
        immediate,
        refresh_scope = ?refresh_scope,
        cause,
        pending_git = pending_count,
        "watcher_enqueue_git_refresh"
    );
    match pending_git.get_mut(&repo_root) {
        Some(GitRefreshState::Pending {
            debounce_deadline,
            refresh_scope: pending_scope,
            ..
        }) => {
            *debounce_deadline = if immediate {
                now
            } else {
                now + DEBOUNCE_DURATION
            };
            pending_scope.widen(refresh_scope);
        },
        Some(GitRefreshState::Running {
            dirty_since_start,
            refresh_scope: pending_scope,
        }) => {
            *dirty_since_start = true;
            pending_scope.widen(refresh_scope);
        },
        None => {
            pending_git.insert(
                repo_root,
                GitRefreshState::Pending {
                    debounce_deadline: if immediate {
                        now
                    } else {
                        now + DEBOUNCE_DURATION
                    },
                    max_deadline: now + MAX_WAIT,
                    refresh_scope,
                },
            );
        },
    }
}

fn emit_root_git_info_refresh(
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    repo_root: &Path,
) {
    let started = Instant::now();
    let Some(root_entry) = projects
        .values()
        .find(|entry| entry.abs_path.as_path() == repo_root)
    else {
        return;
    };
    let Some(info) = GitInfo::detect_fast(repo_root) else {
        return;
    };
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        repo_root = %repo_root.display(),
        path = %root_entry.project_label,
        path_state = %info.path_state.label(),
        "watcher_root_git_info_refresh"
    );
    let _ = bg_tx.send(BackgroundMsg::GitInfo {
        path: root_entry.abs_path.clone(),
        info,
    });
}

fn fire_git_updates(
    handle: &tokio::runtime::Handle,
    git_limit: &Arc<tokio::sync::Semaphore>,
    git_done_tx: &mpsc::Sender<AbsolutePath>,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    pending_git: &mut HashMap<AbsolutePath, GitRefreshState>,
) {
    let now = Instant::now();
    let ready: Vec<(AbsolutePath, GitRefreshKind)> = pending_git
        .iter()
        .filter_map(|(repo_root, state)| match state {
            GitRefreshState::Pending {
                debounce_deadline,
                max_deadline,
                refresh_scope,
            } if now >= *debounce_deadline || now >= *max_deadline => {
                Some((repo_root.clone(), *refresh_scope))
            },
            GitRefreshState::Pending { .. } | GitRefreshState::Running { .. } => None,
        })
        .collect();

    for (repo_root, refresh_scope) in ready {
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
            GitRefreshState::Running {
                dirty_since_start: false,
                refresh_scope:     GitRefreshKind::PathStateOnly,
            },
        );
        spawn_git_refresh(
            handle,
            git_limit,
            git_done_tx.clone(),
            bg_tx.clone(),
            repo_root,
            affected,
            refresh_scope,
        );
    }
}

fn spawn_git_refresh(
    handle: &tokio::runtime::Handle,
    git_limit: &Arc<tokio::sync::Semaphore>,
    git_done_tx: mpsc::Sender<AbsolutePath>,
    bg_tx: mpsc::Sender<BackgroundMsg>,
    repo_root: AbsolutePath,
    affected: Vec<(String, String)>,
    refresh_scope: GitRefreshKind,
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
        let repo_root_for_detect = repo_root.clone();
        let git_info =
            tokio::task::spawn_blocking(move || GitInfo::detect_fast(&repo_root_for_detect))
                .await
                .ok()
                .flatten();
        if let Some(info) = git_info {
            for (path, _) in &affected {
                let _ = bg_tx.send(BackgroundMsg::GitInfo {
                    path: AbsolutePath::from(path.clone()),
                    info: info.clone(),
                });
            }
        }

        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            repo_root = %repo_root.display(),
            affected_rows = affected.len(),
            refresh_scope = ?refresh_scope,
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
    projects: &HashMap<AbsolutePath, ProjectEntry>,
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

    for project_label in ready {
        let Some(state) = pending_disk.get_mut(&project_label) else {
            continue;
        };
        *state = DiskState::Running {
            dirty_since_start: false,
        };
        let Some(entry) = projects.values().find(|e| e.project_label == project_label) else {
            continue;
        };
        spawn_disk_update(
            handle,
            disk_limit,
            disk_done_tx.clone(),
            bg_tx.clone(),
            project_label.clone(),
            entry.abs_path.clone(),
        );
    }
}

fn spawn_disk_update(
    handle: &tokio::runtime::Handle,
    disk_limit: &Arc<tokio::sync::Semaphore>,
    disk_done_tx: mpsc::Sender<String>,
    bg_tx: mpsc::Sender<BackgroundMsg>,
    project_label: String,
    abs_path: AbsolutePath,
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
            path = %project_label,
            abs_path = %abs_path.display(),
            "watcher_disk_queue_wait"
        );

        let started = Instant::now();
        let abs_for_msg = abs_path.clone();
        let bytes = tokio::task::spawn_blocking(move || scan::dir_size(&abs_path))
            .await
            .ok()
            .unwrap_or(0);
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            path = %project_label,
            bytes,
            "watcher_disk_usage"
        );
        let _ = bg_tx.send(BackgroundMsg::DiskUsage {
            path: abs_for_msg,
            bytes,
        });
        let _ = disk_done_tx.send(project_label);
    });
}

fn probe_new_projects(
    bg_tx: &mpsc::Sender<BackgroundMsg>,
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
                // refresh repairs that initial partial shape once the checkout
                // settles.
                spawn_project_refresh_after(bg_tx.clone(), abs_path.clone(), NEW_PROJECT_DEBOUNCE);
            }
            let tx = bg_tx.clone();
            let task_ctx = scan::FetchContext {
                client: client.clone(),
            };
            let lang_tx = bg_tx.clone();
            let lang_path = abs_path.clone();
            rayon::spawn(move || {
                let request = scan::ProjectDetailRequest {
                    tx: &tx,
                    ctx: &task_ctx,
                    _project_path: display_path.as_str(),
                    abs_path: &abs_path,
                    project_name: project_name.as_deref(),
                    repo_presence,
                };
                scan::fetch_project_details(&request);
            });
            rayon::spawn(move || {
                let stats = scan::collect_language_stats_single(&lang_path);
                if !stats.entries.is_empty() {
                    let _ = lang_tx.send(scan::BackgroundMsg::LanguageStatsBatch {
                        entries: vec![(lang_path, stats)],
                    });
                }
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
fn project_level_dir(
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
fn probe_project(dir: &Path, non_rust: NonRustInclusion) -> Option<RootItem> {
    let cargo_toml = dir.join("Cargo.toml");
    if cargo_toml.exists() {
        return scan::discover_project_item(dir);
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
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::OnceLock;
    use std::time::Duration;

    use super::*;
    use crate::lint;
    use crate::project::GitPathState;
    use crate::project::GitPathState::Clean;
    use crate::project::GitPathState::Modified;

    fn test_runtime() -> &'static tokio::runtime::Runtime {
        static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        TEST_RT.get_or_init(|| {
            tokio::runtime::Runtime::new().unwrap_or_else(|_| std::process::abort())
        })
    }

    #[test]
    fn initial_registration_complete_transitions_watcher_out_of_initializing() {
        let (watch_tx, watch_rx) = mpsc::channel();
        let mut projects = HashMap::new();
        let mut project_parents = HashSet::new();
        let mut watched_git_metadata = HashSet::new();
        let mut initializing = true;

        watch_tx
            .send(WatcherMsg::InitialRegistrationComplete)
            .expect("send registration complete");

        let drained = drain_watch_messages(
            &watch_rx,
            &mut projects,
            &mut project_parents,
            &mut watched_git_metadata,
            &mut initializing,
        );

        assert!(drained.registration_completed);
        assert!(!initializing);
    }

    #[test]
    fn registration_batch_completes_without_metadata_watch_calls() {
        let (watch_tx, watch_rx) = mpsc::channel();
        let project_dir = tempfile::tempdir().expect("tempdir");
        init_git_repo(project_dir.path());

        watch_tx
            .send(WatcherMsg::Register(WatchRequest {
                project_label: project_dir.path().display().to_string(),
                abs_path:      AbsolutePath::from(project_dir.path()),
                repo_root:     Some(AbsolutePath::from(project_dir.path())),
            }))
            .expect("send register");
        watch_tx
            .send(WatcherMsg::InitialRegistrationComplete)
            .expect("send registration complete");

        let (result_tx, result_rx) = mpsc::channel();
        let watch_thread = std::thread::spawn(move || {
            let mut projects = HashMap::new();
            let mut project_parents = HashSet::new();
            let mut watched_git_metadata = HashSet::new();
            let mut initializing = true;
            let drained = drain_watch_messages(
                &watch_rx,
                &mut projects,
                &mut project_parents,
                &mut watched_git_metadata,
                &mut initializing,
            );
            let _ = result_tx.send((drained, initializing));
        });

        let (drained, initializing) = result_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("drain result without blocking");
        watch_thread.join().expect("watch thread join");

        assert!(drained.registration_completed);
        assert!(!initializing);
    }

    #[test]
    fn spawn_watcher_thread_keeps_watcher_guard_alive_until_shutdown() {
        struct DropSignal(std::sync::Arc<std::sync::atomic::AtomicBool>);

        impl Drop for DropSignal {
            fn drop(&mut self) { self.0.store(true, std::sync::atomic::Ordering::SeqCst); }
        }

        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watcher_guard = DropSignal(std::sync::Arc::clone(&dropped));
        let (watch_tx, watch_rx) = mpsc::channel();
        let (notify_tx, notify_rx) = mpsc::channel();
        let (bg_tx, _bg_rx) = mpsc::channel();
        let client = HttpClient::new(test_runtime().handle().clone()).expect("http client");

        spawn_watcher_thread(
            WatcherLoopContext {
                watch_roots: Vec::new(),
                bg_tx,
                ci_run_count: 0,
                non_rust: NonRustInclusion::Exclude,
                client,
            },
            watch_rx,
            notify_rx,
            watcher_guard,
        );

        std::thread::sleep(POLL_INTERVAL + Duration::from_millis(100));
        assert!(
            !dropped.load(std::sync::atomic::Ordering::SeqCst),
            "watcher guard dropped before watcher thread shutdown"
        );

        drop(notify_tx);
        drop(watch_tx);
        std::thread::sleep(POLL_INTERVAL + Duration::from_millis(100));
        assert!(
            dropped.load(std::sync::atomic::Ordering::SeqCst),
            "watcher guard should drop after watcher thread exits"
        );
    }

    fn wait_for_completion<T>(rx: &mpsc::Receiver<T>) {
        rx.recv_timeout(Duration::from_secs(1))
            .unwrap_or_else(|_| panic!("timed out waiting for background completion"));
    }

    fn collect_messages_until(
        rx: &mpsc::Receiver<BackgroundMsg>,
        predicate: impl Fn(&BackgroundMsg) -> bool,
    ) -> Vec<BackgroundMsg> {
        collect_messages_until_with_timeout(rx, Duration::from_secs(1), predicate)
    }

    fn collect_messages_until_with_timeout(
        rx: &mpsc::Receiver<BackgroundMsg>,
        timeout: Duration,
        predicate: impl Fn(&BackgroundMsg) -> bool,
    ) -> Vec<BackgroundMsg> {
        let first = rx
            .recv_timeout(timeout)
            .unwrap_or_else(|_| panic!("timed out waiting for background message"));
        let started = Instant::now();
        let mut messages = vec![first];
        while !messages.iter().any(&predicate) {
            let remaining = timeout.saturating_sub(started.elapsed());
            let next = rx
                .recv_timeout(remaining)
                .unwrap_or_else(|_| panic!("timed out waiting for expected background message"));
            messages.push(next);
        }
        messages
    }

    // ── project_level_dir ────────────────────────────────────────────

    #[test]
    fn project_level_dir_handles_synthetic_path_shapes() {
        struct Case {
            name:        &'static str,
            watch_roots: &'static [&'static str],
            parents:     &'static [&'static str],
            event:       &'static str,
            expected:    Option<&'static str>,
        }

        let cases = [
            Case {
                name:        "sibling of known project",
                watch_roots: &["/home/user"],
                parents:     &["/home/user/rust"],
                event:       "/home/user/rust/bevy_style_fix/src/main.rs",
                expected:    Some("/home/user/rust/bevy_style_fix"),
            },
            Case {
                name:        "direct child of scan root",
                watch_roots: &["/home/user/rust"],
                parents:     &[],
                event:       "/home/user/rust/new_project/Cargo.toml",
                expected:    Some("/home/user/rust/new_project"),
            },
            Case {
                name:        "event is new directory itself",
                watch_roots: &["/home/user"],
                parents:     &["/home/user/rust"],
                event:       "/home/user/rust/new_wt",
                expected:    Some("/home/user/rust/new_wt"),
            },
            Case {
                name:        "deeply nested event resolves to project dir",
                watch_roots: &["/home/user"],
                parents:     &["/home/user/rust"],
                event:       "/home/user/rust/cargo-port_wt/src/tui/render.rs",
                expected:    Some("/home/user/rust/cargo-port_wt"),
            },
            Case {
                name:        "event at scan root returns none",
                watch_roots: &["/home/user"],
                parents:     &["/home/user/rust"],
                event:       "/home/user",
                expected:    None,
            },
            Case {
                name:        "event outside scan root returns none",
                watch_roots: &["/home/user/rust"],
                parents:     &[],
                event:       "/tmp/other/file.rs",
                expected:    None,
            },
            Case {
                name:        "multiple parent levels rust",
                watch_roots: &["/home/user"],
                parents:     &["/home/user/code/rust", "/home/user/code/python"],
                event:       "/home/user/code/rust/new_crate/src/lib.rs",
                expected:    Some("/home/user/code/rust/new_crate"),
            },
            Case {
                name:        "multiple parent levels python",
                watch_roots: &["/home/user"],
                parents:     &["/home/user/code/rust", "/home/user/code/python"],
                event:       "/home/user/code/python/new_pkg/setup.py",
                expected:    Some("/home/user/code/python/new_pkg"),
            },
            Case {
                name:        "synthetic path resolves to scan root child",
                watch_roots: &["/home/user"],
                parents:     &[],
                event:       "/home/user/rust/bevy/src/lib.rs",
                expected:    Some("/home/user/rust"),
            },
        ];

        for case in cases {
            let watch_roots: Vec<AbsolutePath> = case
                .watch_roots
                .iter()
                .map(|r| AbsolutePath::from((*r).to_string()))
                .collect();
            let parents = case
                .parents
                .iter()
                .map(|p| AbsolutePath::from((*p).to_string()))
                .collect();
            let result = project_level_dir(Path::new(case.event), &watch_roots, &parents);
            assert_eq!(
                result.as_deref(),
                case.expected.map(Path::new),
                "{}",
                case.name
            );
        }
    }

    /// Filesystem markers (`Cargo.toml`) are detected regardless of
    /// whether `project_parents` is empty or populated.
    #[test]
    fn project_level_dir_finds_filesystem_markers() {
        struct Case {
            name:     &'static str,
            parents:  HashSet<AbsolutePath>,
            event:    AbsolutePath,
            expected: AbsolutePath,
        }

        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let watch_roots = vec![AbsolutePath::from(tmp.path())];
        let project_dir = tmp.path().join("rust").join("new_project");
        let unknown_parent_project = tmp.path().join("python").join("new_thing");
        let workspace_root = tmp.path().join("rust").join("bevy_brp_test");
        let member_dir = workspace_root.join("extras");

        std::fs::create_dir_all(&project_dir).expect("create dirs");
        std::fs::write(project_dir.join("Cargo.toml"), b"[package]").expect("write Cargo.toml");
        std::fs::create_dir_all(&unknown_parent_project).expect("create dirs");
        std::fs::write(unknown_parent_project.join("Cargo.toml"), b"[package]")
            .expect("write Cargo.toml");
        std::fs::create_dir_all(member_dir.join("src")).expect("create member dirs");
        std::fs::write(
            workspace_root.join("Cargo.toml"),
            b"[workspace]\nmembers=[\"extras\"]",
        )
        .expect("write workspace Cargo.toml");
        std::fs::write(
            member_dir.join("Cargo.toml"),
            b"[package]\nname=\"extras\"\nversion=\"0.1.0\"",
        )
        .expect("write member Cargo.toml");

        let cases = [
            Case {
                name:     "finds cargo toml under empty parents",
                parents:  HashSet::new(),
                event:    AbsolutePath::from(project_dir.join("src/main.rs")),
                expected: AbsolutePath::from(project_dir.clone()),
            },
            Case {
                name:     "finds project in unknown parent via filesystem",
                parents:  HashSet::from([AbsolutePath::from(tmp.path().join("rust"))]),
                event:    AbsolutePath::from(unknown_parent_project.join("src/lib.rs")),
                expected: AbsolutePath::from(unknown_parent_project.clone()),
            },
            Case {
                name:     "nested workspace member resolves to workspace root",
                parents:  HashSet::from([AbsolutePath::from(tmp.path().join("rust"))]),
                event:    AbsolutePath::from(member_dir.join("src/lib.rs")),
                expected: AbsolutePath::from(workspace_root),
            },
        ];

        for case in cases {
            let result = project_level_dir(&case.event, &watch_roots, &case.parents);
            assert_eq!(result, Some(case.expected), "{}", case.name);
        }
    }

    // ── handle_event ─────────────────────────────────────────────────

    fn make_project_entry(project_label: &str, abs_path: &Path) -> (AbsolutePath, ProjectEntry) {
        (
            AbsolutePath::from(abs_path),
            ProjectEntry {
                project_label:  project_label.to_string(),
                abs_path:       AbsolutePath::from(abs_path),
                repo_root:      None,
                git_dir:        None,
                common_git_dir: None,
            },
        )
    }

    fn assert_pending_disk(states: &HashMap<String, DiskState>, project_path: &str) {
        assert!(matches!(
            states.get(project_path),
            Some(DiskState::Pending { .. })
        ));
    }

    fn event_with_path(path: &AbsolutePath) -> notify::Event {
        notify::Event {
            kind:  notify::event::EventKind::Any,
            paths: vec![path.to_path_buf()],
            attrs: notify::event::EventAttributes::default(),
        }
    }

    #[allow(
        clippy::type_complexity,
        reason = "test fixture returning multiple setup values"
    )]
    fn repo_with_member_event_context(
        tmp: &tempfile::TempDir,
    ) -> (
        AbsolutePath,
        AbsolutePath,
        HashMap<AbsolutePath, ProjectEntry>,
        Vec<AbsolutePath>,
        HashSet<AbsolutePath>,
        HashSet<AbsolutePath>,
    ) {
        let project_dir = tmp.path().join("my_project");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        init_git_repo(&project_dir);
        let member_dir = project_dir.join("crates").join("member");
        std::fs::create_dir_all(&member_dir).expect("create member dir");

        let mut projects = HashMap::new();
        projects.insert(
            AbsolutePath::from(project_dir.clone()),
            ProjectEntry {
                project_label:  "~/my_project".to_string(),
                abs_path:       AbsolutePath::from(project_dir.clone()),
                repo_root:      Some(AbsolutePath::from(project_dir.clone())),
                git_dir:        Some(AbsolutePath::from(project_dir.join(".git"))),
                common_git_dir: Some(AbsolutePath::from(project_dir.join(".git"))),
            },
        );
        projects.insert(
            AbsolutePath::from(member_dir.clone()),
            ProjectEntry {
                project_label:  "~/my_project/crates/member".to_string(),
                abs_path:       AbsolutePath::from(member_dir.clone()),
                repo_root:      Some(AbsolutePath::from(project_dir.clone())),
                git_dir:        Some(AbsolutePath::from(project_dir.join(".git"))),
                common_git_dir: Some(AbsolutePath::from(project_dir.join(".git"))),
            },
        );

        let watch_roots = vec![AbsolutePath::from(tmp.path())];
        let project_parents = HashSet::from([AbsolutePath::from(tmp.path())]);
        let discovered = HashSet::new();
        (
            AbsolutePath::from(project_dir),
            AbsolutePath::from(member_dir),
            projects,
            watch_roots,
            project_parents,
            discovered,
        )
    }

    fn assert_repo_git_fast_path(event_rel_path: &str, context: &str) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (project_dir, _, projects, watch_roots, project_parents, discovered) =
            repo_with_member_event_context(&tmp);
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
            &project_dir.join(event_rel_path),
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
            |msg| matches!(msg, BackgroundMsg::GitInfo { path, .. } if *path == *project_dir),
        );

        let mut got_root_git_info = false;
        for msg in &messages {
            if matches!(msg, BackgroundMsg::GitInfo { path, .. } if *path == *project_dir) {
                got_root_git_info = true;
            }
        }

        assert!(got_root_git_info, "{context}");
        assert!(pending_disk.is_empty(), "{context}");
        assert!(pending_git.contains_key(project_dir.as_path()), "{context}");
        assert!(pending_new.is_empty(), "{context}");
    }

    #[allow(
        clippy::type_complexity,
        reason = "test fixture returning multiple setup values"
    )]
    fn worktree_git_event_context(
        tmp: &tempfile::TempDir,
    ) -> (
        AbsolutePath,
        AbsolutePath,
        HashMap<AbsolutePath, ProjectEntry>,
        Vec<AbsolutePath>,
        HashSet<AbsolutePath>,
        HashSet<AbsolutePath>,
    ) {
        let wt_git_dir = tmp
            .path()
            .join("main_repo_git")
            .join("worktrees")
            .join("wt");
        std::fs::create_dir_all(&wt_git_dir).expect("create worktree git dir");
        let common_git_dir = tmp.path().join("main_repo_git");
        std::fs::create_dir_all(common_git_dir.join("refs").join("heads"))
            .expect("create common refs dir");

        let wt_root = tmp.path().join("main_repo_style_fix");
        std::fs::create_dir_all(&wt_root).expect("create worktree root");
        std::fs::write(
            wt_root.join(".git"),
            format!("gitdir: {}\n", wt_git_dir.display()),
        )
        .expect("write .git file");
        std::fs::write(wt_git_dir.join("commondir"), "../..").expect("write commondir");

        let mut projects = HashMap::new();
        projects.insert(
            AbsolutePath::from(wt_root.clone()),
            ProjectEntry {
                project_label:  "~/main_repo_style_fix".to_string(),
                abs_path:       AbsolutePath::from(wt_root.clone()),
                repo_root:      Some(AbsolutePath::from(wt_root.clone())),
                git_dir:        Some(AbsolutePath::from(wt_git_dir.clone())),
                common_git_dir: Some(AbsolutePath::from(common_git_dir)),
            },
        );
        let watch_roots = vec![AbsolutePath::from(tmp.path())];
        let project_parents = HashSet::from([AbsolutePath::from(tmp.path())]);
        let discovered = HashSet::new();
        (
            AbsolutePath::from(wt_root),
            AbsolutePath::from(wt_git_dir),
            projects,
            watch_roots,
            project_parents,
            discovered,
        )
    }

    #[test]
    fn known_project_event_goes_to_pending_disk() {
        let watch_roots = vec![AbsolutePath::from("/home/user".to_string())];
        let mut projects = HashMap::new();
        let (key, entry) = make_project_entry("~/rust/bevy", Path::new("/home/user/rust/bevy"));
        projects.insert(key, entry);
        let project_parents = HashSet::from(["/home/user/rust".into()]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
    fn tracked_file_edit_and_revert_refresh_git_path_state() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("demo");
        write_tracked_file(&project_dir, "fn main() {}\n");
        init_git_repo(&project_dir);

        let projects = tracked_file_projects(&project_dir);
        let watch_roots = vec![AbsolutePath::from(tmp.path())];
        let project_parents = HashSet::from([AbsolutePath::from(tmp.path())]);
        let discovered = HashSet::new();
        let ctx =
            tracked_file_event_context(&watch_roots, &projects, &project_parents, &discovered);
        let (bg_tx, bg_rx) = mpsc::channel();
        let (git_done_tx, git_done_rx) = mpsc::channel();
        let git_limit = Arc::new(tokio::sync::Semaphore::new(1));

        let run_refresh =
            |event_path: &Path,
             expected: GitPathState,
             pending_disk: &mut HashMap<String, DiskState>,
             pending_git: &mut HashMap<AbsolutePath, GitRefreshState>,
             pending_new: &mut HashMap<AbsolutePath, Instant>| {
                handle_event(
                    event_path,
                    &ctx,
                    &bg_tx,
                    pending_disk,
                    pending_git,
                    pending_new,
                );
                let past = Instant::now()
                    .checked_sub(Duration::from_secs(1))
                    .expect("1s subtraction should not underflow");
                let Some(GitRefreshState::Pending {
                    debounce_deadline,
                    max_deadline,
                    ..
                }) = pending_git.get_mut(project_dir.as_path())
                else {
                    panic!("expected pending git refresh for tracked file event");
                };
                *debounce_deadline = past;
                *max_deadline = past;
                fire_git_updates(
                    test_runtime().handle(),
                    &git_limit,
                    &git_done_tx,
                    &bg_tx,
                    &projects,
                    pending_git,
                );
                let messages = collect_messages_until(
                    &bg_rx,
                    |msg| matches!(msg, BackgroundMsg::GitInfo { path, .. } if *path == *project_dir),
                );
                let git_msg = messages
                    .into_iter()
                    .find_map(|msg| match msg {
                        BackgroundMsg::GitInfo { path, info }
                            if path.as_path() == project_dir.as_path() =>
                        {
                            Some(info)
                        },
                        _ => None,
                    })
                    .expect("git info message for project");
                assert_eq!(git_msg.path_state, expected);
                let repo_root = git_done_rx
                    .recv_timeout(Duration::from_secs(1))
                    .expect("git refresh completion");
                handle_git_completion(pending_git, repo_root);
            };

        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        write_tracked_file(&project_dir, "fn main() { println!(\"changed\"); }\n");
        run_refresh(
            &project_dir.join("src").join("main.rs"),
            Modified,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        write_tracked_file(&project_dir, "fn main() {}\n");
        run_refresh(
            &project_dir.join("src").join("main.rs"),
            Clean,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );
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
        let watch_roots = vec![AbsolutePath::from(project_root.path())];
        let project_parents = HashSet::new();
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
            crate::project::RootItem::Rust(crate::project::RustProject::Workspace(ws)) => {
                ws.cargo().examples().iter().map(|g| g.names.len()).sum()
            },
            crate::project::RootItem::Rust(crate::project::RustProject::Package(pkg)) => {
                pkg.cargo().examples().iter().map(|g| g.names.len()).sum()
            },
            _ => 0,
        };
        assert_eq!(example_count, 1);
        assert_pending_disk(&pending_disk, "~/rust/demo");
        assert!(pending_git.is_empty());
        assert!(pending_new.is_empty());
    }

    fn tracked_file_event_context<'a>(
        watch_roots: &'a [AbsolutePath],
        projects: &'a HashMap<AbsolutePath, ProjectEntry>,
        project_parents: &'a HashSet<AbsolutePath>,
        discovered: &'a HashSet<AbsolutePath>,
    ) -> EventContext<'a> {
        EventContext {
            watch_roots,
            projects,
            project_parents,
            discovered,
        }
    }

    fn tracked_file_projects(project_dir: &Path) -> HashMap<AbsolutePath, ProjectEntry> {
        let mut projects = HashMap::new();
        projects.insert(
            AbsolutePath::from(project_dir.to_path_buf()),
            ProjectEntry {
                project_label:  "~/demo".to_string(),
                abs_path:       AbsolutePath::from(project_dir.to_path_buf()),
                repo_root:      Some(AbsolutePath::from(project_dir.to_path_buf())),
                git_dir:        Some(AbsolutePath::from(project_dir.join(".git"))),
                common_git_dir: Some(AbsolutePath::from(project_dir.join(".git"))),
            },
        );
        projects
    }

    fn write_tracked_file(project_dir: &Path, contents: &str) {
        std::fs::create_dir_all(project_dir.join("src")).expect("create src");
        std::fs::write(project_dir.join("src").join("main.rs"), contents).expect("write main.rs");
    }

    #[test]
    fn git_exclude_event_refreshes_git_immediately() {
        assert_repo_git_fast_path(
            ".git/info/exclude",
            "exclude edits should bypass disk queue and keep the repo refresh queued for children",
        );
    }

    #[test]
    fn git_internal_noise_is_ignored() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("my_project");
        std::fs::create_dir_all(project_dir.join(".git").join("objects")).expect("create git dir");

        let mut projects = HashMap::new();
        projects.insert(
            AbsolutePath::from(project_dir.clone()),
            ProjectEntry {
                project_label:  "~/my_project".to_string(),
                abs_path:       AbsolutePath::from(project_dir.clone()),
                repo_root:      Some(AbsolutePath::from(project_dir.clone())),
                git_dir:        Some(AbsolutePath::from(project_dir.join(".git"))),
                common_git_dir: Some(AbsolutePath::from(project_dir.join(".git"))),
            },
        );
        let watch_roots = vec![AbsolutePath::from(tmp.path())];
        let project_parents = HashSet::from([AbsolutePath::from(tmp.path())]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
        assert_repo_git_fast_path(
            ".git/index",
            "index writes should refresh path state without a full GitInfo refresh",
        );
    }

    /// Worktree projects have `.git` as a file (not a directory) that
    /// points to a git dir elsewhere. Commit events fire under that
    /// real git dir, not under `repo_root/.git`. Verify the watcher
    /// recognises these events and enqueues a git refresh.
    #[test]
    fn worktree_index_event_enqueues_git_refresh() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (wt_root, wt_git_dir, projects, watch_roots, project_parents, discovered) =
            worktree_git_event_context(&tmp);
        std::fs::write(wt_git_dir.join("HEAD"), "ref: refs/heads/wt-branch\n").expect("write HEAD");
        std::fs::write(wt_git_dir.join("index"), "fake-index").expect("write index");
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
            pending_git.contains_key(wt_root.as_path()),
            "worktree index event should enqueue a git refresh for the worktree project"
        );
    }

    #[test]
    fn worktree_logs_head_event_enqueues_git_refresh() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (wt_root, wt_git_dir, projects, watch_roots, project_parents, discovered) =
            worktree_git_event_context(&tmp);
        let logs_head = wt_git_dir.join("logs").join("HEAD");
        std::fs::create_dir_all(logs_head.parent().expect("logs dir")).expect("create logs dir");
        std::fs::write(&logs_head, "old..new commit message\n").expect("write logs/HEAD");
        let ctx = EventContext {
            watch_roots:     &watch_roots,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, _bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            &logs_head,
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(
            pending_git.contains_key(wt_root.as_path()),
            "worktree logs/HEAD updates should enqueue a git refresh for the worktree project"
        );
    }

    #[test]
    fn worktree_noise_under_real_git_dir_is_ignored() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (_wt_root, wt_git_dir, projects, watch_roots, project_parents, discovered) =
            worktree_git_event_context(&tmp);
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
    fn worktree_common_branch_ref_event_enqueues_full_git_refresh() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (wt_root, _wt_git_dir, projects, watch_roots, project_parents, discovered) =
            worktree_git_event_context(&tmp);
        let common_git_dir = tmp.path().join("main_repo_git");
        let branch_ref = common_git_dir.join("refs").join("heads").join("wt-branch");
        std::fs::write(&branch_ref, "deadbeef\n").expect("write branch ref");
        let ctx = EventContext {
            watch_roots:     &watch_roots,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, _bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        handle_event(
            &branch_ref,
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(
            matches!(
                pending_git.get(wt_root.as_path()),
                Some(GitRefreshState::Pending {
                    refresh_scope: GitRefreshKind::FullMetadata,
                    ..
                })
            ),
            "shared branch ref writes should enqueue a full git refresh for linked worktrees"
        );
    }

    #[test]
    fn shared_common_git_dir_event_refreshes_all_projects() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let common_git_dir = tmp.path().join("main_repo").join(".git");
        std::fs::create_dir_all(common_git_dir.join("refs").join("heads"))
            .expect("create common refs dir");

        let main_root = tmp.path().join("main_repo");
        let wt_git_dir = common_git_dir.join("worktrees").join("style_fix");
        std::fs::create_dir_all(&wt_git_dir).expect("create worktree git dir");
        let wt_root = tmp.path().join("main_repo_style_fix");
        std::fs::create_dir_all(&wt_root).expect("create worktree root");

        let mut projects = HashMap::new();
        projects.insert(
            AbsolutePath::from(main_root.clone()),
            ProjectEntry {
                project_label:  "~/main_repo".to_string(),
                abs_path:       AbsolutePath::from(main_root.clone()),
                repo_root:      Some(AbsolutePath::from(main_root.clone())),
                git_dir:        Some(AbsolutePath::from(common_git_dir.clone())),
                common_git_dir: Some(AbsolutePath::from(common_git_dir.clone())),
            },
        );
        projects.insert(
            AbsolutePath::from(wt_root.clone()),
            ProjectEntry {
                project_label:  "~/main_repo_style_fix".to_string(),
                abs_path:       AbsolutePath::from(wt_root.clone()),
                repo_root:      Some(AbsolutePath::from(wt_root.clone())),
                git_dir:        Some(AbsolutePath::from(wt_git_dir)),
                common_git_dir: Some(AbsolutePath::from(common_git_dir.clone())),
            },
        );

        let watch_roots = vec![AbsolutePath::from(tmp.path())];
        let project_parents = HashSet::from([AbsolutePath::from(tmp.path())]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, _bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        let branch_ref = common_git_dir.join("refs").join("heads").join("style_fix");
        std::fs::write(&branch_ref, "deadbeef\n").expect("write branch ref");

        handle_event(
            &branch_ref,
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(
            pending_git.contains_key(main_root.as_path()),
            "main repo should be enqueued for git refresh"
        );
        assert!(
            pending_git.contains_key(wt_root.as_path()),
            "worktree should also be enqueued for git refresh"
        );
    }

    #[test]
    fn buffered_worktree_git_dir_event_replays_after_registration_complete() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (wt_root, wt_git_dir, projects, watch_roots, project_parents, discovered) =
            worktree_git_event_context(&tmp);
        let ctx = EventContext {
            watch_roots:     &watch_roots,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, _bg_rx) = mpsc::channel();
        let dispatch = WatcherDispatchContext {
            event: ctx,
            bg_tx: &bg_tx,
        };
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();
        let buffered = vec![event_with_path(&AbsolutePath::from(
            wt_git_dir.join("index"),
        ))];

        replay_buffered_events(
            &buffered,
            &dispatch,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(
            pending_git.contains_key(wt_root.as_path()),
            "buffered worktree git-dir events should replay through the normal classifier"
        );
        assert!(pending_new.is_empty());
    }

    #[test]
    fn buffered_worktree_common_git_event_replays_after_registration_complete() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (wt_root, _wt_git_dir, projects, watch_roots, project_parents, discovered) =
            worktree_git_event_context(&tmp);
        let common_git_dir = tmp.path().join("main_repo_git");
        let branch_ref = common_git_dir.join("refs").join("heads").join("wt-branch");
        std::fs::write(&branch_ref, "deadbeef\n").expect("write branch ref");
        let ctx = EventContext {
            watch_roots:     &watch_roots,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, _bg_rx) = mpsc::channel();
        let dispatch = WatcherDispatchContext {
            event: ctx,
            bg_tx: &bg_tx,
        };
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();
        let buffered = vec![event_with_path(&AbsolutePath::from(branch_ref))];

        replay_buffered_events(
            &buffered,
            &dispatch,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(
            matches!(
                pending_git.get(wt_root.as_path()),
                Some(GitRefreshState::Pending {
                    refresh_scope: GitRefreshKind::FullMetadata,
                    ..
                })
            ),
            "buffered common-git-dir events should still trigger the full metadata path"
        );
        assert!(pending_new.is_empty());
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

        let watch_roots = vec![AbsolutePath::from(project_root.path())];
        let project_parents = HashSet::new();
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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

        let watch_roots = vec![AbsolutePath::from(project_root.path())];
        let project_parents = HashSet::new();
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
        let base = tmp.path().canonicalize().expect("canonicalize tmpdir");

        // Create the new project directory (handle_event checks is_dir)
        let new_project = base.join("new_project");
        std::fs::create_dir_all(&new_project).expect("failed to create new_project dir");

        // Register an existing sibling so project_parents is populated
        let existing = base.join("existing_project");
        let mut projects = HashMap::new();
        let (key, entry) = make_project_entry("~/existing_project", &existing);
        projects.insert(key, entry);
        let watch_roots = vec![AbsolutePath::from(base.clone())];
        let project_parents = HashSet::from([AbsolutePath::from(base)]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
        assert!(pending_new.contains_key(new_project.as_path()));
    }

    #[test]
    fn replayed_event_for_already_registered_project_uses_known_project_path() {
        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let base = tmp.path().to_path_buf();
        let project_dir = base.join("existing_project");
        std::fs::create_dir_all(project_dir.join("src")).expect("create project dir");

        let mut projects = HashMap::new();
        let (key, entry) = make_project_entry("~/existing_project", &project_dir);
        projects.insert(key, entry);
        let watch_roots = vec![AbsolutePath::from(base.clone())];
        let project_parents = HashSet::from([AbsolutePath::from(base)]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, _bg_rx) = mpsc::channel();
        let dispatch = WatcherDispatchContext {
            event: ctx,
            bg_tx: &bg_tx,
        };
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();
        let buffered = vec![event_with_path(&AbsolutePath::from(
            project_dir.join("src").join("lib.rs"),
        ))];

        replay_buffered_events(
            &buffered,
            &dispatch,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert_pending_disk(&pending_disk, "~/existing_project");
        assert!(pending_new.is_empty());
    }

    #[test]
    fn already_discovered_directory_not_re_enqueued() {
        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let base = tmp.path().canonicalize().expect("canonicalize tmpdir");

        let project_dir = base.join("my_project");
        std::fs::create_dir_all(&project_dir).expect("failed to create project dir");

        let projects = HashMap::new();
        let watch_roots = vec![AbsolutePath::from(base.clone())];
        let project_parents = HashSet::from([AbsolutePath::from(base)]);
        let discovered = HashSet::from([AbsolutePath::from(project_dir.clone())]);
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
        let base = tmp.path().canonicalize().expect("canonicalize tmpdir");

        // ~/rust/new_wt — two levels below scan root, no siblings registered
        let new_wt = base.join("rust").join("new_wt");
        std::fs::create_dir_all(&new_wt).expect("create dirs");
        std::fs::write(new_wt.join("Cargo.toml"), b"[package]").expect("write Cargo.toml");

        let projects = HashMap::new();
        let watch_roots = vec![AbsolutePath::from(base)];
        let project_parents = HashSet::new(); // empty — early scan
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
            pending_new.contains_key(new_wt.as_path()),
            "expected pending_new to contain {}, got: {:?}",
            new_wt.display(),
            pending_new.keys().collect::<Vec<_>>()
        );
    }

    // ── resolve_include_dirs ────────────────────────────────────────

    #[test]
    fn resolve_include_dirs_cases() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/user"));
        let cases: Vec<(&str, Vec<String>, Vec<AbsolutePath>)> = vec![
            ("empty_returns_empty", Vec::<String>::new(), vec![]),
            (
                "relative_joins_to_home",
                vec!["rust".to_string(), ".claude".to_string()],
                vec![
                    AbsolutePath::from(home.join("rust")),
                    AbsolutePath::from(home.join(".claude")),
                ],
            ),
            (
                "tilde_expands_to_home",
                vec!["~/rust".to_string(), "~/.claude".to_string()],
                vec![
                    AbsolutePath::from(home.join("rust")),
                    AbsolutePath::from(home.join(".claude")),
                ],
            ),
            (
                "absolute_used_as_is",
                vec!["/opt/projects".to_string()],
                vec!["/opt/projects".into()],
            ),
        ];

        for (name, include_dirs, expected) in cases {
            let dirs = scan::resolve_include_dirs(&include_dirs);
            assert_eq!(dirs, expected, "{name}");
        }
    }

    #[test]
    fn register_watch_roots_reports_elapsed_for_representative_roots() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let rust_root = tmp.path().join("rust");
        let claude_root = tmp.path().join(".claude");
        std::fs::create_dir_all(&rust_root).expect("create rust root");
        std::fs::create_dir_all(&claude_root).expect("create claude root");
        let watch_dirs = vec![
            AbsolutePath::from(rust_root),
            AbsolutePath::from(claude_root),
        ];
        let (notify_tx, _notify_rx) = mpsc::channel();
        let handler = move |res| {
            let _ = notify_tx.send(res);
        };
        let mut watcher = notify::recommended_watcher(handler).expect("recommended watcher");
        let started = Instant::now();

        register_watch_roots(&mut watcher, &watch_dirs);

        eprintln!(
            "register_watch_roots_elapsed_ms={}",
            crate::perf_log::ms(started.elapsed().as_millis())
        );
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
            AbsolutePath::from(project_dir.clone()),
            ProjectEntry {
                project_label:  "~/my_project".to_string(),
                abs_path:       AbsolutePath::from(project_dir.clone()),
                repo_root:      Some(AbsolutePath::from(project_dir.clone())),
                git_dir:        Some(AbsolutePath::from(project_dir.join(".git"))),
                common_git_dir: Some(AbsolutePath::from(project_dir.join(".git"))),
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
                BackgroundMsg::DiskUsage { path, .. } if *path == *project_dir => {
                    got_disk = true;
                },
                BackgroundMsg::GitInfo { path, .. } if *path == *project_dir => {
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
        projects.insert(
            AbsolutePath::from(project_dir.clone()),
            ProjectEntry {
                project_label:  "~/no_git".to_string(),
                abs_path:       AbsolutePath::from(project_dir.clone()),
                repo_root:      None,
                git_dir:        None,
                common_git_dir: None,
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
                BackgroundMsg::DiskUsage { path, .. } if *path == *project_dir => {
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
        let mut pending_new = HashMap::from([(AbsolutePath::from(linked_dir.clone()), past)]);
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
        let RootItem::Rust(crate::project::RustProject::Package(pkg)) = item else {
            panic!("expected package worktree item");
        };
        assert_eq!(pkg.path(), linked_dir.as_path());
        assert_eq!(pkg.worktree_name(), Some("app_test"));
        let canonical = crate::project::AbsolutePath::from(
            primary_dir.canonicalize().expect("canonical primary"),
        );
        assert_eq!(pkg.worktree_primary_abs_path(), Some(&canonical));
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
        let mut pending_new = HashMap::from([(AbsolutePath::from(linked_dir.clone()), past)]);
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
        let RootItem::Rust(crate::project::RustProject::Workspace(ws)) = item else {
            panic!("expected workspace worktree item");
        };
        assert_eq!(ws.path(), linked_dir.as_path());
        assert_eq!(ws.worktree_name(), Some("obsidian_knife_test"));
        let canonical = crate::project::AbsolutePath::from(
            primary_dir.canonicalize().expect("canonical primary"),
        );
        assert_eq!(ws.worktree_primary_abs_path(), Some(&canonical));
    }

    #[test]
    fn project_refresh_normalizes_workspace_members() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("bevy_brp");
        let member_dir = project_dir.join("extras");

        std::fs::create_dir_all(member_dir.join("src")).expect("create member src");
        std::fs::write(
            project_dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"extras\"]\n",
        )
        .expect("write workspace manifest");
        std::fs::write(
            member_dir.join("Cargo.toml"),
            "[package]\nname = \"extras\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write member manifest");
        std::fs::write(member_dir.join("src").join("lib.rs"), "pub fn demo() {}\n")
            .expect("write member lib");

        let (bg_tx, bg_rx) = mpsc::channel();
        spawn_project_refresh_after(bg_tx, AbsolutePath::from(project_dir), Duration::ZERO);

        let BackgroundMsg::ProjectRefreshed { item } = bg_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("project refreshed message")
        else {
            panic!("unexpected message");
        };
        let RootItem::Rust(crate::project::RustProject::Workspace(ws)) = item else {
            panic!("expected normalized workspace refresh");
        };
        assert!(
            ws.has_members(),
            "workspace refresh should rebuild member groups, not emit a flat workspace"
        );
        assert_eq!(ws.groups()[0].members()[0].path(), member_dir.as_path());
    }

    #[test]
    fn project_refresh_emits_disk_usage_for_workspace_members() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project_dir = tmp.path().join("bevy_brp");
        let member_dir = project_dir.join("extras");

        std::fs::create_dir_all(member_dir.join("src")).expect("create member src");
        std::fs::write(
            project_dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"extras\"]\n",
        )
        .expect("write workspace manifest");
        std::fs::write(
            member_dir.join("Cargo.toml"),
            "[package]\nname = \"extras\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write member manifest");
        std::fs::write(member_dir.join("src").join("lib.rs"), "pub fn demo() {}\n")
            .expect("write member lib");

        let (bg_tx, bg_rx) = mpsc::channel();
        spawn_project_refresh_after(bg_tx, AbsolutePath::from(project_dir), Duration::ZERO);

        let _ = bg_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("project refreshed message");
        let BackgroundMsg::DiskUsageBatch { entries, .. } = bg_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("disk usage batch message")
        else {
            panic!("expected disk usage batch");
        };

        let member_bytes = entries
            .iter()
            .find(|(path, _)| **path == *member_dir)
            .map(|(_, bytes)| *bytes)
            .expect("member disk usage entry");
        assert!(
            member_bytes > 0,
            "workspace member should receive a non-zero disk usage entry"
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
            AbsolutePath::from(linked_dir.clone()),
            ProjectEntry {
                project_label:  "~/app_test".to_string(),
                abs_path:       AbsolutePath::from(linked_dir.clone()),
                repo_root:      None,
                git_dir:        None,
                common_git_dir: None,
            },
        );
        let watch_roots = vec![AbsolutePath::from(tmp.path())];
        let project_parents = HashSet::from([AbsolutePath::from(tmp.path())]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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
            AbsolutePath::from(linked_dir.clone()),
            ProjectEntry {
                project_label:  "~/obsidian_knife_test".to_string(),
                abs_path:       AbsolutePath::from(linked_dir.clone()),
                repo_root:      None,
                git_dir:        None,
                common_git_dir: None,
            },
        );
        let watch_roots = vec![AbsolutePath::from(tmp.path())];
        let project_parents = HashSet::from([AbsolutePath::from(tmp.path())]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
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

    /// When notify delivers an event via a symlinked path, the candidate
    /// should be canonicalized so it matches the real path in `discovered`.
    #[test]
    fn symlinked_event_path_canonicalizes_to_real_project() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let real_dir = tmp.path().join("real_project");
        std::fs::create_dir_all(&real_dir).expect("create real dir");
        std::fs::write(real_dir.join("Cargo.toml"), "[package]\nname = \"real\"").expect("write");

        let link_parent = tmp.path().join("links");
        std::fs::create_dir_all(&link_parent).expect("create link parent");
        std::os::unix::fs::symlink(&real_dir, link_parent.join("linked_project"))
            .expect("create symlink");

        let watch_roots = vec![AbsolutePath::from(tmp.path())];
        let project_parents = HashSet::from([AbsolutePath::from(tmp.path())]);
        // Mark the real (canonical) path as already discovered.
        let canonical = real_dir.canonicalize().expect("canonicalize");
        let discovered = HashSet::from([AbsolutePath::from(canonical)]);
        let projects = HashMap::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };
        let (bg_tx, _bg_rx) = mpsc::channel();
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();

        // Fire an event through the symlink path.
        handle_event(
            &link_parent
                .join("linked_project")
                .join("src")
                .join("lib.rs"),
            &ctx,
            &bg_tx,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        assert!(
            pending_new.is_empty(),
            "symlinked path should canonicalize and match discovered project, \
             but got: {pending_new:?}"
        );
    }
}
