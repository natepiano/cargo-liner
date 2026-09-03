#![allow(
    clippy::expect_used,
    reason = "tests should stop immediately when git or a fixture is wrong"
)]

//! Git invocation shared by the `cargo-berth` integration tests.
//!
//! Each integration test is its own crate, so before this existed every test
//! file grew its own run-and-capture pair around `Command::new("git")`. They
//! differed only in policy — whether git may take optional locks, and which
//! variables the test process must keep out of a hook — so the policy stays with
//! each file and the invoking lives here.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::process::Output;

/// Names a specific `cargo-berth` for a managed hook, ahead of the installed one.
pub const EXECUTABLE_ENVIRONMENT: &str = "CARGO_BERTH_EXECUTABLE";

/// Start a git command whose managed hooks run the `cargo-berth` under test.
///
/// A managed hook resolves `cargo-berth` when it runs, and an installed copy
/// answers that resolution first. Every git command a test issues can fire a
/// hook, so each one names the built binary and the environment carries it
/// through git into the hook. Without this a test proves nothing about the code
/// it was compiled from: it reports on whatever `cargo install` last left behind.
///
/// `executable` is the test crate's own `env!("CARGO_BIN_EXE_cargo-berth")`,
/// which only that crate can expand.
#[must_use]
pub fn git_command(executable: &str) -> Command {
    let mut command = Command::new("git");
    command.env(EXECUTABLE_ENVIRONMENT, executable);
    command
}

/// Whether git may take the optional locks it caches its own work under.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum OptionalLocks {
    /// Let git take them, as an ordinary working checkout does.
    Taken,
    /// Refuse them, so reading a repository never writes to it.
    Refused,
}

/// How one test file drives git against the repository its fixtures build.
#[derive(Clone, Copy)]
pub struct GitDriver {
    /// The `cargo-berth` a managed hook must run, from the test crate's own
    /// `env!("CARGO_BIN_EXE_cargo-berth")`.
    pub executable:          &'static str,
    /// Whether git may take the locks it would otherwise write while reading.
    pub optional_locks:      OptionalLocks,
    /// Variables cleared before git runs, so a hook cannot inherit this process's.
    pub cleared_environment: &'static [&'static str],
}

impl GitDriver {
    /// Run git and assert it succeeded.
    ///
    /// # Panics
    ///
    /// Panics when git cannot be started or reports failure.
    pub fn run<Arguments, Argument>(self, repository_root: &Path, arguments: Arguments)
    where
        Arguments: IntoIterator<Item = Argument>,
        Argument: AsRef<OsStr>,
    {
        let (mut command, arguments) = self.prepare(repository_root, arguments);
        let output = capture(&mut command);
        assert_success(&output, &arguments);
    }

    /// Run git, assert it succeeded, and return its trimmed standard output.
    ///
    /// # Panics
    ///
    /// Panics when git cannot be started, reports failure, or writes output
    /// that is not UTF-8.
    #[must_use]
    pub fn stdout<Arguments, Argument>(self, repository_root: &Path, arguments: Arguments) -> String
    where
        Arguments: IntoIterator<Item = Argument>,
        Argument: AsRef<OsStr>,
    {
        let (mut command, arguments) = self.prepare(repository_root, arguments);
        let output = capture(&mut command);
        assert_success(&output, &arguments);
        String::from_utf8(output.stdout)
            .expect("git output should be UTF-8")
            .trim()
            .to_owned()
    }

    /// Run git and return its captured result, whether it succeeded or not.
    ///
    /// # Panics
    ///
    /// Panics when git cannot be started.
    #[must_use]
    pub fn output<Arguments, Argument>(self, repository_root: &Path, arguments: Arguments) -> Output
    where
        Arguments: IntoIterator<Item = Argument>,
        Argument: AsRef<OsStr>,
    {
        let (mut command, _) = self.prepare(repository_root, arguments);
        capture(&mut command)
    }

    /// Run git with one variable set, overriding whatever this policy clears.
    ///
    /// The variable is applied after the policy's clears, so a test can hand a
    /// hook exactly the value the policy exists to keep out of it.
    ///
    /// # Panics
    ///
    /// Panics when git cannot be started.
    #[must_use]
    pub fn output_with_environment(
        self,
        repository_root: &Path,
        arguments: &[&str],
        name: &str,
        value: &str,
    ) -> Output {
        let (mut command, _) = self.prepare(repository_root, arguments);
        command.env(name, value);
        capture(&mut command)
    }

    /// Run git without capturing its streams and report whether it succeeded.
    ///
    /// # Panics
    ///
    /// Panics when git cannot be started.
    #[must_use]
    pub fn succeeds(self, repository_root: &Path, arguments: &[&str]) -> bool {
        let (mut command, _) = self.prepare(repository_root, arguments);
        command.status().expect("git should run").success()
    }

    /// Build the command this policy describes, and its arguments for diagnostics.
    fn prepare<Arguments, Argument>(
        self,
        repository_root: &Path,
        arguments: Arguments,
    ) -> (Command, Vec<OsString>)
    where
        Arguments: IntoIterator<Item = Argument>,
        Argument: AsRef<OsStr>,
    {
        let arguments: Vec<OsString> = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_owned())
            .collect();
        let mut command = git_command(self.executable);
        if self.optional_locks == OptionalLocks::Refused {
            command.arg("--no-optional-locks");
        }
        command.args(&arguments).current_dir(repository_root);
        for name in self.cleared_environment {
            command.env_remove(name);
        }
        (command, arguments)
    }
}

fn capture(command: &mut Command) -> Output { command.output().expect("git should run") }

fn assert_success(output: &Output, arguments: &[OsString]) {
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
