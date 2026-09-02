//! The locked gate decision: who is entering, what holds them, and what that permits.

use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::error::GateError;
use super::error::GateTransactionRejection;
use super::permit;
use super::reference_transaction::PreviousMain;
use super::reference_transaction::ProposedMainMove;
use super::reference_transaction::ReferenceTransactionIssuingDirectory;
use super::reference_transaction::ReferenceTransactionPhase;
use crate::alert::Alert;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::config::GateMode;
use crate::coordination_identity;
use crate::coordination_identity::CoordinationIdentityValidationContext;
use crate::coordination_identity::CoordinationIdentityValidationError;
use crate::coordination_identity::RecoveryCommandLine;
use crate::coordination_identity::RunnableRecoveryCommandLine;
use crate::edge::IntegrationConstraintProjection;
use crate::edge::IntegrationHold;
use crate::edge::IntegrationReservationFacts;
use crate::edge::IntegrationSubject;
use crate::git;
use crate::git::GitError;
use crate::ids::CoordinationRunId;
use crate::ids::ForcedIntegrationPermitId;
use crate::ids::GitObjectId;
use crate::ids::ProjectionGeneration;
use crate::ids::ReservationId;
use crate::ids::WireOrderedReservationIds;
use crate::ledger;
use crate::ledger::BypassCause;
use crate::ledger::BypassOccurrenceTime;
use crate::ledger::BypassRecording;
use crate::ledger::BypassedAction;
use crate::ledger::ForcedIntegrationReason;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::ReconciliationValidation;
use crate::ledger::SkippedDeferral;
use crate::ledger::SkippedIntegrationHoldSet;
use crate::ledger::SkippedOrderingEdge;
use crate::ledger::WorktreeContext;
use crate::reconcile;
use crate::reservation::ReservationLifecycle;

/// The typed integration request produced from clap's force and reason primitives.
#[derive(Clone)]
pub(crate) enum IntegrationRequest {
    /// Apply normal gate policy.
    EnforceOrdering,
    /// Issue a one-use permit carrying this inseparable non-empty reason.
    ForceOnce(ForcedIntegrationReason),
}

/// A violation identifies the entering reservation and every hold that blocks it.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct IntegrationViolation {
    /// Facts needed to identify the reservation, plan, phase, and footprint.
    pub(crate) reservation:           IntegrationReservationFacts,
    /// The exact counterpart facts needed to identify every named blocker.
    pub(crate) blocking_reservations: Vec<IntegrationReservationFacts>,
    /// Every edge and unresolved deferral currently holding this reservation.
    pub(crate) holds:                 Vec<IntegrationHold>,
}

/// A decision made against one proposed main update at one journal generation.
pub(crate) enum GateDecision {
    /// No newly entering reservation is held.
    Clear {
        /// The exact replay generation validated under the mutation lock.
        generation: ProjectionGeneration,
    },
    /// Observe-only policy found violations but permits the update.
    Observed {
        /// The exact replay generation validated under the mutation lock.
        generation: ProjectionGeneration,
        /// Every violation that enforcing mode would reject.
        violations: Vec<IntegrationViolation>,
    },
    /// Enforcing policy rejects these violations.
    Blocked {
        /// The exact replay generation validated under the mutation lock.
        generation: ProjectionGeneration,
        /// Every violation preventing this update.
        violations: Vec<IntegrationViolation>,
    },
    /// A forced integration journalled a permit for the next matching main update.
    PermitIssued {
        /// The exact replay generation validated under the mutation lock.
        generation:          ProjectionGeneration,
        /// The one-use semantic permit identity.
        permit_id:           ForcedIntegrationPermitId,
        /// The reservation the permit authorizes.
        reservation_id:      ReservationId,
        /// Every hold deliberately skipped.
        skipped_holds:       SkippedIntegrationHoldSet,
        /// Holds on other entering reservations reported under observe-only policy.
        observed_violations: Vec<IntegrationViolation>,
    },
    /// Matching one-use permits were consumed and their skipped holds stayed intact.
    Forced {
        /// The exact replay generation validated under the mutation lock.
        generation: ProjectionGeneration,
    },
}

/// A complete gate result with reconciliation alerts from the same lock hold.
pub(crate) struct GateResult {
    /// The integration decision.
    pub(crate) decision: GateDecision,
    /// Durable alerts produced by the preceding actual-trunk reconciliation.
    pub(crate) alerts:   Vec<Alert>,
}

#[derive(Clone)]
pub(super) enum GatePurpose {
    Hook {
        phase:             ReferenceTransactionPhase,
        issuing_directory: ReferenceTransactionIssuingDirectory,
    },
    Integrate {
        reservation_id:      ReservationId,
        request:             IntegrationRequest,
        identity_validation: Box<CoordinationIdentityValidationContext>,
    },
}

/// Evaluate one explicit integration request through the same locked decision path as the hook.
pub(crate) fn evaluate_integration(
    invocation_directory: &Path,
    reservation_id: ReservationId,
    request: IntegrationRequest,
    previous: GitObjectId,
    proposed: GitObjectId,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<Enrollment<GateResult>, GateError> {
    let worktree_context = WorktreeContext::discover(invocation_directory)?;
    let resolved_edit_authorization = ledger::resolve_identity(&worktree_context)?;
    let identity_validation = CoordinationIdentityValidationContext::for_user_command(
        resolved_edit_authorization,
        &worktree_context,
        recovery_command_line,
    );
    let update = ProposedMainMove {
        previous: PreviousMain::Existing(previous),
        proposed,
    };
    let purpose = GatePurpose::Integrate {
        reservation_id,
        request,
        identity_validation: Box::new(identity_validation),
    };
    evaluate_locked(invocation_directory, &update, &purpose)
}

pub(super) fn evaluate_locked(
    invocation_directory: &Path,
    update: &ProposedMainMove,
    purpose: &GatePurpose,
) -> Result<Enrollment<GateResult>, GateError> {
    let worktree_context = WorktreeContext::discover(invocation_directory)?;
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
    let identity_validation = purpose.identity_validation()?;
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let ledger_repository = ledger.repository_identity()?;
    let resolved_edit_authorization = identity_validation.resolved_edit_authorization();
    let journal_mutation_actor = resolved_edit_authorization
        .journal_mutation_actor_for(purpose.coordination_run_id(&identity_validation));
    let outcome = ledger
        .transact_reconciliation(
            journal_mutation_actor.worktree_id,
            journal_mutation_actor.coordination_run_id,
            |state| {
                let prepared = match reconcile::prepare_gate_reconciliation(
                    state.events(),
                    state.generation(),
                    &worktree_context,
                    ledger_repository,
                    &berth_config,
                    update.proposed.clone(),
                ) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        return ReconciliationValidation::Reject(
                            GateTransactionRejection::Reconciliation(error),
                        );
                    },
                };
                if let Err(error) = coordination_identity::validate_coordination_identity(
                    prepared.reservations(),
                    &identity_validation,
                ) {
                    let rejection = match error {
                        CoordinationIdentityValidationError::Rejected(rejection) => {
                            GateTransactionRejection::CoordinationIdentity(rejection)
                        },
                        CoordinationIdentityValidationError::InvalidCanonicalWorktreeRoot => {
                            GateTransactionRejection::InvalidCanonicalWorktreeRoot
                        },
                    };
                    return ReconciliationValidation::Reject(rejection);
                }
                let newly_reachable =
                    match newly_reachable_commits(worktree_context.repository_root(), update) {
                        Ok(newly_reachable) => newly_reachable,
                        Err(error) => {
                            return ReconciliationValidation::Reject(
                                GateTransactionRejection::Git(error),
                            );
                        },
                    };
                let entering = entering_reservations(prepared.constraints(), &newly_reachable);
                let (decision, operations) = match decide(
                    state.events(),
                    prepared.constraints(),
                    &entering,
                    purpose,
                    berth_config.gate_mode,
                ) {
                    Ok(decision) => decision,
                    Err(error) => return ReconciliationValidation::Reject(error),
                };
                let (operations, action) = prepared.into_action(operations, decision);
                ReconciliationValidation::Apply {
                    operations,
                    recoverable_operations: Vec::new(),
                    action,
                }
            },
            reconcile::GateReconciliationAction::commit,
        )
        .map_err(|error| match error {
            LedgerCommittedActionError::Transaction(error) => GateError::Transaction(error),
            LedgerCommittedActionError::Action(error) => GateError::Reconciliation(error),
        })?;
    match outcome {
        LedgerCommittedActionOutcome::Appended {
            output: (report, decision),
            ..
        } => Ok(Enrollment::Enrolled(GateResult {
            decision,
            alerts: report.alerts,
        })),
        LedgerCommittedActionOutcome::Rejected(rejection) => Err(rejection.into()),
    }
}

pub(super) fn entering_reservations(
    constraints: &IntegrationConstraintProjection,
    newly_reachable: &[GitObjectId],
) -> Vec<ReservationId> {
    constraints
        .reservations
        .iter()
        .filter(|reservation| {
            !matches!(reservation.lifecycle, ReservationLifecycle::Released { .. })
        })
        .filter_map(|reservation| match &reservation.subject {
            IntegrationSubject::Commit { object_id } if newly_reachable.contains(object_id) => {
                Some(reservation.reservation_id)
            },
            IntegrationSubject::WorktreeHeadUnavailable => Some(reservation.reservation_id),
            IntegrationSubject::Commit { .. } | IntegrationSubject::NotApplicable => None,
        })
        .collect()
}

pub(super) fn decide(
    events: &[JournalEvent],
    constraints: &IntegrationConstraintProjection,
    entering: &[ReservationId],
    purpose: &GatePurpose,
    gate_mode: GateMode,
) -> Result<(GateDecision, Vec<JournalOperation>), GateTransactionRejection> {
    let mut violations = Vec::new();
    for reservation_id in entering {
        let holds = constraints
            .holds_for(*reservation_id)
            .cloned()
            .collect::<Vec<_>>();
        if holds.is_empty() {
            continue;
        }
        let reservation = constraints
            .reservation(*reservation_id)
            .map_err(GateTransactionRejection::MissingConstraintFact)?
            .clone();
        let blocking_reservations = blocking_reservations(constraints, *reservation_id, &holds)?;
        violations.push(IntegrationViolation {
            reservation,
            blocking_reservations,
            holds,
        });
    }
    match purpose {
        GatePurpose::Hook { phase, .. } => decide_hook(
            events,
            constraints.generation,
            &violations,
            gate_mode,
            *phase,
        ),
        GatePurpose::Integrate {
            reservation_id,
            request,
            ..
        } => decide_integration(
            constraints.generation,
            *reservation_id,
            request,
            entering,
            violations,
            gate_mode,
        ),
    }
}

pub(super) fn newly_reachable_commits(
    repository_root: &Path,
    update: &ProposedMainMove,
) -> Result<Vec<GitObjectId>, GitError> {
    match &update.previous {
        PreviousMain::Existing(previous) => {
            git::newly_reachable_commits(repository_root, previous, &update.proposed)
        },
        PreviousMain::Absent => git::reachable_commits(repository_root, &update.proposed),
    }
}

fn blocking_reservations(
    constraints: &IntegrationConstraintProjection,
    subject: ReservationId,
    holds: &[IntegrationHold],
) -> Result<Vec<IntegrationReservationFacts>, GateTransactionRejection> {
    let blocker_ids = WireOrderedReservationIds::sorted_and_deduplicated(
        holds
            .iter()
            .map(|hold| match hold {
                IntegrationHold::OrderingEdge { predecessor, .. } => *predecessor,
                IntegrationHold::DeferredOverlap {
                    deferred, blocker, ..
                } if *deferred == subject => *blocker,
                IntegrationHold::DeferredOverlap { deferred, .. } => *deferred,
            })
            .collect(),
    );
    blocker_ids
        .into_vec()
        .into_iter()
        .map(|blocker_id| {
            constraints
                .reservation(blocker_id)
                .cloned()
                .map_err(GateTransactionRejection::MissingConstraintFact)
        })
        .collect()
}

fn decide_hook(
    events: &[JournalEvent],
    generation: ProjectionGeneration,
    violations: &[IntegrationViolation],
    gate_mode: GateMode,
    phase: ReferenceTransactionPhase,
) -> Result<(GateDecision, Vec<JournalOperation>), GateTransactionRejection> {
    if violations.is_empty() {
        return Ok((GateDecision::Clear { generation }, Vec::new()));
    }
    let permits = permit::available_forced_integration_permits(events)
        .map_err(GateTransactionRejection::PermitReplay)?;
    let mut covered = Vec::new();
    let mut uncovered = Vec::new();
    for violation in violations {
        let covering_permit = permits.iter().find(|permit| {
            permit.reservation_id == violation.reservation.reservation_id
                && skipped_set_covers(&permit.skipped_holds, &violation.holds)
        });
        if let Some(permit) = covering_permit {
            covered.push(permit);
        } else {
            uncovered.push(violation.clone());
        }
    }
    if gate_mode.enforces() && !uncovered.is_empty() {
        return Ok((
            GateDecision::Blocked {
                generation,
                violations: uncovered,
            },
            Vec::new(),
        ));
    }
    let operations = match phase {
        ReferenceTransactionPhase::Committed => covered
            .into_iter()
            .flat_map(|permit| {
                [
                    JournalOperation::ConsumeForcedIntegrationPermit {
                        permit_id:      permit.permit_id,
                        reservation_id: permit.reservation_id,
                    },
                    JournalOperation::Bypass {
                        action:          BypassedAction::Integration,
                        cause:           BypassCause::ForcedIntegration {
                            permit_id: permit.permit_id,
                            reason:    permit.reason.clone(),
                        },
                        occurrence_time: BypassOccurrenceTime::EventRecordedAt,
                        recording:       BypassRecording::Direct,
                    },
                ]
            })
            .collect(),
        ReferenceTransactionPhase::Preparing
        | ReferenceTransactionPhase::Prepared
        | ReferenceTransactionPhase::Aborted
        | ReferenceTransactionPhase::Unrecognized => Vec::new(),
    };
    if uncovered.is_empty() {
        Ok((GateDecision::Forced { generation }, operations))
    } else {
        Ok((
            GateDecision::Observed {
                generation,
                violations: uncovered,
            },
            operations,
        ))
    }
}

fn decide_integration(
    generation: ProjectionGeneration,
    reservation_id: ReservationId,
    request: &IntegrationRequest,
    entering: &[ReservationId],
    violations: Vec<IntegrationViolation>,
    gate_mode: GateMode,
) -> Result<(GateDecision, Vec<JournalOperation>), GateTransactionRejection> {
    if !entering.contains(&reservation_id) {
        return Err(GateTransactionRejection::ReservationNotEntering(
            reservation_id,
        ));
    }
    match request {
        IntegrationRequest::ForceOnce(reason) => {
            let Some(violation) = violations
                .iter()
                .find(|violation| violation.reservation.reservation_id == reservation_id)
            else {
                return Err(GateTransactionRejection::NoHoldToForce(reservation_id));
            };
            let observed_violations = violations
                .iter()
                .filter(|violation| violation.reservation.reservation_id != reservation_id)
                .cloned()
                .collect::<Vec<_>>();
            if gate_mode.enforces() && !observed_violations.is_empty() {
                return Ok((
                    GateDecision::Blocked {
                        generation,
                        violations,
                    },
                    Vec::new(),
                ));
            }
            let skipped_holds = skipped_holds(&violation.holds)?;
            let permit_id = ForcedIntegrationPermitId::new();
            Ok((
                GateDecision::PermitIssued {
                    generation,
                    permit_id,
                    reservation_id,
                    skipped_holds: skipped_holds.clone(),
                    observed_violations,
                },
                vec![JournalOperation::ForcedIntegrationPermit {
                    permit_id,
                    reservation_id,
                    reason: reason.clone(),
                    skipped_holds,
                }],
            ))
        },
        IntegrationRequest::EnforceOrdering if !violations.is_empty() && gate_mode.enforces() => {
            Ok((
                GateDecision::Blocked {
                    generation,
                    violations,
                },
                Vec::new(),
            ))
        },
        IntegrationRequest::EnforceOrdering if !violations.is_empty() => Ok((
            GateDecision::Observed {
                generation,
                violations,
            },
            Vec::new(),
        )),
        IntegrationRequest::EnforceOrdering => Ok((GateDecision::Clear { generation }, Vec::new())),
    }
}

fn skipped_holds(
    holds: &[IntegrationHold],
) -> Result<SkippedIntegrationHoldSet, GateTransactionRejection> {
    let mut edges = Vec::new();
    let mut deferrals = Vec::new();
    for hold in holds {
        match hold {
            IntegrationHold::OrderingEdge {
                edge_id,
                predecessor,
                ..
            } => edges.push(SkippedOrderingEdge {
                edge_id:     *edge_id,
                predecessor: *predecessor,
            }),
            IntegrationHold::DeferredOverlap {
                declaration_event_id,
                deferred,
                blocker,
                ..
            } => deferrals.push(SkippedDeferral {
                declaration_event_id: *declaration_event_id,
                deferred:             *deferred,
                blocker:              *blocker,
            }),
        }
    }
    SkippedIntegrationHoldSet::new(edges, deferrals)
        .map_err(|_| GateTransactionRejection::MissingSkippedHold)
}

fn skipped_set_covers(
    skipped_holds: &SkippedIntegrationHoldSet,
    holds: &[IntegrationHold],
) -> bool {
    let edge_ids = holds
        .iter()
        .filter_map(|hold| match hold {
            IntegrationHold::OrderingEdge { edge_id, .. } => Some(*edge_id),
            IntegrationHold::DeferredOverlap { .. } => None,
        })
        .collect::<Vec<_>>();
    let deferral_event_ids = holds
        .iter()
        .filter_map(|hold| match hold {
            IntegrationHold::DeferredOverlap {
                declaration_event_id,
                ..
            } => Some(*declaration_event_id),
            IntegrationHold::OrderingEdge { .. } => None,
        })
        .collect::<Vec<_>>();
    skipped_holds.covers(&edge_ids, &deferral_event_ids)
}

impl GatePurpose {
    pub(super) fn identity_validation(
        &self,
    ) -> Result<CoordinationIdentityValidationContext, GateError> {
        match self {
            Self::Hook {
                issuing_directory, ..
            } => {
                let issuing_directory = match issuing_directory {
                    ReferenceTransactionIssuingDirectory::CapturedByManagedHook(
                        issuing_directory,
                    ) => issuing_directory,
                    ReferenceTransactionIssuingDirectory::MissingFromLegacyHook => {
                        return Err(GateError::LegacyReferenceTransactionHook);
                    },
                };
                let worktree_context = WorktreeContext::discover(issuing_directory)?;
                let resolved_edit_authorization = ledger::resolve_identity(&worktree_context)?;
                Ok(CoordinationIdentityValidationContext::for_git_gate(
                    resolved_edit_authorization,
                    &worktree_context,
                    RunnableRecoveryCommandLine::clear_session_mapping(),
                    RunnableRecoveryCommandLine::board(),
                ))
            },
            Self::Integrate {
                identity_validation,
                ..
            } => Ok(identity_validation.as_ref().clone()),
        }
    }

    fn coordination_run_id(
        &self,
        identity_validation: &CoordinationIdentityValidationContext,
    ) -> CoordinationRunId {
        match self {
            Self::Hook { .. } => CoordinationRunId::new(),
            Self::Integrate { .. } => {
                identity_validation
                    .resolved_edit_authorization()
                    .coordination_run_id
            },
        }
    }
}
