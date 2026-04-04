use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
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
use crate::config::LintCommandConfig;
use crate::config::LintConfig;
use crate::scan::BackgroundMsg;

const LINT_DEBOUNCE: Duration = Duration::from_millis(750);
const DELETE_LINT_DEBOUNCE: Duration = Duration::from_millis(1500);
const STOP_POLL: Duration = Duration::from_millis(250);

pub struct RegisterProjectRequest {
    pub project_path: String,
    pub abs_path:     PathBuf,
    pub is_rust:      bool,
}

pub fn project_is_eligible(
    lint: &LintConfig,
    project_path: &str,
    abs_path: &Path,
    is_rust: bool,
) -> bool {
    should_watch_project(
        lint,
        &RegisterProjectRequest {
            project_path: project_path.to_string(),
            abs_path: abs_path.to_path_buf(),
            is_rust,
        },
    )
}

pub struct RuntimeHandle {
    tx: mpsc::Sender<SupervisorMsg>,
}

impl RuntimeHandle {
    pub fn sync_projects(&self, projects: Vec<RegisterProjectRequest>) {
        let _ = self.tx.send(SupervisorMsg::SyncProjects {
            projects,
            force_immediate_run: false,
        });
    }

    pub fn sync_projects_immediately(&self, projects: Vec<RegisterProjectRequest>) {
        let _ = self.tx.send(SupervisorMsg::SyncProjects {
            projects,
            force_immediate_run: true,
        });
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
        projects:            Vec<RegisterProjectRequest>,
        force_immediate_run: bool,
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

    let cache_root = cache_paths::lint_runs_root_for(config);
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
    cache_root: PathBuf,
    lint: LintConfig,
    cache_size_bytes: Option<u64>,
    bg_tx: mpsc::Sender<BackgroundMsg>,
) {
    let commands = lint.resolved_commands();
    let mut workers: HashMap<PathBuf, ProjectWorker> = HashMap::new();
    let mut initialized = false;

    loop {
        match rx.recv() {
            Ok(SupervisorMsg::SyncProjects {
                projects,
                force_immediate_run,
            }) => {
                let desired = desired_projects(&lint, projects);
                if !initialized {
                    clear_orphaned_running_statuses(&desired, &cache_root, &bg_tx);
                }
                reconcile_workers(
                    &mut workers,
                    desired,
                    &cache_root,
                    &commands,
                    cache_size_bytes,
                    should_trigger_new_runs(initialized, force_immediate_run),
                    &bg_tx,
                );
                initialized = true;
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

const fn should_trigger_new_runs(initialized: bool, force_immediate_run: bool) -> bool {
    initialized || force_immediate_run
}

fn clear_orphaned_running_statuses(
    desired: &HashMap<PathBuf, RegisterProjectRequest>,
    cache_root: &Path,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
) {
    for (project_root, request) in desired {
        let Ok(cleared) = read_write::clear_latest_if_running_under(cache_root, project_root)
        else {
            continue;
        };
        if cleared {
            let _ = bg_tx.send(BackgroundMsg::LintStatus {
                path:   request.project_path.clone(),
                status: LintStatus::NoLog,
            });
        }
    }
}

fn desired_projects(
    lint: &LintConfig,
    projects: Vec<RegisterProjectRequest>,
) -> HashMap<PathBuf, RegisterProjectRequest> {
    projects
        .into_iter()
        .filter(|request| should_watch_project(lint, request))
        .map(|request| (request.abs_path.clone(), request))
        .collect()
}

fn reconcile_workers(
    workers: &mut HashMap<PathBuf, ProjectWorker>,
    desired: HashMap<PathBuf, RegisterProjectRequest>,
    cache_root: &Path,
    commands: &[LintCommandConfig],
    cache_size_bytes: Option<u64>,
    trigger_new_runs: bool,
    bg_tx: &mpsc::Sender<BackgroundMsg>,
) {
    let stale: Vec<PathBuf> = workers
        .keys()
        .filter(|path| !desired.contains_key(*path))
        .cloned()
        .collect();
    for path in stale {
        if let Some(worker) = workers.remove(&path) {
            stop_worker(worker);
        }
    }
    for (path, request) in desired {
        workers.entry(path).or_insert_with(|| {
            spawn_project_worker(
                request.project_path,
                request.abs_path,
                cache_root.to_path_buf(),
                commands.to_vec(),
                cache_size_bytes,
                trigger_new_runs,
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
    project_path: String,
    project_root: PathBuf,
    cache_root: PathBuf,
    commands: Vec<LintCommandConfig>,
    cache_size_bytes: Option<u64>,
    run_immediately: bool,
    bg_tx: mpsc::Sender<BackgroundMsg>,
) -> ProjectWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
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
                            let _ = run_commands_for_project(
                                &project_root,
                                &project_path,
                                &cache_root,
                                &commands,
                                cache_size_bytes,
                                &bg_tx,
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
        &request.project_path,
        &request.abs_path,
        false,
    ) {
        return false;
    }
    !matches_prefixes(
        &lint.exclude,
        &request.project_path,
        &request.abs_path,
        false,
    )
}

fn matches_prefixes(
    prefixes: &[String],
    project_path: &str,
    abs_path: &Path,
    empty_means_match: bool,
) -> bool {
    if prefixes.is_empty() {
        return empty_means_match;
    }
    let abs = abs_path.to_string_lossy();
    prefixes.iter().any(|prefix| {
        project_path.starts_with(prefix)
            || abs.starts_with(prefix)
            || project_path
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
    project_path: &str,
    cache_root: &Path,
    commands: &[LintCommandConfig],
    cache_size_bytes: Option<u64>,
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
    crate::perf_log::log_event(&format!(
        "startup_lint_started path={} abs_path={}",
        project_path,
        project_root.display()
    ));
    let _ = bg_tx.send(BackgroundMsg::LintStatus {
        path:   project_path.to_string(),
        status: status::read_status_under(cache_root, project_root),
    });

    let manifest_path = project_root.join("Cargo.toml");
    let mut failed = false;
    for (index, command) in commands.iter().enumerate() {
        if !project_still_runnable(project_root) {
            let _ = read_write::clear_latest_under(cache_root, project_root);
            let _ = bg_tx.send(BackgroundMsg::LintStatus {
                path:   project_path.to_string(),
                status: LintStatus::NoLog,
            });
            return Ok(());
        }
        let execution = run_command(project_root, &manifest_path, &output_dir, command, index)?;
        if let Some(command_run) = run.commands.get_mut(index) {
            command_run.status = if execution.success {
                LintCommandStatus::Passed
            } else {
                LintCommandStatus::Failed
            };
            command_run.duration_ms = Some(execution.duration_ms);
            command_run.exit_code = execution.exit_code;
        }
        read_write::write_latest_under(cache_root, project_root, &run)?;
        if !execution.success {
            failed = true;
        }
    }

    if !project_still_runnable(project_root) {
        let _ = read_write::clear_latest_under(cache_root, project_root);
        let _ = bg_tx.send(BackgroundMsg::LintStatus {
            path:   project_path.to_string(),
            status: LintStatus::NoLog,
        });
        return Ok(());
    }

    run.finished_at = Some(Local::now().to_rfc3339());
    run.duration_ms = Some(u64::try_from(run_started.elapsed().as_millis()).unwrap_or(u64::MAX));
    run.status = if failed {
        LintRunStatus::Failed
    } else {
        LintRunStatus::Passed
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
    let _ = bg_tx.send(BackgroundMsg::LintStatus {
        path:   project_path.to_string(),
        status: status::read_status_under(cache_root, project_root),
    });
    Ok(())
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
        .arg("-lc")
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
    use super::*;
    use crate::config::CargoPortConfig;

    fn request(path: &str, abs_path: &Path, is_rust: bool) -> RegisterProjectRequest {
        RegisterProjectRequest {
            project_path: path.to_string(),
            abs_path: abs_path.to_path_buf(),
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
        let path = PathBuf::from("/tmp/demo");
        let mut workers = HashMap::new();
        let (worker, exited) = dummy_worker();
        workers.insert(path, worker);
        let (bg_tx, _bg_rx) = mpsc::channel();

        reconcile_workers(
            &mut workers,
            HashMap::new(),
            Path::new("/tmp/cache"),
            &Vec::new(),
            None,
            false,
            &bg_tx,
        );

        assert!(workers.is_empty());
        assert!(exited.load(Ordering::Relaxed));
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
        let desired = HashMap::from([(project_dir.path().to_path_buf(), request)]);

        let mut workers = HashMap::new();
        let (bg_tx, _bg_rx) = mpsc::channel();
        reconcile_workers(
            &mut workers,
            desired,
            Path::new("/tmp/cache"),
            &Vec::new(),
            None,
            true,
            &bg_tx,
        );

        assert_eq!(workers.len(), 1);
        for (_, worker) in workers.drain() {
            stop_worker(worker);
        }
    }

    #[test]
    fn force_immediate_run_overrides_first_sync_cold_start() {
        assert!(!should_trigger_new_runs(false, false));
        assert!(should_trigger_new_runs(false, true));
        assert!(should_trigger_new_runs(true, false));
        assert!(should_trigger_new_runs(true, true));
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
