//! Shared-ledger location, initialization, identity storage, and transactions.

mod constants;
mod journal;
mod lock;
mod projection;

use std::ffi::OsString;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use constants::COORDINATION_RUN_ENVIRONMENT;
use constants::COORDINATION_RUN_MARKER_FILE_NAME;
use constants::COORDINATION_RUN_MARKER_RETIREMENT_SUFFIX;
use constants::JOURNAL_FILE_NAME;
use constants::LEDGER_DIRECTORY_NAME;
use constants::LOCK_FILE_NAME;
use constants::MAXIMUM_JOURNAL_RECORD_BYTES;
use constants::MUTATING_VERB_CONTENTION_TOLERANCE;
use constants::PROJECTION_FILE_NAME;
use constants::REPO_INSTANCE_ID_FILE_NAME;
use constants::WORKTREE_ID_FILE_NAME;
pub(crate) use journal::BypassCause;
pub(crate) use journal::BypassedAction;
pub(crate) use journal::CanonicalWorktreeRoot;
pub(crate) use journal::ClaimHeadCommit;
pub(crate) use journal::ClaimHeadSnapshot;
pub(crate) use journal::ClaimSource;
pub(crate) use journal::CollisionPathSet;
pub(crate) use journal::ForcedIntegrationReason;
pub(crate) use journal::ForeignReservationIdSet;
pub(crate) use journal::FullRefName;
pub(crate) use journal::IncursionPathSet;
use journal::Journal;
pub(crate) use journal::JournalActor;
use journal::JournalAppendError;
use journal::JournalError;
pub(crate) use journal::JournalEvent;
pub(crate) use journal::JournalOperation;
use journal::JournalReplay;
pub(crate) use journal::NonEmptyReservationPurpose;
pub(crate) use journal::OrderingDirection;
pub(crate) use journal::ProtectedPhaseStartHead;
pub(crate) use journal::ReservationPurpose;
pub(crate) use journal::ReservationScope;
pub(crate) use journal::ReservationScopeAdditionSet;
pub(crate) use journal::ReservationScopeSet;
pub(crate) use journal::ReservationSnapshot;
pub(crate) use journal::ScopeKind;
pub(crate) use journal::SkippedDeferral;
pub(crate) use journal::SkippedIntegrationHoldSet;
pub(crate) use journal::SkippedOrderingEdge;
pub(crate) use journal::TrunkCommitAtClaim;
pub(crate) use journal::WidenCause;
pub(crate) use journal::WorkPlanReference;
pub(crate) use journal::WorktreeAdministrativeLocator;
use lock::MutationLock;
use lock::MutationLockError;
use projection::Projection;
use projection::ProjectionError;
use projection::ProjectionSynchronization;
use projection::read_validated;
use uuid::Uuid;

use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::config::InitializationState;
use crate::git;
use crate::git::GitError;
use crate::ids::CoordinationRunId;
use crate::ids::InvalidUuidV7;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RepoInstanceId;
use crate::ids::WorktreeId;
use crate::ids::WorktreeKind;

/// The shared append-only ledger for one git common directory.
pub(crate) struct Ledger {
    paths: LedgerPaths,
}

/// Repository and administrative paths discovered without executing git.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeContext {
    repository_root:                   PathBuf,
    worktree_administrative_directory: PathBuf,
    common_git_directory:              PathBuf,
    administrative_locator:            WorktreeAdministrativeLocator,
    worktree_kind:                     WorktreeKind,
}

/// The relationship between a `.git` file target and any shared git directory.
enum GitAdministrativeLayout {
    /// The target is the complete administrative directory for a main worktree.
    Main,
    /// The target belongs to a linked worktree and names its shared directory.
    Linked { common_git_directory: PathBuf },
}

/// A coordination-run marker atomically detached for content-based retirement.
struct DetachedCoordinationRunMarker {
    administrative_directory: PathBuf,
    marker_path:              PathBuf,
    retirement_path:          PathBuf,
}

/// Whether a marker was present when retirement atomically detached its pathname.
enum CoordinationRunMarkerAtRetirement {
    /// No marker existed at the retirement point.
    AlreadyAbsent,
    /// The exact marker present at the retirement point has a private pathname.
    Detached(DetachedCoordinationRunMarker),
}

/// The content-based decision for one atomically detached marker.
enum DetachedCoordinationRunMarkerDisposition {
    /// The detached marker names the released run.
    Remove,
    /// The detached marker names another run.
    PreserveDifferentRun,
    /// The detached marker does not contain a UUID-v7 run id.
    PreserveMalformed,
}

/// Validated journal truth for a mutation-free edit check.
pub(crate) struct EditCheckLedgerSnapshot {
    events:           Vec<JournalEvent>,
    worktree_context: WorktreeContext,
}

/// The initialized resources reported through the typed `init` result payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LedgerInitialization {
    /// Whether this call created the journal ledger or found it present.
    pub(crate) ledger:        InitializationState,
    /// Whether this call created the repository configuration or retained it.
    pub(crate) configuration: InitializationState,
}

/// The exact journal material discarded by confirmed reinitialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LedgerReinitialization {
    /// The number of bytes removed from `journal.ndjson`.
    pub(crate) discarded_bytes:            u64,
    /// The number of newline-terminated records that were present, valid or corrupt.
    pub(crate) discarded_complete_records: u64,
}

/// The coordination identity an edit check can prove for its current process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditAuthorization {
    /// The process environment explicitly supplied the coordination run.
    Environment(CoordinationRunId),
    /// The worktree marker supplied a run paired with its minted worktree identity.
    Marker {
        /// The run named by the marker.
        coordination_run_id: CoordinationRunId,
        /// The opaque identity from the same administrative directory.
        worktree_id:         WorktreeId,
    },
    /// The caller has no run identity and must not receive a same-worktree exemption.
    Unidentified,
}

/// The filesystem result of retiring one coordination-run marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoordinationRunMarkerRemoval {
    /// The marker named the released run and was removed.
    Removed,
    /// No marker existed when release checked it.
    AlreadyAbsent,
    /// The marker named another run and remains untouched.
    PreservedDifferentRun,
    /// The marker was not a UUID-v7 run id and remains for reconciliation.
    PreservedMalformed,
}

impl EditAuthorization {
    /// Resolve the active run from the environment, then the worktree marker.
    pub(crate) fn resolve(worktree_administrative_directory: &Path) -> Self {
        Self::resolve_from_environment(
            std::env::var_os(COORDINATION_RUN_ENVIRONMENT),
            worktree_administrative_directory,
        )
    }

    fn resolve_from_environment(
        environment_run: Option<OsString>,
        worktree_administrative_directory: &Path,
    ) -> Self {
        environment_run.map_or_else(
            || {
                let marker_path =
                    worktree_administrative_directory.join(COORDINATION_RUN_MARKER_FILE_NAME);
                fs::read_to_string(marker_path)
                    .ok()
                    .and_then(|value| value.trim().parse().ok())
                    .and_then(|coordination_run_id| {
                        read_worktree_id(worktree_administrative_directory)
                            .ok()
                            .map(|worktree_id| Self::Marker {
                                coordination_run_id,
                                worktree_id,
                            })
                    })
                    .unwrap_or(Self::Unidentified)
            },
            |value| {
                value
                    .into_string()
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .map_or(Self::Unidentified, Self::Environment)
            },
        )
    }
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
        Ok(Self {
            repository_root,
            worktree_administrative_directory,
            common_git_directory,
            administrative_locator,
            worktree_kind,
        })
    }

    /// Return the canonical repository worktree root.
    pub(crate) fn repository_root(&self) -> &Path { &self.repository_root }

    /// Return the administrative directory for this worktree.
    pub(crate) fn administrative_directory(&self) -> &Path {
        &self.worktree_administrative_directory
    }

    /// Return the common git directory shared by every linked worktree.
    pub(crate) fn common_git_directory(&self) -> &Path { &self.common_git_directory }

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
            .worktree_administrative_directory
            .join(COORDINATION_RUN_MARKER_FILE_NAME);
        let publication_attempt_id = Uuid::now_v7();
        let temporary_path = self.worktree_administrative_directory.join(format!(
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
            fs::File::open(&self.worktree_administrative_directory)?.sync_all()?;
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
        active_run_matches: impl FnOnce(CoordinationRunId) -> bool,
    ) -> Result<(), LedgerError> {
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
            .worktree_administrative_directory
            .join(COORDINATION_RUN_MARKER_FILE_NAME);
        let retirement_path = self.worktree_administrative_directory.join(format!(
            "{COORDINATION_RUN_MARKER_FILE_NAME}.{}.{COORDINATION_RUN_MARKER_RETIREMENT_SUFFIX}",
            Uuid::now_v7()
        ));
        match fs::rename(&marker_path, &retirement_path) {
            Ok(()) => Ok(CoordinationRunMarkerAtRetirement::Detached(
                DetachedCoordinationRunMarker {
                    administrative_directory: self.worktree_administrative_directory.clone(),
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

impl DetachedCoordinationRunMarker {
    fn retire(
        self,
        released_run_id: CoordinationRunId,
    ) -> Result<CoordinationRunMarkerRemoval, LedgerError> {
        let marker = match fs::read_to_string(&self.retirement_path) {
            Ok(marker) => marker,
            Err(error) => {
                self.restore()?;
                return Err(LedgerError::Io(error));
            },
        };
        let disposition = marker.trim().parse::<CoordinationRunId>().map_or(
            DetachedCoordinationRunMarkerDisposition::PreserveMalformed,
            |marker_run_id| {
                if marker_run_id == released_run_id {
                    DetachedCoordinationRunMarkerDisposition::Remove
                } else {
                    DetachedCoordinationRunMarkerDisposition::PreserveDifferentRun
                }
            },
        );
        match disposition {
            DetachedCoordinationRunMarkerDisposition::Remove => {
                self.remove()?;
                Ok(CoordinationRunMarkerRemoval::Removed)
            },
            DetachedCoordinationRunMarkerDisposition::PreserveDifferentRun => {
                self.restore()?;
                Ok(CoordinationRunMarkerRemoval::PreservedDifferentRun)
            },
            DetachedCoordinationRunMarkerDisposition::PreserveMalformed => {
                self.restore()?;
                Ok(CoordinationRunMarkerRemoval::PreservedMalformed)
            },
        }
    }

    fn remove(&self) -> Result<(), LedgerError> {
        if fs::metadata(&self.retirement_path)?.is_dir() {
            fs::remove_dir(&self.retirement_path)?;
        } else {
            fs::remove_file(&self.retirement_path)?;
        }
        fs::File::open(&self.administrative_directory)?.sync_all()?;
        Ok(())
    }

    fn sweep(
        self,
        active_run_matches: impl FnOnce(CoordinationRunId) -> bool,
    ) -> Result<(), LedgerError> {
        let retirement_metadata = match fs::metadata(&self.retirement_path) {
            Ok(retirement_metadata) => retirement_metadata,
            Err(error) => {
                self.restore()?;
                return Err(LedgerError::Io(error));
            },
        };
        if retirement_metadata.is_dir() {
            return self.remove();
        }
        let marker = match fs::read_to_string(&self.retirement_path) {
            Ok(marker) => marker,
            Err(error) => {
                self.restore()?;
                return Err(LedgerError::Io(error));
            },
        };
        match marker.trim().parse::<CoordinationRunId>() {
            Ok(coordination_run_id) if active_run_matches(coordination_run_id) => self.restore(),
            Ok(_) | Err(_) => self.remove(),
        }
    }

    fn restore(&self) -> Result<(), LedgerError> {
        match fs::hard_link(&self.retirement_path, &self.marker_path) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {},
            Err(error) => return Err(LedgerError::Io(error)),
        }
        self.remove()
    }
}

impl EditCheckLedgerSnapshot {
    /// Borrow every complete journal fact visible to this read.
    pub(crate) fn events(&self) -> &[JournalEvent] { &self.events }

    /// Return the filesystem-discovered worktree context.
    pub(crate) const fn worktree_context(&self) -> &WorktreeContext { &self.worktree_context }
}

/// The replayed journal facts visible to a transaction's validation step.
pub(crate) struct ReplayedLedgerState<'replay> {
    events:             &'replay [JournalEvent],
    generation:         ProjectionGeneration,
    journal_end_offset: JournalByteOffset,
}

impl ReplayedLedgerState<'_> {
    /// Borrow every replayed fact in append order.
    pub(crate) const fn events(&self) -> &[JournalEvent] { self.events }

    /// Return the projection generation represented by the replay.
    pub(crate) const fn generation(&self) -> ProjectionGeneration { self.generation }

    /// Return the journal byte offset represented by the replay.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "The claim engine inspects the replay point; no verb reaches it yet."
        )
    )]
    const fn journal_end_offset(&self) -> JournalByteOffset { self.journal_end_offset }
}

/// The only two outcomes a transaction validator can authorize.
pub(crate) enum TransactionValidation<Rejection> {
    /// Append this operation and publish the resulting projection.
    Append(Box<JournalOperation>),
    /// Return the semantic rejection without changing durable state.
    Reject(Rejection),
}

/// A locked validation result that carries work permitted only after its append commits.
pub(crate) enum CommittedActionValidation<Rejection, CommittedAction> {
    /// Append this operation, then execute the action while retaining the mutation lock.
    Append {
        /// The journal operation that must commit first.
        operation: Box<JournalOperation>,
        /// The side effect authorized by the committed operation.
        action:    CommittedAction,
    },
    /// Return the semantic rejection without changing durable state.
    Reject(Rejection),
}

/// A locked reconciliation decision that may append several replayable conclusions.
pub(crate) enum ReconciliationValidation<Rejection, CommittedAction> {
    /// Append each operation, then execute the repair action under the same lock.
    Apply {
        /// Journal operations computed from the locked replay.
        operations: Vec<JournalOperation>,
        /// Idempotent filesystem and git repairs authorized after the appends.
        action:     CommittedAction,
    },
    /// Stop without changing journal or side-effect state.
    Reject(Rejection),
}

/// The durable result of a validation-controlled ledger transaction.
pub(crate) enum LedgerTransactionOutcome<Rejection> {
    /// Exactly one approved event was appended and published.
    Appended(Box<JournalEvent>),
    /// Validation rejected the proposal before any append.
    Rejected(Rejection),
}

/// The result of a transaction whose appended record authorizes a locked side effect.
pub(crate) enum LedgerCommittedActionOutcome<Rejection, CommittedActionOutput> {
    /// The event committed, its action ran under the lock, and the projection published.
    Appended(CommittedActionOutput),
    /// Validation rejected the proposal before any append or side effect.
    Rejected(Rejection),
}

/// A stored worktree identity paired with its separate worktree role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeIdentity {
    /// The opaque identity minted for this administrative directory instance.
    pub(crate) id: WorktreeId,
    /// Whether this is the main or a linked worktree.
    kind:          WorktreeKind,
}

impl Ledger {
    /// Resolve the shared ledger and create its journal, projection, and default config.
    pub(crate) fn initialize(repository_root: &Path) -> Result<LedgerInitialization, LedgerError> {
        let ledger = Self::locate(repository_root)?;
        fs::create_dir_all(&ledger.paths.directory)?;
        let transaction = ledger.begin_initialization()?;
        let configuration = BerthConfig::initialize(repository_root)?;
        // Existing policy must parse before this transaction publishes the ledger.
        BerthConfig::read(repository_root)?;
        transaction.publish(&ledger.paths)?;
        Ok(LedgerInitialization {
            ledger: transaction.journal_initialization,
            configuration,
        })
    }

    /// Attach to an initialized ledger without creating any missing state.
    pub(crate) fn open(invocation_directory: &Path) -> Result<Self, LedgerError> {
        let repository_root = git::repository_root(invocation_directory)?;
        let ledger = Self::locate(&repository_root)?;
        ledger.require_existing()?;
        Ok(ledger)
    }

    /// Read the clone identity that owns this ledger.
    pub(crate) fn repository_identity(&self) -> Result<RepoInstanceId, LedgerError> {
        read_repo_instance_id(&self.paths.repo_instance_id)
    }

    /// Read validated journal truth without git, locking, repair, or publication.
    pub(crate) fn read_for_edit_check(
        invocation_directory: &Path,
    ) -> Result<EditCheckLedgerSnapshot, LedgerError> {
        let worktree_context = WorktreeContext::discover(invocation_directory)?;
        let ledger = Self::at_common_git_directory(worktree_context.common_git_directory());
        ledger.require_existing()?;
        let repo_instance_id = read_repo_instance_id(&ledger.paths.repo_instance_id)?;
        let replay = Journal::replay_read_only(&ledger.paths.journal)?;
        validate_journal_repository(repo_instance_id, &replay)?;
        read_validated(&ledger.paths.projection, repo_instance_id, &replay)?;
        Ok(EditCheckLedgerSnapshot {
            events: replay.events,
            worktree_context,
        })
    }

    /// Validate against one locked replay and append only the approved operation.
    pub(crate) fn transact<Rejection>(
        &self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        validate: impl FnOnce(ReplayedLedgerState<'_>) -> TransactionValidation<Rejection>,
    ) -> Result<LedgerTransactionOutcome<Rejection>, LedgerTransactionError> {
        let mut transaction = self
            .begin_mutation()
            .map_err(LedgerTransactionError::from_ledger_error)?;
        let replayed_state = ReplayedLedgerState {
            events:             &transaction.replay.events,
            generation:         transaction.replay.generation,
            journal_end_offset: transaction.replay.end_offset,
        };
        match validate(replayed_state) {
            TransactionValidation::Append(operation) => {
                let event = transaction.append(worktree_id, coordination_run_id, *operation)?;
                transaction
                    .publish(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)?;
                Ok(LedgerTransactionOutcome::Appended(Box::new(event)))
            },
            TransactionValidation::Reject(rejection) => {
                transaction
                    .publish_if_rebuild_required(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)?;
                Ok(LedgerTransactionOutcome::Rejected(rejection))
            },
        }
    }

    /// Attempt one bypass-audit append without ever waiting behind a live lock holder.
    pub(crate) fn try_transact<Rejection>(
        &self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        validate: impl FnOnce(ReplayedLedgerState<'_>) -> TransactionValidation<Rejection>,
    ) -> Result<LedgerTransactionOutcome<Rejection>, LedgerTransactionError> {
        let mut transaction = self
            .begin_mutation_with_tolerance(Duration::ZERO)
            .map_err(LedgerTransactionError::from_ledger_error)?;
        let replayed_state = ReplayedLedgerState {
            events:             &transaction.replay.events,
            generation:         transaction.replay.generation,
            journal_end_offset: transaction.replay.end_offset,
        };
        match validate(replayed_state) {
            TransactionValidation::Append(operation) => {
                let event = transaction.append(worktree_id, coordination_run_id, *operation)?;
                transaction
                    .publish(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)?;
                Ok(LedgerTransactionOutcome::Appended(Box::new(event)))
            },
            TransactionValidation::Reject(rejection) => {
                transaction
                    .publish_if_rebuild_required(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)?;
                Ok(LedgerTransactionOutcome::Rejected(rejection))
            },
        }
    }

    /// Append a validated operation before executing its authorized action under the same lock.
    pub(crate) fn transact_with_committed_action<
        Rejection,
        CommittedAction,
        CommittedActionOutput,
        CommittedActionError,
    >(
        &self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        validate: impl FnOnce(
            ReplayedLedgerState<'_>,
        ) -> CommittedActionValidation<Rejection, CommittedAction>,
        commit_action: impl FnOnce(
            CommittedAction,
        ) -> Result<CommittedActionOutput, CommittedActionError>,
    ) -> Result<
        LedgerCommittedActionOutcome<Rejection, CommittedActionOutput>,
        LedgerCommittedActionError<CommittedActionError>,
    > {
        let mut transaction = self
            .begin_mutation()
            .map_err(LedgerTransactionError::from)
            .map_err(LedgerCommittedActionError::Transaction)?;
        let replayed_state = ReplayedLedgerState {
            events:             &transaction.replay.events,
            generation:         transaction.replay.generation,
            journal_end_offset: transaction.replay.end_offset,
        };
        match validate(replayed_state) {
            CommittedActionValidation::Append { operation, action } => {
                transaction
                    .append(worktree_id, coordination_run_id, *operation)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                let action_output = commit_action(action);
                transaction
                    .publish(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                action_output.map_or_else(
                    |error| Err(LedgerCommittedActionError::Action(error)),
                    |output| Ok(LedgerCommittedActionOutcome::Appended(output)),
                )
            },
            CommittedActionValidation::Reject(rejection) => {
                transaction
                    .publish_if_rebuild_required(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                Ok(LedgerCommittedActionOutcome::Rejected(rejection))
            },
        }
    }

    /// Append reconciliation conclusions and run their repairs under one mutation lock.
    pub(crate) fn transact_reconciliation<
        Rejection,
        CommittedAction,
        CommittedActionOutput,
        CommittedActionError,
    >(
        &self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        validate: impl FnOnce(
            ReplayedLedgerState<'_>,
        ) -> ReconciliationValidation<Rejection, CommittedAction>,
        commit_action: impl FnOnce(
            CommittedAction,
        ) -> Result<CommittedActionOutput, CommittedActionError>,
    ) -> Result<
        LedgerCommittedActionOutcome<Rejection, CommittedActionOutput>,
        LedgerCommittedActionError<CommittedActionError>,
    > {
        let mut transaction = self
            .begin_mutation()
            .map_err(LedgerTransactionError::from)
            .map_err(LedgerCommittedActionError::Transaction)?;
        let replayed_state = ReplayedLedgerState {
            events:             &transaction.replay.events,
            generation:         transaction.replay.generation,
            journal_end_offset: transaction.replay.end_offset,
        };
        match validate(replayed_state) {
            ReconciliationValidation::Apply { operations, action } => {
                for operation in operations {
                    transaction
                        .append(worktree_id, coordination_run_id, operation)
                        .map_err(LedgerCommittedActionError::Transaction)?;
                }
                let action_output = commit_action(action);
                transaction
                    .publish(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                action_output.map_or_else(
                    |error| Err(LedgerCommittedActionError::Action(error)),
                    |output| Ok(LedgerCommittedActionOutcome::Appended(output)),
                )
            },
            ReconciliationValidation::Reject(rejection) => {
                transaction
                    .publish_if_rebuild_required(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                Ok(LedgerCommittedActionOutcome::Rejected(rejection))
            },
        }
    }

    /// Remove and rebuild only the disposable projection from journal truth.
    pub(crate) fn repair_projection(repository_root: &Path) -> Result<(), LedgerError> {
        let ledger = Self::locate(repository_root)?;
        ledger.require_existing()?;
        let _lock = MutationLock::acquire(&ledger.paths.lock, MUTATING_VERB_CONTENTION_TOLERANCE)?;
        let repo_instance_id = read_repo_instance_id(&ledger.paths.repo_instance_id)?;
        let replay = Journal::replay_read_only(&ledger.paths.journal)?;
        validate_journal_repository(repo_instance_id, &replay)?;
        match fs::remove_file(&ledger.paths.projection) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(error) => return Err(LedgerError::Io(error)),
        }
        Projection::from_replay(repo_instance_id, &replay)
            .publish(&ledger.paths.directory, &ledger.paths.projection)?;
        Ok(())
    }

    /// Discard journal truth only after the caller has confirmed pending orders were reviewed.
    pub(crate) fn reinitialize_after_review(
        repository_root: &Path,
    ) -> Result<LedgerReinitialization, LedgerError> {
        let ledger = Self::locate(repository_root)?;
        if !ledger.paths.directory.is_dir() || !ledger.paths.repo_instance_id.is_file() {
            return Err(LedgerError::NotInitialized);
        }
        let _lock = MutationLock::acquire(&ledger.paths.lock, MUTATING_VERB_CONTENTION_TOLERANCE)?;
        let journal_bytes = match fs::read(&ledger.paths.journal) {
            Ok(journal_bytes) => journal_bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(LedgerError::Io(error)),
        };
        let discarded_bytes = u64::try_from(journal_bytes.len())
            .map_err(|_| LedgerError::JournalSizeUnrepresentable)?;
        let complete_record_count = journal_bytes.split(|byte| *byte == b'\n').count() - 1;
        let discarded_complete_records = u64::try_from(complete_record_count)
            .map_err(|_| LedgerError::JournalSizeUnrepresentable)?;
        let (journal, _) = Journal::open_or_create(&ledger.paths.journal)?;
        journal.truncate()?;
        let repo_instance_id = read_repo_instance_id(&ledger.paths.repo_instance_id)?;
        let replay = Journal::replay_read_only(&ledger.paths.journal)?;
        Projection::from_replay(repo_instance_id, &replay)
            .publish(&ledger.paths.directory, &ledger.paths.projection)?;
        fs::File::open(&ledger.paths.directory)?.sync_all()?;
        Ok(LedgerReinitialization {
            discarded_bytes,
            discarded_complete_records,
        })
    }

    fn locate(repository_root: &Path) -> Result<Self, LedgerError> {
        let common_git_directory = git::common_directory(repository_root)?;
        Ok(Self::at_common_git_directory(&common_git_directory))
    }

    fn at_common_git_directory(common_git_directory: &Path) -> Self {
        let directory = common_git_directory.join(LEDGER_DIRECTORY_NAME);
        Self {
            paths: LedgerPaths {
                journal: directory.join(JOURNAL_FILE_NAME),
                lock: directory.join(LOCK_FILE_NAME),
                projection: directory.join(PROJECTION_FILE_NAME),
                repo_instance_id: directory.join(REPO_INSTANCE_ID_FILE_NAME),
                directory,
            },
        }
    }

    fn require_existing(&self) -> Result<(), LedgerError> {
        if !self.paths.directory.is_dir()
            || !self.paths.journal.is_file()
            || !self.paths.repo_instance_id.is_file()
        {
            return Err(LedgerError::NotInitialized);
        }
        Ok(())
    }

    fn begin_mutation(&self) -> Result<LedgerTransaction, LedgerError> {
        self.begin_mutation_with_tolerance(MUTATING_VERB_CONTENTION_TOLERANCE)
    }

    fn begin_mutation_with_tolerance(
        &self,
        contention_tolerance: Duration,
    ) -> Result<LedgerTransaction, LedgerError> {
        self.require_existing()?;
        let lock = MutationLock::acquire(&self.paths.lock, contention_tolerance)?;
        let repo_instance_id = read_repo_instance_id(&self.paths.repo_instance_id)?;
        let journal = Journal::open_existing(&self.paths.journal)?;
        self.begin_locked_transaction(
            lock,
            journal,
            InitializationState::Existing,
            repo_instance_id,
        )
    }

    fn begin_initialization(&self) -> Result<LedgerTransaction, LedgerError> {
        let lock = MutationLock::acquire(&self.paths.lock, MUTATING_VERB_CONTENTION_TOLERANCE)?;
        let repo_instance_id = read_or_mint_repo_instance_id(&self.paths.repo_instance_id)?;
        let (journal, journal_initialization) = Journal::open_or_create(&self.paths.journal)?;
        self.begin_locked_transaction(lock, journal, journal_initialization, repo_instance_id)
    }

    fn begin_locked_transaction(
        &self,
        lock: MutationLock,
        journal: Journal,
        journal_initialization: InitializationState,
        repo_instance_id: RepoInstanceId,
    ) -> Result<LedgerTransaction, LedgerError> {
        let replay = journal.replay_repairing_tail()?;
        validate_journal_repository(repo_instance_id, &replay)?;
        let projection_synchronization =
            read_validated(&self.paths.projection, repo_instance_id, &replay)?;
        Ok(LedgerTransaction {
            _lock: lock,
            journal,
            journal_initialization,
            projection_synchronization,
            replay,
            repo_instance_id,
        })
    }
}

impl LedgerTransaction {
    fn append(
        &mut self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        operation: JournalOperation,
    ) -> Result<JournalEvent, LedgerTransactionError> {
        let next_generation = next_projection_generation(self.replay.generation)
            .map_err(LedgerTransactionError::LedgerUnreadable)?;
        let event = JournalEvent::for_operation(
            JournalActor {
                repository: self.repo_instance_id,
                worktree:   worktree_id,
                run:        coordination_run_id,
            },
            next_generation,
            operation,
        );
        self.journal.append(&event).map_err(|error| match error {
            JournalAppendError::RecordTooLarge { bytes } => {
                LedgerTransactionError::CorrectableInput(
                    CorrectableTransactionInput::RecordTooLarge {
                        bytes,
                        maximum_bytes: MAXIMUM_JOURNAL_RECORD_BYTES,
                    },
                )
            },
            JournalAppendError::Io(error) => {
                LedgerTransactionError::LedgerUnreadable(LedgerError::Io(error))
            },
            JournalAppendError::Serialization(error) => {
                LedgerTransactionError::LedgerUnreadable(LedgerError::JournalEncoding(error))
            },
        })?;
        self.replay = self
            .journal
            .replay_repairing_tail()
            .map_err(LedgerError::from)
            .map_err(LedgerTransactionError::LedgerUnreadable)?;
        Ok(event)
    }

    fn publish(&self, paths: &LedgerPaths) -> Result<(), LedgerError> {
        Projection::from_replay(self.repo_instance_id, &self.replay)
            .publish(&paths.directory, &paths.projection)?;
        Ok(())
    }

    fn publish_if_rebuild_required(&self, paths: &LedgerPaths) -> Result<(), LedgerError> {
        match self.projection_synchronization {
            ProjectionSynchronization::Current => Ok(()),
            ProjectionSynchronization::RebuildRequired => self.publish(paths),
        }
    }
}

struct LedgerTransaction {
    _lock:                      MutationLock,
    journal:                    Journal,
    journal_initialization:     InitializationState,
    projection_synchronization: ProjectionSynchronization,
    replay:                     JournalReplay,
    repo_instance_id:           RepoInstanceId,
}

struct LedgerPaths {
    directory:        PathBuf,
    journal:          PathBuf,
    projection:       PathBuf,
    lock:             PathBuf,
    repo_instance_id: PathBuf,
}

fn read_repo_instance_id(path: &Path) -> Result<RepoInstanceId, LedgerError> {
    fs::read_to_string(path)?
        .trim()
        .parse()
        .map_err(LedgerError::InvalidRepoInstanceId)
}

fn validate_journal_repository(
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

/// Read or mint the clone-wide identity stored beside the journal.
fn read_or_mint_repo_instance_id(path: &Path) -> Result<RepoInstanceId, LedgerError> {
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
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let new_id = WorktreeId::new();
            let mut identity_file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&identity_path)
            {
                Ok(identity_file) => identity_file,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
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

/// Read an existing worktree identity without minting a replacement.
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
    /// The shared ledger has not been initialized for this repository.
    NotInitialized,
    /// No containing `.git` directory or file was found by filesystem traversal.
    RepositoryNotFound,
    /// A `.git` file did not contain a usable `gitdir:` locator.
    InvalidGitDirectoryFile,
    /// A linked-worktree administrative directory had no valid common-directory locator.
    InvalidCommonDirectoryFile,
    /// A linked-worktree administrative directory was outside the common git directory.
    AdministrativeDirectoryOutsideCommonGitDirectory,
    /// A discovered administrative path was not UTF-8.
    NonUtf8AdministrativePath,
    /// The derived administrative locator did not satisfy the journal contract.
    InvalidAdministrativeLocator(String),
    /// A canonical worktree root could not be reconstructed during identity validation.
    InvalidCanonicalWorktreeRoot,
    /// Git could not locate the common administrative directory.
    Git(GitError),
    /// Ordinary filesystem access failed.
    Io(std::io::Error),
    /// Repository policy could not be initialized or read.
    Config(ConfigError),
    /// The append-only journal could not be replayed safely.
    Journal(JournalError),
    /// A validated fact could not be encoded for the journal.
    JournalEncoding(serde_json::Error),
    /// The projection cache could not be validated or published.
    Projection(ProjectionError),
    /// The mutation lock could not be acquired.
    MutationLock(MutationLockError),
    /// The stored repository identity is not a UUID-v7 value.
    InvalidRepoInstanceId(InvalidUuidV7),
    /// The stored worktree identity is not a UUID-v7 value.
    InvalidWorktreeId(InvalidUuidV7),
    /// A registered administrative directory did not prove the recorded holder identity.
    WorktreeIdentityMismatch,
    /// A journal event names a repository identity different from its ledger.
    RepositoryIdentityMismatch,
    /// The projection counter can no longer advance.
    ProjectionGenerationExhausted,
    /// The journal size could not be represented by the public reinitialization result.
    JournalSizeUnrepresentable,
    /// Best-effort bypass auditing could not wait for or encode the journal transaction.
    BypassAuditUnavailable,
}

impl Display for LedgerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized => formatter.write_str(
                "the cargo-berth ledger is not initialized; run cargo-berth init and retry",
            ),
            Self::RepositoryNotFound => {
                formatter.write_str("no containing git worktree could be found")
            },
            Self::InvalidGitDirectoryFile => {
                formatter.write_str("the worktree .git file has no valid gitdir locator")
            },
            Self::InvalidCommonDirectoryFile => formatter
                .write_str("the linked-worktree administrative directory has no valid commondir"),
            Self::AdministrativeDirectoryOutsideCommonGitDirectory => formatter.write_str(
                "the worktree administrative directory is outside the common git directory",
            ),
            Self::NonUtf8AdministrativePath => {
                formatter.write_str("a discovered git administrative path is not UTF-8")
            },
            Self::InvalidAdministrativeLocator(locator) => {
                write!(
                    formatter,
                    "invalid worktree administrative locator: {locator}"
                )
            },
            Self::InvalidCanonicalWorktreeRoot => {
                formatter.write_str("a validated worktree root is not canonical absolute UTF-8")
            },
            Self::Git(error) => write!(formatter, "could not locate ledger: {error}"),
            Self::Io(error) => write!(formatter, "ledger I/O failed: {error}"),
            Self::Config(error) => write!(formatter, "ledger configuration failed: {error}"),
            Self::Journal(error) => write!(formatter, "journal replay failed: {error}"),
            Self::JournalEncoding(error) => {
                write!(formatter, "journal encoding failed: {error}")
            },
            Self::Projection(error) => write!(formatter, "projection validation failed: {error}"),
            Self::MutationLock(error) => write!(formatter, "ledger mutation lock failed: {error}"),
            Self::InvalidRepoInstanceId(error) => {
                write!(formatter, "invalid stored repository identity: {error}")
            },
            Self::InvalidWorktreeId(error) => {
                write!(formatter, "invalid stored worktree identity: {error}")
            },
            Self::WorktreeIdentityMismatch => formatter
                .write_str("registered worktree identity does not match the recorded holder"),
            Self::RepositoryIdentityMismatch => {
                formatter.write_str("journal belongs to a different repository instance")
            },
            Self::ProjectionGenerationExhausted => {
                formatter.write_str("projection generation counter is exhausted")
            },
            Self::JournalSizeUnrepresentable => {
                formatter.write_str("journal size cannot be represented for reinitialization")
            },
            Self::BypassAuditUnavailable => {
                formatter.write_str("the bypass audit could not be journalled immediately")
            },
        }
    }
}

impl std::error::Error for LedgerError {}

impl From<GitError> for LedgerError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

impl From<std::io::Error> for LedgerError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<ConfigError> for LedgerError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<JournalError> for LedgerError {
    fn from(error: JournalError) -> Self { Self::Journal(error) }
}

impl From<ProjectionError> for LedgerError {
    fn from(error: ProjectionError) -> Self { Self::Projection(error) }
}

impl From<MutationLockError> for LedgerError {
    fn from(error: MutationLockError) -> Self { Self::MutationLock(error) }
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

/// A transaction failure classified for a stateful command boundary.
#[derive(Debug)]
pub(crate) enum LedgerTransactionError {
    /// Durable state could not be read or published reliably.
    LedgerUnreadable(LedgerError),
    /// Another live mutation retained the descriptor through the bounded retry window.
    LockContention,
    /// The proposal is validly classified as caller-correctable input.
    CorrectableInput(CorrectableTransactionInput),
}

impl LedgerTransactionError {
    fn from_ledger_error(error: LedgerError) -> Self {
        match error {
            LedgerError::MutationLock(MutationLockError::AcquisitionTimedOut) => {
                Self::LockContention
            },
            ledger_error => Self::LedgerUnreadable(ledger_error),
        }
    }
}

impl From<LedgerError> for LedgerTransactionError {
    fn from(error: LedgerError) -> Self { Self::from_ledger_error(error) }
}

impl Display for LedgerTransactionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LedgerUnreadable(error) => error.fmt(formatter),
            Self::LockContention => formatter.write_str(
                "another cargo-berth operation is still running; wait for it to finish, then retry",
            ),
            Self::CorrectableInput(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for LedgerTransactionError {}

/// A failure before or after an append that authorizes one committed side effect.
#[derive(Debug)]
pub(crate) enum LedgerCommittedActionError<CommittedActionError> {
    /// The locked journal transaction itself failed.
    Transaction(LedgerTransactionError),
    /// The journal append committed, but its authorized side effect failed.
    Action(CommittedActionError),
}

impl<CommittedActionError: fmt::Display> Display
    for LedgerCommittedActionError<CommittedActionError>
{
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transaction(error) => error.fmt(formatter),
            Self::Action(error) => error.fmt(formatter),
        }
    }
}

impl<CommittedActionError> std::error::Error for LedgerCommittedActionError<CommittedActionError> where
    CommittedActionError: std::error::Error + 'static
{
}

/// A rejected mutation input that the caller can reduce and submit again.
#[derive(Debug)]
pub(crate) enum CorrectableTransactionInput {
    /// The encoded journal fact exceeded the bounded record format.
    RecordTooLarge {
        /// The proposed record size including its newline.
        bytes:         usize,
        /// The maximum accepted record size.
        maximum_bytes: usize,
    },
}

impl Display for CorrectableTransactionInput {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecordTooLarge {
                bytes,
                maximum_bytes,
            } => write!(
                formatter,
                "the proposed reservation record is {bytes} bytes, above the {maximum_bytes}-byte limit; reduce its scopes or shorten its provenance and purpose, then retry"
            ),
        }
    }
}

impl std::error::Error for CorrectableTransactionInput {}

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
    use std::thread::JoinHandle;

    use serde_json::Value;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::BypassCause;
    use super::BypassedAction;
    use super::COORDINATION_RUN_MARKER_FILE_NAME;
    use super::CommittedActionValidation;
    use super::CoordinationRunMarkerAtRetirement;
    use super::CoordinationRunMarkerRemoval;
    use super::CorrectableTransactionInput;
    use super::EditAuthorization;
    use super::ForcedIntegrationReason;
    use super::JournalEvent;
    use super::JournalOperation;
    use super::Ledger;
    use super::LedgerCommittedActionOutcome;
    use super::LedgerError;
    use super::LedgerTransactionError;
    use super::LedgerTransactionOutcome;
    use super::TransactionValidation;
    use super::WorktreeContext;
    use super::worktree_identity;
    use crate::ids::CoordinationRunId;
    use crate::ids::ForcedIntegrationPermitId;
    use crate::ids::RepoInstanceId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ids::WorktreeKind;

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
        let ledger = Arc::new(Ledger::open(repository.path()).expect("ledger should open"));
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
        Ledger::repair_projection(repository.path()).expect("projection should rebuild");
        assert_eq!(
            fs::read(projection_path).expect("rebuilt projection should read"),
            original_projection
        );
    }

    #[test]
    fn validation_controls_whether_exactly_one_record_is_appended() {
        let repository = scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");
        let journal_before = fs::read(&ledger.paths.journal).expect("journal should read");
        let projection_before = fs::read(&ledger.paths.projection).expect("projection should read");

        let rejected = ledger
            .transact(WorktreeId::new(), CoordinationRunId::new(), |state| {
                assert!(state.events().is_empty());
                assert_eq!(u64::from(state.generation()), 0);
                assert_eq!(u64::from(state.journal_end_offset()), 0);
                TransactionValidation::Reject("overlap")
            })
            .expect("semantic rejection should not be a ledger failure");

        assert!(matches!(
            rejected,
            LedgerTransactionOutcome::Rejected("overlap")
        ));
        assert_eq!(
            fs::read(&ledger.paths.journal).expect("journal should reread"),
            journal_before
        );
        assert_eq!(
            fs::read(&ledger.paths.projection).expect("projection should reread"),
            projection_before
        );

        let appended = ledger
            .transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
                TransactionValidation::<()>::Append(Box::new(renewal_operation()))
            })
            .expect("approved transaction should append");

        assert!(matches!(&appended, LedgerTransactionOutcome::Appended(_)));
        let LedgerTransactionOutcome::Appended(event) = appended else {
            return;
        };
        assert_eq!(u64::from(event.projection_generation), 1);
        assert_eq!(
            fs::read_to_string(&ledger.paths.journal)
                .expect("journal should read")
                .lines()
                .count(),
            1
        );
        let projection: Value = serde_json::from_slice(
            &fs::read(&ledger.paths.projection).expect("projection should read"),
        )
        .expect("projection should decode");
        assert_eq!(projection["generation"], 1);
    }

    #[test]
    fn committed_actions_run_after_append_while_the_mutation_lock_is_held() {
        let repository = scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");
        let competing_lock = fs::File::options()
            .read(true)
            .write(true)
            .open(&ledger.paths.lock)
            .expect("competing lock descriptor should open");

        let outcome = ledger
            .transact_with_committed_action(
                WorktreeId::new(),
                CoordinationRunId::new(),
                |_| CommittedActionValidation::<(), ()>::Append {
                    operation: Box::new(renewal_operation()),
                    action:    (),
                },
                |()| {
                    let contention = competing_lock.try_lock();
                    assert!(
                        matches!(contention, Err(std::fs::TryLockError::WouldBlock)),
                        "the committed action must retain the mutation lock"
                    );
                    Ok::<(), std::io::Error>(())
                },
            )
            .expect("committed action transaction should succeed");

        assert!(matches!(
            outcome,
            LedgerCommittedActionOutcome::Appended(())
        ));
        assert_eq!(
            fs::read_to_string(&ledger.paths.journal)
                .expect("journal should read")
                .lines()
                .count(),
            1
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

    #[test]
    fn crate_visible_transaction_types_support_a_validator() {
        let repository = scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");

        let outcome = sibling_style_validator::append_bypass(&ledger)
            .expect("crate-visible validator should append");

        assert!(matches!(
            outcome,
            LedgerTransactionOutcome::Appended(event)
                if matches!(
                    event.operation,
                    JournalOperation::Bypass {
                        action: BypassedAction::Editing,
                        ..
                    }
                )
        ));
    }

    #[test]
    fn opening_requires_an_initialized_ledger_and_accepts_nested_callers() {
        let repository = scratch_repository();
        let ledger_directory = repository.path().join(".git").join("cargo-berth");

        assert!(matches!(
            Ledger::open(repository.path()),
            Err(LedgerError::NotInitialized)
        ));
        assert!(!ledger_directory.exists());

        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let nested_directory = repository.path().join("crates").join("nested");
        fs::create_dir_all(&nested_directory).expect("nested directory should exist");
        assert!(Ledger::open(&nested_directory).is_ok());
    }

    #[test]
    fn edit_authorization_prefers_environment_then_marker_then_unidentified() {
        let administrative_directory = tempdir().expect("administrative directory should exist");
        let environment_run = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b"
            .parse::<CoordinationRunId>()
            .expect("environment run should parse");
        let marker_run = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c"
            .parse::<CoordinationRunId>()
            .expect("marker run should parse");
        let marker_worktree =
            worktree_identity(administrative_directory.path(), WorktreeKind::Linked)
                .expect("marker worktree identity should mint")
                .id;
        fs::write(
            administrative_directory
                .path()
                .join(COORDINATION_RUN_MARKER_FILE_NAME),
            format!("{marker_run}\n"),
        )
        .expect("coordination marker should write");

        assert_eq!(
            EditAuthorization::resolve_from_environment(
                Some(environment_run.to_string().into()),
                administrative_directory.path(),
            ),
            EditAuthorization::Environment(environment_run)
        );
        assert_eq!(
            EditAuthorization::resolve_from_environment(None, administrative_directory.path(),),
            EditAuthorization::Marker {
                coordination_run_id: marker_run,
                worktree_id:         marker_worktree,
            }
        );

        fs::remove_file(
            administrative_directory
                .path()
                .join(COORDINATION_RUN_MARKER_FILE_NAME),
        )
        .expect("coordination marker should remove");
        assert_eq!(
            EditAuthorization::resolve_from_environment(None, administrative_directory.path(),),
            EditAuthorization::Unidentified
        );
        assert!(matches!(
            EditAuthorization::resolve(administrative_directory.path()),
            EditAuthorization::Environment(_)
                | EditAuthorization::Marker { .. }
                | EditAuthorization::Unidentified
        ));
    }

    #[test]
    fn oversized_records_are_correctable_input_not_unreadable_state() {
        let repository = scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");

        let result = ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
            TransactionValidation::<()>::Append(Box::new(JournalOperation::Bypass {
                action: BypassedAction::Editing,
                cause:  BypassCause::ForcedIntegration {
                    permit_id: ForcedIntegrationPermitId::new(),
                    reason:    "x"
                        .repeat(super::MAXIMUM_JOURNAL_RECORD_BYTES)
                        .parse::<ForcedIntegrationReason>()
                        .expect("oversized reason should remain non-empty"),
                },
            }))
        });

        assert!(matches!(
            result,
            Err(LedgerTransactionError::CorrectableInput(
                CorrectableTransactionInput::RecordTooLarge { .. }
            ))
        ));
        assert!(
            fs::read_to_string(&ledger.paths.journal)
                .expect("journal should read")
                .is_empty()
        );
    }

    #[test]
    fn rejected_transaction_rebuilds_a_stale_projection() {
        let repository = scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");
        let mut unpublished_transaction = ledger
            .begin_mutation()
            .expect("unpublished transaction should begin");
        unpublished_transaction
            .append(
                WorktreeId::new(),
                CoordinationRunId::new(),
                renewal_operation(),
            )
            .expect("record should append before publication");
        std::mem::drop(unpublished_transaction);

        let mut record_was_visible = false;
        let outcome = ledger
            .transact(WorktreeId::new(), CoordinationRunId::new(), |state| {
                record_was_visible = state.events().len() == 1;
                TransactionValidation::Reject(())
            })
            .expect("next reader should replay journal truth");

        assert!(record_was_visible);
        assert!(matches!(outcome, LedgerTransactionOutcome::Rejected(())));
        let journal = fs::read(&ledger.paths.journal).expect("journal should read");
        let projection: Value = serde_json::from_slice(
            &fs::read(&ledger.paths.projection).expect("projection should read"),
        )
        .expect("projection should decode");
        assert_eq!(projection["generation"], 1);
        assert_eq!(projection["journal_end_offset"], journal.len());
        assert_eq!(
            projection["events"]
                .as_array()
                .expect("projection events should be an array")
                .len(),
            1
        );
    }

    #[test]
    fn projection_reads_validate_both_generation_and_journal_byte_offset() {
        for field in ["generation", "journal_end_offset"] {
            let repository = scratch_repository();
            Ledger::initialize(repository.path()).expect("ledger should initialize");
            let ledger = Ledger::open(repository.path()).expect("ledger should open");
            let mut projection: Value = serde_json::from_slice(
                &fs::read(&ledger.paths.projection).expect("projection should read"),
            )
            .expect("projection should decode");
            projection[field] = serde_json::json!(1);
            fs::write(
                &ledger.paths.projection,
                serde_json::to_vec_pretty(&projection).expect("projection should encode"),
            )
            .expect("projection should write");

            assert!(matches!(
                ledger.begin_mutation(),
                Err(LedgerError::Projection(
                    super::projection::ProjectionError::CacheAhead
                ))
            ));
        }
    }

    fn scratch_repository() -> TempDir {
        let repository = tempdir().expect("temporary repository should exist");
        let git_init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repository.path())
            .status()
            .expect("git should initialize a scratch repository");
        assert!(git_init.success());
        repository
    }

    fn append_renewal(ledger: Arc<Ledger>) -> JoinHandle<Result<(), LedgerTransactionError>> {
        thread::spawn(move || {
            ledger
                .transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
                    TransactionValidation::<()>::Append(Box::new(renewal_operation()))
                })
                .map(|_| ())
        })
    }

    fn renewal_operation() -> JournalOperation {
        JournalOperation::Renew {
            reservation_id: ReservationId::new(),
        }
    }

    mod sibling_style_validator {
        use crate::ids::CoordinationRunId;
        use crate::ids::WorktreeId;
        use crate::ledger::BypassCause;
        use crate::ledger::BypassedAction;
        use crate::ledger::JournalOperation;
        use crate::ledger::Ledger;
        use crate::ledger::LedgerTransactionError;
        use crate::ledger::LedgerTransactionOutcome;
        use crate::ledger::TransactionValidation;

        pub(super) fn append_bypass(
            ledger: &Ledger,
        ) -> Result<LedgerTransactionOutcome<()>, LedgerTransactionError> {
            ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |state| {
                assert!(state.events().is_empty());
                assert_eq!(u64::from(state.generation()), 0);
                assert_eq!(u64::from(state.journal_end_offset()), 0);
                TransactionValidation::Append(Box::new(JournalOperation::Bypass {
                    action: BypassedAction::Editing,
                    cause:  BypassCause::EnvironmentOverride,
                }))
            })
        }
    }
}
