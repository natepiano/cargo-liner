//! Filesystem discovery of a worktree's repository, administrative, and ledger paths.

use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use uuid::Uuid;

use super::constants::COORDINATION_RUN_MARKER_FILE_NAME;
use super::constants::COORDINATION_RUN_MARKER_RETIREMENT_SUFFIX;
use super::constants::LEDGER_DIRECTORY_NAME;
use super::coordination_run_marker::CoordinationRunMarkerAtRetirement;
use super::coordination_run_marker::CoordinationRunMarkerRemoval;
use super::coordination_run_marker::DetachedCoordinationRunMarker;
use super::error::LedgerError;
use super::journal::WorktreeAdministrativeLocator;
use crate::ids::CoordinationRunId;
use crate::ids::WorktreeKind;

/// Repository and administrative paths discovered without executing git.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeContext {
    repository_root:          PathBuf,
    administrative_directory: WorktreeAdministrativeDirectory,
    common_git_directory:     PathBuf,
    shared_ledger_directory:  SharedLedgerDirectory,
    administrative_locator:   WorktreeAdministrativeLocator,
    worktree_kind:            WorktreeKind,
}

/// The per-worktree Git directory that owns worktree and run identity markers.
#[derive(Clone, Debug, Eq, PartialEq)]
struct WorktreeAdministrativeDirectory(PathBuf);

/// The common cargo-berth directory that owns the journal and session mappings.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SharedLedgerDirectory(PathBuf);

/// The relationship between a `.git` file target and any shared git directory.
enum GitAdministrativeLayout {
    /// The target is the complete administrative directory for a main worktree.
    Main,
    /// The target belongs to a linked worktree and names its shared directory.
    Linked { common_git_directory: PathBuf },
}

/// Whether a root from Git's registry has readable administrative metadata.
pub(crate) enum RegisteredWorktreeAvailability {
    /// The root has a validated administrative layout for this repository.
    Readable(WorktreeContext),
    /// The root disappeared, became unreadable, or now names another repository.
    Unavailable,
}

impl WorktreeContext {
    /// Discover the containing worktree using only `.git` filesystem metadata.
    pub(crate) fn discover(invocation_directory: &Path) -> Result<Self, LedgerError> {
        let invocation_directory = fs::canonicalize(invocation_directory)?;
        for repository_root in invocation_directory.ancestors() {
            let dot_git = repository_root.join(".git");
            if dot_git.is_dir() {
                let common_git_directory = fs::canonicalize(&dot_git)?;
                return Self::build(
                    repository_root,
                    common_git_directory.clone(),
                    common_git_directory,
                    WorktreeKind::Main,
                );
            }
            if dot_git.is_file() {
                let worktree_administrative_directory = read_git_directory_file(&dot_git)?;
                return match read_git_administrative_layout(&worktree_administrative_directory)? {
                    GitAdministrativeLayout::Main => Self::build(
                        repository_root,
                        worktree_administrative_directory.clone(),
                        worktree_administrative_directory,
                        WorktreeKind::Main,
                    ),
                    GitAdministrativeLayout::Linked {
                        common_git_directory,
                    } => Self::build(
                        repository_root,
                        worktree_administrative_directory,
                        common_git_directory,
                        WorktreeKind::Linked,
                    ),
                };
            }
        }
        Err(LedgerError::RepositoryNotFound)
    }

    /// Validate one root already returned by Git's worktree registry.
    pub(crate) fn from_registered_root(
        repository_root: &Path,
        common_git_directory: &Path,
    ) -> Result<RegisteredWorktreeAvailability, LedgerError> {
        let Ok(repository_root) = fs::canonicalize(repository_root) else {
            return Ok(RegisteredWorktreeAvailability::Unavailable);
        };
        let dot_git = repository_root.join(".git");
        if dot_git.is_dir() {
            let Ok(registered_git_directory) = fs::canonicalize(&dot_git) else {
                return Ok(RegisteredWorktreeAvailability::Unavailable);
            };
            if registered_git_directory != common_git_directory {
                return Ok(RegisteredWorktreeAvailability::Unavailable);
            }
            return Self::build_registered(
                &repository_root,
                registered_git_directory.clone(),
                registered_git_directory,
                WorktreeKind::Main,
            )
            .map(RegisteredWorktreeAvailability::Readable);
        }
        if dot_git.is_file() {
            let Ok(administrative_directory) = read_git_directory_file(&dot_git) else {
                return Ok(RegisteredWorktreeAvailability::Unavailable);
            };
            let Ok(administrative_layout) =
                read_git_administrative_layout(&administrative_directory)
            else {
                return Ok(RegisteredWorktreeAvailability::Unavailable);
            };
            return match administrative_layout {
                GitAdministrativeLayout::Main
                    if administrative_directory == common_git_directory =>
                {
                    Self::build_registered(
                        &repository_root,
                        administrative_directory.clone(),
                        administrative_directory,
                        WorktreeKind::Main,
                    )
                    .map(RegisteredWorktreeAvailability::Readable)
                },
                GitAdministrativeLayout::Linked {
                    common_git_directory: registered_common_git_directory,
                } if registered_common_git_directory == common_git_directory => {
                    Self::build_registered(
                        &repository_root,
                        administrative_directory,
                        registered_common_git_directory,
                        WorktreeKind::Linked,
                    )
                    .map(RegisteredWorktreeAvailability::Readable)
                },
                GitAdministrativeLayout::Main | GitAdministrativeLayout::Linked { .. } => {
                    Ok(RegisteredWorktreeAvailability::Unavailable)
                },
            };
        }
        Ok(RegisteredWorktreeAvailability::Unavailable)
    }

    fn build(
        repository_root: &Path,
        worktree_administrative_directory: PathBuf,
        common_git_directory: PathBuf,
        worktree_kind: WorktreeKind,
    ) -> Result<Self, LedgerError> {
        let repository_root = fs::canonicalize(repository_root)?;
        let locator = match worktree_kind {
            WorktreeKind::Main => ".".to_owned(),
            WorktreeKind::Linked => worktree_administrative_directory
                .strip_prefix(&common_git_directory)
                .map_err(|_| LedgerError::AdministrativeDirectoryOutsideCommonGitDirectory)?
                .to_str()
                .ok_or(LedgerError::NonUtf8AdministrativePath)?
                .to_owned(),
        };
        let administrative_locator = WorktreeAdministrativeLocator::from_str(&locator)
            .map_err(|_| LedgerError::InvalidAdministrativeLocator(locator))?;
        let shared_ledger_directory =
            SharedLedgerDirectory(common_git_directory.join(LEDGER_DIRECTORY_NAME));
        Ok(Self {
            repository_root,
            administrative_directory: WorktreeAdministrativeDirectory(
                worktree_administrative_directory,
            ),
            common_git_directory,
            shared_ledger_directory,
            administrative_locator,
            worktree_kind,
        })
    }

    fn build_registered(
        repository_root: &Path,
        worktree_administrative_directory: PathBuf,
        common_git_directory: PathBuf,
        worktree_kind: WorktreeKind,
    ) -> Result<Self, LedgerError> {
        let locator = match worktree_kind {
            WorktreeKind::Main => ".".to_owned(),
            WorktreeKind::Linked => worktree_administrative_directory
                .strip_prefix(&common_git_directory)
                .map_err(|_| LedgerError::AdministrativeDirectoryOutsideCommonGitDirectory)?
                .to_str()
                .ok_or(LedgerError::NonUtf8AdministrativePath)?
                .to_owned(),
        };
        let administrative_locator = WorktreeAdministrativeLocator::from_str(&locator)
            .map_err(|_| LedgerError::InvalidAdministrativeLocator(locator))?;
        let shared_ledger_directory =
            SharedLedgerDirectory(common_git_directory.join(LEDGER_DIRECTORY_NAME));
        Ok(Self {
            repository_root: repository_root.to_path_buf(),
            administrative_directory: WorktreeAdministrativeDirectory(
                worktree_administrative_directory,
            ),
            common_git_directory,
            shared_ledger_directory,
            administrative_locator,
            worktree_kind,
        })
    }

    /// Return the canonical repository worktree root.
    pub(crate) fn repository_root(&self) -> &Path { &self.repository_root }

    /// Return the administrative directory for this worktree.
    pub(crate) fn administrative_directory(&self) -> &Path { &self.administrative_directory.0 }

    /// Return the common git directory shared by every linked worktree.
    pub(crate) fn common_git_directory(&self) -> &Path { &self.common_git_directory }

    /// Return the shared cargo-berth ledger directory.
    pub(crate) fn ledger_directory(&self) -> PathBuf { self.shared_ledger_directory.0.clone() }

    /// Return the common-directory-relative worktree administrative locator.
    pub(crate) const fn administrative_locator(&self) -> &WorktreeAdministrativeLocator {
        &self.administrative_locator
    }

    /// Return whether this is the main or a linked worktree.
    pub(crate) const fn worktree_kind(&self) -> WorktreeKind { self.worktree_kind }

    /// Atomically publish the successful claimant's coordination-run marker.
    pub(crate) fn publish_coordination_run_marker(
        &self,
        coordination_run_id: CoordinationRunId,
    ) -> Result<(), LedgerError> {
        let marker_path = self
            .administrative_directory
            .0
            .join(COORDINATION_RUN_MARKER_FILE_NAME);
        let publication_attempt_id = Uuid::now_v7();
        let temporary_path = self.administrative_directory.0.join(format!(
            "{COORDINATION_RUN_MARKER_FILE_NAME}.{coordination_run_id}.{publication_attempt_id}.tmp"
        ));
        let publication = (|| -> Result<(), std::io::Error> {
            let mut temporary_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)?;
            temporary_file.write_all(format!("{coordination_run_id}\n").as_bytes())?;
            temporary_file.sync_all()?;
            fs::rename(&temporary_path, marker_path)?;
            fs::File::open(&self.administrative_directory.0)?.sync_all()?;
            Ok(())
        })();
        if publication.is_err() {
            std::mem::drop(fs::remove_file(temporary_path));
        }
        publication.map_err(LedgerError::Io)
    }

    /// Remove the marker only when it still names the released run.
    pub(crate) fn remove_coordination_run_marker(
        &self,
        released_run_id: CoordinationRunId,
    ) -> Result<CoordinationRunMarkerRemoval, LedgerError> {
        match self.detach_coordination_run_marker()? {
            CoordinationRunMarkerAtRetirement::AlreadyAbsent => {
                Ok(CoordinationRunMarkerRemoval::AlreadyAbsent)
            },
            CoordinationRunMarkerAtRetirement::Detached(detached_marker) => {
                detached_marker.retire(released_run_id)
            },
        }
    }

    /// Remove a malformed or inactive marker while preserving an active run's marker.
    pub(crate) fn sweep_coordination_run_marker(
        &self,
        active_run_matches: impl Fn(CoordinationRunId) -> bool,
    ) -> Result<(), LedgerError> {
        let marker_path = self
            .administrative_directory
            .0
            .join(COORDINATION_RUN_MARKER_FILE_NAME);
        if fs::read_to_string(marker_path)
            .ok()
            .and_then(|marker| marker.trim().parse::<CoordinationRunId>().ok())
            .is_some_and(&active_run_matches)
        {
            return Ok(());
        }
        match self.detach_coordination_run_marker()? {
            CoordinationRunMarkerAtRetirement::AlreadyAbsent => Ok(()),
            CoordinationRunMarkerAtRetirement::Detached(detached_marker) => {
                detached_marker.sweep(active_run_matches)
            },
        }
    }

    fn detach_coordination_run_marker(
        &self,
    ) -> Result<CoordinationRunMarkerAtRetirement, LedgerError> {
        let marker_path = self
            .administrative_directory
            .0
            .join(COORDINATION_RUN_MARKER_FILE_NAME);
        let retirement_path = self.administrative_directory.0.join(format!(
            "{COORDINATION_RUN_MARKER_FILE_NAME}.{}.{COORDINATION_RUN_MARKER_RETIREMENT_SUFFIX}",
            Uuid::now_v7()
        ));
        match fs::rename(&marker_path, &retirement_path) {
            Ok(()) => Ok(CoordinationRunMarkerAtRetirement::Detached(
                DetachedCoordinationRunMarker {
                    administrative_directory: self.administrative_directory.0.clone(),
                    marker_path,
                    retirement_path,
                },
            )),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Ok(CoordinationRunMarkerAtRetirement::AlreadyAbsent)
            },
            Err(error) => Err(LedgerError::Io(error)),
        }
    }
}

fn read_git_directory_file(dot_git_path: &Path) -> Result<PathBuf, LedgerError> {
    let contents = fs::read_to_string(dot_git_path)?;
    let locator = contents
        .trim()
        .strip_prefix("gitdir:")
        .map(str::trim)
        .filter(|locator| !locator.is_empty())
        .ok_or(LedgerError::InvalidGitDirectoryFile)?;
    let locator = PathBuf::from(locator);
    let repository_root = dot_git_path
        .parent()
        .ok_or(LedgerError::InvalidGitDirectoryFile)?;
    let administrative_directory = if locator.is_absolute() {
        locator
    } else {
        repository_root.join(locator)
    };
    fs::canonicalize(administrative_directory).map_err(LedgerError::Io)
}

fn read_git_administrative_layout(
    worktree_administrative_directory: &Path,
) -> Result<GitAdministrativeLayout, LedgerError> {
    let contents = match fs::read_to_string(worktree_administrative_directory.join("commondir")) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(GitAdministrativeLayout::Main);
        },
        Err(error) => return Err(LedgerError::Io(error)),
    };
    let locator = contents.trim();
    if locator.is_empty() {
        return Err(LedgerError::InvalidCommonDirectoryFile);
    }
    fs::canonicalize(worktree_administrative_directory.join(locator))
        .map(|common_git_directory| GitAdministrativeLayout::Linked {
            common_git_directory,
        })
        .map_err(LedgerError::Io)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::COORDINATION_RUN_MARKER_FILE_NAME;
    use super::CoordinationRunMarkerAtRetirement;
    use super::CoordinationRunMarkerRemoval;
    use super::WorktreeContext;
    use crate::ids::CoordinationRunId;
    use crate::ids::WorktreeKind;
    use crate::ledger::test_support::scratch_repository;

    #[test]
    fn git_file_without_common_directory_is_a_main_worktree() {
        let temporary_directory = tempdir().expect("temporary directory should exist");
        let repository_root = temporary_directory.path().join("worktree");
        let administrative_directory = temporary_directory.path().join("external-git");
        fs::create_dir(&repository_root).expect("worktree directory should exist");
        fs::create_dir(&administrative_directory).expect("administrative directory should exist");
        fs::write(
            repository_root.join(".git"),
            format!("gitdir: {}\n", administrative_directory.display()),
        )
        .expect("git directory file should write");

        let worktree_context =
            WorktreeContext::discover(&repository_root).expect("worktree should be discovered");
        let canonical_administrative_directory = fs::canonicalize(&administrative_directory)
            .expect("administrative directory should canonicalize");

        assert_eq!(worktree_context.worktree_kind(), WorktreeKind::Main);
        assert_eq!(
            worktree_context.administrative_directory(),
            canonical_administrative_directory
        );
        assert_eq!(
            worktree_context.common_git_directory(),
            canonical_administrative_directory
        );
    }

    #[test]
    fn detached_marker_retirement_preserves_a_concurrent_publication()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = scratch_repository();
        let worktree_context =
            WorktreeContext::discover(repository.path()).expect("worktree should be discovered");
        let released_run_id = CoordinationRunId::new();
        let replacement_run_id = CoordinationRunId::new();
        worktree_context
            .publish_coordination_run_marker(released_run_id)
            .expect("released run marker should publish");
        let marker_at_retirement = worktree_context
            .detach_coordination_run_marker()
            .expect("marker should detach");
        let detached_marker = match marker_at_retirement {
            CoordinationRunMarkerAtRetirement::Detached(detached_marker) => detached_marker,
            CoordinationRunMarkerAtRetirement::AlreadyAbsent => {
                return Err(std::io::Error::other(
                    "published marker must be present for retirement",
                )
                .into());
            },
        };
        worktree_context
            .publish_coordination_run_marker(replacement_run_id)
            .expect("replacement run marker should publish");

        let removal = detached_marker
            .retire(released_run_id)
            .expect("detached marker should retire");

        assert_eq!(removal, CoordinationRunMarkerRemoval::Removed);
        assert_eq!(
            fs::read_to_string(
                worktree_context
                    .administrative_directory()
                    .join(COORDINATION_RUN_MARKER_FILE_NAME)
            )
            .expect("replacement marker should remain")
            .trim(),
            replacement_run_id.to_string()
        );
        Ok(())
    }
}
