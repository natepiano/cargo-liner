use std::io;
use std::io::BufReader;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::thread;

#[cfg(not(unix))]
use sysinfo::Pid;
#[cfg(not(unix))]
use sysinfo::ProcessRefreshKind;
#[cfg(not(unix))]
use sysinfo::ProcessesToUpdate;
#[cfg(not(unix))]
use sysinfo::Signal;
#[cfg(not(unix))]
use sysinfo::System;

use crate::channel::Sender;
use crate::ci;
use crate::constants::CARGO_COMMAND_NAME;
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
use crate::tui::messages::ExampleMsg;
use crate::tui::panes::CiFetchKind;
use crate::tui::panes::PendingCiFetch;
use crate::tui::panes::PendingExampleRun;
use crate::tui::panes::RunTargetKind;

pub(super) fn spawn_example_process(app: &mut App, run: &PendingExampleRun) {
    let mut command = Command::new(CARGO_COMMAND_NAME);
    match run.run_target_kind {
        RunTargetKind::Binary => {
            command.arg(CARGO_RUN_SUBCOMMAND);
        },
        RunTargetKind::Example => {
            command
                .arg(CARGO_RUN_SUBCOMMAND)
                .arg(CARGO_EXAMPLE_FLAG)
                .arg(&run.target_name);
        },
        RunTargetKind::Bench => {
            command
                .arg(CARGO_BENCH_SUBCOMMAND)
                .arg(CARGO_BENCH_FLAG)
                .arg(&run.target_name);
        },
    }
    if run.build_mode.is_release() {
        command.arg(CARGO_RELEASE_FLAG);
    }
    if let Some(pkg) = &run.package_name {
        command.arg(CARGO_PACKAGE_FLAG).arg(pkg);
    }
    // Cargo does not auto-enable a target's `required-features`, so a
    // feature-gated target (e.g. an example with `required-features`)
    // errors out unless we pass them ourselves.
    if !run.required_features.is_empty() {
        command
            .arg(CARGO_FEATURES_FLAG)
            .arg(run.required_features.join(","));
    }
    command
        .current_dir(&run.abs_path)
        .arg(CARGO_COLOR_ALWAYS_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate_example_process(&mut command);

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            app.inflight
                .set_example_title(Some(run.display_path.clone()));
            app.set_example_output(vec![format!("Failed to start: {e}")]);
            app.inflight
                .set_example_running(Some(run.display_path.clone()));
            return;
        },
    };

    // On Unix, `isolate_example_process` makes the cargo child PID the
    // process-group id too. `stop_example_process` uses it from the main
    // thread to terminate the whole launched run without signaling cargo-port.
    let pid = child.id();
    *app.inflight
        .example_child()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pid);

    let display_path = run.display_path.clone();
    let target_name = run.target_name.clone();
    let mode = run.build_mode.label();
    app.inflight.set_example_title(Some(display_path.clone()));
    app.set_example_output(vec![format!("Building {target_name}{mode}...")]);
    app.inflight
        .set_example_running(Some(format!("{display_path}{mode}")));

    // Take ownership of pipes before moving child to thread
    let stderr = child.stderr.take();
    let stdout = child.stdout.take();

    let pid_holder = app.inflight.example_child();
    let example_sender = app.background.example_sender();
    thread::spawn(move || {
        let stderr_reader = stderr.map(|stream| {
            let example_sender = example_sender.clone();
            thread::spawn(move || read_with_progress(&example_sender, stream))
        });
        let stdout_reader = stdout.map(|stream| {
            let example_sender = example_sender.clone();
            thread::spawn(move || read_with_progress(&example_sender, stream))
        });

        // Wait for the child to finish and clear the PID.
        // Disk usage is updated automatically by the filesystem watcher.
        let _ = child.wait();
        if let Some(reader) = stderr_reader {
            let _ = reader.join();
        }
        if let Some(reader) = stdout_reader {
            let _ = reader.join();
        }
        *pid_holder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;

        let _ = example_sender.send(ExampleMsg::Finished);
    });
}

#[cfg(unix)]
fn isolate_example_process(cmd: &mut Command) { cmd.process_group(0); }

#[cfg(not(unix))]
fn isolate_example_process(_: &mut Command) {}

#[cfg(unix)]
pub(super) fn stop_example_process(pid: u32) -> bool {
    signal_with_kill("-TERM", format!("-{pid}")) || signal_with_kill("-TERM", pid.to_string())
}

#[cfg(unix)]
fn signal_with_kill(signal: &str, target: String) -> bool {
    Command::new("kill")
        .arg(signal)
        .arg(target)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(unix))]
pub(super) fn stop_example_process(pid: u32) -> bool {
    let mut system = System::new();
    let pid = Pid::from_u32(pid);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system
        .process(pid)
        .is_some_and(|process| process.kill_with(Signal::Term).unwrap_or(false))
}

/// Read a stream byte-by-byte, splitting on `\n` (new line) and `\r` (progress update).
/// `\r`-terminated chunks are sent as `Progress` so the UI replaces the last line.
fn read_with_progress(example_sender: &Sender<ExampleMsg>, stream: impl io::Read) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];

    while reader.read_exact(&mut byte).is_ok() {
        match byte[0] {
            b'\n' => {
                let line = String::from_utf8_lossy(&buf).to_string();
                let _ = example_sender.send(ExampleMsg::Output(line));
                buf.clear();
            },
            b'\r' => {
                if !buf.is_empty() {
                    let line = String::from_utf8_lossy(&buf).to_string();
                    let _ = example_sender.send(ExampleMsg::Progress(line));
                    buf.clear();
                }
            },
            b => buf.push(b),
        }
    }
    // Flush any remaining data
    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf).to_string();
        let _ = example_sender.send(ExampleMsg::Output(line));
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
