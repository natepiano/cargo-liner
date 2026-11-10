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
use rustix::process::WaitId;

/// Repository history walks and hooks may need seconds on a loaded host.
pub(super) const GIT_COMMAND_DEADLINE: Duration = Duration::from_secs(30);
/// Adopted fixture children may have pipe writers outside a group we own.
pub(super) const PIPE_EOF_DEADLINE: Duration = Duration::from_millis(250);
/// SIGKILL cleanup is bounded independently of the operation that failed.
const REAP_DEADLINE: Duration = Duration::from_millis(250);
/// SIGKILL cleanup checks exit without allocating descriptors or workers.
const REAP_CHECK_INTERVAL: Duration = Duration::from_millis(1);

use super::constants::OUTPUT_READ_BUFFER_SIZE;
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
        let output = collect_output(
            self.child.stdout.take().map(OwnedFd::from),
            self.child.stderr.take().map(OwnedFd::from),
        )?;
        let stdin = self.child.stdin.take();
        let written = match stdin {
            Some(mut stdin) => worker(move || stdin.write_all(&input))?,
            None => completed(()),
        };
        let status = self.wait(duration)?;
        after_exit(&mut self);
        let completion = self.finish_output()?;
        let OutputStreams { stdout, stderr } = output.receive(&completion, self.command())?;
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
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(
                        REAP_CHECK_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                    );
                },
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
    let Some(mut pipe) = pipe else {
        return Ok(completed(Vec::new()));
    };
    #[cfg(test)]
    let scheduling = NEXT_DRAIN.replace(DrainScheduling::Ready);
    worker(move || {
        #[cfg(test)]
        if let DrainScheduling::Paused(continued) = scheduling {
            continued.recv().map_err(io::Error::other)?;
        }
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

/// The output streams collected from one command.
struct OutputStreams {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// The active output pipes select one bounded collection path.
enum CommandOutputPipes {
    NoPipes,
    Stdout(Receiver<io::Result<Vec<u8>>>),
    Stderr(Receiver<io::Result<Vec<u8>>>),
    Both(Receiver<io::Result<OutputStreams>>),
}

impl CommandOutputPipes {
    fn receive(self, completion: &PipeCompletion, command: &str) -> io::Result<OutputStreams> {
        match self {
            Self::NoPipes => Ok(OutputStreams {
                stdout: Vec::new(),
                stderr: Vec::new(),
            }),
            Self::Stdout(output) => Ok(OutputStreams {
                stdout: completion.receive(command, &output)?,
                stderr: Vec::new(),
            }),
            Self::Stderr(output) => Ok(OutputStreams {
                stdout: Vec::new(),
                stderr: completion.receive(command, &output)?,
            }),
            Self::Both(output) => completion.receive(command, &output),
        }
    }
}

/// A command output pipe remains open until its last writer closes it.
enum OutputPipe {
    Open { pipe: OwnedFd, bytes: Vec<u8> },
    Complete(Vec<u8>),
}

impl OutputPipe {
    fn new(pipe: Option<OwnedFd>) -> io::Result<Self> {
        match pipe {
            Some(pipe) => {
                rustix::io::ioctl_fionbio(&pipe, true)?;
                Ok(Self::Open {
                    pipe,
                    bytes: Vec::new(),
                })
            },
            None => Ok(Self::Complete(Vec::new())),
        }
    }

    fn register<'pipe>(&'pipe self, descriptors: &mut Vec<PollFd<'pipe>>) {
        if let Self::Open { pipe, .. } = self {
            descriptors.push(PollFd::new(pipe, PollFlags::IN));
        }
    }

    fn read_available(&mut self) -> io::Result<()> {
        let Self::Open { pipe, bytes } = self else {
            return Ok(());
        };
        let mut buffer = [0; OUTPUT_READ_BUFFER_SIZE];
        loop {
            match rustix::io::read(&*pipe, &mut buffer) {
                Ok(0) => break,
                Ok(length) => {
                    bytes.extend_from_slice(&buffer[..length]);
                    return Ok(());
                },
                Err(rustix::io::Errno::INTR) => {},
                Err(rustix::io::Errno::AGAIN) => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
        let bytes = std::mem::take(bytes);
        *self = Self::Complete(bytes);
        Ok(())
    }

    fn into_bytes(self) -> io::Result<Vec<u8>> {
        match self {
            Self::Complete(bytes) => Ok(bytes),
            Self::Open { .. } => Err(io::Error::other(
                "output pipe remains open after collection",
            )),
        }
    }
}

/// Collect both active output pipes on one worker without blocking either pipe.
fn collect_output(
    stdout: Option<OwnedFd>,
    stderr: Option<OwnedFd>,
) -> io::Result<CommandOutputPipes> {
    let (stdout, stderr) = match (stdout, stderr) {
        (None, None) => return Ok(CommandOutputPipes::NoPipes),
        (Some(stdout), None) => {
            return drain(Some(std::fs::File::from(stdout))).map(CommandOutputPipes::Stdout);
        },
        (None, Some(stderr)) => {
            return drain(Some(std::fs::File::from(stderr))).map(CommandOutputPipes::Stderr);
        },
        (Some(stdout), Some(stderr)) => (stdout, stderr),
    };
    let mut stdout = OutputPipe::new(Some(stdout))?;
    let mut stderr = OutputPipe::new(Some(stderr))?;
    #[cfg(test)]
    let scheduling = NEXT_DRAIN.replace(DrainScheduling::Ready);
    worker(move || {
        #[cfg(test)]
        if let DrainScheduling::Paused(continued) = scheduling {
            continued.recv().map_err(io::Error::other)?;
        }
        loop {
            let mut descriptors = Vec::with_capacity(2);
            stdout.register(&mut descriptors);
            stderr.register(&mut descriptors);
            if descriptors.is_empty() {
                drop(descriptors);
                return Ok(OutputStreams {
                    stdout: stdout.into_bytes()?,
                    stderr: stderr.into_bytes()?,
                });
            }
            match rustix::event::poll(&mut descriptors, None) {
                Ok(_) => {},
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            let stdout_open = matches!(stdout, OutputPipe::Open { .. });
            let stdout_ready = stdout_open && !descriptors[0].revents().is_empty();
            let stderr_ready = matches!(stderr, OutputPipe::Open { .. })
                && !descriptors[usize::from(stdout_open)].revents().is_empty();
            drop(descriptors);
            if stdout_ready {
                stdout.read_available()?;
            }
            if stderr_ready {
                stderr.read_available()?;
            }
        }
    })
    .map(CommandOutputPipes::Both)
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
    if let Some(status) = child.try_wait()? {
        return Ok(status);
    }
    let deadline = ChildDeadline::new(command, "child exit", duration);
    let notification = ExitNotification::new(child.id())?;
    notification.receive(&deadline)?;
    child.wait()
}

/// A completed pipe task uses the same collection path without a worker.
fn completed<T>(value: T) -> Receiver<io::Result<T>> {
    let (sender, receiver) = mpsc::channel();
    let _ = sender.send(Ok(value));
    receiver
}

/// An exit event leaves the child available for its owner's final wait.
enum ExitNotification {
    #[cfg(target_os = "linux")]
    ProcessDescriptor(OwnedFd),
    Waiter(Receiver<io::Result<()>>),
}

impl ExitNotification {
    fn new(child_id: u32) -> io::Result<Self> {
        let child_id = i32::try_from(child_id).map_err(io::Error::other)?;
        let child_id = rustix::process::Pid::from_raw(child_id)
            .ok_or_else(|| io::Error::other("child process identifier must be positive"))?;
        #[cfg(target_os = "linux")]
        match rustix::process::pidfd_open(child_id, rustix::process::PidfdFlags::empty()) {
            Ok(descriptor) => return Ok(Self::ProcessDescriptor(descriptor)),
            Err(rustix::io::Errno::NOSYS | rustix::io::Errno::PERM) => {},
            Err(error) => return Err(error.into()),
        }
        // WNOWAIT retains the exit status for std::process::Child.
        let notification = worker(move || {
            loop {
                match rustix::process::waitid(
                    WaitId::Pid(child_id),
                    rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOWAIT,
                ) {
                    Ok(_) => return Ok(()),
                    Err(rustix::io::Errno::INTR) => {},
                    Err(error) => return Err(error.into()),
                }
            }
        })?;
        Ok(Self::Waiter(notification))
    }

    fn receive(&self, deadline: &ChildDeadline) -> io::Result<()> {
        match self {
            #[cfg(target_os = "linux")]
            Self::ProcessDescriptor(descriptor) => {
                let mut descriptors = [PollFd::new(descriptor, PollFlags::IN)];
                loop {
                    let remaining = deadline.end.saturating_duration_since(Instant::now());
                    let timeout = Timespec::try_from(remaining).map_err(io::Error::other)?;
                    match rustix::event::poll(&mut descriptors, Some(&timeout)) {
                        Ok(0) => return Err(deadline.timeout()),
                        Ok(_) => return Ok(()),
                        Err(rustix::io::Errno::INTR) => {},
                        Err(error) => return Err(error.into()),
                    }
                }
            },
            Self::Waiter(receiver) => deadline.receive(receiver),
        }
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

    #[derive(Clone, Copy)]
    enum OutputWorkload {
        ClosedDescendant,
        BufferedStreams,
        ContinuousStdout,
    }

    #[test]
    fn exited_child_output_ignores_descendants_that_closed_their_pipes()
    -> Result<(), Box<dyn Error>> {
        for (capture_stdout, capture_stderr, workload) in [
            (true, true, OutputWorkload::ClosedDescendant),
            (true, false, OutputWorkload::ClosedDescendant),
            (false, true, OutputWorkload::ClosedDescendant),
            (false, false, OutputWorkload::ClosedDescendant),
            (true, true, OutputWorkload::BufferedStreams),
            (true, true, OutputWorkload::ContinuousStdout),
        ] {
            let mut command = Command::new("/bin/sh");
            let script = match workload {
                OutputWorkload::ClosedDescendant => {
                    "ready=$(sleep 30 >/dev/null 2>&1 & printf ready); test \"$ready\" = ready; printf stdout; printf stderr >&2"
                },
                OutputWorkload::BufferedStreams => {
                    "head -c 131072 /dev/zero >&2; head -c 131072 /dev/zero"
                },
                OutputWorkload::ContinuousStdout => {
                    "head -c 1 /dev/zero; cat /dev/zero & writer=$!; head -c 262144 /dev/zero >&2; kill \"$writer\"; wait \"$writer\" 2>/dev/null; exit 0"
                },
            };
            command
                .args(["-c", script])
                .stdin(Stdio::null())
                .stdout(if capture_stdout {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stderr(if capture_stderr {
                    Stdio::piped()
                } else {
                    Stdio::null()
                });
            let child = with_spawn_lock(|| ChildProcess::spawn(&mut command))??;
            // The background child closes its pipes before command substitution reaches EOF.
            let output = child.output(Vec::new(), GIT_COMMAND_DEADLINE, |_| {})?;
            assert!(output.status.success());
            match workload {
                OutputWorkload::BufferedStreams => {
                    assert_eq!(output.stdout, vec![0; 131_072]);
                    assert_eq!(output.stderr, vec![0; 131_072]);
                    continue;
                },
                OutputWorkload::ContinuousStdout => {
                    assert_ne!(output.stdout, [] as [u8; 0]);
                    assert!(output.stdout.iter().all(|byte| *byte == 0));
                    assert_eq!(output.stderr, vec![0; 262_144]);
                    continue;
                },
                OutputWorkload::ClosedDescendant => {},
            }
            if capture_stdout {
                assert_eq!(output.stdout, b"stdout");
            } else {
                assert_eq!(output.stdout, [] as [u8; 0]);
            }
            if capture_stderr {
                assert_eq!(output.stderr, b"stderr");
            } else {
                assert_eq!(output.stderr, [] as [u8; 0]);
            }
        }
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
