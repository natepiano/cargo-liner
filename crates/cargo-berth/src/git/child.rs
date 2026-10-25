//! Deadlines and process-group ownership shared by Git and fixture commands.

use std::io;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

/// Repository history walks and hooks may need seconds on a loaded host.
pub(super) const GIT_COMMAND_DEADLINE: Duration = Duration::from_secs(30);
/// Once the child exits, pipe EOF needs no more child computation.
pub(super) const PIPE_EOF_DEADLINE: Duration = Duration::from_millis(250);
/// SIGKILL cleanup is bounded independently of the operation that failed.
const REAP_DEADLINE: Duration = Duration::from_millis(250);
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(1);

// Keep dependency-owned probes and our pipe creation inside the same spawn boundary.
static PROCESS_SPAWN: Mutex<()> = Mutex::new(());

/// Serialize pipe creation and child spawn.
///
/// # Errors
/// Returns an error if an earlier spawn panicked while holding the lock.
pub(super) fn with_spawn_lock<T>(operation: impl FnOnce() -> T) -> io::Result<T> {
    let guard = PROCESS_SPAWN
        .lock()
        .map_err(|_| io::Error::other("Git spawn lock was poisoned"))?;
    let result = operation();
    drop(guard);
    Ok(result)
}

/// A deadline whose error identifies both the child command and the pending work.
pub(super) struct ChildDeadline {
    command: String,
    purpose: &'static str,
    end:     Instant,
}

impl ChildDeadline {
    /// Start a deadline for one named child operation.
    #[must_use]
    pub(super) fn new(command: &str, purpose: &'static str, duration: Duration) -> Self {
        Self {
            command: command.to_owned(),
            purpose,
            end: Instant::now() + duration,
        }
    }

    /// Receive a worker result without waiting past the operation's deadline.
    ///
    /// # Errors
    /// Returns a named timeout, a worker error, or an error if the worker panicked.
    pub(super) fn receive<T>(&self, receiver: &Receiver<io::Result<T>>) -> io::Result<T> {
        match receiver.recv_timeout(self.end.saturating_duration_since(Instant::now())) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(self.timeout()),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::other(format!(
                "{}: worker ended while waiting for {}",
                self.command, self.purpose
            ))),
        }
    }

    fn timeout(&self) -> Error {
        io::Error::new(
            ErrorKind::TimedOut,
            format!(
                "{}: deadline exceeded while waiting for {}",
                self.command, self.purpose
            ),
        )
    }
}

/// An owned child and process group, stopped and reaped on every exit path.
pub(super) struct ChildProcess<Cleanup: DescendantCleanup = OwnedProcessGroup> {
    child:   Child,
    command: String,
    cleanup: GroupCleanup,
    group:   Cleanup,
}

/// The descendants an owner may terminate when its child exits or times out.
pub(super) trait DescendantCleanup {
    /// Terminate descendants owned by this policy, never the parent's group.
    fn stop_descendants(&self, child_id: u32);
}

/// A private process group created by `ChildProcess::spawn`.
pub(super) struct OwnedProcessGroup;

impl DescendantCleanup for OwnedProcessGroup {
    fn stop_descendants(&self, child_id: u32) { signal_group(child_id); }
}

enum GroupCleanup {
    Required,
    Completed,
}

impl ChildProcess<OwnedProcessGroup> {
    /// Spawn in a private process group, preserving the command's configured streams.
    /// The caller owns the spawn lock when it also creates pipes.
    ///
    /// # Errors
    /// Returns the operating system's spawn error.
    pub(super) fn spawn(command: &mut Command) -> io::Result<Self> {
        let description = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|argument| argument.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        let child = command.process_group(0).spawn()?;
        Ok(Self::from_child(child, description, OwnedProcessGroup))
    }
}

impl<Cleanup: DescendantCleanup> ChildProcess<Cleanup> {
    /// Adopt a child with its command description and explicit group ownership.
    #[must_use]
    pub(super) const fn from_child(child: Child, command: String, group: Cleanup) -> Self {
        Self {
            child,
            command,
            cleanup: GroupCleanup::Required,
            group,
        }
    }

    /// Access standard-library pipe handles and nonblocking readiness queries.
    pub(super) const fn child_mut(&mut self) -> &mut Child { &mut self.child }

    /// Describe the executable and arguments in a timeout diagnostic.
    #[must_use]
    pub(super) fn command(&self) -> &str { &self.command }

    /// Wait for exit without letting a stalled child block the caller indefinitely.
    ///
    /// # Errors
    /// Returns a named timeout or an operating system wait error.
    pub(super) fn wait(&mut self, duration: Duration) -> io::Result<ExitStatus> {
        let result = wait_for_exit(&mut self.child, &self.command, duration);
        if result.is_err() {
            self.stop();
        }
        result
    }

    /// Collect output while independently draining both pipes and supplying input.
    ///
    /// # Errors
    /// Returns a named timeout or a pipe, worker, or wait error.
    pub(super) fn output(
        mut self,
        input: Vec<u8>,
        duration: Duration,
        after_exit: impl FnOnce(&mut Self),
    ) -> io::Result<Output> {
        let stdout = drain(self.child.stdout.take())?;
        let stderr = drain(self.child.stderr.take())?;
        let stdin = self.child.stdin.take();
        let written = worker(move || {
            if let Some(mut stdin) = stdin {
                stdin.write_all(&input)?;
            }
            Ok(())
        })?;
        let status = self.wait(duration)?;
        after_exit(&mut self);
        let deadline = ChildDeadline::new(self.command(), "child output EOF", PIPE_EOF_DEADLINE);
        let stdout = deadline.receive(&stdout)?;
        let stderr = deadline.receive(&stderr)?;
        match deadline.receive(&written) {
            Err(error) if error.kind() == ErrorKind::BrokenPipe && !status.success() => {},
            result => result?,
        }
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    /// Kill the group, including inherited pipe writers, then reap the direct child.
    pub(super) fn stop(&mut self) {
        if matches!(self.cleanup, GroupCleanup::Completed) {
            return;
        }
        let _ = self.child.kill();
        self.group.stop_descendants(self.child.id());
        let deadline = Instant::now() + REAP_DEADLINE;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.cleanup = GroupCleanup::Completed;
                    return;
                },
                Ok(None) if Instant::now() < deadline => thread::sleep(EXIT_POLL_INTERVAL),
                _ => return,
            }
        }
    }
}

impl<Cleanup: DescendantCleanup> Drop for ChildProcess<Cleanup> {
    fn drop(&mut self) { self.stop(); }
}

#[allow(
    unsafe_code,
    reason = "POSIX kill has no Rust standard-library process-group equivalent"
)]
fn signal_group(group: u32) {
    unsafe extern "C" {
        fn kill(pid: std::ffi::c_int, signal: std::ffi::c_int) -> std::ffi::c_int;
    }
    // SIGKILL is 9 on the supported Linux and macOS hosts. ChildProcess creates
    // this group itself; a negative PID addresses only that owned process group.
    const SIGKILL: std::ffi::c_int = 9;
    if let Ok(group) = std::ffi::c_int::try_from(group) {
        // SAFETY: kill takes scalar integers, and accesses no Rust memory.
        let _ = unsafe { kill(-group, SIGKILL) };
    }
}

/// Run a pipe task whose result is received through a `ChildDeadline`.
///
/// # Errors
/// Returns an error if the operating system cannot start the worker.
pub(super) fn worker<T: Send + 'static>(
    operation: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<Receiver<io::Result<T>>> {
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("child-pipe".to_owned())
        .spawn(move || {
            let _ = sender.send(operation());
        })?;
    Ok(receiver)
}

/// Drain an optional standard-library pipe on a worker, never on its waiting caller.
///
/// # Errors
/// Returns an error if the pipe worker cannot start.
pub(super) fn drain(
    pipe: Option<impl Read + Send + 'static>,
) -> io::Result<Receiver<io::Result<Vec<u8>>>> {
    worker(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            pipe.read_to_end(&mut bytes)?;
        }
        Ok(bytes)
    })
}

/// Run a command with bounded exit and output waits.
///
/// # Errors
/// Returns a spawn error, named timeout, or pipe error.
pub(super) fn command_output(command: &mut Command, duration: Duration) -> io::Result<Output> {
    with_spawn_lock(|| {
        ChildProcess::spawn(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
    })??
    .output(Vec::new(), duration, |_| {})
}

/// Wait for a standard-library child with a named deadline.
///
/// # Errors
/// Returns a named timeout or an operating system wait error. The owner stops
/// the child on error, using its own process-group policy.
pub(super) fn wait_for_exit(
    child: &mut Child,
    command: &str,
    duration: Duration,
) -> io::Result<ExitStatus> {
    let deadline = ChildDeadline::new(command, "child exit", duration);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline.end {
            return Err(deadline.timeout());
        }
        thread::sleep(EXIT_POLL_INTERVAL);
    }
}
