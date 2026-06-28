use std::os::unix::process::CommandExt;

use tui_pane::PERF_LOG_TARGET;

use super::AbsolutePath;
use super::Arc;
use super::AtomicBool;
use super::BackgroundMsg;
use super::CARGO_TOML;
use super::CachedLintStatus;
use super::ChildSlot;
use super::Command;
use super::HashMap;
use super::Instant;
use super::LintCommand;
use super::LintCommandConfig;
use super::LintCommandStatus;
use super::LintRun;
use super::LintRunOrigin;
use super::LintRunStatus;
use super::LintStatus;
use super::Local;
use super::Mutex;
use super::Ordering;
use super::Path;
use super::Read;
use super::Sender;
use super::Stdio;
use super::cache_size_index;
use super::history;
use super::io;
use super::paths;
use super::project_still_runnable;
use super::read_status_from_disk;
use super::read_write;
use super::status;
use super::thread;

pub(super) struct RunCommandsConfig<'a> {
    pub(super) cache_root:       &'a Path,
    pub(super) commands:         &'a [LintCommandConfig],
    pub(super) cache_size_bytes: Option<u64>,
    /// Set while lint is paused. Checked between commands so a pause kills the
    /// run mid-flight and leaves no terminal record.
    pub(super) paused:           &'a AtomicBool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandOutcome {
    Passed,
    Failed,
}

impl CommandOutcome {
    const fn succeeded(self) -> bool { matches!(self, Self::Passed) }
}

impl From<bool> for CommandOutcome {
    fn from(success: bool) -> Self { if success { Self::Passed } else { Self::Failed } }
}

struct CommandExecution {
    outcome:     CommandOutcome,
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
struct RunFinalizeGuard<'a> {
    cache_root:    &'a Path,
    project_root:  &'a Path,
    status_cache:  &'a Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    background_tx: &'a Sender<BackgroundMsg>,
    origin:        LintRunOrigin,
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
                self.origin,
            );
        }
    }
}

/// Build the initial `Running` run record with one `Pending` entry per
/// command. The run id doubles as the `runs/{run_id}` archive directory name,
/// so it is sanitized to be path-safe — the raw RFC3339 timestamp has `:`,
/// which is illegal on Windows. `started_at` keeps the unsanitized timestamp.
pub(super) fn build_pending_run(commands: &[LintCommandConfig], started_at_str: String) -> LintRun {
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

pub(super) fn run_commands_for_project(
    project_root: &Path,
    project_label: &str,
    config: &RunCommandsConfig<'_>,
    status_cache: &Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    background_tx: &Sender<BackgroundMsg>,
    child_slot: &ChildSlot,
    origin: LintRunOrigin,
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
        origin,
    };
    tracing::trace!(
        target: PERF_LOG_TARGET,
        path = project_label,
        abs_path = %project_root.display(),
        origin = ?origin,
        "lint_run_started"
    );
    publish_status(
        status_cache,
        project_root,
        status::read_status_under(cache_root, project_root),
        background_tx,
        origin,
    );

    let result = execute_commands(
        project_root,
        cache_root,
        commands,
        &output_dir,
        &mut run,
        child_slot,
        config.paused,
    )?;
    if matches!(result, CommandsResult::ProjectRemoved) {
        let _ = read_write::clear_latest_under(cache_root, project_root);
        publish_status(
            status_cache,
            project_root,
            LintStatus::NoLog,
            background_tx,
            origin,
        );
        return Ok(());
    }
    if matches!(result, CommandsResult::Interrupted) {
        // A pause killed this run mid-flight. The run was triggered by a source
        // change and never finished, so its outcome is unknown — do not fall
        // back to the prior (now-stale) terminal status. Clear the on-disk
        // `Running` marker ourselves so the `RunFinalizeGuard` drop is a no-op,
        // then publish `Stale`. Resume re-lints the project (the supervisor
        // remembers it in its catch-up set).
        let _ = read_write::clear_latest_under(cache_root, project_root);
        publish_status(
            status_cache,
            project_root,
            LintStatus::Stale,
            background_tx,
            origin,
        );
        return Ok(());
    }

    run.finished_at = Some(Local::now().to_rfc3339());
    run.duration_ms = Some(u64::try_from(run_started.elapsed().as_millis()).unwrap_or(u64::MAX));
    run.status = match result {
        CommandsResult::AllPassed => LintRunStatus::Passed,
        CommandsResult::SomeFailed
        | CommandsResult::ProjectRemoved
        | CommandsResult::Interrupted => LintRunStatus::Failed,
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
        origin,
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
    /// Lint was paused mid-run; the child was killed. The caller leaves no
    /// terminal record so the project reverts to its prior status.
    Interrupted,
}

fn execute_commands(
    project_root: &Path,
    cache_root: &Path,
    commands: &[LintCommandConfig],
    output_dir: &Path,
    run: &mut LintRun,
    child_slot: &ChildSlot,
    paused: &AtomicBool,
) -> io::Result<CommandsResult> {
    let manifest_path = project_root.join(CARGO_TOML);
    let mut failed = false;
    for (index, command) in commands.iter().enumerate() {
        if !project_still_runnable(project_root) {
            return Ok(CommandsResult::ProjectRemoved);
        }
        if paused.load(Ordering::Relaxed) {
            return Ok(CommandsResult::Interrupted);
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
        tracing::trace!(
            target: PERF_LOG_TARGET,
            command = %command.name,
            duration_ms = tui_pane::perf_log_ms(cmd_started.elapsed().as_millis()),
            success = execution.outcome.succeeded(),
            path = %project_root.display(),
            "lint_command_finished"
        );
        if let Some(command_run) = run.commands.get_mut(index) {
            command_run.status = if execution.outcome.succeeded() {
                LintCommandStatus::Passed
            } else {
                LintCommandStatus::Failed
            };
            command_run.duration_ms = Some(execution.duration_ms);
            command_run.exit_code = execution.exit_code;
        }
        read_write::write_latest_under(cache_root, project_root, run)?;
        if !execution.outcome.succeeded() {
            failed = true;
        }
    }
    if !project_still_runnable(project_root) {
        return Ok(CommandsResult::ProjectRemoved);
    }
    if paused.load(Ordering::Relaxed) {
        return Ok(CommandsResult::Interrupted);
    }
    if failed {
        Ok(CommandsResult::SomeFailed)
    } else {
        Ok(CommandsResult::AllPassed)
    }
}

pub(super) fn publish_status(
    status_cache: &Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    project_root: &Path,
    status: LintStatus,
    background_tx: &Sender<BackgroundMsg>,
    origin: LintRunOrigin,
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
        origin,
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

/// Make the lint command its own process-group leader (group id == child pid)
/// so a pause can signal the whole group. Mirrors the example runner's
/// `isolate_example_process`. No-op on non-Unix, where group kill is
/// unavailable and a plain `Child::kill` is the fallback.
#[cfg(unix)]
fn isolate_lint_process(command: &mut Command) { command.process_group(0); }

#[cfg(not(unix))]
fn isolate_lint_process(_: &mut Command) {}

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
    let mut shell = lint_shell(&expanded);
    shell
        .current_dir(project_root)
        .env("PROJECT_DIR", project_root)
        .env("MANIFEST_PATH", manifest_path)
        .env("LINT_OUTPUT_DIR", output_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Own a process group so a pause can kill the whole tree (`/bin/sh` plus
    // the `cargo`/`rustc` descendants). A plain `Child::kill` would only signal
    // the shell, leaving cargo running and the run effectively un-cancelled.
    isolate_lint_process(&mut shell);
    let spawn_result = shell.spawn();

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
        outcome: CommandOutcome::from(success),
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
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::cache_paths;
    use crate::channel;
    use crate::config::CargoPortConfig;
    use crate::config::LintCommandConfig;

    #[test]
    fn writes_reports_under_configured_cache_root() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write manifest");

        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config.cache.root = cache_dir.path().to_string_lossy().to_string();
        let cache_root = cache_paths::lint_runs_root_for(&cargo_port_config);
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
    fn skips_non_projects_before_writing_status() {
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
    fn finalize_guard_leaves_completed_marker() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");
        let completed = LintRun {
            run_id:        "completed".to_string(),
            started_at:    "2026-04-01T18:00:00-04:00".to_string(),
            finished_at:   Some("2026-04-01T18:00:10-04:00".to_string()),
            duration_ms:   Some(10_000),
            status:        LintRunStatus::Passed,
            commands:      Vec::new(),
            archive_bytes: 0,
        };
        read_write::write_latest_under(cache_dir.path(), project_dir.path(), &completed)
            .expect("write passed");
        let status_cache = Arc::new(Mutex::new(HashMap::new()));
        let (background_tx, _background_rx) = channel::unbounded();

        {
            let _guard = RunFinalizeGuard {
                cache_root:    cache_dir.path(),
                project_root:  project_dir.path(),
                status_cache:  &status_cache,
                background_tx: &background_tx,
                origin:        LintRunOrigin::Normal,
            };
        }

        assert!(paths::latest_path_under(cache_dir.path(), project_dir.path()).exists());
    }
}
