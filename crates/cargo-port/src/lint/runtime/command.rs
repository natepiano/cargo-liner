#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use tui_pane::PERF_LOG_TARGET;

use super::AbsolutePath;
use super::Arc;
use super::AtomicU8;
use super::BackgroundMsg;
use super::BufRead;
use super::BufReader;
use super::CARGO_TOML;
use super::CachedLintStatus;
use super::ChildSlot;
use super::Command;
use super::DateTime;
use super::FILE_LOCK_WAIT_MARKER;
use super::FixedOffset;
use super::HashMap;
use super::Instant;
use super::LintCommand;
use super::LintCommandConfig;
use super::LintCommandStatus;
use super::LintRun;
use super::LintRunOrigin;
use super::LintRunPhase;
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
use super::supervisor::PauseState;
use super::thread;

pub(super) struct RunCommandsConfig<'a> {
    pub(super) cache_root:       &'a Path,
    pub(super) commands:         &'a [LintCommandConfig],
    pub(super) cache_size_bytes: Option<u64>,
    /// Checked between commands so a global or project pause kills the run
    /// mid-flight and leaves no terminal record.
    pub(super) pause_state:      &'a PauseState,
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

/// Which of a command's two output pipes a scanned line arrived on. Each pipe
/// tracks its own blocked state, so a line on one cannot clear a wait still in
/// force on the other.
#[derive(Clone, Copy)]
enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    const fn bit(self) -> u8 {
        match self {
            Self::Stdout => 1,
            Self::Stderr => 1 << 1,
        }
    }
}

/// Lock-free per-pipe blocked state for one lint run, shared by both output
/// reader threads. The combined phase is [`LintRunPhase::Blocked`] while either
/// pipe's most recent line matched [`FILE_LOCK_WAIT_MARKER`]. The bitmask is an
/// implementation detail; every method speaks `LintRunPhase`.
#[derive(Clone, Default)]
struct SharedPhase(Arc<AtomicU8>);

impl SharedPhase {
    /// Records `phase` for `stream`, returning the combined phase only when it
    /// changed — the caller publishes on transitions, not on every line.
    fn record(&self, stream: OutputStream, phase: LintRunPhase) -> Option<LintRunPhase> {
        let (before, after) = match phase {
            LintRunPhase::Blocked => {
                let before = self.0.fetch_or(stream.bit(), Ordering::Relaxed);
                (before, before | stream.bit())
            },
            LintRunPhase::Executing => {
                let before = self.0.fetch_and(!stream.bit(), Ordering::Relaxed);
                (before, before & !stream.bit())
            },
        };
        let after_phase = Self::phase(after);
        (Self::phase(before) != after_phase).then_some(after_phase)
    }

    /// Clears both pipes as a command ends, returning the combined phase only
    /// when that was a change. A command killed while cargo was still waiting
    /// would otherwise leave the run stuck red.
    fn clear(&self) -> Option<LintRunPhase> {
        (self.0.swap(0, Ordering::Relaxed) != 0).then_some(LintRunPhase::Executing)
    }

    const fn phase(mask: u8) -> LintRunPhase {
        if mask == 0 {
            LintRunPhase::Executing
        } else {
            LintRunPhase::Blocked
        }
    }
}

/// Publishes [`LintRunPhase`] transitions straight from a command's output
/// reader threads, so every lint spinner for the project turns red the moment
/// cargo starts waiting on a file lock instead of at the next terminal write.
///
/// This bypasses [`publish_status`] deliberately: the status cache holds
/// historical results only and never accepts `Running`, so there is nothing to
/// update there.
#[derive(Clone)]
struct PhaseReporter {
    project_root:  AbsolutePath,
    started_at:    DateTime<FixedOffset>,
    origin:        LintRunOrigin,
    background_tx: Sender<BackgroundMsg>,
    phase:         SharedPhase,
}

impl PhaseReporter {
    fn record(&self, stream: OutputStream, phase: LintRunPhase) {
        if let Some(combined) = self.phase.record(stream, phase) {
            self.publish(combined);
        }
    }

    fn clear(&self) {
        if let Some(combined) = self.phase.clear() {
            self.publish(combined);
        }
    }

    fn publish(&self, phase: LintRunPhase) {
        let _ = self.background_tx.send(BackgroundMsg::LintStatus {
            path:   self.project_root.clone(),
            status: LintStatus::Running(self.started_at, phase),
            origin: self.origin,
        });
    }
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
    let started_at = Local::now().fixed_offset();
    let mut run = build_pending_run(commands, started_at.to_rfc3339());
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
        &CommandContext {
            project_root,
            manifest_path: &project_root.join(CARGO_TOML),
            cache_root,
            output_dir: &output_dir,
            child_slot,
            reporter: &PhaseReporter {
                project_root: AbsolutePath::from(project_root),
                started_at,
                origin,
                background_tx: background_tx.clone(),
                phase: SharedPhase::default(),
            },
        },
        commands,
        &mut run,
        config.pause_state,
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

/// The values every command in one lint run shares — everything
/// [`run_command`] needs beyond the command line and its index.
struct CommandContext<'a> {
    project_root:  &'a Path,
    manifest_path: &'a Path,
    cache_root:    &'a Path,
    output_dir:    &'a Path,
    child_slot:    &'a ChildSlot,
    reporter:      &'a PhaseReporter,
}

fn execute_commands(
    context: &CommandContext<'_>,
    commands: &[LintCommandConfig],
    run: &mut LintRun,
    pause_state: &PauseState,
) -> io::Result<CommandsResult> {
    let project_root = context.project_root;
    let mut failed = false;
    for (index, command) in commands.iter().enumerate() {
        if !project_still_runnable(project_root) {
            return Ok(CommandsResult::ProjectRemoved);
        }
        if pause_state.is_project_paused(&AbsolutePath::from(project_root)) {
            return Ok(CommandsResult::Interrupted);
        }
        let cmd_started = Instant::now();
        let execution = run_command(context, command, index)?;
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
        read_write::write_latest_under(context.cache_root, project_root, run)?;
        if !execution.outcome.succeeded() {
            failed = true;
        }
    }
    if !project_still_runnable(project_root) {
        return Ok(CommandsResult::ProjectRemoved);
    }
    if pause_state.is_project_paused(&AbsolutePath::from(project_root)) {
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

/// Drains one of a command's output pipes, returning its bytes verbatim for the
/// log while reporting file-lock waits as they arrive. Reading a line at a time
/// rather than to EOF is what makes a wait observable while the command is
/// still running; the accumulated bytes are the same either way.
fn scan_stream<R: Read>(
    source: Option<R>,
    stream: OutputStream,
    reporter: &PhaseReporter,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    let Some(source) = source else {
        return bytes;
    };
    let mut reader = BufReader::new(source);
    loop {
        let line_start = bytes.len();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) | Err(_) => return bytes,
            Ok(_) => {},
        }
        let line = String::from_utf8_lossy(&bytes[line_start..]);
        // Cargo prints nothing when it finally acquires the lock, so any line
        // that is not another wait notice means the command resumed.
        let phase = if line.contains(FILE_LOCK_WAIT_MARKER) {
            LintRunPhase::Blocked
        } else {
            LintRunPhase::Executing
        };
        reporter.record(stream, phase);
    }
}

fn run_command(
    context: &CommandContext<'_>,
    command: &LintCommandConfig,
    index: usize,
) -> io::Result<CommandExecution> {
    let project_root = context.project_root;
    let output_dir = context.output_dir;
    let child_slot = context.child_slot;
    let log_name = command_log_name(command, index);
    let log_path = output_dir.join(format!("{log_name}-latest.log"));
    let tmp_path = output_dir.join(format!("{log_name}-latest.log.tmp"));

    let started = Instant::now();
    let expanded = expand_lint_placeholders(
        &command.command,
        project_root,
        context.manifest_path,
        output_dir,
    );
    let mut shell = lint_shell(&expanded);
    shell
        .current_dir(project_root)
        .env("PROJECT_DIR", project_root)
        .env("MANIFEST_PATH", context.manifest_path)
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
            let stdout_reporter = context.reporter.clone();
            let stdout_join =
                thread::spawn(move || scan_stream(stdout, OutputStream::Stdout, &stdout_reporter));
            let stderr_reporter = context.reporter.clone();
            let stderr_join =
                thread::spawn(move || scan_stream(stderr, OutputStream::Stderr, &stderr_reporter));
            let mut bytes = stdout_join.join().unwrap_or_default();
            bytes.extend(stderr_join.join().unwrap_or_default());
            // Both pipes are at EOF; a wait notice that was the last thing the
            // command printed must not outlive it.
            context.reporter.clear();
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
    cache_size_index::apply_write_delta(context.cache_root, old_size, new_size);
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

    use super::*;
    use crate::cache_paths;
    use crate::channel;
    use crate::channel::Receiver;
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
        let pause_state = PauseState::default();
        run_commands_for_project(
            project_dir.path(),
            "~/rust/demo",
            &RunCommandsConfig {
                cache_root:       cache_root.as_path(),
                commands:         &commands,
                cache_size_bytes: None,
                pause_state:      &pause_state,
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

    fn phase_reporter(background_tx: &Sender<BackgroundMsg>) -> PhaseReporter {
        PhaseReporter {
            project_root:  AbsolutePath::from(Path::new("/abs/demo")),
            started_at:    Local::now().fixed_offset(),
            origin:        LintRunOrigin::Normal,
            background_tx: background_tx.clone(),
            phase:         SharedPhase::default(),
        }
    }

    /// Drain every `Running` phase published so far, in order.
    fn published_phases(background_rx: &Receiver<BackgroundMsg>) -> Vec<LintRunPhase> {
        let mut phases = Vec::new();
        while let Ok(msg) = background_rx.try_recv() {
            if let BackgroundMsg::LintStatus {
                status: LintStatus::Running(_, phase),
                ..
            } = msg
            {
                phases.push(phase);
            }
        }
        phases
    }

    #[test]
    fn scanning_output_reports_file_lock_waits_and_passes_log_bytes_through() {
        // Cargo wraps the leading status word in ANSI escapes and prints
        // nothing at all when it finally takes the lock.
        let raw = concat!(
            "    Compiling demo v0.1.0\n",
            "\u{1b}[1m\u{1b}[92m    Blocking\u{1b}[0m waiting for file lock on build directory\n",
            "    Checking demo v0.1.0\n",
        );
        let (background_tx, background_rx) = channel::unbounded();
        let reporter = phase_reporter(&background_tx);

        let bytes = scan_stream(
            Some(io::Cursor::new(raw.as_bytes())),
            OutputStream::Stderr,
            &reporter,
        );

        assert_eq!(
            bytes,
            raw.as_bytes(),
            "log bytes must pass through verbatim"
        );
        assert_eq!(
            published_phases(&background_rx),
            vec![LintRunPhase::Blocked, LintRunPhase::Executing]
        );
    }

    #[test]
    fn a_wait_on_one_stream_is_not_cleared_by_output_on_the_other() {
        let (background_tx, background_rx) = channel::unbounded();
        let reporter = phase_reporter(&background_tx);

        reporter.record(OutputStream::Stderr, LintRunPhase::Blocked);
        reporter.record(OutputStream::Stdout, LintRunPhase::Executing);
        assert_eq!(
            published_phases(&background_rx),
            vec![LintRunPhase::Blocked],
            "stdout progress must not clear a wait still held on stderr"
        );

        reporter.record(OutputStream::Stderr, LintRunPhase::Executing);
        assert_eq!(
            published_phases(&background_rx),
            vec![LintRunPhase::Executing]
        );
    }

    #[test]
    fn ending_a_command_clears_a_wait_it_never_recovered_from() {
        let (background_tx, background_rx) = channel::unbounded();
        let reporter = phase_reporter(&background_tx);

        reporter.record(OutputStream::Stderr, LintRunPhase::Blocked);
        reporter.clear();

        assert_eq!(
            published_phases(&background_rx),
            vec![LintRunPhase::Blocked, LintRunPhase::Executing]
        );
    }

    #[test]
    fn a_running_command_publishes_its_file_lock_wait() {
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
            command: "echo Blocking waiting for file lock on build directory && echo Checking demo"
                .to_string(),
        }];

        let (background_tx, background_rx) = channel::unbounded();
        let pause_state = PauseState::default();
        run_commands_for_project(
            project_dir.path(),
            "~/rust/demo",
            &RunCommandsConfig {
                cache_root:       cache_root.as_path(),
                commands:         &commands,
                cache_size_bytes: None,
                pause_state:      &pause_state,
            },
            &Arc::new(Mutex::new(HashMap::new())),
            &background_tx,
            &Arc::new(Mutex::new(None)),
            LintRunOrigin::Normal,
        )
        .expect("run commands");

        assert!(
            published_phases(&background_rx).contains(&LintRunPhase::Blocked),
            "the wait notice on the command's output should reach the UI"
        );
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
        let pause_state = PauseState::default();
        run_commands_for_project(
            project_dir.path(),
            "~/rust/demo",
            &RunCommandsConfig {
                cache_root:       cache_dir.path(),
                commands:         &commands,
                cache_size_bytes: None,
                pause_state:      &pause_state,
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
