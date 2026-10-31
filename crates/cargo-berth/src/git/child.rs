//! Deadlines and process-group ownership shared by Git and fixture commands.

use std::io;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;
use std::process::Stdio;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use rustix::event::PollFd;
use rustix::event::PollFlags;
use rustix::event::Timespec;

/// Repository history walks and hooks may need seconds on a loaded host.
pub(super) const GIT_COMMAND_DEADLINE: Duration = Duration::from_secs(30);
/// Adopted fixture children may have pipe writers outside a group we own.
pub(super) const PIPE_EOF_DEADLINE: Duration = Duration::from_millis(250);
/// SIGKILL cleanup is bounded independently of the operation that failed.
const REAP_DEADLINE: Duration = Duration::from_millis(250);
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(1);

use super::with_spawn_lock;

#[cfg(test)]
enum DrainScheduling {
    Ready,
    Paused(Receiver<()>),
}

#[cfg(test)]
thread_local! {
    static NEXT_DRAIN: std::cell::RefCell<DrainScheduling> = const {
        std::cell::RefCell::new(DrainScheduling::Ready)
    };
    static DRAINS_AFTER_CLEANUP: std::cell::RefCell<Vec<mpsc::Sender<()>>> = const {
        std::cell::RefCell::new(Vec::new())
    };
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
    child:      Child,
    command:    String,
    cleanup:    GroupCleanup,
    group:      Cleanup,
    output_eof: Vec<OwnedFd>,
}

/// The descendants an owner may terminate when its child exits or times out.
pub(super) trait DescendantCleanup {
    /// Terminate descendants owned by this policy, never the parent's group.
    fn stop_descendants(&self, child_id: u32);

    /// Adopted children do not own their parent's group or all their pipe writers.
    fn owns_process_group(&self) -> bool { false }
}

/// Whether EOF follows owned-group cleanup or still depends on unowned writers.
pub(super) enum PipeCompletion {
    GroupStopped,
    InheritedWritersTerminated,
    UnownedWriters,
}

impl PipeCompletion {
    /// Collect a worker after owned writers have stopped, without a scheduling deadline.
    ///
    /// # Errors
    /// Returns the worker's I/O error, a disconnected worker, or an unowned-pipe timeout.
    pub(super) fn receive<T>(
        &self,
        command: &str,
        receiver: &Receiver<io::Result<T>>,
    ) -> io::Result<T> {
        if matches!(self, Self::UnownedWriters) {
            return ChildDeadline::new(command, "unowned child output EOF", PIPE_EOF_DEADLINE)
                .receive(receiver);
        }
        receiver.recv().map_err(|_| {
            io::Error::other(format!(
                "{command}: worker ended while collecting child output EOF"
            ))
        })?
    }

    /// Report inherited pipe writers independently of worker scheduling.
    ///
    /// # Errors
    /// Returns a named error when cleanup had to terminate inherited pipe writers.
    pub(super) fn check(&self, command: &str) -> io::Result<()> {
        if matches!(self, Self::InheritedWritersTerminated) {
            return Err(io::Error::other(format!(
                "{command}: child exited with inherited pipe writers still open; terminated the process group before collecting output EOF"
            )));
        }
        Ok(())
    }
}

/// A private process group created by `ChildProcess::spawn`.
pub(super) struct OwnedProcessGroup;

impl DescendantCleanup for OwnedProcessGroup {
    fn stop_descendants(&self, child_id: u32) { signal_group(child_id); }

    fn owns_process_group(&self) -> bool { true }
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
        let mut process = Self::from_child(child, description, OwnedProcessGroup);
        // Keep close-on-exec read handles even when a drain or protocol worker
        // owns the original streams. EOF is a kernel fact, not worker progress.
        for pipe in process
            .child
            .stdout
            .iter()
            .map(AsFd::as_fd)
            .chain(process.child.stderr.iter().map(AsFd::as_fd))
        {
            process.output_eof.push(pipe.try_clone_to_owned()?);
        }
        Ok(process)
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
            output_eof: Vec::new(),
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
        let completion = self.finish_output()?;
        let stdout = completion.receive(self.command(), &stdout)?;
        let stderr = completion.receive(self.command(), &stderr)?;
        let written = completion.receive(self.command(), &written);
        completion.check(self.command())?;
        match written {
            Err(error) if error.kind() == ErrorKind::BrokenPipe && !status.success() => {},
            result => result?,
        }
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    /// End owned pipe writers after direct-child exit, before collecting worker results.
    ///
    /// # Errors
    /// Returns an operating system error if pipe EOF cannot be inspected.
    pub(super) fn finish_output(&mut self) -> io::Result<PipeCompletion> {
        if matches!(self.cleanup, GroupCleanup::Completed) {
            return Ok(PipeCompletion::GroupStopped);
        }
        let completion = if self.group.owns_process_group() {
            output_writers_remain(&self.output_eof).map(|open| {
                if open {
                    PipeCompletion::InheritedWritersTerminated
                } else {
                    PipeCompletion::GroupStopped
                }
            })
        } else {
            Ok(PipeCompletion::UnownedWriters)
        };
        self.stop();
        completion
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
                    #[cfg(test)]
                    for continued in DRAINS_AFTER_CLEANUP.take() {
                        let _ = continued.send(());
                    }
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

fn signal_group(group: u32) {
    if let Ok(group) = i32::try_from(group)
        && let Some(group) = rustix::process::Pid::from_raw(group)
    {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
}

/// Inspect pipe EOF without reading bytes or waiting for a drain to be scheduled.
fn output_writers_remain(pipes: &[OwnedFd]) -> io::Result<bool> {
    // Darwin registers the EOF filter only when input is requested.
    let mut descriptors: Vec<_> = pipes
        .iter()
        .map(|pipe| PollFd::new(pipe, PollFlags::IN))
        .collect();
    loop {
        match rustix::event::poll(&mut descriptors, Some(&Timespec::default())) {
            Ok(_) => {
                return Ok(descriptors
                    .iter()
                    .any(|descriptor| !descriptor.revents().contains(PollFlags::HUP)));
            },
            Err(rustix::io::Errno::INTR) => {},
            Err(error) => return Err(error.into()),
        }
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
    #[cfg(test)]
    let scheduling = NEXT_DRAIN.replace(DrainScheduling::Ready);
    worker(move || {
        #[cfg(test)]
        if let DrainScheduling::Paused(continued) = scheduling {
            continued.recv().map_err(io::Error::other)?;
        }
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

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::process::Command;
    use std::process::Stdio;
    use std::sync::mpsc;

    use super::ChildProcess;
    use super::DRAINS_AFTER_CLEANUP;
    use super::DrainScheduling;
    use super::GIT_COMMAND_DEADLINE;
    use super::NEXT_DRAIN;
    use super::with_spawn_lock;

    #[test]
    fn exited_child_output_ignores_descendants_that_closed_their_pipes()
    -> Result<(), Box<dyn Error>> {
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "ready=$(sleep 30 >/dev/null 2>&1 & printf ready); test \"$ready\" = ready; printf stdout; printf stderr >&2",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = with_spawn_lock(|| ChildProcess::spawn(&mut command))??;
        // Command substitution reaches EOF only after the background child has
        // closed its inherited pipes. It still belongs to our process group.
        let output = child.output(Vec::new(), GIT_COMMAND_DEADLINE, |_| {})?;
        assert!(output.status.success());
        assert_eq!(output.stdout, b"stdout");
        assert_eq!(output.stderr, b"stderr");
        Ok(())
    }

    #[test]
    fn exited_child_output_survives_a_delayed_drain() -> Result<(), Box<dyn Error>> {
        let (continued, paused) = mpsc::channel();
        NEXT_DRAIN.set(DrainScheduling::Paused(paused));
        DRAINS_AFTER_CLEANUP.set(vec![continued]);
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "printf stdout; printf stderr >&2"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = with_spawn_lock(|| ChildProcess::spawn(&mut command))??;
        // The drain cannot report until group cleanup. The former implementation
        // reaches its EOF timeout first; ordinary scheduling cannot release the gate.
        let output = child.output(Vec::new(), GIT_COMMAND_DEADLINE, |_| {})?;
        assert!(output.status.success());
        assert_eq!(output.stdout, b"stdout");
        assert_eq!(output.stderr, b"stderr");
        Ok(())
    }
}
