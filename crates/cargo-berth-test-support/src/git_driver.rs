//! Git invocation shared by the `cargo-berth` integration tests.
//!
//! Before `GitDriver` existed every test file grew its own run-and-capture pair
//! around `Command::new("git")`. They differed only in policy — whether git may
//! take optional locks, and which variables the test process must keep out of a
//! hook — so the policy stays with each file and the invoking lives here.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use crate::berth_command::CLAUDE_CODE_SESSION_ENVIRONMENT;

/// Names a specific `cargo-berth` for a managed hook, ahead of the installed one.
pub const EXECUTABLE_ENVIRONMENT: &str = "CARGO_BERTH_EXECUTABLE";

/// Points git's hook lookup at a path that holds no hook on any platform.
const HOOKS_DISABLED_CONFIGURATION: &str = "core.hooksPath=/dev/null";

/// Start a git command whose managed hooks run the `cargo-berth` under test.
///
/// A managed hook resolves `cargo-berth` when it runs, and an installed copy
/// answers that resolution first. Every git command a test issues can fire a
/// hook, so each one names the built binary and the environment carries it
/// through git into the hook. Without this a test proves nothing about the code
/// it was compiled from: it reports on whatever `cargo install` last left behind.
///
/// The hook's `cargo-berth` would read this process's Claude Code session as its
/// harness session, so the command clears `CLAUDE_CODE_SESSION_ENVIRONMENT`.
///
/// `executable` is the test crate's own `env!("CARGO_BIN_EXE_cargo-berth")`,
/// which only that crate can expand.
#[must_use]
pub fn git_command(executable: &str) -> Command {
    let mut command = Command::new("git");
    command
        .env(EXECUTABLE_ENVIRONMENT, executable)
        .env_remove(CLAUDE_CODE_SESSION_ENVIRONMENT);
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

    /// Run git without running any hook, and assert it succeeded.
    ///
    /// Only for a fixture step whose hooked form changes no `cargo-berth` state, as
    /// a gate test proves for each kind: a configuration commit on a trunk with no
    /// reservation
    /// (`hooked_configuration_commit_on_an_unreserved_trunk_leaves_berth_state_unchanged`), and
    /// a switch to a new or existing branch
    /// (`hooked_branch_switches_of_the_recovery_fixture_leave_berth_state_unchanged`).
    /// Any other commit keeps its hooks: the post-commit drift check records it,
    /// even with no reservation in the ledger.
    ///
    /// # Panics
    ///
    /// Panics when git cannot be started or reports failure.
    pub fn run_without_hooks<Arguments, Argument>(
        self,
        repository_root: &Path,
        arguments: Arguments,
    ) where
        Arguments: IntoIterator<Item = Argument>,
        Argument: AsRef<OsStr>,
    {
        let hooks_disabled =
            [OsStr::new("-c"), OsStr::new(HOOKS_DISABLED_CONFIGURATION)].map(OsStr::to_owned);
        self.run(
            repository_root,
            hooks_disabled.into_iter().chain(
                arguments
                    .into_iter()
                    .map(|argument| argument.as_ref().to_owned()),
            ),
        );
    }

    /// Add a linked worktree at `worktree_root` on a new `branch` started from
    /// `start_point`, without running any hook.
    ///
    /// A hooked add starts `cargo-berth` from the `reference-transaction` hook for
    /// the new branch, about 90 processes in all. For a branch that is neither the
    /// trunk nor a listed gate target, with no apply rebase stopped in
    /// `repository_root`, that run changes nothing in the repository, as the gate
    /// test `hooked_worktree_add_of_a_new_branch_leaves_the_repository_unchanged`
    /// proves. A fixture add under those conditions skips the hook and starts
    /// only git.
    ///
    /// # Panics
    ///
    /// Panics when git cannot be started or reports failure.
    pub fn add_worktree_without_hooks(
        self,
        repository_root: &Path,
        worktree_root: &Path,
        branch: &str,
        start_point: &str,
    ) {
        self.run_without_hooks(
            repository_root,
            [
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("--quiet"),
                OsStr::new("-b"),
                OsStr::new(branch),
                worktree_root.as_os_str(),
                OsStr::new(start_point),
            ],
        );
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
