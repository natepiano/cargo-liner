//! Git command construction.

use std::io;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::thread;

use super::constants::GIT_BINARY;
use super::constants::GIT_NO_OPTIONAL_LOCKS_ARG;

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

/// Run one dynamically assembled git operation with complete standard input.
pub(super) fn git_output_dynamic_with_input(
    repository_root: &Path,
    arguments: &[String],
    input: &[u8],
) -> io::Result<Output> {
    let mut child = git_command(repository_root)
        .args(arguments)
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
