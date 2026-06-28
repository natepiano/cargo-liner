use tui_pane::PERF_LOG_TARGET;

use super::AbsolutePath;
use super::Arc;
use super::AtomicBool;
use super::BackgroundMsg;
use super::CARGO_TOML;
use super::CachedLintStatus;
use super::CargoPortConfig;
use super::Child;
use super::ChildSlot;
use super::DiscoveryLint;
use super::HashMap;
use super::HashSet;
use super::Instant;
use super::JoinHandle;
use super::LINTS_HISTORY_JSONL;
use super::LINTS_LATEST_JSON;
use super::LintCommandConfig;
use super::LintConfig;
use super::LintEventKind;
use super::LintRunOrigin;
use super::LintRunStatus;
use super::LintStatus;
use super::LintTriggerEvent;
use super::LintTriggerKind;
use super::Mutex;
use super::Ordering;
use super::Path;
use super::RecvTimeoutError;
use super::RegisterProjectRequest;
use super::RunCommandsConfig;
use super::RuntimeHandle;
use super::STOP_POLL;
use super::Sender;
use super::StdReceiver;
use super::StdSender;
use super::SupervisorMsg;
use super::cache_paths;
use super::handle::SpawnResult;
use super::mpsc;
use super::paths;
use super::project;
use super::publish_status;
use super::read_write;
use super::run_commands_for_project;
use super::status;
use super::thread;

pub(super) struct ProjectWorker {
    pub(super) stop:       Arc<AtomicBool>,
    pub(super) trigger_tx: StdSender<LintTriggerEvent>,
    pub(super) child:      ChildSlot,
    pub(super) handle:     JoinHandle<()>,
}

pub fn spawn(
    cargo_port_config: &CargoPortConfig,
    background_tx: Sender<BackgroundMsg>,
) -> SpawnResult {
    if !cargo_port_config.lint.enabled.is_enabled() {
        return SpawnResult {
            handle:                  None,
            warning:                 None,
            #[cfg(test)]
            supervisor:              None,
        };
    }

    let cache_root = cache_paths::lint_runs_root_for(cargo_port_config);
    let cache_size_bytes = cargo_port_config.lint.cache_size_bytes().unwrap_or(None);
    let lint = cargo_port_config.lint.clone();
    let (supervisor_sender, supervisor_receiver) = mpsc::channel();
    let supervisor = thread::spawn(move || {
        supervisor_loop(
            &supervisor_receiver,
            &cache_root,
            &lint,
            cache_size_bytes,
            &background_tx,
        );
    });
    #[cfg(test)]
    let supervisor = Some(supervisor);
    #[cfg(not(test))]
    drop(supervisor);
    SpawnResult {
        handle: Some(RuntimeHandle { supervisor_sender }),
        warning: None,
        #[cfg(test)]
        supervisor,
    }
}

fn supervisor_loop(
    supervisor_receiver: &StdReceiver<SupervisorMsg>,
    cache_root: &AbsolutePath,
    lint: &LintConfig,
    cache_size_bytes: Option<u64>,
    background_tx: &Sender<BackgroundMsg>,
) {
    let mut workers: HashMap<AbsolutePath, ProjectWorker> = HashMap::new();
    // Lazy hydration: the cache starts empty and `cached_status_for_project`
    // reads disk on miss. Disk hydration is terminal-only: leftover
    // `Running` files from a dead process are cleared and do not enter this
    // cache. The previous eager scan walked every cache subdirectory (~8000
    // stale project keys after a few months of use), blocking the supervisor
    // for ~2s before it could process a single registration.
    let status_cache: Arc<Mutex<HashMap<String, CachedLintStatus>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let paused = Arc::new(AtomicBool::new(false));
    let catch_up: Arc<Mutex<HashSet<AbsolutePath>>> = Arc::new(Mutex::new(HashSet::new()));
    let worker_config = WorkerConfig {
        cache_root: cache_root.clone(),
        commands: lint.resolved_commands(),
        cache_size_bytes,
        status_cache: Arc::clone(&status_cache),
        paused: Arc::clone(&paused),
        catch_up: Arc::clone(&catch_up),
    };

    loop {
        match supervisor_receiver.recv() {
            Ok(SupervisorMsg::SyncProjects { projects }) => {
                emit_current_statuses(&projects, &status_cache, cache_root, background_tx);
                let desired = desired_projects(lint, &projects);
                reconcile_workers(
                    &mut workers,
                    desired,
                    &worker_config,
                    WorkerStart::Idle,
                    background_tx,
                );
            },
            Ok(SupervisorMsg::RegisterProject { project }) => {
                let abs_path = project.abs_path.clone();
                let accepted = should_watch_project(lint, &project);
                tracing::trace!(
                    target: PERF_LOG_TARGET,
                    path = %abs_path.display(),
                    label = %project.project_label,
                    accepted,
                    "lint_supervisor_register_project"
                );
                if accepted {
                    workers.entry(abs_path.clone()).or_insert_with(|| {
                        spawn_project_worker(
                            project.project_label.clone(),
                            abs_path.clone(),
                            &worker_config,
                            WorkerStart::for_discovery(lint.on_discovery),
                            background_tx.clone(),
                        )
                    });
                }
                let _ = background_tx.send(BackgroundMsg::LintStartupStatus {
                    path:   abs_path.clone(),
                    status: cached_status_for_project(&status_cache, cache_root, &abs_path),
                });
            },
            Ok(SupervisorMsg::UnregisterProject { abs_path }) => {
                if let Some(worker) = workers.remove(&abs_path) {
                    stop_worker(worker);
                    let _ = background_tx.send(BackgroundMsg::LintStatus {
                        path:   abs_path,
                        status: LintStatus::NoLog,
                        origin: LintRunOrigin::Normal,
                    });
                }
            },
            Ok(SupervisorMsg::LintTriggered { event }) => {
                if let Some(worker) = workers.get(&event.project_root) {
                    tracing::debug!(
                        project_root = %event.project_root.display(),
                        trigger = ?event.trigger,
                        event_kind = ?event.event_kind,
                        removal = event.is_removal(),
                        "lint_supervisor_trigger_dispatch"
                    );
                    let _ = worker.trigger_tx.send(event);
                } else {
                    tracing::warn!(
                        project_root = %event.project_root.display(),
                        trigger = ?event.trigger,
                        event_kind = ?event.event_kind,
                        removal = event.is_removal(),
                        workers = workers.len(),
                        "lint_supervisor_trigger_dropped_no_worker"
                    );
                }
            },
            Ok(SupervisorMsg::Pause) => pause_workers(&paused, &workers),
            Ok(SupervisorMsg::Resume) => resume_workers(&paused, &catch_up, &workers),
            Err(_) => {
                for (_, worker) in workers.drain() {
                    stop_worker(worker);
                }
                return;
            },
        }
    }
}

fn emit_current_statuses(
    projects: &[RegisterProjectRequest],
    status_cache: &Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    cache_root: &Path,
    background_tx: &Sender<BackgroundMsg>,
) {
    for request in projects {
        let _ = background_tx.send(BackgroundMsg::LintStartupStatus {
            path:   request.abs_path.clone(),
            status: cached_status_for_project(status_cache, cache_root, &request.abs_path),
        });
    }
}

fn cached_status_for_project(
    status_cache: &Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    cache_root: &Path,
    project_root: &Path,
) -> CachedLintStatus {
    let key = paths::project_key(project_root);
    if let Ok(statuses) = status_cache.lock()
        && let Some(status) = statuses.get(&key)
    {
        return status.clone();
    }
    let status = read_status_from_disk(cache_root, project_root);
    if let Ok(mut statuses) = status_cache.lock() {
        statuses.insert(key, status.clone());
    }
    status
}

/// Read a project's most recent terminal lint status from disk. Tries
/// `latest.json` first, then falls back to the newest non-`Running` line
/// from `history.jsonl`. A `Running` `latest.json` is always stale on
/// hydration (the active runtime tracks its own run in memory), so it is
/// cleared from disk and the history fallback fires. Clearing — rather than
/// only ignoring it in memory — stops external readers (the `/clippy` cache
/// check) from waiting on a run a dead app left behind.
pub(crate) fn read_status_from_disk(cache_root: &Path, project_root: &Path) -> CachedLintStatus {
    let project_dir = paths::project_dir_under(cache_root, project_root);
    let terminal_latest = match read_write::read_latest_file(&project_dir.join(LINTS_LATEST_JSON)) {
        Some(run) if matches!(run.status, LintRunStatus::Running) => {
            let _ = read_write::clear_latest_under(cache_root, project_root);
            None
        },
        other => other,
    };
    let run = terminal_latest.or_else(|| {
        read_write::read_history_file(&project_dir.join(LINTS_HISTORY_JSONL))
            .into_iter()
            .rev()
            .find(|r| !matches!(r.status, LintRunStatus::Running))
    });
    run.and_then(|run| CachedLintStatus::from_lint_status(&status::parse_run(&run)))
        .unwrap_or_default()
}

pub(super) fn desired_projects(
    lint: &LintConfig,
    projects: &[RegisterProjectRequest],
) -> HashMap<AbsolutePath, RegisterProjectRequest> {
    projects
        .iter()
        .filter(|request| should_watch_project(lint, request))
        .cloned()
        .map(|request| (request.abs_path.clone(), request))
        .collect()
}

/// Shared configuration for spawning lint workers.
pub(super) struct WorkerConfig {
    pub(super) cache_root:       AbsolutePath,
    pub(super) commands:         Vec<LintCommandConfig>,
    pub(super) cache_size_bytes: Option<u64>,
    pub(super) status_cache:     Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    /// Set while lint is paused. Every worker reads it before starting a run
    /// and bails mid-run when a pause kills its child.
    pub(super) paused:           Arc<AtomicBool>,
    /// Projects whose runs were killed or whose triggers arrived while paused.
    /// Drained on resume to re-dispatch the catch-up runs.
    pub(super) catch_up:         Arc<Mutex<HashSet<AbsolutePath>>>,
}

/// Whether a freshly spawned worker runs a lint immediately or waits idle for
/// a trigger. Startup sync always spawns workers `Idle` — the app drives any
/// startup lints after the startup phase completes (see
/// `App::kick_off_startup_lints`), so the supervisor never adds lint work to
/// the startup window. Only live post-startup discovery (`for_discovery`) runs
/// immediately, and only when discovery linting is enabled.
#[derive(Clone, Copy)]
pub(super) enum WorkerStart {
    Idle,
    RunNow,
}

impl WorkerStart {
    const fn for_discovery(discovery_lint: DiscoveryLint) -> Self {
        match discovery_lint {
            DiscoveryLint::Immediate => Self::RunNow,
            DiscoveryLint::Deferred => Self::Idle,
        }
    }

    fn initial_scheduled_run(self) -> Option<ScheduledLintRun> {
        match self {
            Self::Idle => None,
            Self::RunNow => Some(ScheduledLintRun {
                deadline: Instant::now(),
                origin:   LintRunOrigin::Normal,
            }),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ScheduledLintRun {
    pub(super) deadline: Instant,
    pub(super) origin:   LintRunOrigin,
}

impl ScheduledLintRun {
    fn coalesce(self, deadline: Instant, origin: LintRunOrigin) -> Self {
        Self {
            deadline: self.deadline.max(deadline),
            origin:   self.origin.merged_with(origin),
        }
    }
}

pub(super) fn schedule_lint_run(
    scheduled: Option<ScheduledLintRun>,
    trigger: &LintTriggerEvent,
) -> ScheduledLintRun {
    let deadline = Instant::now() + trigger.debounce();
    let origin = lint_run_origin_for_trigger(trigger);
    scheduled.map_or(ScheduledLintRun { deadline, origin }, |current| {
        current.coalesce(deadline, origin)
    })
}

const fn lint_run_origin_for_trigger(trigger: &LintTriggerEvent) -> LintRunOrigin {
    match &trigger.trigger {
        LintTriggerKind::Startup => LintRunOrigin::CatchUp,
        LintTriggerKind::Manifest | LintTriggerKind::Lockfile | LintTriggerKind::RustSource => {
            LintRunOrigin::Normal
        },
    }
}

pub(super) fn reconcile_workers(
    workers: &mut HashMap<AbsolutePath, ProjectWorker>,
    desired: HashMap<AbsolutePath, RegisterProjectRequest>,
    config: &WorkerConfig,
    start: WorkerStart,
    background_tx: &Sender<BackgroundMsg>,
) {
    let stale: Vec<AbsolutePath> = workers
        .keys()
        .filter(|path| !desired.contains_key(*path))
        .cloned()
        .collect();
    for path in stale {
        if let Some(worker) = workers.remove(&path) {
            stop_worker(worker);
            let _ = background_tx.send(BackgroundMsg::LintStatus {
                path,
                status: LintStatus::NoLog,
                origin: LintRunOrigin::Normal,
            });
        }
    }
    for (path, request) in desired {
        workers.entry(path).or_insert_with(|| {
            spawn_project_worker(
                request.project_label,
                request.abs_path,
                config,
                start,
                background_tx.clone(),
            )
        });
    }
}

pub(super) fn stop_worker(worker: ProjectWorker) {
    worker.stop.store(true, Ordering::Relaxed);
    if let Ok(mut slot) = worker.child.lock()
        && let Some(mut child) = slot.take()
    {
        kill_child_tree(&mut child);
    }
    drop(worker.trigger_tx);
    let _ = worker.handle.join();
}

/// Kill a worker's in-flight child without stopping the worker thread. Used by
/// the pause path: the worker stays alive and idle, gated by the `paused` flag.
fn kill_worker_child(child: &ChildSlot) {
    if let Ok(mut slot) = child.lock()
        && let Some(mut running_child) = slot.take()
    {
        kill_child_tree(&mut running_child);
    }
}

/// Kill a lint command's whole process group — the `/bin/sh` wrapper plus the
/// `cargo`/`rustc` descendants it spawned — then reap it. Lint commands are
/// spawned with `process_group(0)`, so the child pid is its own group id; a
/// negative target signals the group. A plain `Child::kill` would only signal
/// the shell and leave cargo running, which is what made pause look like it
/// never cancelled.
fn kill_child_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{}", child.id()))
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Record `project_root` for the resume catch-up sweep. Called when a run is
/// held back because lint is paused, or when a pause kills a run mid-flight.
fn remember_catch_up(catch_up: &Mutex<HashSet<AbsolutePath>>, project_root: &AbsolutePath) {
    if let Ok(mut pending) = catch_up.lock() {
        pending.insert(project_root.clone());
    }
}

/// Enter the paused state: stop accepting new runs and kill every in-flight
/// child. Workers stay alive and idle, gated by `paused`.
fn pause_workers(paused: &AtomicBool, workers: &HashMap<AbsolutePath, ProjectWorker>) {
    paused.store(true, Ordering::Relaxed);
    for worker in workers.values() {
        kill_worker_child(&worker.child);
    }
}

/// Leave the paused state and re-dispatch a `CatchUp`-origin run for every
/// project remembered while paused (killed mid-run or triggered by a file
/// change). Mirrors the startup staleness sweep's `Startup` trigger.
fn resume_workers(
    paused: &AtomicBool,
    catch_up: &Mutex<HashSet<AbsolutePath>>,
    workers: &HashMap<AbsolutePath, ProjectWorker>,
) {
    paused.store(false, Ordering::Relaxed);
    let pending = catch_up
        .lock()
        .map(|mut set| std::mem::take(&mut *set))
        .unwrap_or_default();
    for project_root in pending {
        if let Some(worker) = workers.get(&project_root) {
            let _ = worker.trigger_tx.send(LintTriggerEvent {
                project_root,
                trigger: LintTriggerKind::Startup,
                event_kind: LintEventKind::CreateOrModify,
            });
        }
    }
}

/// Owned per-worker state. Built by [`spawn_project_worker`] and moved into the
/// worker thread, which drives [`WorkerContext::run`] until the channel closes
/// or `stop` is set.
struct WorkerContext {
    project_root:     AbsolutePath,
    project_label:    String,
    cache_root:       AbsolutePath,
    commands:         Vec<LintCommandConfig>,
    cache_size_bytes: Option<u64>,
    status_cache:     Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    child_slot:       ChildSlot,
    background_tx:    Sender<BackgroundMsg>,
    paused:           Arc<AtomicBool>,
    catch_up:         Arc<Mutex<HashSet<AbsolutePath>>>,
    stop:             Arc<AtomicBool>,
    trigger_rx:       StdReceiver<LintTriggerEvent>,
    start:            WorkerStart,
}

impl WorkerContext {
    fn run(self) {
        let mut scheduled_run = self.start.initial_scheduled_run();
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }

            let timeout = scheduled_run.map_or(STOP_POLL, |scheduled| {
                scheduled
                    .deadline
                    .saturating_duration_since(Instant::now())
                    .min(STOP_POLL)
            });

            if let Ok(trigger) = self.trigger_rx.try_recv() {
                self.log_trigger(&trigger);
                scheduled_run = Some(schedule_lint_run(scheduled_run, &trigger));
            }

            match self.trigger_rx.recv_timeout(timeout) {
                Ok(trigger) => {
                    self.log_trigger(&trigger);
                    scheduled_run = Some(schedule_lint_run(scheduled_run, &trigger));
                },
                Err(RecvTimeoutError::Timeout) => {},
                Err(RecvTimeoutError::Disconnected) => return,
            }

            if let Some(scheduled) = scheduled_run
                && Instant::now() >= scheduled.deadline
            {
                self.run_due(scheduled.origin);
                scheduled_run = None;
            }
        }
    }

    fn log_trigger(&self, trigger: &LintTriggerEvent) {
        tracing::debug!(
            path = %self.project_root.display(),
            trigger = ?trigger.trigger,
            event_kind = ?trigger.event_kind,
            removal = trigger.is_removal(),
            "lint_worker_trigger_received"
        );
    }

    fn run_due(&self, origin: LintRunOrigin) {
        if self.paused.load(Ordering::Relaxed) {
            // Hold the run back while paused; remember the project so resume
            // re-lints it under the same catch-up policy as the startup
            // staleness sweep. Publish `Stale` so a trigger that arrives while
            // paused shows the same cancelled marker as a run the pause killed
            // mid-flight, rather than keeping its prior terminal dot.
            remember_catch_up(&self.catch_up, &self.project_root);
            publish_status(
                &self.status_cache,
                &self.project_root,
                LintStatus::Stale,
                &self.background_tx,
                origin,
            );
            return;
        }
        if self.stop.load(Ordering::Relaxed) || !project_still_runnable(&self.project_root) {
            return;
        }
        tracing::trace!(
            target: PERF_LOG_TARGET,
            path = %self.project_root.display(),
            origin = ?origin,
            "lint_worker_run_start"
        );
        let run_started = Instant::now();
        let _ = run_commands_for_project(
            &self.project_root,
            &self.project_label,
            &RunCommandsConfig {
                cache_root:       &self.cache_root,
                commands:         &self.commands,
                cache_size_bytes: self.cache_size_bytes,
                paused:           &self.paused,
            },
            &self.status_cache,
            &self.background_tx,
            &self.child_slot,
            origin,
        );
        // A pause landed mid-run and killed the child; queue the interrupted
        // project for the resume sweep.
        if self.paused.load(Ordering::Relaxed) {
            remember_catch_up(&self.catch_up, &self.project_root);
        }
        tracing::trace!(
            target: PERF_LOG_TARGET,
            path = %self.project_root.display(),
            origin = ?origin,
            duration_ms = tui_pane::perf_log_ms(run_started.elapsed().as_millis()),
            "lint_worker_run_complete"
        );
    }
}

fn spawn_project_worker(
    project_label: String,
    project_root: AbsolutePath,
    config: &WorkerConfig,
    start: WorkerStart,
    background_tx: Sender<BackgroundMsg>,
) -> ProjectWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let child: ChildSlot = Arc::new(Mutex::new(None));
    let (trigger_tx, trigger_rx) = mpsc::channel::<LintTriggerEvent>();
    let context = WorkerContext {
        project_root,
        project_label,
        cache_root: config.cache_root.clone(),
        commands: config.commands.clone(),
        cache_size_bytes: config.cache_size_bytes,
        status_cache: Arc::clone(&config.status_cache),
        child_slot: Arc::clone(&child),
        background_tx,
        paused: Arc::clone(&config.paused),
        catch_up: Arc::clone(&config.catch_up),
        stop: Arc::clone(&stop),
        trigger_rx,
        start,
    };
    let handle = thread::spawn(move || context.run());
    ProjectWorker {
        stop,
        trigger_tx,
        child,
        handle,
    }
}

pub(super) fn should_watch_project(lint: &LintConfig, request: &RegisterProjectRequest) -> bool {
    if !request.abs_path.join(CARGO_TOML).is_file() {
        return false;
    }
    if !request_matches_prefixes(&lint.include, request, false) {
        return false;
    }
    !request_matches_prefixes(&lint.exclude, request, false)
}

fn request_matches_prefixes(
    prefixes: &[String],
    request: &RegisterProjectRequest,
    empty_means_match: bool,
) -> bool {
    matches_prefixes(
        prefixes,
        &request.project_label,
        &request.abs_path,
        empty_means_match,
    ) || request.linked_primary_root.as_ref().is_some_and(|root| {
        let label = project::home_relative_path(root);
        matches_prefixes(prefixes, &label, root, false)
    })
}

fn matches_prefixes(
    prefixes: &[String],
    project_label: &str,
    abs_path: &Path,
    empty_means_match: bool,
) -> bool {
    if prefixes.is_empty() {
        return empty_means_match;
    }
    let abs = abs_path.to_string_lossy();
    prefixes.iter().any(|prefix| {
        project_label.starts_with(prefix)
            || abs.starts_with(prefix)
            || project_label
                .split('/')
                .any(|part| !part.is_empty() && part.starts_with(prefix))
            || abs_path
                .components()
                .filter_map(|component| component.as_os_str().to_str())
                .any(|part| !part.is_empty() && part.starts_with(prefix))
    })
}

pub(super) fn project_still_runnable(project_root: &Path) -> bool {
    project_root.is_dir() && project_root.join(CARGO_TOML).is_file()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::time::Duration;

    use chrono::Local;
    use crossbeam_channel::RecvTimeoutError;

    use super::*;
    use crate::channel;
    use crate::config::CargoPortConfig;
    use crate::config::LintIndicator;
    use crate::lint::LintRun;
    use crate::lint::history;
    use crate::lint::trigger::LintEventKind::CreateOrModify;
    use crate::lint::trigger::LintTriggerKind::RustSource;
    use crate::lint::trigger::LintTriggerKind::Startup;

    fn request(path: &str, abs_path: &Path) -> RegisterProjectRequest {
        RegisterProjectRequest::new(path, AbsolutePath::from(abs_path))
    }

    fn run(status: LintRunStatus) -> LintRun {
        LintRun {
            run_id: "run-1".to_string(),
            started_at: "2026-03-30T14:22:01-05:00".to_string(),
            finished_at: Some("2026-03-30T14:22:18-05:00".to_string()),
            duration_ms: Some(17_000),
            status,
            commands: Vec::new(),
            archive_bytes: 0,
        }
    }

    #[test]
    fn normal_trigger_overrides_pending_startup_origin() {
        let project_root = AbsolutePath::from(Path::new("/tmp/demo"));
        let startup = LintTriggerEvent {
            project_root: project_root.clone(),
            trigger:      Startup,
            event_kind:   CreateOrModify,
        };
        let source = LintTriggerEvent {
            project_root,
            trigger: RustSource,
            event_kind: CreateOrModify,
        };

        let scheduled = schedule_lint_run(None, &startup);
        assert_eq!(scheduled.origin, LintRunOrigin::CatchUp);
        let scheduled = schedule_lint_run(Some(scheduled), &source);
        assert_eq!(scheduled.origin, LintRunOrigin::Normal);
    }

    #[test]
    fn include_and_exclude_filters_match_display_or_absolute_paths() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let lint = LintConfig {
            enabled: LintIndicator::Enabled,
            include: vec!["~/rust/demo".to_string()],
            exclude: vec![project_dir.path().to_string_lossy().to_string()],
            commands: Vec::new(),
            ..LintConfig::default()
        };

        let req = request("~/rust/demo", project_dir.path());
        assert!(!should_watch_project(&lint, &req));

        let lint = LintConfig {
            exclude: Vec::new(),
            ..lint
        };
        assert!(should_watch_project(&lint, &req));
    }

    #[test]
    fn include_filters_match_project_name_prefixes() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");

        let lint = LintConfig {
            enabled: LintIndicator::Enabled,
            include: vec!["bevy_lagrange".to_string()],
            exclude: Vec::new(),
            commands: Vec::new(),
            ..LintConfig::default()
        };

        let direct = request("~/rust/bevy_lagrange", project_dir.path());
        let worktree = request("~/rust/bevy_lagrange_style_fix", project_dir.path());

        assert!(should_watch_project(&lint, &direct));
        assert!(should_watch_project(&lint, &worktree));
    }

    #[test]
    fn include_filters_match_linked_worktree_primary_root() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[workspace]\nmembers=[]\n",
        )
        .expect("write manifest");
        let primary_root = AbsolutePath::from(project_dir.path().join("bevy_hana"));

        let lint = LintConfig {
            enabled: LintIndicator::Enabled,
            include: vec!["bevy_hana".to_string()],
            exclude: Vec::new(),
            commands: Vec::new(),
            ..LintConfig::default()
        };
        let request =
            RegisterProjectRequest::new("~/rust/test", AbsolutePath::from(project_dir.path()))
                .with_linked_primary_root(Some(primary_root));

        assert!(should_watch_project(&lint, &request));
    }

    #[test]
    fn empty_allow_list_watches_no_projects() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let req = request("~/rust/demo", project_dir.path());
        assert!(!should_watch_project(&LintConfig::default(), &req));
    }

    #[test]
    fn desired_projects_removes_unwanted_entries() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let lint = LintConfig {
            enabled: LintIndicator::Enabled,
            include: vec!["~/rust/demo".to_string()],
            exclude: vec!["~/rust/demo/excluded".to_string()],
            commands: Vec::new(),
            ..LintConfig::default()
        };

        let desired = desired_projects(
            &lint,
            &[
                request("~/rust/demo", project_dir.path()),
                request("~/rust/demo/excluded", project_dir.path()),
                request("~/rust/not-rust", project_dir.path()),
            ],
        );

        assert_eq!(desired.len(), 1);
        assert!(desired.contains_key(project_dir.path()));
    }

    #[test]
    fn main_watcher_trigger_source_schedules_lint_runs() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        std::fs::create_dir_all(project_dir.path().join("src")).expect("create src");
        std::fs::write(project_dir.path().join("src/lib.rs"), "pub fn demo() {}\n")
            .expect("write src");

        let cache_dir = tempfile::tempdir().expect("tempdir");
        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config.cache.root = cache_dir.path().to_string_lossy().to_string();
        cargo_port_config.lint.enabled = LintIndicator::Enabled;
        cargo_port_config.lint.include = vec!["~/rust/demo".to_string()];
        cargo_port_config.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cargo_port_config, background_tx);
        let runtime = spawn.handle.expect("runtime handle");
        let request = request("~/rust/demo", project_dir.path());
        runtime.sync_projects(vec![request.clone()]);
        runtime.register_project(request);
        runtime.lint_trigger(LintTriggerEvent {
            project_root: AbsolutePath::from(project_dir.path()),
            trigger:      RustSource,
            event_kind:   CreateOrModify,
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_passed = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match background_rx.recv_timeout(remaining) {
                Ok(BackgroundMsg::LintStatus {
                    path,
                    status,
                    origin,
                }) if path.as_path() == project_dir.path()
                    && matches!(status, LintStatus::Passed(_))
                    && origin == LintRunOrigin::Normal =>
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

        assert!(
            saw_passed,
            "expected watcher-originated lint trigger to run lint"
        );
    }

    #[test]
    fn trigger_while_paused_publishes_stale_without_running() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");

        let cache_dir = tempfile::tempdir().expect("tempdir");
        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config.cache.root = cache_dir.path().to_string_lossy().to_string();
        cargo_port_config.lint.enabled = LintIndicator::Enabled;
        cargo_port_config.lint.include = vec!["~/rust/demo".to_string()];
        cargo_port_config.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cargo_port_config, background_tx);
        let runtime = spawn.handle.expect("runtime handle");
        let request = request("~/rust/demo", project_dir.path());
        runtime.sync_projects(vec![request.clone()]);
        runtime.register_project(request);
        runtime.pause();
        runtime.lint_trigger(LintTriggerEvent {
            project_root: AbsolutePath::from(project_dir.path()),
            trigger:      RustSource,
            event_kind:   CreateOrModify,
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_stale = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match background_rx.recv_timeout(remaining) {
                Ok(BackgroundMsg::LintStatus { path, status, .. })
                    if path.as_path() == project_dir.path() =>
                {
                    assert!(
                        !matches!(status, LintStatus::Running(_) | LintStatus::Passed(_)),
                        "a paused trigger must not start or finish a run, got {status:?}"
                    );
                    if matches!(status, LintStatus::Stale) {
                        saw_stale = true;
                        break;
                    }
                },
                Ok(_) => {},
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }

        assert!(
            saw_stale,
            "a trigger that arrives while paused should publish Stale"
        );
    }

    #[test]
    fn hydration_clears_stranded_running_and_falls_back_to_history() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        history::append_history_under(
            cache_dir.path(),
            project_dir.path(),
            &run(LintRunStatus::Failed),
            None,
        )
        .expect("append history");
        read_write::write_latest_under(
            cache_dir.path(),
            project_dir.path(),
            &run(LintRunStatus::Running),
        )
        .expect("write latest");

        let status = read_status_from_disk(cache_dir.path(), project_dir.path());

        assert!(
            matches!(status, CachedLintStatus::Failed(_)),
            "history fallback"
        );
        assert!(
            !paths::latest_path_under(cache_dir.path(), project_dir.path()).exists(),
            "a dead app's running marker should be cleared from disk on hydration"
        );
    }

    #[test]
    fn sync_projects_hydrates_terminal_cache_without_running_discovery_lint() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config.cache.root = cache_dir.path().to_string_lossy().to_string();
        cargo_port_config.lint.enabled = LintIndicator::Enabled;
        cargo_port_config.lint.include = vec![project_dir.path().to_string_lossy().to_string()];
        cargo_port_config.lint.on_discovery = DiscoveryLint::Immediate;
        cargo_port_config.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];
        let cache_root = cache_paths::lint_runs_root_for(&cargo_port_config);
        let finished_at = Local::now().to_rfc3339();
        let cached_run = LintRun {
            run_id:        paths::sanitize_run_id(&finished_at),
            started_at:    finished_at.clone(),
            finished_at:   Some(finished_at),
            duration_ms:   Some(1),
            status:        LintRunStatus::Passed,
            commands:      Vec::new(),
            archive_bytes: 0,
        };
        read_write::write_latest_under(cache_root.as_path(), project_dir.path(), &cached_run)
            .expect("write cached latest");

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cargo_port_config, background_tx);
        let runtime = spawn.handle.expect("runtime handle");
        runtime.sync_projects(vec![request("~/rust/demo", project_dir.path())]);

        let deadline = Instant::now() + Duration::from_secs(1);
        let mut saw_cached_passed = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match background_rx.recv_timeout(remaining) {
                Ok(BackgroundMsg::LintStartupStatus { path, status })
                    if path.as_path() == project_dir.path()
                        && matches!(status, CachedLintStatus::Passed(_)) =>
                {
                    saw_cached_passed = true;
                },
                Ok(BackgroundMsg::LintStatus { path, status, .. })
                    if path.as_path() == project_dir.path() =>
                {
                    panic!("sync should not run lint command, got {status:?}");
                },
                Ok(_) => {},
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }

        assert!(
            saw_cached_passed,
            "sync should publish the cached terminal startup lint status"
        );
    }

    #[test]
    fn sync_projects_defers_lint_even_when_immediate_and_uncached() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config.cache.root = cache_dir.path().to_string_lossy().to_string();
        cargo_port_config.lint.enabled = LintIndicator::Enabled;
        cargo_port_config.lint.include = vec![project_dir.path().to_string_lossy().to_string()];
        cargo_port_config.lint.on_discovery = DiscoveryLint::Immediate;
        cargo_port_config.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cargo_port_config, background_tx);
        let runtime = spawn.handle.expect("runtime handle");
        runtime.sync_projects(vec![request("~/rust/demo", project_dir.path())]);

        // The supervisor no longer runs lints on sync — the app drives any
        // startup lints after startup completes. Sync only hydrates the
        // cached startup status (`NoLog` here). The `echo` command would
        // resolve in well under this window if sync still ran it.
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut saw_nolog = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match background_rx.recv_timeout(remaining) {
                Ok(BackgroundMsg::LintStartupStatus { path, status })
                    if path.as_path() == project_dir.path()
                        && matches!(status, CachedLintStatus::NoLog) =>
                {
                    saw_nolog = true;
                },
                Ok(BackgroundMsg::LintStatus { path, status, .. })
                    if path.as_path() == project_dir.path() =>
                {
                    panic!("sync must not run lint under deferred startup, got {status:?}");
                },
                Ok(_) => {},
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }

        assert!(
            saw_nolog,
            "sync should hydrate the uncached startup status as NoLog"
        );
    }

    #[test]
    fn register_project_honors_immediate_discovery_lint() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config.cache.root = cache_dir.path().to_string_lossy().to_string();
        cargo_port_config.lint.enabled = LintIndicator::Enabled;
        cargo_port_config.lint.include = vec![project_dir.path().to_string_lossy().to_string()];
        cargo_port_config.lint.on_discovery = DiscoveryLint::Immediate;
        cargo_port_config.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cargo_port_config, background_tx);
        let runtime = spawn.handle.expect("runtime handle");
        runtime.register_project(request("~/rust/demo", project_dir.path()));

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_passed = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match background_rx.recv_timeout(remaining) {
                Ok(BackgroundMsg::LintStatus {
                    path,
                    status,
                    origin,
                }) if path.as_path() == project_dir.path()
                    && matches!(status, LintStatus::Passed(_))
                    && origin == LintRunOrigin::Normal =>
                {
                    saw_passed = true;
                    break;
                },
                Ok(_) => {},
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }

        assert!(
            saw_passed,
            "new project discovery should still honor immediate lint runs"
        );
    }

    #[test]
    fn reconcile_workers_stops_stale_threads() {
        let path = "/tmp/demo".into();
        let mut workers = HashMap::new();
        let (worker, exited) = dummy_worker();
        workers.insert(path, worker);
        let (background_tx, background_rx) = channel::unbounded();
        let config = WorkerConfig {
            cache_root:       "/tmp/cache".into(),
            commands:         Vec::new(),
            cache_size_bytes: None,
            status_cache:     Arc::new(Mutex::new(HashMap::new())),
            paused:           Arc::new(AtomicBool::new(false)),
            catch_up:         Arc::new(Mutex::new(HashSet::new())),
        };

        reconcile_workers(
            &mut workers,
            HashMap::new(),
            &config,
            WorkerStart::Idle,
            &background_tx,
        );

        assert!(workers.is_empty());
        assert!(exited.load(Ordering::Relaxed));
        assert!(matches!(
            background_rx.try_recv(),
            Ok(BackgroundMsg::LintStatus {
                status: LintStatus::NoLog,
                ..
            })
        ));
    }

    #[test]
    fn run_now_worker_start_creates_worker() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let request = request("~/rust/demo", project_dir.path());
        let desired = HashMap::from([(AbsolutePath::from(project_dir.path()), request)]);

        let mut workers = HashMap::new();
        let (background_tx, _) = channel::unbounded();
        let config = WorkerConfig {
            cache_root:       "/tmp/cache".into(),
            commands:         Vec::new(),
            cache_size_bytes: None,
            status_cache:     Arc::new(Mutex::new(HashMap::new())),
            paused:           Arc::new(AtomicBool::new(false)),
            catch_up:         Arc::new(Mutex::new(HashSet::new())),
        };
        reconcile_workers(
            &mut workers,
            desired,
            &config,
            WorkerStart::RunNow,
            &background_tx,
        );

        assert_eq!(workers.len(), 1);
        for (_, worker) in workers.drain() {
            stop_worker(worker);
        }
    }

    fn dummy_worker() -> (ProjectWorker, Arc<AtomicBool>) {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let exited = Arc::new(AtomicBool::new(false));
        let exited_flag = Arc::clone(&exited);
        let (trigger_tx, trigger_rx) = mpsc::channel::<LintTriggerEvent>();
        let handle = thread::spawn(move || {
            drop(trigger_rx);
            while !stop_flag.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
            }
            exited_flag.store(true, Ordering::Relaxed);
        });
        (
            ProjectWorker {
                stop,
                trigger_tx,
                child: Arc::new(Mutex::new(None)),
                handle,
            },
            exited,
        )
    }
}
