//! Ownership of a test command and the descendants its pausing shims start.

use std::io;
use std::process::Child;
use std::process::Command;
use std::process::Output;

use crate::child_process;
use crate::child_process::ChildProcess;
use crate::child_process::GIT_COMMAND_DEADLINE;

/// A test command whose entire process group is terminated when its owner unwinds.
/// Keep this guard before the temporary directories used by a pausing shim are removed.
pub struct TestProcessGroup {
    process: ChildProcess,
}

impl TestProcessGroup {
    /// Start a command in its own group, preserving its configured streams.
    ///
    /// # Errors
    /// Returns the operating system error if the command cannot start.
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        Ok(Self {
            process: child_process::with_spawn_lock(|| ChildProcess::spawn(command))??,
        })
    }

    /// Access the owned child to supply a hook payload or inspect readiness.
    pub const fn child_mut(&mut self) -> &mut Child { self.process.child_mut() }

    /// Collect output after the test continues its shim, ending remaining descendants.
    ///
    /// # Errors
    /// Returns a named timeout or an I/O error when the child or its pipes fail.
    pub fn wait_with_output(self) -> io::Result<Output> { self.output_with_input(&[]) }

    /// Supply input while draining output, ending the group on exit or timeout.
    ///
    /// # Errors
    /// Returns a named timeout or a pipe, worker, or wait error.
    pub fn output_with_input(self, input: &[u8]) -> io::Result<Output> {
        self.process
            .output(input.to_vec(), GIT_COMMAND_DEADLINE, ChildProcess::stop)
    }
}
