//! Asking a remote machine for its agents: `ssh <host> cargo-handler
//! probe`, and what its exit status and output say about the machine.
//!
//! Every probe of a host shares one ssh connection while probes keep
//! coming: the first opens a control socket, the rest reuse it, and it
//! closes a minute after the last. [`RemoteRunner`] stands between the
//! scheduler and ssh so the tests never start one.

use std::io::Read;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use super::MachineState;
use super::probe;
use crate::constants::BINARY_NAME;
use crate::constants::COMMAND_NOT_FOUND_STATUS;
use crate::constants::NOT_INSTALLED_REASON;
use crate::constants::PROBE_COMMAND;
use crate::constants::PROBE_EXIT_CHECK;
use crate::constants::PROBE_SIGNALLED_REASON;
use crate::constants::PROBE_TIMEOUT;
use crate::constants::SSH_BASE_OPTIONS;
use crate::constants::SSH_CONNECTION_HASH_LENGTH;
use crate::constants::SSH_CONNECTION_HASH_TOKEN;
use crate::constants::SSH_CONTROL_MASTER;
use crate::constants::SSH_CONTROL_PATH_LIMIT;
use crate::constants::SSH_CONTROL_PERSIST;
use crate::constants::SSH_CONTROL_SOCKET;
use crate::constants::SSH_DIRNAME;
use crate::constants::SSH_NOT_STARTED_REASON;
use crate::constants::SSH_PROGRAM;
use crate::constants::SSH_UNREACHABLE_STATUS;
use crate::constants::TIMED_OUT_REASON;
use crate::constants::UNREACHABLE_REASON;

/// Runs the probe on a remote machine.
pub(crate) trait RemoteRunner: Send + Sync {
    /// Run `cargo-handler probe` on `host` and say how it went.
    fn probe(&self, host: &str) -> ProbeOutcome;
}

/// How one probe went.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProbeOutcome {
    /// ssh exited within [`PROBE_TIMEOUT`].
    Finished {
        /// ssh's exit status; none when a signal ended it.
        status: Option<i32>,
        /// Everything it printed to stdout.
        stdout: Vec<u8>,
    },
    /// ssh ran past [`PROBE_TIMEOUT`] and was killed.
    TimedOut,
    /// ssh could not be started.
    NotStarted,
}

/// What `outcome` says about the machine: its rows, or why there are
/// none.
pub(crate) fn machine_state(outcome: ProbeOutcome) -> MachineState {
    let failed = |reason: &str| MachineState::Failed(reason.to_string());
    match outcome {
        ProbeOutcome::Finished {
            status: Some(0),
            stdout,
        } => probe::parse_report(&stdout).map_or_else(MachineState::Failed, MachineState::Answered),
        ProbeOutcome::Finished {
            status: Some(SSH_UNREACHABLE_STATUS),
            ..
        } => failed(UNREACHABLE_REASON),
        ProbeOutcome::Finished {
            status: Some(COMMAND_NOT_FOUND_STATUS),
            ..
        } => failed(NOT_INSTALLED_REASON),
        ProbeOutcome::Finished {
            status: Some(status),
            ..
        } => MachineState::Failed(format!("probe exited with status {status}")),
        ProbeOutcome::Finished { status: None, .. } => failed(PROBE_SIGNALLED_REASON),
        ProbeOutcome::TimedOut => failed(TIMED_OUT_REASON),
        ProbeOutcome::NotStarted => failed(SSH_NOT_STARTED_REASON),
    }
}

/// The control socket's path under `home`, as ssh is handed it.
///
/// None when the path ssh expands it to would reach
/// [`SSH_CONTROL_PATH_LIMIT`] bytes, which a Unix socket cannot bind,
/// or when the path is not UTF-8: the probe then opens a connection of
/// its own each time rather than failing every time.
pub(crate) fn control_path(home: &Path) -> Option<String> {
    let path = home
        .join(SSH_DIRNAME)
        .join(SSH_CONTROL_SOCKET)
        .to_str()?
        .to_string();
    let expanded = path.len() - SSH_CONNECTION_HASH_TOKEN.len() + SSH_CONNECTION_HASH_LENGTH;
    (expanded < SSH_CONTROL_PATH_LIMIT).then_some(path)
}

/// ssh's arguments for probing `host`, sharing a connection through
/// `control` when there is one.
pub(crate) fn ssh_arguments(host: &str, control: Option<&str>) -> Vec<String> {
    let mut arguments: Vec<String> = SSH_BASE_OPTIONS.iter().map(ToString::to_string).collect();
    if let Some(control) = control {
        for option in [
            SSH_CONTROL_MASTER.to_string(),
            format!("ControlPath={control}"),
            SSH_CONTROL_PERSIST.to_string(),
        ] {
            arguments.push("-o".to_string());
            arguments.push(option);
        }
    }
    arguments.extend([
        host.to_string(),
        BINARY_NAME.to_string(),
        PROBE_COMMAND.to_string(),
    ]);
    arguments
}

/// Probes over ssh.
#[derive(Debug)]
pub(crate) struct SshRunner {
    /// The control socket every probe shares, when there is room for
    /// one.
    control: Option<String>,
}

impl SshRunner {
    /// A runner sharing connections through a socket under the user's
    /// `~/.ssh`.
    pub(crate) fn new() -> Self {
        Self {
            control: dirs::home_dir().and_then(|home| control_path(&home)),
        }
    }
}

impl RemoteRunner for SshRunner {
    /// ssh's stdin and stderr go nowhere, since the summary owns the
    /// terminal. Its stdout is read on a thread of its own, so a probe
    /// that stops answering holds that thread and not this one past the
    /// deadline.
    fn probe(&self, host: &str) -> ProbeOutcome {
        let deadline = Instant::now() + PROBE_TIMEOUT;
        let Ok(mut child) = Command::new(SSH_PROGRAM)
            .args(ssh_arguments(host, self.control.as_deref()))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
        else {
            return ProbeOutcome::NotStarted;
        };
        let Some(mut stdout) = child.stdout.take() else {
            end(&mut child);
            return ProbeOutcome::NotStarted;
        };
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            // A read that fails partway keeps what arrived before it,
            // and the parse says whether that is a report.
            let _ = stdout.read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        });
        let Ok(stdout) = receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        else {
            end(&mut child);
            return ProbeOutcome::TimedOut;
        };
        let Some(status) = exit_status(&mut child, deadline) else {
            end(&mut child);
            return ProbeOutcome::TimedOut;
        };
        ProbeOutcome::Finished {
            status: status.code(),
            stdout,
        }
    }
}

/// `child`'s exit status once it exits, checked every
/// [`PROBE_EXIT_CHECK`] until `deadline`. None past the deadline, and
/// when the status cannot be read.
fn exit_status(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(PROBE_EXIT_CHECK),
            Ok(None) | Err(_) => return None,
        }
    }
}

/// Kill `child` and reap it.
fn end(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::census::Agent;
    use crate::census::AgentRow;
    use crate::constants::UNREADABLE_PROBE_REASON;

    /// A probe that finished with `status` and printed `stdout`.
    fn finished(status: Option<i32>, stdout: &str) -> ProbeOutcome {
        ProbeOutcome::Finished {
            status,
            stdout: stdout.as_bytes().to_vec(),
        }
    }

    /// A failure with `reason`.
    fn failed(reason: &str) -> MachineState { MachineState::Failed(reason.to_string()) }

    /// Each way a probe ends says what it says about the machine.
    #[test]
    fn each_outcome_names_the_machine_state() {
        let report = r#"{"schema":1,"machine":"mac","rows":[{"agent":"claude","name":"natemccoy-30","status":"idle","started":5,"pid":80020,"directory":"~"}]}"#;
        assert_eq!(
            machine_state(finished(Some(0), report)),
            MachineState::Answered(vec![AgentRow {
                agent:     Agent::Claude,
                name:      "natemccoy-30".to_string(),
                status:    Some("idle".to_string()),
                started:   5,
                pid:       80_020,
                directory: "~".to_string(),
            }])
        );
        assert_eq!(
            machine_state(finished(Some(0), r#"{"schema":7}"#)),
            failed("probe version 7, expected 1")
        );
        assert_eq!(
            machine_state(finished(Some(0), "motd")),
            failed(UNREADABLE_PROBE_REASON)
        );
        assert_eq!(
            machine_state(finished(Some(255), "")),
            failed("unreachable")
        );
        assert_eq!(
            machine_state(finished(Some(127), "")),
            failed("cargo-handler not installed")
        );
        assert_eq!(
            machine_state(finished(Some(3), "")),
            failed("probe exited with status 3")
        );
        assert_eq!(
            machine_state(finished(None, "")),
            failed(PROBE_SIGNALLED_REASON)
        );
        assert_eq!(machine_state(ProbeOutcome::TimedOut), failed("timed out"));
        assert_eq!(
            machine_state(ProbeOutcome::NotStarted),
            failed(SSH_NOT_STARTED_REASON)
        );
    }

    /// A home directory short enough leaves room for the socket once
    /// ssh expands `%C` into its 40 characters.
    #[test]
    fn a_short_home_gets_a_control_socket() {
        assert_eq!(
            control_path(Path::new("/home/natepiano")),
            Some("/home/natepiano/.ssh/cargo-handler-%C".to_string())
        );
    }

    /// A home directory long enough that the expanded socket path would
    /// reach 100 bytes gets none.
    #[test]
    fn a_long_home_gets_no_control_socket() {
        // `/.ssh/cargo-handler-` is 20 bytes and `%C` expands to 40, so
        // a 40-byte home comes to exactly 100.
        let at_the_limit = format!("/{}", "h".repeat(39));
        let under_it = format!("/{}", "h".repeat(38));

        assert_eq!(control_path(Path::new(&at_the_limit)), None);
        assert!(control_path(Path::new(&under_it)).is_some());
    }

    /// The base options come first, then the shared connection when
    /// there is one, then the host and the command it runs.
    #[test]
    fn ssh_is_asked_to_run_the_probe_on_the_host() {
        assert_eq!(
            ssh_arguments("mac", None),
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                "mac",
                "cargo-handler",
                "probe"
            ]
        );
        assert_eq!(
            ssh_arguments("mac", Some("/home/natepiano/.ssh/cargo-handler-%C")),
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                "-o",
                "ControlMaster=auto",
                "-o",
                "ControlPath=/home/natepiano/.ssh/cargo-handler-%C",
                "-o",
                "ControlPersist=60",
                "mac",
                "cargo-handler",
                "probe"
            ]
        );
    }
}
