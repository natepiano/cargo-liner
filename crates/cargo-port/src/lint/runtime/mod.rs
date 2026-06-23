use std::collections::HashMap;
use std::collections::HashSet;
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
use std::time::Instant;

use chrono::Local;

use super::cache_size_index;
use super::constants::STOP_POLL;
use super::history;
use super::paths;
use super::read_write;
use super::run::LintCommand;
use super::run::LintCommandStatus;
use super::run::LintRun;
use super::run::LintRunOrigin;
use super::run::LintRunStatus;
use super::status;
use super::status::CachedLintStatus;
use super::status::LintStatus;
use super::trigger::LintEventKind;
use super::trigger::LintTriggerEvent;
use super::trigger::LintTriggerKind;
use crate::cache_paths;
use crate::channel::Sender;
use crate::config::CargoPortConfig;
use crate::config::DiscoveryLint;
use crate::config::LintCommandConfig;
use crate::config::LintConfig;
use crate::constants::CARGO_TOML;
use crate::constants::LINTS_HISTORY_JSONL;
use crate::constants::LINTS_LATEST_JSON;
use crate::project;
use crate::project::AbsolutePath;
use crate::scan::BackgroundMsg;

mod command;
mod handle;
mod request;
mod supervisor;

use command::RunCommandsConfig;
#[cfg(test)]
pub(crate) use command::RunFinalizeGuard;
#[cfg(test)]
use command::build_pending_run;
use command::publish_status;
use command::run_commands_for_project;
use handle::ChildSlot;
pub use handle::RuntimeHandle;
use handle::SupervisorMsg;
pub use request::RegisterProjectRequest;
pub use request::project_is_eligible;
#[cfg(test)]
use supervisor::ProjectWorker;
#[cfg(test)]
use supervisor::WorkerConfig;
#[cfg(test)]
use supervisor::WorkerStart;
#[cfg(test)]
use supervisor::desired_projects;
use supervisor::project_still_runnable;
pub(crate) use supervisor::read_status_from_disk;
#[cfg(test)]
use supervisor::reconcile_workers;
#[cfg(test)]
use supervisor::schedule_lint_run;
#[cfg(test)]
use supervisor::should_watch_project;
pub use supervisor::spawn;
#[cfg(test)]
use supervisor::stop_worker;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::time::Duration;

    use crossbeam_channel::RecvTimeoutError;

    use super::*;
    use crate::channel;
    use crate::config::CargoPortConfig;
    use crate::config::LintIndicator;
    use crate::lint::trigger::LintEventKind::CreateOrModify;
    use crate::lint::trigger::LintTriggerKind::RustSource;
    use crate::lint::trigger::LintTriggerKind::Startup;

    fn request(path: &str, abs_path: &Path) -> RegisterProjectRequest {
        RegisterProjectRequest::new(path, AbsolutePath::from(abs_path))
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
    fn non_rust_projects_are_never_watched() {
        let project_dir = tempfile::tempdir().expect("tempdir");
        assert!(!project_is_eligible(
            &LintConfig::default(),
            "~/rust/not-rust",
            project_dir.path(),
            false
        ));
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
                paused:           &AtomicBool::new(false),
            },
            &Arc::new(Mutex::new(HashMap::new())),
            &tx,
            &Arc::new(Mutex::new(None)),
            LintRunOrigin::Normal,
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
        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        cfg.lint.enabled = LintIndicator::Enabled;
        cfg.lint.include = vec!["~/rust/demo".to_string()];
        cfg.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cfg, background_tx);
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
        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        cfg.lint.enabled = LintIndicator::Enabled;
        cfg.lint.include = vec!["~/rust/demo".to_string()];
        cfg.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cfg, background_tx);
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
        cfg.lint.enabled = LintIndicator::Enabled;
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
        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        cfg.lint.enabled = LintIndicator::Enabled;
        cfg.lint.include = vec![project_dir.path().to_string_lossy().to_string()];
        cfg.lint.on_discovery = DiscoveryLint::Immediate;
        cfg.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cfg, background_tx);
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
        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = cache_dir.path().to_string_lossy().to_string();
        cfg.lint.enabled = LintIndicator::Enabled;
        cfg.lint.include = vec![project_dir.path().to_string_lossy().to_string()];
        cfg.lint.on_discovery = DiscoveryLint::Immediate;
        cfg.lint.commands = vec![LintCommandConfig {
            name:    "echo".to_string(),
            command: "echo lint ok".to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let spawn = spawn(&cfg, background_tx);
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
                paused:           &AtomicBool::new(false),
            },
            &Arc::new(Mutex::new(HashMap::new())),
            &tx,
            &Arc::new(Mutex::new(None)),
            LintRunOrigin::Normal,
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
                origin:        LintRunOrigin::CatchUp,
            };
        }

        assert!(matches!(
            background_rx.try_recv(),
            Ok(BackgroundMsg::LintStatus {
                status: LintStatus::NoLog,
                origin: LintRunOrigin::CatchUp,
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
