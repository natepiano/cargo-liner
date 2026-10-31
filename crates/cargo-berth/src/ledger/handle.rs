//! The shared-ledger handle and the validation-controlled transactions it drives.

use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs;
use std::fs::Metadata;
use std::io::ErrorKind;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use super::compaction;
use super::constants::GATE_TARGETS_FILE_NAME;
use super::constants::GATE_TARGETS_TEMPORARY_FILE_NAME;
use super::constants::JOURNAL_COMPACTION_REFUSAL_FILE_NAME;
use super::constants::JOURNAL_COMPACTION_THRESHOLD_BYTES;
use super::constants::JOURNAL_FILE_NAME;
use super::constants::LEDGER_DIRECTORY_NAME;
use super::constants::LOCK_FILE_NAME;
use super::constants::MAXIMUM_JOURNAL_RECORD_BYTES;
use super::constants::MUTATING_VERB_CONTENTION_TOLERANCE;
use super::constants::PROJECTION_FILE_NAME;
use super::constants::REPLAY_CHECKPOINT_FILE_NAME;
use super::constants::REPO_INSTANCE_ID_FILE_NAME;
use super::error::CorrectableTransactionInput;
use super::error::LedgerCommittedActionError;
use super::error::LedgerError;
use super::error::LedgerTransactionError;
use super::identity;
use super::journal::ClaimSource;
use super::journal::Journal;
use super::journal::JournalActor;
use super::journal::JournalAppendError;
use super::journal::JournalEvent;
use super::journal::JournalOperation;
use super::journal::JournalReplay;
use super::journal::JournalReplaySource;
use super::journal::PreparedJournalAppend;
use super::journal::RecordCeiling;
use super::lock::MutationLock;
use super::ordering_repair;
use super::ordering_repair::OrderingCycleRepair;
use super::preparation;
use super::projection;
use super::projection::JournalPosition;
use super::projection::Projection;
use super::projection::ProjectionError;
use super::projection::ProjectionSynchronization;
use super::replay_checkpoint;
use super::replay_checkpoint::ReplayCheckpoint;
use super::target::IntegrationTarget;
use super::worktree_context::WorktreeContext;
use crate::config::BerthConfig;
use crate::config::ConfigurationLookup;
use crate::config::Enrollment;
use crate::config::InitializationState;
use crate::edge::CycleFreeOrderingOperation;
use crate::edge::EdgeReplayError;
use crate::edge::OrderingDeclarationError;
use crate::edge::OrderingGraph;
use crate::git;
use crate::ids::CoordinationRunId;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RecordedAt;
use crate::ids::RepoInstanceId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::reservation::RecordedTarget;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::session;
use crate::session::CurrentSessionMappingRemoval;
use crate::session::PreparedSessionMappingPublication;
use crate::session::SessionIdentityMappingPublication;
use crate::session::SessionMappingVersion;

/// Journal truth and file identity used for one preparation pass.
struct JournalPreparation {
    replay: JournalReplay,
    device: u64,
    inode:  u64,
}

impl JournalPreparation {
    fn still_current(&self, path: &Path) -> Result<bool, LedgerError> {
        let metadata = fs::metadata(path)?;
        Ok(metadata.dev() == self.device
            && metadata.ino() == self.inode
            && metadata.len() >= u64::from(self.replay.end_offset))
    }
}

/// The shared ledger for one git common directory.
pub(crate) struct Ledger {
    pub(super) paths:                      LedgerPaths,
    /// The superseded record bytes past which a transaction compacts the journal after it.
    pub(super) compaction_threshold_bytes: u64,
}

/// Validated journal truth for a mutation-free edit check.
pub(crate) struct EditCheckLedgerSnapshot {
    reservations:     Result<RetainedReservationSet, ReservationReplayError>,
    worktree_context: WorktreeContext,
    configuration:    BerthConfig,
}

/// Validated journal truth read without holding the mutation lock.
pub(crate) struct ValidatedJournal {
    coordination_events: Vec<JournalEvent>,
    reservations:        Result<RetainedReservationSet, ReservationReplayError>,
}

impl ValidatedJournal {
    /// Take every record a coordination reader reads, in append order.
    ///
    /// See [`JournalOperation::is_coordination_record`].
    pub(crate) fn into_coordination_events(self) -> Vec<JournalEvent> { self.coordination_events }

    /// Take the reservation set folded from every record, or the first record replay rejected.
    pub(crate) fn into_reservations(
        self,
    ) -> Result<RetainedReservationSet, ReservationReplayError> {
        self.reservations
    }
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
    /// Split this read into its folded reservation set, the filesystem-discovered worktree
    /// context, and the configuration that governs the check.
    pub(crate) fn into_parts(
        self,
    ) -> (
        Result<RetainedReservationSet, ReservationReplayError>,
        WorktreeContext,
        BerthConfig,
    ) {
        (self.reservations, self.worktree_context, self.configuration)
    }
}

/// The replayed journal facts visible to a transaction's validation step.
#[derive(Clone, Copy)]
pub(crate) struct ReplayedLedgerState<'replay> {
    coordination_events: &'replay [JournalEvent],
    ordering_graph:      &'replay OnceCell<Result<OrderingGraph, EdgeReplayError>>,
    reservations:        &'replay Result<RetainedReservationSet, ReservationReplayError>,
    generation:          ProjectionGeneration,
    journal_end_offset:  JournalByteOffset,
}

impl<'replay> ReplayedLedgerState<'replay> {
    /// Borrow every replayed record a coordination reader reads, in append order.
    ///
    /// See [`JournalOperation::is_coordination_record`].
    pub(crate) const fn coordination_events(&self) -> &[JournalEvent] { self.coordination_events }

    /// Reuse the ordering graph for validation and admission against this exact replay.
    pub(crate) fn ordering_graph(&self) -> Result<&OrderingGraph, EdgeReplayError> {
        self.ordering_graph
            .get_or_init(|| OrderingGraph::replay(self.coordination_events))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Borrow the reservation set folded from every replayed record.
    ///
    /// # Errors
    ///
    /// Returns a copy of the first record the fold could not apply.
    pub(crate) fn reservations(
        &self,
    ) -> Result<&'replay RetainedReservationSet, ReservationReplayError> {
        self.reservations.as_ref().map_err(Clone::clone)
    }

    /// Return the projection generation represented by the replay.
    pub(crate) const fn generation(&self) -> ProjectionGeneration { self.generation }

    /// Return the journal byte offset represented by the replay.
    pub(crate) const fn journal_end_offset(&self) -> JournalByteOffset { self.journal_end_offset }
}

/// A transaction either rejects its input or appends raw or already-admitted facts.
pub(crate) enum TransactionValidation<Rejection> {
    /// Append this operation and publish the resulting projection.
    Append(Box<JournalOperation>),
    /// Append the same operation admitted against this transaction's replay.
    AppendChecked(Box<CycleFreeOrderingOperation>),
    /// Return the semantic rejection without changing durable state.
    Reject(Rejection),
}

/// A prepared validation result carrying work permitted only after its append commits.
pub(crate) enum CommittedActionValidation<Rejection, CommittedAction> {
    /// Append this operation, then execute the action while retaining the mutation lock.
    Append {
        /// The journal operation that must commit first.
        operation: Box<JournalOperation>,
        /// The side effect authorized by the committed operation.
        action:    CommittedAction,
    },
    /// Publish an operation already admitted against this transaction's replay.
    AppendChecked {
        /// The checked operation, retained through append without a second admission.
        operation: Box<CycleFreeOrderingOperation>,
        /// Work authorized only after the checked operation commits.
        action:    CommittedAction,
    },
    /// Commit prerequisite evidence and its dependent operation under the same mutation lock.
    AppendWithPrerequisites {
        /// Evidence that must precede the dependent operation in replay order.
        prerequisites: Vec<JournalOperation>,
        /// The dependent operation whose append authorizes the action.
        operation:     Box<JournalOperation>,
        /// Work authorized only after all operations commit.
        action:        CommittedAction,
    },
    /// The requested state already holds; run the locked action without appending.
    AlreadySatisfied(CommittedAction),
    /// Return the semantic rejection without changing durable state.
    Reject(Rejection),
}

/// A prepared reconciliation decision that may append several replayable conclusions.
pub(crate) enum ReconciliationValidation<Rejection, CommittedAction> {
    /// Append each operation, then execute the repair action under the same lock.
    Apply {
        /// Journal operations whose failure invalidates the entire reconciliation.
        operations:             Vec<JournalOperation>,
        /// Worktree actors for cover claims within the operation batch.
        cover_actors:           HashMap<ReservationId, CoverClaimActor>,
        /// Marker imports whose failure is reported without rejecting other reconciliation work.
        recoverable_operations: Vec<JournalOperation>,
        /// Idempotent filesystem and git repairs authorized after the appends.
        action:                 CommittedAction,
    },
    /// Stop without changing journal or side-effect state.
    Reject(Rejection),
}

/// An append's session consequence, prepared against the enclosing mutation's session version.
struct PreparedOperationSessionChange {
    operation:   JournalOperation,
    publication: PreparedSessionMappingPublication,
}

impl PreparedOperationSessionChange {
    fn prepare(
        ledger_directory: &Path,
        run: CoordinationRunId,
        operations: Vec<JournalOperation>,
    ) -> Vec<Self> {
        let publications =
            PreparedSessionMappingPublication::prepare_sequence(ledger_directory, run, &operations);
        operations
            .into_iter()
            .zip(publications)
            .map(|(operation, publication)| Self {
                operation,
                publication,
            })
            .collect()
    }
}

/// An admitted operation and its session publication, bound to one preparation pass.
struct CycleFreeSessionOperation {
    operation:   CycleFreeOrderingOperation,
    publication: PreparedSessionMappingPublication,
}

impl CycleFreeSessionOperation {
    fn prepare(
        ledger_directory: &Path,
        run: CoordinationRunId,
        operation: CycleFreeOrderingOperation,
    ) -> Self {
        let publication = PreparedSessionMappingPublication::prepare_sequence(
            ledger_directory,
            run,
            std::slice::from_ref(operation.operation()),
        )
        .remove(0);
        Self {
            operation,
            publication,
        }
    }

    fn prepare_sequence(
        state: &ReplayedLedgerState<'_>,
        ledger_directory: &Path,
        run: CoordinationRunId,
        operations: Vec<JournalOperation>,
    ) -> Result<Vec<Self>, LedgerTransactionError> {
        let prepared = PreparedOperationSessionChange::prepare(ledger_directory, run, operations);
        let (operations, publications): (Vec<_>, Vec<_>) = prepared
            .into_iter()
            .map(|prepared| (prepared.operation, prepared.publication))
            .unzip();
        let admitted = CycleFreeOrderingOperation::check_sequence(state, operations)
            .map_err(ordering_declaration_transaction_error)?;
        Ok(admitted
            .into_iter()
            .zip(publications)
            .map(|(operation, publication)| Self {
                operation,
                publication,
            })
            .collect())
    }
}

enum PreparedTransactionValidation<Rejection> {
    Append(Result<Box<CycleFreeSessionOperation>, LedgerTransactionError>),
    Reject(Rejection),
}

enum PreparedCommittedActionValidation<Rejection, Action> {
    Append {
        operations: Result<Vec<CycleFreeSessionOperation>, LedgerTransactionError>,
        action:     Action,
    },
    AlreadySatisfied(Action),
    Reject(Rejection),
}

impl<Rejection, Action> PreparedCommittedActionValidation<Rejection, Action> {
    fn prepare(
        validation: CommittedActionValidation<Rejection, Action>,
        state: &ReplayedLedgerState<'_>,
        ledger_directory: &Path,
        run: CoordinationRunId,
    ) -> Self {
        match validation {
            CommittedActionValidation::AppendWithPrerequisites {
                mut prerequisites,
                operation,
                action,
            } => {
                prerequisites.push(*operation);
                Self::Append {
                    operations: CycleFreeSessionOperation::prepare_sequence(
                        state,
                        ledger_directory,
                        run,
                        prerequisites,
                    ),
                    action,
                }
            },
            CommittedActionValidation::Append { operation, action } => Self::Append {
                operations: CycleFreeSessionOperation::prepare_sequence(
                    state,
                    ledger_directory,
                    run,
                    vec![*operation],
                ),
                action,
            },
            CommittedActionValidation::AppendChecked { operation, action } => Self::Append {
                operations: Ok(vec![CycleFreeSessionOperation::prepare(
                    ledger_directory,
                    run,
                    *operation,
                )]),
                action,
            },
            CommittedActionValidation::AlreadySatisfied(action) => Self::AlreadySatisfied(action),
            CommittedActionValidation::Reject(rejection) => Self::Reject(rejection),
        }
    }
}

/// A reconciliation whose encoding and resulting replay precede lock acquisition.
enum PreparedReconciliationValidation<Rejection, Action> {
    Apply {
        batch:                  Result<Box<PreparedReconciliationAppend>, LedgerTransactionError>,
        recoverable_operations: Vec<PreparedOperationSessionChange>,
        action:                 Action,
    },
    Reject(Rejection),
}

impl<Rejection, Action> PreparedReconciliationValidation<Rejection, Action> {
    fn prepare(
        validation: ReconciliationValidation<Rejection, Action>,
        replay: &JournalReplay,
        actor: &JournalActor,
        ledger_directory: &Path,
    ) -> Self {
        match validation {
            ReconciliationValidation::Apply {
                operations,
                cover_actors,
                recoverable_operations,
                action,
            } => Self::Apply {
                batch: PreparedReconciliationAppend::prepare(
                    replay,
                    actor,
                    operations,
                    &cover_actors,
                    ledger_directory,
                )
                .map(Box::new),
                recoverable_operations: PreparedOperationSessionChange::prepare(
                    ledger_directory,
                    actor.run,
                    recoverable_operations,
                ),
                action,
            },
            ReconciliationValidation::Reject(rejection) => Self::Reject(rejection),
        }
    }
}

/// Encoded reconciliation records and their replay bound to the source generation.
struct PreparedReconciliationAppend {
    records:         PreparedJournalAppend,
    session_mapping: PreparedSessionMappingPublication,
    replay:          JournalReplay,
}

impl PreparedReconciliationAppend {
    fn prepare(
        replay: &JournalReplay,
        actor: &JournalActor,
        operations: Vec<JournalOperation>,
        cover_actors: &HashMap<ReservationId, CoverClaimActor>,
        ledger_directory: &Path,
    ) -> Result<Self, LedgerTransactionError> {
        let mut generation = replay.generation;
        let mut events = Vec::with_capacity(operations.len());
        let ordering_graph = OnceCell::new();
        let state = ReplayedLedgerState {
            coordination_events: &replay.coordination_events,
            ordering_graph:      &ordering_graph,
            reservations:        &replay.reservations,
            generation:          replay.generation,
            journal_end_offset:  replay.end_offset,
        };
        let operations = CycleFreeOrderingOperation::check_sequence(&state, operations)
            .map_err(ordering_declaration_transaction_error)?;
        for checked in operations {
            let operation = checked.into_operation();
            generation = next_projection_generation(generation)
                .map_err(LedgerTransactionError::LedgerUnreadable)?;
            let operation_actor = match &operation {
                JournalOperation::Claim {
                    reservation_id,
                    source: ClaimSource::Cover { .. },
                    ..
                } => cover_actors.get(reservation_id).map_or_else(
                    || actor.clone(),
                    |cover| JournalActor {
                        repository: actor.repository,
                        worktree:   cover.worktree,
                        run:        cover.run,
                    },
                ),
                _ => actor.clone(),
            };
            events.push(JournalEvent::for_operation(
                operation_actor,
                generation,
                operation,
            ));
        }
        let records =
            PreparedJournalAppend::encode(&events).map_err(journal_append_transaction_error)?;
        let (replay, _) = replay
            .clone()
            .extended_over(records.records())
            .map_err(LedgerError::from)
            .map_err(LedgerTransactionError::LedgerUnreadable)?;
        Ok(Self {
            records,
            session_mapping: session::PreparedSessionMappingPublication::prepare(
                ledger_directory,
                &events,
            ),
            replay,
        })
    }
}

/// The branch checkout and run that own one engine-created cover claim.
#[derive(Clone, Copy)]
pub(crate) struct CoverClaimActor {
    pub(crate) worktree: WorktreeId,
    pub(crate) run:      CoordinationRunId,
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
    /// Exactly one validated event was appended and published.
    Appended {
        /// The durable journal event.
        event:                       Box<JournalEvent>,
        /// Whether the event's session identity consequence was published.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    /// Validation rejected the operation before any append.
    Rejected(Rejection),
}

/// The result of applying or rejecting locked reconciliation work.
pub(crate) enum LedgerReconciliationOutcome<Rejection, CommittedActionOutput> {
    /// Reconciliation committed and its repair action ran under the lock.
    Appended {
        /// The output produced by the repair action.
        output:                      CommittedActionOutput,
        /// Whether the journal's session identity consequences were published.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    /// Validation rejected reconciliation before any append or repair.
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
    /// Validation found the requested state already durable; no event was appended.
    AlreadySatisfied {
        /// The output of the locked action.
        output:                      CommittedActionOutput,
        /// No new journal identity consequence was needed.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    /// Validation rejected the operation before any append or side effect.
    Rejected(Rejection),
}

impl Ledger {
    /// Resolve the shared ledger and initialize its journal, projection, and repository policy.
    pub(crate) fn initialize(repository_root: &Path) -> Result<LedgerInitialization, LedgerError> {
        let worktree_context = WorktreeContext::discover(repository_root)?;
        let ledger = Self::locate(repository_root)?;
        fs::create_dir_all(&ledger.paths.directory)?;
        let configuration = BerthConfig::initialize(&worktree_context.configuration_lookup())?;
        let transaction = ledger.begin_initialization()?;
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
        let snapshot = git::RepositorySnapshot::discover(invocation_directory)?;
        Self::open_from_discovered_worktree(snapshot.context())
    }

    /// Attach using a worktree context already discovered from `.git` filesystem metadata.
    pub(crate) fn open_from_discovered_worktree(
        worktree_context: &WorktreeContext,
    ) -> Result<Self, LedgerError> {
        let ledger = Self::at_common_git_directory(
            worktree_context.common_git_directory(),
            worktree_context.repository_root(),
        );
        ledger.require_existing()?;
        Ok(ledger)
    }

    /// Read the clone identity that owns this ledger.
    pub(crate) fn repository_identity(&self) -> Result<RepoInstanceId, LedgerError> {
        identity::read_repo_instance_id(&self.paths.repo_instance_id)
    }

    /// Remove only the harness-session mapping selected by this process.
    pub(crate) fn remove_current_session_mapping(
        &self,
    ) -> Result<CurrentSessionMappingRemoval, LedgerError> {
        let (lock, (), prepared) = preparation::prepare_with_lock(
            &self.paths.lock,
            MUTATING_VERB_CONTENTION_TOLERANCE,
            (),
            |()| {
                session::PreparedCurrentSessionRemoval::prepare(&self.paths.directory)
                    .map_err(LedgerError::SessionIdentityStore)
            },
            |(), prepared| {
                prepared
                    .still_current(&self.paths.directory)
                    .map_err(LedgerError::from)
            },
        )?;
        let result = prepared
            .publish(&self.paths.directory)
            .map_err(LedgerError::SessionIdentityStore);
        std::mem::drop(lock);
        result
    }

    /// Read validated journal truth without git, locking, repair, or publication.
    pub(crate) fn read_for_edit_check(
        invocation_directory: &Path,
    ) -> Result<Enrollment<EditCheckLedgerSnapshot>, LedgerError> {
        let worktree_context = WorktreeContext::discover(invocation_directory)?;
        let configuration = match BerthConfig::read(&worktree_context.configuration_lookup())? {
            Enrollment::Enrolled(berth_config) => berth_config,
            Enrollment::Unconfigured {
                expected_configuration_path,
            } => {
                return Ok(Enrollment::Unconfigured {
                    expected_configuration_path,
                });
            },
        };
        let ledger = Self::at_common_git_directory(
            worktree_context.common_git_directory(),
            worktree_context.repository_root(),
        );
        let reservations = ledger.read_validated_journal()?.into_reservations();
        Ok(Enrollment::Enrolled(EditCheckLedgerSnapshot {
            reservations,
            worktree_context,
            configuration,
        }))
    }

    /// Read a movement stamp with one journal metadata lookup and one small projection read.
    ///
    /// Cost is independent of journal length: no records are read or replayed, no lock is
    /// acquired, and nothing is published. A concurrent append or projection publication can
    /// cause an extra refresh; the ensuing reconciliation validates the actual journal.
    /// An undecodable projection counts as movement so reconciliation can rebuild it.
    pub(crate) fn current_journal_position(&self) -> Result<JournalPosition, LedgerError> {
        let byte_offset = JournalByteOffset::from(fs::metadata(&self.paths.journal)?.len());
        projection::read_position(&self.paths.projection, byte_offset)
            .map_err(LedgerError::Projection)
    }

    /// Read validated journal truth without holding the mutation lock.
    pub(crate) fn read_validated_journal(&self) -> Result<ValidatedJournal, LedgerError> {
        self.require_existing()?;
        let repo_instance_id = identity::read_repo_instance_id(&self.paths.repo_instance_id)?;
        retry_concurrent_projection_publication(|| {
            let replay = Journal::replay_read_only_from_checkpoint(
                &self.paths.journal,
                &ReplayCheckpoint::new(&self.paths.replay_checkpoint, repo_instance_id),
            )?;
            identity::validate_journal_repository(repo_instance_id, &replay)?;
            projection::read_validated(&self.paths.projection, repo_instance_id, &replay)?;
            Ok(ValidatedJournal {
                coordination_events: replay.coordination_events,
                reservations:        replay.reservations,
            })
        })
    }

    /// Prepare against journal truth, then append only if its generation remains current.
    pub(crate) fn transact<Rejection>(
        &self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        validate: impl FnMut(ReplayedLedgerState<'_>) -> TransactionValidation<Rejection>,
    ) -> Result<LedgerTransactionOutcome<Rejection>, LedgerTransactionError> {
        self.transact_tolerating(
            MUTATING_VERB_CONTENTION_TOLERANCE,
            worktree_id,
            coordination_run_id,
            validate,
        )
    }

    /// Attempt one bypass-audit append without ever waiting behind a live lock holder.
    pub(crate) fn try_transact<Rejection>(
        &self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        validate: impl FnMut(ReplayedLedgerState<'_>) -> TransactionValidation<Rejection>,
    ) -> Result<LedgerTransactionOutcome<Rejection>, LedgerTransactionError> {
        self.transact_tolerating(Duration::ZERO, worktree_id, coordination_run_id, validate)
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
        validate: impl FnMut(
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
        mut validate: impl FnMut(
            ReplayedLedgerState<'_>,
        ) -> CommittedActionValidation<Rejection, CommittedAction>,
        commit_action: impl FnOnce(
            CommittedAction,
        ) -> Result<CommittedActionOutput, CommittedActionError>,
        consume_locked_outcome: impl FnOnce(
            LedgerCommittedActionOutcome<Rejection, CommittedActionOutput>,
        ) -> LockedOutcome,
    ) -> Result<LockedOutcome, LedgerCommittedActionError<CommittedActionError>> {
        let (transaction, validation) = self
            .prepare_mutation(MUTATING_VERB_CONTENTION_TOLERANCE, |state| {
                PreparedCommittedActionValidation::prepare(
                    validate(state),
                    &state,
                    &self.paths.directory,
                    coordination_run_id,
                )
            })
            .map_err(LedgerTransactionError::from)
            .map_err(LedgerCommittedActionError::Transaction)?;
        self.conclude(transaction, |transaction| {
            let mut session_mapping_publication = SessionIdentityMappingPublication::Published;
            let (operations, action) = match validation {
                PreparedCommittedActionValidation::Append { operations, action } => {
                    (operations, action)
                },
                PreparedCommittedActionValidation::AlreadySatisfied(action) => {
                    transaction
                        .publish_if_rebuild_required(&self.paths)
                        .map_err(LedgerTransactionError::LedgerUnreadable)
                        .map_err(LedgerCommittedActionError::Transaction)?;
                    let output =
                        commit_action(action).map_err(LedgerCommittedActionError::Action)?;
                    return Ok(consume_locked_outcome(
                        LedgerCommittedActionOutcome::AlreadySatisfied {
                            output,
                            session_mapping_publication:
                                SessionIdentityMappingPublication::LeftUnmapped,
                        },
                    ));
                },
                PreparedCommittedActionValidation::Reject(rejection) => {
                    transaction
                        .publish_if_rebuild_required(&self.paths)
                        .map_err(LedgerTransactionError::LedgerUnreadable)
                        .map_err(LedgerCommittedActionError::Transaction)?;
                    return Ok(consume_locked_outcome(
                        LedgerCommittedActionOutcome::Rejected(rejection),
                    ));
                },
            };
            for operation in operations.map_err(LedgerCommittedActionError::Transaction)? {
                let appended = transaction
                    .append(worktree_id, coordination_run_id, operation)
                    .map_err(LedgerCommittedActionError::Transaction)?;
                session_mapping_publication =
                    session_mapping_publication.merge(appended.session_mapping_publication);
            }
            let action_output = commit_action(action);
            transaction
                .publish(&self.paths)
                .map_err(LedgerTransactionError::LedgerUnreadable)
                .map_err(LedgerCommittedActionError::Transaction)?;
            let outcome = action_output.map_or_else(
                |error| Err(LedgerCommittedActionError::Action(error)),
                |output| {
                    Ok(LedgerCommittedActionOutcome::Appended {
                        output,
                        session_mapping_publication,
                    })
                },
            )?;
            Ok(consume_locked_outcome(outcome))
        })
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
        validate: impl FnMut(
            ReplayedLedgerState<'_>,
        ) -> ReconciliationValidation<Rejection, CommittedAction>,
        commit_action: impl FnOnce(
            CommittedAction,
            &ReplayedLedgerState<'_>,
            &RecoverableReconciliationAppendFailures,
        ) -> Result<CommittedActionOutput, CommittedActionError>,
    ) -> Result<
        LedgerReconciliationOutcome<Rejection, CommittedActionOutput>,
        LedgerCommittedActionError<CommittedActionError>,
    > {
        self.transact_reconciliation_then(
            worktree_id,
            coordination_run_id,
            validate,
            commit_action,
            |output, _, _| Ok(output),
        )
    }

    /// Run ordered repairs under the lock and build the result after releasing it.
    pub(crate) fn transact_reconciliation_then<
        Rejection,
        CommittedAction,
        CommittedActionOutput,
        CommittedActionError,
        Output,
    >(
        &self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        mut validate: impl FnMut(
            ReplayedLedgerState<'_>,
        ) -> ReconciliationValidation<Rejection, CommittedAction>,
        commit_action: impl FnOnce(
            CommittedAction,
            &ReplayedLedgerState<'_>,
            &RecoverableReconciliationAppendFailures,
        ) -> Result<CommittedActionOutput, CommittedActionError>,
        finish: impl FnOnce(
            CommittedActionOutput,
            &ReplayedLedgerState<'_>,
            &RecoverableReconciliationAppendFailures,
        ) -> Result<Output, CommittedActionError>,
    ) -> Result<
        LedgerReconciliationOutcome<Rejection, Output>,
        LedgerCommittedActionError<CommittedActionError>,
    > {
        let (transaction, validation) = self
            .prepare_mutation_from_replay(
                MUTATING_VERB_CONTENTION_TOLERANCE,
                |replay, repository| {
                    PreparedReconciliationValidation::prepare(
                        validate(ReplayedLedgerState {
                            coordination_events: &replay.coordination_events,
                            ordering_graph:      &OnceCell::new(),
                            reservations:        &replay.reservations,
                            generation:          replay.generation,
                            journal_end_offset:  replay.end_offset,
                        }),
                        replay,
                        &JournalActor {
                            repository,
                            worktree: worktree_id,
                            run: coordination_run_id,
                        },
                        &self.paths.directory,
                    )
                },
            )
            .map_err(LedgerTransactionError::from)
            .map_err(LedgerCommittedActionError::Transaction)?;
        self.conclude_then(
            transaction,
            |transaction| match validation {
                PreparedReconciliationValidation::Apply {
                    batch,
                    recoverable_operations,
                    action,
                } => {
                    let batch_publication = transaction
                        .append_prepared_reconciliation(
                            *batch.map_err(LedgerCommittedActionError::Transaction)?,
                        )
                        .map_err(LedgerCommittedActionError::Transaction)?;
                    let (recoverable_publication, recoverable_failures) = transaction
                        .append_recoverable_operations(
                            worktree_id,
                            coordination_run_id,
                            recoverable_operations,
                        )
                        .map_err(LedgerCommittedActionError::Transaction)?;
                    let session_mapping_publication =
                        batch_publication.merge(recoverable_publication);
                    let committed_state = ReplayedLedgerState {
                        coordination_events: &transaction.replay.coordination_events,
                        ordering_graph:      &OnceCell::new(),
                        reservations:        &transaction.replay.reservations,
                        generation:          transaction.replay.generation,
                        journal_end_offset:  transaction.replay.end_offset,
                    };
                    let action_output =
                        commit_action(action, &committed_state, &recoverable_failures);
                    transaction
                        .publish(&self.paths)
                        .map_err(LedgerTransactionError::LedgerUnreadable)
                        .map_err(LedgerCommittedActionError::Transaction)?;
                    action_output.map_or_else(
                        |error| Err(LedgerCommittedActionError::Action(error)),
                        |output| {
                            Ok(LedgerReconciliationOutcome::Appended {
                                output: (output, recoverable_failures),
                                session_mapping_publication,
                            })
                        },
                    )
                },
                PreparedReconciliationValidation::Reject(rejection) => {
                    transaction
                        .publish_if_rebuild_required(&self.paths)
                        .map_err(LedgerTransactionError::LedgerUnreadable)
                        .map_err(LedgerCommittedActionError::Transaction)?;
                    Ok(LedgerReconciliationOutcome::Rejected(rejection))
                },
            },
            |outcome, state| match outcome? {
                LedgerReconciliationOutcome::Appended {
                    output: (output, failures),
                    session_mapping_publication,
                } => {
                    let output = finish(output, &state, &failures)
                        .map_err(LedgerCommittedActionError::Action)?;
                    Ok(LedgerReconciliationOutcome::Appended {
                        output,
                        session_mapping_publication,
                    })
                },
                LedgerReconciliationOutcome::Rejected(rejection) => {
                    Ok(LedgerReconciliationOutcome::Rejected(rejection))
                },
            },
        )
    }
    /// Remove and rebuild only the disposable projection from journal truth.
    ///
    /// Also removes the replay checkpoint, so the journal replays from byte 0, and the journal
    /// compaction refusal, so the next transaction past the threshold compacts again.
    pub(crate) fn repair_projection(repository_root: &Path) -> Result<(), LedgerError> {
        let ledger = Self::locate(repository_root)?;
        ledger.require_existing()?;
        let repo_instance_id = ledger.repository_identity()?;
        let (lock, (), prepared) = preparation::prepare_with_lock(
            &ledger.paths.lock,
            MUTATING_VERB_CONTENTION_TOLERANCE,
            (),
            |()| {
                let before = fs::metadata(&ledger.paths.journal)?;
                let replay = Journal::replay_read_only(&ledger.paths.journal)?;
                identity::validate_journal_repository(repo_instance_id, &replay)?;
                Ok::<_, LedgerError>((
                    JournalPreparation {
                        replay,
                        device: before.dev(),
                        inode: before.ino(),
                    },
                    before.len(),
                ))
            },
            |(), (prepared, length)| {
                Ok(prepared.still_current(&ledger.paths.journal)?
                    && fs::metadata(&ledger.paths.journal)?.len() == *length)
            },
        )?;
        let (prepared, _) = prepared;
        replay_checkpoint::remove(&ledger.paths.replay_checkpoint)?;
        compaction::remove_refusal(&ledger.paths.compaction_refusal)?;
        let result = Projection::from_replay(repo_instance_id, &prepared.replay)
            .publish(&ledger.paths.directory, &ledger.paths.projection)
            .map_err(LedgerError::from);
        std::mem::drop(lock);
        result
    }

    /// Append the retirement of the first live cycle-closing edge, preserving all other facts.
    pub(crate) fn repair_ordering_cycle(
        repository_root: &Path,
    ) -> Result<OrderingCycleRepair, LedgerTransactionError> {
        let ledger = Self::locate(repository_root)?;
        ledger.require_existing()?;
        let context = WorktreeContext::discover(repository_root)?;
        let identity = identity::resolve_identity(&context)?;
        ordering_repair::repair(
            &ledger.paths,
            JournalActor {
                repository: ledger.repository_identity()?,
                worktree:   identity.worktree_id,
                run:        identity.coordination_run_id,
            },
        )
    }

    /// Discard journal truth only after the caller has confirmed pending orders were reviewed.
    pub(crate) fn reinitialize_after_review(
        repository_root: &Path,
    ) -> Result<LedgerReinitialization, LedgerError> {
        let ledger = Self::locate(repository_root)?;
        if !ledger.paths.directory.is_dir() || !ledger.paths.repo_instance_id.is_file() {
            return Err(LedgerError::NotInitialized);
        }
        let gate_policy = GateTargetPolicy::read(&ledger.paths.repository_root)?;
        let repo_instance_id = ledger.repository_identity()?;
        let (lock, (), (_, result)) = preparation::prepare_with_lock(
            &ledger.paths.lock,
            MUTATING_VERB_CONTENTION_TOLERANCE,
            (),
            |()| {
                let before = fs::metadata(&ledger.paths.journal).ok();
                let journal_bytes = match fs::read(&ledger.paths.journal) {
                    Ok(bytes) => bytes,
                    Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
                    Err(error) => return Err(LedgerError::from(error)),
                };
                let discarded_bytes = u64::try_from(journal_bytes.len())
                    .map_err(|_| LedgerError::JournalSizeUnrepresentable)?;
                let discarded_complete_records =
                    u64::try_from(journal_bytes.split(|byte| *byte == b'\n').count() - 1)
                        .map_err(|_| LedgerError::JournalSizeUnrepresentable)?;
                Ok((
                    before,
                    LedgerReinitialization {
                        discarded_bytes,
                        discarded_complete_records,
                    },
                ))
            },
            |(), (before, _)| {
                let after = fs::metadata(&ledger.paths.journal).ok();
                let stamp = |metadata: &Metadata| {
                    (
                        metadata.dev(),
                        metadata.ino(),
                        metadata.len(),
                        metadata.mtime(),
                        metadata.mtime_nsec(),
                    )
                };
                Ok(before.as_ref().map(stamp) == after.as_ref().map(stamp))
            },
        )?;
        replay_checkpoint::remove(&ledger.paths.replay_checkpoint)?;
        let empty_path = ledger.paths.directory.join("journal.reinitialize.tmp");
        let empty = fs::File::create(&empty_path)?;
        empty.sync_all()?;
        fs::rename(empty_path, &ledger.paths.journal)?;
        let replay = JournalReplay::empty();
        Projection::from_replay(repo_instance_id, &replay)
            .publish(&ledger.paths.directory, &ledger.paths.projection)?;
        gate_policy.publish(&ledger.paths.directory, &replay)?;
        fs::File::open(&ledger.paths.directory)?.sync_all()?;
        std::mem::drop(lock);
        Ok(result)
    }

    fn locate(repository_root: &Path) -> Result<Self, LedgerError> {
        let snapshot = git::RepositorySnapshot::discover(repository_root)?;
        Ok(Self::at_common_git_directory(
            snapshot.common_directory(),
            snapshot.repository_root(),
        ))
    }

    fn at_common_git_directory(common_git_directory: &Path, repository_root: &Path) -> Self {
        let directory = common_git_directory.join(LEDGER_DIRECTORY_NAME);
        Self {
            paths:                      LedgerPaths {
                compaction_refusal: directory.join(JOURNAL_COMPACTION_REFUSAL_FILE_NAME),
                journal: directory.join(JOURNAL_FILE_NAME),
                lock: directory.join(LOCK_FILE_NAME),
                projection: directory.join(PROJECTION_FILE_NAME),
                replay_checkpoint: directory.join(REPLAY_CHECKPOINT_FILE_NAME),
                repo_instance_id: directory.join(REPO_INSTANCE_ID_FILE_NAME),
                repository_root: repository_root.to_path_buf(),
                directory,
            },
            compaction_threshold_bytes: JOURNAL_COMPACTION_THRESHOLD_BYTES,
        }
    }

    pub(super) fn require_existing(&self) -> Result<(), LedgerError> {
        if !self.paths.directory.is_dir()
            || !self.paths.journal.is_file()
            || !self.paths.repo_instance_id.is_file()
        {
            return Err(LedgerError::NotInitialized);
        }
        Ok(())
    }

    /// [`Self::transact`], waiting at most `contention_tolerance` for the mutation lock.
    fn transact_tolerating<Rejection>(
        &self,
        contention_tolerance: Duration,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        mut validate: impl FnMut(ReplayedLedgerState<'_>) -> TransactionValidation<Rejection>,
    ) -> Result<LedgerTransactionOutcome<Rejection>, LedgerTransactionError> {
        let (transaction, validation) = self
            .prepare_mutation(contention_tolerance, |state| match validate(state) {
                TransactionValidation::Append(operation) => PreparedTransactionValidation::Append(
                    CycleFreeSessionOperation::prepare_sequence(
                        &state,
                        &self.paths.directory,
                        coordination_run_id,
                        vec![*operation],
                    )
                    .map(|mut operations| Box::new(operations.remove(0))),
                ),
                TransactionValidation::AppendChecked(operation) => {
                    PreparedTransactionValidation::Append(Ok(Box::new(
                        CycleFreeSessionOperation::prepare(
                            &self.paths.directory,
                            coordination_run_id,
                            *operation,
                        ),
                    )))
                },
                TransactionValidation::Reject(rejection) => {
                    PreparedTransactionValidation::Reject(rejection)
                },
            })
            .map_err(LedgerTransactionError::from_ledger_error)?;
        self.conclude(transaction, |transaction| match validation {
            PreparedTransactionValidation::Append(operation) => {
                let journal_append =
                    transaction.append(worktree_id, coordination_run_id, *operation?)?;
                transaction
                    .publish(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)?;
                Ok(LedgerTransactionOutcome::Appended {
                    event:                       Box::new(journal_append.event),
                    session_mapping_publication: journal_append.session_mapping_publication,
                })
            },
            PreparedTransactionValidation::Reject(rejection) => {
                transaction
                    .publish_if_rebuild_required(&self.paths)
                    .map_err(LedgerTransactionError::LedgerUnreadable)?;
                Ok(LedgerTransactionOutcome::Rejected(rejection))
            },
        })
    }

    /// Run `body` in `transaction`, then release its mutation lock and compact the journal when
    /// the replay holds enough superseded records or expired finished history.
    fn conclude<Concluded>(
        &self,
        transaction: LedgerTransaction,
        body: impl FnOnce(&mut LedgerTransaction) -> Concluded,
    ) -> Concluded {
        self.conclude_then(transaction, body, |concluded, _| concluded)
    }

    fn conclude_then<Concluded, Output>(
        &self,
        mut transaction: LedgerTransaction,
        body: impl FnOnce(&mut LedgerTransaction) -> Concluded,
        finish: impl FnOnce(Concluded, ReplayedLedgerState<'_>) -> Output,
    ) -> Output {
        let concluded = body(&mut transaction);
        let droppable_bytes = transaction.replay.superseded.droppable_bytes;
        let history_due = transaction
            .replay
            .finished_history
            .is_due(&RecordedAt::now());
        let LedgerTransaction {
            _lock: lock,
            replay,
            retired_replays,
            ..
        } = transaction;
        std::mem::drop(lock);
        std::mem::drop(retired_replays);
        let output = finish(
            concluded,
            ReplayedLedgerState {
                coordination_events: &replay.coordination_events,
                ordering_graph:      &OnceCell::new(),
                reservations:        &replay.reservations,
                generation:          replay.generation,
                journal_end_offset:  replay.end_offset,
            },
        );
        if history_due {
            self.compact_expired_history();
        } else {
            self.compact_if_due(droppable_bytes);
        }
        output
    }

    /// Prepare optimistically against journal truth, retaining new tails across retries.
    /// Sustained contention falls back to preparing while holding the mutation lock.
    /// Validators must not acquire the mutation lock themselves.
    fn prepare_mutation<Decision>(
        &self,
        contention_tolerance: Duration,
        mut validate: impl FnMut(ReplayedLedgerState<'_>) -> Decision,
    ) -> Result<(LedgerTransaction, Decision), LedgerError> {
        self.prepare_mutation_from_replay(contention_tolerance, |replay, _| {
            validate(ReplayedLedgerState {
                coordination_events: &replay.coordination_events,
                ordering_graph:      &OnceCell::new(),
                reservations:        &replay.reservations,
                generation:          replay.generation,
                journal_end_offset:  replay.end_offset,
            })
        })
    }

    fn prepare_mutation_from_replay<Decision>(
        &self,
        contention_tolerance: Duration,
        mut validate: impl FnMut(&JournalReplay, RepoInstanceId) -> Decision,
    ) -> Result<(LedgerTransaction, Decision), LedgerError> {
        self.require_existing()?;
        let repo_instance_id = self.repository_identity()?;
        let gate_policy = GateTargetPolicy::read(&self.paths.repository_root)?;
        let journal_preparation = self.prepare_journal(repo_instance_id)?;
        let journal = Journal::open_existing(&self.paths.journal)?;
        let (lock, journal_preparation, (_, decision)) = preparation::prepare_with_lock(
            &self.paths.lock,
            contention_tolerance,
            journal_preparation,
            |preparation| {
                // Keep tails already read so unrelated writers cannot force a whole replay.
                if preparation.still_current(&self.paths.journal)? {
                    Journal::advance_read_only_tail(
                        &self.paths.journal,
                        &mut preparation.replay,
                        JournalReplaySource::new(preparation.device, preparation.inode),
                    )?;
                }
                if !preparation.still_current(&self.paths.journal)? {
                    *preparation = self.prepare_journal(repo_instance_id)?;
                }
                let mappings = SessionMappingVersion::read(&self.paths.directory)?;
                let decision = validate(&preparation.replay, repo_instance_id);
                Ok::<_, LedgerError>((mappings, decision))
            },
            |preparation, (mappings, _)| {
                if !preparation.still_current(&self.paths.journal)? {
                    return Ok(false);
                }
                let generation = preparation.replay.generation;
                journal.advance_repairing_tail(&mut preparation.replay)?;
                Ok(preparation.replay.generation == generation
                    && *mappings == SessionMappingVersion::read(&self.paths.directory)?)
            },
        )?;
        let mut replay = journal_preparation.replay;
        // Locked preparation also repairs an incomplete tail before publication.
        journal.advance_repairing_tail(&mut replay)?;
        let projection_synchronization =
            projection::read_validated(&self.paths.projection, repo_instance_id, &replay)?;
        Ok((
            LedgerTransaction {
                _lock: lock,
                journal,
                journal_initialization: InitializationState::Existing,
                ledger_directory: self.paths.directory.clone(),
                gate_policy,
                projection_synchronization,
                replay,
                repo_instance_id,
                retired_replays: Vec::new(),
            },
            decision,
        ))
    }

    fn prepare_journal(
        &self,
        repo_instance_id: RepoInstanceId,
    ) -> Result<JournalPreparation, LedgerError> {
        retry_concurrent_projection_publication(|| {
            let before = fs::metadata(&self.paths.journal)?;
            let replay = Journal::replay_read_only_from_checkpoint(
                &self.paths.journal,
                &ReplayCheckpoint::new(&self.paths.replay_checkpoint, repo_instance_id),
            )?;
            let after = fs::metadata(&self.paths.journal)?;
            if before.dev() != after.dev() || before.ino() != after.ino() {
                return Err(LedgerError::Projection(ProjectionError::CacheAhead));
            }
            identity::validate_journal_repository(repo_instance_id, &replay)?;
            Ok(JournalPreparation {
                replay,
                device: after.dev(),
                inode: after.ino(),
            })
        })
    }

    fn begin_initialization(&self) -> Result<LedgerTransaction, LedgerError> {
        let gate_policy = GateTargetPolicy::read(&self.paths.repository_root)?;
        let lock = MutationLock::acquire(&self.paths.lock, MUTATING_VERB_CONTENTION_TOLERANCE)?;
        let repo_instance_id =
            identity::read_or_create_repo_instance_id(&self.paths.repo_instance_id)?;
        let (journal, journal_initialization) = Journal::open_or_create(&self.paths.journal)?;
        if journal_initialization == InitializationState::Existing {
            std::mem::drop(lock);
            return self
                .prepare_mutation(MUTATING_VERB_CONTENTION_TOLERANCE, |_| ())
                .map(|(transaction, ())| transaction);
        }
        Ok(LedgerTransaction {
            _lock: lock,
            journal,
            journal_initialization,
            ledger_directory: self.paths.directory.clone(),
            gate_policy,
            projection_synchronization: ProjectionSynchronization::RebuildRequired,
            replay: JournalReplay::empty(),
            repo_instance_id,
            retired_replays: Vec::new(),
        })
    }
}

impl LedgerTransaction {
    fn append_prepared_reconciliation(
        &mut self,
        batch: PreparedReconciliationAppend,
    ) -> Result<SessionIdentityMappingPublication, LedgerTransactionError> {
        self.journal
            .append_prepared(&batch.records)
            .map_err(journal_append_transaction_error)?;
        self.retired_replays
            .push(std::mem::replace(&mut self.replay, batch.replay));
        Ok(batch.session_mapping.publish(&self.ledger_directory))
    }

    fn admit(
        &self,
        prepared: PreparedOperationSessionChange,
    ) -> Result<CycleFreeSessionOperation, LedgerTransactionError> {
        let state = ReplayedLedgerState {
            coordination_events: &self.replay.coordination_events,
            ordering_graph:      &OnceCell::new(),
            reservations:        &self.replay.reservations,
            generation:          self.replay.generation,
            journal_end_offset:  self.replay.end_offset,
        };
        let operation = CycleFreeOrderingOperation::check(&state, prepared.operation)
            .map_err(ordering_declaration_transaction_error)?;
        Ok(CycleFreeSessionOperation {
            operation,
            publication: prepared.publication,
        })
    }

    fn append(
        &mut self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        prepared: CycleFreeSessionOperation,
    ) -> Result<JournalAppend, LedgerTransactionError> {
        let CycleFreeSessionOperation {
            operation,
            publication,
        } = prepared;
        let next_generation = next_projection_generation(self.replay.generation)
            .map_err(LedgerTransactionError::LedgerUnreadable)?;
        let event = JournalEvent::for_operation(
            JournalActor {
                repository: self.repo_instance_id,
                worktree:   worktree_id,
                run:        coordination_run_id,
            },
            next_generation,
            operation.into_operation(),
        );
        self.journal
            .append(&event)
            .map_err(journal_append_transaction_error)?;
        self.journal
            .advance_repairing_tail(&mut self.replay)
            .map_err(LedgerError::from)
            .map_err(LedgerTransactionError::LedgerUnreadable)?;
        let session_mapping_publication = publication.publish(&self.ledger_directory);
        Ok(JournalAppend {
            event,
            session_mapping_publication,
        })
    }

    /// Append marker imports individually, retaining failures for reconciliation's report.
    fn append_recoverable_operations(
        &mut self,
        worktree_id: WorktreeId,
        coordination_run_id: CoordinationRunId,
        operations: Vec<PreparedOperationSessionChange>,
    ) -> Result<
        (
            SessionIdentityMappingPublication,
            RecoverableReconciliationAppendFailures,
        ),
        LedgerTransactionError,
    > {
        let mut publication = SessionIdentityMappingPublication::Published;
        let mut failures = RecoverableReconciliationAppendFailures {
            operations: Vec::new(),
        };
        for operation in operations {
            let retained_operation = operation.operation.clone();
            if let Ok(journal_append) = self
                .admit(operation)
                .and_then(|checked| self.append(worktree_id, coordination_run_id, checked))
            {
                publication = publication.merge(journal_append.session_mapping_publication);
            } else {
                self.recover_after_recoverable_append_failure()?;
                failures.operations.push(retained_operation);
            }
        }
        Ok((publication, failures))
    }

    fn recover_after_recoverable_append_failure(&mut self) -> Result<(), LedgerTransactionError> {
        self.journal
            .advance_repairing_tail(&mut self.replay)
            .map_err(LedgerError::from)
            .map_err(LedgerTransactionError::LedgerUnreadable)?;
        identity::validate_journal_repository(self.repo_instance_id, &self.replay)
            .map_err(LedgerTransactionError::LedgerUnreadable)
    }

    fn publish(&self, paths: &LedgerPaths) -> Result<(), LedgerError> {
        Projection::from_replay(self.repo_instance_id, &self.replay)
            .publish(&paths.directory, &paths.projection)?;
        self.gate_policy
            .publish(&self.ledger_directory, &self.replay)?;
        Ok(())
    }
}

/// Declaration verbs return correctable cycle refusals before reaching this safety boundary.
/// An unchecked internal proposal still cannot append edges that make replay unreadable.
fn ordering_declaration_transaction_error(
    error: OrderingDeclarationError,
) -> LedgerTransactionError {
    let replay_error = match error {
        OrderingDeclarationError::Cycle(_) => EdgeReplayError::Cycle,
        OrderingDeclarationError::Replay(error) => error,
    };
    LedgerTransactionError::LedgerUnreadable(LedgerError::OrderingReplay(replay_error))
}

/// Repository policy resolved before the mutation lock is acquired.
enum GateTargetPolicy {
    Unconfigured,
    Enrolled(IntegrationTarget),
}

impl GateTargetPolicy {
    fn read(repository_root: &Path) -> Result<Self, LedgerError> {
        let worktree_context = WorktreeContext::discover(repository_root)?;
        let main_repository_root = if worktree_context.common_git_directory().file_name()
            == Some(std::ffi::OsStr::new(".git"))
        {
            worktree_context
                .common_git_directory()
                .parent()
                .unwrap_or(repository_root)
        } else {
            repository_root
        };
        let configuration = BerthConfig::read(&ConfigurationLookup::Own {
            repository_root: main_repository_root,
        })?;
        let Enrollment::Enrolled(configuration) = configuration else {
            return Ok(Self::Unconfigured);
        };
        let Ok(trunk) = configuration.repository_trunk() else {
            return Ok(Self::Unconfigured);
        };
        Ok(Self::Enrolled(trunk))
    }

    fn publish(&self, ledger_directory: &Path, replay: &JournalReplay) -> Result<(), LedgerError> {
        let Self::Enrolled(trunk) = self else {
            return Ok(());
        };
        let Ok(reservations) = &replay.reservations else {
            return Ok(());
        };
        let targets = reservations
            .iter()
            .filter(|reservation| {
                !matches!(
                    reservation.lifecycle(),
                    ReservationLifecycle::Released { .. }
                )
            })
            .filter_map(|reservation| match reservation.target() {
                RecordedTarget::Recorded { target, .. } if target != trunk => {
                    Some(target.reference().as_str().to_owned())
                },
                RecordedTarget::Recorded { .. } | RecordedTarget::Unrecorded => None,
            })
            .collect::<BTreeSet<_>>();
        let contents = targets
            .into_iter()
            .fold(String::new(), |mut contents, target| {
                contents.push_str(&target);
                contents.push('\n');
                contents
            });
        let path = ledger_directory.join(GATE_TARGETS_FILE_NAME);
        match fs::read_to_string(&path) {
            Ok(existing) if existing == contents => return Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound && contents.is_empty() => {
                return Ok(());
            },
            Ok(_) | Err(_) => {},
        }
        let temporary_path = ledger_directory.join(GATE_TARGETS_TEMPORARY_FILE_NAME);
        let mut temporary_file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary_path)?;
        temporary_file.write_all(contents.as_bytes())?;
        temporary_file.sync_all()?;
        fs::rename(temporary_path, path)?;
        fs::File::open(ledger_directory)?.sync_all()?;
        Ok(())
    }
}

impl LedgerTransaction {
    fn publish_if_rebuild_required(&self, paths: &LedgerPaths) -> Result<(), LedgerError> {
        match self.projection_synchronization {
            ProjectionSynchronization::Current => Ok(()),
            ProjectionSynchronization::RebuildRequired => self.publish(paths),
        }
    }
}

pub(super) fn journal_append_transaction_error(
    error: JournalAppendError,
) -> LedgerTransactionError {
    match error {
        JournalAppendError::RecordTooLarge {
            bytes,
            ceiling: RecordCeiling::Declared,
        } => {
            LedgerTransactionError::CorrectableInput(CorrectableTransactionInput::RecordTooLarge {
                bytes,
                maximum_bytes: MAXIMUM_JOURNAL_RECORD_BYTES,
            })
        },
        JournalAppendError::RecordTooLarge {
            bytes,
            ceiling: ceiling @ RecordCeiling::Derived,
        } => LedgerTransactionError::DerivedRecordTooLarge {
            bytes,
            maximum_bytes: ceiling.maximum_bytes(),
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
    retired_replays:            Vec<JournalReplay>,
    _lock:                      MutationLock,
    journal:                    Journal,
    journal_initialization:     InitializationState,
    ledger_directory:           PathBuf,
    gate_policy:                GateTargetPolicy,
    projection_synchronization: ProjectionSynchronization,
    replay:                     JournalReplay,
    repo_instance_id:           RepoInstanceId,
}

struct JournalAppend {
    event:                       JournalEvent,
    session_mapping_publication: SessionIdentityMappingPublication,
}

pub(super) struct LedgerPaths {
    pub(super) compaction_refusal: PathBuf,
    pub(super) directory:          PathBuf,
    pub(super) journal:            PathBuf,
    pub(super) projection:         PathBuf,
    pub(super) replay_checkpoint:  PathBuf,
    pub(super) lock:               PathBuf,
    pub(super) repo_instance_id:   PathBuf,
    repository_root:               PathBuf,
}

/// A projection published after a lock-free journal read requires a fresh pair of reads.
fn retry_concurrent_projection_publication<Read>(
    mut read: impl FnMut() -> Result<Read, LedgerError>,
) -> Result<Read, LedgerError> {
    let mut result = read();
    for _ in 1..3 {
        if !matches!(
            result,
            Err(LedgerError::Projection(ProjectionError::CacheAhead))
        ) {
            break;
        }
        result = read();
    }
    result
}

pub(super) fn next_projection_generation(
    current_generation: ProjectionGeneration,
) -> Result<ProjectionGeneration, LedgerError> {
    let current_generation: u64 = current_generation.into();
    current_generation
        .checked_add(1)
        .map(ProjectionGeneration::from)
        .ok_or(LedgerError::ProjectionGenerationExhausted)
}

#[cfg(test)]
impl Ledger {
    /// Append historical facts an older cargo-berth accepted, without current declaration checks.
    pub(crate) fn append_legacy_journal_operation(
        &self,
        actor: JournalActor,
        operation: JournalOperation,
    ) -> Result<(), LedgerTransactionError> {
        let lock = MutationLock::acquire(&self.paths.lock, MUTATING_VERB_CONTENTION_TOLERANCE)
            .map_err(LedgerError::from)?;
        let journal = Journal::open_existing(&self.paths.journal).map_err(LedgerError::from)?;
        let replay = Journal::replay_read_only(&self.paths.journal).map_err(LedgerError::from)?;
        identity::validate_journal_repository(actor.repository, &replay)?;
        let event = JournalEvent::for_operation(
            actor,
            next_projection_generation(replay.generation)?,
            operation,
        );
        journal
            .append(&event)
            .map_err(journal_append_transaction_error)?;
        replay_checkpoint::remove(&self.paths.replay_checkpoint).map_err(LedgerError::from)?;
        std::mem::drop(lock);
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::fs::TryLockError;
    use std::path::Path;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::thread;
    use std::thread::JoinHandle;

    use cargo_berth_test_support::ChildDeadlineExt as _;
    use serde_json::Value;

    use super::CommittedActionValidation;
    use super::CorrectableTransactionInput;
    use super::JournalEvent;
    use super::JournalOperation;
    use super::Ledger;
    use super::LedgerCommittedActionOutcome;
    use super::LedgerError;
    use super::LedgerReconciliationOutcome;
    use super::LedgerTransactionError;
    use super::LedgerTransactionOutcome;
    use super::MAXIMUM_JOURNAL_RECORD_BYTES;
    use super::TransactionValidation;
    use crate::drift::WorkingTreeFingerprint;
    use crate::ids::CoordinationRunId;
    use crate::ids::GitObjectId;
    use crate::ids::RepoInstanceId;
    use crate::ids::ReservationId;
    use crate::ids::ReservationScopePath;
    use crate::ids::WorktreeId;
    use crate::ledger::BypassedAction;
    use crate::ledger::ForcedIntegrationReason;
    use crate::ledger::ObservedReservation;
    use crate::ledger::ReservationScope;
    use crate::ledger::ScopeKind;
    use crate::ledger::constants::MAXIMUM_DERIVED_JOURNAL_RECORD_BYTES;
    use crate::ledger::journal;
    use crate::ledger::journal::SkippedIntegrationHoldSet;
    use crate::ledger::projection::ProjectionError;
    use crate::ledger::test_support;
    use crate::reservation::HolderMergeExtentCoverage;
    use crate::reservation::MergeExtent;
    use crate::reservation::MergeExtentKey;
    use crate::reservation::ReleaseDisposition;
    use crate::reservation::ReservationRunStatus;

    #[test]
    fn mutation_preparation_proceeds_while_another_process_holds_the_lock()
    -> Result<(), Box<dyn std::error::Error>> {
        const CHILD_ROOT: &str = "CARGO_BERTH_TEST_PREPARATION_ROOT";
        const TEST: &str = "ledger::handle::tests::mutation_preparation_proceeds_while_another_process_holds_the_lock";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let root = std::path::Path::new(&root);
            let ledger = Ledger::open(root)?;
            ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
                fs::write(root.join("prepared"), b"prepared")
                    .expect("preparation signal should write");
                TransactionValidation::<()>::Append(Box::new(renewal_operation()))
            })?;
            return Ok(());
        }
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let holder = fs::File::options()
            .read(true)
            .write(true)
            .open(&ledger.paths.lock)?;
        holder.lock()?;
        let ready = repository.path().join("acquiring");
        let child = cargo_berth_test_support::FixtureCommand::new(std::env::current_exe()?)
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD_ROOT, repository.path())
            .env(
                super::super::constants::MUTATION_LOCK_READY_PATH_ENVIRONMENT,
                &ready,
            )
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let started = std::time::Instant::now();
        while !ready.exists() && started.elapsed() < std::time::Duration::from_millis(500) {
            thread::sleep(std::time::Duration::from_millis(1));
        }
        let prepared_while_locked = repository.path().join("prepared").is_file();
        holder.unlock()?;
        let output = child.bounded_wait_with_output()?;
        assert!(output.status.success(), "{output:?}");
        assert!(ready.exists(), "child must reach the acquire signal");
        assert!(
            prepared_while_locked,
            "preparation must finish before waiting for the holder"
        );
        assert_eq!(
            journal::complete_record_events(&fs::read(&ledger.paths.journal)?)?.len(),
            1
        );
        Ok(())
    }

    #[test]
    fn session_retirements_prepare_the_index_before_waiting_for_the_mutation_lock()
    -> Result<(), Box<dyn std::error::Error>> {
        const CHILD_ROOT: &str = "CARGO_BERTH_TEST_SESSION_PREPARATION_ROOT";
        const CHILD_MODE: &str = "CARGO_BERTH_TEST_SESSION_PREPARATION_MODE";
        const RETIRED: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f";
        const TEST: &str = "ledger::handle::tests::session_retirements_prepare_the_index_before_waiting_for_the_mutation_lock";
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let ledger = Ledger::open(std::path::Path::new(&root))?;
            if std::env::var(CHILD_MODE)? == "clear" {
                assert_eq!(
                    ledger.remove_current_session_mapping()?,
                    super::CurrentSessionMappingRemoval::Removed
                );
            } else {
                ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
                    TransactionValidation::<()>::Append(Box::new(JournalOperation::Release {
                        reservation_id: RETIRED.parse().expect("retired identity"),
                        disposition:    ReleaseDisposition::Integrated,
                        source:         None,
                    }))
                })?;
            }
            return Ok(());
        }
        for mode in ["release", "clear"] {
            let repository = test_support::scratch_repository();
            Ledger::initialize(repository.path())?;
            let ledger = Ledger::open(repository.path())?;
            let run = CoordinationRunId::new();
            let retained = ReservationId::new();
            let index = ledger.paths.directory.join("session-identities.json");
            fs::write(
                &index,
                serde_json::to_vec(&serde_json::json!({"identities": {
                    "retiring-session": {"coordination_run_id": run, "reservation_id": RETIRED},
                    "retained-session": {"coordination_run_id": run, "reservation_id": retained},
                }}))?,
            )?;
            let lock = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&ledger.paths.lock)?;
            lock.lock()?;
            let ready = repository.path().join("session-prepared");
            let mut child = cargo_berth_test_support::FixtureCommand::new(std::env::current_exe()?)
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD_ROOT, repository.path())
                .env(CHILD_MODE, mode)
                .env("CARGO_BERTH_SESSION_ID", "retiring-session")
                .env(
                    super::super::constants::MUTATION_LOCK_READY_PATH_ENVIRONMENT,
                    &ready,
                )
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()?;
            while !ready.exists() {
                if child.try_wait()?.is_some() {
                    let output = child.bounded_wait_with_output()?;
                    panic!("{mode} exited before signaling preparation: {output:?}");
                }
                thread::sleep(std::time::Duration::from_millis(1));
            }
            // Corrupt this disposable index in place without replacing its publication inode.
            // Publishing the already prepared change must preserve the unrelated mapping.
            fs::write(&index, b"unreadable disposable index")?;
            lock.unlock()?;
            let output = child.bounded_wait_with_output()?;
            assert!(output.status.success(), "{mode}: {output:?}");
            let actual: Value = serde_json::from_slice(&fs::read(&index)?)?;
            assert_eq!(
                actual["identities"]
                    .as_object()
                    .ok_or("session index")?
                    .len(),
                1
            );
            assert_eq!(
                actual["identities"]["retained-session"]["reservation_id"],
                serde_json::json!(retained)
            );
        }
        Ok(())
    }

    #[test]
    fn prepared_reconciliation_bytes_remain_unpublished_until_the_locked_append()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let before = fs::read(&ledger.paths.journal)?;
        let retired = [ReservationId::new(), ReservationId::new()];
        let retained = ReservationId::new();
        let run = CoordinationRunId::new();
        let mapping_path = ledger.paths.directory.join("session-identities.json");
        let identity =
            |reservation| super::session::SessionReservationIdentity::new(run, reservation);
        let mappings = serde_json::json!({"identities": {
            "first": identity(retired[0]), "second": identity(retired[1]), "retained": identity(retained),
        }});
        fs::write(&mapping_path, serde_json::to_vec(&mappings)?)?;
        let mapping_before = fs::read(&mapping_path)?;
        let competing = fs::File::options()
            .read(true)
            .write(true)
            .open(&ledger.paths.lock)?;
        let (transaction, batch) = ledger.prepare_mutation_from_replay(
            std::time::Duration::ZERO,
            |replay, repository| {
                competing
                    .try_lock()
                    .expect("batch preparation must leave the lock available");
                let batch = super::PreparedReconciliationAppend::prepare(
                    replay,
                    &journal::JournalActor {
                        repository,
                        worktree: WorktreeId::new(),
                        run: CoordinationRunId::new(),
                    },
                    retired
                        .iter()
                        .map(|reservation_id| JournalOperation::Release {
                            reservation_id: *reservation_id,
                            disposition:    ReleaseDisposition::Integrated,
                            source:         None,
                        })
                        .collect(),
                    &std::collections::HashMap::new(),
                    &ledger.paths.directory,
                )
                .expect("batch should prepare while another descriptor holds the lock");
                assert_eq!(
                    fs::read(&ledger.paths.journal).expect("journal should read"),
                    before
                );
                assert_eq!(
                    fs::read(&mapping_path).expect("mapping should read"),
                    mapping_before,
                    "session retirements must stay unpublished during preparation"
                );
                competing
                    .unlock()
                    .expect("preparation holder should unlock");
                batch
            },
        )?;
        assert!(matches!(
            competing.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        let expected = batch.replay.clone();
        ledger.conclude(transaction, |transaction| {
            transaction.append_prepared_reconciliation(batch)
        })?;
        let actual = journal::Journal::replay_read_only(&ledger.paths.journal)?;
        assert_eq!(actual, expected);
        assert_eq!(actual.record_count, 2);
        let mappings: Value = serde_json::from_slice(&fs::read(&mapping_path)?)?;
        assert_eq!(
            mappings,
            serde_json::json!({"identities": {"retained": identity(retained)}})
        );
        Ok(())
    }

    #[test]
    fn an_intervening_event_reprepares_the_decision_instead_of_appending_stale_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let stale = renewal_operation();
        let fresh = renewal_operation();
        let mut attempts = 0;
        ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |state| {
            attempts += 1;
            if attempts == 1 {
                let competing = fs::File::options()
                    .read(true)
                    .write(true)
                    .open(&ledger.paths.lock)
                    .expect("competing descriptor should open");
                competing
                    .try_lock()
                    .expect("preparation must not hold the lock");
                competing
                    .unlock()
                    .expect("competing descriptor should unlock");
                ledger
                    .transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
                        TransactionValidation::<()>::Append(Box::new(renewal_operation()))
                    })
                    .expect("intervening event should append");
                TransactionValidation::<()>::Append(Box::new(stale.clone()))
            } else {
                assert_eq!(u64::from(state.generation()), 1);
                TransactionValidation::Append(Box::new(fresh.clone()))
            }
        })?;
        let events = journal::complete_record_events(&fs::read(&ledger.paths.journal)?)?;
        assert_eq!(attempts, 2);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].operation, fresh);
        assert!(!events.iter().any(|event| event.operation == stale));
        Ok(())
    }

    #[test]
    fn session_selection_changes_without_an_event_reprepare_the_decision()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let run = CoordinationRunId::new();
        let stale = ReservationId::new();
        let fresh = ReservationId::new();
        let index = ledger.paths.directory.join("session-identities.json");
        let mapping = |reservation| {
            serde_json::to_vec(&serde_json::json!({
                "identities": {"selection-session": {
                    "coordination_run_id": run, "reservation_id": reservation
                }}
            }))
        };
        fs::write(&index, mapping(stale)?)?;
        let replacement = ledger.paths.directory.join("replacement-session-index");
        fs::write(&replacement, mapping(fresh)?)?;
        let mut attempts = 0;
        ledger.transact(WorktreeId::new(), run, |state| {
            attempts += 1;
            assert_eq!(u64::from(state.generation()), 0);
            let sessions = super::session::phase_reservation_sessions(&ledger.paths.directory, run)
                .expect("session selection should read");
            let selected = *sessions
                .keys()
                .next()
                .expect("session must select a reservation");
            if attempts == 1 {
                ledger
                    .transact_with_committed_action(
                        WorktreeId::new(),
                        run,
                        |_| CommittedActionValidation::<(), ()>::AlreadySatisfied(()),
                        |()| fs::rename(&replacement, &index),
                    )
                    .expect("the selection switch should publish without an append");
            }
            TransactionValidation::<()>::Append(Box::new(JournalOperation::Renew {
                reservation_id: selected,
            }))
        })?;
        let events = journal::complete_record_events(&fs::read(&ledger.paths.journal)?)?;
        assert_eq!(attempts, 2);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].operation,
            JournalOperation::Renew {
                reservation_id: fresh,
            }
        );
        Ok(())
    }

    #[test]
    fn repeated_invalidation_prepares_under_lock_and_appends_only_the_current_result()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let stale = renewal_operation();
        let current = renewal_operation();
        let competing = fs::File::options()
            .read(true)
            .write(true)
            .open(&ledger.paths.lock)?;
        let mut attempts = 0;
        ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
            attempts += 1;
            let operation = match competing.try_lock() {
                Ok(()) => {
                    competing.unlock().expect("release the preparation probe");
                    ledger
                        .transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
                            TransactionValidation::<()>::Append(Box::new(renewal_operation()))
                        })
                        .expect("intervening event should append");
                    stale.clone()
                },
                Err(TryLockError::WouldBlock) => current.clone(),
                Err(TryLockError::Error(error)) => panic!("lock probe failed: {error}"),
            };
            TransactionValidation::<()>::Append(Box::new(operation))
        })?;
        assert_eq!(
            attempts,
            super::super::constants::MUTATION_PREPARATION_ATTEMPTS as usize + 1
        );
        let events = journal::complete_record_events(&fs::read(&ledger.paths.journal)?)?;
        assert_eq!(events.len(), attempts);
        assert!(!events.iter().any(|event| event.operation == stale));
        assert_eq!(
            events
                .iter()
                .filter(|event| event.operation == current)
                .count(),
            1
        );
        assert_eq!(
            events.last().expect("writer must append").operation,
            current
        );
        Ok(())
    }

    #[test]
    fn reconciliation_report_building_runs_after_the_committed_lock_is_released()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let competing = fs::File::options()
            .read(true)
            .write(true)
            .open(&ledger.paths.lock)?;
        ledger.transact_reconciliation_then(
            WorktreeId::new(),
            CoordinationRunId::new(),
            |_| super::ReconciliationValidation::<(), ()>::Apply {
                operations:             vec![renewal_operation()],
                cover_actors:           std::collections::HashMap::new(),
                recoverable_operations: Vec::new(),
                action:                 (),
            },
            |(), state, _| {
                assert_eq!(u64::from(state.generation()), 1);
                assert!(matches!(
                    competing.try_lock(),
                    Err(std::fs::TryLockError::WouldBlock)
                ));
                Ok::<(), std::io::Error>(())
            },
            |(), state, _| {
                competing
                    .try_lock()
                    .expect("report builder should not retain the mutation lock");
                assert_eq!(u64::from(state.generation()), 1);
                competing.unlock()
            },
        )?;
        Ok(())
    }

    #[test]
    fn journal_movement_reads_no_records_and_detects_unpublished_appends()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let initial = ledger.current_journal_position()?;
        let projection_before = fs::read(&ledger.paths.projection)?;
        // Movement detection does not decode even a complete malformed record.
        fs::write(&ledger.paths.journal, b"invalid journal record\n")?;
        let appended = ledger.current_journal_position()?;
        assert_ne!(appended, initial);
        assert_eq!(fs::read(&ledger.paths.projection)?, projection_before);
        assert!(ledger.read_validated_journal().is_err());
        fs::remove_file(&ledger.paths.projection)?;
        assert_ne!(ledger.current_journal_position()?, appended);
        assert!(!ledger.paths.projection.exists());
        Ok(())
    }

    #[test]
    fn rejected_reconciliation_does_not_append_or_run_its_committed_action() {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");
        let before = fs::read(&ledger.paths.journal).expect("journal should read");
        let mut committed = false;
        let result = ledger
            .transact_reconciliation(
                WorktreeId::new(),
                CoordinationRunId::new(),
                |_| super::ReconciliationValidation::Reject("rewrite subject changed"),
                |(): (), _, _| {
                    committed = true;
                    Ok::<(), std::convert::Infallible>(())
                },
            )
            .expect("rejected validation should return without mutation");
        assert!(matches!(
            result,
            LedgerReconciliationOutcome::Rejected("rewrite subject changed")
        ));
        assert!(!committed);
        assert_eq!(
            fs::read(&ledger.paths.journal).expect("journal should read"),
            before
        );
    }

    #[test]
    fn lock_free_read_retries_a_projection_published_after_journal_replay() {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");
        let repository_id = ledger.repository_identity().expect("identity should read");
        let operation = renewal_operation();
        let mut attempts = 0;
        let record_count = super::retry_concurrent_projection_publication(|| {
            attempts += 1;
            let replay = super::Journal::replay_read_only(&ledger.paths.journal)?;
            if attempts == 1 {
                ledger
                    .transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
                        TransactionValidation::<()>::Append(Box::new(operation.clone()))
                    })
                    .expect("concurrent publication should append");
            }
            super::projection::read_validated(&ledger.paths.projection, repository_id, &replay)?;
            Ok(replay.record_count)
        })
        .expect("fresh journal and projection should agree");
        assert_eq!(attempts, 2);
        assert_eq!(record_count, 1);
        let events = journal::complete_record_events(
            &fs::read(&ledger.paths.journal).expect("journal should read"),
        )
        .expect("journal should decode");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, operation);
    }

    #[test]
    fn lock_free_read_bounds_cache_ahead_retries_and_preserves_other_failures() {
        let mut attempts = 0;
        let result = super::retry_concurrent_projection_publication::<()>(|| {
            attempts += 1;
            Err(LedgerError::Projection(ProjectionError::CacheAhead))
        });
        assert_eq!(attempts, 3);
        assert!(matches!(
            result,
            Err(LedgerError::Projection(ProjectionError::CacheAhead))
        ));
        attempts = 0;
        let result = super::retry_concurrent_projection_publication::<()>(|| {
            attempts += 1;
            Err(LedgerError::Projection(
                ProjectionError::JournalFingerprintMismatch,
            ))
        });
        assert_eq!(attempts, 1);
        assert!(matches!(
            result,
            Err(LedgerError::Projection(
                ProjectionError::JournalFingerprintMismatch
            ))
        ));
    }

    #[test]
    fn concurrent_mutations_append_without_losing_either_record() {
        let repository = test_support::scratch_repository();
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
    fn ordering_admission_reuses_the_transaction_graph_without_rechecking_at_append()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let holder = crate::ids::ReservationId::new();
        ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
            TransactionValidation::<()>::Append(Box::new(ordering_claim(
                repository.path(),
                holder,
                TestClaimOrdering::Unordered,
            )))
        })?;
        let requester = crate::ids::ReservationId::new();
        let operation = ordering_claim(
            repository.path(),
            requester,
            TestClaimOrdering::After(holder),
        );
        let before = crate::edge::ordering_work();
        let appended = ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |state| {
            state
                .ordering_graph()
                .expect("claim binding should read the graph");
            let checked = crate::edge::CycleFreeOrderingOperation::check(&state, operation.clone())
                .expect("ordered claim should be admitted");
            TransactionValidation::<()>::AppendChecked(Box::new(checked))
        })?;
        assert!(matches!(
            appended,
            LedgerTransactionOutcome::Appended { .. }
        ));
        let after = crate::edge::ordering_work();
        assert_eq!(after.graph_replays - before.graph_replays, 1);
        assert_eq!(after.cycle_checks - before.cycle_checks, 1);

        let successor = crate::ids::ReservationId::new();
        let operation = ordering_claim(
            repository.path(),
            successor,
            TestClaimOrdering::After(requester),
        );
        let before = crate::edge::ordering_work();
        let appended = ledger.transact_with_committed_action(
            WorktreeId::new(),
            CoordinationRunId::new(),
            |state| {
                state
                    .ordering_graph()
                    .expect("named claim binding should read the graph");
                let checked =
                    crate::edge::CycleFreeOrderingOperation::check(&state, operation.clone())
                        .expect("named ordering should be admitted");
                CommittedActionValidation::<(), ()>::AppendChecked {
                    operation: Box::new(checked),
                    action:    (),
                }
            },
            Ok::<_, std::io::Error>,
        )?;
        assert!(matches!(
            appended,
            LedgerCommittedActionOutcome::Appended { .. }
        ));
        let after = crate::edge::ordering_work();
        assert_eq!(after.graph_replays - before.graph_replays, 1);
        assert_eq!(after.cycle_checks - before.cycle_checks, 1);
        let events = ledger.read_validated_journal()?.into_coordination_events();
        assert_eq!(crate::edge::OrderingGraph::replay(&events)?.edge_count(), 2);
        Ok(())
    }

    #[test]
    fn ordering_batches_extend_one_graph_and_refuse_the_first_cycle()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let ids = [
            crate::ids::ReservationId::new(),
            crate::ids::ReservationId::new(),
            crate::ids::ReservationId::new(),
        ];
        for id in ids {
            ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
                TransactionValidation::<()>::Append(Box::new(ordering_claim(
                    repository.path(),
                    id,
                    TestClaimOrdering::Unordered,
                )))
            })?;
        }
        let operations = ids
            .into_iter()
            .zip(ids.into_iter().cycle().skip(1))
            .map(|(predecessor, successor)| JournalOperation::Sequence {
                predecessor,
                successor,
                edge_id: crate::ids::EdgeId::new(),
                scopes: crate::ledger::ReservationScopeSet::try_from(vec![
                    crate::ledger::ReservationScope {
                        path: "shared.rs".parse().expect("scope path should parse"),
                        kind: crate::ledger::ScopeKind::File,
                    },
                ])
                .expect("shared scope should be nonempty"),
                reason: "order shared work".parse().expect("reason should parse"),
            })
            .collect::<Vec<_>>();
        let journal = fs::read(&ledger.paths.journal)?;
        let before = crate::edge::ordering_work();
        let result = ledger.transact_reconciliation(
            WorktreeId::new(),
            CoordinationRunId::new(),
            |_| super::ReconciliationValidation::<(), ()>::Apply {
                operations:             operations.clone(),
                cover_actors:           std::collections::HashMap::new(),
                recoverable_operations: vec![],
                action:                 (),
            },
            |(), _, _| Ok::<_, std::io::Error>(()),
        );
        assert!(matches!(
            result,
            Err(super::LedgerCommittedActionError::Transaction(
                LedgerTransactionError::LedgerUnreadable(LedgerError::OrderingReplay(
                    crate::edge::EdgeReplayError::Cycle
                ))
            ))
        ));
        let after = crate::edge::ordering_work();
        assert_eq!(after.graph_replays - before.graph_replays, 1);
        assert_eq!(after.cycle_checks - before.cycle_checks, operations.len());
        assert_eq!(fs::read(&ledger.paths.journal)?, journal);
        ledger.transact_reconciliation(
            WorktreeId::new(),
            CoordinationRunId::new(),
            |_| super::ReconciliationValidation::<(), ()>::Apply {
                operations:             operations[..2].to_vec(),
                cover_actors:           std::collections::HashMap::new(),
                recoverable_operations: vec![],
                action:                 (),
            },
            |(), _, _| Ok::<_, std::io::Error>(()),
        )?;
        let events = ledger.read_validated_journal()?.into_coordination_events();
        assert_eq!(crate::edge::OrderingGraph::replay(&events)?.edge_count(), 2);
        Ok(())
    }

    /// The relationship recorded by the cost-test claim fixture.
    #[derive(Clone, Copy)]
    enum TestClaimOrdering {
        Unordered,
        After(ReservationId),
    }

    fn ordering_claim(
        root: &Path,
        id: ReservationId,
        ordering: TestClaimOrdering,
    ) -> JournalOperation {
        let authorization = match ordering {
            TestClaimOrdering::Unordered => serde_json::json!({ "kind": "no_conflict" }),
            TestClaimOrdering::After(holder) => serde_json::json!({
                "kind": "sequence_per_holder", "reason": "order shared work", "replaced_orderings": [],
                "overlaps": [{ "reservation_id": holder, "scope_revision": [{ "path": "shared.rs", "kind": "file" }], "scopes": [{ "path": "shared.rs", "kind": "file" }] }],
                "holders": [{ "blocker": holder, "direction": "holder_before_requester", "edge_id": crate::ids::EdgeId::new() }],
            }),
        };
        serde_json::from_value(serde_json::json!({
            "op": "claim", "reservation_id": id, "scopes": [{ "path": "shared.rs", "kind": "file" }],
            "source": { "kind": "explicit" }, "purpose": { "kind": "not_provided_by_caller" },
            "trunk_at_claim": "1111111111111111111111111111111111111111",
            "head_snapshot": { "kind": "branch", "full_ref": "refs/heads/lane", "head": "1111111111111111111111111111111111111111" },
            "phase_start_head": "1111111111111111111111111111111111111111",
            "worktree_root": root, "worktree_administrative_locator": "main",
            "authorization": authorization, "coordination_identity_provenance": "not_presented",
        })).expect("ordering claim fixture should decode")
    }

    #[test]
    fn validation_controls_whether_exactly_one_record_is_appended() {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");
        let journal_before = fs::read(&ledger.paths.journal).expect("journal should read");
        let projection_before = fs::read(&ledger.paths.projection).expect("projection should read");

        let rejected = ledger
            .transact(WorktreeId::new(), CoordinationRunId::new(), |state| {
                assert_eq!(state.coordination_events(), []);
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
    fn already_satisfied_actions_keep_the_journal_unchanged_and_consume_under_lock()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let before = fs::read(&ledger.paths.journal)?;
        let competing_lock = fs::File::options()
            .read(true)
            .write(true)
            .open(&ledger.paths.lock)?;
        let output = ledger.transact_with_committed_action_and_consume_locked_outcome(
            WorktreeId::new(),
            CoordinationRunId::new(),
            |_| CommittedActionValidation::<(), _>::AlreadySatisfied("selected reservation"),
            |selection| {
                assert!(matches!(
                    competing_lock.try_lock(),
                    Err(std::fs::TryLockError::WouldBlock)
                ));
                Ok::<_, std::io::Error>(selection)
            },
            |outcome| {
                assert!(matches!(
                    competing_lock.try_lock(),
                    Err(std::fs::TryLockError::WouldBlock)
                ));
                match outcome {
                    LedgerCommittedActionOutcome::AlreadySatisfied { output, .. } => Ok(output),
                    LedgerCommittedActionOutcome::Appended { .. }
                    | LedgerCommittedActionOutcome::Rejected(()) => {
                        Err("satisfied validation should not append or reject")
                    },
                }
            },
        )??;
        assert_eq!(output, "selected reservation");
        assert_eq!(fs::read(&ledger.paths.journal)?, before);
        Ok(())
    }

    #[test]
    fn committed_actions_run_after_append_while_the_mutation_lock_is_held() {
        let repository = test_support::scratch_repository();
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
        let repository = test_support::scratch_repository();
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
        let repository = test_support::scratch_repository();
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
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");

        let result = ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
            TransactionValidation::<()>::Append(Box::new(
                JournalOperation::ForcedIntegrationPermit {
                    permit_id:      crate::ids::ForcedIntegrationPermitId::new(),
                    reservation_id: ReservationId::new(),
                    reason:         "x"
                        .repeat(MAXIMUM_JOURNAL_RECORD_BYTES)
                        .parse::<ForcedIntegrationReason>()
                        .expect("oversized reason should remain non-empty"),
                    skipped_holds:  SkippedIntegrationHoldSet::OrderingEdges {
                        edges: vec![super::super::journal::SkippedOrderingEdge {
                            edge_id:     crate::ids::EdgeId::new(),
                            predecessor: ReservationId::new(),
                        }],
                    },
                },
            ))
        });

        assert!(matches!(
            result,
            Err(LedgerTransactionError::CorrectableInput(
                CorrectableTransactionInput::RecordTooLarge { .. }
            ))
        ));
        assert_eq!(
            fs::read_to_string(&ledger.paths.journal).expect("journal should read"),
            ""
        );
    }

    /// A key whose own path lists are empty, isolating the extent's scopes as what grows.
    fn empty_merge_extent_key() -> MergeExtentKey {
        MergeExtentKey {
            reservation_group: HolderMergeExtentCoverage::FullHolder,
            trunk:             GitObjectId::from_str(&"a".repeat(40))
                .expect("trunk oid should parse"),
            head:              GitObjectId::from_str(&"b".repeat(40))
                .expect("head oid should parse"),
            working_tree:      WorkingTreeFingerprint {
                tracked_paths:   Vec::new(),
                untracked_paths: Vec::new(),
            },
        }
    }

    /// Paths shaped like a real branch surface, which is what makes these records grow.
    fn oversized_observed_scopes(count: usize) -> Vec<ReservationScope> {
        (0..count)
            .map(|index| ReservationScope {
                path: ReservationScopePath::from_str(&format!(
                    "crates/cargo-berth/src/observed/module_{index}/source.rs"
                ))
                .expect("generated scope path should parse"),
                kind: ScopeKind::File,
            })
            .collect()
    }

    #[test]
    fn an_observed_branch_surface_records_past_the_caller_correctable_limit() {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");

        // Wider than the reporting repository's 203-path extent, so the record this appends is
        // well past MAXIMUM_JOURNAL_RECORD_BYTES and would once have failed the whole transaction.
        let scopes = oversized_observed_scopes(400);
        let extent = MergeExtent::derived(empty_merge_extent_key(), scopes.clone());

        let result = ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
            TransactionValidation::<()>::Append(Box::new(
                JournalOperation::HolderMergeExtentObserved {
                    extent:       extent.clone(),
                    reservations: ObservedReservation {
                        reservation_id: ReservationId::new(),
                        run_status:     ReservationRunStatus::default(),
                    }
                    .into(),
                },
            ))
        });

        assert!(
            result.is_ok(),
            "a derived branch surface must record rather than stop the repository"
        );
        let journal = fs::read_to_string(&ledger.paths.journal).expect("journal should read");
        assert!(
            journal.len() > MAXIMUM_JOURNAL_RECORD_BYTES,
            "the appended record should exceed the caller-correctable limit"
        );
        // Every observed path is present: a surface recorded short would protect less than it
        // observed, which under-refuses and lets two worktrees write the same file.
        for scope in &scopes {
            assert!(
                journal.contains(&scope.path.to_string()),
                "the recorded extent dropped {scope:?}"
            );
        }
    }

    #[test]
    fn a_derived_record_past_its_own_ceiling_is_not_reported_as_caller_input() {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");

        let component_bytes = 4 * MAXIMUM_JOURNAL_RECORD_BYTES;
        let long_component = "x".repeat(component_bytes);
        let scope_count = MAXIMUM_DERIVED_JOURNAL_RECORD_BYTES / component_bytes + 1;
        let scopes = (0..scope_count)
            .map(|index| ReservationScope {
                path: ReservationScopePath::from_str(&format!("observed/{index}/{long_component}"))
                    .expect("long observed path should parse"),
                kind: ScopeKind::File,
            })
            .collect();
        let extent = MergeExtent::derived(empty_merge_extent_key(), scopes);

        let result = ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
            TransactionValidation::<()>::Append(Box::new(
                JournalOperation::HolderMergeExtentObserved {
                    extent:       extent.clone(),
                    reservations: ObservedReservation {
                        reservation_id: ReservationId::new(),
                        run_status:     ReservationRunStatus::default(),
                    }
                    .into(),
                },
            ))
        });

        // The ceiling still exists, so a runaway is caught -- but it is not the caller's to
        // correct, and reporting it as correctable input is what once told a session to
        // reduce scopes it had never declared.
        assert!(matches!(
            result,
            Err(LedgerTransactionError::DerivedRecordTooLarge { .. })
        ));
    }

    #[test]
    fn rejected_transaction_rebuilds_a_stale_projection() {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");
        let mut unpublished_transaction = ledger
            .prepare_mutation(super::MUTATING_VERB_CONTENTION_TOLERANCE, |_| ())
            .map(|(transaction, ())| transaction)
            .expect("unpublished transaction should begin");
        unpublished_transaction
            .append(
                WorktreeId::new(),
                CoordinationRunId::new(),
                unpublished_transaction
                    .admit(super::PreparedOperationSessionChange {
                        operation:   renewal_operation(),
                        publication: super::PreparedSessionMappingPublication::Unchanged,
                    })
                    .expect("legacy renewal should be admitted"),
            )
            .expect("record should append before publication");
        std::mem::drop(unpublished_transaction);

        let mut record_was_visible = false;
        let outcome = ledger
            .transact(WorktreeId::new(), CoordinationRunId::new(), |state| {
                record_was_visible = u64::from(state.journal_end_offset()) > 0;
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
            let repository = test_support::scratch_repository();
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
                ledger
                    .prepare_mutation(super::MUTATING_VERB_CONTENTION_TOLERANCE, |_| ())
                    .map(|(transaction, ())| transaction),
                Err(LedgerError::Projection(ProjectionError::CacheAhead))
            ));
        }
    }

    #[test]
    fn projection_repair_and_reinitialization_remove_the_replay_checkpoint() {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path()).expect("ledger should initialize");
        let ledger = Ledger::open(repository.path()).expect("ledger should open");

        Ledger::repair_projection(repository.path())
            .expect("repair should accept a ledger without a checkpoint");
        fs::write(&ledger.paths.replay_checkpoint, b"stale").expect("checkpoint should write");
        Ledger::repair_projection(repository.path()).expect("projection should rebuild");
        assert!(!ledger.paths.replay_checkpoint.exists());

        fs::write(&ledger.paths.replay_checkpoint, b"stale").expect("checkpoint should write");
        Ledger::reinitialize_after_review(repository.path()).expect("ledger should reinitialize");
        assert!(!ledger.paths.replay_checkpoint.exists());
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
                assert_eq!(state.coordination_events(), []);
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
