//! Bounded standard-library command and child waits for fixtures.

use std::io;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;

use crate::child_process;
use crate::child_process::ChildProcess;
use crate::child_process::DescendantCleanup;
use crate::child_process::GIT_COMMAND_DEADLINE;
use crate::child_process::PIPE_EOF_DEADLINE;

/// Fixture command execution with the shared child-process deadline.
pub trait CommandDeadlineExt {
    /// Collect output with a named timeout and process-group cleanup.
    ///
    /// # Errors
    /// Returns a spawn error, named timeout or pipe error.
    fn bounded_output(&mut self) -> io::Result<Output>;

    /// Wait for command exit with a named timeout.
    ///
    /// # Errors
    /// Returns a spawn error, named timeout or pipe error.
    fn bounded_status(&mut self) -> io::Result<ExitStatus>;
}

impl CommandDeadlineExt for Command {
    fn bounded_output(&mut self) -> io::Result<Output> { crate::command_output(self) }

    fn bounded_status(&mut self) -> io::Result<ExitStatus> {
        let mut process = child_process::with_spawn_lock(|| ChildProcess::spawn(self))??;
        process.wait(GIT_COMMAND_DEADLINE)
    }
}

/// Bounded waits for fixtures that already own a standard-library child.
pub trait ChildDeadlineExt {
    /// Drain and collect the child with a deadline, without owning its parent's group.
    ///
    /// # Errors
    /// Returns a named timeout, wait error, or pipe error.
    fn bounded_wait_with_output(self) -> io::Result<Output>;

    /// Reap a child without waiting forever after an unsuccessful termination.
    ///
    /// # Errors
    /// Returns a named timeout or a wait error.
    fn bounded_wait(&mut self) -> io::Result<ExitStatus>;
}

impl ChildDeadlineExt for Child {
    fn bounded_wait_with_output(self) -> io::Result<Output> {
        let command = format!("child {}", self.id());
        ChildProcess::from_child(self, command, DirectChildOnly).output(
            Vec::new(),
            GIT_COMMAND_DEADLINE,
            |_| {},
        )
    }

    fn bounded_wait(&mut self) -> io::Result<ExitStatus> {
        let command = format!("child {}", self.id());
        let result = child_process::wait_for_exit(self, &command, GIT_COMMAND_DEADLINE);
        if result.is_err() {
            let _ = self.kill();
            let _ = child_process::wait_for_exit(self, &command, PIPE_EOF_DEADLINE);
        }
        result
    }
}

/// A standard-library child whose parent's process group is never ours to stop.
struct DirectChildOnly;

impl DescendantCleanup for DirectChildOnly {
    fn stop_descendants(&self, _: u32) {}
}
