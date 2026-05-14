//! Watches the scan root recursively for filesystem changes and maps
//! events to discovered projects for disk-usage and git-sync updates.
//!
//! A single `notify` subscription covers the entire scan root. Events are
//! matched to projects by prefix, debounced, and result in both
//! `BackgroundMsg::DiskUsage` and `BackgroundMsg::CheckoutInfo` / `BackgroundMsg::RepoInfo`
//! updates. New project directories are detected automatically; removed directories trigger a
//! zero-byte update so the app can mark them as deleted.
//!
//! On macOS (`FSEvents`) this is a small fixed set of kernel subscriptions
//! regardless of tree size: one for the scan roots. Linux / Windows may want
//! a different approach in the future to avoid inotify watch limits.

mod events;
mod probe;
mod refresh;
mod roots;

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::time::Instant;

use events::WatcherBackgroundSinks;
use events::drain_completed_refreshes;
use events::drain_notify_events;
use events::process_notify_events;
use notify::Event;
use notify::Watcher;
use probe::probe_new_projects;
use refresh::fire_disk_updates;
use refresh::fire_git_updates;
use roots::RegisteredRoots;
use roots::register_cargo_home_watch;
use roots::register_watch_roots;

use super::config::NonRustInclusion;
use super::constants::DEBOUNCE_DURATION;
use super::constants::MAX_WAIT;
use super::constants::POLL_INTERVAL;
use super::constants::WATCHER_DISK_CONCURRENCY;
use super::constants::WATCHER_GIT_CONCURRENCY;
use super::http::HttpClient;
use super::lint::RuntimeHandle;
use super::project;
#[cfg(test)]
use super::project::ProjectFields;
use super::scan::BackgroundMsg;
use super::scan::MetadataDispatchContext;
use crate::constants::SCAN_METADATA_CONCURRENCY;
use crate::project::AbsolutePath;
use crate::project::WorkspaceMetadataStore;

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
// Ancestor `.cargo/` watch-set subsystem is not yet implemented.
// Today we only refresh cargo metadata when a `Cargo.toml` /
// `Cargo.lock` / `rust-toolchain[.toml]` / `.cargo/config[.toml]`
// edit fires inside an already-registered project tree. Edits to
// an out-of-tree ancestor `.cargo/config.toml` (e.g.
// `~/.cargo/config.toml` when the project is elsewhere) will go
// undetected until the subsystem lands.
// The missing piece is: walk each project root → CARGO_HOME at
// register time, collect the ancestor `.cargo/` dirs, diff the union
// across projects on add/remove, and register notify watches on the
// diff. Tracked for Step 1b follow-up.
pub(crate) fn spawn_watcher(
    watch_roots: &[AbsolutePath],
    bg_tx: Sender<BackgroundMsg>,
    ci_run_count: u32,
    non_rust: NonRustInclusion,
    client: HttpClient,
    lint_runtime: Option<RuntimeHandle>,
    metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
) -> Sender<WatcherMsg> {
    let (watch_tx, watch_rx) = mpsc::channel();
    let (notify_tx, notify_rx) = mpsc::channel();
    let handler = move |res| {
        let _ = notify_tx.send(res);
    };
    let Ok(mut watcher) = notify::recommended_watcher(handler) else {
        return watch_tx;
    };
    let started = Instant::now();
    let (registered_roots, failures) = register_watch_roots(&mut watcher, watch_roots);
    for failure in &failures {
        tracing::error!(
            dir = %failure.dir.display(),
            reason = %failure.reason,
            "watcher_root_registration_failed"
        );
    }
    tracing::info!(
        requested = watch_roots.len(),
        registered = registered_roots.dirs().len(),
        failed = failures.len(),
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        "watcher_root_registration_complete"
    );
    register_cargo_home_watch(&mut watcher, &registered_roots);
    let metadata_dispatch = MetadataDispatchContext {
        handle: client.handle.clone(),
        tx: bg_tx.clone(),
        metadata_store,
        metadata_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_METADATA_CONCURRENCY)),
    };
    let ctx = WatcherLoopContext {
        watch_roots: registered_roots,
        bg_tx,
        ci_run_count,
        non_rust,
        client,
        lint_runtime,
        metadata_dispatch,
    };

    spawn_watcher_thread(ctx, watch_rx, notify_rx, watcher);

    watch_tx
}

struct WatcherLoopContext {
    watch_roots:       RegisteredRoots,
    bg_tx:             Sender<BackgroundMsg>,
    ci_run_count:      u32,
    non_rust:          NonRustInclusion,
    client:            HttpClient,
    lint_runtime:      Option<RuntimeHandle>,
    metadata_dispatch: MetadataDispatchContext,
}

fn spawn_watcher_thread<W: Watcher + Send + 'static>(
    ctx: WatcherLoopContext,
    watch_rx: Receiver<WatcherMsg>,
    notify_rx: Receiver<notify::Result<Event>>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchState {
    Idle,
    Pending {
        debounce_deadline: Instant,
        max_deadline:      Instant,
    },
    Running,
    RunningDirty,
}

impl WatchState {
    fn pending(now: Instant, immediate: bool) -> Self {
        Self::Pending {
            debounce_deadline: if immediate {
                now
            } else {
                now + DEBOUNCE_DURATION
            },
            max_deadline:      now + MAX_WAIT,
        }
    }
}

struct WatcherLoopState {
    projects:        HashMap<AbsolutePath, ProjectEntry>,
    project_parents: HashSet<AbsolutePath>,
    pending_disk:    HashMap<String, WatchState>,
    pending_git:     HashMap<AbsolutePath, WatchState>,
    pending_new:     HashMap<AbsolutePath, Instant>,
    discovered:      HashSet<AbsolutePath>,
    initializing:    bool,
    buffered_events: Vec<Event>,
}

impl WatcherLoopState {
    fn new() -> Self {
        Self {
            projects:        HashMap::new(),
            project_parents: HashSet::new(),
            pending_disk:    HashMap::new(),
            pending_git:     HashMap::new(),
            pending_new:     HashMap::new(),
            discovered:      HashSet::new(),
            initializing:    true,
            buffered_events: Vec::new(),
        }
    }
}

fn watcher_loop<W: Watcher + Send + 'static>(
    ctx: &WatcherLoopContext,
    watch_rx: &Receiver<WatcherMsg>,
    notify_rx: &Receiver<notify::Result<Event>>,
    mut watcher: W,
) {
    let WatcherLoopContext {
        watch_roots,
        bg_tx,
        ci_run_count,
        non_rust,
        client,
        lint_runtime: _,
        metadata_dispatch,
    } = ctx;
    let mut state = WatcherLoopState::new();
    let (disk_done_tx, disk_done_rx) = mpsc::channel::<String>();
    let (git_done_tx, git_done_rx) = mpsc::channel::<AbsolutePath>();
    let disk_limit = Arc::new(tokio::sync::Semaphore::new(WATCHER_DISK_CONCURRENCY));
    let git_limit = Arc::new(tokio::sync::Semaphore::new(WATCHER_GIT_CONCURRENCY));

    let mut tick: u64 = 0;
    loop {
        tick += 1;
        let watch_drain = drain_watch_messages(watch_rx, &mut state, &mut watcher);
        if watch_drain.disconnected {
            tracing::info!(tick, "watcher_loop_exit_disconnected");
            return;
        }

        let notify_events = drain_notify_events(notify_rx);
        process_notify_events(
            tick,
            &watch_drain,
            notify_events,
            watch_roots.dirs(),
            &WatcherBackgroundSinks {
                bg_tx,
                lint_runtime: ctx.lint_runtime.as_ref(),
                metadata_dispatch: Some(metadata_dispatch),
            },
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

pub(super) struct WatchDrainResult {
    pub(super) disconnected:           bool,
    pub(super) registration_completed: bool,
}

fn drain_watch_messages(
    watch_rx: &Receiver<WatcherMsg>,
    state: &mut WatcherLoopState,
    _watcher: &mut impl Watcher,
) -> WatchDrainResult {
    let mut result = WatchDrainResult {
        disconnected:           false,
        registration_completed: false,
    };
    loop {
        match watch_rx.try_recv() {
            Ok(WatcherMsg::Register(req)) => {
                apply_watch_request(req, state);
            },
            Ok(WatcherMsg::InitialRegistrationComplete) => {
                state.initializing = false;
                result.registration_completed = true;
            },
            Err(TryRecvError::Empty) => return result,
            Err(TryRecvError::Disconnected) => {
                result.disconnected = true;
                return result;
            },
        }
    }
}

fn apply_watch_request(req: WatchRequest, state: &mut WatcherLoopState) {
    if let Some(parent) = req.abs_path.parent() {
        state.project_parents.insert(AbsolutePath::from(parent));
    }
    let git_dir = req.repo_root.as_deref().and_then(project::resolve_git_dir);
    let common_git_dir = req
        .repo_root
        .as_deref()
        .and_then(project::resolve_common_git_dir);
    state.projects.insert(
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

/// Background sinks the watcher fans events out to. Bundled so
/// `process_notify_events` stays under the clippy `too_many_arguments`
/// threshold as more dispatch targets get added.
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
mod tests;
