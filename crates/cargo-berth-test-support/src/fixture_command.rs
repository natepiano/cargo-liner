//! Fixture commands that cannot bypass the shared pipe-creation and spawn boundary.

use std::ffi::OsStr;
use std::io;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;
use std::process::Stdio;

use crate::ChildDeadlineExt as _;

/// A fixture command whose launches share Git's pipe-creation lock.
/// The standard command stays private so builder calls cannot escape that boundary.
#[derive(Debug)]
pub struct FixtureCommand {
    command: Command,
}

impl FixtureCommand {
    /// Configure a fixture executable without starting it.
    #[must_use]
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            command: Command::new(program),
        }
    }

    /// Append one literal argument.
    pub fn arg(&mut self, argument: impl AsRef<OsStr>) -> &mut Self {
        self.command.arg(argument);
        self
    }

    /// Append literal arguments.
    pub fn args<I, S>(&mut self, arguments: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(arguments);
        self
    }

    /// Set the child's working directory.
    pub fn current_dir(&mut self, directory: impl AsRef<Path>) -> &mut Self {
        self.command.current_dir(directory);
        self
    }

    /// Set one child environment variable.
    pub fn env(&mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.command.env(name, value);
        self
    }

    /// Set child environment variables.
    pub fn envs<I, K, V>(&mut self, variables: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.command.envs(variables);
        self
    }

    /// Remove one child environment variable.
    pub fn env_remove(&mut self, name: impl AsRef<OsStr>) -> &mut Self {
        self.command.env_remove(name);
        self
    }

    /// Remove the inherited environment.
    pub fn env_clear(&mut self) -> &mut Self {
        self.command.env_clear();
        self
    }

    /// Configure the child's input stream.
    pub fn stdin(&mut self, stream: impl Into<Stdio>) -> &mut Self {
        self.command.stdin(stream);
        self
    }

    /// Configure the child's output stream for a spawned child.
    pub fn stdout(&mut self, stream: impl Into<Stdio>) -> &mut Self {
        self.command.stdout(stream);
        self
    }

    /// Configure the child's error stream for a spawned child.
    pub fn stderr(&mut self, stream: impl Into<Stdio>) -> &mut Self {
        self.command.stderr(stream);
        self
    }

    /// Choose a process group for a directly spawned child.
    pub fn process_group(&mut self, group: i32) -> &mut Self {
        self.command.process_group(group);
        self
    }

    /// Launch with configured streams, serialized against every fixture pipe setup.
    ///
    /// # Errors
    /// Returns a spawn error or a poisoned spawn-lock error.
    pub fn spawn(&mut self) -> io::Result<Child> {
        crate::with_spawn_lock(|| self.command.spawn())?
    }

    /// Capture both output streams with the shared exit and cleanup deadlines.
    ///
    /// # Errors
    /// Returns a spawn, timeout, or pipe error.
    pub fn output(&mut self) -> io::Result<Output> { crate::command_output(&mut self.command) }

    /// Wait for a child using its configured streams and the fixture deadline.
    ///
    /// # Errors
    /// Returns a spawn or wait error.
    pub fn status(&mut self) -> io::Result<ExitStatus> { self.spawn()?.bounded_wait() }

    pub(crate) const fn command_mut(&mut self) -> &mut Command { &mut self.command }
}
