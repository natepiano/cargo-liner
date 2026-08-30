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

/// The opaque identity observed with one registered-worktree snapshot.
#[derive(Clone, Copy)]
pub(super) enum RegisteredWorktreeIdentity {
    /// The administrative directory contained a valid worktree identity.
    Resolved(WorktreeId),
    /// The administrative directory did not yield a valid worktree identity.
    Unavailable,
}

impl RegisteredWorktreeIdentity {
    /// Read the identity once for every consumer of one registry snapshot.
    pub(super) fn observe(candidate: &WorktreeContext) -> Self {
        ledger::read_worktree_identity(candidate.administrative_directory())
            .map_or(Self::Unavailable, Self::Resolved)
    }
}

/// The administrative backlink observed with one registered-worktree snapshot.
#[derive(Clone, Copy)]
pub(super) enum RegisteredWorktreeBacklink {
    /// The administrative backlink names the registered worktree root.
    Matches,
    /// The backlink was absent, unreadable, or named another root.
    Unavailable,
}

impl RegisteredWorktreeBacklink {
    /// Read the backlink once for every consumer of one registry snapshot.
    pub(super) fn observe(candidate: &WorktreeContext) -> Self {
        match backlink_matches(candidate) {
            Ok(true) => Self::Matches,
            Ok(false) | Err(_) => Self::Unavailable,
        }
    }
}

/// The validated location of the same non-recyclable worktree holder.
pub(super) enum ValidatedWorktreeOwner {
    /// The opaque identity remains at the recorded canonical root.
    RecordedRoot,
    /// The opaque identity remains valid after `git worktree move` changed its root.
    Relocated { current_root: CanonicalWorktreeRoot },
}

/// The durable identity and location recorded for one reservation owner.
#[derive(Clone, Copy)]
pub(super) struct RecordedWorktreeOwner<'recorded> {
    pub(super) repository: RepoInstanceId,
    pub(super) worktree:   WorktreeId,
    pub(super) root:       &'recorded CanonicalWorktreeRoot,
    pub(super) locator:    &'recorded WorktreeAdministrativeLocator,
}

/// One registry snapshot's evidence about a possible reservation owner.
#[derive(Clone, Copy)]
pub(super) struct RegisteredWorktreeOwnerObservation<'observed> {
    pub(super) context:  &'observed WorktreeContext,
    pub(super) identity: RegisteredWorktreeIdentity,
    pub(super) backlink: RegisteredWorktreeBacklink,
}

/// Validate repository, administrative directory, backlink, identity, and root together.
pub(super) fn validate_same_owner(
    ledger_repository: RepoInstanceId,
    common_git_directory: &Path,
    recorded: RecordedWorktreeOwner<'_>,
    observed: RegisteredWorktreeOwnerObservation<'_>,
) -> Result<ValidatedWorktreeOwner, LedgerError> {
    if recorded.repository != ledger_repository
        || observed.context.common_git_directory() != common_git_directory
        || observed.context.administrative_locator() != recorded.locator
        || !matches!(
            observed.identity,
            RegisteredWorktreeIdentity::Resolved(candidate_worktree_id)
                if candidate_worktree_id == recorded.worktree
        )
        || !matches!(observed.backlink, RegisteredWorktreeBacklink::Matches)
    {
        return Err(LedgerError::WorktreeIdentityMismatch);
    }
    let current_root = observed
        .context
        .repository_root()
        .to_str()
        .ok_or(LedgerError::NonUtf8AdministrativePath)?
        .parse()
        .map_err(|_| LedgerError::InvalidCanonicalWorktreeRoot)?;
    if &current_root == recorded.root {
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
            let registered_git_file = candidate.repository_root().join(".git");
            if backlink == registered_git_file {
                return Ok(true);
            }
            Ok(fs::canonicalize(backlink)? == fs::canonicalize(registered_git_file)?)
        },
    }
}
