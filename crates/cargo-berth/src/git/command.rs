//! Git command construction.

use std::io;
use std::path::Path;
use std::process::Command;
use std::process::Output;

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
