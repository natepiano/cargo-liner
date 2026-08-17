use std::io;
#[cfg(test)]
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::thread;

use crate::channel::Sender;
use crate::ci;
use crate::constants::CARGO_COMMAND_NAME;
use crate::process_observation::identity;
use crate::process_observation::identity::CurrentProcessIdentityObservation;
use crate::project::AbsolutePath;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::app::PendingClean;
use crate::tui::constants::CARGO_BENCH_FLAG;
use crate::tui::constants::CARGO_BENCH_SUBCOMMAND;
use crate::tui::constants::CARGO_CLEAN_SUBCOMMAND;
use crate::tui::constants::CARGO_COLOR_ALWAYS_FLAG;
use crate::tui::constants::CARGO_EXAMPLE_FLAG;
use crate::tui::constants::CARGO_FEATURES_FLAG;
use crate::tui::constants::CARGO_PACKAGE_FLAG;
use crate::tui::constants::CARGO_RELEASE_FLAG;
use crate::tui::constants::CARGO_RUN_SUBCOMMAND;
use crate::tui::messages::CiFetchMsg;
use crate::tui::messages::CleanMsg;
use crate::tui::messages::OwnedRunEvent;
use crate::tui::panes::CargoPackageInvocation;
use crate::tui::panes::CiFetchKind;
use crate::tui::panes::PendingCiFetch;
use crate::tui::panes::PendingExampleRun;
use crate::tui::panes::RunTargetKind;
use crate::tui::state;
use crate::tui::state::Inflight;
use crate::tui::state::OwnedRunActivation;
use crate::tui::state::OwnedRunId;
use crate::tui::state::OwnedRunProcessActor;
use crate::tui::state::OwnedRunStartingRequest;
use crate::tui::state::OwnedRunTermination;
use crate::tui::state::OwnedRunTerminationSubmission;

/// Attempt to signal the current run's identity-bound process group.
pub(in crate::tui) enum OwnedRunStopSignal {
    Submitted,
    NotSubmitted,
}

pub(in crate::tui) fn signal_owned_run(inflight: &mut Inflight) -> OwnedRunStopSignal {
    match inflight.owned_run_termination() {
        OwnedRunTermination::Available {
            owned_run_termination_token,
            ..
        } => match inflight.submit_owned_run_termination(owned_run_termination_token) {
            OwnedRunTerminationSubmission::Submitted(_) => OwnedRunStopSignal::Submitted,
            OwnedRunTerminationSubmission::RequestAlreadyPending
            | OwnedRunTerminationSubmission::TokenRefused
            | OwnedRunTerminationSubmission::ActorUnavailable => OwnedRunStopSignal::NotSubmitted,
        },
        OwnedRunTermination::RequestPending { .. } | OwnedRunTermination::NoRunningRun => {
            OwnedRunStopSignal::NotSubmitted
        },
    }
}

/// Spawn the request owned by the current `Starting` lifecycle.
pub(super) fn spawn_owned_run_process(app: &mut App, owned_run_id: OwnedRunId) {
    let pending_example_run = match app.inflight.starting_owned_run_request(owned_run_id) {
        OwnedRunStartingRequest::Starting(pending_example_run) => pending_example_run.clone(),
        OwnedRunStartingRequest::NoMatchingStartingRun => return,
    };
    let mut command = cargo_command_for_owned_run(&pending_example_run);
    isolate_owned_process(&mut command);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let output_was_empty = app.inflight.owned_run().output_is_empty();
            app.inflight
                .fail_owned_run_start(owned_run_id, format!("Failed to start: {error}"));
            app.owned_run_output_replaced(output_was_empty);
            return;
        },
    };

    // On Unix, `isolate_owned_process` makes the cargo child PID the process
    // group ID. The child cannot enter a live lifecycle until a strong identity
    // has been observed and revalidated for that root lifetime.
    let process_group_id = child.id();
    let CurrentProcessIdentityObservation::Verified(verified_process_identity) =
        identity::observe_current_process_identity(process_group_id)
    else {
        state::discard_unverified_owned_process_group(&mut child);
        let output_was_empty = app.inflight.owned_run().output_is_empty();
        app.inflight.fail_owned_run_start(
            owned_run_id,
            "Failed to establish a verified process identity".to_string(),
        );
        app.owned_run_output_replaced(output_was_empty);
        return;
    };

    let root_identity = verified_process_identity.clone().into_process_identity();
    let stderr = child.stderr.take();
    let stdout = child.stdout.take();
    let started_sender = app.background.example_sender();
    let stderr_reader = stderr.map(|stream| {
        let example_sender = started_sender.clone();
        thread::spawn(move || read_with_progress(&example_sender, owned_run_id, stream))
    });
    let stdout_reader = stdout.map(|stream| {
        let example_sender = started_sender.clone();
        thread::spawn(move || read_with_progress(&example_sender, owned_run_id, stream))
    });
    let mut output_readers = Vec::new();
    if let Some(stderr_reader) = stderr_reader {
        output_readers.push(stderr_reader);
    }
    if let Some(stdout_reader) = stdout_reader {
        output_readers.push(stdout_reader);
    }
    let owned_run_process_actor = OwnedRunProcessActor::prepare(
        owned_run_id,
        child,
        verified_process_identity,
        output_readers,
        started_sender.clone(),
    );
    let output_was_empty = app.inflight.owned_run().output_is_empty();
    match app
        .inflight
        .activate_owned_run(owned_run_id, owned_run_process_actor, root_identity)
    {
        OwnedRunActivation::Activated => {},
        OwnedRunActivation::NoMatchingStartingRun(owned_run_process_actor) => {
            owned_run_process_actor.discard_unactivated();
            return;
        },
    }
    app.owned_run_output_replaced(output_was_empty);
    app.inflight.start_owned_run_process_actor(owned_run_id);
    let _ = started_sender.send(OwnedRunEvent::Started { owned_run_id });
}

fn cargo_command_for_owned_run(pending_example_run: &PendingExampleRun) -> Command {
    let mut command = Command::new(CARGO_COMMAND_NAME);
    match pending_example_run.run_target_kind {
        RunTargetKind::Binary => {
            command.arg(CARGO_RUN_SUBCOMMAND);
        },
        RunTargetKind::Example => {
            command
                .arg(CARGO_RUN_SUBCOMMAND)
                .arg(CARGO_EXAMPLE_FLAG)
                .arg(&pending_example_run.target_name);
        },
        RunTargetKind::Bench => {
            command
                .arg(CARGO_BENCH_SUBCOMMAND)
                .arg(CARGO_BENCH_FLAG)
                .arg(&pending_example_run.target_name);
        },
    }
    if pending_example_run.build_mode.is_release() {
        command.arg(CARGO_RELEASE_FLAG);
    }
    match &pending_example_run.cargo_package_invocation {
        CargoPackageInvocation::WorkspaceDefault => {},
        CargoPackageInvocation::Package(package_name) => {
            command.arg(CARGO_PACKAGE_FLAG).arg(package_name);
        },
    }
    // Cargo does not auto-enable a target's `required-features`, so a
    // feature-gated target (e.g. an example with `required-features`)
    // errors out unless we pass them ourselves.
    if !pending_example_run.required_features.is_empty() {
        command
            .arg(CARGO_FEATURES_FLAG)
            .arg(pending_example_run.required_features.join(","));
    }
    command
        .current_dir(&pending_example_run.abs_path)
        .arg(CARGO_COLOR_ALWAYS_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[cfg(unix)]
fn isolate_owned_process(command: &mut Command) { command.process_group(0); }

#[cfg(not(unix))]
fn isolate_owned_process(_: &mut Command) {}

/// Read a stream byte-by-byte, splitting on `\n` (new line) and `\r` (progress update).
/// `\r`-terminated chunks are sent as `Progress` so the UI replaces the last line.
fn read_with_progress(
    example_sender: &Sender<OwnedRunEvent>,
    owned_run_id: OwnedRunId,
    stream: impl io::Read,
) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];

    while reader.read_exact(&mut byte).is_ok() {
        match byte[0] {
            b'\n' => {
                let line = String::from_utf8_lossy(&buf).to_string();
                let _ = example_sender.send(OwnedRunEvent::Output { owned_run_id, line });
                buf.clear();
            },
            b'\r' => {
                if !buf.is_empty() {
                    let line = String::from_utf8_lossy(&buf).to_string();
                    let _ = example_sender.send(OwnedRunEvent::Progress { owned_run_id, line });
                    buf.clear();
                }
            },
            b => buf.push(b),
        }
    }
    // Flush any remaining data
    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf).to_string();
        let _ = example_sender.send(OwnedRunEvent::Output { owned_run_id, line });
    }
}

pub(super) fn spawn_clean_process(app: &mut App, pending: &PendingClean) {
    let mut command = std::process::Command::new(CARGO_COMMAND_NAME);
    command
        .arg(CARGO_CLEAN_SUBCOMMAND)
        .current_dir(&pending.abs_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            app.clean_spawn_failed(&pending.abs_path);
            app.show_timed_toast("cargo clean failed", e.to_string());
            return;
        },
    };
    let clean_sender = app.background.clean_sender();
    let abs_path = pending.abs_path.clone();
    thread::spawn(move || {
        let _ = child.wait();
        let _ = clean_sender.send(CleanMsg::Finished(abs_path));
    });
}

pub(super) fn spawn_ci_fetch(app: &App, fetch: &PendingCiFetch) -> bool {
    // Derive (repo_url, owner, repo) from local git info — no network needed.
    // Use `fetch_url_for` so a worktree without upstream tracking still resolves.
    let path = Path::new(&fetch.project_path);
    let Some(repo_url) = app.project_list.fetch_url_for(path) else {
        return false;
    };
    let Some(owner_repo) = ci::parse_owner_repo(&repo_url) else {
        return false;
    };

    let ci_fetch_sender = app.background.ci_fetch_sender();
    let background_tx = app.background.background_sender();
    let client = app.net.http_client();
    let project_path = fetch.project_path.clone();
    let ci_run_count = fetch.ci_run_count;
    let oldest_created_at = fetch.oldest_created_at.clone();
    let ci_fetch_kind = fetch.ci_fetch_kind;
    let url = repo_url;

    thread::spawn(move || {
        let (result, network) = match ci_fetch_kind {
            CiFetchKind::Older => {
                let oldest = oldest_created_at
                    .as_deref()
                    .unwrap_or("1970-01-01T00:00:00Z");
                scan::fetch_older_runs(
                    &client,
                    &url,
                    owner_repo.owner(),
                    owner_repo.repo(),
                    oldest,
                    ci_run_count,
                )
            },
            CiFetchKind::Sync => {
                let (result, _meta, signal) = scan::fetch_ci_runs_cached(
                    &client,
                    &url,
                    owner_repo.owner(),
                    owner_repo.repo(),
                    ci_run_count,
                );
                (result, signal)
            },
        };
        scan::emit_service_signal(&background_tx, network);
        let _ = ci_fetch_sender.send(CiFetchMsg::Complete {
            path: project_path,
            result,
            kind: ci_fetch_kind,
        });
    });
    true
}
/// Spawn a background thread to fetch details for a single project ahead of the main scan.
pub(super) fn spawn_priority_fetch(app: &App, _: &str, abs_path: &str, name: Option<&String>) {
    let sender = app.background.background_sender();
    let client = app.net.http_client();
    let abs = AbsolutePath::from(abs_path);
    let project_name = name.cloned();

    thread::spawn(move || {
        let path: AbsolutePath = abs.clone();
        scan::emit_git_info(&sender, &abs);

        let bytes = scan::dir_size(&abs);
        let _ = sender.send(BackgroundMsg::DiskUsage {
            path: path.clone(),
            bytes,
        });

        if let Some(name) = project_name.as_ref() {
            let _ = sender.send(BackgroundMsg::CratesIoFetchQueued { name: name.clone() });
            let (info, signal) = client.fetch_crates_io_info(name);
            scan::emit_service_signal(&sender, signal);
            if let Some(info) = info {
                let _ = sender.send(BackgroundMsg::CratesIoVersion {
                    path,
                    version: info.version,
                    prerelease: info.prerelease,
                    downloads: info.downloads,
                });
            }
            let _ = sender.send(BackgroundMsg::CratesIoFetchComplete { name: name.clone() });
        }
    });
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::time::Duration;
    #[cfg(unix)]
    use std::time::Instant;

    use super::*;
    #[cfg(unix)]
    use crate::support;
    use crate::tui::panes::BuildMode;

    /// How long the cleanup assertions wait for a killed process to leave the
    /// process table.
    #[cfg(unix)]
    const PROCESS_EXIT_DEADLINE: Duration = Duration::from_secs(5);
    /// Gap between `kill(pid, 0)` probes while waiting out
    /// [`PROCESS_EXIT_DEADLINE`].
    #[cfg(unix)]
    const PROCESS_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

    /// Whether a `kill(pid, 0)` probe still reaches a process or process group.
    #[cfg(unix)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ProcessVisibility {
        Visible,
        Gone,
    }

    #[cfg(unix)]
    impl From<bool> for ProcessVisibility {
        fn from(signal_reaches_target: bool) -> Self {
            if signal_reaches_target {
                Self::Visible
            } else {
                Self::Gone
            }
        }
    }

    fn pending_example_run(cargo_package_invocation: CargoPackageInvocation) -> PendingExampleRun {
        PendingExampleRun {
            abs_path: "/tmp/demo".to_string(),
            target_name: "demo".to_string(),
            display_path: "demo".to_string(),
            cargo_package_invocation,
            run_target_kind: RunTargetKind::Binary,
            build_mode: BuildMode::Debug,
            required_features: Vec::new(),
        }
    }

    fn cargo_arguments(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn workspace_default_invocation_omits_the_package_flag() {
        let command = cargo_command_for_owned_run(&pending_example_run(
            CargoPackageInvocation::WorkspaceDefault,
        ));

        assert_eq!(
            cargo_arguments(&command),
            vec![
                CARGO_RUN_SUBCOMMAND.to_string(),
                CARGO_COLOR_ALWAYS_FLAG.to_string()
            ]
        );
    }

    #[test]
    fn named_package_invocation_passes_the_package_flag_and_name() {
        let command = cargo_command_for_owned_run(&pending_example_run(
            CargoPackageInvocation::Package("demo-member".to_string()),
        ));

        assert_eq!(
            cargo_arguments(&command),
            vec![
                CARGO_RUN_SUBCOMMAND.to_string(),
                CARGO_PACKAGE_FLAG.to_string(),
                "demo-member".to_string(),
                CARGO_COLOR_ALWAYS_FLAG.to_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn unverified_process_group_cleanup_reaps_the_root_and_descendant() -> io::Result<()> {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("trap '' TERM; (trap '' TERM; exec sleep 30) & echo $!; wait")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        isolate_owned_process(&mut command);
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing descendant PID stream"))?;
        let mut descendant_pid = String::new();
        BufReader::new(stdout).read_line(&mut descendant_pid)?;
        let descendant_pid: u32 = descendant_pid
            .trim()
            .parse()
            .map_err(|_| io::Error::other("descendant PID stream did not carry a pid"))?;

        let process_group_id = child.id();
        state::discard_unverified_owned_process_group(&mut child);

        assert!(child.try_wait()?.is_some());
        assert_eq!(
            visibility_before_deadline(|| support::process_group_exists(process_group_id).into()),
            ProcessVisibility::Gone
        );
        assert_eq!(
            visibility_before_deadline(|| support::process_exists(descendant_pid).into()),
            ProcessVisibility::Gone
        );
        Ok(())
    }

    /// Probe until the target is gone or [`PROCESS_EXIT_DEADLINE`] passes, and
    /// report the last observation.
    ///
    /// One probe taken straight after the kill is not enough: `SIGKILL` stops the
    /// process immediately but leaves it in the process table answering
    /// `kill(pid, 0)` until its parent reaps it, and a descendant orphaned by the
    /// group kill waits on init to do that.
    #[cfg(unix)]
    fn visibility_before_deadline(
        mut probe_target: impl FnMut() -> ProcessVisibility,
    ) -> ProcessVisibility {
        let deadline = Instant::now() + PROCESS_EXIT_DEADLINE;
        loop {
            let process_visibility = probe_target();
            if process_visibility == ProcessVisibility::Gone || Instant::now() >= deadline {
                return process_visibility;
            }
            thread::sleep(PROCESS_EXIT_POLL_INTERVAL);
        }
    }
}
