//! The small git subprocess surface required by the ledger.

mod command;
mod constants;

use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use command::git_output;
use constants::GIT_COMMON_DIRECTORY_ARG;
use constants::GIT_REV_PARSE_COMMAND;
use constants::GIT_SHOW_TOPLEVEL_ARG;

/// Resolve the shared administrative directory for a repository worktree.
pub(crate) fn common_directory(repository_root: &Path) -> Result<PathBuf, GitError> {
    let output = git_output(
        repository_root,
        [GIT_REV_PARSE_COMMAND, GIT_COMMON_DIRECTORY_ARG],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let git_directory = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let path = PathBuf::from(git_directory.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        repository_root.join(path)
    })
}

/// Resolve the worktree root for an invocation from anywhere in the repository.
pub(crate) fn repository_root(invocation_directory: &Path) -> Result<PathBuf, GitError> {
    let output = git_output(
        invocation_directory,
        [GIT_REV_PARSE_COMMAND, GIT_SHOW_TOPLEVEL_ARG],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let repository_root = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let path = PathBuf::from(repository_root.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        invocation_directory.join(path)
    })
}

/// A failure while resolving git's shared administrative directory.
#[derive(Debug)]
pub(crate) enum GitError {
    /// Git could not be started or read.
    Io(std::io::Error),
    /// Git completed unsuccessfully.
    CommandFailed {
        /// The git subcommand that failed.
        command: &'static str,
        /// The diagnostic git reported.
        stderr:  String,
    },
    /// Git printed a non-UTF-8 administrative path.
    InvalidOutput(std::string::FromUtf8Error),
}

impl fmt::Display for GitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not run git: {error}"),
            Self::CommandFailed { command, stderr } => {
                write!(formatter, "git {command} failed: {stderr}")
            },
            Self::InvalidOutput(error) => {
                write!(formatter, "git printed a non-UTF-8 path: {error}")
            },
        }
    }
}

impl std::error::Error for GitError {}

impl From<std::io::Error> for GitError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}
