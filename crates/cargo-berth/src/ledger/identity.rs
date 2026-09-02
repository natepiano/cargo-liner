//! The non-recyclable identities stored beside the journal and in each worktree.

use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;

use super::authorization::EditAuthorization;
use super::authorization::ResolvedEditAuthorization;
use super::constants::WORKTREE_ID_FILE_NAME;
use super::error::LedgerError;
use super::journal::JournalReplay;
use super::worktree_context::WorktreeContext;
use crate::ids::RepoInstanceId;
use crate::ids::WorktreeId;
use crate::ids::WorktreeKind;

/// A stored worktree identity paired with its separate worktree role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeIdentity {
    /// The opaque identity issued for this administrative directory instance.
    pub(crate) id: WorktreeId,
    /// Whether this is the main or a linked worktree.
    kind:          WorktreeKind,
}

pub(super) fn read_repo_instance_id(path: &Path) -> Result<RepoInstanceId, LedgerError> {
    fs::read_to_string(path)?
        .trim()
        .parse()
        .map_err(LedgerError::InvalidRepoInstanceId)
}

pub(super) fn validate_journal_repository(
    repo_instance_id: RepoInstanceId,
    replay: &JournalReplay,
) -> Result<(), LedgerError> {
    if replay
        .events
        .iter()
        .any(|event| event.actor.repository != repo_instance_id)
    {
        Err(LedgerError::RepositoryIdentityMismatch)
    } else {
        Ok(())
    }
}

/// Read or create the clone-wide identity stored beside the journal.
pub(super) fn read_or_create_repo_instance_id(path: &Path) -> Result<RepoInstanceId, LedgerError> {
    match fs::read_to_string(path) {
        Ok(identifier) => identifier
            .trim()
            .parse()
            .map_err(LedgerError::InvalidRepoInstanceId),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let repo_instance_id = RepoInstanceId::new();
            let mut identity_file = match OpenOptions::new().write(true).create_new(true).open(path)
            {
                Ok(identity_file) => identity_file,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    return read_or_create_repo_instance_id(path);
                },
                Err(error) => return Err(LedgerError::Io(error)),
            };
            identity_file.write_all(format!("{repo_instance_id}\n").as_bytes())?;
            identity_file.sync_all()?;
            Ok(repo_instance_id)
        },
        Err(error) => Err(LedgerError::Io(error)),
    }
}

/// Create or read a worktree's non-recyclable identity inside its administrative directory.
pub(crate) fn worktree_identity(
    administrative_directory: &Path,
    kind: WorktreeKind,
) -> Result<WorktreeIdentity, LedgerError> {
    Ok(WorktreeIdentity {
        id: create_or_read_worktree_id(administrative_directory)?,
        kind,
    })
}

/// Resolve the worktree and coordination-run ids recorded by a journal mutation.
pub(crate) fn resolve_identity(
    worktree_context: &WorktreeContext,
) -> Result<ResolvedEditAuthorization, LedgerError> {
    let worktree_identity = worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let edit_authorization =
        EditAuthorization::resolve_for_worktree(worktree_context, worktree_identity.id);
    Ok(ResolvedEditAuthorization::for_edit_authorization(
        worktree_identity.id,
        edit_authorization,
    ))
}

/// Create the worktree's identity on first use and read it on every later one.
fn create_or_read_worktree_id(administrative_directory: &Path) -> Result<WorktreeId, LedgerError> {
    let identity_path = administrative_directory.join(WORKTREE_ID_FILE_NAME);
    match fs::read_to_string(&identity_path) {
        Ok(identifier) => identifier
            .trim()
            .parse()
            .map_err(LedgerError::InvalidWorktreeId),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let new_id = WorktreeId::new();
            let mut identity_file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&identity_path)
            {
                Ok(identity_file) => identity_file,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    return create_or_read_worktree_id(administrative_directory);
                },
                Err(error) => return Err(LedgerError::Io(error)),
            };
            identity_file.write_all(format!("{new_id}\n").as_bytes())?;
            identity_file.sync_all()?;
            Ok(new_id)
        },
        Err(error) => Err(LedgerError::Io(error)),
    }
}

/// Read an existing worktree identity without creating a replacement.
pub(crate) fn read_worktree_identity(
    administrative_directory: &Path,
) -> Result<WorktreeId, LedgerError> {
    read_worktree_id(administrative_directory)
}

fn read_worktree_id(administrative_directory: &Path) -> Result<WorktreeId, LedgerError> {
    fs::read_to_string(administrative_directory.join(WORKTREE_ID_FILE_NAME))?
        .trim()
        .parse()
        .map_err(LedgerError::InvalidWorktreeId)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::worktree_identity;
    use crate::ids::WorktreeKind;

    #[test]
    fn recycled_administrative_directory_creates_a_new_worktree_identity() {
        let temporary_directory = tempdir().expect("temporary directory should exist");
        let administrative_directory = temporary_directory.path().join("worktrees").join("phase");
        fs::create_dir_all(&administrative_directory)
            .expect("administrative directory should exist");
        let first_identity = worktree_identity(&administrative_directory, WorktreeKind::Linked)
            .expect("first identity should be created");

        fs::remove_dir_all(&administrative_directory)
            .expect("administrative directory should prune");
        fs::create_dir_all(&administrative_directory)
            .expect("administrative directory should recreate");
        let second_identity = worktree_identity(&administrative_directory, WorktreeKind::Linked)
            .expect("second identity should be created");

        assert_ne!(first_identity.id, second_identity.id);
        assert_eq!(second_identity.kind, WorktreeKind::Linked);
    }
}
