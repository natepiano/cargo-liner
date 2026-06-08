use std::collections::HashMap;
use std::io;
use std::io::Read;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::mpsc::Receiver as StdReceiver;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::Sender as StdSender;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use chrono::Local;

use super::cache_size_index;
use super::history;
use super::paths;
use super::read_write;
use super::status;
use super::trigger::LintEventKind;
use super::trigger::LintTriggerEvent;
use super::trigger::LintTriggerKind;
use super::types::CachedLintStatus;
use super::types::LintCommand;
use super::types::LintCommandStatus;
use super::types::LintRun;
use super::types::LintRunStatus;
use super::types::LintStatus;
use crate::cache_paths;
use crate::channel::Sender;
use crate::config::CargoPortConfig;
use crate::config::DiscoveryLint;
use crate::config::LintCommandConfig;
use crate::config::LintConfig;
use crate::constants::CARGO_TOML;
use crate::constants::LINTS_HISTORY_JSONL;
use crate::constants::LINTS_LATEST_JSON;
use crate::project::AbsolutePath;
use crate::scan::BackgroundMsg;

const STOP_POLL: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct RegisterProjectRequest {
    pub project_label: String,
    pub abs_path:      AbsolutePath,
    pub is_rust:       bool,
}

pub fn project_is_eligible(
    lint: &LintConfig,
    project_label: &str,
    abs_path: &Path,
    is_rust: bool,
) -> bool {
    should_watch_project(
        lint,
        &RegisterProjectRequest {
            project_label: project_label.to_string(),
            abs_path: AbsolutePath::from(abs_path),
            is_rust,
        },
    )
}

#[derive(Clone)]
pub struct RuntimeHandle {
    tx: StdSender<SupervisorMsg>,
}

impl RuntimeHandle {
    pub fn sync_projects(&self, projects: Vec<RegisterProjectRequest>) {
        let _ = self.tx.send(SupervisorMsg::SyncProjects { projects });
    }

    pub fn register_project(&self, project: RegisterProjectRequest) {
        let _ = self.tx.send(SupervisorMsg::RegisterProject { project });
    }

    pub fn unregister_project(&self, abs_path: AbsolutePath) {
        let _ = self.tx.send(SupervisorMsg::UnregisterProject { abs_path });
    }

    pub fn lint_trigger(&self, event: LintTriggerEvent) {
        let _ = self.tx.send(SupervisorMsg::LintTriggered { event });
    }

    /// Schedule a lint run for a project the app's post-startup staleness
    /// check flagged (source newer than the last run, or never linted under
    /// immediate discovery). Routed through the same `LintTriggered` path as
    /// watcher events so the worker debounces and coalesces it normally.
    pub fn request_startup_lint(&self, project_root: AbsolutePath) {
        let _ = self.tx.send(SupervisorMsg::LintTriggered {
            event: LintTriggerEvent {
                project_root,
                trigger: LintTriggerKind::Startup,
                event_kind: LintEventKind::CreateOrModify,
                removal: false,
            },
        });
    }
}

pub struct SpawnResult {
    pub handle:  Option<RuntimeHandle>,
    pub warning: Option<String>,
}

enum SupervisorMsg {
    SyncProjects {
        projects: Vec<RegisterProjectRequest>,
    },
    RegisterProject {
        project: RegisterProjectRequest,
    },
    UnregisterProject {
        abs_path: AbsolutePath,
    },
    LintTriggered {
        event: LintTriggerEvent,
    },
}

type ChildSlot = Arc<Mutex<Option<Child>>>;

struct RunCommandsConfig<'a> {
    cache_root:       &'a Path,
    commands:         &'a [LintCommandConfig],
    cache_size_bytes: Option<u64>,
}

struct ProjectWorker {
    stop:       Arc<AtomicBool>,
    trigger_tx: StdSender<LintTriggerEvent>,
    child:      ChildSlot,
    handle:     JoinHandle<()>,
}

pub fn spawn(config: &CargoPortConfig, background_tx: Sender<BackgroundMsg>) -> SpawnResult {
    if !config.lint.enabled {
        return SpawnResult {
            handle:  None,
            warning: None,
        };
    }

    let cache_root = cache_paths::lint_runs_root_for(config);
    let cache_size_bytes = config.lint.cache_size_bytes().unwrap_or(None);
    let lint = config.lint.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || supervisor_loop(rx, cache_root, lint, cache_size_bytes, background_tx));
    SpawnResult {
        handle:  Some(RuntimeHandle { tx }),
        warning: None,
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "supervisor owns its queue and worker map for the lifetime of the runtime"
)]
fn supervisor_loop(
    rx: StdReceiver<SupervisorMsg>,
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
        match rx.recv() {
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
                tracing::info!(
                    path = %abs_path.display(),
                    label = %project.project_label,
                    is_rust = project.is_rust,
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
                    });
                }
            },
            Ok(SupervisorMsg::LintTriggered { event }) => {
                if let Some(worker) = workers.get(&event.project_root) {
                    tracing::debug!(
                        project_root = %event.project_root.display(),
                        trigger = ?event.trigger,
                        event_kind = ?event.event_kind,
                        removal = event.removal,
                        "lint_supervisor_trigger_dispatch"
                    );
                    let _ = worker.trigger_tx.send(event);
                } else {
                    tracing::warn!(
                        project_root = %event.project_root.display(),
                        trigger = ?event.trigger,
                        event_kind = ?event.event_kind,
                        removal = event.removal,
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
pub(super) fn read_status_from_disk(cache_root: &Path, project_root: &Path) -> CachedLintStatus {
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

fn desired_projects(
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
struct WorkerConfig {
    cache_root:       AbsolutePath,
    commands:         Vec<LintCommandConfig>,
    cache_size_bytes: Option<u64>,
    status_cache:     Arc<Mutex<HashMap<String, CachedLintStatus>>>,
}

/// Whether a freshly spawned worker runs a lint immediately or waits idle for
/// a trigger. Startup sync always spawns workers `Idle` — the app drives any
/// startup lints after the startup phase completes (see
/// `App::kick_off_startup_lints`), so the supervisor never adds lint work to
/// the startup window. Only live post-startup discovery (`for_discovery`) runs
/// immediately, and only when discovery linting is enabled.
#[derive(Clone, Copy)]
enum WorkerStart {
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

fn reconcile_workers(
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

fn stop_worker(worker: ProjectWorker) {
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
        let mut next_run_at = run_immediately.then(Instant::now);
        loop {
            if stop_flag.load(Ordering::Relaxed) {
                return;
            }

            let timeout = next_run_at.map_or(STOP_POLL, |deadline: Instant| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(STOP_POLL)
            });

            if let Ok(trigger) = trigger_rx.try_recv() {
                tracing::debug!(
                    path = %project_root.display(),
                    trigger = ?trigger.trigger,
                    event_kind = ?trigger.event_kind,
                    removal = trigger.removal,
                    "lint_worker_trigger_received"
                );
                next_run_at = Some(next_run_at.map_or_else(
                    || Instant::now() + trigger.debounce(),
                    |current| current.max(Instant::now() + trigger.debounce()),
                ));
            }

            match trigger_rx.recv_timeout(timeout) {
                Ok(trigger) => {
                    tracing::debug!(
                        path = %project_root.display(),
                        trigger = ?trigger.trigger,
                        event_kind = ?trigger.event_kind,
                        removal = trigger.removal,
                        "lint_worker_trigger_received"
                    );
                    next_run_at = Some(next_run_at.map_or_else(
                        || Instant::now() + trigger.debounce(),
                        |current| current.max(Instant::now() + trigger.debounce()),
                    ));
                },
                Err(RecvTimeoutError::Timeout) => {},
                Err(RecvTimeoutError::Disconnected) => return,
            }

            if let Some(deadline) = next_run_at
                && Instant::now() >= deadline
            {
                if !stop_flag.load(Ordering::Relaxed) && project_still_runnable(&project_root) {
                    tracing::info!(
                        path = %project_root.display(),
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
                    );
                    tracing::info!(
                        path = %project_root.display(),
                        duration_ms = tui_pane::perf_log_ms(run_started.elapsed().as_millis()),
                        "lint_worker_run_complete"
                    );
                }
                next_run_at = None;
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

fn should_watch_project(lint: &LintConfig, request: &RegisterProjectRequest) -> bool {
    if !request.is_rust || !request.abs_path.join(CARGO_TOML).is_file() {
        return false;
    }
    if !matches_prefixes(
        &lint.include,
        &request.project_label,
        &request.abs_path,
        false,
    ) {
        return false;
    }
    !matches_prefixes(
        &lint.exclude,
        &request.project_label,
        &request.abs_path,
        false,
    )
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

fn project_still_runnable(project_root: &Path) -> bool {
    project_root.is_dir() && project_root.join(CARGO_TOML).is_file()
}

struct CommandExecution {
    success:     bool,
    exit_code:   Option<i32>,
    duration_ms: u64,
}

/// Clears a stranded `Running` `latest.json` if the run never reaches its
/// terminal write — an early return, a panic, or the worker being joined
/// mid-command when the app shuts down. A completed run has already rewritten
/// the marker to `Passed`/`Failed`, so the drop is a no-op for it. Without
/// this, a run interrupted between the initial `Running` write and the
/// terminal write strands the marker, and external readers (the `/clippy`
/// cache check) wait on a run that will never finish.
pub(super) struct RunFinalizeGuard<'a> {
    pub(super) cache_root:    &'a Path,
    pub(super) project_root:  &'a Path,
    pub(super) status_cache:  &'a Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    pub(super) background_tx: &'a Sender<BackgroundMsg>,
}

impl Drop for RunFinalizeGuard<'_> {
    fn drop(&mut self) {
        let Ok(cleared) =
            read_write::clear_latest_if_running_under(self.cache_root, self.project_root)
        else {
            return;
        };
        if cleared {
            publish_status(
                self.status_cache,
                self.project_root,
                read_status_from_disk(self.cache_root, self.project_root).into_lint_status(),
                self.background_tx,
            );
        }
    }
}

/// Build the initial `Running` run record with one `Pending` entry per
/// command. The run id doubles as the `runs/{run_id}` archive directory name,
/// so it is sanitized to be path-safe — the raw RFC3339 timestamp has `:`,
/// which is illegal on Windows. `started_at` keeps the unsanitized timestamp.
fn build_pending_run(commands: &[LintCommandConfig], started_at_str: String) -> LintRun {
    LintRun {
        run_id:        paths::sanitize_run_id(&started_at_str),
        started_at:    started_at_str,
        finished_at:   None,
        duration_ms:   None,
        status:        LintRunStatus::Running,
        commands:      commands
            .iter()
            .enumerate()
            .map(|(index, command)| {
                let log_name = command_log_name(command, index);
                LintCommand {
                    name:        if command.name.trim().is_empty() {
                        log_name.clone()
                    } else {
                        command.name.trim().to_string()
                    },
                    command:     command.command.clone(),
                    status:      LintCommandStatus::Pending,
                    duration_ms: None,
                    exit_code:   None,
                    log_file:    format!("{log_name}-latest.log"),
                }
            })
            .collect(),
        archive_bytes: 0,
    }
}

fn run_commands_for_project(
    project_root: &Path,
    project_label: &str,
    config: &RunCommandsConfig<'_>,
    status_cache: &Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    background_tx: &Sender<BackgroundMsg>,
    child_slot: &ChildSlot,
) -> io::Result<()> {
    if !project_still_runnable(project_root) {
        return Ok(());
    }

    let cache_root = config.cache_root;
    let commands = config.commands;
    let cache_size_bytes = config.cache_size_bytes;
    let output_dir = paths::output_dir_under(cache_root, project_root);
    std::fs::create_dir_all(&output_dir)?;
    let run_started = Instant::now();
    let mut run = build_pending_run(commands, Local::now().to_rfc3339());
    read_write::write_latest_under(cache_root, project_root, &run)?;
    let _finalize = RunFinalizeGuard {
        cache_root,
        project_root,
        status_cache,
        background_tx,
    };
    tracing::info!(
        path = project_label,
        abs_path = %project_root.display(),
        "startup_lint_started"
    );
    publish_status(
        status_cache,
        project_root,
        status::read_status_under(cache_root, project_root),
        background_tx,
    );

    let result = execute_commands(
        project_root,
        cache_root,
        commands,
        &output_dir,
        &mut run,
        child_slot,
    )?;
    if matches!(result, CommandsResult::ProjectRemoved) {
        let _ = read_write::clear_latest_under(cache_root, project_root);
        publish_status(status_cache, project_root, LintStatus::NoLog, background_tx);
        return Ok(());
    }

    run.finished_at = Some(Local::now().to_rfc3339());
    run.duration_ms = Some(u64::try_from(run_started.elapsed().as_millis()).unwrap_or(u64::MAX));
    run.status = match result {
        CommandsResult::AllPassed => LintRunStatus::Passed,
        CommandsResult::SomeFailed | CommandsResult::ProjectRemoved => LintRunStatus::Failed,
    };

    write_terminal_run(
        cache_root,
        project_root,
        run,
        cache_size_bytes,
        background_tx,
    )?;
    publish_status(
        status_cache,
        project_root,
        status::read_status_under(cache_root, project_root),
        background_tx,
    );
    Ok(())
}

/// Persist a finished run: archive its logs to the per-run directory, write
/// the terminal `latest.json`, then append to history. Archiving and the
/// history append are best-effort — on archive failure the un-archived run is
/// kept (its `log_file` still points at the rolling `*-latest.log`, which
/// exists). The terminal `latest.json` write is the one that must land, so an
/// archive error never strands the run at `Running` and spins the UI forever.
fn write_terminal_run(
    cache_root: &Path,
    project_root: &Path,
    mut run: LintRun,
    cache_size_bytes: Option<u64>,
    background_tx: &Sender<BackgroundMsg>,
) -> io::Result<()> {
    match history::archive_run_output(cache_root, project_root, &run) {
        Ok(archived) => run = archived,
        Err(err) => tracing::warn!(
            path = %project_root.display(),
            error = %err,
            "lint_archive_failed"
        ),
    }
    read_write::write_latest_under(cache_root, project_root, &run)?;
    match history::append_history_under(cache_root, project_root, &run, cache_size_bytes) {
        Ok(prune_stats) if prune_stats.runs_evicted > 0 => {
            let _ = background_tx.send(BackgroundMsg::LintCachePruned {
                runs_evicted:    prune_stats.runs_evicted,
                bytes_reclaimed: prune_stats.bytes_reclaimed,
            });
        },
        Ok(_) => {},
        Err(err) => tracing::warn!(
            path = %project_root.display(),
            error = %err,
            "lint_history_append_failed"
        ),
    }
    Ok(())
}

enum CommandsResult {
    AllPassed,
    SomeFailed,
    ProjectRemoved,
}

fn execute_commands(
    project_root: &Path,
    cache_root: &Path,
    commands: &[LintCommandConfig],
    output_dir: &Path,
    run: &mut LintRun,
    child_slot: &ChildSlot,
) -> io::Result<CommandsResult> {
    let manifest_path = project_root.join(CARGO_TOML);
    let mut failed = false;
    for (index, command) in commands.iter().enumerate() {
        if !project_still_runnable(project_root) {
            return Ok(CommandsResult::ProjectRemoved);
        }
        let cmd_started = Instant::now();
        let execution = run_command(
            project_root,
            &manifest_path,
            cache_root,
            output_dir,
            command,
            index,
            child_slot,
        )?;
        tracing::info!(
            command = %command.name,
            duration_ms = tui_pane::perf_log_ms(cmd_started.elapsed().as_millis()),
            success = execution.success,
            path = %project_root.display(),
            "lint_command_finished"
        );
        if let Some(command_run) = run.commands.get_mut(index) {
            command_run.status = if execution.success {
                LintCommandStatus::Passed
            } else {
                LintCommandStatus::Failed
            };
            command_run.duration_ms = Some(execution.duration_ms);
            command_run.exit_code = execution.exit_code;
        }
        read_write::write_latest_under(cache_root, project_root, run)?;
        if !execution.success {
            failed = true;
        }
    }
    if !project_still_runnable(project_root) {
        return Ok(CommandsResult::ProjectRemoved);
    }
    if failed {
        Ok(CommandsResult::SomeFailed)
    } else {
        Ok(CommandsResult::AllPassed)
    }
}

fn publish_status(
    status_cache: &Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    project_root: &Path,
    status: LintStatus,
    background_tx: &Sender<BackgroundMsg>,
) {
    if let Ok(mut statuses) = status_cache.lock() {
        let key = paths::project_key(project_root);
        if let Some(cached_status) = CachedLintStatus::from_lint_status(&status) {
            statuses.insert(key, cached_status);
        }
    }
    let _ = background_tx.send(BackgroundMsg::LintStatus {
        path: AbsolutePath::from(project_root),
        status,
    });
}

/// Substitute the lint placeholder variables (`$NAME` and `${NAME}`) in
/// `command` with their resolved paths. Done in Rust rather than relying on
/// the shell so commands behave identically under `/bin/sh` and `cmd.exe`
/// (the latter does not expand `$NAME`). The matching variables are still set
/// on the child env below, so user-authored variables keep working through
/// whichever shell runs the command.
fn expand_lint_placeholders(
    command: &str,
    project_root: &Path,
    manifest_path: &Path,
    output_dir: &Path,
) -> String {
    let mut expanded = command.to_string();
    for (name, path) in [
        ("PROJECT_DIR", project_root),
        ("MANIFEST_PATH", manifest_path),
        ("LINT_OUTPUT_DIR", output_dir),
    ] {
        let value = path.to_string_lossy();
        expanded = expanded.replace(&format!("${{{name}}}"), value.as_ref());
        expanded = expanded.replace(&format!("${name}"), value.as_ref());
    }
    expanded
}

/// Build the shell `Command` that runs a lint command line. `/bin/sh` does
/// not exist on Windows (spawn fails with os error 3), so route through
/// `cmd /C` there. The command is passed verbatim via `raw_arg`, wrapped in an
/// outer quote pair: `cmd` strips the outer pair and preserves inner quotes
/// (e.g. around a manifest path with spaces) that its default arg quoting
/// would otherwise pass through to the program literally.
#[cfg(windows)]
fn lint_shell(command_line: &str) -> Command {
    let mut shell = Command::new("cmd");
    shell.raw_arg(format!("/C \"{command_line}\""));
    shell
}

#[cfg(not(windows))]
fn lint_shell(command_line: &str) -> Command {
    let mut shell = Command::new("/bin/sh");
    shell.arg("-c").arg(command_line);
    shell
}

fn run_command(
    project_root: &Path,
    manifest_path: &Path,
    cache_root: &Path,
    output_dir: &Path,
    command: &LintCommandConfig,
    index: usize,
    child_slot: &ChildSlot,
) -> io::Result<CommandExecution> {
    let log_name = command_log_name(command, index);
    let log_path = output_dir.join(format!("{log_name}-latest.log"));
    let tmp_path = output_dir.join(format!("{log_name}-latest.log.tmp"));

    let started = Instant::now();
    let expanded =
        expand_lint_placeholders(&command.command, project_root, manifest_path, output_dir);
    let spawn_result = lint_shell(&expanded)
        .current_dir(project_root)
        .env("PROJECT_DIR", project_root)
        .env("MANIFEST_PATH", manifest_path)
        .env("LINT_OUTPUT_DIR", output_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let (success, exit_code, bytes) = match spawn_result {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            if let Ok(mut slot) = child_slot.lock() {
                *slot = Some(child);
            }
            let stdout_join = thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut s) = stdout {
                    let _ = s.read_to_end(&mut buf);
                }
                buf
            });
            let stderr_join = thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut s) = stderr {
                    let _ = s.read_to_end(&mut buf);
                }
                buf
            });
            let mut bytes = stdout_join.join().unwrap_or_default();
            bytes.extend(stderr_join.join().unwrap_or_default());
            let taken = child_slot.lock().ok().and_then(|mut slot| slot.take());
            match taken {
                Some(mut child) => match child.wait() {
                    Ok(status) => (status.success(), status.code(), bytes),
                    Err(err) => (
                        false,
                        None,
                        format!(
                            "failed to await lint command '{}': {err}\n",
                            command.command
                        )
                        .into_bytes(),
                    ),
                },
                None => (false, None, bytes),
            }
        },
        Err(err) => (
            false,
            None,
            format!(
                "failed to spawn lint command '{}': {err}\n",
                command.command
            )
            .into_bytes(),
        ),
    };

    let old_size = cache_size_index::file_size_or_zero(&log_path);
    std::fs::write(&tmp_path, bytes)?;
    std::fs::rename(tmp_path, &log_path)?;
    let new_size = cache_size_index::file_size_or_zero(&log_path);
    cache_size_index::apply_write_delta(cache_root, old_size, new_size);
    Ok(CommandExecution {
        success,
        exit_code,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

fn command_log_name(command: &LintCommandConfig, index: usize) -> String {
    let base = if command.name.trim().is_empty() {
        format!("command-{}", index + 1)
    } else {
        command.name.trim().to_string()
    };
    let sanitized = sanitize_name(&base);
    if sanitized.is_empty() {
        format!("command-{}", index + 1)
    } else {
        sanitized
    }
}

fn sanitize_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    sanitized.trim_matches('-').to_string()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use crossbeam_channel::RecvTimeoutError;

    use super::*;
    use crate::channel;
    use crate::config::CargoPortConfig;
    use crate::lint::trigger::LintEventKind::CreateOrModify;
    use crate::lint::trigger::LintTriggerKind::RustSource;

    fn request(path: &str, abs_path: &Path, is_rust: bool) -> RegisterProjectRequest {
        RegisterProjectRequest {
            project_label: path.to_string(),
            abs_path: AbsolutePath::from(abs_path),
            is_rust,
        }
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
            enabled: true,
            include: vec!["~/rust/demo".to_string()],
            exclude: vec![project_dir.path().to_string_lossy().to_string()],
            commands: Vec::new(),
            ..LintConfig::default()
        };

        let req = request("~/rust/demo", project_dir.path(), true);
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
            enabled: true,
            include: vec!["bevy_lagrange".to_string()],
            exclude: Vec::new(),
            commands: Vec::new(),
            ..LintConfig::default()
        };

        let direct = request("~/rust/bevy_lagrange", project_dir.path(), true);
        let worktree = request("~/rust/bevy_lagrange_style_fix", project_dir.path(), true);

        assert!(should_watch_project(&lint, &direct));
        assert!(should_watch_project(&lint, &worktree));
    }

    #[test]
    fn non_rust_projects_are_never_watched() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        let req = request("~/rust/not-rust", project_dir.path(), false);
        assert!(!should_watch_project(&LintConfig::default(), &req));
    }

    #[test]
    fn empty_allow_list_watches_no_projects() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let req = request("~/rust/demo", project_dir.path(), true);
        assert!(!should_watch_project(&LintConfig::default(), &req));
    }

    #[test]
    fn lint_commands_write_reports_under_configured_cache_root() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");

        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        let cache_root = cache_paths::lint_runs_root_for(&cfg);
        let commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (tx, _rx) = channel::unbounded();
        run_commands_for_project(
            project_dir.path(),
            "~/rust/demo",
            &RunCommandsConfig {
                cache_root:       cache_root.as_path(),
                commands:         &commands,
                cache_size_bytes: None,
            },
            &Arc::new(Mutex::new(HashMap::new())),
            &tx,
            &Arc::new(Mutex::new(None)),
        )
        .expect("run commands");

        let report_dir = paths::output_dir_under(&cache_root, project_dir.path());
        let latest_path = paths::latest_path_under(&cache_root, project_dir.path());
        let history_path = paths::history_path_under(&cache_root, project_dir.path());
        let report = std::fs::read_to_string(report_dir.join("echo-latest.log"))
            .expect("read command report");
        let latest = std::fs::read_to_string(latest_path).expect("read latest report");
        let history = std::fs::read_to_string(history_path).expect("read history report");

        // `cmd`'s `echo` emits `\r\n`; normalize so the check is host-agnostic.
        assert_eq!(report.replace("\r\n", "\n"), "lint ok\n");
        assert!(latest.contains("\"status\": \"passed\""));
        assert!(history.contains("\"status\":\"passed\""));
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
            enabled: true,
            include: vec!["~/rust/demo".to_string()],
            exclude: vec!["~/rust/demo/excluded".to_string()],
            commands: Vec::new(),
            ..LintConfig::default()
        };

        let desired = desired_projects(
            &lint,
            &[
                request("~/rust/demo", project_dir.path(), true),
                request("~/rust/demo/excluded", project_dir.path(), true),
                request("~/rust/not-rust", project_dir.path(), false),
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
        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        cfg.lint.enabled = true;
        cfg.lint.include = vec!["~/rust/demo".to_string()];
        cfg.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cfg, background_tx);
        let runtime = spawn.handle.expect("runtime handle");
        let request = request("~/rust/demo", project_dir.path(), true);
        runtime.sync_projects(vec![request.clone()]);
        runtime.register_project(request);
        runtime.lint_trigger(LintTriggerEvent {
            project_root: AbsolutePath::from(project_dir.path()),
            trigger:      RustSource,
            event_kind:   CreateOrModify,
            removal:      false,
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_passed = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match background_rx.recv_timeout(remaining) {
                Ok(BackgroundMsg::LintStatus { path, status })
                    if path.as_path() == project_dir.path()
                        && matches!(status, LintStatus::Passed(_)) =>
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
    fn sync_projects_hydrates_terminal_cache_without_running_discovery_lint() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        cfg.lint.enabled = true;
        cfg.lint.include = vec![project_dir.path().to_string_lossy().to_string()];
        cfg.lint.on_discovery = DiscoveryLint::Immediate;
        cfg.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];
        let cache_root = cache_paths::lint_runs_root_for(&cfg);
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
        let spawn = spawn(&cfg, background_tx);
        let runtime = spawn.handle.expect("runtime handle");
        runtime.sync_projects(vec![request("~/rust/demo", project_dir.path(), true)]);

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
                Ok(BackgroundMsg::LintStatus { path, status })
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
        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        cfg.lint.enabled = true;
        cfg.lint.include = vec![project_dir.path().to_string_lossy().to_string()];
        cfg.lint.on_discovery = DiscoveryLint::Immediate;
        cfg.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cfg, background_tx);
        let runtime = spawn.handle.expect("runtime handle");
        runtime.sync_projects(vec![request("~/rust/demo", project_dir.path(), true)]);

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
                Ok(BackgroundMsg::LintStatus { path, status })
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
        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        cfg.lint.enabled = true;
        cfg.lint.include = vec![project_dir.path().to_string_lossy().to_string()];
        cfg.lint.on_discovery = DiscoveryLint::Immediate;
        cfg.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cfg, background_tx);
        let runtime = spawn.handle.expect("runtime handle");
        runtime.register_project(request("~/rust/demo", project_dir.path(), true));

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_passed = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match background_rx.recv_timeout(remaining) {
                Ok(BackgroundMsg::LintStatus { path, status })
                    if path.as_path() == project_dir.path()
                        && matches!(status, LintStatus::Passed(_)) =>
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
    fn run_commands_skips_non_projects_before_writing_status() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (tx, _rx) = channel::unbounded();
        run_commands_for_project(
            project_dir.path(),
            "~/rust/demo",
            &RunCommandsConfig {
                cache_root:       cache_dir.path(),
                commands:         &commands,
                cache_size_bytes: None,
            },
            &Arc::new(Mutex::new(HashMap::new())),
            &tx,
            &Arc::new(Mutex::new(None)),
        )
        .expect("run commands");

        let latest_path = paths::latest_path_under(cache_dir.path(), project_dir.path());
        let history_path = paths::history_path_under(cache_dir.path(), project_dir.path());
        assert!(!latest_path.exists());
        assert!(!history_path.exists());
    }

    #[test]
    fn finalize_guard_publishes_terminal_status_for_stranded_running_marker() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let run = build_pending_run(&[], Local::now().to_rfc3339());
        read_write::write_latest_under(cache_dir.path(), project_dir.path(), &run)
            .expect("write running latest");
        let status_cache = Arc::new(Mutex::new(HashMap::new()));
        let (background_tx, background_rx) = channel::unbounded();

        {
            let _guard = RunFinalizeGuard {
                cache_root:    cache_dir.path(),
                project_root:  project_dir.path(),
                status_cache:  &status_cache,
                background_tx: &background_tx,
            };
        }

        assert!(matches!(
            background_rx.try_recv(),
            Ok(BackgroundMsg::LintStatus {
                status: LintStatus::NoLog,
                ..
            })
        ));
        assert!(matches!(
            read_status_from_disk(cache_dir.path(), project_dir.path()),
            CachedLintStatus::NoLog
        ));
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
        let request = request("~/rust/demo", project_dir.path(), true);
        let desired = HashMap::from([(AbsolutePath::from(project_dir.path()), request)]);

        let mut workers = HashMap::new();
        let (background_tx, _) = channel::unbounded();
        let config = WorkerConfig {
            cache_root:       "/tmp/cache".into(),
            commands:         Vec::new(),
            cache_size_bytes: None,
            status_cache:     Arc::new(Mutex::new(HashMap::new())),
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
