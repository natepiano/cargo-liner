//! The shared-ledger handle and the validation-controlled transactions it drives.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use super::constants::JOURNAL_FILE_NAME;
use super::constants::LEDGER_DIRECTORY_NAME;
use super::constants::LOCK_FILE_NAME;
use super::constants::MAXIMUM_JOURNAL_RECORD_BYTES;
use super::constants::MUTATING_VERB_CONTENTION_TOLERANCE;
use super::constants::PROJECTION_FILE_NAME;
use super::constants::REPO_INSTANCE_ID_FILE_NAME;
use super::error::CorrectableTransactionInput;
use super::error::LedgerCommittedActionError;
use super::error::LedgerError;
use super::error::LedgerTransactionError;
use super::identity::read_or_create_repo_instance_id;
use super::identity::read_repo_instance_id;
use super::identity::validate_journal_repository;
use super::journal::Journal;
use super::journal::JournalActor;
use super::journal::JournalAppendError;
use super::journal::JournalEvent;
use super::journal::JournalOperation;
use super::journal::JournalReplay;
use super::lock::MutationLock;
use super::projection::Projection;
use super::projection::ProjectionSynchronization;
use super::projection::read_validated;
use super::worktree_context::WorktreeContext;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::config::InitializationState;
use crate::git;
use crate::ids::CoordinationRunId;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RepoInstanceId;
use crate::ids::WorktreeId;
use crate::session;
use crate::session::CurrentSessionMappingRemoval;
use crate::session::SessionIdentityMappingPublication;

/// The shared append-only ledger for one git common directory.
pub(crate) struct Ledger {
    paths: LedgerPaths,
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
    pub(crate) const fn journal_end_offset(&self) -> JournalByteOffset { self.journal_end_offset }
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
        /// Journal operations whose failure invalidates the entire reconciliation.
        operations:             Vec<JournalOperation>,
        /// Marker imports whose failure is reported without rejecting other reconciliation work.
        recoverable_operations: Vec<JournalOperation>,
        /// Idempotent filesystem and git repairs authorized after the appends.
        action:                 CommittedAction,
    },
    /// Stop without changing journal or side-effect state.
    Reject(Rejection),
}

/// Marker imports that reconciliation could not append after repairing any partial tail.
pub(crate) struct RecoverableReconciliationAppendFailures {
    operations: Vec<JournalOperation>,
}

impl RecoverableReconciliationAppendFailures {
    /// Return whether this exact recoverable operation failed to append.
    pub(crate) fn contains(&self, operation: &JournalOperation) -> bool {
        self.operations.contains(operation)
    }
}

/// The durable result of a validation-controlled ledger transaction.
pub(crate) enum LedgerTransactionOutcome<Rejection> {
    /// Exactly one approved event was appended and published.
    Appended {
        /// The durable journal event.
        event:                       Box<JournalEvent>,
        /// Whether the event's session identity consequence was published.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    /// Validation rejected the proposal before any append.
    Rejected(Rejection),
}

/// The result of a transaction whose appended record authorizes a locked side effect.
pub(crate) enum LedgerCommittedActionOutcome<Rejection, CommittedActionOutput> {
    /// The event committed, its action ran under the lock, and the projection published.
    Appended {
        /// The output produced by the committed action.
        output:                      CommittedActionOutput,
        /// Whether the event's session identity consequence was published.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    /// Validation rejected the proposal before any append or side effect.
    Rejected(Rejection),
}

impl Ledger {
    /// Resolve the shared ledger and create its journal, projection, and default config.
    pub(crate) fn initialize(repository_root: &Path) -> Result<LedgerInitialization, LedgerError> {
        let ledger = Self::locate(repository_root)?;
        fs::create_dir_all(&ledger.paths.directory)?;
        let transaction = ledger.begin_initialization()?;
        let configuration = BerthConfig::initialize(repository_root)?;
        transaction.publish(&ledger.paths)?;
        Ok(LedgerInitialization {
            ledger: transaction.journal_initialization,
            configuration,
        })
    }

    /// Attach to an initialized ledger after the caller has resolved repository enrollment.
    ///
    /// Production callers must match [`Enrollment`] from [`BerthConfig::read`] before opening the
    /// shared ledger.
    pub(crate) fn open(invocation_directory: &Path) -> Result<Self, LedgerError> {
        let repository_root = git::repository_root(invocation_directory)?;
        let ledger = Self::locate(&repository_root)?;
        ledger.require_existing()?;
        Ok(ledger)
    }

    /// Attach using a worktree context already discovered from `.git` filesystem metadata.
    pub(crate) fn open_from_discovered_worktree(
        worktree_context: &WorktreeContext,
    ) -> Result<Self, LedgerError> {
        let ledger = Self::at_common_git_directory(worktree_context.common_git_directory());
        ledger.require_existing()?;
        Ok(ledger)
    }

    /// Read the clone identity that owns this ledger.
    pub(crate) fn repository_identity(&self) -> Result<RepoInstanceId, LedgerError> {
        read_repo_instance_id(&self.paths.repo_instance_id)
    }

    /// Remove only the harness-session mapping selected by this process.
    pub(crate) fn remove_current_session_mapping(
        &self,
    ) -> Result<CurrentSessionMappingRemoval, LedgerError> {
        let _lock = MutationLock::acquire(&self.paths.lock, MUTATING_VERB_CONTENTION_TOLERANCE)?;
        session::remove_current_mapping(&self.paths.directory)
            .map_err(LedgerError::SessionIdentityStore)
    }

    /// Read validated journal truth without git, locking, repair, or publication.
    pub(crate) fn read_for_edit_check(
        invocation_directory: &Path,
    ) -> Result<Enrollment<EditCheckLedgerSnapshot>, LedgerError> {
        let worktree_context = WorktreeContext::discover(invocation_directory)?;
        match BerthConfig::read(worktree_context.repository_root())? {
            Enrollment::Enrolled(_) => {},
            Enrollment::Unconfigured {
                expected_configuration_path,
            } => {
                return Ok(Enrollment::Unconfigured {
                    expected_configuration_path,
                });
            },
        }
        let ledger = Self::at_common_git_directory(worktree_context.common_git_directory());
        let events = ledger.read_validated_events()?;
        Ok(Enrollment::Enrolled(EditCheckLedgerSnapshot {
            events,
            worktree_context,
        }))
    }

    /// Read validated journal truth without holding the mutation lock.
    fn read_validated_events(&self) -> Result<Vec<JournalEvent>, LedgerError> {
        self.require_existing()?;
        let repo_instance_id = read_repo_instance_id(&self.paths.repo_instance_id)?;
        let replay = Journal::replay_read_only(&self.paths.journal)?;
        validate_journal_repository(repo_instance_id, &replay)?;
        read_validated(&self.paths.projection, repo_instance_id, &replay)?;
        Ok(replay.events)
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
                let journal_append =
                    transaction.append(worktree_id, coordination_run_id, *operation)?;
                transaction
                    .publish(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)?;
                Ok(LedgerTransactionOutcome::Appended {
                    event:                       Box::new(journal_append.event),
                    session_mapping_publication: journal_append.session_mapping_publication,
                })
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
                let journal_append =
                    transaction.append(worktree_id, coordination_run_id, *operation)?;
                transaction
                    .publish(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)?;
                Ok(LedgerTransactionOutcome::Appended {
                    event:                       Box::new(journal_append.event),
                    session_mapping_publication: journal_append.session_mapping_publication,
                })
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
        self.transact_with_committed_action_and_consume_locked_outcome(
            worktree_id,
            coordination_run_id,
            validate,
            commit_action,
            |outcome| outcome,
        )
    }

    /// Consume a committed-action outcome before its mutation lock is released.
    pub(crate) fn transact_with_committed_action_and_consume_locked_outcome<
        Rejection,
        CommittedAction,
        CommittedActionOutput,
        CommittedActionError,
        LockedOutcome,
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
        consume_locked_outcome: impl FnOnce(
            LedgerCommittedActionOutcome<Rejection, CommittedActionOutput>,
        ) -> LockedOutcome,
    ) -> Result<LockedOutcome, LedgerCommittedActionError<CommittedActionError>> {
        let mut transaction = self
            .begin_mutation()
            .map_err(LedgerTransactionError::from)
            .map_err(LedgerCommittedActionError::Transaction)?;
        let replayed_state = ReplayedLedgerState {
            events:             &transaction.replay.events,
            generation:         transaction.replay.generation,
            journal_end_offset: transaction.replay.end_offset,
        };
        let outcome = match validate(replayed_state) {
            CommittedActionValidation::Append { operation, action } => {
                let journal_append = transaction
                    .append(worktree_id, coordination_run_id, *operation)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                let action_output = commit_action(action);
                transaction
                    .publish(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                action_output.map_or_else(
                    |error| Err(LedgerCommittedActionError::Action(error)),
                    |output| {
                        Ok(LedgerCommittedActionOutcome::Appended {
                            output,
                            session_mapping_publication: journal_append.session_mapping_publication,
                        })
                    },
                )?
            },
            CommittedActionValidation::Reject(rejection) => {
                transaction
                    .publish_if_rebuild_required(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                LedgerCommittedActionOutcome::Rejected(rejection)
            },
        };
        let locked_outcome = consume_locked_outcome(outcome);
        std::mem::drop(transaction);
        Ok(locked_outcome)
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
            &ReplayedLedgerState<'_>,
            &RecoverableReconciliationAppendFailures,
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
            ReconciliationValidation::Apply {
                operations,
                recoverable_operations,
                action,
            } => {
                let mut session_mapping_publication = transaction
                    .append_reconciliation_operations(worktree_id, coordination_run_id, operations)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                let mut recoverable_failures = RecoverableReconciliationAppendFailures {
                    operations: Vec::new(),
                };
                for operation in recoverable_operations {
                    let retained_operation = operation.clone();
                    if let Ok(journal_append) =
                        transaction.append(worktree_id, coordination_run_id, operation)
                    {
                        session_mapping_publication = session_mapping_publication
                            .merge(journal_append.session_mapping_publication);
                    } else {
                        transaction
                            .recover_after_recoverable_append_failure()
                            .map_err(LedgerCommittedActionError::Transaction)?;
                        recoverable_failures.operations.push(retained_operation);
                    }
                }
                let committed_state = ReplayedLedgerState {
                    events:             &transaction.replay.events,
                    generation:         transaction.replay.generation,
                    journal_end_offset: transaction.replay.end_offset,
                };
                let action_output = commit_action(action, &committed_state, &recoverable_failures);
                transaction
                    .publish(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                action_output.map_or_else(
                    |error| Err(LedgerCommittedActionError::Action(error)),
                    |output| {
                        Ok(LedgerCommittedActionOutcome::Appended {
                            output,
                            session_mapping_publication,
                        })
                    },
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
        let repo_instance_id = read_or_create_repo_instance_id(&self.paths.repo_instance_id)?;
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
            ledger_directory: self.paths.directory.clone(),
            projection_synchronization,
            replay,
            repo_instance_id,
        })
    }
}

impl LedgerTransaction {
    fn append_reconciliation_operations(
        &mut self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        operations: Vec<JournalOperation>,
    ) -> Result<SessionIdentityMappingPublication, LedgerTransactionError> {
        if operations.is_empty() {
            return Ok(SessionIdentityMappingPublication::Published);
        }
        let actor = JournalActor {
            repository: self.repo_instance_id,
            worktree:   worktree_id,
            run:        coordination_run_id,
        };
        let mut generation = self.replay.generation;
        let mut events = Vec::with_capacity(operations.len());
        for operation in operations {
            generation = next_projection_generation(generation)
                .map_err(LedgerTransactionError::LedgerUnreadable)?;
            events.push(JournalEvent::for_operation(
                actor.clone(),
                generation,
                operation,
            ));
        }
        self.journal
            .append_events(&events)
            .map_err(journal_append_transaction_error)?;
        self.replay = self
            .journal
            .replay_repairing_tail()
            .map_err(LedgerError::from)
            .map_err(LedgerTransactionError::LedgerUnreadable)?;
        let mut session_mapping_publication = SessionIdentityMappingPublication::Published;
        for event in &events {
            session_mapping_publication = session_mapping_publication
                .merge(session::apply_journal_event(&self.ledger_directory, event));
        }
        Ok(session_mapping_publication)
    }

    fn append(
        &mut self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        operation: JournalOperation,
    ) -> Result<JournalAppend, LedgerTransactionError> {
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
        self.journal
            .append(&event)
            .map_err(journal_append_transaction_error)?;
        self.replay = self
            .journal
            .replay_repairing_tail()
            .map_err(LedgerError::from)
            .map_err(LedgerTransactionError::LedgerUnreadable)?;
        let session_mapping_publication =
            session::apply_journal_event(&self.ledger_directory, &event);
        Ok(JournalAppend {
            event,
            session_mapping_publication,
        })
    }

    fn recover_after_recoverable_append_failure(&mut self) -> Result<(), LedgerTransactionError> {
        self.replay = self
            .journal
            .replay_repairing_tail()
            .map_err(LedgerError::from)
            .map_err(LedgerTransactionError::LedgerUnreadable)?;
        validate_journal_repository(self.repo_instance_id, &self.replay)
            .map_err(LedgerTransactionError::LedgerUnreadable)
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

fn journal_append_transaction_error(error: JournalAppendError) -> LedgerTransactionError {
    match error {
        JournalAppendError::RecordTooLarge { bytes } => {
            LedgerTransactionError::CorrectableInput(CorrectableTransactionInput::RecordTooLarge {
                bytes,
                maximum_bytes: MAXIMUM_JOURNAL_RECORD_BYTES,
            })
        },
        JournalAppendError::Io(error) => {
            LedgerTransactionError::LedgerUnreadable(LedgerError::Io(error))
        },
        JournalAppendError::Serialization(error) => {
            LedgerTransactionError::LedgerUnreadable(LedgerError::JournalEncoding(error))
        },
    }
}

struct LedgerTransaction {
    _lock:                      MutationLock,
    journal:                    Journal,
    journal_initialization:     InitializationState,
    ledger_directory:           PathBuf,
    projection_synchronization: ProjectionSynchronization,
    replay:                     JournalReplay,
    repo_instance_id:           RepoInstanceId,
}

struct JournalAppend {
    event:                       JournalEvent,
    session_mapping_publication: SessionIdentityMappingPublication,
}

struct LedgerPaths {
    directory:        PathBuf,
    journal:          PathBuf,
    projection:       PathBuf,
    lock:             PathBuf,
    repo_instance_id: PathBuf,
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::thread;
    use std::thread::JoinHandle;

    use serde_json::Value;

    use super::CommittedActionValidation;
    use super::CorrectableTransactionInput;
    use super::JournalEvent;
    use super::JournalOperation;
    use super::Ledger;
    use super::LedgerCommittedActionOutcome;
    use super::LedgerError;
    use super::LedgerTransactionError;
    use super::LedgerTransactionOutcome;
    use super::MAXIMUM_JOURNAL_RECORD_BYTES;
    use super::TransactionValidation;
    use crate::ids::CoordinationRunId;
    use crate::ids::ForcedIntegrationPermitId;
    use crate::ids::RepoInstanceId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::BypassCause;
    use crate::ledger::BypassOccurrenceTime;
    use crate::ledger::BypassRecording;
    use crate::ledger::BypassedAction;
    use crate::ledger::ForcedIntegrationReason;
    use crate::ledger::projection::ProjectionError;
    use crate::ledger::test_support::scratch_repository;

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

        assert!(matches!(
            &appended,
            LedgerTransactionOutcome::Appended { .. }
        ));
        let LedgerTransactionOutcome::Appended { event, .. } = appended else {
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
            LedgerCommittedActionOutcome::Appended { output: (), .. }
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
    fn crate_visible_transaction_types_support_a_validator() {
        let repository = scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");

        let outcome = sibling_style_validator::append_bypass(&ledger)
            .expect("crate-visible validator should append");

        assert!(matches!(
            outcome,
            LedgerTransactionOutcome::Appended { event, .. }
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
    fn oversized_records_are_correctable_input_not_unreadable_state() {
        let repository = scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");

        let result = ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
            TransactionValidation::<()>::Append(Box::new(JournalOperation::Bypass {
                action:          BypassedAction::Editing,
                cause:           BypassCause::ForcedIntegration {
                    permit_id: ForcedIntegrationPermitId::new(),
                    reason:    "x"
                        .repeat(MAXIMUM_JOURNAL_RECORD_BYTES)
                        .parse::<ForcedIntegrationReason>()
                        .expect("oversized reason should remain non-empty"),
                },
                occurrence_time: BypassOccurrenceTime::EventRecordedAt,
                recording:       BypassRecording::Direct,
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
        assert!(projection.get("events").is_none());
        assert_eq!(
            projection
                .as_object()
                .expect("projection should be an object")
                .len(),
            5
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
                Err(LedgerError::Projection(ProjectionError::CacheAhead))
            ));
        }
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
        use crate::ledger::BypassOccurrenceTime;
        use crate::ledger::BypassRecording;
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
                    action:          BypassedAction::Editing,
                    cause:           BypassCause::EnvironmentOverride {
                        bypassed_merge: crate::ledger::BypassedMergeIdentity::from_hook_token(
                            "ledger-append-test",
                        )
                        .expect("test bypass identity should be non-empty"),
                    },
                    occurrence_time: BypassOccurrenceTime::EventRecordedAt,
                    recording:       BypassRecording::Direct,
                }))
            })
        }
    }
}
