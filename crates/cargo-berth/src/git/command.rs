//! Git command construction.

use std::ffi::OsStr;
use std::io;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::thread;

use super::constants::GIT_BINARY;
use super::constants::GIT_NO_OPTIONAL_LOCKS_ARG;

/// Whether a git invocation produced a complete process output.
pub(super) enum GitCommandExecution {
    /// The process ran to completion, including a possible non-zero exit status.
    Completed(Output),
    /// The invocation returned an I/O error instead of a process output.
    CouldNotRun,
}

impl From<io::Result<Output>> for GitCommandExecution {
    fn from(output: io::Result<Output>) -> Self {
        output.map_or(Self::CouldNotRun, Self::Completed)
    }
}

/// Build a git subprocess rooted at `repository_root` without optional locks.
pub(super) fn git_command(repository_root: &Path) -> Command {
    let mut command = Command::new(GIT_BINARY);
    command
        .arg(GIT_NO_OPTIONAL_LOCKS_ARG)
        .current_dir(repository_root);
    command
}

/// Run one git operation and return its complete output.
pub(super) fn git_output<const ARGUMENT_COUNT: usize>(
    repository_root: &Path,
    arguments: [&str; ARGUMENT_COUNT],
) -> io::Result<Output> {
    git_command(repository_root).args(arguments).output()
}

/// Run one git operation whose revision arguments are assembled at runtime.
pub(super) fn git_output_dynamic(
    repository_root: &Path,
    arguments: &[String],
) -> io::Result<Output> {
    git_command(repository_root).args(arguments).output()
}

/// Run one dynamically assembled git operation with explicit environment values.
pub(super) fn git_output_dynamic_with_environment(
    repository_root: &Path,
    arguments: &[String],
    environment: &[(&str, &OsStr)],
) -> io::Result<Output> {
    git_command(repository_root)
        .args(arguments)
        .envs(environment.iter().copied())
        .output()
}

/// Run one dynamically assembled git operation with complete standard input.
pub(super) fn git_output_dynamic_with_input(
    repository_root: &Path,
    arguments: &[String],
    input: &[u8],
) -> io::Result<Output> {
    let mut command = git_command(repository_root);
    command.args(arguments);
    command_output_with_input(command, input)
}

/// Run one dynamically assembled git operation with environment values and standard input.
pub(super) fn git_output_dynamic_with_environment_and_input(
    repository_root: &Path,
    arguments: &[String],
    environment: &[(&str, &OsStr)],
    input: &[u8],
) -> io::Result<Output> {
    let mut command = git_command(repository_root);
    command.args(arguments).envs(environment.iter().copied());
    command_output_with_input(command, input)
}

/// Run a configured git command while writing its complete standard input concurrently.
fn command_output_with_input(mut command: Command, input: &[u8]) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("git child standard input was not piped"))?;
    thread::scope(|scope| {
        let input_writer = scope.spawn(move || {
            let result = stdin.write_all(input);
            drop(stdin);
            result
        });
        let output = child.wait_with_output();
        input_writer
            .join()
            .map_err(|_| io::Error::other("git child standard input writer panicked"))??;
        output
    })
}
