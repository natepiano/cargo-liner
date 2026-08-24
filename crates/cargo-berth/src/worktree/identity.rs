//! Validation that a registered worktree still represents the recorded holder.

use std::fs;
use std::path::Path;

use crate::ids::RepoInstanceId;
use crate::ids::WorktreeId;
use crate::ids::WorktreeKind;
use crate::ledger;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::LedgerError;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::ledger::WorktreeContext;

/// The validated location of the same non-recyclable worktree holder.
pub(super) enum ValidatedWorktreeOwner {
    /// The opaque identity remains at the recorded canonical root.
    RecordedRoot,
    /// The opaque identity remains valid after `git worktree move` changed its root.
    Relocated { current_root: CanonicalWorktreeRoot },
}

/// Validate repository, administrative directory, backlink, identity, and root together.
pub(super) fn validate_same_owner(
    ledger_repository: RepoInstanceId,
    recorded_repository: RepoInstanceId,
    common_git_directory: &Path,
    recorded_worktree_id: WorktreeId,
    recorded_root: &CanonicalWorktreeRoot,
    recorded_locator: &WorktreeAdministrativeLocator,
    candidate: &WorktreeContext,
) -> Result<ValidatedWorktreeOwner, LedgerError> {
    if recorded_repository != ledger_repository
        || candidate.common_git_directory() != common_git_directory
        || candidate.administrative_locator() != recorded_locator
        || ledger::read_worktree_identity(candidate.administrative_directory())?
            != recorded_worktree_id
        || !backlink_matches(candidate)?
    {
        return Err(LedgerError::WorktreeIdentityMismatch);
    }
    let current_root = candidate
        .repository_root()
        .to_str()
        .ok_or(LedgerError::NonUtf8AdministrativePath)?
        .parse()
        .map_err(|_| LedgerError::InvalidCanonicalWorktreeRoot)?;
    if &current_root == recorded_root {
        Ok(ValidatedWorktreeOwner::RecordedRoot)
    } else {
        Ok(ValidatedWorktreeOwner::Relocated { current_root })
    }
}

fn backlink_matches(candidate: &WorktreeContext) -> Result<bool, LedgerError> {
    match candidate.worktree_kind() {
        WorktreeKind::Main => {
            Ok(candidate.administrative_directory() == candidate.common_git_directory())
        },
        WorktreeKind::Linked => {
            let backlink = fs::read_to_string(candidate.administrative_directory().join("gitdir"))?;
            let backlink = Path::new(backlink.trim());
            let backlink = if backlink.is_absolute() {
                backlink.to_path_buf()
            } else {
                candidate.administrative_directory().join(backlink)
            };
            Ok(fs::canonicalize(backlink)?
                == fs::canonicalize(candidate.repository_root().join(".git"))?)
        },
    }
}
