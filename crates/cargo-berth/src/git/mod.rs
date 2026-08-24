//! The small git subprocess surface required by the ledger.

mod command;
mod constants;
mod refs;

use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use command::git_output;
use constants::GIT_COMMON_DIRECTORY_ARG;
use constants::GIT_HEAD_REVISION;
use constants::GIT_IS_ANCESTOR_ARG;
use constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use constants::GIT_MERGE_BASE_COMMAND;
use constants::GIT_NOT_ANCESTOR_EXIT_CODE;
use constants::GIT_REV_PARSE_COMMAND;
use constants::GIT_SHOW_TOPLEVEL_ARG;

use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::ReservationId;

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

/// Read the full object id currently named by `HEAD`.
pub(crate) fn head_object_id(repository_root: &Path) -> Result<GitObjectId, GitError> {
    object_id(repository_root, GIT_HEAD_REVISION)
}

/// Read the full object id currently named by a local branch.
pub(crate) fn branch_object_id(
    repository_root: &Path,
    branch: &str,
) -> Result<GitObjectId, GitError> {
    object_id(
        repository_root,
        &format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}"),
    )
}

/// Determine whether one commit is an ancestor of another.
pub(crate) fn reachability(
    repository_root: &Path,
    ancestor: &GitObjectId,
    descendant: &GitObjectId,
) -> Result<Reachability, GitError> {
    let ancestor = ancestor.to_string();
    let descendant = descendant.to_string();
    let output = git_output(
        repository_root,
        [
            GIT_MERGE_BASE_COMMAND,
            GIT_IS_ANCESTOR_ARG,
            &ancestor,
            &descendant,
        ],
    )?;
    if output.status.success() {
        Ok(Reachability::Ancestor)
    } else if output.status.code() == Some(GIT_NOT_ANCESTOR_EXIT_CODE) {
        Ok(Reachability::NotAncestor)
    } else {
        Ok(Reachability::ObjectUnknown)
    }
}

/// Create or update a reservation's retention ref.
pub(crate) fn write_reservation_retention_ref(
    repository_root: &Path,
    reservation_id: ReservationId,
    protected_tip: &GitObjectId,
) -> Result<(), GitError> {
    refs::write(repository_root, reservation_id, protected_tip)
}

/// Delete a reservation's retention ref.
#[expect(
    dead_code,
    reason = "a retention ref lives until every dependent successor is terminal, so nothing deletes one yet"
)]
pub(crate) fn delete_reservation_retention_ref(
    repository_root: &Path,
    reservation_id: ReservationId,
) -> Result<(), GitError> {
    refs::delete(repository_root, reservation_id)
}

fn object_id(repository_root: &Path, revision: &str) -> Result<GitObjectId, GitError> {
    let output = git_output(repository_root, [GIT_REV_PARSE_COMMAND, revision])?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let object_id = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    GitObjectId::from_str(object_id.trim()).map_err(GitError::InvalidObjectId)
}

/// The three outcomes of `git merge-base --is-ancestor`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Reachability {
    /// The first commit is reachable from the second.
    Ancestor,
    /// Both objects exist, but the first is not reachable from the second.
    NotAncestor,
    /// Git could not read one or both objects.
    ObjectUnknown,
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
    /// Git printed text that was not a full object id.
    InvalidObjectId(InvalidGitObjectId),
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
            Self::InvalidObjectId(error) => {
                write!(formatter, "git printed an invalid object id: {error}")
            },
        }
    }
}

impl std::error::Error for GitError {}

impl From<std::io::Error> for GitError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}
