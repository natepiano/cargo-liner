use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use chrono::Local;
use notify::RecursiveMode;
use notify::Watcher;

use super::history;
use super::paths;
use super::read_write;
use super::status;
use super::types::LintCommand;
use super::types::LintCommandStatus;
use super::types::LintRun;
use super::types::LintRunStatus;
use super::types::LintStatus;
use crate::cache_paths;
use crate::config::CargoPortConfig;
use crate::config::DiscoveryLint;
use crate::config::LintCommandConfig;
use crate::config::LintConfig;
use crate::constants::LINTS_HISTORY_JSONL;
use crate::constants::LINTS_LATEST_JSON;
use crate::project::AbsolutePath;
use crate::scan::BackgroundMsg;

const LINT_DEBOUNCE: Duration = Duration::from_millis(750);
const DELETE_LINT_DEBOUNCE: Duration = Duration::from_millis(1500);
const STOP_POLL: Duration = Duration::from_millis(250);

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

pub struct RuntimeHandle {
    tx: mpsc::Sender<SupervisorMsg>,
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
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) { let _ = self.tx.send(SupervisorMsg::Shutdown); }
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
    Shutdown,
}

struct ProjectWorker {
    stop:   Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

pub fn spawn(config: &CargoPortConfig, bg_tx: mpsc::Sender<BackgroundMsg>) -> SpawnResult {
    if !config.lint.enabled {
        return SpawnResult {
            handle:  None,
            warning: None,
        };
    }

    let cache_root = AbsolutePath::from(cache_paths::lint_runs_root_for(config));
    let cache_size_bytes = config.lint.cache_size_bytes().unwrap_or(None);
    let lint = config.lint.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || supervisor_loop(rx, cache_root, lint, cache_size_bytes, bg_tx));
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
    rx: mpsc::Receiver<SupervisorMsg>,
    cache_root: AbsolutePath,
    lint: LintConfig,
    cache_size_bytes: Option<u64>,
    bg_tx: mpsc::Sender<BackgroundMsg>,
) {
    let mut workers: HashMap<AbsolutePath, ProjectWorker> = HashMap::new();
    let _ = read_write::clear_running_latest_files_under(&cache_root);
    let status_cache = Arc::new(Mutex::new(hydrate_status_cache(&cache_root)));
    let worker_config = WorkerConfig {
        cache_root,
        commands: lint.resolved_commands(),
        cache_size_bytes,
        on_discovery: lint.on_discovery,
        status_cache: Arc::clone(&status_cache),
    };

    loop {
        match rx.recv() {
            Ok(SupervisorMsg::SyncProjects { projects }) => {
                let desired = desired_projects(&lint, projects);
                emit_current_statuses(&desired, &status_cache, &bg_tx);
                reconcile_workers(&mut workers, desired, &worker_config, &bg_tx);
            },
            Ok(SupervisorMsg::RegisterProject { project }) => {
                if should_watch_project(&lint, &project) {
                    let abs_path = project.abs_path.clone();
                    workers.entry(abs_path.clone()).or_insert_with(|| {
                        spawn_project_worker(
                            project.project_label.clone(),
                            abs_path.clone(),
                            &worker_config,
                            bg_tx.clone(),
                        )
                    });
                    let _ = bg_tx.send(BackgroundMsg::LintStatus {
                        path:   abs_path.clone(),
                        status: cached_status_for_project(&status_cache, &abs_path),
                    });
                }
            },
            Ok(SupervisorMsg::UnregisterProject { abs_path }) => {
                if let Some(worker) = workers.remove(&abs_path) {
                    stop_worker(worker);
                    let _ = bg_tx.send(BackgroundMsg::LintStatus {
                        path:   abs_path,
                        status: LintStatus::NoLog,
                    });
                }
            },
            Ok(SupervisorMsg::Shutdown) | Err(_) => {
                for (_, worker) in workers.drain() {
                    stop_worker(worker);
                }
                return;
            },
        }
    }
}

fn hydrate_status_cache(cache_root: &Path) -> HashMap<String, LintStatus> {
    let Ok(entries) = std::fs::read_dir(cache_root) else {
        return HashMap::new();
    };

    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let dir = entry.path();
            let run = read_write::read_latest_file(&dir.join(LINTS_LATEST_JSON)).or_else(|| {
                // `latest.json` may be missing if the app was killed mid-lint and
                // the stale file was cleaned up. Fall back to the last completed
                // run from history so the status icon isn't lost.
                read_write::read_history_file(&dir.join(LINTS_HISTORY_JSONL))
                    .into_iter()
                    .rev()
                    .find(|r| !matches!(r.status, LintRunStatus::Running))
            })?;
            let status = status::parse_run(&run);
            if matches!(status, LintStatus::NoLog) {
                return None;
            }
            Some((entry.file_name().to_string_lossy().to_string(), status))
        })
        .collect()
}

fn emit_current_statuses(
    desired: &HashMap<AbsolutePath, RegisterProjectRequest>,
    status_cache: &Arc<Mutex<HashMap<String, LintStatus>>>,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
) {
    for request in desired.values() {
        let _ = bg_tx.send(BackgroundMsg::LintStatus {
            path:   request.abs_path.clone(),
            status: cached_status_for_project(status_cache, &request.abs_path),
        });
    }
}

fn cached_status_for_project(
    status_cache: &Arc<Mutex<HashMap<String, LintStatus>>>,
    project_root: &Path,
) -> LintStatus {
    status_cache
        .lock()
        .ok()
        .and_then(|statuses| statuses.get(&paths::project_key(project_root)).cloned())
        .unwrap_or(LintStatus::NoLog)
}

fn desired_projects(
    lint: &LintConfig,
    projects: Vec<RegisterProjectRequest>,
) -> HashMap<AbsolutePath, RegisterProjectRequest> {
    projects
        .into_iter()
        .filter(|request| should_watch_project(lint, request))
        .map(|request| (request.abs_path.clone(), request))
        .collect()
}

/// Shared configuration for spawning lint workers.
struct WorkerConfig {
    cache_root:       AbsolutePath,
    commands:         Vec<LintCommandConfig>,
    cache_size_bytes: Option<u64>,
    on_discovery:     DiscoveryLint,
    status_cache:     Arc<Mutex<HashMap<String, LintStatus>>>,
}

fn reconcile_workers(
    workers: &mut HashMap<AbsolutePath, ProjectWorker>,
    desired: HashMap<AbsolutePath, RegisterProjectRequest>,
    config: &WorkerConfig,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
) {
    let stale: Vec<AbsolutePath> = workers
        .keys()
        .filter(|path| !desired.contains_key(*path))
        .cloned()
        .collect();
    for path in stale {
        if let Some(worker) = workers.remove(&path) {
            stop_worker(worker);
            let _ = bg_tx.send(BackgroundMsg::LintStatus {
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
                bg_tx.clone(),
            )
        });
    }
}

fn stop_worker(worker: ProjectWorker) {
    worker.stop.store(true, Ordering::Relaxed);
    let _ = worker.handle.join();
}

fn spawn_project_worker(
    project_label: String,
    project_root: AbsolutePath,
    config: &WorkerConfig,
    bg_tx: mpsc::Sender<BackgroundMsg>,
) -> ProjectWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let worker_project_label = project_label;
    let cache_root = config.cache_root.clone();
    let commands = config.commands.clone();
    let cache_size_bytes = config.cache_size_bytes;
    let status_cache = Arc::clone(&config.status_cache);
    let run_immediately = matches!(config.on_discovery, DiscoveryLint::Immediate);
    let handle = thread::spawn(move || {
        let (event_tx, event_rx) = mpsc::channel();
        let handler = move |res| {
            let _ = event_tx.send(res);
        };
        let Ok(mut watcher) = notify::recommended_watcher(handler) else {
            return;
        };
        if watcher
            .watch(&project_root, RecursiveMode::Recursive)
            .is_err()
        {
            return;
        }

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

            match event_rx.recv_timeout(timeout) {
                Ok(Ok(event)) => {
                    if let Some(debounce) = event_debounce(&project_root, &event) {
                        next_run_at = Some(next_run_at.map_or_else(
                            || Instant::now() + debounce,
                            |current| current.max(Instant::now() + debounce),
                        ));
                    }
                },
                Ok(Err(_)) => {},
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(deadline) = next_run_at
                        && Instant::now() >= deadline
                    {
                        if project_still_runnable(&project_root) {
                            tracing::info!(
                                path = %project_root.display(),
                                "lint_worker_run_start"
                            );
                            let run_started = Instant::now();
                            let _ = run_commands_for_project(
                                &project_root,
                                &worker_project_label,
                                &cache_root,
                                &commands,
                                cache_size_bytes,
                                &status_cache,
                                &bg_tx,
                            );
                            tracing::info!(
                                path = %project_root.display(),
                                duration_ms = crate::perf_log::ms(run_started.elapsed().as_millis()),
                                "lint_worker_run_complete"
                            );
                        }
                        next_run_at = None;
                    }
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    ProjectWorker { stop, handle }
}

fn should_watch_project(lint: &LintConfig, request: &RegisterProjectRequest) -> bool {
    if !request.is_rust || !request.abs_path.join("Cargo.toml").is_file() {
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

fn is_relevant_change(project_root: &Path, path: &Path) -> bool {
    if !path.starts_with(project_root) {
        return false;
    }
    if path.components().any(|component| {
        let part = component.as_os_str();
        part == "target" || part == ".git"
    }) {
        return false;
    }
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    file_name == "Cargo.toml"
        || file_name == "Cargo.lock"
        || path.extension().is_some_and(|ext| ext == "rs")
}

fn event_debounce(project_root: &Path, event: &notify::Event) -> Option<Duration> {
    if !event
        .paths
        .iter()
        .any(|path| is_relevant_change(project_root, path))
    {
        return None;
    }
    if matches!(event.kind, notify::event::EventKind::Remove(_)) {
        Some(DELETE_LINT_DEBOUNCE)
    } else {
        Some(LINT_DEBOUNCE)
    }
}

fn project_still_runnable(project_root: &Path) -> bool {
    project_root.is_dir() && project_root.join("Cargo.toml").is_file()
}

struct CommandExecution {
    success:     bool,
    exit_code:   Option<i32>,
    duration_ms: u64,
}

pub fn run_commands_for_project(
    project_root: &Path,
    project_label: &str,
    cache_root: &Path,
    commands: &[LintCommandConfig],
    cache_size_bytes: Option<u64>,
    status_cache: &Arc<Mutex<HashMap<String, LintStatus>>>,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
) -> io::Result<()> {
    if !project_still_runnable(project_root) {
        return Ok(());
    }

    let output_dir = paths::output_dir_under(cache_root, project_root);
    std::fs::create_dir_all(&output_dir)?;
    let started_at = Local::now();
    let started_at_str = started_at.to_rfc3339();
    let run_started = Instant::now();
    let mut run = LintRun {
        run_id:      started_at_str.clone(),
        started_at:  started_at_str,
        finished_at: None,
        duration_ms: None,
        status:      LintRunStatus::Running,
        commands:    commands
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
    };
    read_write::write_latest_under(cache_root, project_root, &run)?;
    tracing::info!(
        path = project_label,
        abs_path = %project_root.display(),
        "startup_lint_started"
    );
    publish_status(
        status_cache,
        project_root,
        status::read_status_under(cache_root, project_root),
        bg_tx,
    );

    let result = execute_commands(project_root, cache_root, commands, &output_dir, &mut run)?;
    if matches!(result, CommandsResult::ProjectRemoved) {
        let _ = read_write::clear_latest_under(cache_root, project_root);
        publish_status(status_cache, project_root, LintStatus::NoLog, bg_tx);
        return Ok(());
    }

    run.finished_at = Some(Local::now().to_rfc3339());
    run.duration_ms = Some(u64::try_from(run_started.elapsed().as_millis()).unwrap_or(u64::MAX));
    run.status = match result {
        CommandsResult::AllPassed => LintRunStatus::Passed,
        CommandsResult::SomeFailed | CommandsResult::ProjectRemoved => LintRunStatus::Failed,
    };

    run = history::archive_run_output(cache_root, project_root, &run)?;
    read_write::write_latest_under(cache_root, project_root, &run)?;
    let prune_stats =
        history::append_history_under(cache_root, project_root, &run, cache_size_bytes)?;
    if prune_stats.runs_evicted > 0 {
        let _ = bg_tx.send(BackgroundMsg::LintCachePruned {
            runs_evicted:    prune_stats.runs_evicted,
            bytes_reclaimed: prune_stats.bytes_reclaimed,
        });
    }
    publish_status(
        status_cache,
        project_root,
        status::read_status_under(cache_root, project_root),
        bg_tx,
    );
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
) -> io::Result<CommandsResult> {
    let manifest_path = project_root.join("Cargo.toml");
    let mut failed = false;
    for (index, command) in commands.iter().enumerate() {
        if !project_still_runnable(project_root) {
            return Ok(CommandsResult::ProjectRemoved);
        }
        let cmd_started = Instant::now();
        let execution = run_command(project_root, &manifest_path, output_dir, command, index)?;
        tracing::info!(
            command = %command.name,
            duration_ms = crate::perf_log::ms(cmd_started.elapsed().as_millis()),
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
    status_cache: &Arc<Mutex<HashMap<String, LintStatus>>>,
    project_root: &Path,
    status: LintStatus,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
) {
    if let Ok(mut statuses) = status_cache.lock() {
        let key = paths::project_key(project_root);
        if matches!(status, LintStatus::NoLog) {
            statuses.remove(&key);
        } else {
            statuses.insert(key, status.clone());
        }
    }
    let _ = bg_tx.send(BackgroundMsg::LintStatus {
        path: AbsolutePath::from(project_root),
        status,
    });
}

fn run_command(
    project_root: &Path,
    manifest_path: &Path,
    output_dir: &Path,
    command: &LintCommandConfig,
    index: usize,
) -> io::Result<CommandExecution> {
    let log_name = command_log_name(command, index);
    let log_path = output_dir.join(format!("{log_name}-latest.log"));
    let tmp_path = output_dir.join(format!("{log_name}-latest.log.tmp"));

    let started = Instant::now();
    let shell_output = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command.command)
        .current_dir(project_root)
        .env("PROJECT_DIR", project_root)
        .env("MANIFEST_PATH", manifest_path)
        .env("LINT_OUTPUT_DIR", output_dir)
        .output();

    let (success, exit_code, bytes) = match shell_output {
        Ok(output) => {
            let mut bytes = output.stdout;
            bytes.extend_from_slice(&output.stderr);
            (output.status.success(), output.status.code(), bytes)
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

    std::fs::write(&tmp_path, bytes)?;
    std::fs::rename(tmp_path, log_path)?;
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
    use std::path::PathBuf;

    use super::*;
    use crate::config::CargoPortConfig;

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
    fn relevant_changes_ignore_git_and_target_paths() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        assert!(is_relevant_change(
            project_dir.path(),
            &project_dir.path().join("src/main.rs")
        ));
        assert!(is_relevant_change(
            project_dir.path(),
            &project_dir.path().join("Cargo.toml")
        ));
        assert!(!is_relevant_change(
            project_dir.path(),
            &project_dir.path().join("target/debug/app")
        ));
        assert!(!is_relevant_change(
            project_dir.path(),
            &project_dir.path().join(".git/index")
        ));
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
            command: "printf 'lint ok\\n'".to_string(),
        }];

        let (tx, _rx) = mpsc::channel();
        run_commands_for_project(
            project_dir.path(),
            "~/rust/demo",
            &cache_root,
            &commands,
            None,
            &Arc::new(Mutex::new(HashMap::new())),
            &tx,
        )
        .expect("run commands");

        let report_dir = paths::output_dir_under(&cache_root, project_dir.path());
        let latest_path = paths::latest_path_under(&cache_root, project_dir.path());
        let history_path = paths::history_path_under(&cache_root, project_dir.path());
        let report = std::fs::read_to_string(report_dir.join("echo-latest.log"))
            .expect("read command report");
        let latest = std::fs::read_to_string(latest_path).expect("read latest report");
        let history = std::fs::read_to_string(history_path).expect("read history report");

        assert_eq!(report, "lint ok\n");
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
            vec![
                request("~/rust/demo", project_dir.path(), true),
                request("~/rust/demo/excluded", project_dir.path(), true),
                request("~/rust/not-rust", project_dir.path(), false),
            ],
        );

        assert_eq!(desired.len(), 1);
        assert!(desired.contains_key(project_dir.path()));
    }

    #[test]
    fn remove_events_use_longer_debounce() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        let source_path = project_dir.path().join("src/lib.rs");
        let remove_event = notify::Event {
            kind:  notify::event::EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![source_path.clone()],
            attrs: notify::event::EventAttributes::default(),
        };
        let modify_event = notify::Event {
            kind:  notify::event::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![source_path],
            attrs: notify::event::EventAttributes::default(),
        };

        assert_eq!(
            event_debounce(project_dir.path(), &remove_event),
            Some(DELETE_LINT_DEBOUNCE)
        );
        assert_eq!(
            event_debounce(project_dir.path(), &modify_event),
            Some(LINT_DEBOUNCE)
        );
    }

    #[test]
    fn run_commands_skips_non_projects_before_writing_status() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "printf 'lint ok\\n'".to_string(),
        }];

        let (tx, _rx) = mpsc::channel();
        run_commands_for_project(
            project_dir.path(),
            "~/rust/demo",
            cache_dir.path(),
            &commands,
            None,
            &Arc::new(Mutex::new(HashMap::new())),
            &tx,
        )
        .expect("run commands");

        let latest_path = paths::latest_path_under(cache_dir.path(), project_dir.path());
        let history_path = paths::history_path_under(cache_dir.path(), project_dir.path());
        assert!(!latest_path.exists());
        assert!(!history_path.exists());
    }

    #[test]
    fn reconcile_workers_stops_stale_threads() {
        let path = AbsolutePath::from(PathBuf::from("/tmp/demo"));
        let mut workers = HashMap::new();
        let (worker, exited) = dummy_worker();
        workers.insert(path, worker);
        let (bg_tx, bg_rx) = mpsc::channel();
        let config = WorkerConfig {
            cache_root:       AbsolutePath::from(PathBuf::from("/tmp/cache")),
            commands:         Vec::new(),
            cache_size_bytes: None,
            on_discovery:     DiscoveryLint::Deferred,
            status_cache:     Arc::new(Mutex::new(HashMap::new())),
        };

        reconcile_workers(&mut workers, HashMap::new(), &config, &bg_tx);

        assert!(workers.is_empty());
        assert!(exited.load(Ordering::Relaxed));
        assert!(matches!(
            bg_rx.try_recv(),
            Ok(BackgroundMsg::LintStatus {
                status: LintStatus::NoLog,
                ..
            })
        ));
    }

    #[test]
    fn later_syncs_mark_new_workers_for_immediate_run() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");
        let request = request("~/rust/demo", project_dir.path(), true);
        let desired = HashMap::from([(AbsolutePath::from(project_dir.path()), request)]);

        let mut workers = HashMap::new();
        let (bg_tx, _bg_rx) = mpsc::channel();
        let config = WorkerConfig {
            cache_root:       AbsolutePath::from(PathBuf::from("/tmp/cache")),
            commands:         Vec::new(),
            cache_size_bytes: None,
            on_discovery:     DiscoveryLint::Immediate,
            status_cache:     Arc::new(Mutex::new(HashMap::new())),
        };
        reconcile_workers(&mut workers, desired, &config, &bg_tx);

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
        let handle = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
            }
            exited_flag.store(true, Ordering::Relaxed);
        });
        (ProjectWorker { stop, handle }, exited)
    }
}
