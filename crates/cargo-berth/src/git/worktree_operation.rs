//! Git state that makes a worktree's intermediate paths unsuitable for drift.

use std::path::Path;

use super::constants::GIT_CHERRY_PICK_STATE_PATH;
use super::constants::GIT_INDEX_LOCK_PATH;
use super::constants::GIT_MERGE_STATE_PATH;
use super::constants::GIT_REBASE_APPLY_STATE_PATH;
use super::constants::GIT_REBASE_MERGE_STATE_PATH;
use super::constants::GIT_REVERT_STATE_PATH;

/// Whether Git is currently replacing this worktree's index and checking out paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorktreeSettlement {
    Settled,
    GitOperationInProgress,
}

impl WorktreeSettlement {
    /// Linked worktrees have their own index lock, separate from the common Git directory.
    pub(crate) fn observe(worktree_administrative_directory: &Path) -> Self {
        if worktree_administrative_directory
            .join(GIT_INDEX_LOCK_PATH)
            .exists()
        {
            Self::GitOperationInProgress
        } else {
            Self::Settled
        }
    }
}

/// The operation Git records in a worktree's administrative directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorktreeOperation {
    /// Git records no pending operation.
    Idle,
    /// A rebase is replaying commits or waiting for conflict resolution.
    Rebase,
    /// A merge is waiting to be committed or aborted.
    Merge,
    /// A cherry-pick stopped before completing its commit.
    CherryPick,
    /// A revert stopped before completing its commit.
    Revert,
}

/// One marker set serves drift, enrollment, and rewrite reconciliation.
const WORKTREE_OPERATION_MARKERS: [(&str, WorktreeOperation); 5] = [
    (GIT_REBASE_MERGE_STATE_PATH, WorktreeOperation::Rebase),
    (GIT_REBASE_APPLY_STATE_PATH, WorktreeOperation::Rebase),
    (GIT_MERGE_STATE_PATH, WorktreeOperation::Merge),
    (GIT_CHERRY_PICK_STATE_PATH, WorktreeOperation::CherryPick),
    (GIT_REVERT_STATE_PATH, WorktreeOperation::Revert),
];

impl WorktreeOperation {
    /// Observe pending work before interpreting the worktree's paths or history.
    ///
    /// Rebase markers take precedence over markers written while replaying a commit.
    /// A completed `--no-commit` cherry-pick or revert leaves no pending marker and
    /// is therefore an ordinary dirty worktree.
    pub(crate) fn observe(worktree_administrative_directory: &Path) -> Self {
        WORKTREE_OPERATION_MARKERS
            .iter()
            .find_map(|(marker, operation)| {
                worktree_administrative_directory
                    .join(marker)
                    .exists()
                    .then_some(*operation)
            })
            .unwrap_or(Self::Idle)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::GIT_CHERRY_PICK_STATE_PATH;
    use super::GIT_MERGE_STATE_PATH;
    use super::GIT_REBASE_APPLY_STATE_PATH;
    use super::GIT_REBASE_MERGE_STATE_PATH;
    use super::GIT_REVERT_STATE_PATH;
    use super::WorktreeOperation;

    #[test]
    fn observe_names_each_pending_operation_and_prefers_rebase()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        assert_eq!(
            WorktreeOperation::observe(directory.path()),
            WorktreeOperation::Idle
        );
        for (marker, expected) in [
            (GIT_REBASE_MERGE_STATE_PATH, WorktreeOperation::Rebase),
            (GIT_REBASE_APPLY_STATE_PATH, WorktreeOperation::Rebase),
            (GIT_MERGE_STATE_PATH, WorktreeOperation::Merge),
            (GIT_CHERRY_PICK_STATE_PATH, WorktreeOperation::CherryPick),
            (GIT_REVERT_STATE_PATH, WorktreeOperation::Revert),
        ] {
            let marker_path = directory.path().join(marker);
            fs::write(&marker_path, "pending")?;
            assert_eq!(WorktreeOperation::observe(directory.path()), expected);
            fs::remove_file(marker_path)?;
        }
        // `GIT_REBASE_MERGE_STATE_PATH` and `GIT_CHERRY_PICK_STATE_PATH` can coexist because a
        // stopped replay can expose another operation's marker at the same time.
        fs::create_dir(directory.path().join(GIT_REBASE_MERGE_STATE_PATH))?;
        fs::write(directory.path().join(GIT_CHERRY_PICK_STATE_PATH), "pending")?;
        assert_eq!(
            WorktreeOperation::observe(directory.path()),
            WorktreeOperation::Rebase
        );
        Ok(())
    }
}
