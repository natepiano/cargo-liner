use tui_pane::PERF_LOG_TARGET;

use super::AbsolutePath;
use super::Arc;
use super::AtomicBool;
use super::BackgroundMsg;
use super::CARGO_TOML;
use super::CachedLintStatus;
use super::CargoPortConfig;
use super::ChildSlot;
use super::DiscoveryLint;
use super::HashMap;
use super::Instant;
use super::JoinHandle;
use super::LINTS_HISTORY_JSONL;
use super::LINTS_LATEST_JSON;
use super::LintCommandConfig;
use super::LintConfig;
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

pub fn spawn(config: &CargoPortConfig, background_tx: Sender<BackgroundMsg>) -> SpawnResult {
    if !config.lint.enabled.is_enabled() {
        return SpawnResult {
            handle:                  None,
            warning:                 None,
            #[cfg(test)]
            supervisor:              None,
        };
    }

    let cache_root = cache_paths::lint_runs_root_for(config);
    let cache_size_bytes = config.lint.cache_size_bytes().unwrap_or(None);
    let lint = config.lint.clone();
    let (supervisor_sender, supervisor_receiver) = mpsc::channel();
    let supervisor = thread::spawn(move || {
        supervisor_loop(
            supervisor_receiver,
            cache_root,
            lint,
            cache_size_bytes,
            background_tx,
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

#[allow(
    clippy::needless_pass_by_value,
    reason = "supervisor owns its queue and worker map for the lifetime of the runtime"
)]
fn supervisor_loop(
    supervisor_receiver: StdReceiver<SupervisorMsg>,
    cache_root: AbsolutePath,
    lint: LintConfig,
    cache_size_bytes: Option<u64>,
    background_tx: Sender<BackgroundMsg>,
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
    let worker_config = WorkerConfig {
        cache_root: cache_root.clone(),
        commands: lint.resolved_commands(),
        cache_size_bytes,
        status_cache: Arc::clone(&status_cache),
    };

    loop {
        match supervisor_receiver.recv() {
            Ok(SupervisorMsg::SyncProjects { projects }) => {
                emit_current_statuses(&projects, &status_cache, &cache_root, &background_tx);
                let desired = desired_projects(&lint, &projects);
                reconcile_workers(
                    &mut workers,
                    desired,
                    &worker_config,
                    WorkerStart::Idle,
                    &background_tx,
                );
            },
            Ok(SupervisorMsg::RegisterProject { project }) => {
                let abs_path = project.abs_path.clone();
                let accepted = should_watch_project(&lint, &project);
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
                    status: cached_status_for_project(&status_cache, &cache_root, &abs_path),
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
        let _ = child.kill();
        let _ = child.wait();
    }
    drop(worker.trigger_tx);
    let _ = worker.handle.join();
}

fn spawn_project_worker(
    project_label: String,
    project_root: AbsolutePath,
    config: &WorkerConfig,
    start: WorkerStart,
    background_tx: Sender<BackgroundMsg>,
) -> ProjectWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let child: ChildSlot = Arc::new(Mutex::new(None));
    let child_slot = Arc::clone(&child);
    let (trigger_tx, trigger_rx) = mpsc::channel::<LintTriggerEvent>();
    let worker_project_label = project_label;
    let cache_root = config.cache_root.clone();
    let commands = config.commands.clone();
    let cache_size_bytes = config.cache_size_bytes;
    let status_cache = Arc::clone(&config.status_cache);
    let run_immediately = matches!(start, WorkerStart::RunNow);
    let handle = thread::spawn(move || {
        let mut scheduled_run = run_immediately.then(|| ScheduledLintRun {
            deadline: Instant::now(),
            origin:   LintRunOrigin::Normal,
        });
        loop {
            if stop_flag.load(Ordering::Relaxed) {
                return;
            }

            let timeout = scheduled_run.map_or(STOP_POLL, |scheduled| {
                scheduled
                    .deadline
                    .saturating_duration_since(Instant::now())
                    .min(STOP_POLL)
            });

            if let Ok(trigger) = trigger_rx.try_recv() {
                tracing::debug!(
                    path = %project_root.display(),
                    trigger = ?trigger.trigger,
                    event_kind = ?trigger.event_kind,
                    removal = trigger.is_removal(),
                    "lint_worker_trigger_received"
                );
                scheduled_run = Some(schedule_lint_run(scheduled_run, &trigger));
            }

            match trigger_rx.recv_timeout(timeout) {
                Ok(trigger) => {
                    tracing::debug!(
                        path = %project_root.display(),
                        trigger = ?trigger.trigger,
                        event_kind = ?trigger.event_kind,
                        removal = trigger.is_removal(),
                        "lint_worker_trigger_received"
                    );
                    scheduled_run = Some(schedule_lint_run(scheduled_run, &trigger));
                },
                Err(RecvTimeoutError::Timeout) => {},
                Err(RecvTimeoutError::Disconnected) => return,
            }

            if let Some(scheduled) = scheduled_run
                && Instant::now() >= scheduled.deadline
            {
                if !stop_flag.load(Ordering::Relaxed) && project_still_runnable(&project_root) {
                    tracing::trace!(
                        target: PERF_LOG_TARGET,
                        path = %project_root.display(),
                        origin = ?scheduled.origin,
                        "lint_worker_run_start"
                    );
                    let run_started = Instant::now();
                    let _ = run_commands_for_project(
                        &project_root,
                        &worker_project_label,
                        &RunCommandsConfig {
                            cache_root: &cache_root,
                            commands: &commands,
                            cache_size_bytes,
                        },
                        &status_cache,
                        &background_tx,
                        &child_slot,
                        scheduled.origin,
                    );
                    tracing::trace!(
                        target: PERF_LOG_TARGET,
                        path = %project_root.display(),
                        origin = ?scheduled.origin,
                        duration_ms = tui_pane::perf_log_ms(run_started.elapsed().as_millis()),
                        "lint_worker_run_complete"
                    );
                }
                scheduled_run = None;
            }
        }
    });
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
