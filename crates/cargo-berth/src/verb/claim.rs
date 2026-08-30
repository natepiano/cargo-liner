//! Atomic reservation acquisition.

use std::convert::Infallible;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;

use crate::answer::ConflictAuthorization;
use crate::answer::OverlapAuthorizationRequest;
use crate::answer::OverlapEscalationPayload;
use crate::answer::OverlapProposal;
use crate::answer::OverlapProposalSubmission;
use crate::answer::OverlapRequester;
use crate::answer::PermissiveOverlapAnswer;
use crate::answer::PermissiveOverlapAuthorizationRequest;
use crate::answer::RequesterCoordinationIdentity;
use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::config::Enrollment;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::coordination_identity::CoordinationIdentityValidationContext;
use crate::coordination_identity::CoordinationIdentityValidationError;
use crate::coordination_identity::RecoveryCommandLine;
use crate::coordination_identity::validate_coordination_identity;
use crate::edge::EdgeReplayError;
use crate::edge::OrderingGraph;
use crate::git;
use crate::git::GitError;
use crate::git::ReferenceLookup;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadCommit;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::CommittedActionValidation;
use crate::ledger::EditAuthorization;
use crate::ledger::FullRefName;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReplayedLedgerState;
use crate::ledger::ReservationPurpose;
use crate::ledger::ReservationScopeAdditionSet;
use crate::ledger::ResolvedEditAuthorization;
use crate::ledger::TransactionValidation;
use crate::ledger::TrunkObservationAtClaim;
use crate::ledger::WidenCause;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::CoordinationRunMarkerPublication;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation::EditBlockingStatus;
use crate::reservation::Reservation;
use crate::reservation::ReservationConflict;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::reservation::WidenScopeBinding;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;
use crate::scope::PathCaseError;
use crate::scope::ReservationScopeSet;
use crate::session;
use crate::session::SessionIdentityMappingPublication;

const HEADS_REF_PREFIX: &str = "refs/heads/";

/// A parsed claim whose provenance carries domain types rather than CLI options.
pub(crate) struct ClaimRequest {
    /// Lexically valid scopes before repository-case antichain reduction.
    pub(crate) declared_scopes:            DeclaredReservationScopeSet,
    /// The acquisition origin retained in the audit trail.
    pub(crate) source:                     ClaimSource,
    /// The caller-supplied explanation state.
    pub(crate) purpose:                    ReservationPurpose,
    /// How the claim selects the coordination run that will own it.
    pub(crate) coordination_run_selection: ClaimCoordinationRunSelection,
    /// The phase-start commit selection.
    pub(crate) phase_start:                PhaseStartSelection,
    /// The semantic overlap answer, its reason, and proposal state.
    pub(crate) overlap_authorization:      OverlapAuthorizationRequest,
}

/// How a claim chooses the coordination run that will own its reservation.
pub(crate) enum ClaimCoordinationRunSelection {
    /// Use the run identity supplied through `--run`.
    Specified(CoordinationRunId),
    /// Continue the active run from process or worktree context, or start one.
    ContinueOrStart,
}

/// The replay validation required by the source of a resolved claim run.
#[derive(Clone, Copy)]
enum ClaimRunValidation {
    /// An explicit argument or process environment identifies a marker-independent caller.
    IndependentWithPresentedIdentity(CoordinationRunId),
    /// A marker-independent caller presented no identity, so only its actor run is minted.
    IndependentWithoutPresentedIdentity {
        /// The concrete run stamped on the new reservation and transaction.
        actor_run_id: CoordinationRunId,
    },
    /// A session mapping or marker must still agree with locked reservation state.
    ResolvedIdentityRequired(ResolvedEditAuthorization),
}

/// How a claim chooses its protected phase-start commit.
pub(crate) enum PhaseStartSelection {
    /// Use the current worktree HEAD observed during acquisition.
    CurrentHead,
    /// Retain the full object id supplied by the caller.
    Protected(ProtectedPhaseStartHead),
}

struct ClaimRepositoryFacts {
    head_snapshot: ClaimHeadSnapshot,
    current_head:  GitObjectId,
    worktree_root: CanonicalWorktreeRoot,
}

#[derive(Clone, Copy)]
struct SymbolicReferenceDepth(u8);

impl SymbolicReferenceDepth {
    const MAXIMUM: u8 = 32;
    const ROOT: Self = Self(0);

    fn descend(self, reference: &str) -> Result<Self, ClaimError> {
        if self.0 >= Self::MAXIMUM {
            return Err(ClaimError::SymbolicReferenceDepthExceeded {
                reference: reference.to_owned(),
                maximum:   Self::MAXIMUM,
            });
        }
        Ok(Self(self.0 + 1))
    }
}

enum FilesystemReferenceResolution {
    Resolved(GitObjectId),
    RequiresGitResolution {
        rejection_if_git_reports_missing: ClaimError,
    },
}

struct PreparedClaim {
    reservation_id:                  ReservationId,
    scopes:                          ReservationScopeSet,
    source:                          ClaimSource,
    purpose:                         ReservationPurpose,
    trunk_at_claim:                  TrunkObservationAtClaim,
    head_snapshot:                   ClaimHeadSnapshot,
    phase_start_head:                ProtectedPhaseStartHead,
    worktree_root:                   CanonicalWorktreeRoot,
    worktree_administrative_locator: WorktreeAdministrativeLocator,
}

struct ClaimValidationContext {
    run_validation:         ClaimRunValidation,
    worktree_context:       WorktreeContext,
    recovery_command_line:  RecoveryCommandLine,
    worktree_id:            WorktreeId,
    path_case:              PathCase,
    requester:              OverlapRequester,
    overlap_authorization:  OverlapAuthorizationRequest,
    maximum_reservations:   u32,
    maximum_ordering_edges: u32,
}

enum ClaimRunValidationError {
    CoordinationIdentity(CoordinationIdentityRejection),
    InvalidCanonicalWorktreeRoot,
}

/// Acquire a reservation or return a typed conflict without appending.
pub(crate) fn execute(
    claim_request: ClaimRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Claim, &error.to_string());
        },
    };
    let reconciliation_report =
        match reconcile::reconcile(&invocation_directory, RecoveredBypassReporting::Defer) {
            Ok(Enrollment::Enrolled(reconciliation_report)) => reconciliation_report,
            Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            }) => {
                return OutputEnvelope::unconfigured(
                    CommandVerb::Claim,
                    &expected_configuration_path,
                );
            },
            Err(error) => return error.into_output(CommandVerb::Claim),
        };
    let output_envelope = match acquire(claim_request, recovery_command_line) {
        Ok(Enrollment::Enrolled(ClaimExecution::Claimed {
            reservation_id,
            coordination_run_id,
            scopes,
            marker_publication,
            session_mapping_publication,
        })) => OutputEnvelope::claimed(
            reservation_id,
            coordination_run_id,
            scopes,
            marker_publication,
            session_mapping_publication,
        ),
        Ok(Enrollment::Enrolled(ClaimExecution::Blocked { conflicts })) => {
            OutputEnvelope::blocked_claim(conflicts)
        },
        Ok(Enrollment::Enrolled(ClaimExecution::AuthorizationRequired(escalation))) => {
            OutputEnvelope::claim_authorization_required(*escalation)
        },
        Ok(Enrollment::Enrolled(ClaimExecution::ReservationLimitReached(maximum))) => {
            OutputEnvelope::reservation_limit_reached(maximum)
        },
        Ok(Enrollment::Enrolled(ClaimExecution::OrderingEdgeLimitReached(maximum))) => {
            OutputEnvelope::claim_ordering_edge_limit_reached(maximum)
        },
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => OutputEnvelope::unconfigured(CommandVerb::Claim, &expected_configuration_path),
        Err(error) => error.into_output(CommandVerb::Claim),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

enum ClaimExecution {
    Claimed {
        reservation_id:              ReservationId,
        coordination_run_id:         CoordinationRunId,
        scopes:                      ReservationScopeSet,
        marker_publication:          CoordinationRunMarkerPublication,
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    Blocked {
        conflicts: Vec<ReservationConflict>,
    },
    AuthorizationRequired(Box<OverlapEscalationPayload>),
    ReservationLimitReached(u32),
    OrderingEdgeLimitReached(u32),
}

/// How one successful first-touch transaction established reservation coverage.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FirstTouchReservationAcquisitionKind {
    /// The transaction appended a new first-touch reservation.
    Appended,
    /// The transaction enlarged the acting run's existing first-touch reservation.
    Widened,
    /// The existing first-touch reservation already covered every claimable path.
    AlreadyHeld,
}

/// The complete durable identity and publication result of first-touch protection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FirstTouchReservationAcquisition {
    /// How the transaction established coverage.
    pub(crate) kind:             FirstTouchReservationAcquisitionKind,
    /// The reservation that protects the paths.
    pub(crate) reservation_id:   ReservationId,
    /// The coordination run that owns the reservation.
    coordination_run_id:         CoordinationRunId,
    /// The original phase-start commit retained by the reservation.
    phase_start_head:            ProtectedPhaseStartHead,
    /// Whether the worktree marker records the coordination run.
    marker_publication:          CoordinationRunMarkerPublication,
    /// Whether the harness session mapping records the reservation.
    session_mapping_publication: SessionIdentityMappingPublication,
}

/// Whether a first-touch transaction may protect the nonconflicting part of a post-write request.
#[derive(Clone, Copy)]
pub(crate) enum FirstTouchConflictHandling {
    /// Refuse the complete request when any path has a foreign holder.
    RefuseRequest,
    /// Protect every free path and return the foreign-held subset for reporting.
    ProtectFreePaths,
}

/// A first-touch request whose fixed source and authorization rule exclude permissive answers.
pub(crate) struct FirstTouchClaimRequest {
    /// Lexically valid paths before exact-file antichain reduction.
    pub(crate) declared_scopes:   DeclaredReservationScopeSet,
    /// How a post-write request handles a mixture of free and foreign-held paths.
    pub(crate) conflict_handling: FirstTouchConflictHandling,
}

/// The foreign-held portion of a successful partial post-write acquisition.
pub(crate) enum FirstTouchConflictOutcome {
    /// Every requested path is protected by the acting run's reservation.
    None,
    /// The write already entered paths that a foreign reservation protects.
    PostWriteIncursion {
        /// The exact requested file scopes covered by foreign holders.
        scopes:    ReservationScopeSet,
        /// Only the foreign holders covering those scopes.
        conflicts: Vec<ReservationConflict>,
    },
}

/// The states a first-touch request can produce without permissive authorization.
pub(crate) enum FirstTouchClaimExecution {
    /// The acting run holds a first-touch reservation over every returned scope.
    Acquired {
        /// The durable reservation identity, baseline, and publication results.
        acquisition: FirstTouchReservationAcquisition,
        /// The exact requested file scopes now protected by the reservation.
        scopes:      ReservationScopeSet,
        /// Any foreign-held paths excluded from a post-write acquisition.
        conflicts:   FirstTouchConflictOutcome,
    },
    /// A pre-write request was refused, or every post-write path had a foreign holder.
    Blocked {
        /// The exact requested file scopes refused by foreign holders.
        scopes:    ReservationScopeSet,
        /// Every holder covering at least one refused scope.
        conflicts: Vec<ReservationConflict>,
    },
    /// A fresh reservation could not be appended under repository policy.
    ReservationLimitReached(u32),
}

struct CommittedFirstTouchAcquisition {
    kind:             FirstTouchReservationAcquisitionKind,
    reservation_id:   ReservationId,
    phase_start_head: ProtectedPhaseStartHead,
    scopes:           ReservationScopeSet,
    conflicts:        FirstTouchConflictOutcome,
}

struct FirstTouchValidationContext {
    run_validation:        ClaimRunValidation,
    worktree_context:      WorktreeContext,
    recovery_command_line: RecoveryCommandLine,
    coordination_run_id:   CoordinationRunId,
    worktree_id:           WorktreeId,
    path_case:             PathCase,
    conflict_handling:     FirstTouchConflictHandling,
    maximum_reservations:  u32,
}

enum FirstTouchClaimRejection {
    Blocked {
        scopes:    ReservationScopeSet,
        conflicts: Vec<ReservationConflict>,
    },
    AlreadyHeld(CommittedFirstTouchAcquisition),
    Replay(ReservationReplayError),
    CoordinationIdentity(CoordinationIdentityRejection),
    InvalidCanonicalWorktreeRoot,
    ReservationLimitReached(u32),
}

fn acquire(
    claim_request: ClaimRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<Enrollment<ClaimExecution>, ClaimError> {
    let ClaimRequest {
        declared_scopes,
        source,
        purpose,
        coordination_run_selection,
        phase_start,
        overlap_authorization,
    } = claim_request;
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let resolved_edit_authorization = ledger::resolve_identity(&worktree_context)?;
    let claim_run_validation = coordination_run_selection.resolve(resolved_edit_authorization);
    let actor_run_id = claim_run_validation.actor_run_id();
    let journal_mutation_actor =
        resolved_edit_authorization.journal_mutation_actor_for(actor_run_id);
    let path_case = PathCase::read(worktree_context.common_git_directory())?;
    let scopes = match &source {
        ClaimSource::FirstTouch => declared_scopes.into_exact_file_antichain(path_case),
        ClaimSource::WorkPlan { .. } | ClaimSource::Explicit => {
            declared_scopes.into_minimal_antichain(path_case)
        },
    };
    let claim_repository_facts =
        ClaimRepositoryFacts::read(&worktree_context, claim_run_validation)?;
    let phase_start_head = match phase_start {
        PhaseStartSelection::CurrentHead => {
            ProtectedPhaseStartHead::from(claim_repository_facts.current_head.clone())
        },
        PhaseStartSelection::Protected(protected_phase_start_head) => protected_phase_start_head,
    };
    let berth_config = match BerthConfig::read(worktree_context.repository_root())? {
        Enrollment::Enrolled(berth_config) => berth_config,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let trunk_at_claim = read_trunk_commit(&worktree_context, &berth_config.trunk, &source)?;
    let ledger = Ledger::open_from_discovered_worktree(&worktree_context)?;
    let reservation_id = ReservationId::new();
    let prepared_claim = PreparedClaim {
        reservation_id,
        scopes: scopes.clone(),
        source,
        purpose,
        trunk_at_claim,
        head_snapshot: claim_repository_facts.head_snapshot,
        phase_start_head,
        worktree_root: claim_repository_facts.worktree_root,
        worktree_administrative_locator: worktree_context.administrative_locator().clone(),
    };
    let requester = OverlapRequester::new(
        claim_run_validation.presented_coordination_identity(),
        journal_mutation_actor.worktree_id,
        prepared_claim.source.clone(),
        prepared_claim.purpose.clone(),
    );
    let outcome = ledger.transact(
        journal_mutation_actor.worktree_id,
        journal_mutation_actor.coordination_run_id,
        |state| {
            validate_claim_transaction(
                &state,
                prepared_claim,
                ClaimValidationContext {
                    run_validation: claim_run_validation,
                    worktree_context: worktree_context.clone(),
                    recovery_command_line: recovery_command_line.clone(),
                    worktree_id: journal_mutation_actor.worktree_id,
                    path_case,
                    requester,
                    overlap_authorization,
                    maximum_reservations: berth_config.maximum_reservations,
                    maximum_ordering_edges: berth_config.maximum_ordering_edges,
                },
            )
        },
    )?;
    claim_execution_from_outcome(outcome, reservation_id, scopes, &worktree_context)
        .map(Enrollment::Enrolled)
}

/// Acquire, widen, or reuse the acting run's one first-touch reservation.
pub(crate) fn acquire_first_touch(
    request: FirstTouchClaimRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<Enrollment<FirstTouchClaimExecution>, ClaimError> {
    let FirstTouchClaimRequest {
        declared_scopes,
        conflict_handling,
    } = request;
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let resolved_edit_authorization = ledger::resolve_identity(&worktree_context)?;
    let run_validation =
        ClaimCoordinationRunSelection::ContinueOrStart.resolve(resolved_edit_authorization);
    let coordination_run_id = run_validation.actor_run_id();
    let journal_mutation_actor =
        resolved_edit_authorization.journal_mutation_actor_for(coordination_run_id);
    let path_case = PathCase::read(worktree_context.common_git_directory())?;
    let scopes = declared_scopes.into_exact_file_antichain(path_case);
    let source = ClaimSource::FirstTouch;
    let repository_facts = ClaimRepositoryFacts::read(&worktree_context, run_validation)?;
    let phase_start_head = ProtectedPhaseStartHead::from(repository_facts.current_head.clone());
    let berth_config = match BerthConfig::read(worktree_context.repository_root())? {
        Enrollment::Enrolled(berth_config) => berth_config,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let trunk_at_claim = read_trunk_commit(&worktree_context, &berth_config.trunk, &source)?;
    let ledger = Ledger::open_from_discovered_worktree(&worktree_context)?;
    let prepared_claim = PreparedClaim {
        reservation_id: ReservationId::new(),
        scopes,
        source,
        purpose: ReservationPurpose::NotProvidedByCaller,
        trunk_at_claim,
        head_snapshot: repository_facts.head_snapshot,
        phase_start_head,
        worktree_root: repository_facts.worktree_root,
        worktree_administrative_locator: worktree_context.administrative_locator().clone(),
    };
    let execution = ledger.transact_with_committed_action_and_consume_locked_outcome(
        journal_mutation_actor.worktree_id,
        journal_mutation_actor.coordination_run_id,
        |state| {
            validate_first_touch_transaction(
                &state,
                prepared_claim,
                FirstTouchValidationContext {
                    run_validation,
                    worktree_context: worktree_context.clone(),
                    recovery_command_line: recovery_command_line.clone(),
                    coordination_run_id,
                    worktree_id: journal_mutation_actor.worktree_id,
                    path_case,
                    conflict_handling,
                    maximum_reservations: berth_config.maximum_reservations,
                },
            )
        },
        Ok::<_, Infallible>,
        |outcome| {
            first_touch_execution_from_outcome(outcome, coordination_run_id, &worktree_context)
        },
    );
    let execution = match execution {
        Ok(execution) => execution,
        Err(LedgerCommittedActionError::Transaction(error)) => {
            return Err(ClaimError::Transaction(error));
        },
        Err(LedgerCommittedActionError::Action(error)) => match error {},
    };
    execution.map(Enrollment::Enrolled)
}

fn first_touch_execution_from_outcome(
    outcome: LedgerCommittedActionOutcome<FirstTouchClaimRejection, CommittedFirstTouchAcquisition>,
    coordination_run_id: CoordinationRunId,
    worktree_context: &WorktreeContext,
) -> Result<FirstTouchClaimExecution, ClaimError> {
    match outcome {
        LedgerCommittedActionOutcome::Appended {
            output,
            session_mapping_publication,
        } => Ok(first_touch_acquired_execution(
            output,
            coordination_run_id,
            publish_coordination_run_marker(worktree_context, coordination_run_id),
            session_mapping_publication,
        )),
        LedgerCommittedActionOutcome::Rejected(FirstTouchClaimRejection::AlreadyHeld(output)) => {
            let session_mapping_publication = session::publish_reservation_identity(
                &worktree_context.ledger_directory(),
                coordination_run_id,
                output.reservation_id,
            );
            Ok(first_touch_acquired_execution(
                output,
                coordination_run_id,
                publish_coordination_run_marker(worktree_context, coordination_run_id),
                session_mapping_publication,
            ))
        },
        LedgerCommittedActionOutcome::Rejected(FirstTouchClaimRejection::Blocked {
            scopes,
            conflicts,
        }) => Ok(FirstTouchClaimExecution::Blocked { scopes, conflicts }),
        LedgerCommittedActionOutcome::Rejected(FirstTouchClaimRejection::Replay(error)) => {
            Err(ClaimError::ReservationReplay(error))
        },
        LedgerCommittedActionOutcome::Rejected(FirstTouchClaimRejection::CoordinationIdentity(
            rejection,
        )) => Err(ClaimError::CoordinationIdentity(rejection)),
        LedgerCommittedActionOutcome::Rejected(
            FirstTouchClaimRejection::InvalidCanonicalWorktreeRoot,
        ) => Err(ClaimError::InvalidCanonicalWorktreeRoot),
        LedgerCommittedActionOutcome::Rejected(
            FirstTouchClaimRejection::ReservationLimitReached(maximum),
        ) => Ok(FirstTouchClaimExecution::ReservationLimitReached(maximum)),
    }
}

fn first_touch_acquired_execution(
    committed: CommittedFirstTouchAcquisition,
    coordination_run_id: CoordinationRunId,
    marker_publication: CoordinationRunMarkerPublication,
    session_mapping_publication: SessionIdentityMappingPublication,
) -> FirstTouchClaimExecution {
    let CommittedFirstTouchAcquisition {
        kind,
        reservation_id,
        phase_start_head,
        scopes,
        conflicts,
    } = committed;
    FirstTouchClaimExecution::Acquired {
        acquisition: FirstTouchReservationAcquisition {
            kind,
            reservation_id,
            coordination_run_id,
            phase_start_head,
            marker_publication,
            session_mapping_publication,
        },
        scopes,
        conflicts,
    }
}

fn publish_coordination_run_marker(
    worktree_context: &WorktreeContext,
    coordination_run_id: CoordinationRunId,
) -> CoordinationRunMarkerPublication {
    worktree_context
        .publish_coordination_run_marker(coordination_run_id)
        .map_or_else(
            |error| CoordinationRunMarkerPublication::Unavailable {
                diagnostic: error.to_string(),
            },
            |()| CoordinationRunMarkerPublication::Published,
        )
}

fn claim_execution_from_outcome(
    outcome: LedgerTransactionOutcome<ClaimRejection>,
    reservation_id: ReservationId,
    scopes: ReservationScopeSet,
    worktree_context: &WorktreeContext,
) -> Result<ClaimExecution, ClaimError> {
    match outcome {
        LedgerTransactionOutcome::Appended {
            event,
            session_mapping_publication,
        } => {
            let coordination_run_id = event.actor.run;
            let marker_publication =
                publish_coordination_run_marker(worktree_context, coordination_run_id);
            Ok(ClaimExecution::Claimed {
                reservation_id,
                coordination_run_id,
                scopes,
                marker_publication,
                session_mapping_publication,
            })
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::Conflict(conflicts)) => {
            Ok(ClaimExecution::Blocked { conflicts })
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::AuthorizationRequired(escalation)) => {
            Ok(ClaimExecution::AuthorizationRequired(escalation))
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::Replay(error)) => {
            Err(ClaimError::ReservationReplay(error))
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::EdgeReplay(error)) => {
            Err(ClaimError::EdgeReplay(error))
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::CoordinationIdentity(rejection)) => {
            Err(ClaimError::CoordinationIdentity(rejection))
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::InvalidCanonicalWorktreeRoot) => {
            Err(ClaimError::InvalidCanonicalWorktreeRoot)
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::ReservationLimitReached(maximum)) => {
            Ok(ClaimExecution::ReservationLimitReached(maximum))
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::OrderingEdgeLimitReached(maximum)) => {
            Ok(ClaimExecution::OrderingEdgeLimitReached(maximum))
        },
    }
}

fn validate_claim_transaction(
    state: &ReplayedLedgerState<'_>,
    prepared_claim: PreparedClaim,
    context: ClaimValidationContext,
) -> TransactionValidation<ClaimRejection> {
    let ClaimValidationContext {
        run_validation,
        worktree_context,
        recovery_command_line,
        worktree_id,
        path_case,
        requester,
        overlap_authorization,
        maximum_reservations,
        maximum_ordering_edges,
    } = context;
    let reservations = match RetainedReservationSet::replay(state.events()) {
        Ok(reservations) => reservations,
        Err(error) => return TransactionValidation::Reject(ClaimRejection::Replay(error)),
    };
    if let Err(error) =
        run_validation.validate(&reservations, &worktree_context, &recovery_command_line)
    {
        return TransactionValidation::Reject(ClaimRejection::from(error));
    }
    if count_reaches_limit(reservations.nonterminal_count(), maximum_reservations) {
        return TransactionValidation::Reject(ClaimRejection::ReservationLimitReached(
            maximum_reservations,
        ));
    }
    let ordering_graph = match OrderingGraph::replay(state.events()) {
        Ok(ordering_graph) => ordering_graph,
        Err(error) => {
            return TransactionValidation::Reject(ClaimRejection::EdgeReplay(error));
        },
    };
    let conflicts =
        reservations.conflicts_for_claim(&prepared_claim.scopes, worktree_id, path_case);
    match overlap_authorization {
        OverlapAuthorizationRequest::Absent if conflicts.is_empty() => {
            TransactionValidation::Append(Box::new(
                prepared_claim.into_operation(ConflictAuthorization::NoConflict),
            ))
        },
        OverlapAuthorizationRequest::Absent => {
            TransactionValidation::Reject(ClaimRejection::Conflict(conflicts))
        },
        OverlapAuthorizationRequest::Permissive(request) => validate_authorization(
            *request,
            conflicts,
            requester,
            prepared_claim,
            &ordering_graph,
            maximum_ordering_edges,
        ),
    }
}

fn validate_first_touch_transaction(
    state: &ReplayedLedgerState<'_>,
    mut prepared_claim: PreparedClaim,
    context: FirstTouchValidationContext,
) -> CommittedActionValidation<FirstTouchClaimRejection, CommittedFirstTouchAcquisition> {
    let FirstTouchValidationContext {
        run_validation,
        worktree_context,
        recovery_command_line,
        coordination_run_id,
        worktree_id,
        path_case,
        conflict_handling,
        maximum_reservations,
    } = context;
    let reservations = match RetainedReservationSet::replay(state.events()) {
        Ok(reservations) => reservations,
        Err(error) => {
            return CommittedActionValidation::Reject(FirstTouchClaimRejection::Replay(error));
        },
    };
    if let Err(rejection) = validate_first_touch_run(
        run_validation,
        &reservations,
        &worktree_context,
        &recovery_command_line,
    ) {
        return CommittedActionValidation::Reject(rejection);
    }
    let requested_scopes = prepared_claim.scopes.clone();
    let conflicts = reservations.conflicts_for_first_touch(
        &requested_scopes,
        coordination_run_id,
        worktree_id,
        path_case,
    );
    let (protected_scopes, conflict_outcome) = match split_first_touch_scopes(
        &requested_scopes,
        conflicts,
        conflict_handling,
        path_case,
    ) {
        FirstTouchScopeDecision::Protect { scopes, conflicts } => (scopes, conflicts),
        FirstTouchScopeDecision::Block { scopes, conflicts } => {
            return CommittedActionValidation::Reject(FirstTouchClaimRejection::Blocked {
                scopes,
                conflicts,
            });
        },
    };
    let (protected_scopes, conflict_outcome) = match reuse_first_touch_reservation(
        &reservations,
        requested_scopes,
        protected_scopes,
        conflict_outcome,
        coordination_run_id,
        worktree_id,
        path_case,
    ) {
        FirstTouchReservationReuse::AppendRequired { scopes, conflicts } => (scopes, conflicts),
        FirstTouchReservationReuse::Complete(validation) => return validation,
    };
    if count_reaches_limit(reservations.nonterminal_count(), maximum_reservations) {
        return CommittedActionValidation::Reject(
            FirstTouchClaimRejection::ReservationLimitReached(maximum_reservations),
        );
    }
    prepared_claim.scopes = protected_scopes.clone();
    let acquisition = CommittedFirstTouchAcquisition {
        kind:             FirstTouchReservationAcquisitionKind::Appended,
        reservation_id:   prepared_claim.reservation_id,
        phase_start_head: prepared_claim.phase_start_head.clone(),
        scopes:           protected_scopes,
        conflicts:        conflict_outcome,
    };
    CommittedActionValidation::Append {
        operation: Box::new(prepared_claim.into_operation(ConflictAuthorization::NoConflict)),
        action:    acquisition,
    }
}

enum FirstTouchReservationReuse {
    AppendRequired {
        scopes:    ReservationScopeSet,
        conflicts: FirstTouchConflictOutcome,
    },
    Complete(CommittedActionValidation<FirstTouchClaimRejection, CommittedFirstTouchAcquisition>),
}

enum FirstTouchProtectedScopeOwnership<'reservation> {
    AlreadyHeld(&'reservation Reservation),
    Residual(ReservationScopeSet),
}

fn reuse_first_touch_reservation(
    reservations: &RetainedReservationSet,
    requested_scopes: ReservationScopeSet,
    protected_scopes: ReservationScopeSet,
    conflicts: FirstTouchConflictOutcome,
    coordination_run_id: CoordinationRunId,
    worktree_id: WorktreeId,
    path_case: PathCase,
) -> FirstTouchReservationReuse {
    let own_active_reservations = reservations
        .iter()
        .filter(|reservation| {
            is_own_active_blocking_reservation(reservation, coordination_run_id, worktree_id)
        })
        .collect::<Vec<_>>();
    let protected_scopes = match partition_first_touch_protected_scopes(
        &protected_scopes,
        &own_active_reservations,
        path_case,
    ) {
        FirstTouchProtectedScopeOwnership::AlreadyHeld(reservation) => {
            return already_held_first_touch(reservation, protected_scopes, conflicts);
        },
        FirstTouchProtectedScopeOwnership::Residual(residual) => residual,
    };
    let own_first_touch_reservation = own_active_reservations
        .iter()
        .copied()
        .find(|reservation| matches!(reservation.source(), ClaimSource::FirstTouch));
    let Some(reservation) = own_first_touch_reservation else {
        return FirstTouchReservationReuse::AppendRequired {
            scopes: protected_scopes,
            conflicts,
        };
    };
    let added = protected_scopes
        .as_slice()
        .iter()
        .filter(|candidate| {
            !reservation
                .scopes()
                .as_slice()
                .iter()
                .any(|held| held.contains(candidate, path_case))
        })
        .cloned()
        .collect::<Vec<_>>();
    let Ok(added_scopes) = ReservationScopeAdditionSet::try_from(added) else {
        return already_held_first_touch(reservation, protected_scopes, conflicts);
    };
    let validation = match reservations.bind_widened_scopes(reservation, &added_scopes, path_case) {
        WidenScopeBinding::Authorized(authorization) => CommittedActionValidation::Append {
            operation: Box::new(JournalOperation::Widen {
                reservation_id: reservation.id(),
                added_scopes,
                cause: WidenCause::Drift,
                authorization,
                edit_blocking_status: reservation.edit_blocking_status(),
            }),
            action:    CommittedFirstTouchAcquisition {
                kind: FirstTouchReservationAcquisitionKind::Widened,
                reservation_id: reservation.id(),
                phase_start_head: reservation.phase_start_head().clone(),
                scopes: protected_scopes,
                conflicts,
            },
        },
        WidenScopeBinding::Blocked(conflicts) => {
            CommittedActionValidation::Reject(FirstTouchClaimRejection::Blocked {
                scopes: requested_scopes,
                conflicts,
            })
        },
    };
    FirstTouchReservationReuse::Complete(validation)
}

fn partition_first_touch_protected_scopes<'reservation>(
    protected_scopes: &ReservationScopeSet,
    own_active_reservations: &[&'reservation Reservation],
    path_case: PathCase,
) -> FirstTouchProtectedScopeOwnership<'reservation> {
    let mut covering_reservations = Vec::new();
    let mut residual = Vec::new();
    for candidate in protected_scopes.as_slice() {
        match own_active_reservations.iter().copied().find(|reservation| {
            reservation
                .scopes()
                .as_slice()
                .iter()
                .any(|held| held.contains(candidate, path_case))
        }) {
            Some(reservation) => covering_reservations.push(reservation),
            None => residual.push(candidate.clone()),
        }
    }
    if let Ok(residual) = ReservationScopeSet::try_from(residual) {
        return FirstTouchProtectedScopeOwnership::Residual(residual);
    }

    // Replay order breaks ties: use the first participating `FirstTouch` reservation,
    // otherwise the first holder of the first protected scope in normalized set order.
    let reporting_reservation = own_active_reservations
        .iter()
        .copied()
        .filter(|reservation| matches!(reservation.source(), ClaimSource::FirstTouch))
        .find(|reservation| covering_reservations.contains(reservation))
        .unwrap_or(covering_reservations[0]);
    FirstTouchProtectedScopeOwnership::AlreadyHeld(reporting_reservation)
}

fn already_held_first_touch(
    reservation: &Reservation,
    scopes: ReservationScopeSet,
    conflicts: FirstTouchConflictOutcome,
) -> FirstTouchReservationReuse {
    FirstTouchReservationReuse::Complete(CommittedActionValidation::Reject(
        FirstTouchClaimRejection::AlreadyHeld(CommittedFirstTouchAcquisition {
            kind: FirstTouchReservationAcquisitionKind::AlreadyHeld,
            reservation_id: reservation.id(),
            phase_start_head: reservation.phase_start_head().clone(),
            scopes,
            conflicts,
        }),
    ))
}

fn is_own_active_blocking_reservation(
    reservation: &Reservation,
    coordination_run_id: CoordinationRunId,
    worktree_id: WorktreeId,
) -> bool {
    reservation.actor().run == coordination_run_id
        && reservation.actor().worktree == worktree_id
        && matches!(reservation.lifecycle(), ReservationLifecycle::Active)
        && reservation.edit_blocking_status() == EditBlockingStatus::Blocking
}

enum FirstTouchScopeDecision {
    Protect {
        scopes:    ReservationScopeSet,
        conflicts: FirstTouchConflictOutcome,
    },
    Block {
        scopes:    ReservationScopeSet,
        conflicts: Vec<ReservationConflict>,
    },
}

fn split_first_touch_scopes(
    requested_scopes: &ReservationScopeSet,
    conflicts: Vec<ReservationConflict>,
    conflict_handling: FirstTouchConflictHandling,
    path_case: PathCase,
) -> FirstTouchScopeDecision {
    if conflicts.is_empty() {
        return FirstTouchScopeDecision::Protect {
            scopes:    requested_scopes.clone(),
            conflicts: FirstTouchConflictOutcome::None,
        };
    }
    if matches!(conflict_handling, FirstTouchConflictHandling::RefuseRequest) {
        return FirstTouchScopeDecision::Block {
            scopes: requested_scopes.clone(),
            conflicts,
        };
    }
    let (blocked, free): (Vec<_>, Vec<_>) =
        requested_scopes
            .as_slice()
            .iter()
            .cloned()
            .partition(|candidate| {
                conflicts.iter().any(|conflict| {
                    conflict
                        .overlapping_scopes
                        .as_slice()
                        .iter()
                        .any(|held| held.overlaps(candidate, path_case))
                })
            });
    let Ok(scopes) = ReservationScopeSet::try_from(free) else {
        return FirstTouchScopeDecision::Block {
            scopes: requested_scopes.clone(),
            conflicts,
        };
    };
    let Ok(blocked_scopes) = ReservationScopeSet::try_from(blocked) else {
        return FirstTouchScopeDecision::Block {
            scopes: requested_scopes.clone(),
            conflicts,
        };
    };
    FirstTouchScopeDecision::Protect {
        scopes,
        conflicts: FirstTouchConflictOutcome::PostWriteIncursion {
            scopes: blocked_scopes,
            conflicts,
        },
    }
}

fn validate_first_touch_run(
    run_validation: ClaimRunValidation,
    reservations: &RetainedReservationSet,
    worktree_context: &WorktreeContext,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<(), FirstTouchClaimRejection> {
    match run_validation.validate(reservations, worktree_context, recovery_command_line) {
        Ok(()) => Ok(()),
        Err(ClaimRunValidationError::CoordinationIdentity(rejection)) => {
            Err(FirstTouchClaimRejection::CoordinationIdentity(rejection))
        },
        Err(ClaimRunValidationError::InvalidCanonicalWorktreeRoot) => {
            Err(FirstTouchClaimRejection::InvalidCanonicalWorktreeRoot)
        },
    }
}

fn validate_authorization(
    request: PermissiveOverlapAuthorizationRequest,
    conflicts: Vec<ReservationConflict>,
    requester: OverlapRequester,
    prepared_claim: PreparedClaim,
    ordering_graph: &OrderingGraph,
    maximum_ordering_edges: u32,
) -> TransactionValidation<ClaimRejection> {
    let PermissiveOverlapAuthorizationRequest {
        answer,
        reason,
        proposal_submission,
    } = request;
    let [conflict] = conflicts.as_slice() else {
        return TransactionValidation::Reject(ClaimRejection::Conflict(conflicts));
    };
    if conflict.reservation_id != answer.blocker() {
        return TransactionValidation::Reject(ClaimRejection::Conflict(conflicts));
    }
    let edge_effect = match &answer {
        PermissiveOverlapAnswer::Sequence { .. } => ClaimEdgeEffect::Adds,
        PermissiveOverlapAnswer::Defer { .. } | PermissiveOverlapAnswer::Override { .. } => {
            ClaimEdgeEffect::Unchanged
        },
    };
    let proposal =
        OverlapProposal::recompute(requester, reason, &prepared_claim.scopes, answer, conflict);
    match proposal_submission {
        OverlapProposalSubmission::Apply(proposal_token) if proposal_token.matches(&proposal) => {
            if matches!(edge_effect, ClaimEdgeEffect::Adds)
                && count_reaches_limit(ordering_graph.edge_count(), maximum_ordering_edges)
            {
                return TransactionValidation::Reject(ClaimRejection::OrderingEdgeLimitReached(
                    maximum_ordering_edges,
                ));
            }
            let authorization = ConflictAuthorization::from_approved_proposal(proposal);
            TransactionValidation::Append(Box::new(prepared_claim.into_operation(authorization)))
        },
        OverlapProposalSubmission::Mint | OverlapProposalSubmission::Apply(_) => {
            TransactionValidation::Reject(ClaimRejection::AuthorizationRequired(Box::new(
                proposal.escalation(conflicts),
            )))
        },
    }
}

#[derive(Clone, Copy)]
enum ClaimEdgeEffect {
    Unchanged,
    Adds,
}

fn count_reaches_limit(count: usize, maximum: u32) -> bool {
    u64::try_from(count).map_or(true, |count| count >= u64::from(maximum))
}

impl PreparedClaim {
    fn into_operation(self, authorization: ConflictAuthorization) -> JournalOperation {
        JournalOperation::Claim {
            reservation_id: self.reservation_id,
            scopes: self.scopes,
            source: self.source,
            purpose: self.purpose,
            trunk_at_claim: self.trunk_at_claim,
            head_snapshot: self.head_snapshot,
            phase_start_head: self.phase_start_head,
            worktree_root: self.worktree_root,
            worktree_administrative_locator: self.worktree_administrative_locator,
            authorization,
        }
    }
}

impl ClaimCoordinationRunSelection {
    const fn resolve(
        self,
        resolved_edit_authorization: ResolvedEditAuthorization,
    ) -> ClaimRunValidation {
        match self {
            Self::Specified(coordination_run_id) => {
                ClaimRunValidation::IndependentWithPresentedIdentity(coordination_run_id)
            },
            Self::ContinueOrStart => match resolved_edit_authorization.edit_authorization() {
                EditAuthorization::Session { .. } | EditAuthorization::Marker { .. } => {
                    ClaimRunValidation::ResolvedIdentityRequired(resolved_edit_authorization)
                },
                EditAuthorization::Environment {
                    coordination_run_id,
                    ..
                } => ClaimRunValidation::IndependentWithPresentedIdentity(coordination_run_id),
                EditAuthorization::Unidentified => {
                    ClaimRunValidation::IndependentWithoutPresentedIdentity {
                        actor_run_id: resolved_edit_authorization.coordination_run_id,
                    }
                },
            },
        }
    }
}

impl ClaimRunValidation {
    const fn requires_live_head_revalidation(self) -> bool {
        matches!(self, Self::ResolvedIdentityRequired(_))
    }

    const fn actor_run_id(self) -> CoordinationRunId {
        match self {
            Self::IndependentWithPresentedIdentity(actor_run_id)
            | Self::IndependentWithoutPresentedIdentity { actor_run_id } => actor_run_id,
            Self::ResolvedIdentityRequired(resolved_edit_authorization) => {
                resolved_edit_authorization.coordination_run_id
            },
        }
    }

    const fn presented_coordination_identity(self) -> RequesterCoordinationIdentity {
        match self {
            Self::IndependentWithPresentedIdentity(coordination_run_id) => {
                RequesterCoordinationIdentity::Presented(coordination_run_id)
            },
            Self::IndependentWithoutPresentedIdentity { .. } => {
                RequesterCoordinationIdentity::NotPresented
            },
            Self::ResolvedIdentityRequired(resolved_edit_authorization) => {
                RequesterCoordinationIdentity::Presented(
                    resolved_edit_authorization.coordination_run_id,
                )
            },
        }
    }

    fn validate(
        self,
        reservations: &RetainedReservationSet,
        worktree_context: &WorktreeContext,
        recovery_command_line: &RecoveryCommandLine,
    ) -> Result<(), ClaimRunValidationError> {
        match self {
            Self::IndependentWithPresentedIdentity(_)
            | Self::IndependentWithoutPresentedIdentity { .. } => Ok(()),
            Self::ResolvedIdentityRequired(resolved_edit_authorization) => {
                let identity_validation = CoordinationIdentityValidationContext::for_user_command(
                    resolved_edit_authorization,
                    worktree_context,
                    recovery_command_line,
                );
                validate_coordination_identity(reservations, &identity_validation)
                    .map_err(ClaimRunValidationError::from)
            },
        }
    }
}

impl From<CoordinationIdentityValidationError> for ClaimRunValidationError {
    fn from(error: CoordinationIdentityValidationError) -> Self {
        match error {
            CoordinationIdentityValidationError::Rejected(rejection) => {
                Self::CoordinationIdentity(rejection)
            },
            CoordinationIdentityValidationError::InvalidCanonicalWorktreeRoot => {
                Self::InvalidCanonicalWorktreeRoot
            },
        }
    }
}

impl From<ClaimRunValidationError> for ClaimRejection {
    fn from(error: ClaimRunValidationError) -> Self {
        match error {
            ClaimRunValidationError::CoordinationIdentity(rejection) => {
                Self::CoordinationIdentity(rejection)
            },
            ClaimRunValidationError::InvalidCanonicalWorktreeRoot => {
                Self::InvalidCanonicalWorktreeRoot
            },
        }
    }
}

impl ClaimRepositoryFacts {
    fn read(
        worktree_context: &WorktreeContext,
        run_validation: ClaimRunValidation,
    ) -> Result<Self, ClaimError> {
        let (current_head, head_snapshot) = if run_validation.requires_live_head_revalidation() {
            read_live_head_snapshot(worktree_context)?
        } else {
            read_head_snapshot_from_files(worktree_context)?
        };
        let worktree_root = worktree_context
            .repository_root()
            .to_str()
            .ok_or(ClaimError::NonUtf8WorktreeRoot)?
            .parse()
            .map_err(|_| ClaimError::InvalidCanonicalWorktreeRoot)?;
        Ok(Self {
            head_snapshot,
            current_head,
            worktree_root,
        })
    }
}

fn read_live_head_snapshot(
    worktree_context: &WorktreeContext,
) -> Result<(GitObjectId, ClaimHeadSnapshot), ClaimError> {
    let current_head = crate::reservation::current_head(worktree_context.repository_root())?;
    let head_snapshot = match git::head_attachment(worktree_context.repository_root())? {
        git::HeadAttachment::Branch { full_ref } => ClaimHeadSnapshot::Branch {
            full_ref,
            head: ClaimHeadCommit::from(current_head.clone()),
        },
        git::HeadAttachment::Detached => ClaimHeadSnapshot::Detached {
            head: ClaimHeadCommit::from(current_head.clone()),
        },
    };
    Ok((current_head, head_snapshot))
}

fn read_trunk_commit(
    worktree_context: &WorktreeContext,
    trunk: &str,
    _source: &ClaimSource,
) -> Result<TrunkObservationAtClaim, ClaimError> {
    let trunk_ref = format!("{HEADS_REF_PREFIX}{trunk}");
    match read_reference(worktree_context, &trunk_ref) {
        Ok(trunk_commit) => Ok(TrunkObservationAtClaim::from(trunk_commit)),
        Err(ClaimError::MissingReference(reference)) => reference
            .parse::<FullRefName>()
            .map(TrunkObservationAtClaim::from)
            .map_err(|_| ClaimError::InvalidTrunkReference),
        Err(error) => Err(error),
    }
}

fn read_head_snapshot_from_files(
    worktree_context: &WorktreeContext,
) -> Result<(GitObjectId, ClaimHeadSnapshot), ClaimError> {
    let head = fs::read_to_string(worktree_context.administrative_directory().join("HEAD"))?;
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref: ") {
        let full_ref = reference.parse().or_else(|_| {
            git::symbolic_head_reference(worktree_context.repository_root())
                .map_err(|_| ClaimError::InvalidHeadReference)
        })?;
        let current_head = read_reference(worktree_context, &full_ref.to_string())?;
        return Ok((
            current_head.clone(),
            ClaimHeadSnapshot::Branch {
                full_ref,
                head: ClaimHeadCommit::from(current_head),
            },
        ));
    }
    let current_head: GitObjectId = head.parse().map_err(ClaimError::InvalidGitObjectId)?;
    Ok((
        current_head.clone(),
        ClaimHeadSnapshot::Detached {
            head: ClaimHeadCommit::from(current_head),
        },
    ))
}

fn read_reference(
    worktree_context: &WorktreeContext,
    reference: &str,
) -> Result<GitObjectId, ClaimError> {
    match read_reference_from_files(worktree_context.common_git_directory(), reference) {
        FilesystemReferenceResolution::Resolved(object_id) => Ok(object_id),
        FilesystemReferenceResolution::RequiresGitResolution {
            rejection_if_git_reports_missing,
        } => resolve_reference_through_git(
            worktree_context,
            reference,
            rejection_if_git_reports_missing,
        ),
    }
}

fn resolve_reference_through_git(
    worktree_context: &WorktreeContext,
    reference: &str,
    rejection_if_git_reports_missing: ClaimError,
) -> Result<GitObjectId, ClaimError> {
    match git::reference_lookup(worktree_context.repository_root(), reference)? {
        ReferenceLookup::Present(object_id) => Ok(object_id),
        ReferenceLookup::Missing => Err(rejection_if_git_reports_missing),
    }
}

fn read_reference_from_files(
    common_git_directory: &Path,
    reference: &str,
) -> FilesystemReferenceResolution {
    read_reference_from_files_at_depth(
        common_git_directory,
        reference,
        SymbolicReferenceDepth::ROOT,
    )
}

fn read_reference_from_files_at_depth(
    common_git_directory: &Path,
    reference: &str,
    depth: SymbolicReferenceDepth,
) -> FilesystemReferenceResolution {
    match fs::read_to_string(common_git_directory.join(reference)) {
        Ok(value) => parse_reference_value(common_git_directory, reference, &value, depth),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            match read_packed_reference(common_git_directory, reference) {
                Ok(object_id) => FilesystemReferenceResolution::Resolved(object_id),
                Err(ClaimError::MissingReference(_)) => {
                    FilesystemReferenceResolution::RequiresGitResolution {
                        rejection_if_git_reports_missing: ClaimError::MissingReference(
                            reference.to_owned(),
                        ),
                    }
                },
                Err(error) => FilesystemReferenceResolution::RequiresGitResolution {
                    rejection_if_git_reports_missing: error,
                },
            }
        },
        Err(error) if error.kind() == ErrorKind::NotADirectory => {
            FilesystemReferenceResolution::RequiresGitResolution {
                rejection_if_git_reports_missing: ClaimError::MissingReference(
                    reference.to_owned(),
                ),
            }
        },
        Err(error) => FilesystemReferenceResolution::RequiresGitResolution {
            rejection_if_git_reports_missing: ClaimError::Io(error),
        },
    }
}

fn parse_reference_value(
    common_git_directory: &Path,
    reference: &str,
    value: &str,
    depth: SymbolicReferenceDepth,
) -> FilesystemReferenceResolution {
    let value = value.trim();
    if let Some(target) = value.strip_prefix("ref: ") {
        return match depth.descend(reference) {
            Ok(depth) => read_reference_from_files_at_depth(common_git_directory, target, depth),
            Err(error) => FilesystemReferenceResolution::RequiresGitResolution {
                rejection_if_git_reports_missing: error,
            },
        };
    }
    value.parse().map_or_else(
        |_| FilesystemReferenceResolution::RequiresGitResolution {
            rejection_if_git_reports_missing: ClaimError::InvalidStoredReference(
                reference.to_owned(),
            ),
        },
        FilesystemReferenceResolution::Resolved,
    )
}

fn read_packed_reference(
    common_git_directory: &Path,
    reference: &str,
) -> Result<GitObjectId, ClaimError> {
    let packed_references = match fs::read_to_string(common_git_directory.join("packed-refs")) {
        Ok(packed_references) => packed_references,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(ClaimError::MissingReference(reference.to_owned()));
        },
        Err(error) => return Err(ClaimError::Io(error)),
    };
    let object_id = packed_references.lines().find_map(|line| {
        let (object_id, name) = line.split_once(' ')?;
        (name == reference).then_some(object_id)
    });
    object_id.map_or_else(
        || Err(ClaimError::MissingReference(reference.to_owned())),
        |object_id| {
            object_id
                .parse()
                .map_err(|_| ClaimError::InvalidStoredReference(reference.to_owned()))
        },
    )
}

enum ClaimRejection {
    Conflict(Vec<ReservationConflict>),
    AuthorizationRequired(Box<OverlapEscalationPayload>),
    Replay(ReservationReplayError),
    CoordinationIdentity(CoordinationIdentityRejection),
    InvalidCanonicalWorktreeRoot,
    ReservationLimitReached(u32),
    OrderingEdgeLimitReached(u32),
    EdgeReplay(EdgeReplayError),
}

#[derive(Debug)]
pub(crate) enum ClaimError {
    Io(std::io::Error),
    Git(GitError),
    Config(ConfigError),
    Ledger(LedgerError),
    PathCase(PathCaseError),
    Transaction(LedgerTransactionError),
    ReservationReplay(ReservationReplayError),
    EdgeReplay(EdgeReplayError),
    CoordinationIdentity(CoordinationIdentityRejection),
    InvalidGitObjectId(InvalidGitObjectId),
    InvalidHeadReference,
    InvalidTrunkReference,
    MissingReference(String),
    InvalidStoredReference(String),
    SymbolicReferenceDepthExceeded { reference: String, maximum: u8 },
    NonUtf8WorktreeRoot,
    InvalidCanonicalWorktreeRoot,
}

impl Display for ClaimError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "claim I/O failed: {error}"),
            Self::Git(error) => error.fmt(formatter),
            Self::Config(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::PathCase(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::ReservationReplay(error) => {
                write!(formatter, "reservation replay failed: {error}")
            },
            Self::EdgeReplay(error) => write!(formatter, "ordering replay failed: {error}"),
            Self::CoordinationIdentity(rejection) => rejection.fmt(formatter),
            Self::InvalidGitObjectId(error) => error.fmt(formatter),
            Self::InvalidHeadReference => {
                formatter.write_str("git returned an invalid full HEAD reference")
            },
            Self::InvalidTrunkReference => {
                formatter.write_str("the configured trunk is not a valid full git reference")
            },
            Self::MissingReference(reference) => {
                write!(formatter, "git reference {reference} does not exist")
            },
            Self::InvalidStoredReference(reference) => {
                write!(
                    formatter,
                    "git reference {reference} does not contain a full object id"
                )
            },
            Self::SymbolicReferenceDepthExceeded { reference, maximum } => write!(
                formatter,
                "git symbolic reference resolution reached its maximum depth of {maximum} at {reference}"
            ),
            Self::NonUtf8WorktreeRoot => {
                formatter.write_str("the canonical worktree root is not UTF-8")
            },
            Self::InvalidCanonicalWorktreeRoot => {
                formatter.write_str("the worktree root is not a canonical absolute path")
            },
        }
    }
}

impl std::error::Error for ClaimError {}

impl ClaimError {
    pub(crate) fn into_output(self, command_verb: CommandVerb) -> OutputEnvelope {
        match self {
            Self::Transaction(error) => match error {
                LedgerTransactionError::CorrectableInput(error) => {
                    OutputEnvelope::invalid_input(command_verb, &error.to_string())
                },
                LedgerTransactionError::LockContention => {
                    OutputEnvelope::contention(command_verb, &error.to_string())
                },
                LedgerTransactionError::LedgerUnreadable(error) => {
                    OutputEnvelope::ledger_error(command_verb, &error)
                },
            },
            Self::CoordinationIdentity(rejection) => {
                OutputEnvelope::coordination_identity_rejected(command_verb, rejection)
            },
            Self::Config(error) => {
                OutputEnvelope::ledger_error(command_verb, &LedgerError::Config(error))
            },
            Self::Ledger(error) => OutputEnvelope::ledger_error(command_verb, &error),
            Self::ReservationReplay(error) => OutputEnvelope::replay_failure(command_verb, &error),
            error => OutputEnvelope::ledger_unreadable(command_verb, &error.to_string()),
        }
    }
}

impl From<std::io::Error> for ClaimError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<GitError> for ClaimError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

impl From<ConfigError> for ClaimError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<LedgerError> for ClaimError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}

impl From<PathCaseError> for ClaimError {
    fn from(error: PathCaseError) -> Self { Self::PathCase(error) }
}

impl From<LedgerTransactionError> for ClaimError {
    fn from(error: LedgerTransactionError) -> Self { Self::Transaction(error) }
}

#[cfg(test)]
mod tests {
    use super::ClaimCoordinationRunSelection;
    use super::ClaimRunValidation;
    use crate::ids::CoordinationRunId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::EditAuthorization;
    use crate::ledger::ResolvedEditAuthorization;

    #[test]
    fn continue_or_start_uses_the_session_authorization_resolved_with_the_actor() {
        let coordination_run_id = CoordinationRunId::new();
        let reservation_id = ReservationId::new();
        let worktree_id = WorktreeId::new();
        let resolved_edit_authorization = ResolvedEditAuthorization::for_edit_authorization(
            worktree_id,
            EditAuthorization::Session {
                coordination_run_id,
                reservation_id,
                worktree_id,
            },
        );

        let run_validation =
            ClaimCoordinationRunSelection::ContinueOrStart.resolve(resolved_edit_authorization);

        assert!(matches!(
            run_validation,
            ClaimRunValidation::ResolvedIdentityRequired(actual)
                if actual == resolved_edit_authorization
        ));
    }
}
