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

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use notify::Error;
use notify::Event;
use notify::RecursiveMode;
use notify::Watcher;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;

use super::config::NonRustInclusion;
use super::constants::DEBOUNCE_DURATION;
use super::constants::MAX_WAIT;
use super::constants::NEW_PROJECT_DEBOUNCE;
use super::constants::POLL_INTERVAL;
use super::constants::WATCHER_DISK_CONCURRENCY;
use super::constants::WATCHER_GIT_CONCURRENCY;
use super::http::HttpClient;
use super::lint;
use super::lint::RuntimeHandle;
use super::project;
use super::project::CheckoutInfo;
use super::project::GitRepoPresence;
#[cfg(test)]
use super::project::ProjectFields;
use super::project::RepoInfo;
use super::scan;
use super::scan::BackgroundMsg;
use super::scan::MetadataDispatchContext;
use crate::constants::SCAN_METADATA_CONCURRENCY;
use crate::enrichment;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::project::RootItem::NonRust;
use crate::project::WorkspaceMetadataStore;
use crate::scan::FetchContext;
use crate::scan::ProjectDetailRequest;

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

/// Witness that every advertised watch root has been successfully
/// registered with the underlying [`Watcher`]. The only way to
/// construct one is through [`register_watch_roots`], which produces
/// it alongside any per-root failures. Downstream code that depends on
/// "we are watching exactly these roots" takes a `&RegisteredRoots`
/// instead of a `&[AbsolutePath]`, so the previously-representable
/// state where the watcher loop runs but silently dropped a watch root
/// is no longer constructible.
pub(crate) struct RegisteredRoots {
    dirs: Vec<AbsolutePath>,
}

impl RegisteredRoots {
    pub(crate) fn dirs(&self) -> &[AbsolutePath] { &self.dirs }

    /// True when `path` is equal to or descends from any registered
    /// root. Used to suppress redundant per-project ancestor watches
    /// that would re-register an already-recursively-watched dir as
    /// `NonRecursive` — on macOS `FSEvents` this changes the mode for
    /// the path and silently drops subsequent recursive events.
    pub(crate) fn covers(&self, path: &Path) -> bool {
        self.dirs.iter().any(|root| path.starts_with(root))
    }
}

impl Default for RegisteredRoots {
    /// An empty registered set — trivially consistent (we are
    /// watching nothing, and we claim to be watching nothing). Used
    /// by tests that exercise watcher logic without exercising
    /// registration.
    fn default() -> Self { Self { dirs: Vec::new() } }
}

pub(crate) struct WatchRootRegistrationFailure {
    pub(crate) dir:    AbsolutePath,
    pub(crate) reason: WatchRootRegistrationFailureReason,
}

pub(crate) enum WatchRootRegistrationFailureReason {
    NotADirectory,
    Notify(Error),
}

impl Display for WatchRootRegistrationFailureReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotADirectory => f.write_str("path is not a directory"),
            Self::Notify(err) => write!(f, "notify watch failed: {err}"),
        }
    }
}

/// Try to register every entry in `watch_dirs` with the underlying
/// [`Watcher`]. Returns the witness for the subset that succeeded plus
/// the per-root failures. The caller must visibly handle the failures
/// (the reason this function returns a tuple instead of `Result` is
/// that running with a partial root set is still better than not
/// running at all — but the caller cannot pretend the failures don't
/// exist, because they are returned by value).
fn register_watch_roots(
    watcher: &mut impl Watcher,
    watch_dirs: &[AbsolutePath],
) -> (RegisteredRoots, Vec<WatchRootRegistrationFailure>) {
    let mut registered = Vec::with_capacity(watch_dirs.len());
    let mut failures = Vec::new();
    for dir in watch_dirs {
        if !dir.is_dir() {
            failures.push(WatchRootRegistrationFailure {
                dir:    dir.clone(),
                reason: WatchRootRegistrationFailureReason::NotADirectory,
            });
            continue;
        }
        match watcher.watch(dir, RecursiveMode::Recursive) {
            Ok(()) => registered.push(dir.clone()),
            Err(err) => failures.push(WatchRootRegistrationFailure {
                dir:    dir.clone(),
                reason: WatchRootRegistrationFailureReason::Notify(err),
            }),
        }
    }
    (RegisteredRoots { dirs: registered }, failures)
}

/// Resolve `$CARGO_HOME` (falling back to `~/.cargo`).
fn resolve_cargo_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CARGO_HOME")
        && !home.is_empty()
    {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|home| home.join(".cargo"))
}

/// Subscribe to the cargo home directory (`$CARGO_HOME` or
/// `~/.cargo`) so edits to `~/.cargo/config.toml` reach the watcher
/// even when the user's recursive `include_dirs` don't cover it.
/// Skipped when the cargo home is already inside one of the recursive
/// roots — registering it again as `NonRecursive` would clobber the
/// recursive subscription on macOS `FSEvents`.
fn register_cargo_home_watch(watcher: &mut impl Watcher, registered_roots: &RegisteredRoots) {
    let Some(cargo_home) = resolve_cargo_home() else {
        return;
    };
    if !cargo_home.is_dir() {
        return;
    }
    if registered_roots.covers(cargo_home.as_path()) {
        return;
    }
    match watcher.watch(cargo_home.as_path(), RecursiveMode::NonRecursive) {
        Ok(()) => tracing::info!(
            cargo_home = %cargo_home.display(),
            "watcher_cargo_home_registered"
        ),
        Err(err) => tracing::error!(
            cargo_home = %cargo_home.display(),
            error = %err,
            "watcher_cargo_home_registration_failed"
        ),
    }
}

struct WatchDrainResult {
    disconnected:           bool,
    registration_completed: bool,
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
struct WatcherBackgroundSinks<'a> {
    bg_tx:             &'a Sender<BackgroundMsg>,
    lint_runtime:      Option<&'a RuntimeHandle>,
    metadata_dispatch: Option<&'a MetadataDispatchContext>,
}

fn process_notify_events(
    tick: u64,
    watch_drain: &WatchDrainResult,
    notify_events: Vec<Event>,
    watch_roots: &[AbsolutePath],
    sinks: &WatcherBackgroundSinks<'_>,
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
            event:             EventContext {
                watch_roots,
                projects: &state.projects,
                project_parents: &state.project_parents,
                discovered: &state.discovered,
            },
            bg_tx:             sinks.bg_tx,
            lint_runtime:      sinks.lint_runtime,
            metadata_dispatch: sinks.metadata_dispatch,
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
            event:             EventContext {
                watch_roots,
                projects: &state.projects,
                project_parents: &state.project_parents,
                discovered: &state.discovered,
            },
            bg_tx:             sinks.bg_tx,
            lint_runtime:      sinks.lint_runtime,
            metadata_dispatch: sinks.metadata_dispatch,
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

fn drain_notify_events(notify_rx: &Receiver<notify::Result<Event>>) -> Vec<Event> {
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
    events: &[Event],
    ctx: &WatcherDispatchContext<'_>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
) {
    for event in events {
        for event_path in &event.paths {
            handle_notify_event(
                event_path,
                Some(event),
                &ctx.event,
                ctx.bg_tx,
                ctx.lint_runtime,
                ctx.metadata_dispatch,
                pending_disk,
                pending_git,
                pending_new,
            );
        }
    }
}

fn drain_completed_refreshes(
    disk_done_rx: &Receiver<String>,
    git_done_rx: &Receiver<AbsolutePath>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
) {
    while let Ok(project_path) = disk_done_rx.try_recv() {
        handle_disk_completion(pending_disk, &project_path);
    }

    while let Ok(repo_root) = git_done_rx.try_recv() {
        handle_git_completion(pending_git, &repo_root);
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
    event:             EventContext<'a>,
    bg_tx:             &'a Sender<BackgroundMsg>,
    lint_runtime:      Option<&'a RuntimeHandle>,
    /// `None` in test harnesses that do not provide a tokio runtime;
    /// disables the metadata refresh branch rather than panicking.
    metadata_dispatch: Option<&'a MetadataDispatchContext>,
}

#[allow(
    clippy::too_many_arguments,
    reason = "watcher dispatch needs the raw event plus debounce maps and background contexts"
)]
fn handle_notify_event(
    event_path: &Path,
    event: Option<&Event>,
    ctx: &EventContext<'_>,
    bg_tx: &Sender<BackgroundMsg>,
    lint_runtime: Option<&RuntimeHandle>,
    metadata_dispatch: Option<&MetadataDispatchContext>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
) {
    let now = Instant::now();

    try_dispatch_out_of_tree_cargo_config_refresh(event_path, ctx, metadata_dispatch);

    let mut matched_fast_git = false;
    for entry in ctx.projects.values() {
        if is_fast_git_refresh_event(event_path, entry)
            && let Some(refresh_key) = git_refresh_key(entry)
        {
            matched_fast_git = true;
            tracing::info!(
                refresh_key = %refresh_key.display(),
                event_path = %event_path.display(),
                "watcher_fast_git_event"
            );
            emit_root_git_info_refresh(bg_tx, entry);
            enqueue_git_refresh(pending_git, refresh_key, now, false, "fast_git");
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
        if let Some(lint_runtime) = lint_runtime
            && let Some(event) = event
            && let Some(lint_trigger) =
                lint::classify_event_path(&entry.abs_path, event.kind, event_path)
        {
            lint_runtime.lint_trigger(lint_trigger);
        }
        if let Some(dispatch) = metadata_dispatch
            && let Some(kind) =
                lint::classify_cargo_metadata_event_path(entry.abs_path.as_path(), event_path)
        {
            tracing::info!(
                workspace_root = %entry.abs_path.display(),
                event_path = %event_path.display(),
                ?kind,
                "watcher_cargo_metadata_refresh"
            );
            scan::spawn_cargo_metadata_refresh(dispatch.clone(), entry.abs_path.clone());
        }
        if is_target_metadata_event(event_path, entry.abs_path.as_path()) {
            spawn_project_refresh(bg_tx.clone(), entry.abs_path.clone());
        }
        if is_internal_git_path(event_path, entry) {
            if let Some(refresh_key) = git_refresh_key(entry)
                && is_internal_git_refresh_event(event_path, entry)
            {
                enqueue_git_refresh(pending_git, refresh_key, now, false, "git_internal");
            }
            return;
        }
        let resolved_target =
            metadata_dispatch.and_then(|dispatch| dispatch.resolved_target_dir(&entry.abs_path));
        let is_target_event = is_target_event_for(
            event_path,
            entry.abs_path.as_path(),
            resolved_target.as_deref(),
        );
        schedule_disk_refresh(pending_disk, &entry.project_label, now);
        if !is_target_event && let Some(refresh_key) = git_refresh_key(entry) {
            enqueue_git_refresh(pending_git, refresh_key, now, false, "project_event");
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

#[cfg(test)]
fn handle_event(
    event_path: &Path,
    ctx: &EventContext<'_>,
    bg_tx: &Sender<BackgroundMsg>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
) {
    // Tests skip the metadata-refresh branch; no tokio runtime is
    // provided so the `None` arm is the safe default.
    handle_notify_event(
        event_path,
        None,
        ctx,
        bg_tx,
        None,
        None,
        pending_disk,
        pending_git,
        pending_new,
    );
}

fn schedule_disk_refresh(
    pending_disk: &mut HashMap<String, WatchState>,
    project_label: &str,
    now: Instant,
) {
    match pending_disk
        .entry(project_label.to_string())
        .or_insert(WatchState::Idle)
    {
        state @ WatchState::Idle => {
            *state = WatchState::pending(now, false);
        },
        WatchState::Pending {
            debounce_deadline, ..
        } => {
            *debounce_deadline = now + DEBOUNCE_DURATION;
        },
        state @ WatchState::Running => *state = WatchState::RunningDirty,
        WatchState::RunningDirty => {},
    }
}

fn handle_disk_completion(pending_disk: &mut HashMap<String, WatchState>, project_label: &str) {
    let now = Instant::now();
    let Some(state) = pending_disk.get_mut(project_label) else {
        return;
    };
    *state = match *state {
        WatchState::Running => WatchState::Idle,
        WatchState::RunningDirty => WatchState::pending(now, false),
        WatchState::Idle | WatchState::Pending { .. } => return,
    };
}

fn handle_git_completion(
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    repo_root: &AbsolutePath,
) {
    let now = Instant::now();
    let Some(state) = pending_git.get_mut(repo_root) else {
        return;
    };
    *state = match *state {
        WatchState::Running => WatchState::Idle,
        WatchState::RunningDirty => WatchState::pending(now, false),
        WatchState::Idle | WatchState::Pending { .. } => return,
    };
}

fn is_fast_git_refresh_event(event_path: &Path, entry: &ProjectEntry) -> bool {
    let Some(repo_root) = entry.repo_root.as_deref() else {
        return false;
    };
    let Some(git_dir) = entry.git_dir.as_deref() else {
        return false;
    };
    let Some(common_git_dir) = entry.common_git_dir.as_deref() else {
        return false;
    };
    event_path == repo_root.join(".gitignore")
        || event_path == git_dir.join("index")
        || event_path == git_dir.join("info").join("exclude")
        || event_path == git_dir.join("HEAD")
        || event_path == common_git_dir.join("packed-refs")
        || event_path.starts_with(common_git_dir.join("refs").join("heads"))
        || event_path.starts_with(common_git_dir.join("refs").join("remotes"))
        || is_worktree_git_fallback_event(event_path, git_dir)
}

fn is_internal_git_refresh_event(event_path: &Path, entry: &ProjectEntry) -> bool {
    let Some(git_dir) = entry.git_dir.as_deref() else {
        return false;
    };
    let Some(common_git_dir) = entry.common_git_dir.as_deref() else {
        return false;
    };
    let Some(repo_root) = entry.repo_root.as_deref() else {
        return false;
    };
    event_path == repo_root.join(".gitignore")
        || event_path == git_dir.join("index")
        || event_path == git_dir.join("index.lock")
        || event_path == git_dir.join("info").join("exclude")
        || event_path == git_dir.join("HEAD")
        || event_path == git_dir.join("FETCH_HEAD")
        || event_path == git_dir.join("ORIG_HEAD")
        || event_path == git_dir.join("config")
        || event_path == git_dir.join("packed-refs")
        || event_path.starts_with(git_dir.join("refs").join("heads"))
        || event_path.starts_with(git_dir.join("refs").join("remotes"))
        || event_path == common_git_dir.join("packed-refs")
        || event_path.starts_with(common_git_dir.join("refs").join("heads"))
        || event_path.starts_with(common_git_dir.join("refs").join("remotes"))
        || is_worktree_git_fallback_event(event_path, git_dir)
}

fn is_worktree_git_fallback_event(event_path: &Path, git_dir: &Path) -> bool {
    let logs_dir = git_dir.join("logs");
    event_path == git_dir || event_path == logs_dir || event_path.starts_with(&logs_dir)
}

/// Key used to dedup git refreshes in `pending_git`. Prefers the
/// shared `common_git_dir` so primary + linked worktrees of the same
/// repo collapse into a single pending refresh; falls back to the
/// per-entry `repo_root` when the common-git-dir lookup is missing
/// (degenerate case — e.g. a worktree whose `.git` file points at a
/// path we couldn't resolve).
fn git_refresh_key(entry: &ProjectEntry) -> Option<AbsolutePath> {
    entry
        .common_git_dir
        .clone()
        .or_else(|| entry.repo_root.clone())
}

fn enqueue_git_refresh(
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    repo_root: AbsolutePath,
    now: Instant,
    immediate: bool,
    cause: &str,
) {
    let pending_count = pending_git
        .iter()
        .filter(|(path, _)| path.as_path() != repo_root.as_path())
        .filter(|(_, state)| matches!(state, WatchState::Pending { .. }))
        .count()
        + usize::from(!matches!(
            pending_git.get(&repo_root),
            Some(WatchState::Pending { .. })
        ));
    tracing::info!(
        repo_root = %repo_root.display(),
        immediate,
        cause,
        pending_git = pending_count,
        "watcher_enqueue_git_refresh"
    );
    match pending_git.entry(repo_root).or_insert(WatchState::Idle) {
        state @ WatchState::Idle => *state = WatchState::pending(now, immediate),
        WatchState::Pending {
            debounce_deadline, ..
        } => {
            *debounce_deadline = if immediate {
                now
            } else {
                now + DEBOUNCE_DURATION
            };
        },
        state @ WatchState::Running => *state = WatchState::RunningDirty,
        WatchState::RunningDirty => {},
    }
}

fn is_ready_to_launch(state: &WatchState, now: Instant) -> bool {
    matches!(
        state,
        WatchState::Pending {
            debounce_deadline,
            max_deadline,
        } if now >= *debounce_deadline || now >= *max_deadline
    )
}

const fn mark_running(state: &mut WatchState) {
    if matches!(state, WatchState::Pending { .. }) {
        *state = WatchState::Running;
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

/// Cargo's `target-directory` may be redirected by an out-of-tree
/// `<dir>/.cargo/config[.toml]` (typically `~/.cargo/config.toml`,
/// the cargo home). Edits to such a config affect every project
/// nested under `<dir>`, none of which contains the event path —
/// so the per-project `classify_cargo_metadata_event_path` gate at
/// the bottom of `handle_notify_event` will not fire for them.
///
/// When the event basename matches a cargo-metadata trigger AND the
/// path looks like `<dir>/.cargo/config[.toml]`, fan a metadata
/// refresh out to every project whose root is descendant of `<dir>`.
fn try_dispatch_out_of_tree_cargo_config_refresh(
    event_path: &Path,
    ctx: &EventContext<'_>,
    metadata_dispatch: Option<&MetadataDispatchContext>,
) {
    let Some(dispatch) = metadata_dispatch else {
        return;
    };
    if !matches!(
        lint::classify_cargo_metadata_basename(event_path),
        Some(lint::CargoMetadataTriggerKind::CargoConfig)
    ) {
        return;
    }
    let Some(cargo_dir) = event_path.parent() else {
        return;
    };
    let Some(host_dir) = cargo_dir.parent() else {
        return;
    };
    for project_root in ctx.projects.keys() {
        if project_root.as_path().starts_with(host_dir) {
            scan::spawn_cargo_metadata_refresh(dispatch.clone(), project_root.clone());
        }
    }
}

/// Does `event_path` sit under the workspace's resolved target
/// directory? `resolved_target = None` means we don't yet have
/// workspace metadata — fall back to `<project_root>/target`, which is
/// what cargo uses by default.
///
/// When the metadata *is* available (e.g. target is redirected via
/// `CARGO_TARGET_DIR` or `.cargo/config.toml`), events under the real
/// target dir are suppressed and events under the in-tree `target/`
/// decoy are treated as ordinary project events. The design doc
/// (call-site migrations → step 2) calls this out explicitly.
fn is_target_event_for(
    event_path: &Path,
    project_root: &Path,
    resolved_target: Option<&Path>,
) -> bool {
    let fallback = project_root.join("target");
    let dir = resolved_target.unwrap_or(fallback.as_path());
    event_path.starts_with(dir)
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

fn spawn_project_refresh(bg_tx: Sender<BackgroundMsg>, project_root: AbsolutePath) {
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

fn spawn_project_refresh_after(
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

fn emit_root_git_info_refresh(bg_tx: &Sender<BackgroundMsg>, entry: &ProjectEntry) {
    let started = Instant::now();
    let Some(repo) = RepoInfo::get(entry.abs_path.as_path()) else {
        return;
    };
    let checkout = CheckoutInfo::get(entry.abs_path.as_path(), repo.local_main_branch.as_deref());
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        path = %entry.project_label,
        git_status = checkout.as_ref().map_or("unknown", |c| c.status.label()),
        "watcher_root_git_info_refresh"
    );
    let _ = bg_tx.send(BackgroundMsg::RepoInfo {
        path: entry.abs_path.clone(),
        info: repo,
    });
    if let Some(checkout) = checkout {
        let _ = bg_tx.send(BackgroundMsg::CheckoutInfo {
            path: entry.abs_path.clone(),
            info: checkout,
        });
    }
}

fn fire_git_updates(
    handle: &Handle,
    git_limit: &Arc<Semaphore>,
    git_done_tx: &Sender<AbsolutePath>,
    bg_tx: &Sender<BackgroundMsg>,
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
) {
    let now = Instant::now();
    let ready: Vec<AbsolutePath> = pending_git
        .iter()
        .filter(|(_, state)| is_ready_to_launch(state, now))
        .map(|(refresh_key, _)| refresh_key.clone())
        .collect();

    for refresh_key in ready {
        // Affected = every entry whose refresh-key matches; for the
        // common-git-dir case that's primary + all linked siblings of
        // the same repo. Each entry needs its own `CheckoutInfo`
        // probe because branch/HEAD/status differ per worktree.
        let affected: Vec<AbsolutePath> = projects
            .values()
            .filter(|entry| git_refresh_key(entry).as_ref() == Some(&refresh_key))
            .map(|entry| entry.abs_path.clone())
            .collect();
        if affected.is_empty() {
            if let Some(state) = pending_git.get_mut(&refresh_key) {
                *state = WatchState::Idle;
            }
            continue;
        }
        // Identify the primary checkout: the one whose own `.git` is
        // the common git dir (i.e., its working tree is `<git_dir>/..`).
        // Falls back to the first affected entry when no clear primary
        // is visible (e.g., entry registered without `common_git_dir`).
        let primary_path = projects
            .values()
            .find(|entry| {
                entry.git_dir.as_deref() == Some(refresh_key.as_path())
                    || entry.common_git_dir.as_deref() == Some(refresh_key.as_path())
                        && entry.abs_path.as_path().join(".git").is_dir()
            })
            .map_or_else(|| affected[0].clone(), |entry| entry.abs_path.clone());
        if let Some(state) = pending_git.get_mut(&refresh_key) {
            mark_running(state);
        }
        spawn_git_refresh(
            handle,
            git_limit,
            git_done_tx.clone(),
            bg_tx.clone(),
            refresh_key,
            primary_path,
            affected,
        );
    }
}

fn spawn_git_refresh(
    handle: &Handle,
    git_limit: &Arc<Semaphore>,
    git_done_tx: Sender<AbsolutePath>,
    bg_tx: Sender<BackgroundMsg>,
    refresh_key: AbsolutePath,
    primary_path: AbsolutePath,
    affected: Vec<AbsolutePath>,
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
            refresh_key = %refresh_key.display(),
            affected_rows = affected.len(),
            "watcher_git_queue_wait"
        );

        // Probe the per-repo half once at the primary's path. Linked
        // siblings reuse this RepoInfo via the entry's `git_repo` slot
        // (the primary-only write policy in `handle_repo_info` keeps
        // just this copy).
        let started = Instant::now();
        let primary_for_repo = primary_path.clone();
        let repo_info =
            tokio::task::spawn_blocking(move || RepoInfo::get(primary_for_repo.as_path()))
                .await
                .ok()
                .flatten();
        let local_main_branch = repo_info.as_ref().and_then(|r| r.local_main_branch.clone());
        if let Some(repo_info) = repo_info {
            let _ = bg_tx.send(BackgroundMsg::RepoInfo {
                path: primary_path.clone(),
                info: repo_info,
            });
        }

        // Probe the per-checkout half for each affected path. These are
        // cheap (no per-remote loop); each yields the worktree's own
        // branch/HEAD/status.
        for checkout_path in affected {
            let probe_path = checkout_path.clone();
            let lmb = local_main_branch.clone();
            let checkout = tokio::task::spawn_blocking(move || {
                CheckoutInfo::get(probe_path.as_path(), lmb.as_deref())
            })
            .await
            .ok()
            .flatten();
            if let Some(checkout) = checkout {
                let _ = bg_tx.send(BackgroundMsg::CheckoutInfo {
                    path: checkout_path,
                    info: checkout,
                });
            }
        }

        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            refresh_key = %refresh_key.display(),
            "watcher_git_refresh"
        );
        let _ = git_done_tx.send(refresh_key);
    });
}

fn fire_disk_updates(
    handle: &Handle,
    disk_limit: &Arc<Semaphore>,
    disk_done_tx: &Sender<String>,
    bg_tx: &Sender<BackgroundMsg>,
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    pending_disk: &mut HashMap<String, WatchState>,
) {
    let now = Instant::now();
    let ready: Vec<String> = pending_disk
        .iter()
        .filter(|(_, state)| is_ready_to_launch(state, now))
        .map(|(key, _)| key.clone())
        .collect();

    for project_label in ready {
        let Some(entry) = projects.values().find(|e| e.project_label == project_label) else {
            if let Some(state) = pending_disk.get_mut(&project_label) {
                *state = WatchState::Idle;
            }
            continue;
        };
        if let Some(state) = pending_disk.get_mut(&project_label) {
            mark_running(state);
        }
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
    handle: &Handle,
    disk_limit: &Arc<Semaphore>,
    disk_done_tx: Sender<String>,
    bg_tx: Sender<BackgroundMsg>,
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
        return Some(NonRust(project::from_git_dir(dir)));
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
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use lint::RegisterProjectRequest;
    use mpsc::Receiver;
    use mpsc::RecvTimeoutError;
    use notify::Config;
    use notify::Event;
    use notify::Watcher;
    use notify::WatcherKind;
    use notify::event::DataChange;
    use notify::event::EventKind;
    use notify::event::ModifyKind;
    use tempfile::TempDir;

    use super::*;
    use crate::lint;
    use crate::project::GitStatus;
    use crate::project::GitStatus::Clean;
    use crate::project::GitStatus::Modified;
    use crate::project::RustProject;
    use crate::test_support;

    fn test_metadata_dispatch(client: &HttpClient) -> MetadataDispatchContext {
        let (tx, _rx) = mpsc::channel();
        MetadataDispatchContext {
            handle: client.handle.clone(),
            tx,
            metadata_store: Arc::new(std::sync::Mutex::new(WorkspaceMetadataStore::new())),
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    /// No-op `notify::Watcher` for unit tests that don't actually
    /// care about filesystem subscriptions. `drain_watch_messages`
    /// and `apply_watch_request` need *something* that implements
    /// Watcher so the ancestor `.cargo/` registry can call
    /// `watch(dir, …)`; this satisfies the trait without touching
    /// the real FS layer. `unwatch` and the configuration knobs
    /// return `Ok(())` / `()` since they're never actually exercised
    /// by the tests.
    struct NoopWatcher;

    impl Watcher for NoopWatcher {
        fn new<F: notify::EventHandler>(_event_handler: F, _config: Config) -> notify::Result<Self>
        where
            Self: Sized,
        {
            Ok(Self)
        }

        fn watch(&mut self, _path: &Path, _mode: RecursiveMode) -> notify::Result<()> { Ok(()) }

        fn unwatch(&mut self, _path: &Path) -> notify::Result<()> { Ok(()) }

        fn configure(&mut self, _config: Config) -> notify::Result<bool> { Ok(true) }

        fn kind() -> WatcherKind
        where
            Self: Sized,
        {
            WatcherKind::NullWatcher
        }
    }

    /// Test double whose `watch()` returns `Err` for any path matching
    /// `fail_on`. Lets the registration test simulate the real-world
    /// case where notify accepts some watch roots and rejects others.
    struct SelectiveFailWatcher {
        fail_on: PathBuf,
    }

    impl Watcher for SelectiveFailWatcher {
        fn new<F: notify::EventHandler>(_event_handler: F, _config: Config) -> notify::Result<Self>
        where
            Self: Sized,
        {
            Ok(Self {
                fail_on: PathBuf::new(),
            })
        }

        fn watch(&mut self, path: &Path, _mode: RecursiveMode) -> notify::Result<()> {
            if path == self.fail_on {
                Err(notify::Error::generic("simulated watch failure"))
            } else {
                Ok(())
            }
        }

        fn unwatch(&mut self, _path: &Path) -> notify::Result<()> { Ok(()) }

        fn configure(&mut self, _config: Config) -> notify::Result<bool> { Ok(true) }

        fn kind() -> WatcherKind
        where
            Self: Sized,
        {
            WatcherKind::NullWatcher
        }
    }

    /// Regression: `register_watch_roots` is the only constructor of
    /// `RegisteredRoots`, and it must record (not silently drop) every
    /// per-root failure — both `notify::Watcher::watch` errors and
    /// non-directory inputs. The previous `let _ = watcher.watch(...)`
    /// implementation made it impossible to detect that one of several
    /// configured roots had failed to register, so the watcher loop
    /// would run claiming to watch every advertised root while in fact
    /// silently dropping events for one.
    #[test]
    fn register_watch_roots_reports_per_root_failures() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ok_a = tmp.path().join("ok_a");
        let fails = tmp.path().join("fails");
        let ok_b = tmp.path().join("ok_b");
        let missing = tmp.path().join("does_not_exist");
        for dir in [&ok_a, &fails, &ok_b] {
            std::fs::create_dir_all(dir).expect("mkdir");
        }
        let mut watcher = SelectiveFailWatcher {
            fail_on: fails.clone(),
        };
        let dirs = [
            AbsolutePath::from(ok_a.clone()),
            AbsolutePath::from(fails.clone()),
            AbsolutePath::from(ok_b.clone()),
            AbsolutePath::from(missing.clone()),
        ];

        let (registered, failures) = register_watch_roots(&mut watcher, &dirs);

        assert_eq!(
            registered.dirs(),
            &[AbsolutePath::from(ok_a), AbsolutePath::from(ok_b)],
            "only the dirs whose `watch()` succeeded should be in `RegisteredRoots`"
        );
        assert_eq!(failures.len(), 2, "two roots should fail");
        assert_eq!(failures[0].dir.as_path(), fails.as_path());
        assert!(matches!(
            failures[0].reason,
            WatchRootRegistrationFailureReason::Notify(_)
        ));
        assert_eq!(failures[1].dir.as_path(), missing.as_path());
        assert!(matches!(
            failures[1].reason,
            WatchRootRegistrationFailureReason::NotADirectory
        ));
    }

    /// Records every `watch()` and `unwatch()` call so a test can
    /// assert that the watcher API was (or was not) touched for a
    /// given path. Every call returns `Ok` regardless of mode.
    struct RecordingWatcher {
        watched:   Arc<Mutex<Vec<(PathBuf, RecursiveMode)>>>,
        unwatched: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl Watcher for RecordingWatcher {
        fn new<F: notify::EventHandler>(_event_handler: F, _config: Config) -> notify::Result<Self>
        where
            Self: Sized,
        {
            Ok(Self {
                watched:   Arc::new(std::sync::Mutex::new(Vec::new())),
                unwatched: Arc::new(std::sync::Mutex::new(Vec::new())),
            })
        }

        fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
            self.watched
                .lock()
                .expect("recording watcher lock")
                .push((path.to_path_buf(), mode));
            Ok(())
        }

        fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
            self.unwatched
                .lock()
                .expect("recording watcher lock")
                .push(path.to_path_buf());
            Ok(())
        }

        fn configure(&mut self, _config: Config) -> notify::Result<bool> { Ok(true) }

        fn kind() -> WatcherKind
        where
            Self: Sized,
        {
            WatcherKind::NullWatcher
        }
    }

    /// Regression: `register_cargo_home_watch` must not register
    /// `~/.cargo` (or `$CARGO_HOME`) when the cargo home is already
    /// inside one of the recursive watch roots. macOS `FSEvents`
    /// tracks one mode per path, so a redundant `NonRecursive` call
    /// would clobber the recursive subscription — the failure mode
    /// that originally killed event delivery for everything under
    /// `~/rust`.
    #[test]
    fn cargo_home_watch_skipped_when_covered_by_recursive_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cargo_home = tmp.path().join(".cargo");
        std::fs::create_dir_all(&cargo_home).expect("mkdir cargo_home");
        // Recursive root that contains the cargo home (the parent dir).
        let registered_roots = RegisteredRoots {
            dirs: vec![AbsolutePath::from(tmp.path())],
        };

        let mut watcher = RecordingWatcher::new_for_test();
        let watched_handle = Arc::clone(&watcher.watched);

        // SAFETY: tests run serially within the watcher::tests module,
        // so the env-var mutation cannot race with another test.
        unsafe {
            std::env::set_var("CARGO_HOME", cargo_home.as_os_str());
        }
        register_cargo_home_watch(&mut watcher, &registered_roots);
        unsafe { std::env::remove_var("CARGO_HOME") };

        let recorded: Vec<(PathBuf, RecursiveMode)> = watched_handle
            .lock()
            .expect("recording watcher lock")
            .clone();
        assert!(
            recorded.is_empty(),
            "cargo home is covered by a recursive root — no extra watch should be registered, \
             recorded calls: {recorded:?}"
        );
    }

    impl RecordingWatcher {
        fn new_for_test() -> Self {
            Self {
                watched:   Arc::new(std::sync::Mutex::new(Vec::new())),
                unwatched: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }
    }

    /// **TEST-ONLY** wrapper around any [`notify::Watcher`] that
    /// records every `watch()` call and refuses any subsequent call
    /// that would re-register the same path or land on a path covered
    /// by an existing recursive watch. Used to assert the
    /// architectural invariant below — **not** a production type.
    struct GuardedWatcher<W: notify::Watcher> {
        inner:      W,
        registered: HashMap<PathBuf, RecursiveMode>,
    }

    impl<W: notify::Watcher> GuardedWatcher<W> {
        fn wrap(inner: W) -> Self {
            Self {
                inner,
                registered: HashMap::new(),
            }
        }
    }

    impl<W: notify::Watcher> Watcher for GuardedWatcher<W> {
        fn new<F: notify::EventHandler>(_eh: F, _cfg: Config) -> notify::Result<Self>
        where
            Self: Sized,
        {
            Err(notify::Error::generic(
                "GuardedWatcher is test infrastructure; construct via `GuardedWatcher::wrap`",
            ))
        }

        fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
            if self.registered.contains_key(path) {
                return Err(notify::Error::generic(&format!(
                    "guarded watcher refused: `{}` already registered",
                    path.display()
                )));
            }
            for (existing, existing_mode) in &self.registered {
                if *existing_mode == RecursiveMode::Recursive
                    && path.starts_with(existing)
                    && existing.as_path() != path
                {
                    return Err(notify::Error::generic(&format!(
                        "guarded watcher refused: `{}` would be shadowed by recursive watch on \
                         `{}` (registering it would silently change the mode of the recursive \
                         watch on macOS FSEvents)",
                        path.display(),
                        existing.display()
                    )));
                }
            }
            self.inner.watch(path, mode)?;
            self.registered.insert(path.to_path_buf(), mode);
            Ok(())
        }

        fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
            let result = self.inner.unwatch(path);
            self.registered.remove(path);
            result
        }

        fn configure(&mut self, config: Config) -> notify::Result<bool> {
            self.inner.configure(config)
        }

        fn kind() -> WatcherKind
        where
            Self: Sized,
        {
            W::kind()
        }
    }

    /// **ARCHITECTURAL INVARIANT — DO NOT WEAKEN WITHOUT A DESIGN
    /// DISCUSSION WITH THE USER.**
    ///
    /// Decision (2026-04-24): the watcher subsystem registers exactly
    /// one notify watch per path. Recursive watch roots cover
    /// everything inside them; no second `watch()` call may land on a
    /// path already covered by a recursive root. The full per-project
    /// ancestor-watch subsystem was removed for this reason — the
    /// invariant must hold by construction in production code, not by
    /// a runtime guard.
    ///
    /// This invariant exists because macOS `FSEvents` tracks one mode
    /// per path — the original "git status never refreshes for
    /// projects under `~/rust`" bug was caused by a `NonRecursive`
    /// call silently overwriting a `Recursive` watch on the same path.
    ///
    /// The two tests below enforce the invariant from two angles:
    ///   1. `guarded_watcher_rejects_overlap_with_recursive_root` — proves the test-only
    ///      `GuardedWatcher` correctly detects both classes of redundant call (duplicate,
    ///      shadowed).
    ///   2. `startup_registration_introduces_no_overlapping_watches` — runs the production startup
    ///      registration sequence (`register_watch_roots` + `register_cargo_home_watch`) through
    ///      `GuardedWatcher` and asserts no rejection occurs. If anyone adds a redundant
    ///      `watcher.watch()` call anywhere in that sequence, the guard rejects it and the test
    ///      fails.
    ///
    /// **If either test fails, the right response is not to relax the
    /// guard — it is to bring the design conflict back to the user
    /// before changing the behavior.**
    #[test]
    fn guarded_watcher_rejects_overlap_with_recursive_root() {
        let mut guard = GuardedWatcher::wrap(RecordingWatcher::new_for_test());

        // First, a recursive root succeeds.
        let root = PathBuf::from("/tmp/cargo_port_test_root");
        guard
            .watch(&root, RecursiveMode::Recursive)
            .expect("recursive root accepted");

        // Same path again — refused (would be a redundant double-register).
        let dup_err = guard
            .watch(&root, RecursiveMode::Recursive)
            .expect_err("duplicate watch must be rejected");
        assert!(
            dup_err.to_string().contains("already registered"),
            "duplicate-watch error should be self-explanatory, got: {dup_err}"
        );

        // Path covered by the recursive root — refused, regardless of mode.
        let nested = root.join("project");
        let nested_err = guard
            .watch(&nested, RecursiveMode::NonRecursive)
            .expect_err("nested NonRecursive watch must be rejected");
        assert!(
            nested_err
                .to_string()
                .contains("shadowed by recursive watch"),
            "shadowed-watch error should call out the recursive root, got: {nested_err}"
        );

        // After unwatch, the path can be re-registered.
        guard.unwatch(&root).expect("unwatch root");
        guard
            .watch(&nested, RecursiveMode::NonRecursive)
            .expect("after unwatch, nested watch is permitted again");
    }

    /// **ARCHITECTURAL INVARIANT — see preceding test for the design
    /// rationale and the standing decision with the user.**
    ///
    /// Drives the production startup registration sequence
    /// (`register_watch_roots` followed by `register_cargo_home_watch`)
    /// through a `GuardedWatcher`. If any code path inside those
    /// functions issues a redundant `watcher.watch()` call — a
    /// duplicate path, or a path shadowed by an already-registered
    /// recursive root — the guard returns `Err`, the failure is
    /// observable here, and the test fails.
    ///
    /// Inputs are picked to exercise the realistic case: two
    /// recursive roots, a cargo home that lives outside both — exactly
    /// the configuration that uncovered the original bug. Adding a
    /// new `watcher.watch()` call to any helper invoked here will fail
    /// this test if it overlaps an existing registration.
    #[test]
    fn startup_registration_introduces_no_overlapping_watches() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root_a = tmp.path().join("rust");
        let root_b = tmp.path().join("claude");
        let cargo_home = tmp.path().join("cargo_home");
        for dir in [&root_a, &root_b, &cargo_home] {
            std::fs::create_dir_all(dir).expect("mkdir root");
        }
        let watch_roots = [AbsolutePath::from(root_a), AbsolutePath::from(root_b)];

        let mut guard = GuardedWatcher::wrap(RecordingWatcher::new_for_test());

        let (registered_roots, failures) = register_watch_roots(&mut guard, &watch_roots);
        assert!(
            failures.is_empty(),
            "register_watch_roots must not produce per-root failures for non-overlapping inputs; \
             got: {:?}",
            failures
                .iter()
                .map(|f| (f.dir.display().to_string(), f.reason.to_string()))
                .collect::<Vec<_>>()
        );
        assert_eq!(registered_roots.dirs().len(), watch_roots.len());

        // SAFETY: tests serialise within the watcher::tests module so
        // the env-var write cannot race with another test reading it.
        unsafe {
            std::env::set_var("CARGO_HOME", cargo_home.as_os_str());
        }
        register_cargo_home_watch(&mut guard, &registered_roots);
        unsafe { std::env::remove_var("CARGO_HOME") };

        // Expected registered set: the two recursive roots plus the
        // cargo home (which sits outside both). Anything more or less
        // means a code path in the startup sequence either dropped a
        // watch or registered an overlapping one.
        let expected_count = watch_roots.len() + 1;
        assert_eq!(
            guard.registered.len(),
            expected_count,
            "guard's registered set should contain exactly the recursive roots plus cargo_home; \
             got: {:?}",
            guard.registered
        );
    }

    /// `try_dispatch_out_of_tree_cargo_config_refresh` must spawn a
    /// metadata refresh for every project nested under the dir that
    /// contains the changed `.cargo/config.toml`. This stands in for
    /// the deleted ancestor-registry dispatch path: when a
    /// `.cargo/config.toml` event arrives via the recursive watch
    /// (e.g. on `<root>/.cargo/config.toml`), every workspace under
    /// `<root>` whose `target-directory` could be redirected by that
    /// config gets re-read.
    #[test]
    fn out_of_tree_cargo_config_refresh_fans_out_to_descendant_projects() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let host = tmp.path().to_path_buf();
        let cargo_dir = host.join(".cargo");
        let event_path = cargo_dir.join("config.toml");
        let project_under = host.join("nested").join("project");
        let project_outside = tmp
            .path()
            .parent()
            .unwrap_or_else(|| tmp.path())
            .join("elsewhere");

        let mut projects = HashMap::new();
        for path in [&project_under, &project_outside] {
            projects.insert(
                AbsolutePath::from(path.clone()),
                ProjectEntry {
                    project_label:  path.display().to_string(),
                    abs_path:       AbsolutePath::from(path.clone()),
                    repo_root:      None,
                    git_dir:        None,
                    common_git_dir: None,
                },
            );
        }
        let watch_roots = vec![AbsolutePath::from(host.clone())];
        let project_parents = HashSet::from([AbsolutePath::from(host.clone())]);
        let discovered = HashSet::new();
        let ctx = EventContext {
            watch_roots:     &watch_roots,
            projects:        &projects,
            project_parents: &project_parents,
            discovered:      &discovered,
        };

        let (tx, rx) = mpsc::channel();
        let dispatch = MetadataDispatchContext {
            handle: test_support::test_runtime().handle().clone(),
            tx,
            metadata_store: Arc::new(std::sync::Mutex::new(WorkspaceMetadataStore::new())),
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(1)),
        };

        try_dispatch_out_of_tree_cargo_config_refresh(&event_path, &ctx, Some(&dispatch));

        let mut refreshed: Vec<AbsolutePath> = Vec::new();
        while let Ok(msg) = rx.recv_timeout(Duration::from_millis(200)) {
            if let BackgroundMsg::CargoMetadata { workspace_root, .. } = msg {
                refreshed.push(workspace_root);
            }
        }
        assert_eq!(
            refreshed,
            vec![AbsolutePath::from(project_under)],
            "only the project nested under `{}` should be refreshed; outside project must not be",
            host.display()
        );
    }

    // ── is_target_event_for ──────────────────────────────────────────

    #[test]
    fn is_target_event_for_uses_in_tree_default_without_metadata() {
        let root = Path::new("/home/u/proj");
        let in_tree = root.join("target/debug/foo");
        assert!(
            is_target_event_for(&in_tree, root, None),
            "default: events under <project>/target/ classify as target events"
        );
        let src = root.join("src/main.rs");
        assert!(
            !is_target_event_for(&src, root, None),
            "events outside target/ are not target events"
        );
    }

    #[test]
    fn is_target_event_for_honors_resolved_out_of_tree_target() {
        let root = Path::new("/home/u/proj");
        let resolved = PathBuf::from("/tmp/custom-target");
        let in_resolved = resolved.join("debug/foo");
        let in_tree_decoy = root.join("target/debug/foo");

        assert!(
            is_target_event_for(&in_resolved, root, Some(&resolved)),
            "with a resolved out-of-tree target, events there are target events"
        );
        assert!(
            !is_target_event_for(&in_tree_decoy, root, Some(&resolved)),
            "once the target is redirected, the in-tree <project>/target/ decoy \
             is no longer treated as a target event"
        );
    }

    #[test]
    fn initial_registration_complete_transitions_watcher_out_of_initializing() {
        let (watch_tx, watch_rx) = mpsc::channel();
        let mut state = WatcherLoopState::new();
        let mut watcher = NoopWatcher;

        watch_tx
            .send(WatcherMsg::InitialRegistrationComplete)
            .expect("send registration complete");

        let drained = drain_watch_messages(&watch_rx, &mut state, &mut watcher);

        assert!(drained.registration_completed);
        assert!(!state.initializing);
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
            let mut state = WatcherLoopState::new();
            let mut watcher = NoopWatcher;
            let drained = drain_watch_messages(&watch_rx, &mut state, &mut watcher);
            let _ = result_tx.send((drained, state.initializing));
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
        /// Drop-signalling wrapper around a `NoopWatcher`. The loop
        /// now requires `impl Watcher` (Step 7b integration), so we
        /// delegate the trait to the inner watcher and use Drop on
        /// the outer type to prove the guard outlives the thread.
        struct DropSignal {
            flag:  Arc<AtomicBool>,
            inner: NoopWatcher,
        }

        impl Drop for DropSignal {
            fn drop(&mut self) { self.flag.store(true, Ordering::SeqCst); }
        }

        impl Watcher for DropSignal {
            fn new<F: notify::EventHandler>(
                _event_handler: F,
                _config: Config,
            ) -> notify::Result<Self>
            where
                Self: Sized,
            {
                // DropSignal is constructed by test code; the trait's
                // factory constructor isn't exercised here but needs
                // to exist to satisfy the trait.
                Err(notify::Error::generic(
                    "DropSignal::new should not be called in tests",
                ))
            }

            fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
                self.inner.watch(path, mode)
            }

            fn unwatch(&mut self, path: &Path) -> notify::Result<()> { self.inner.unwatch(path) }

            fn configure(&mut self, config: Config) -> notify::Result<bool> {
                self.inner.configure(config)
            }

            fn kind() -> WatcherKind
            where
                Self: Sized,
            {
                WatcherKind::NullWatcher
            }
        }

        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watcher_guard = DropSignal {
            flag:  std::sync::Arc::clone(&dropped),
            inner: NoopWatcher,
        };
        let (watch_tx, watch_rx) = mpsc::channel();
        let (notify_tx, notify_rx) = mpsc::channel();
        let (bg_tx, _bg_rx) = mpsc::channel();
        let client =
            HttpClient::new(test_support::test_runtime().handle().clone()).expect("http client");

        let client_for_dispatch = client.clone();
        spawn_watcher_thread(
            WatcherLoopContext {
                watch_roots: RegisteredRoots::default(),
                bg_tx,
                ci_run_count: 0,
                non_rust: NonRustInclusion::Exclude,
                client,
                lint_runtime: None,
                metadata_dispatch: test_metadata_dispatch(&client_for_dispatch),
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

    fn wait_for_completion<T>(rx: &Receiver<T>) {
        rx.recv_timeout(Duration::from_secs(1))
            .unwrap_or_else(|_| panic!("timed out waiting for background completion"));
    }

    fn collect_messages_until(
        rx: &Receiver<BackgroundMsg>,
        predicate: impl Fn(&BackgroundMsg) -> bool,
    ) -> Vec<BackgroundMsg> {
        collect_messages_until_with_timeout(rx, Duration::from_secs(1), predicate)
    }

    fn collect_messages_until_with_timeout(
        rx: &Receiver<BackgroundMsg>,
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
    fn project_level_dir_handles_synthetic_path_forms() {
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

    fn assert_pending_disk(states: &HashMap<String, WatchState>, project_path: &str) {
        assert!(matches!(
            states.get(project_path),
            Some(WatchState::Pending { .. })
        ));
    }

    fn event_with_path(path: &AbsolutePath) -> Event {
        Event {
            kind:  EventKind::Any,
            paths: vec![path.to_path_buf()],
            attrs: notify::event::EventAttributes::default(),
        }
    }

    #[allow(
        clippy::type_complexity,
        reason = "test fixture returning multiple setup values"
    )]
    fn repo_with_member_event_context(
        tmp: &TempDir,
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
            test_support::test_runtime().handle(),
            &git_limit,
            &git_done_tx,
            &bg_tx,
            &projects,
            &mut pending_git,
        );
        let messages = collect_messages_until(
            &bg_rx,
            |msg| matches!(msg, BackgroundMsg::CheckoutInfo { path, .. } | BackgroundMsg::RepoInfo { path, .. } if *path == *project_dir),
        );

        let mut got_root_git_info = false;
        for msg in &messages {
            if matches!(msg, BackgroundMsg::CheckoutInfo { path, .. } | BackgroundMsg::RepoInfo { path, .. } if *path == *project_dir)
            {
                got_root_git_info = true;
            }
        }

        assert!(got_root_git_info, "{context}");
        assert!(pending_disk.is_empty(), "{context}");
        assert!(
            pending_git.contains_key(project_dir.join(".git").as_path()),
            "{context}"
        );
        assert!(pending_new.is_empty(), "{context}");
    }

    #[allow(
        clippy::type_complexity,
        reason = "test fixture returning multiple setup values"
    )]
    fn worktree_git_event_context(
        tmp: &TempDir,
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
    fn tracked_file_edit_and_revert_refresh_git_status() {
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
             expected: GitStatus,
             pending_disk: &mut HashMap<String, WatchState>,
             pending_git: &mut HashMap<AbsolutePath, WatchState>,
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
                let project_git_dir = project_dir.join(".git");
                let Some(WatchState::Pending {
                    debounce_deadline,
                    max_deadline,
                    ..
                }) = pending_git.get_mut(project_git_dir.as_path())
                else {
                    panic!("expected pending git refresh for tracked file event");
                };
                *debounce_deadline = past;
                *max_deadline = past;
                fire_git_updates(
                    test_support::test_runtime().handle(),
                    &git_limit,
                    &git_done_tx,
                    &bg_tx,
                    &projects,
                    pending_git,
                );
                let messages = collect_messages_until(
                    &bg_rx,
                    |msg| matches!(msg, BackgroundMsg::CheckoutInfo { path, .. } if *path == *project_dir),
                );
                let git_msg = messages
                    .into_iter()
                    .find_map(|msg| match msg {
                        BackgroundMsg::CheckoutInfo { path, info }
                            if path.as_path() == project_dir.as_path() =>
                        {
                            Some(info)
                        },
                        _ => None,
                    })
                    .expect("git info message for project");
                assert_eq!(git_msg.status, expected);
                let repo_root = git_done_rx
                    .recv_timeout(Duration::from_secs(1))
                    .expect("git refresh completion");
                handle_git_completion(pending_git, &repo_root);
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
        // Step 3b retirement: the refreshed `Package`'s `Cargo` no
        // longer carries hand-parsed example data — that flows from
        // the authoritative `cargo metadata` result. The contract
        // pinned here is the refresh-emission pattern (a `Package`
        // arriving on `BackgroundMsg::ProjectRefreshed` for the
        // watched root), not the derived target counts.
        assert!(matches!(
            refreshed,
            crate::project::RootItem::Rust(crate::project::RustProject::Package(_))
        ));
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
        let (_wt_root, wt_git_dir, projects, watch_roots, project_parents, discovered) =
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

        // The worktree project's `common_git_dir` is the shared parent's
        // `.git` (set up as `tmp/main_repo_git` in the fixture), so
        // `pending_git` is keyed on that path, not on `wt_root`.
        let common_git_dir = tmp.path().join("main_repo_git");
        assert!(
            pending_git.contains_key(common_git_dir.as_path()),
            "worktree index event should enqueue a git refresh for the worktree project"
        );
    }

    #[test]
    fn worktree_logs_head_event_enqueues_git_refresh() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (_wt_root, wt_git_dir, projects, watch_roots, project_parents, discovered) =
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

        // The worktree project's `common_git_dir` is the shared parent's
        // `.git` (set up as `tmp/main_repo_git` in the fixture), so
        // `pending_git` is keyed on that path, not on `wt_root`.
        let common_git_dir = tmp.path().join("main_repo_git");
        assert!(
            pending_git.contains_key(common_git_dir.as_path()),
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
        let (_wt_root, _wt_git_dir, projects, watch_roots, project_parents, discovered) =
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

        // The worktree project's `common_git_dir` is the shared parent's
        // `.git`, so `pending_git` is keyed on that path, not on `wt_root`.
        assert!(
            matches!(
                pending_git.get(common_git_dir.as_path()),
                Some(WatchState::Pending { .. })
            ),
            "shared branch ref writes should enqueue a git refresh for linked worktrees"
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
                repo_root:      Some(AbsolutePath::from(main_root)),
                git_dir:        Some(AbsolutePath::from(common_git_dir.clone())),
                common_git_dir: Some(AbsolutePath::from(common_git_dir.clone())),
            },
        );
        projects.insert(
            AbsolutePath::from(wt_root.clone()),
            ProjectEntry {
                project_label:  "~/main_repo_style_fix".to_string(),
                abs_path:       AbsolutePath::from(wt_root.clone()),
                repo_root:      Some(AbsolutePath::from(wt_root)),
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

        // Stage 4: pending_git is keyed on `common_git_dir` so primary
        // + linked siblings collapse into a single pending refresh
        // (the spawn then fans out to both via `affected`).
        assert!(
            pending_git.contains_key(common_git_dir.as_path()),
            "shared common_git_dir should be enqueued for git refresh"
        );
        assert_eq!(
            pending_git.len(),
            1,
            "primary + linked sibling should dedup to one pending entry"
        );
        // Verify both projects would be picked up by `fire_git_updates`'s
        // affected filter for this key.
        let affected_count = projects
            .values()
            .filter(|entry| git_refresh_key(entry).as_deref() == Some(common_git_dir.as_path()))
            .count();
        assert_eq!(affected_count, 2, "both projects affected by shared event");
        // Touch wt_root to assert it's still part of the affected set
        // even though it doesn't have its own pending_git key.
        let _ = wt_root;
    }

    #[test]
    fn buffered_worktree_git_dir_event_replays_after_registration_complete() {
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
        let dispatch = WatcherDispatchContext {
            event:             ctx,
            bg_tx:             &bg_tx,
            lint_runtime:      None,
            metadata_dispatch: None,
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

        // The worktree project's `common_git_dir` is the shared parent's
        // `.git` (set up as `tmp/main_repo_git` in the fixture), so
        // `pending_git` is keyed on that path, not on `wt_root`.
        let common_git_dir = tmp.path().join("main_repo_git");
        assert!(
            pending_git.contains_key(common_git_dir.as_path()),
            "buffered worktree git-dir events should replay through the normal classifier"
        );
        assert!(pending_new.is_empty());
    }

    #[test]
    fn buffered_worktree_common_git_event_replays_after_registration_complete() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (_wt_root, _wt_git_dir, projects, watch_roots, project_parents, discovered) =
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
            event:             ctx,
            bg_tx:             &bg_tx,
            lint_runtime:      None,
            metadata_dispatch: None,
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

        // The worktree project's `common_git_dir` is the shared parent's
        // `.git`, so `pending_git` is keyed on that path, not on `wt_root`.
        assert!(
            matches!(
                pending_git.get(common_git_dir.as_path()),
                Some(WatchState::Pending { .. })
            ),
            "buffered common-git-dir events should still trigger a git refresh"
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
    fn watcher_event_schedules_lint_run_through_main_runtime() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        std::fs::create_dir_all(project_dir.path().join("src")).expect("create src");
        let source_path = project_dir.path().join("src/lib.rs");
        std::fs::write(&source_path, "pub fn demo() {}\n").expect("write source");

        let cache_dir = tempfile::tempdir().expect("tempdir");
        let mut cfg = crate::config::CargoPortConfig::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        cfg.lint.enabled = true;
        cfg.lint.include = vec!["~/rust/demo".to_string()];
        cfg.lint.commands = vec![crate::config::LintCommandConfig {
            name:    "echo".to_string(),
            command: "printf 'lint ok\\n'".to_string(),
        }];

        let (bg_tx, bg_rx) = mpsc::channel();
        let runtime = lint::spawn(&cfg, bg_tx.clone())
            .handle
            .expect("runtime handle");
        let request = RegisterProjectRequest {
            project_label: "~/rust/demo".to_string(),
            abs_path:      AbsolutePath::from(project_dir.path()),
            is_rust:       true,
        };
        runtime.sync_projects(vec![request.clone()]);
        runtime.register_project(request);

        let mut projects = HashMap::new();
        let (key, entry) = make_project_entry("~/rust/demo", project_dir.path());
        projects.insert(key, entry);
        let watch_roots = vec![AbsolutePath::from(project_dir.path())];
        let project_parents = HashSet::new();
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
        let event = Event {
            kind:  EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            paths: vec![source_path.clone()],
            attrs: notify::event::EventAttributes::default(),
        };

        handle_notify_event(
            &source_path,
            Some(&event),
            &ctx,
            &bg_tx,
            Some(&runtime),
            None,
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_passed = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match bg_rx.recv_timeout(remaining) {
                Ok(BackgroundMsg::LintStatus { path, status })
                    if path.as_path() == project_dir.path()
                        && matches!(status, lint::LintStatus::Passed(_)) =>
                {
                    saw_passed = true;
                    break;
                },
                Ok(_) => {},
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    break;
                },
            }
        }

        assert!(saw_passed, "expected watcher event to schedule a lint run");
        assert_pending_disk(&pending_disk, "~/rust/demo");
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
            event:             ctx,
            bg_tx:             &bg_tx,
            lint_runtime:      None,
            metadata_dispatch: None,
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
    /// `LocalGitInfo::get` returns `Some`.
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
            WatchState::Pending {
                debounce_deadline: past,
                max_deadline:      past,
            },
        )]);

        let disk_limit = Arc::new(tokio::sync::Semaphore::new(1));
        let (disk_done_tx, disk_done_rx) = mpsc::channel();
        fire_disk_updates(
            test_support::test_runtime().handle(),
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
                BackgroundMsg::CheckoutInfo { path, .. } | BackgroundMsg::RepoInfo { path, .. }
                    if *path == *project_dir =>
                {
                    got_git = true;
                },
                _ => {},
            }
        }
        assert!(got_disk, "expected DiskUsage message");
        assert!(!got_git, "disk updates should no longer emit GitInfo");
        assert!(matches!(
            pending.get("~/my_project"),
            Some(WatchState::Running)
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
            WatchState::Pending {
                debounce_deadline: past,
                max_deadline:      past,
            },
        )]);

        let disk_limit = Arc::new(tokio::sync::Semaphore::new(1));
        let (disk_done_tx, disk_done_rx) = mpsc::channel();
        fire_disk_updates(
            test_support::test_runtime().handle(),
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
                BackgroundMsg::CheckoutInfo { .. } | BackgroundMsg::RepoInfo { .. } => {
                    got_git = true;
                },
                _ => {},
            }
        }
        assert!(got_disk, "expected DiskUsage message");
        assert!(!got_git, "should not send GitInfo for untracked project");
    }

    #[test]
    fn disk_completion_requeues_once_when_project_changed_while_running() {
        let mut pending = HashMap::from([("~/rust/bevy".to_string(), WatchState::RunningDirty)]);

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
            &crate::http::HttpClient::new(test_support::test_runtime().handle().clone())
                .expect("http client"),
        );

        let BackgroundMsg::ProjectDiscovered { item } = bg_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("project discovered message")
        else {
            panic!("unexpected message");
        };
        let RootItem::Rust(RustProject::Package(pkg)) = item else {
            panic!("expected package worktree item");
        };
        assert_eq!(pkg.path(), linked_dir.as_path());
        let canonical = crate::project::AbsolutePath::from(
            primary_dir.canonicalize().expect("canonical primary"),
        );
        assert_eq!(
            pkg.worktree_status(),
            &crate::project::WorktreeStatus::Linked { primary: canonical }
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
        let mut pending_new = HashMap::from([(AbsolutePath::from(linked_dir.clone()), past)]);
        let mut discovered = HashSet::new();

        probe_new_projects(
            &bg_tx,
            &mut pending_new,
            &mut discovered,
            5,
            NonRustInclusion::default(),
            &crate::http::HttpClient::new(test_support::test_runtime().handle().clone())
                .expect("http client"),
        );

        let BackgroundMsg::ProjectDiscovered { item } = bg_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("project discovered message")
        else {
            panic!("unexpected message");
        };
        let RootItem::Rust(RustProject::Workspace(ws)) = item else {
            panic!("expected workspace worktree item");
        };
        assert_eq!(ws.path(), linked_dir.as_path());
        let canonical = crate::project::AbsolutePath::from(
            primary_dir.canonicalize().expect("canonical primary"),
        );
        assert_eq!(
            ws.worktree_status(),
            &crate::project::WorktreeStatus::Linked { primary: canonical }
        );
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
        let RootItem::Rust(RustProject::Workspace(ws)) = item else {
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
            .map(|(_, sizes)| sizes.total)
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
            WatchState::Pending {
                debounce_deadline: past,
                max_deadline:      past,
            },
        );
        let disk_limit = Arc::new(tokio::sync::Semaphore::new(1));
        let (disk_done_tx, disk_done_rx) = mpsc::channel();
        fire_disk_updates(
            test_support::test_runtime().handle(),
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
            WatchState::Pending {
                debounce_deadline: past,
                max_deadline:      past,
            },
        );
        let disk_limit = Arc::new(tokio::sync::Semaphore::new(1));
        let (disk_done_tx, disk_done_rx) = mpsc::channel();
        fire_disk_updates(
            test_support::test_runtime().handle(),
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
