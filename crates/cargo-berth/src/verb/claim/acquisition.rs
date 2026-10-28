//! Atomic ordinary, named and first-touch reservation acquisition.

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::ClaimAnswerPolicy;
use super::authorization;
use super::authorization::AnswerKind;
use super::authorization::AnsweringWorktree;
use super::authorization::OrderingContext;
use super::failure::ClaimError;
use super::failure::ClaimRejection;
use super::incomplete_answer::IncompleteClaimAnswerRejection;
use super::phase_candidate::PhaseReservationCandidate;
use super::recovery_command::ClaimRecoveryCommand;
use super::repository_facts;
use super::repository_facts::ClaimRepositoryFacts;
use super::request::ClaimCoordinationRunSelection;
use super::request::ClaimRequest;
use super::request::ClaimReservationSelection;
use super::request::ClaimRunValidation;
use super::request::ClaimRunValidationError;
use super::request::ClaimValidationContext;
use crate::answer::ClaimSequenceBinding;
use crate::answer::ConflictAuthorization;
use crate::answer::ContradictedOrdering;
use crate::answer::OverlapApprover;
use crate::answer::OverlapAuthorizationRequest;
use crate::answer::PermissiveOverlapAuthorizationRequest;
use crate::config::BerthConfig;
use crate::config::DefaultAnswer;
use crate::config::Enrollment;
use crate::coordination_identity::CoordinationIdentityProvenance;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::coordination_identity::RecoveryCommandLine;
use crate::edge::CycleFreeOrderingOperation;
use crate::edge::OrderingCycle;
use crate::edge::OrderingDeclarationError;
use crate::git;
use crate::git::RepositorySnapshot;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WireOrderedReservationIds;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::ClaimTarget;
use crate::ledger::CommittedActionValidation;
use crate::ledger::EditAuthorization;
use crate::ledger::IntegrationTarget;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::RefusedVerb;
use crate::ledger::ReplayedLedgerState;
use crate::ledger::ReservationPurpose;
use crate::ledger::ReservationScope;
use crate::ledger::ReservationScopeAdditionSet;
use crate::ledger::ReservationScopeSet;
use crate::ledger::ResolvedEditAuthorization;
use crate::ledger::ResolvedJournalMutationActor;
use crate::ledger::TargetSelectionRequest;
use crate::ledger::TransactionValidation;
use crate::ledger::TrunkObservationAtClaim;
use crate::ledger::WidenCause;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::CoordinationRunMarkerPublication;
use crate::output::OutputEnvelope;
use crate::output::TargetView;
use crate::reconcile;
use crate::reconcile::ReconciliationReport;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation::ActingHeadContainment;
use crate::reservation::RecordedTarget;
use crate::reservation::Reservation;
use crate::reservation::ReservationConflict;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::reservation::WidenScopeBinding;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;
use crate::session;
use crate::session::FirstTouchSessionReservationMapping;
use crate::session::HarnessSessionIdentity;
use crate::session::PreparedSessionMappingPublication;
use crate::session::SessionIdentityMappingPublication;
use crate::session::SessionReservationIdentity;
use crate::verb::refusal;

/// Maintenance committed while a claim retries a full ledger, reported by this invocation.
#[derive(Default)]
pub(crate) struct ClaimCapacityRecovery {
    reports: Vec<ReconciliationReport>,
}

impl ClaimCapacityRecovery {
    fn recover(
        &mut self,
        repository: &RepositorySnapshot,
        ledger: &Ledger,
        config: &BerthConfig,
    ) -> Result<(), ClaimError> {
        let report = reconcile::reconcile_for_claim_capacity(repository, ledger, config)
            .map_err(|error| ClaimError::CapacityRecovery(Box::new(error)))?;
        self.reports.push(report);
        Ok(())
    }

    pub(crate) fn into_output(self, output: OutputEnvelope) -> OutputEnvelope {
        self.reports
            .into_iter()
            .fold(output, |output, report| report.into_output(output))
    }
}

#[derive(Clone)]
struct PreparedClaim {
    reservation_id:                   ReservationId,
    scopes:                           ReservationScopeSet,
    source:                           ClaimSource,
    purpose:                          ReservationPurpose,
    trunk_at_claim:                   TrunkObservationAtClaim,
    target:                           ClaimTarget,
    head_snapshot:                    ClaimHeadSnapshot,
    phase_start_head:                 ProtectedPhaseStartHead,
    worktree_root:                    CanonicalWorktreeRoot,
    worktree_administrative_locator:  WorktreeAdministrativeLocator,
    coordination_identity_provenance: CoordinationIdentityProvenance,
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
    let ClaimRequest {
        reservation_selection,
        target: _,
        declared_scopes: _,
        source: _,
        purpose: _,
        coordination_run_selection: _,
        phase_start: _,
        overlap_authorization: _,
        answer_recovery,
    } = &claim_request;
    let answer_recovery = answer_recovery.clone();
    let reservation_selection = *reservation_selection;
    let mut capacity_recovery = ClaimCapacityRecovery::default();
    let output = reconciliation_report.into_output(claim_acquisition_output(
        acquire(claim_request, recovery_command_line, &mut capacity_recovery),
        reservation_selection,
        &answer_recovery,
    ));
    capacity_recovery.into_output(output)
}

/// Render one acquisition result while preserving its blocked envelope contract.
fn claim_acquisition_output(
    outcome: Result<Enrollment<ClaimExecution>, ClaimError>,
    reservation_selection: ClaimReservationSelection,
    answer_recovery: &ClaimRecoveryCommand,
) -> OutputEnvelope {
    match outcome {
        Ok(Enrollment::Enrolled(ClaimExecution::Selected {
            acquisition,
            scopes,
        })) => OutputEnvelope::selected_claim(acquisition, scopes),
        Ok(Enrollment::Enrolled(ClaimExecution::Claimed {
            reservation_id,
            target,
            coordination_run_id,
            scopes,
            marker_publication,
            session_mapping_publication,
        })) => OutputEnvelope::claimed(
            reservation_id,
            target,
            coordination_run_id,
            scopes,
            marker_publication,
            session_mapping_publication,
        ),
        Ok(Enrollment::Enrolled(ClaimExecution::Answered {
            reservation_id,
            blockers,
            scopes,
            answer_policy,
        })) => OutputEnvelope::answered(reservation_id, blockers, scopes, answer_policy),
        Ok(Enrollment::Enrolled(ClaimExecution::Blocked {
            conflicts,
            approver,
        })) => OutputEnvelope::blocked_claim(conflicts, approver),
        Ok(Enrollment::Enrolled(ClaimExecution::IncompleteAnswer {
            rejection,
            approver,
        })) => {
            let message = rejection.message(answer_recovery);
            OutputEnvelope::blocked_claim_with_answer_recovery(
                rejection.conflicts,
                approver,
                message,
            )
        },
        Ok(Enrollment::Enrolled(ClaimExecution::AnsweredWithoutOverlap(blockers))) => {
            OutputEnvelope::invalid_input(
                CommandVerb::Claim,
                &format!(
                    "No foreign reservation overlaps the requested paths, so the overlap answer naming {} has nothing to authorize. Claim the same paths again with no overlap answer.",
                    named_reservations(&blockers)
                ),
            )
        },
        Ok(Enrollment::Enrolled(ClaimExecution::ContradictsLiveOrdering(contradicted))) => {
            OutputEnvelope::claim_contradicts_live_ordering(contradicted)
        },
        Ok(Enrollment::Enrolled(ClaimExecution::OrderingCycle(cycle))) => {
            OutputEnvelope::ordering_cycle_refused(CommandVerb::Claim, &cycle)
        },
        Ok(Enrollment::Enrolled(ClaimExecution::NothingToReplace(blockers))) => {
            OutputEnvelope::invalid_input(
                CommandVerb::Claim,
                &format!(
                    "--replace found no live ordering edge between {} and {} that points the other way, so it has nothing to replace. Claim the same paths again without --replace.",
                    named_reservations(&blockers),
                    match reservation_selection {
                        ClaimReservationSelection::NewReservation =>
                            "an earlier reservation of this run in this worktree".to_owned(),
                        ClaimReservationSelection::Existing(id) =>
                            format!("selected reservation {id}"),
                    }
                ),
            )
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
    }
}

/// Name one reservation as `reservation <id>`, or several as `reservations <id>, <id>`.
fn named_reservations(reservation_ids: &[ReservationId]) -> String {
    match reservation_ids {
        [reservation_id] => format!("reservation {reservation_id}"),
        _ => format!(
            "reservations {}",
            reservation_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

enum ClaimExecution {
    Selected {
        acquisition: ReservationAcquisition,
        scopes:      ReservationScopeSet,
    },
    Claimed {
        reservation_id:              ReservationId,
        target:                      TargetView,
        coordination_run_id:         CoordinationRunId,
        scopes:                      ReservationScopeSet,
        marker_publication:          CoordinationRunMarkerPublication,
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    /// A `--defer` claim, or an unanswered one under the repository's default answer, recorded
    /// its answers without acquiring a reservation.
    Answered {
        reservation_id: Option<ReservationId>,
        blockers:       WireOrderedReservationIds,
        scopes:         ReservationScopeSet,
        /// Whether the repository default or the caller's explicit `--defer` recorded answers.
        answer_policy:  ClaimAnswerPolicy,
    },
    Blocked {
        conflicts: Vec<ReservationConflict>,
        approver:  Option<OverlapApprover>,
    },
    IncompleteAnswer {
        rejection: Box<IncompleteClaimAnswerRejection>,
        approver:  Option<OverlapApprover>,
    },
    AnsweredWithoutOverlap(Vec<ReservationId>),
    ContradictsLiveOrdering(Vec<ContradictedOrdering>),
    OrderingCycle(OrderingCycle),
    NothingToReplace(Vec<ReservationId>),
    ReservationLimitReached(u32),
    OrderingEdgeLimitReached(u32),
}

/// How one successful transaction established coverage or recorded a named answer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReservationAcquisitionKind {
    /// The transaction appended a new first-touch reservation.
    Appended,
    /// The transaction enlarged the acting run's existing first-touch reservation.
    Widened,
    /// The acting run's reservations in this worktree already covered every claimable path.
    AlreadyHeld,
    /// An explicit claim recorded an answer without acquiring paths.
    Answered,
}

/// The durable identity and publication result of reservation acquisition or a named answer.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "FirstTouchReservationAcquisition", transform = crate::output::closed_value_serializes_as_object)]
pub(crate) struct ReservationAcquisition {
    /// How the transaction established coverage.
    pub(crate) kind:                        ReservationAcquisitionKind,
    /// The reservation selected by the transaction.
    pub(crate) reservation_id:              ReservationId,
    /// The target retained by the acquired reservation.
    target:                                 Box<TargetView>,
    /// The coordination run that owns the reservation.
    coordination_run_id:                    CoordinationRunId,
    /// The original phase-start commit retained by the reservation.
    #[schemars(with = "String", length(min = 1))]
    phase_start_head:                       ProtectedPhaseStartHead,
    /// Whether the worktree marker records the coordination run.
    marker_publication:                     CoordinationRunMarkerPublication,
    /// Whether the harness session mapping records the reservation.
    pub(crate) session_mapping_publication: SessionIdentityMappingPublication,
    /// Requested paths that active siblings already protect instead of this reservation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) already_held:                Vec<SiblingHeldScopes>,
}

/// The requested paths already protected by one active sibling in this run and worktree.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct SiblingHeldScopes {
    /// The sibling that already holds these paths.
    pub(crate) reservation_id: ReservationId,
    /// Exact requested scopes covered by the sibling.
    pub(crate) scopes:         ReservationScopeSet,
}

/// Whether a first-touch transaction may protect the nonconflicting part of a post-write request.
#[derive(Clone, Copy)]
pub(crate) enum FirstTouchConflictHandling {
    /// Refuse the complete request when any path has a foreign holder.
    RefuseRequest,
    /// Protect every free path and return the foreign-held subset for reporting.
    ProtectFreePaths,
}

/// How a `check` invocation selects an active reservation during locked first-touch validation.
#[derive(Clone, Copy)]
pub(crate) enum CheckReservationSelection {
    /// Use a valid harness-session mapping or the acting run's only active reservation.
    SessionMappingOrSingleActive,
    /// Use a valid mapping, open new work for an unmapped session, or join sole active work
    /// when no usable harness session id is available.
    SessionMappingOrNew,
    /// Continue the named active reservation held by the acting run and worktree.
    Explicit(ReservationId),
}

/// Whether a lock-free check can reuse the reservation its caller selected.
pub(in crate::verb) enum CheckAcquisitionDecision {
    /// The selected active reservation's run already protects the complete edit.
    HeldBySelectedReservation {
        /// The exact scopes that the locked acquisition would report.
        scopes:      ReservationScopeSet,
        /// The same already-held acquisition reported by the locked path.
        acquisition: ReservationAcquisition,
    },
    /// Selection or coverage requires the ordinary locked first-touch decision.
    NeedsLockedDecision,
}

/// Reuse the locked path's coverage and acquisition rules without publishing any state.
pub(in crate::verb) fn selected_reservation_check_acquisition(
    reservations: &RetainedReservationSet,
    repository_trunk: &IntegrationTarget,
    scopes: ReservationScopeSet,
    selection: CheckReservationSelection,
    identity: ResolvedEditAuthorization,
    path_case: PathCase,
) -> CheckAcquisitionDecision {
    // An explicit switch must publish the session's new selection under the lock.
    let reservation_id = match (selection, identity.edit_authorization()) {
        (
            CheckReservationSelection::Explicit(id),
            EditAuthorization::Session { reservation_id, .. },
        ) if id == reservation_id => id,
        (CheckReservationSelection::Explicit(id), _)
            if matches!(
                session::current_process_harness_session(),
                HarnessSessionIdentity::Unavailable
            ) =>
        {
            id
        },
        (
            CheckReservationSelection::SessionMappingOrSingleActive
            | CheckReservationSelection::SessionMappingOrNew,
            EditAuthorization::Session { reservation_id, .. },
        ) => reservation_id,
        _ => return CheckAcquisitionDecision::NeedsLockedDecision,
    };
    let FirstTouchReservationSelection::ExplicitReservation(reservation) =
        FirstTouchReservationSelection::from_explicit_selection(
            reservations,
            reservation_id,
            identity.coordination_run_id,
            identity.worktree_id,
        )
    else {
        return CheckAcquisitionDecision::NeedsLockedDecision;
    };
    let FirstTouchProtectedScopeOwnership::AlreadyHeld(reservation) =
        partition_first_touch_protected_scopes(reservations, &scopes, reservation, path_case)
    else {
        return CheckAcquisitionDecision::NeedsLockedDecision;
    };
    let committed = already_held_acquisition(
        reservation,
        repository_trunk,
        scopes,
        FirstTouchConflictOutcome::None,
    );
    let (acquisition, scopes, _) = first_touch_acquisition(
        committed,
        identity.coordination_run_id,
        CoordinationRunMarkerPublication::Published,
        selection
            .reported_session_mapping_publication(SessionIdentityMappingPublication::Published),
    );
    CheckAcquisitionDecision::HeldBySelectedReservation {
        scopes,
        acquisition,
    }
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
    /// The repository's default answer authorized the edit against these foreign holders before
    /// every requested path was protected.
    AnsweredByDefault {
        /// Every holder the recorded answer named.
        answered: WireOrderedReservationIds,
    },
}

/// The states a first-touch request can produce without permissive authorization.
pub(crate) enum FirstTouchClaimExecution {
    /// The acting run holds a first-touch reservation over every returned scope.
    Acquired {
        /// The durable reservation identity, baseline, and publication results.
        acquisition: ReservationAcquisition,
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

/// The reservation identity chosen for one locked first-touch validation.
enum FirstTouchReservationSelection<'reservation> {
    /// The `check --reservation` selector names this eligible active reservation.
    ExplicitReservation(&'reservation Reservation),
    /// The `check --reservation` selector does not name eligible active work.
    IneligibleExplicitReservation(ReservationId),
    /// The current harness-session mapping names this active reservation.
    SessionMappedReservation(&'reservation Reservation),
    /// No usable mapping exists, and this is the run's only active reservation.
    SingleActiveRunReservation(&'reservation Reservation),
    /// No usable mapping exists, and the run has no active reservation.
    NoActiveRunReservation,
    /// No usable mapping exists, and several active reservations are eligible.
    AmbiguousActiveRunReservations {
        /// Every eligible reservation id in deterministic order.
        candidates: Vec<PhaseReservationCandidate>,
    },
}

impl<'reservation> FirstTouchReservationSelection<'reservation> {
    /// Resolve an explicit id only within the acting coordination run and worktree.
    fn from_explicit_selection(
        reservations: &'reservation RetainedReservationSet,
        reservation_id: ReservationId,
        coordination_run_id: CoordinationRunId,
        worktree_id: WorktreeId,
    ) -> Self {
        reservations
            .iter()
            .find(|reservation| {
                reservation.id() == reservation_id
                    && reservation.is_active_for_coordination_run_and_worktree(
                        coordination_run_id,
                        worktree_id,
                    )
            })
            .map_or(
                Self::IneligibleExplicitReservation(reservation_id),
                Self::ExplicitReservation,
            )
    }

    /// Choose the reservation named by the prepared harness-session mapping.
    ///
    /// The transaction checks the mapping publication identity under the lock and prepares again
    /// if it moved. The acting coordination run is the invoking process's own identity,
    /// resolved before the lock. A mapping naming a different run is therefore treated as no
    /// usable mapping and falls through to the unmapped selection, which admits only reservations
    /// belonging to the acting run — so a mapping written by another run can never redirect this
    /// widen onto that run's reservation.
    fn from_locked_session_mapping(
        reservations: &'reservation RetainedReservationSet,
        session_reservation_identity: SessionReservationIdentity,
        ledger_directory: &Path,
        coordination_run_id: CoordinationRunId,
        worktree_id: WorktreeId,
    ) -> Self {
        if session_reservation_identity.coordination_run_id() == coordination_run_id
            && let Some(reservation) = reservations.iter().find(|reservation| {
                reservation.id() == session_reservation_identity.reservation_id()
                    && reservation.is_active_for_coordination_run_and_worktree(
                        coordination_run_id,
                        worktree_id,
                    )
            })
        {
            return Self::SessionMappedReservation(reservation);
        }
        Self::from_unmapped_active_run_reservations(
            reservations,
            ledger_directory,
            coordination_run_id,
            worktree_id,
        )
    }

    fn from_unmapped_active_run_reservations(
        reservations: &'reservation RetainedReservationSet,
        ledger_directory: &Path,
        coordination_run_id: CoordinationRunId,
        worktree_id: WorktreeId,
    ) -> Self {
        let active_run_reservations = reservations
            .iter()
            .filter(|reservation| {
                reservation
                    .is_active_for_coordination_run_and_worktree(coordination_run_id, worktree_id)
            })
            .collect::<Vec<_>>();
        match active_run_reservations.as_slice() {
            [] => Self::NoActiveRunReservation,
            [reservation] => Self::SingleActiveRunReservation(reservation),
            [_, _, ..] => {
                let mappings =
                    session::phase_reservation_sessions(ledger_directory, coordination_run_id);
                let ids = WireOrderedReservationIds::sorted(
                    active_run_reservations
                        .iter()
                        .map(|reservation| reservation.id())
                        .collect(),
                );
                let candidates = ids
                    .as_slice()
                    .iter()
                    .map(|id| {
                        mappings.as_ref().map_or_else(
                            |_| PhaseReservationCandidate::without_session_label(*id),
                            |mappings| {
                                PhaseReservationCandidate::from_sessions(
                                    *id,
                                    mappings.get(id).cloned().unwrap_or_default(),
                                )
                            },
                        )
                    })
                    .collect();
                Self::AmbiguousActiveRunReservations { candidates }
            },
        }
    }
}

struct CommittedReservationAcquisition {
    kind:             ReservationAcquisitionKind,
    reservation_id:   ReservationId,
    target:           Box<TargetView>,
    phase_start_head: ProtectedPhaseStartHead,
    scopes:           ReservationScopeSet,
    conflicts:        FirstTouchConflictOutcome,
    already_held:     Vec<SiblingHeldScopes>,
}

#[derive(Clone)]
struct FirstTouchValidationContext {
    repository:            Arc<RepositorySnapshot>,
    run_validation:        ClaimRunValidation,
    worktree_context:      WorktreeContext,
    recovery_command_line: RecoveryCommandLine,
    coordination_run_id:   CoordinationRunId,
    worktree_id:           WorktreeId,
    path_case:             PathCase,
    /// Refuse every part of the declared edit before acquiring exact-file race scope.
    edit_scopes:           ReservationScopeSet,
    conflict_handling:     FirstTouchConflictHandling,
    reservation_selection: CheckReservationSelection,
    maximum_reservations:  u32,
    repository_trunk:      IntegrationTarget,
}

#[derive(Clone, Copy)]
struct FirstTouchReservationReuseContext<'ledger> {
    ledger_directory:                        &'ledger Path,
    first_touch_session_reservation_mapping: FirstTouchSessionReservationMapping,
    check_reservation_selection:             CheckReservationSelection,
    coordination_run_id:                     CoordinationRunId,
    worktree_id:                             WorktreeId,
    path_case:                               PathCase,
}

enum FirstTouchClaimRejection {
    Blocked {
        scopes:    ReservationScopeSet,
        conflicts: Vec<ReservationConflict>,
    },
    AlreadyHeld {
        acquisition:     Box<CommittedReservationAcquisition>,
        session_mapping: AlreadyHeldSessionMapping,
    },
    Replay(ReservationReplayError),
    CoordinationIdentity(CoordinationIdentityRejection),
    InvalidCanonicalWorktreeRoot,
    ReservationLimitReached(u32),
    IneligibleExplicitReservation(ReservationId),
    AmbiguousActiveRunReservations {
        candidates: Vec<PhaseReservationCandidate>,
    },
}

/// Whether covered paths select a reservation for the current harness session.
enum AlreadyHeldSessionMapping {
    /// Keep a fresh phase session unmapped when siblings cover its entire request.
    PreserveUnmappedSession,
    /// Persist the reservation the invocation selected.
    PublishSelectedReservation,
}

fn acquire(
    claim_request: ClaimRequest,
    recovery_command_line: &RecoveryCommandLine,
    capacity_recovery: &mut ClaimCapacityRecovery,
) -> Result<Enrollment<ClaimExecution>, ClaimError> {
    let ClaimRequest {
        reservation_selection,
        target: _,
        declared_scopes: _,
        source: _,
        purpose: _,
        coordination_run_selection: _,
        phase_start: _,
        overlap_authorization: _,
        answer_recovery: _,
    } = &claim_request;
    match *reservation_selection {
        ClaimReservationSelection::Existing(reservation_id) => {
            acquire_named(claim_request, reservation_id, recovery_command_line)
        },
        ClaimReservationSelection::NewReservation => {
            acquire_new(claim_request, recovery_command_line, capacity_recovery)
        },
    }
}

fn acquire_new(
    claim_request: ClaimRequest,
    recovery_command_line: &RecoveryCommandLine,
    capacity_recovery: &mut ClaimCapacityRecovery,
) -> Result<Enrollment<ClaimExecution>, ClaimError> {
    let ClaimRequest {
        reservation_selection: _,
        target,
        declared_scopes,
        source,
        purpose,
        coordination_run_selection,
        phase_start,
        overlap_authorization,
        answer_recovery: _,
    } = claim_request;
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let resolved_edit_authorization = ledger::resolve_identity(&worktree_context)?;
    let claim_run_validation = coordination_run_selection.resolve(resolved_edit_authorization);
    let actor_run_id = claim_run_validation.actor_run_id();
    let journal_mutation_actor =
        resolved_edit_authorization.journal_mutation_actor_for(actor_run_id);
    let path_case = PathCase::read(worktree_context.common_git_directory())?;
    let scopes = normalized_claim_scopes(declared_scopes, &source, path_case);
    let claim_repository_facts =
        ClaimRepositoryFacts::read(&worktree_context, claim_run_validation)?;
    let phase_start_head = phase_start.protected_head(&claim_repository_facts.current_head);
    let berth_config = match BerthConfig::read(&worktree_context.configuration_lookup())? {
        Enrollment::Enrolled(berth_config) => berth_config,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let (target, trunk_at_claim) = repository_facts::selected_target_and_tip(
        &worktree_context,
        &claim_repository_facts.head_snapshot,
        target.as_deref().map_or(
            TargetSelectionRequest::ExplicitClaimFromBranch,
            TargetSelectionRequest::ExplicitClaimWithArgument,
        ),
        &berth_config,
    )?;
    let ledger = Ledger::open_from_discovered_worktree(&worktree_context)?;
    let reservation_id = ReservationId::new();
    let prepared_claim = PreparedClaim {
        reservation_id,
        scopes: scopes.clone(),
        source,
        purpose,
        trunk_at_claim,
        target,
        head_snapshot: claim_repository_facts.head_snapshot,
        phase_start_head,
        worktree_root: claim_repository_facts.worktree_root,
        worktree_administrative_locator: worktree_context.administrative_locator().clone(),
        coordination_identity_provenance: claim_run_validation.coordination_identity_provenance(),
    };
    let approver = berth_config.overlap_approver(worktree_context.repository_root());
    let answer_policy =
        ClaimAnswerPolicy::resolve(overlap_authorization, berth_config.default_answer);
    let answer_source = answer_policy.source();
    let target_view =
        TargetView::from_claim(&prepared_claim.target, &prepared_claim.trunk_at_claim);
    let repository_trunk = berth_config
        .repository_trunk()
        .map_err(ClaimError::InvalidRepositoryTrunk)?;
    let mut validation_context = ClaimValidationContext {
        repository: claim_repository_facts.repository,
        run_validation: claim_run_validation,
        worktree_context: worktree_context.clone(),
        recovery_command_line: recovery_command_line.clone(),
        worktree_id: journal_mutation_actor.worktree_id,
        path_case,
        answer_policy,
        maximum_reservations: berth_config.maximum_reservations,
        maximum_ordering_edges: berth_config.maximum_ordering_edges,
        repository_trunk,
    };
    let outcome = prepared_claim.transact_with_capacity_recovery(
        &ledger,
        journal_mutation_actor,
        &mut validation_context,
        &berth_config,
        capacity_recovery,
    )?;
    finish_new_claim(
        outcome,
        &worktree_context,
        journal_mutation_actor,
        approver,
        NewClaimPublication {
            reservation_id,
            target: target_view,
            scopes,
            answer_policy: answer_source,
        },
    )
}

/// Facts to publish after the acquisition transaction appends successfully.
struct NewClaimPublication {
    reservation_id: ReservationId,
    target:         TargetView,
    scopes:         ReservationScopeSet,
    answer_policy:  ClaimAnswerPolicy,
}

fn finish_new_claim(
    outcome: LedgerTransactionOutcome<ClaimRejection>,
    worktree_context: &WorktreeContext,
    requester: ResolvedJournalMutationActor,
    approver: Option<OverlapApprover>,
    publication: NewClaimPublication,
) -> Result<Enrollment<ClaimExecution>, ClaimError> {
    let execution = match outcome {
        LedgerTransactionOutcome::Rejected(rejection) => audited_claim_rejection_execution(
            rejection,
            approver,
            worktree_context,
            requester,
            &publication.scopes,
        )?,
        LedgerTransactionOutcome::Appended {
            event,
            session_mapping_publication,
        } => claim_execution_from_outcome(
            &event,
            session_mapping_publication,
            publication,
            worktree_context,
        ),
    };
    Ok(Enrollment::Enrolled(execution))
}

/// Convert a rejected claim and audit any overlap refusal after releasing its lock.
fn audited_claim_rejection_execution(
    rejection: ClaimRejection,
    approver: Option<OverlapApprover>,
    worktree_context: &WorktreeContext,
    requester: ResolvedJournalMutationActor,
    scopes: &ReservationScopeSet,
) -> Result<ClaimExecution, ClaimError> {
    let execution = claim_rejection_execution(rejection, approver)?;
    record_refusal(&execution, worktree_context, requester, scopes);
    Ok(execution)
}

/// Record only the overlap refusals the caller will receive, after releasing the claim lock.
fn record_refusal(
    execution: &ClaimExecution,
    worktree_context: &WorktreeContext,
    requester: ResolvedJournalMutationActor,
    scopes: &ReservationScopeSet,
) {
    let conflicts = match execution {
        ClaimExecution::Blocked { conflicts, .. } => conflicts,
        ClaimExecution::IncompleteAnswer { rejection, .. } => &rejection.conflicts,
        ClaimExecution::Selected { .. }
        | ClaimExecution::Claimed { .. }
        | ClaimExecution::Answered { .. }
        | ClaimExecution::AnsweredWithoutOverlap(_)
        | ClaimExecution::OrderingCycle(_)
        | ClaimExecution::ContradictsLiveOrdering(_)
        | ClaimExecution::NothingToReplace(_)
        | ClaimExecution::ReservationLimitReached(_)
        | ClaimExecution::OrderingEdgeLimitReached(_) => return,
    };
    refusal::record(
        worktree_context,
        requester,
        RefusedVerb::Claim,
        scopes,
        conflicts,
    );
}

fn normalized_claim_scopes(
    scopes: DeclaredReservationScopeSet,
    source: &ClaimSource,
    path_case: PathCase,
) -> ReservationScopeSet {
    match source {
        ClaimSource::FirstTouch | ClaimSource::Enrolled | ClaimSource::Cover { .. } => {
            scopes.into_exact_file_antichain(path_case)
        },
        ClaimSource::WorkPlan { .. } | ClaimSource::Explicit => {
            scopes.into_minimal_antichain(path_case)
        },
    }
}

/// Acquire, widen, or reuse the acting run's one first-touch reservation.
pub(crate) fn acquire_first_touch(
    request: FirstTouchClaimRequest,
    recovery_command_line: &RecoveryCommandLine,
    capacity_recovery: &mut ClaimCapacityRecovery,
) -> Result<Enrollment<FirstTouchClaimExecution>, ClaimError> {
    acquire_first_touch_with_reservation_selection(
        request,
        CheckReservationSelection::SessionMappingOrSingleActive,
        recovery_command_line,
        capacity_recovery,
    )
}

/// Acquire first-touch protection using the reservation selection parsed by `check`.
pub(in crate::verb) fn acquire_first_touch_for_check(
    request: FirstTouchClaimRequest,
    reservation_selection: CheckReservationSelection,
    recovery_command_line: &RecoveryCommandLine,
    capacity_recovery: &mut ClaimCapacityRecovery,
) -> Result<Enrollment<FirstTouchClaimExecution>, ClaimError> {
    acquire_first_touch_with_reservation_selection(
        request,
        reservation_selection,
        recovery_command_line,
        capacity_recovery,
    )
}

fn acquire_first_touch_with_reservation_selection(
    request: FirstTouchClaimRequest,
    reservation_selection: CheckReservationSelection,
    recovery_command_line: &RecoveryCommandLine,
    capacity_recovery: &mut ClaimCapacityRecovery,
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
    let edit_scopes = declared_scopes.clone().into_minimal_antichain(path_case);
    let scopes = declared_scopes.into_exact_file_antichain(path_case);
    let source = ClaimSource::FirstTouch;
    let repository_facts = ClaimRepositoryFacts::read(&worktree_context, run_validation)?;
    let phase_start_head = ProtectedPhaseStartHead::from(repository_facts.current_head.clone());
    let berth_config = match BerthConfig::read(&worktree_context.configuration_lookup())? {
        Enrollment::Enrolled(berth_config) => berth_config,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let (target, trunk_at_claim) = repository_facts::selected_target_and_tip(
        &worktree_context,
        &repository_facts.head_snapshot,
        TargetSelectionRequest::AutomaticAcquisition,
        &berth_config,
    )?;
    let ledger = Ledger::open_from_discovered_worktree(&worktree_context)?;
    let prepared_claim = PreparedClaim {
        reservation_id: ReservationId::new(),
        scopes,
        source,
        purpose: ReservationPurpose::NotProvidedByCaller,
        trunk_at_claim,
        target,
        head_snapshot: repository_facts.head_snapshot,
        phase_start_head,
        worktree_root: repository_facts.worktree_root,
        worktree_administrative_locator: worktree_context.administrative_locator().clone(),
        coordination_identity_provenance: run_validation.coordination_identity_provenance(),
    };
    let repository_trunk = berth_config
        .repository_trunk()
        .map_err(ClaimError::InvalidRepositoryTrunk)?;
    let mut validation_context = FirstTouchValidationContext {
        repository: repository_facts.repository,
        run_validation,
        worktree_context,
        recovery_command_line: recovery_command_line.clone(),
        coordination_run_id,
        worktree_id: journal_mutation_actor.worktree_id,
        path_case,
        edit_scopes,
        conflict_handling,
        reservation_selection,
        maximum_reservations: berth_config.maximum_reservations,
        repository_trunk,
    };
    let execution = transact_first_touch_with_capacity_recovery(
        &ledger,
        &prepared_claim,
        &mut validation_context,
        &berth_config,
        capacity_recovery,
    )?;
    // Only a refusal of the whole request stops the caller, so only it records the default.
    let (
        FirstTouchClaimExecution::Blocked { .. },
        FirstTouchConflictHandling::RefuseRequest,
        Some(default_answer),
    ) = (&execution, conflict_handling, berth_config.default_answer)
    else {
        return Ok(Enrollment::Enrolled(execution));
    };
    let answered = record_first_touch_default_answer(
        &ledger,
        default_answer,
        &prepared_claim.target.target,
        &validation_context,
    )?;
    let execution = transact_first_touch_with_capacity_recovery(
        &ledger,
        &prepared_claim,
        &mut validation_context,
        &berth_config,
        capacity_recovery,
    )?;
    Ok(Enrollment::Enrolled(
        execution.answered_by_default(answered),
    ))
}

/// Retry a full-ledger decision once after committing stale-scope maintenance.
fn transact_first_touch_with_capacity_recovery(
    ledger: &Ledger,
    prepared_claim: &PreparedClaim,
    validation_context: &mut FirstTouchValidationContext,
    config: &BerthConfig,
    capacity_recovery: &mut ClaimCapacityRecovery,
) -> Result<FirstTouchClaimExecution, ClaimError> {
    let execution = transact_first_touch(ledger, prepared_claim, validation_context)?;
    if matches!(
        execution,
        FirstTouchClaimExecution::ReservationLimitReached(_)
    ) {
        capacity_recovery.recover(&validation_context.repository, ledger, config)?;
        validation_context.repository = Arc::new(validation_context.repository.fresh_observation());
        transact_first_touch(ledger, prepared_claim, validation_context)
    } else {
        Ok(execution)
    }
}

/// Prepare a first-touch decision, validate its generation, and publish its committed result.
fn transact_first_touch(
    ledger: &Ledger,
    prepared_claim: &PreparedClaim,
    validation_context: &FirstTouchValidationContext,
) -> Result<FirstTouchClaimExecution, ClaimError> {
    let coordination_run_id = validation_context.coordination_run_id;
    let reservation_selection = validation_context.reservation_selection;
    let worktree_context = validation_context.worktree_context.clone();
    let selection = std::cell::RefCell::new(PreparedSessionMappingPublication::Unchanged);
    let execution = ledger.transact_with_committed_action_and_consume_locked_outcome(
        validation_context.worktree_id,
        coordination_run_id,
        |state| {
            let validation = validate_first_touch_transaction(
                &state,
                prepared_claim.clone(),
                validation_context.clone(),
            );
            *selection.borrow_mut() = match &validation {
                CommittedActionValidation::Reject(FirstTouchClaimRejection::AlreadyHeld {
                    acquisition,
                    session_mapping: AlreadyHeldSessionMapping::PublishSelectedReservation,
                }) => session::PreparedSessionMappingPublication::prepare_selection(
                    &worktree_context.ledger_directory(),
                    coordination_run_id,
                    acquisition.reservation_id,
                ),
                _ => PreparedSessionMappingPublication::Unchanged,
            };
            validation
        },
        Ok::<_, Infallible>,
        |outcome| {
            first_touch_execution_from_outcome(
                outcome,
                coordination_run_id,
                reservation_selection,
                &worktree_context,
                selection.replace(PreparedSessionMappingPublication::Unchanged),
            )
        },
    );
    match execution {
        Ok(execution) => execution,
        Err(LedgerCommittedActionError::Transaction(error)) => Err(ClaimError::Transaction(error)),
        Err(LedgerCommittedActionError::Action(error)) => match error {},
    }
}

/// Record the repository's default answer to every foreign holder of a refused first touch.
///
/// Conflicts are prepared before acquisition and checked against intervening events. A refusal
/// answers no holder; the first touch retried after it reports whatever still stops the edit.
fn record_first_touch_default_answer(
    ledger: &Ledger,
    default_answer: DefaultAnswer,
    target: &IntegrationTarget,
    validation_context: &FirstTouchValidationContext,
) -> Result<WireOrderedReservationIds, ClaimError> {
    let outcome = ledger.transact(
        validation_context.worktree_id,
        validation_context.coordination_run_id,
        |state| {
            let reservations = match locked_claim_reservations(
                &state,
                validation_context.run_validation,
                &validation_context.repository,
                validation_context.worktree_id,
                &validation_context.recovery_command_line,
                target,
                &validation_context.repository_trunk,
            ) {
                Ok(reservations) => reservations,
                Err(rejection) => return TransactionValidation::Reject(rejection),
            };
            let conflicts = reservations.conflicts_for_first_touch(
                &validation_context.edit_scopes,
                validation_context.coordination_run_id,
                validation_context.worktree_id,
                validation_context.path_case,
            );
            authorization::validate_answer(
                AnswerKind::Override,
                authorization::default_answer_request(default_answer, &conflicts),
                conflicts,
                &reservations,
                AnsweringWorktree {
                    coordination_run_id: validation_context.coordination_run_id,
                    worktree_id:         validation_context.worktree_id,
                },
                validation_context.edit_scopes.clone(),
            )
        },
    )?;
    Ok(match outcome {
        LedgerTransactionOutcome::Appended { event, .. } => match &event.operation {
            JournalOperation::Answer { authorizations, .. } => {
                authorization::answered_blockers(authorizations)
            },
            _ => WireOrderedReservationIds::sorted_and_deduplicated(Vec::new()),
        },
        LedgerTransactionOutcome::Rejected(_) => {
            WireOrderedReservationIds::sorted_and_deduplicated(Vec::new())
        },
    })
}

fn first_touch_execution_from_outcome(
    outcome: LedgerCommittedActionOutcome<
        FirstTouchClaimRejection,
        CommittedReservationAcquisition,
    >,
    coordination_run_id: CoordinationRunId,
    reservation_selection: CheckReservationSelection,
    worktree_context: &WorktreeContext,
    selection: PreparedSessionMappingPublication,
) -> Result<FirstTouchClaimExecution, ClaimError> {
    match outcome {
        LedgerCommittedActionOutcome::Appended {
            output,
            session_mapping_publication,
        }
        | LedgerCommittedActionOutcome::AlreadySatisfied {
            output,
            session_mapping_publication,
        } => Ok(first_touch_acquired_execution(
            output,
            coordination_run_id,
            publish_coordination_run_marker(worktree_context, coordination_run_id),
            reservation_selection.reported_session_mapping_publication(session_mapping_publication),
        )),
        LedgerCommittedActionOutcome::Rejected(FirstTouchClaimRejection::AlreadyHeld {
            acquisition: output,
            session_mapping,
        }) => {
            let publication = match session_mapping {
                AlreadyHeldSessionMapping::PreserveUnmappedSession => {
                    SessionIdentityMappingPublication::LeftUnmapped
                },
                AlreadyHeldSessionMapping::PublishSelectedReservation => {
                    selection.publish(&worktree_context.ledger_directory())
                },
            };
            let session_mapping_publication =
                reservation_selection.reported_session_mapping_publication(publication);
            Ok(first_touch_acquired_execution(
                *output,
                coordination_run_id,
                publish_coordination_run_marker(worktree_context, coordination_run_id),
                session_mapping_publication,
            ))
        },
        LedgerCommittedActionOutcome::Rejected(FirstTouchClaimRejection::Blocked {
            scopes,
            conflicts,
        }) => Ok(FirstTouchClaimExecution::Blocked { scopes, conflicts }),
        LedgerCommittedActionOutcome::Rejected(
            FirstTouchClaimRejection::IneligibleExplicitReservation(reservation_id),
        ) => Err(ClaimError::IneligibleExplicitReservation(reservation_id)),
        LedgerCommittedActionOutcome::Rejected(
            FirstTouchClaimRejection::AmbiguousActiveRunReservations { candidates },
        ) => Err(ClaimError::AmbiguousActiveRunReservations { candidates }),
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

impl FirstTouchClaimExecution {
    /// Report the holders the repository's default answer named on a first touch that protected
    /// every requested path after it.
    fn answered_by_default(self, answered: WireOrderedReservationIds) -> Self {
        match self {
            Self::Acquired {
                acquisition,
                scopes,
                conflicts: FirstTouchConflictOutcome::None,
            } if !answered.is_empty() => Self::Acquired {
                acquisition,
                scopes,
                conflicts: FirstTouchConflictOutcome::AnsweredByDefault { answered },
            },
            execution => execution,
        }
    }
}

impl CheckReservationSelection {
    fn reported_session_mapping_publication(
        self,
        publication: SessionIdentityMappingPublication,
    ) -> SessionIdentityMappingPublication {
        match self {
            Self::SessionMappingOrSingleActive | Self::SessionMappingOrNew => publication,
            Self::Explicit(_) => publication.for_explicit_reservation_selection(),
        }
    }
}

fn first_touch_acquired_execution(
    committed: CommittedReservationAcquisition,
    coordination_run_id: CoordinationRunId,
    marker_publication: CoordinationRunMarkerPublication,
    session_mapping_publication: SessionIdentityMappingPublication,
) -> FirstTouchClaimExecution {
    let (acquisition, scopes, conflicts) = first_touch_acquisition(
        committed,
        coordination_run_id,
        marker_publication,
        session_mapping_publication,
    );
    FirstTouchClaimExecution::Acquired {
        acquisition,
        scopes,
        conflicts,
    }
}

/// Build the wire acquisition once for locked and already-protected checks.
fn first_touch_acquisition(
    committed: CommittedReservationAcquisition,
    coordination_run_id: CoordinationRunId,
    marker_publication: CoordinationRunMarkerPublication,
    session_mapping_publication: SessionIdentityMappingPublication,
) -> (
    ReservationAcquisition,
    ReservationScopeSet,
    FirstTouchConflictOutcome,
) {
    let CommittedReservationAcquisition {
        kind,
        reservation_id,
        target,
        phase_start_head,
        scopes,
        conflicts,
        already_held,
    } = committed;
    (
        ReservationAcquisition {
            kind,
            reservation_id,
            target,
            coordination_run_id,
            phase_start_head,
            marker_publication,
            session_mapping_publication,
            already_held,
        },
        scopes,
        conflicts,
    )
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
    event: &JournalEvent,
    session_mapping_publication: SessionIdentityMappingPublication,
    publication: NewClaimPublication,
    worktree_context: &WorktreeContext,
) -> ClaimExecution {
    let NewClaimPublication {
        reservation_id,
        target,
        scopes,
        answer_policy,
    } = publication;
    // An answer acquired no reservation, so there is no run marker to publish for it.
    if let JournalOperation::Answer {
        reservation_id,
        scopes,
        authorizations,
    } = &event.operation
    {
        return ClaimExecution::Answered {
            reservation_id: *reservation_id,
            blockers: authorization::answered_blockers(authorizations),
            scopes: scopes.clone(),
            answer_policy,
        };
    }
    let coordination_run_id = event.actor.run;
    let marker_publication = publish_coordination_run_marker(worktree_context, coordination_run_id);
    ClaimExecution::Claimed {
        reservation_id,
        target,
        coordination_run_id,
        scopes,
        marker_publication,
        session_mapping_publication,
    }
}

fn claim_rejection_execution(
    rejection: ClaimRejection,
    approver: Option<OverlapApprover>,
) -> Result<ClaimExecution, ClaimError> {
    match rejection {
        ClaimRejection::IneligibleExplicitReservation(id) => {
            Err(ClaimError::IneligibleExplicitReservation(id))
        },
        ClaimRejection::Conflict(conflicts) => Ok(ClaimExecution::Blocked {
            conflicts,
            approver,
        }),
        ClaimRejection::IncompleteAnswer(rejection) => Ok(ClaimExecution::IncompleteAnswer {
            rejection,
            approver,
        }),
        ClaimRejection::AnsweredWithoutOverlap(blockers) => {
            Ok(ClaimExecution::AnsweredWithoutOverlap(blockers))
        },
        ClaimRejection::ContradictsLiveOrdering(contradicted) => {
            Ok(ClaimExecution::ContradictsLiveOrdering(contradicted))
        },
        ClaimRejection::OrderingCycle(cycle) => Ok(ClaimExecution::OrderingCycle(cycle)),
        ClaimRejection::NothingToReplace(blockers) => {
            Ok(ClaimExecution::NothingToReplace(blockers))
        },
        ClaimRejection::Replay(error) => Err(ClaimError::ReservationReplay(error)),
        ClaimRejection::EdgeReplay(error) => Err(ClaimError::EdgeReplay(error)),
        ClaimRejection::CoordinationIdentity(rejection) => {
            Err(ClaimError::CoordinationIdentity(rejection))
        },
        ClaimRejection::InvalidCanonicalWorktreeRoot => {
            Err(ClaimError::InvalidCanonicalWorktreeRoot)
        },
        ClaimRejection::ReservationLimitReached(maximum) => {
            Ok(ClaimExecution::ReservationLimitReached(maximum))
        },
        ClaimRejection::OrderingEdgeLimitReached(maximum) => {
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
        repository,
        run_validation,
        worktree_context: _,
        recovery_command_line,
        worktree_id,
        path_case,
        answer_policy,
        maximum_reservations,
        maximum_ordering_edges,
        repository_trunk,
    } = context;
    let reservations = match locked_claim_reservations(
        state,
        run_validation,
        &repository,
        worktree_id,
        &recovery_command_line,
        &prepared_claim.target.target,
        &repository_trunk,
    ) {
        Ok(reservations) => reservations,
        Err(rejection) => return TransactionValidation::Reject(rejection),
    };
    let conflicts =
        reservations.conflicts_for_claim(&prepared_claim.scopes, worktree_id, path_case);
    let answering_worktree = AnsweringWorktree {
        coordination_run_id: run_validation.actor_run_id(),
        worktree_id,
    };
    // An answer reserves nothing, so the reservation limit does not apply to it.
    let permissive_request = match answer_policy {
        ClaimAnswerPolicy::CallerRequest(OverlapAuthorizationRequest::Defer(request)) => {
            return authorization::validate_answer(
                AnswerKind::Defer,
                *request,
                conflicts,
                &reservations,
                answering_worktree,
                prepared_claim.scopes,
            );
        },
        ClaimAnswerPolicy::RepositoryDefault(default_answer) if !conflicts.is_empty() => {
            return authorization::validate_answer(
                AnswerKind::Override,
                authorization::default_answer_request(default_answer, &conflicts),
                conflicts,
                &reservations,
                answering_worktree,
                prepared_claim.scopes,
            );
        },
        ClaimAnswerPolicy::CallerRequest(OverlapAuthorizationRequest::Absent)
        | ClaimAnswerPolicy::RepositoryDefault(_) => None,
        ClaimAnswerPolicy::CallerRequest(OverlapAuthorizationRequest::Permissive(request)) => {
            Some(*request)
        },
    };
    if authorization::count_reaches_limit(reservations.nonterminal_count(), maximum_reservations) {
        return TransactionValidation::Reject(ClaimRejection::ReservationLimitReached(
            maximum_reservations,
        ));
    }
    let ordering_graph = match state.ordering_graph() {
        Ok(ordering_graph) => ordering_graph,
        Err(error) => {
            return TransactionValidation::Reject(ClaimRejection::EdgeReplay(error));
        },
    };
    match permissive_request {
        None if conflicts.is_empty() => TransactionValidation::Append(Box::new(
            prepared_claim.into_operation(ConflictAuthorization::NoConflict),
        )),
        None => TransactionValidation::Reject(ClaimRejection::Conflict(conflicts)),
        Some(request) => validate_authorization(
            request,
            conflicts,
            prepared_claim,
            &OrderingContext {
                ordering_graph,
                reservations: &reservations,
                answering_worktree,
                reservation_selection: ClaimReservationSelection::NewReservation,
            },
            maximum_ordering_edges,
        ),
    }
}

/// Read the prepared reservations, observe the acting head, and validate the acting run.
fn locked_claim_reservations(
    state: &ReplayedLedgerState<'_>,
    run_validation: ClaimRunValidation,
    repository: &RepositorySnapshot,
    worktree_id: WorktreeId,
    recovery_command_line: &RecoveryCommandLine,
    target: &IntegrationTarget,
    repository_trunk: &IntegrationTarget,
) -> Result<RetainedReservationSet, ClaimRejection> {
    let worktree_context = repository.context();
    let reservations = state
        .reservations()
        .map_err(ClaimRejection::Replay)?
        .clone();
    let acting_head_containment = ActingHeadContainment::observe(
        &reservations,
        repository,
        worktree_id,
        target,
        repository_trunk,
    );
    let reservations = reservations.with_acting_head_containment(acting_head_containment);
    run_validation
        .validate(
            &reservations,
            worktree_context,
            worktree_id,
            recovery_command_line,
        )
        .map_err(ClaimRejection::from)?;
    Ok(reservations)
}

fn validate_first_touch_transaction(
    state: &ReplayedLedgerState<'_>,
    prepared_claim: PreparedClaim,
    context: FirstTouchValidationContext,
) -> CommittedActionValidation<FirstTouchClaimRejection, CommittedReservationAcquisition> {
    let FirstTouchValidationContext {
        repository,
        run_validation,
        worktree_context,
        recovery_command_line,
        coordination_run_id,
        worktree_id,
        path_case,
        edit_scopes,
        conflict_handling,
        reservation_selection,
        maximum_reservations,
        repository_trunk,
    } = context;
    let reservations = match state.reservations() {
        Ok(reservations) => reservations.clone(),
        Err(error) => {
            return CommittedActionValidation::Reject(FirstTouchClaimRejection::Replay(error));
        },
    };
    let acting_head_containment = ActingHeadContainment::observe(
        &reservations,
        &repository,
        worktree_id,
        &prepared_claim.target.target,
        &repository_trunk,
    );
    let reservations = reservations.with_acting_head_containment(acting_head_containment);
    if let Err(rejection) = validate_first_touch_run(
        run_validation,
        &reservations,
        &worktree_context,
        worktree_id,
        &recovery_command_line,
    ) {
        return CommittedActionValidation::Reject(rejection);
    }
    let first_touch_session_mapping =
        session::resolve_first_touch_mapping(&worktree_context.ledger_directory());
    let requested_scopes = prepared_claim.scopes.clone();
    let conflicts = reservations.conflicts_for_first_touch(
        &edit_scopes,
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
    let reuse = select_first_touch_reservation_reuse(
        &reservations,
        &repository_trunk,
        requested_scopes,
        protected_scopes,
        conflict_outcome,
        FirstTouchReservationReuseContext {
            ledger_directory: &worktree_context.ledger_directory(),
            first_touch_session_reservation_mapping: first_touch_session_mapping,
            check_reservation_selection: reservation_selection,
            coordination_run_id,
            worktree_id,
            path_case,
        },
    );
    let (protected_scopes, conflict_outcome, already_held) = match reuse {
        FirstTouchReservationReuse::AppendRequired {
            scopes,
            conflicts,
            already_held,
        } => (scopes, conflicts, already_held),
        FirstTouchReservationReuse::Complete(validation) => {
            return validation;
        },
    };
    if authorization::count_reaches_limit(reservations.nonterminal_count(), maximum_reservations) {
        return CommittedActionValidation::Reject(
            FirstTouchClaimRejection::ReservationLimitReached(maximum_reservations),
        );
    }
    append_first_touch_claim(
        prepared_claim,
        protected_scopes,
        conflict_outcome,
        already_held,
    )
}

/// Construct the claim and its acquisition report from the locked coverage decision.
fn append_first_touch_claim(
    mut prepared_claim: PreparedClaim,
    protected_scopes: ReservationScopeSet,
    conflicts: FirstTouchConflictOutcome,
    already_held: Vec<SiblingHeldScopes>,
) -> CommittedActionValidation<FirstTouchClaimRejection, CommittedReservationAcquisition> {
    prepared_claim.scopes = protected_scopes.clone();
    let acquisition = CommittedReservationAcquisition {
        kind: ReservationAcquisitionKind::Appended,
        reservation_id: prepared_claim.reservation_id,
        target: Box::new(TargetView::from_claim(
            &prepared_claim.target,
            &prepared_claim.trunk_at_claim,
        )),
        phase_start_head: prepared_claim.phase_start_head.clone(),
        scopes: protected_scopes,
        conflicts,
        already_held,
    };
    CommittedActionValidation::Append {
        operation: Box::new(prepared_claim.into_operation(ConflictAuthorization::NoConflict)),
        action:    acquisition,
    }
}

fn select_first_touch_reservation_reuse(
    reservations: &RetainedReservationSet,
    repository_trunk: &IntegrationTarget,
    requested_scopes: ReservationScopeSet,
    protected_scopes: ReservationScopeSet,
    conflicts: FirstTouchConflictOutcome,
    context: FirstTouchReservationReuseContext<'_>,
) -> FirstTouchReservationReuse {
    let FirstTouchReservationReuseContext {
        ledger_directory,
        first_touch_session_reservation_mapping,
        check_reservation_selection,
        coordination_run_id,
        worktree_id,
        path_case,
    } = context;
    if let CheckReservationSelection::Explicit(reservation_id) = check_reservation_selection {
        return reuse_first_touch_reservation(
            reservations,
            repository_trunk,
            requested_scopes,
            protected_scopes,
            conflicts,
            FirstTouchReservationSelection::from_explicit_selection(
                reservations,
                reservation_id,
                coordination_run_id,
                worktree_id,
            ),
            path_case,
        );
    }
    match first_touch_session_reservation_mapping {
        FirstTouchSessionReservationMapping::Mapped(session_reservation_identity) => {
            reuse_first_touch_reservation(
                reservations,
                repository_trunk,
                requested_scopes,
                protected_scopes,
                conflicts,
                FirstTouchReservationSelection::from_locked_session_mapping(
                    reservations,
                    session_reservation_identity,
                    ledger_directory,
                    coordination_run_id,
                    worktree_id,
                ),
                path_case,
            )
        },
        FirstTouchSessionReservationMapping::AvailableSessionWithoutMapping
            if matches!(
                check_reservation_selection,
                CheckReservationSelection::SessionMappingOrNew
            ) =>
        {
            append_unheld_first_touch_scopes(
                reservations,
                repository_trunk,
                protected_scopes,
                conflicts,
                context,
            )
        },
        FirstTouchSessionReservationMapping::AvailableSessionWithoutMapping
        | FirstTouchSessionReservationMapping::HarnessSessionUnavailable => {
            reuse_first_touch_reservation(
                reservations,
                repository_trunk,
                requested_scopes,
                protected_scopes,
                conflicts,
                FirstTouchReservationSelection::from_unmapped_active_run_reservations(
                    reservations,
                    ledger_directory,
                    coordination_run_id,
                    worktree_id,
                ),
                path_case,
            )
        },
    }
}

enum FirstTouchReservationReuse {
    AppendRequired {
        scopes:       ReservationScopeSet,
        conflicts:    FirstTouchConflictOutcome,
        already_held: Vec<SiblingHeldScopes>,
    },
    Complete(CommittedActionValidation<FirstTouchClaimRejection, CommittedReservationAcquisition>),
}

enum FirstTouchProtectedScopeOwnership<'reservation> {
    AlreadyHeld(&'reservation Reservation),
    Residual(ReservationScopeSet),
}

/// Open a phase only for paths its active siblings do not already hold.
fn append_unheld_first_touch_scopes(
    reservations: &RetainedReservationSet,
    repository_trunk: &IntegrationTarget,
    protected_scopes: ReservationScopeSet,
    conflicts: FirstTouchConflictOutcome,
    context: FirstTouchReservationReuseContext<'_>,
) -> FirstTouchReservationReuse {
    let already_held = sibling_held_scopes(
        reservations,
        &protected_scopes,
        context.coordination_run_id,
        context.worktree_id,
        context.path_case,
    );
    // Anchor the shared partition rule on the first requested path's holder when all
    // paths are covered. No reservation is selected for the new phase itself yet.
    let first_holder = protected_scopes.as_slice().iter().find_map(|candidate| {
        reservations.iter().find(|sibling| {
            sibling.is_active_for_coordination_run_and_worktree(
                context.coordination_run_id,
                context.worktree_id,
            ) && sibling
                .scopes()
                .as_slice()
                .iter()
                .any(|held| held.contains(candidate, context.path_case))
        })
    });
    let scopes = match first_holder {
        Some(sibling) => match partition_first_touch_protected_scopes(
            reservations,
            &protected_scopes,
            sibling,
            context.path_case,
        ) {
            FirstTouchProtectedScopeOwnership::AlreadyHeld(holder) => {
                return already_held_first_touch(
                    holder,
                    repository_trunk,
                    protected_scopes,
                    conflicts,
                    AlreadyHeldSessionMapping::PreserveUnmappedSession,
                );
            },
            FirstTouchProtectedScopeOwnership::Residual(scopes) => scopes,
        },
        None => protected_scopes,
    };
    FirstTouchReservationReuse::AppendRequired {
        scopes,
        conflicts,
        already_held,
    }
}

/// Report the exact requested paths each active sibling already protects.
fn sibling_held_scopes(
    reservations: &RetainedReservationSet,
    protected_scopes: &ReservationScopeSet,
    coordination_run_id: CoordinationRunId,
    worktree_id: WorktreeId,
    path_case: PathCase,
) -> Vec<SiblingHeldScopes> {
    reservations
        .iter()
        .filter(|sibling| {
            sibling.is_active_for_coordination_run_and_worktree(coordination_run_id, worktree_id)
        })
        .filter_map(|sibling| {
            let held = protected_scopes
                .as_slice()
                .iter()
                .filter(|candidate| {
                    sibling
                        .scopes()
                        .as_slice()
                        .iter()
                        .any(|scope| scope.contains(candidate, path_case))
                })
                .cloned()
                .collect::<Vec<_>>();
            ReservationScopeSet::try_from(held)
                .ok()
                .map(|scopes| SiblingHeldScopes {
                    reservation_id: sibling.id(),
                    scopes,
                })
        })
        .collect()
}

fn reuse_first_touch_reservation(
    reservations: &RetainedReservationSet,
    repository_trunk: &IntegrationTarget,
    requested_scopes: ReservationScopeSet,
    protected_scopes: ReservationScopeSet,
    conflicts: FirstTouchConflictOutcome,
    first_touch_reservation_selection: FirstTouchReservationSelection<'_>,
    path_case: PathCase,
) -> FirstTouchReservationReuse {
    let reservation = match first_touch_reservation_selection {
        FirstTouchReservationSelection::ExplicitReservation(reservation)
        | FirstTouchReservationSelection::SessionMappedReservation(reservation)
        | FirstTouchReservationSelection::SingleActiveRunReservation(reservation) => reservation,
        FirstTouchReservationSelection::IneligibleExplicitReservation(reservation_id) => {
            return FirstTouchReservationReuse::Complete(CommittedActionValidation::Reject(
                FirstTouchClaimRejection::IneligibleExplicitReservation(reservation_id),
            ));
        },
        FirstTouchReservationSelection::NoActiveRunReservation => {
            return FirstTouchReservationReuse::AppendRequired {
                scopes: protected_scopes,
                conflicts,
                already_held: Vec::new(),
            };
        },
        FirstTouchReservationSelection::AmbiguousActiveRunReservations { candidates } => {
            return FirstTouchReservationReuse::Complete(CommittedActionValidation::Reject(
                FirstTouchClaimRejection::AmbiguousActiveRunReservations { candidates },
            ));
        },
    };
    let protected_scopes = match partition_first_touch_protected_scopes(
        reservations,
        &protected_scopes,
        reservation,
        path_case,
    ) {
        FirstTouchProtectedScopeOwnership::AlreadyHeld(reservation) => {
            return already_held_first_touch(
                reservation,
                repository_trunk,
                protected_scopes,
                conflicts,
                AlreadyHeldSessionMapping::PublishSelectedReservation,
            );
        },
        FirstTouchProtectedScopeOwnership::Residual(residual) => residual,
    };
    widen_first_touch_reservation(
        reservations,
        repository_trunk,
        requested_scopes,
        protected_scopes,
        conflicts,
        reservation,
        path_case,
    )
}

fn widen_first_touch_reservation(
    reservations: &RetainedReservationSet,
    repository_trunk: &IntegrationTarget,
    requested_scopes: ReservationScopeSet,
    protected_scopes: ReservationScopeSet,
    conflicts: FirstTouchConflictOutcome,
    reservation: &Reservation,
    path_case: PathCase,
) -> FirstTouchReservationReuse {
    let added = protected_scopes
        .as_slice()
        .iter()
        .filter(|candidate| {
            !reservation
                .declared_race_scopes()
                .as_slice()
                .iter()
                .any(|held| held.contains(candidate, path_case))
        })
        .cloned()
        .collect::<Vec<_>>();
    let Ok(added_scopes) = ReservationScopeAdditionSet::try_from(added) else {
        return already_held_first_touch(
            reservation,
            repository_trunk,
            protected_scopes,
            conflicts,
            AlreadyHeldSessionMapping::PublishSelectedReservation,
        );
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
            action:    CommittedReservationAcquisition {
                kind: ReservationAcquisitionKind::Widened,
                reservation_id: reservation.id(),
                target: Box::new(TargetView::from_recorded(
                    reservation.target(),
                    repository_trunk,
                    reservation.comparison_snapshot(),
                )),
                phase_start_head: reservation.phase_start_head().clone(),
                scopes: protected_scopes,
                conflicts,
                already_held: Vec::new(),
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

/// Split the protected scopes into those the acting run already declares and the rest.
///
/// Every active reservation of the selected reservation's run in its worktree counts, not
/// only the selected one. Drift already treats those siblings as one party, so an edit
/// inside any of their scopes is covered; checking the selected reservation alone widened
/// it onto a path a sibling declared, and two siblings then declared the same path.
fn partition_first_touch_protected_scopes<'reservation>(
    reservations: &RetainedReservationSet,
    protected_scopes: &ReservationScopeSet,
    reservation: &'reservation Reservation,
    path_case: PathCase,
) -> FirstTouchProtectedScopeOwnership<'reservation> {
    let (worktree_id, coordination_run_id) =
        (reservation.actor().worktree, reservation.actor().run);
    let residual = protected_scopes
        .as_slice()
        .iter()
        .filter(|candidate| {
            !reservations
                .iter()
                .filter(|sibling| {
                    sibling.is_active_for_coordination_run_and_worktree(
                        coordination_run_id,
                        worktree_id,
                    )
                })
                .any(|sibling| {
                    sibling
                        .scopes()
                        .as_slice()
                        .iter()
                        .any(|held| held.contains(candidate, path_case))
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    ReservationScopeSet::try_from(residual).map_or(
        FirstTouchProtectedScopeOwnership::AlreadyHeld(reservation),
        FirstTouchProtectedScopeOwnership::Residual,
    )
}

fn already_held_first_touch(
    reservation: &Reservation,
    repository_trunk: &IntegrationTarget,
    scopes: ReservationScopeSet,
    conflicts: FirstTouchConflictOutcome,
    session_mapping: AlreadyHeldSessionMapping,
) -> FirstTouchReservationReuse {
    FirstTouchReservationReuse::Complete(CommittedActionValidation::Reject(
        FirstTouchClaimRejection::AlreadyHeld {
            acquisition: Box::new(already_held_acquisition(
                reservation,
                repository_trunk,
                scopes,
                conflicts,
            )),
            session_mapping,
        },
    ))
}

/// Retain the selected reservation's identity and baseline for every already-held answer.
fn already_held_acquisition(
    reservation: &Reservation,
    repository_trunk: &IntegrationTarget,
    scopes: ReservationScopeSet,
    conflicts: FirstTouchConflictOutcome,
) -> CommittedReservationAcquisition {
    CommittedReservationAcquisition {
        kind: ReservationAcquisitionKind::AlreadyHeld,
        reservation_id: reservation.id(),
        target: Box::new(TargetView::from_recorded(
            reservation.target(),
            repository_trunk,
            reservation.comparison_snapshot(),
        )),
        phase_start_head: reservation.phase_start_head().clone(),
        scopes,
        conflicts,
        already_held: Vec::new(),
    }
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
    worktree_id: WorktreeId,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<(), FirstTouchClaimRejection> {
    match run_validation.validate(
        reservations,
        worktree_context,
        worktree_id,
        recovery_command_line,
    ) {
        Ok(()) => Ok(()),
        Err(ClaimRunValidationError::CoordinationIdentity(rejection)) => {
            Err(FirstTouchClaimRejection::CoordinationIdentity(rejection))
        },
        Err(ClaimRunValidationError::InvalidCanonicalWorktreeRoot) => {
            Err(FirstTouchClaimRejection::InvalidCanonicalWorktreeRoot)
        },
    }
}

/// Admit a complete claim declaration before converting it to the journal representation.
fn checked_claim_operation(
    state: &ReplayedLedgerState<'_>,
    operation: JournalOperation,
) -> Result<CycleFreeOrderingOperation, ClaimRejection> {
    CycleFreeOrderingOperation::check(state, operation).map_err(|error| match error {
        OrderingDeclarationError::Cycle(cycle) => ClaimRejection::OrderingCycle(cycle),
        OrderingDeclarationError::Replay(error) => ClaimRejection::EdgeReplay(error),
    })
}

/// Record one overlap answer when its named holders are exactly the conflicts, or say why not.
///
/// The answer records in the invocation that carries it. Naming the holders' reservation ids is
/// what keeps the decision from applying to changed facts: the conflicts are recomputed here,
/// under the ledger lock, and a holder the answer leaves unnamed, or a named reservation that no
/// longer conflicts, refuses the claim exactly as an unanswered claim is refused. `--before` and
/// `--after` repeat once per holder, mix in one claim, and record one ordering edge against each
/// holder in the direction its own flag chose, as `--defer` repeats in
/// [`authorization::validate_answer`]; `--override` names one holder, so set equality admits it
/// only while one holder conflicts.
///
/// An answer reaches here because the caller supplied `--before`, `--after`, or `--override`,
/// which the pre-edit refusal asks for by name. By the time the caller runs the answered claim,
/// `conflicts_for_claim` can return nothing at all: the holder may have released, or a refreshed
/// [`crate::reservation::MergeExtent`] may no longer cover the requested scopes.
/// `ClaimRejection::AnsweredWithoutOverlap` reports that as its own outcome, because handing the
/// empty conflict list to [`crate::output::OutputEnvelope::blocked_claim`] would derive
/// `blocked_by` from it and refuse an overlap it could not name.
///
/// A sequence answer that points opposite to a live edge between a named holder and one of the
/// claimant's own reservations is refused as `ClaimRejection::ContradictsLiveOrdering` unless it
/// carries `--replace`, which records those edges as retired; see
/// [`authorization::bind_claim_authorization`].
fn validate_authorization(
    request: PermissiveOverlapAuthorizationRequest,
    conflicts: Vec<ReservationConflict>,
    prepared_claim: PreparedClaim,
    ordering_context: &OrderingContext<'_>,
    maximum_ordering_edges: u32,
) -> TransactionValidation<ClaimRejection> {
    match authorization::bind_claim_authorization(
        request,
        conflicts,
        ordering_context,
        maximum_ordering_edges,
    ) {
        Ok(authorization) => {
            TransactionValidation::Append(Box::new(prepared_claim.into_operation(authorization)))
        },
        Err(rejection) => TransactionValidation::Reject(rejection),
    }
}

impl PreparedClaim {
    /// Retry a full ledger once after maintenance, validating capacity again under the lock.
    fn transact_with_capacity_recovery(
        self,
        ledger: &Ledger,
        requester: ResolvedJournalMutationActor,
        context: &mut ClaimValidationContext,
        config: &BerthConfig,
        capacity_recovery: &mut ClaimCapacityRecovery,
    ) -> Result<LedgerTransactionOutcome<ClaimRejection>, ClaimError> {
        let outcome = self.clone().transact(ledger, requester, context)?;
        if matches!(
            outcome,
            LedgerTransactionOutcome::Rejected(ClaimRejection::ReservationLimitReached(_))
        ) {
            capacity_recovery.recover(&context.repository, ledger, config)?;
            context.repository = Arc::new(context.repository.fresh_observation());
            self.transact(ledger, requester, context)
        } else {
            Ok(outcome)
        }
    }

    /// Validate the claim before acquisition, then publish it after checking its generation.
    fn transact(
        self,
        ledger: &Ledger,
        requester: ResolvedJournalMutationActor,
        context: &ClaimValidationContext,
    ) -> Result<LedgerTransactionOutcome<ClaimRejection>, ClaimError> {
        Ok(ledger.transact(
            requester.worktree_id,
            requester.coordination_run_id,
            |state| match validate_claim_transaction(&state, self.clone(), context.clone()) {
                TransactionValidation::Append(operation) => {
                    match checked_claim_operation(&state, *operation) {
                        Ok(checked) => TransactionValidation::AppendChecked(Box::new(checked)),
                        Err(rejection) => TransactionValidation::Reject(rejection),
                    }
                },
                TransactionValidation::AppendChecked(operation) => {
                    TransactionValidation::AppendChecked(operation)
                },
                TransactionValidation::Reject(rejection) => {
                    TransactionValidation::Reject(rejection)
                },
            },
        )?)
    }

    fn into_operation(self, authorization: ConflictAuthorization) -> JournalOperation {
        JournalOperation::Claim {
            reservation_id:                   self.reservation_id,
            scopes:                           self.scopes,
            source:                           self.source,
            purpose:                          self.purpose,
            trunk_at_claim:                   self.trunk_at_claim,
            target:                           Some(Box::new(self.target)),
            head_snapshot:                    self.head_snapshot,
            phase_start_head:                 self.phase_start_head,
            worktree_root:                    self.worktree_root,
            worktree_administrative_locator:  self.worktree_administrative_locator,
            authorization:                    Box::new(authorization),
            coordination_identity_provenance: self.coordination_identity_provenance,
        }
    }
}

/// Resolve identity once, then bind the answer and scope change under the ledger lock.
fn acquire_named(
    request: ClaimRequest,
    reservation_id: ReservationId,
    recovery: &RecoveryCommandLine,
) -> Result<Enrollment<ClaimExecution>, ClaimError> {
    let ClaimRequest {
        reservation_selection: _,
        target: _,
        declared_scopes,
        source,
        purpose: _,
        coordination_run_selection,
        phase_start: _,
        overlap_authorization,
        answer_recovery: _,
    } = request;
    let worktree = WorktreeContext::discover(&std::env::current_dir()?)?;
    let identity = ledger::resolve_identity(&worktree)?;
    let run_validation = coordination_run_selection.resolve(identity);
    let actor = identity.journal_mutation_actor_for(run_validation.actor_run_id());
    let path_case = PathCase::read(worktree.common_git_directory())?;
    let config = match BerthConfig::read(&worktree.configuration_lookup())? {
        Enrollment::Enrolled(config) => config,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let repository_trunk = config
        .repository_trunk()
        .map_err(ClaimError::InvalidRepositoryTrunk)?;
    let scopes = normalized_claim_scopes(declared_scopes, &source, path_case);
    let ledger = Ledger::open_from_discovered_worktree(&worktree)?;
    let context = ClaimValidationContext {
        repository: Arc::new(git::RepositorySnapshot::new(worktree.clone())),
        run_validation,
        worktree_context: worktree,
        recovery_command_line: recovery.clone(),
        worktree_id: actor.worktree_id,
        path_case,
        answer_policy: ClaimAnswerPolicy::resolve(overlap_authorization, config.default_answer),
        maximum_reservations: config.maximum_reservations,
        maximum_ordering_edges: config.maximum_ordering_edges,
        repository_trunk,
    };
    let selection = std::cell::RefCell::new(PreparedSessionMappingPublication::Unchanged);
    let result = ledger.transact_with_committed_action_and_consume_locked_outcome(
        actor.worktree_id,
        actor.coordination_run_id,
        |state| {
            *selection.borrow_mut() = session::PreparedSessionMappingPublication::prepare_selection(
                &context.worktree_context.ledger_directory(),
                context.run_validation.actor_run_id(),
                reservation_id,
            );
            match validate_named(&state, reservation_id, scopes.clone(), &context) {
                Ok(validation) => validation,
                Err(rejection) => CommittedActionValidation::Reject(rejection),
            }
        },
        Ok::<_, Infallible>,
        |mut outcome| {
            match &mut outcome {
                LedgerCommittedActionOutcome::Appended {
                    output,
                    session_mapping_publication,
                }
                | LedgerCommittedActionOutcome::AlreadySatisfied {
                    output,
                    session_mapping_publication,
                } if matches!(
                    output.kind,
                    ReservationAcquisitionKind::AlreadyHeld | ReservationAcquisitionKind::Answered
                ) =>
                {
                    *session_mapping_publication = selection
                        .replace(PreparedSessionMappingPublication::Unchanged)
                        .publish(&context.worktree_context.ledger_directory());
                },
                _ => {},
            }
            consume_named(outcome, &context, &config)
        },
    );
    match result {
        Ok(execution) => {
            let execution = execution?;
            record_refusal(&execution, &context.worktree_context, actor, &scopes);
            Ok(Enrollment::Enrolled(execution))
        },
        Err(LedgerCommittedActionError::Transaction(error)) => Err(ClaimError::Transaction(error)),
        Err(LedgerCommittedActionError::Action(error)) => match error {},
    }
}

/// Publish the explicit selection before releasing the mutation lock.
fn consume_named(
    outcome: LedgerCommittedActionOutcome<ClaimRejection, CommittedReservationAcquisition>,
    context: &ClaimValidationContext,
    config: &BerthConfig,
) -> Result<ClaimExecution, ClaimError> {
    match outcome {
        LedgerCommittedActionOutcome::Appended {
            output: committed,
            session_mapping_publication,
        }
        | LedgerCommittedActionOutcome::AlreadySatisfied {
            output: committed,
            session_mapping_publication,
        } => {
            let publication = session_mapping_publication.for_explicit_reservation_selection();
            let CommittedReservationAcquisition {
                kind,
                reservation_id,
                target,
                phase_start_head,
                scopes,
                already_held,
                ..
            } = committed;
            Ok(ClaimExecution::Selected {
                acquisition: ReservationAcquisition {
                    kind,
                    reservation_id,
                    coordination_run_id: context.run_validation.actor_run_id(),
                    target,
                    phase_start_head,
                    marker_publication: publish_coordination_run_marker(
                        &context.worktree_context,
                        context.run_validation.actor_run_id(),
                    ),
                    session_mapping_publication: publication,
                    already_held,
                },
                scopes,
            })
        },
        LedgerCommittedActionOutcome::Rejected(rejection) => claim_rejection_execution(
            rejection,
            config.overlap_approver(context.worktree_context.repository_root()),
        ),
    }
}

/// Select using check's eligibility rule before examining overlap answers.
fn validate_named(
    state: &ReplayedLedgerState<'_>,
    id: ReservationId,
    scopes: ReservationScopeSet,
    context: &ClaimValidationContext,
) -> Result<
    CommittedActionValidation<ClaimRejection, CommittedReservationAcquisition>,
    ClaimRejection,
> {
    let reservations = locked_reservations(state, id, context)?;
    let FirstTouchReservationSelection::ExplicitReservation(selected) =
        FirstTouchReservationSelection::from_explicit_selection(
            &reservations,
            id,
            context.run_validation.actor_run_id(),
            context.worktree_id,
        )
    else {
        return Err(ClaimRejection::IneligibleExplicitReservation(id));
    };
    let conflicts =
        reservations.conflicts_for_claim(&scopes, context.worktree_id, context.path_case);
    let authorization = authorization(state, &reservations, conflicts, context, id)?;
    let NamedClaimCoverage {
        additions,
        already_held,
    } = coverage(&reservations, selected, &scopes, context);
    let mutation = if additions.is_empty()
        && ordering_is_already_satisfied(&authorization, state, id, context.path_case)
    {
        NamedClaimMutation::AlreadySatisfied(scopes)
    } else {
        let (operation, kind, scopes) = mutation(selected, scopes, additions, authorization);
        NamedClaimMutation::Append {
            operation: Box::new(checked_claim_operation(state, operation)?),
            kind,
            scopes,
        }
    };
    let (kind, acquired_scopes) = match &mutation {
        NamedClaimMutation::Append { kind, scopes, .. } => (*kind, scopes.clone()),
        NamedClaimMutation::AlreadySatisfied(scopes) => {
            (ReservationAcquisitionKind::AlreadyHeld, scopes.clone())
        },
    };
    let acquisition = CommittedReservationAcquisition {
        kind,
        reservation_id: id,
        target: Box::new(TargetView::from_recorded(
            selected.target(),
            &context.repository_trunk,
            selected.comparison_snapshot(),
        )),
        phase_start_head: selected.phase_start_head().clone(),
        scopes: acquired_scopes,
        conflicts: FirstTouchConflictOutcome::None,
        already_held,
    };
    Ok(match mutation {
        NamedClaimMutation::Append { operation, .. } => CommittedActionValidation::AppendChecked {
            operation,
            action: acquisition,
        },
        NamedClaimMutation::AlreadySatisfied(_) => {
            CommittedActionValidation::AlreadySatisfied(acquisition)
        },
    })
}

/// A named claim either changes durable facts or selects work already authorized.
enum NamedClaimMutation {
    /// A new scope or overlap answer requires one journal record.
    Append {
        operation: Box<CycleFreeOrderingOperation>,
        kind:      ReservationAcquisitionKind,
        scopes:    ReservationScopeSet,
    },
    /// Existing scope declarations and edge-bound answers satisfy the request.
    AlreadySatisfied(ReservationScopeSet),
}

/// Match each scope against answers recorded for this exact reservation and edge.
fn ordering_is_already_satisfied(
    authorization: &NamedAuthorization,
    state: &ReplayedLedgerState<'_>,
    id: ReservationId,
    path_case: PathCase,
) -> bool {
    let NamedAuthorization::Acquire(ConflictAuthorization::SequenceWithExistingEdges {
        overlaps,
        holders,
        ..
    }) = authorization
    else {
        return false;
    };
    holders.iter().all(|binding| {
        let ClaimSequenceBinding::ExistingEdge { holder, .. } = binding else {
            return false;
        };
        overlaps
            .as_slice()
            .iter()
            .filter(|overlap| overlap.reservation_id == holder.blocker)
            .all(|overlap| {
                overlap.scopes.as_slice().iter().all(|scope| {
                    state
                        .coordination_events()
                        .iter()
                        .any(|event| match &event.operation {
                            JournalOperation::Claim {
                                reservation_id,
                                authorization,
                                ..
                            } if *reservation_id == id => authorization.covers_ordering_edge(
                                holder.edge_id,
                                overlap,
                                scope,
                                path_case,
                            ),
                            JournalOperation::Widen {
                                reservation_id,
                                authorization,
                                ..
                            } if *reservation_id == id => authorization.covers_ordering_edge(
                                holder.edge_id,
                                overlap,
                                scope,
                                path_case,
                            ),
                            JournalOperation::Answer {
                                reservation_id: Some(reservation_id),
                                authorizations,
                                ..
                            } if *reservation_id == id => {
                                authorizations.iter().any(|authorization| {
                                    authorization.covers_ordering_edge(
                                        holder.edge_id,
                                        overlap,
                                        scope,
                                        path_case,
                                    )
                                })
                            },
                            _ => false,
                        })
                })
            })
    })
}

struct NamedClaimCoverage {
    additions:    Vec<ReservationScope>,
    already_held: Vec<super::SiblingHeldScopes>,
}

/// Partition requested scopes using the same active-sibling rule as check.
fn coverage(
    reservations: &RetainedReservationSet,
    selected: &Reservation,
    scopes: &ReservationScopeSet,
    context: &ClaimValidationContext,
) -> NamedClaimCoverage {
    let id = selected.id();
    let already_held = sibling_held_scopes(
        reservations,
        scopes,
        context.run_validation.actor_run_id(),
        context.worktree_id,
        context.path_case,
    )
    .into_iter()
    .filter(|held| held.reservation_id != id)
    .collect::<Vec<_>>();
    let additions = scopes
        .as_slice()
        .iter()
        .filter(|candidate| {
            !selected
                .declared_race_scopes()
                .as_slice()
                .iter()
                .any(|held| held.contains(candidate, context.path_case))
                && !already_held
                    .iter()
                    .any(|held| held.scopes.as_slice().contains(candidate))
        })
        .cloned()
        .collect::<Vec<_>>();
    NamedClaimCoverage {
        additions,
        already_held,
    }
}

/// Validate the selected reservation and the invocation's run against locked evidence.
fn locked_reservations(
    state: &ReplayedLedgerState<'_>,
    id: ReservationId,
    context: &ClaimValidationContext,
) -> Result<RetainedReservationSet, ClaimRejection> {
    let recorded = state.reservations().map_err(ClaimRejection::Replay)?;
    let FirstTouchReservationSelection::ExplicitReservation(selected) =
        FirstTouchReservationSelection::from_explicit_selection(
            recorded,
            id,
            context.run_validation.actor_run_id(),
            context.worktree_id,
        )
    else {
        return Err(ClaimRejection::IneligibleExplicitReservation(id));
    };
    let target = match selected.target() {
        RecordedTarget::Recorded { target, .. } => target,
        RecordedTarget::Unrecorded => &context.repository_trunk,
    };
    locked_claim_reservations(
        state,
        context.run_validation,
        &context.repository,
        context.worktree_id,
        &context.recovery_command_line,
        target,
        &context.repository_trunk,
    )
}

/// Choose one nonempty widening or an answer-only event without altering creation metadata.
fn mutation(
    selected: &Reservation,
    scopes: ReservationScopeSet,
    additions: Vec<ReservationScope>,
    authorization: NamedAuthorization,
) -> (
    JournalOperation,
    ReservationAcquisitionKind,
    ReservationScopeSet,
) {
    match (
        authorization,
        ReservationScopeAdditionSet::try_from(additions),
    ) {
        (NamedAuthorization::Acquire(authorization), Ok(added_scopes)) => {
            let acquired = ReservationScopeSet::from(&added_scopes);
            let reason = crate::ledger::ExplicitWidenReason::named_claim();
            (
                JournalOperation::Widen {
                    reservation_id: selected.id(),
                    added_scopes,
                    cause: WidenCause::Explicit { reason },
                    authorization,
                    edit_blocking_status: selected.edit_blocking_status(),
                },
                ReservationAcquisitionKind::Widened,
                acquired,
            )
        },
        (authorization, _) => {
            let (authorizations, kind) = match authorization {
                NamedAuthorization::Acquire(authorization) => {
                    (vec![authorization], ReservationAcquisitionKind::AlreadyHeld)
                },
                NamedAuthorization::Answer(authorizations) => {
                    (authorizations, ReservationAcquisitionKind::Answered)
                },
            };
            (
                JournalOperation::Answer {
                    reservation_id: Some(selected.id()),
                    scopes: scopes.clone(),
                    authorizations,
                },
                kind,
                scopes,
            )
        },
    }
}

enum NamedAuthorization {
    Acquire(ConflictAuthorization),
    Answer(Vec<ConflictAuthorization>),
}

/// Bind explicit answers using the same holder and ordering checks as ordinary claims.
fn authorization(
    state: &ReplayedLedgerState<'_>,
    reservations: &RetainedReservationSet,
    conflicts: Vec<ReservationConflict>,
    context: &ClaimValidationContext,
    id: ReservationId,
) -> Result<NamedAuthorization, ClaimRejection> {
    let answering_worktree = AnsweringWorktree {
        coordination_run_id: context.run_validation.actor_run_id(),
        worktree_id:         context.worktree_id,
    };
    match &context.answer_policy {
        ClaimAnswerPolicy::CallerRequest(OverlapAuthorizationRequest::Permissive(request)) => {
            let graph = state.ordering_graph().map_err(ClaimRejection::EdgeReplay)?;
            authorization::bind_claim_authorization(
                (**request).clone(),
                conflicts,
                &OrderingContext {
                    ordering_graph: graph,
                    reservations,
                    answering_worktree,
                    reservation_selection: ClaimReservationSelection::Existing(id),
                },
                context.maximum_ordering_edges,
            )
            .map(NamedAuthorization::Acquire)
        },
        ClaimAnswerPolicy::CallerRequest(OverlapAuthorizationRequest::Defer(request)) => {
            authorization::bind_answer_authorizations(
                AnswerKind::Defer,
                (**request).clone(),
                conflicts,
            )
            .map(NamedAuthorization::Answer)
        },
        ClaimAnswerPolicy::CallerRequest(OverlapAuthorizationRequest::Absent)
        | ClaimAnswerPolicy::RepositoryDefault(_)
            if conflicts.is_empty() =>
        {
            Ok(NamedAuthorization::Acquire(
                ConflictAuthorization::NoConflict,
            ))
        },
        ClaimAnswerPolicy::RepositoryDefault(default_answer) => {
            authorization::bind_answer_authorizations(
                AnswerKind::Override,
                authorization::default_answer_request(*default_answer, &conflicts),
                conflicts,
            )
            .map(NamedAuthorization::Answer)
        },
        ClaimAnswerPolicy::CallerRequest(OverlapAuthorizationRequest::Absent) => {
            Err(ClaimRejection::Conflict(conflicts))
        },
    }
}
