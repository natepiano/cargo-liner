//! Branch-owned index paths during a pending merge of the target branch.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use super::command;
use super::constants::GIT_CACHED_ARG;
use super::constants::GIT_DIFF_COMMAND;
use super::constants::GIT_MERGE_STATE_PATH;
use super::constants::GIT_NAME_ONLY_ARG;
use super::constants::GIT_NO_RENAMES_ARG;
use super::constants::GIT_NUL_TERMINATED_ARG;
use super::constants::GIT_PATHSPEC_SEPARATOR;
use super::error::GitError;
use super::reachability;
use super::reachability::Reachability;
use super::worktree_operation::WorktreeOperation;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;
use crate::ledger::WorktreeContext;

/// The index paths that remain branch work during a merge of the target branch.
pub(crate) enum TrunkMergeIndex {
    /// No supported merge of the target branch is pending; keep every dirty path.
    Absent,
    /// These index paths differ from the merged target commit, including conflicts.
    Pending {
        /// Preserve staged work and conflict stages that differ from the target side.
        own_paths: HashSet<ReservationScopePath>,
    },
}

impl TrunkMergeIndex {
    /// Exclude imported index content only when the target already contains its merge parent.
    pub(crate) fn observe(root: &Path, trunk: &GitObjectId) -> Result<Self, GitError> {
        let worktree_context =
            WorktreeContext::discover(root).map_err(|error| GitError::RepositoryReadFailed {
                activity:   "locate merge state",
                diagnostic: error.to_string(),
            })?;
        if WorktreeOperation::observe(worktree_context.administrative_directory())
            != WorktreeOperation::Merge
        {
            return Ok(Self::Absent);
        }
        let contents = match fs::read_to_string(
            worktree_context
                .administrative_directory()
                .join(GIT_MERGE_STATE_PATH),
        ) {
            Ok(contents) => contents,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::Absent),
            Err(error) => return Err(error.into()),
        };
        let parents = contents
            .split_whitespace()
            .map(|parent| parent.parse().map_err(GitError::InvalidObjectId))
            .collect::<Result<Vec<GitObjectId>, _>>()?;
        let [parent] = parents.as_slice() else {
            return Ok(Self::Absent);
        };
        if reachability::reachability(root, parent, trunk)? != Reachability::Ancestor {
            return Ok(Self::Absent);
        }
        let parent = parent.to_string();
        let output = command::git_output(
            root,
            [
                GIT_DIFF_COMMAND,
                GIT_CACHED_ARG,
                GIT_NAME_ONLY_ARG,
                GIT_NUL_TERMINATED_ARG,
                GIT_NO_RENAMES_ARG,
                &parent,
                GIT_PATHSPEC_SEPARATOR,
            ],
        )?;
        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: GIT_DIFF_COMMAND,
                stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        let own_paths = String::from_utf8(output.stdout)
            .map_err(GitError::InvalidOutput)?
            .split_terminator('\0')
            .map(|path| path.parse().map_err(GitError::InvalidReservationPath))
            .collect::<Result<HashSet<_>, _>>()?;
        Ok(Self::Pending { own_paths })
    }
}
