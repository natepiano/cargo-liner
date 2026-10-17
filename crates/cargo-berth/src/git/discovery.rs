//! Discovery of a repository's directories and its registered worktrees.
//!
//! Every function here answers "where does git keep this?" for one invocation: the
//! worktree root, the shared administrative directory, the hook directory git resolves
//! after `core.hooksPath`, and the registered-worktree listing.

use std::path::Path;
use std::path::PathBuf;

use super::command;
use super::constants::GIT_COMMON_DIRECTORY_ARG;
use super::constants::GIT_HOOKS_PATH;
use super::constants::GIT_NUL_TERMINATED_ARG;
use super::constants::GIT_PATH_ARG;
use super::constants::GIT_PATH_FORMAT_ABSOLUTE_ARG;
use super::constants::GIT_PORCELAIN_ARG;
use super::constants::GIT_REV_PARSE_COMMAND;
use super::constants::GIT_SHOW_TOPLEVEL_ARG;
use super::constants::GIT_WORKTREE_COMMAND;
use super::constants::GIT_WORKTREE_LIST_ARG;
use super::error::GitError;

/// Resolve the shared administrative directory for a repository worktree.
pub(crate) fn common_directory(repository_root: &Path) -> Result<PathBuf, GitError> {
    let output = command::git_output(
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
    let output = command::git_output(
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

/// Resolve the hook directory Git uses after applying `core.hooksPath`.
pub(crate) fn hooks_directory(repository_root: &Path) -> Result<PathBuf, GitError> {
    let output = command::git_output(
        repository_root,
        [
            GIT_REV_PARSE_COMMAND,
            GIT_PATH_FORMAT_ABSOLUTE_ARG,
            GIT_PATH_ARG,
            GIT_HOOKS_PATH,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let hooks_directory = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    Ok(PathBuf::from(hooks_directory.trim()))
}

/// Read git's NUL-delimited registered-worktree representation.
pub(crate) fn worktree_list_porcelain(repository_root: &Path) -> Result<Vec<u8>, GitError> {
    let output = command::git_output(
        repository_root,
        [
            GIT_WORKTREE_COMMAND,
            GIT_WORKTREE_LIST_ARG,
            GIT_PORCELAIN_ARG,
            GIT_NUL_TERMINATED_ARG,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_WORKTREE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output.stdout)
}
