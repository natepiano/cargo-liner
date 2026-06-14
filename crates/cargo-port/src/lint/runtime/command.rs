use super::AbsolutePath;
use super::Arc;
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
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandOutcome {
    Passed,
    Failed,
}

impl CommandOutcome {
    const fn from_success(success: bool) -> Self {
        if success { Self::Passed } else { Self::Failed }
    }

    const fn succeeded(self) -> bool { matches!(self, Self::Passed) }
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
pub(crate) struct RunFinalizeGuard<'a> {
    pub(crate) cache_root:    &'a Path,
    pub(crate) project_root:  &'a Path,
    pub(crate) status_cache:  &'a Arc<Mutex<HashMap<String, CachedLintStatus>>>,
    pub(crate) background_tx: &'a Sender<BackgroundMsg>,
    pub(crate) origin:        LintRunOrigin,
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
        target: tui_pane::PERF_LOG_TARGET,
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
        tracing::trace!(
            target: tui_pane::PERF_LOG_TARGET,
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
        outcome: CommandOutcome::from_success(success),
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
