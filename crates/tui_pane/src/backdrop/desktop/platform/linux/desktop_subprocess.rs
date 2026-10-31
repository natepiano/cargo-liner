use std::io;
use std::io::ErrorKind;
use std::io::Read;
use std::ops::ControlFlow;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use super::constants::DESKTOP_READ_POLL_INTERVAL;
use super::constants::DESKTOP_STDOUT_CHUNK_BYTES;

/// Why a desktop subprocess could not supply its stdout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DesktopReadFailure {
    /// The child or its stdout did not finish before the shared deadline.
    Expired,
    /// Starting, reading, or completing the command failed.
    Failed(String),
}

/// Read one desktop command with the same cleanup for every backend, bounded by `deadline`.
pub(super) fn read_desktop_command(
    command: &mut Command,
    deadline: Duration,
) -> Result<Vec<u8>, DesktopReadFailure> {
    let deadline = Instant::now() + deadline;
    let mut process = DesktopSubprocess::spawn(command)
        .map_err(|error| DesktopReadFailure::Failed(error.to_string()))?;
    read_desktop_process(&mut process, |delay| wait_for_desktop_read(deadline, delay))
}

/// Poll without holding the desktop state lock, terminating failed or expired reads before retry.
fn read_desktop_process(
    process: &mut impl DesktopProcess,
    mut wait: impl FnMut(Duration) -> ControlFlow<()>,
) -> Result<Vec<u8>, DesktopReadFailure> {
    let failure = loop {
        match process.try_read() {
            Ok(ControlFlow::Break(read)) => return Ok(read),
            Ok(ControlFlow::Continue(())) => {},
            Err(error) => break DesktopReadFailure::Failed(error.to_string()),
        }
        if wait(DESKTOP_READ_POLL_INTERVAL).is_break() {
            break DesktopReadFailure::Expired;
        }
    };
    let _ = process.kill();
    let _ = process.reap();
    Err(failure)
}

/// Stop at the deadline, including when the final polling sleep consumes its remaining time.
fn wait_for_desktop_read(deadline: Instant, delay: Duration) -> ControlFlow<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        thread::sleep(delay.min(remaining));
    }
    if Instant::now() >= deadline {
        ControlFlow::Break(())
    } else {
        ControlFlow::Continue(())
    }
}

/// Subprocess operations used by a desktop read and its deterministic expiry tests.
trait DesktopProcess {
    /// Read available stdout and report a completed stdout only after the child has exited.
    fn try_read(&mut self) -> io::Result<ControlFlow<Vec<u8>>>;

    /// Stop a child whose read cannot complete.
    fn kill(&mut self) -> io::Result<()>;

    /// Collect the child status after stopping it, even if killing it failed.
    fn reap(&mut self) -> io::Result<()>;
}

/// One desktop child and its stdout, drained without blocking the deadline check.
struct DesktopSubprocess {
    /// The child is reaped by completion polling or by termination on failure.
    child:  Child,
    /// A socket permits safe nonblocking reads without adding a platform dependency.
    stdout: UnixStream,
    /// Bytes accumulated before the child finishes and stdout is drained.
    bytes:  Vec<u8>,
}

impl DesktopSubprocess {
    /// Connect stdout before spawning so every setup failure leaves no child running.
    fn spawn(command: &mut Command) -> io::Result<Self> {
        let (stdout, writer) = UnixStream::pair()?;
        stdout.set_nonblocking(true)?;
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(writer)))
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Self {
            child,
            stdout,
            bytes: Vec::new(),
        })
    }
}

impl DesktopProcess for DesktopSubprocess {
    fn try_read(&mut self) -> io::Result<ControlFlow<Vec<u8>>> {
        // Observe exit with `child.try_wait()` before `stdout.read()` so a final write
        // between polling and reading cannot be missed.
        let status = self.child.try_wait()?;
        let mut buffer = [0; DESKTOP_STDOUT_CHUNK_BYTES];
        let count = match self.stdout.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::WouldBlock => 0,
            Err(error) if error.kind() == ErrorKind::Interrupted => {
                return Ok(ControlFlow::Continue(()));
            },
            Err(error) => return Err(error),
        };
        self.bytes.extend_from_slice(&buffer[..count]);
        match status {
            Some(status) if !status.success() => Err(io::Error::other(format!(
                "desktop command exited with {status}"
            ))),
            Some(_) if count == 0 => Ok(ControlFlow::Break(std::mem::take(&mut self.bytes))),
            Some(_) | None => Ok(ControlFlow::Continue(())),
        }
    }

    fn kill(&mut self) -> io::Result<()> { self.child.kill() }

    fn reap(&mut self) -> io::Result<()> { self.child.wait().map(|_| ()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_READ_FAILURE: &str = "read failed";
    const TEST_TERMINAL_STDOUT: &[u8] = b"{terminal-uuid}\n";
    const TEST_ALREADY_EXITED_FAILURE: &str = "already exited";
    const TEST_PROCESS_POLL: &str = "poll";
    const TEST_PROCESS_KILL: &str = "kill";
    const TEST_PROCESS_REAP: &str = "reap";

    /// A subprocess whose completion, read error, and cleanup can be checked without spawning.
    struct ScriptedDesktopProcess {
        completion:  io::Result<ControlFlow<Vec<u8>>>,
        calls:       Vec<&'static str>,
        kill_result: io::Result<()>,
    }

    impl DesktopProcess for ScriptedDesktopProcess {
        fn try_read(&mut self) -> io::Result<ControlFlow<Vec<u8>>> {
            self.calls.push(TEST_PROCESS_POLL);
            std::mem::replace(&mut self.completion, Ok(ControlFlow::Continue(())))
        }

        fn kill(&mut self) -> io::Result<()> {
            self.calls.push(TEST_PROCESS_KILL);
            std::mem::replace(&mut self.kill_result, Ok(()))
        }

        fn reap(&mut self) -> io::Result<()> {
            self.calls.push(TEST_PROCESS_REAP);
            Ok(())
        }
    }

    #[test]
    fn desktop_deadline_kills_and_reaps_even_when_kill_fails() {
        for kill_result in [Ok(()), Err(io::Error::other(TEST_ALREADY_EXITED_FAILURE))] {
            let mut process = ScriptedDesktopProcess {
                completion: Ok(ControlFlow::Continue(())),
                calls: Vec::new(),
                kill_result,
            };
            let deadline = Instant::now();
            let result = read_desktop_process(&mut process, |delay| {
                assert_eq!(delay, DESKTOP_READ_POLL_INTERVAL);
                wait_for_desktop_read(deadline, delay)
            });
            assert_eq!(result, Err(DesktopReadFailure::Expired));
            assert_eq!(
                process.calls,
                vec![TEST_PROCESS_POLL, TEST_PROCESS_KILL, TEST_PROCESS_REAP]
            );
        }
    }

    #[test]
    fn failed_desktop_read_kills_and_reaps_before_returning() {
        let mut process = ScriptedDesktopProcess {
            completion:  Err(io::Error::other(TEST_READ_FAILURE)),
            calls:       Vec::new(),
            kill_result: Ok(()),
        };
        let result = read_desktop_process(&mut process, |_| ControlFlow::Break(()));
        assert_eq!(
            result,
            Err(DesktopReadFailure::Failed(TEST_READ_FAILURE.to_owned()))
        );
        assert_eq!(
            process.calls,
            vec![TEST_PROCESS_POLL, TEST_PROCESS_KILL, TEST_PROCESS_REAP]
        );
    }

    #[test]
    fn completed_desktop_read_returns_stdout_without_termination() {
        let mut process = ScriptedDesktopProcess {
            completion:  Ok(ControlFlow::Break(TEST_TERMINAL_STDOUT.to_vec())),
            calls:       Vec::new(),
            kill_result: Ok(()),
        };
        let result = read_desktop_process(&mut process, |_| ControlFlow::Break(()));
        assert_eq!(result, Ok(TEST_TERMINAL_STDOUT.to_vec()));
        assert_eq!(process.calls, vec![TEST_PROCESS_POLL]);
    }
}
