//! Shared-ledger location, initialization, identity storage, and transactions.

mod constants;
mod journal;
mod lock;
mod projection;

use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use constants::JOURNAL_FILE_NAME;
use constants::LEDGER_DIRECTORY_NAME;
use constants::LOCK_FILE_NAME;
use constants::PROJECTION_FILE_NAME;
use constants::REPO_INSTANCE_ID_FILE_NAME;
use constants::WORKTREE_ID_FILE_NAME;
use journal::Journal;
use journal::JournalEvent;
use journal::JournalOperation;
use journal::JournalReplay;
use lock::MutationLock;
use projection::Projection;
use projection::ProjectionRead;
use projection::read_with_retry;

use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::config::InitializationState;
use crate::git;
use crate::ids::CoordinationRunId;
use crate::ids::InvalidUuidV7;
use crate::ids::ProjectionGeneration;
use crate::ids::RepoInstanceId;
use crate::ids::WorktreeId;
use crate::ids::WorktreeKind;

/// The shared append-only ledger for one git common directory.
pub(crate) struct Ledger {
    paths: LedgerPaths,
}

/// The initialized resources reported through the typed `init` result payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LedgerInitialization {
    /// Whether this call created the journal ledger or found it present.
    pub(crate) ledger:        InitializationState,
    /// Whether this call created the repository configuration or retained it.
    pub(crate) configuration: InitializationState,
}

/// A stored worktree identity paired with its separate worktree role.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Worktree reconciliation reads this identity; only its persistent representation exists so far."
    )
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeIdentity {
    /// The opaque identity minted for this administrative directory instance.
    pub(crate) id:   WorktreeId,
    /// Whether this is the main or a linked worktree.
    pub(crate) kind: WorktreeKind,
}

impl Ledger {
    /// Resolve the shared ledger and create its journal, projection, and default config.
    pub(crate) fn initialize(repository_root: &Path) -> Result<LedgerInitialization, LedgerError> {
        let ledger = Self::resolve(repository_root)?;
        let transaction = ledger.begin_mutation()?;
        let configuration = BerthConfig::initialize(repository_root)?;
        // Existing policy must parse before this transaction publishes the ledger.
        BerthConfig::read(repository_root)?;
        transaction.publish(&ledger.paths)?;
        Ok(LedgerInitialization {
            ledger: transaction.journal_initialization,
            configuration,
        })
    }

    /// Append one operation through the required replay-validate-publish transaction.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "No stateful verb invokes this v1 transaction wrapper yet."
        )
    )]
    pub(crate) fn append_operation(
        &self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        operation: JournalOperation,
    ) -> Result<JournalEvent, LedgerError> {
        let mut transaction = self.begin_mutation()?;
        let event = transaction.append(worktree_id, coordination_run_id, operation)?;
        transaction.publish(&self.paths)?;
        Ok(event)
    }

    /// Rebuild the disposable projection from journal truth.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "No stateful verb recovery invokes a projection rebuild yet."
        )
    )]
    pub(crate) fn rebuild_projection(&self) -> Result<(), LedgerError> {
        let transaction = self.begin_mutation()?;
        transaction.publish(&self.paths)
    }

    fn resolve(repository_root: &Path) -> Result<Self, LedgerError> {
        let common_git_directory = git::common_directory(repository_root)?;
        let directory = common_git_directory.join(LEDGER_DIRECTORY_NAME);
        fs::create_dir_all(&directory)?;
        Ok(Self {
            paths: LedgerPaths {
                journal: directory.join(JOURNAL_FILE_NAME),
                lock: directory.join(LOCK_FILE_NAME),
                projection: directory.join(PROJECTION_FILE_NAME),
                repo_instance_id: directory.join(REPO_INSTANCE_ID_FILE_NAME),
                directory,
            },
        })
    }

    fn begin_mutation(&self) -> Result<LedgerTransaction, LedgerError> {
        let lock = MutationLock::acquire(&self.paths.lock)?;
        let repo_instance_id = read_or_mint_repo_instance_id(&self.paths.repo_instance_id)?;
        let (journal, journal_initialization) = Journal::open_or_create(&self.paths.journal)?;
        let replay = journal.replay_repairing_tail()?;
        match read_with_retry(&self.paths.projection, replay.generation)? {
            ProjectionRead::Present(projection) => {
                projection.validate_against(repo_instance_id, &replay)?;
            },
            ProjectionRead::Missing => {},
        }
        Ok(LedgerTransaction {
            _lock: lock,
            journal,
            journal_initialization,
            replay,
            repo_instance_id,
        })
    }
}

impl LedgerTransaction {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "No verb engine appends through this transaction method yet."
        )
    )]
    fn append(
        &mut self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        operation: JournalOperation,
    ) -> Result<JournalEvent, LedgerError> {
        let next_generation = next_projection_generation(self.replay.generation)?;
        let event = JournalEvent::for_operation(
            journal::JournalActor {
                repository: self.repo_instance_id,
                worktree:   worktree_id,
                run:        coordination_run_id,
            },
            next_generation,
            operation,
        );
        self.journal.append(&event)?;
        self.replay = self.journal.replay_repairing_tail()?;
        Ok(event)
    }

    fn publish(&self, paths: &LedgerPaths) -> Result<(), LedgerError> {
        Projection::from_replay(self.repo_instance_id, &self.replay)
            .publish(&paths.directory, &paths.projection)?;
        Ok(())
    }
}

struct LedgerTransaction {
    _lock:                  MutationLock,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "No stateful verb uses the held journal descriptor yet."
        )
    )]
    journal:                Journal,
    journal_initialization: InitializationState,
    replay:                 JournalReplay,
    repo_instance_id:       RepoInstanceId,
}

struct LedgerPaths {
    directory:        PathBuf,
    journal:          PathBuf,
    projection:       PathBuf,
    lock:             PathBuf,
    repo_instance_id: PathBuf,
}

/// Read or mint the clone-wide identity stored beside the journal.
fn read_or_mint_repo_instance_id(path: &Path) -> Result<RepoInstanceId, LedgerError> {
    match fs::read_to_string(path) {
        Ok(identifier) => identifier
            .trim()
            .parse()
            .map_err(LedgerError::InvalidRepoInstanceId),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let repo_instance_id = RepoInstanceId::new();
            let mut identity_file = match OpenOptions::new().write(true).create_new(true).open(path)
            {
                Ok(identity_file) => identity_file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return read_or_mint_repo_instance_id(path);
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

/// Mint or read a worktree's non-recyclable identity inside its administrative directory.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Worktree reconciliation invokes this identity boundary; no verb reaches it yet."
    )
)]
pub(crate) fn worktree_identity(
    administrative_directory: &Path,
    kind: WorktreeKind,
) -> Result<WorktreeIdentity, LedgerError> {
    let identity_path = administrative_directory.join(WORKTREE_ID_FILE_NAME);
    let id = match fs::read_to_string(&identity_path) {
        Ok(identifier) => identifier
            .trim()
            .parse()
            .map_err(LedgerError::InvalidWorktreeId)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let new_id = WorktreeId::new();
            let mut identity_file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&identity_path)
            {
                Ok(identity_file) => identity_file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return worktree_identity(administrative_directory, kind);
                },
                Err(error) => return Err(LedgerError::Io(error)),
            };
            identity_file.write_all(format!("{new_id}\n").as_bytes())?;
            identity_file.sync_all()?;
            new_id
        },
        Err(error) => return Err(LedgerError::Io(error)),
    };
    Ok(WorktreeIdentity { id, kind })
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Every stateful mutation advances this generation through the transaction wrapper; none exists yet."
    )
)]
fn next_projection_generation(
    current_generation: ProjectionGeneration,
) -> Result<ProjectionGeneration, LedgerError> {
    let current_generation: u64 = current_generation.into();
    current_generation
        .checked_add(1)
        .map(ProjectionGeneration::from)
        .ok_or(LedgerError::ProjectionGenerationExhausted)
}

/// A failure that leaves ledger state unreadable or unpublished.
#[derive(Debug)]
pub(crate) enum LedgerError {
    /// Git could not locate the common administrative directory.
    Git(git::GitError),
    /// Ordinary filesystem access failed.
    Io(std::io::Error),
    /// Repository policy could not be initialized or read.
    Config(ConfigError),
    /// The append-only journal could not be replayed safely.
    Journal(journal::JournalError),
    /// The projection cache could not be validated or published.
    Projection(projection::ProjectionError),
    /// The mutation lock could not be acquired.
    MutationLock(lock::MutationLockError),
    /// The stored repository identity is not a UUID-v7 value.
    InvalidRepoInstanceId(InvalidUuidV7),
    /// The stored worktree identity is not a UUID-v7 value.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Worktree reconciliation reaches this validation error after it starts reading stored identities."
        )
    )]
    InvalidWorktreeId(InvalidUuidV7),
    /// The projection counter can no longer advance.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "A stateful mutation constructs this error when the cache counter is exhausted; none reaches it yet."
        )
    )]
    ProjectionGenerationExhausted,
}

impl fmt::Display for LedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(error) => write!(formatter, "could not locate ledger: {error}"),
            Self::Io(error) => write!(formatter, "ledger I/O failed: {error}"),
            Self::Config(error) => write!(formatter, "ledger configuration failed: {error}"),
            Self::Journal(error) => write!(formatter, "journal replay failed: {error}"),
            Self::Projection(error) => write!(formatter, "projection validation failed: {error}"),
            Self::MutationLock(error) => write!(formatter, "ledger mutation lock failed: {error}"),
            Self::InvalidRepoInstanceId(error) => {
                write!(formatter, "invalid stored repository identity: {error}")
            },
            Self::InvalidWorktreeId(error) => {
                write!(formatter, "invalid stored worktree identity: {error}")
            },
            Self::ProjectionGenerationExhausted => {
                formatter.write_str("projection generation counter is exhausted")
            },
        }
    }
}

impl std::error::Error for LedgerError {}

impl From<git::GitError> for LedgerError {
    fn from(error: git::GitError) -> Self { Self::Git(error) }
}

impl From<std::io::Error> for LedgerError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<ConfigError> for LedgerError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<journal::JournalError> for LedgerError {
    fn from(error: journal::JournalError) -> Self { Self::Journal(error) }
}

impl From<projection::ProjectionError> for LedgerError {
    fn from(error: projection::ProjectionError) -> Self { Self::Projection(error) }
}

impl From<lock::MutationLockError> for LedgerError {
    fn from(error: lock::MutationLockError) -> Self { Self::MutationLock(error) }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::process::Command;
    use std::sync::Arc;
    use std::thread;

    use tempfile::tempdir;

    use super::Ledger;
    use super::journal::JournalEvent;
    use super::journal::JournalOperation;
    use super::worktree_identity;
    use crate::ids::CoordinationRunId;
    use crate::ids::RepoInstanceId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ids::WorktreeKind;

    #[test]
    fn recycled_administrative_directory_mints_a_new_worktree_identity() {
        let temporary_directory = tempdir().expect("temporary directory should exist");
        let administrative_directory = temporary_directory.path().join("worktrees").join("phase");
        fs::create_dir_all(&administrative_directory)
            .expect("administrative directory should exist");
        let first_identity = worktree_identity(&administrative_directory, WorktreeKind::Linked)
            .expect("first identity should mint");

        fs::remove_dir_all(&administrative_directory)
            .expect("administrative directory should prune");
        fs::create_dir_all(&administrative_directory)
            .expect("administrative directory should recreate");
        let second_identity = worktree_identity(&administrative_directory, WorktreeKind::Linked)
            .expect("second identity should mint");

        assert_ne!(first_identity.id, second_identity.id);
        assert_eq!(second_identity.kind, WorktreeKind::Linked);
    }

    #[test]
    fn concurrent_mutations_append_without_losing_either_record() {
        let repository = scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Arc::new(Ledger::resolve(repository.path()).expect("ledger should resolve"));
        let first_writer = append_renewal(Arc::clone(&ledger));
        let second_writer = append_renewal(Arc::clone(&ledger));

        assert!(first_writer.join().is_ok_and(|result| result.is_ok()));
        assert!(second_writer.join().is_ok_and(|result| result.is_ok()));
        let journal = fs::read_to_string(&ledger.paths.journal).expect("journal should read");
        assert_eq!(journal.lines().count(), 2);
        let ledger_repo_instance_id = fs::read_to_string(&ledger.paths.repo_instance_id)
            .expect("repository identity should read")
            .trim()
            .parse::<RepoInstanceId>()
            .expect("repository identity should parse");
        for journal_record in journal.lines() {
            let journal_event = serde_json::from_str::<JournalEvent>(journal_record)
                .expect("journal record should deserialize");
            assert_eq!(journal_event.actor.repository, ledger_repo_instance_id);
        }

        let projection_path = ledger.paths.projection.clone();
        let original_projection = fs::read(&projection_path).expect("projection should read");
        fs::remove_file(&projection_path).expect("projection should delete");
        ledger
            .rebuild_projection()
            .expect("projection should rebuild");
        assert_eq!(
            fs::read(projection_path).expect("rebuilt projection should read"),
            original_projection
        );
    }

    fn scratch_repository() -> tempfile::TempDir {
        let repository = tempdir().expect("temporary repository should exist");
        let git_init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repository.path())
            .status()
            .expect("git should initialize a scratch repository");
        assert!(git_init.success());
        repository
    }

    fn append_renewal(ledger: Arc<Ledger>) -> thread::JoinHandle<Result<(), super::LedgerError>> {
        thread::spawn(move || {
            ledger
                .append_operation(
                    WorktreeId::new(),
                    CoordinationRunId::new(),
                    JournalOperation::Renew {
                        reservation_id: ReservationId::new(),
                    },
                )
                .map(|_| ())
        })
    }
}
