//! User-confirmed orphan recovery, rewritten integration, retirement, and renewal.

use std::convert::Infallible;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io::Error;
use std::path::Path;

use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::config::Enrollment;
use crate::edge::EdgeReplayError;
use crate::edge::OrderingGraph;
use crate::git;
use crate::git::GitError;
use crate::git::Reachability;
use crate::ids::CoordinationRunId;
use crate::ids::EventId;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::CommittedActionValidation;
use crate::ledger::IncursionIncidentId;
use crate::ledger::JournalActor;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::ReconciliationValidation;
use crate::ledger::TransactionValidation;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::output::ResolvePayload;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation;
use crate::reservation::AbandonmentReason;
use crate::reservation::IncursionIncident;
use crate::reservation::IncursionIncidentStatus;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::OrphanRetirementReason;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::Reservation;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::reservation::RewrittenIntegrationTrunkCommit;
use crate::session::SessionIdentityMappingPublication;

/// A parsed resolution request whose variant contains exactly its required evidence.
pub(crate) struct ResolveRequest {
    /// The reservation receiving the reservation or incident decision.
    pub(crate) reservation_id: ReservationId,
    /// The one user-selected resolution decision.
    pub(crate) decision:       ResolveDecision,
}

/// The reservation or incident decision accepted at the command boundary.
pub(crate) enum ResolveDecision {
    /// Apply one recovery decision to the named reservation.
    Reservation(ReservationRecoveryDecision),
    /// Answer the reservation's outstanding incursion incidents.
    Incursion(IncursionAnswerScope),
}

/// Which of a reservation's outstanding incursion incidents a disposition answers.
///
/// Answering one member of a backlog leaves the rest standing, and the notice that
/// reported them keeps firing, so clearing the set is its own disposition rather than
/// a loop the reader has to run by hand.
#[derive(Clone, Copy)]
pub(crate) enum IncursionAnswerScope {
    /// The single named incident.
    One(IncursionIncidentId),
    /// Every incident outstanding for the reservation.
    Every,
}

/// The mutually exclusive reservation recovery decisions.
pub(crate) enum ReservationRecoveryDecision {
    /// Move surviving work to the invoking replacement worktree.
    Recovered,
    /// Record a verified alternate commit already reachable from trunk.
    IntegratedAs(RewrittenIntegrationTrunkCommit),
    /// Record deliberate user-confirmed abandonment.
    Abandon(AbandonmentReason),
    /// Retire a confirmed orphan while preserving that distinct audit outcome.
    RetireOrphan(OrphanRetirementReason),
}

/// A parsed request to refresh one retained reservation's activity.
#[derive(Clone, Copy)]
pub(crate) struct RenewRequest {
    /// The reservation receiving a renewal fact.
    pub(crate) reservation_id: ReservationId,
}

/// Execute one user-confirmed recovery decision.
pub(crate) fn resolve(resolve_request: ResolveRequest) -> OutputEnvelope {
    let resolved_reservation_id = resolve_request.reservation_id;
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Resolve, &error.to_string());
        },
    };
    let mut reconciliation_report =
        match reconcile::reconcile(&invocation_directory, RecoveredBypassReporting::Defer) {
            Ok(Enrollment::Enrolled(reconciliation_report)) => reconciliation_report,
            Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            }) => {
                return OutputEnvelope::unconfigured(
                    CommandVerb::Resolve,
                    &expected_configuration_path,
                );
            },
            Err(error) => return error.into_output(CommandVerb::Resolve),
        };
    let output_envelope = match execute_resolution(resolve_request) {
        Ok(Enrollment::Enrolled(resolve_payload)) => {
            if !matches!(
                resolve_payload,
                ResolvePayload::IncursionResolved { .. }
                    | ResolvePayload::RecordedNow { .. }
                    | ResolvePayload::AlreadyRecordedBySameCoordinationActor { .. }
            ) {
                reconciliation_report
                    .alerts
                    .retain(|alert| alert.reservation_id() != resolved_reservation_id);
            }
            OutputEnvelope::resolved(resolve_payload)
        },
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => OutputEnvelope::unconfigured(CommandVerb::Resolve, &expected_configuration_path),
        Err(error) => error.into_output(CommandVerb::Resolve),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

/// Append one activity renewal without changing scopes, edges, or lifecycle.
pub(crate) fn renew(renew_request: RenewRequest) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Renew, &error.to_string());
        },
    };
    let reconciliation_report =
        match reconcile::reconcile(&invocation_directory, RecoveredBypassReporting::Defer) {
            Ok(Enrollment::Enrolled(reconciliation_report)) => reconciliation_report,
            Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            }) => {
                return OutputEnvelope::unconfigured(
                    CommandVerb::Renew,
                    &expected_configuration_path,
                );
            },
            Err(error) => return error.into_output(CommandVerb::Renew),
        };
    let output_envelope = match execute_renewal(renew_request) {
        Ok(()) => OutputEnvelope::renewed(renew_request.reservation_id),
        Err(error) => error.into_output(CommandVerb::Renew),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

fn execute_resolution(
    resolve_request: ResolveRequest,
) -> Result<Enrollment<ResolvePayload>, RecoveryError> {
    match resolve_request.decision {
        ResolveDecision::Reservation(recovery) => {
            execute_reservation_resolution(ReservationResolutionRequest {
                reservation_id: resolve_request.reservation_id,
                recovery,
            })
        },
        ResolveDecision::Incursion(scope) => {
            execute_incursion_resolution(resolve_request.reservation_id, scope)
                .map(Enrollment::Enrolled)
        },
    }
}

struct ReservationResolutionRequest {
    reservation_id: ReservationId,
    recovery:       ReservationRecoveryDecision,
}

fn execute_incursion_resolution(
    reservation_id: ReservationId,
    scope: IncursionAnswerScope,
) -> Result<ResolvePayload, RecoveryError> {
    match scope {
        IncursionAnswerScope::One(incident_id) => {
            execute_one_incursion_resolution(reservation_id, incident_id)
        },
        IncursionAnswerScope::Every => execute_every_incursion_resolution(reservation_id),
    }
}

/// Answer every incident outstanding for one reservation in a single disposition.
fn execute_every_incursion_resolution(
    reservation_id: ReservationId,
) -> Result<ResolvePayload, RecoveryError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let journal_mutation_actor = ledger::resolve_identity(&worktree_context)?;
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let outcome = ledger.transact_reconciliation(
        journal_mutation_actor.worktree_id,
        journal_mutation_actor.coordination_run_id,
        |state| {
            let reservations = match RetainedReservationSet::replay(state.events()) {
                Ok(reservations) => reservations,
                Err(error) => {
                    return ReconciliationValidation::Reject(RecoveryRejection::Replay(error));
                },
            };
            let incident_ids = reservations
                .outstanding_incursion_incidents()
                .filter(|incident| incident.reservation_id() == reservation_id)
                .map(IncursionIncident::id)
                .collect::<Vec<_>>();
            if incident_ids.is_empty() {
                return ReconciliationValidation::Reject(
                    RecoveryRejection::NoOutstandingIncursion(reservation_id),
                );
            }
            ReconciliationValidation::Apply {
                operations:             incident_ids
                    .iter()
                    .map(|incident_id| JournalOperation::ResolveIncursion {
                        incident_id: *incident_id,
                    })
                    .collect(),
                recoverable_operations: Vec::new(),
                action:                 incident_ids,
            }
        },
        |incident_ids, _, _| Ok::<Vec<IncursionIncidentId>, Infallible>(incident_ids),
    );
    match outcome {
        Ok(LedgerCommittedActionOutcome::Appended {
            output: incident_ids,
            ..
        }) => Ok(ResolvePayload::EveryIncursionResolved {
            reservation_id,
            incident_ids,
        }),
        Ok(LedgerCommittedActionOutcome::Rejected(rejection)) => {
            Err(RecoveryError::Rejected(rejection))
        },
        Err(LedgerCommittedActionError::Transaction(error)) => Err(error.into()),
        Err(LedgerCommittedActionError::Action(error)) => match error {},
    }
}

fn execute_one_incursion_resolution(
    reservation_id: ReservationId,
    incident_id: IncursionIncidentId,
) -> Result<ResolvePayload, RecoveryError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let journal_mutation_actor = ledger::resolve_identity(&worktree_context)?;
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let outcome = ledger.transact(
        journal_mutation_actor.worktree_id,
        journal_mutation_actor.coordination_run_id,
        |state| {
            let reservations = match RetainedReservationSet::replay(state.events()) {
                Ok(reservations) => reservations,
                Err(error) => {
                    return TransactionValidation::Reject(
                        IncursionResolutionNotAppended::Rejected(RecoveryRejection::Replay(error)),
                    );
                },
            };
            let incident = match reservations.incursion_incident(incident_id) {
                Ok(incident) => incident,
                Err(ReservationReplayError::UnknownIncursionIncident(_)) => {
                    return TransactionValidation::Reject(
                        IncursionResolutionNotAppended::Rejected(
                            RecoveryRejection::UnknownIncursionIncident(incident_id),
                        ),
                    );
                },
                Err(error) => {
                    return TransactionValidation::Reject(
                        IncursionResolutionNotAppended::Rejected(RecoveryRejection::Replay(error)),
                    );
                },
            };
            if incident.reservation_id() != reservation_id {
                return TransactionValidation::Reject(
                    IncursionResolutionNotAppended::Rejected(
                        RecoveryRejection::IncursionReservationMismatch {
                            incident_id,
                            reservation_id,
                        },
                    ),
                );
            }
            if let IncursionIncidentStatus::Resolved {
                resolving_actor,
                resolution_event_id,
                resolved_at,
            } = incident.status()
            {
                if resolving_actor.has_coordination_identity(
                    journal_mutation_actor.worktree_id,
                    journal_mutation_actor.coordination_run_id,
                ) {
                    return TransactionValidation::Reject(
                        IncursionResolutionNotAppended::AlreadyRecordedBySameCoordinationActor,
                    );
                }
                return TransactionValidation::Reject(IncursionResolutionNotAppended::Rejected(
                    RecoveryRejection::IncursionIncidentAlreadyResolvedByDifferentCoordinationActor {
                        reservation_id,
                        incident_id,
                        resolving_actor: resolving_actor.clone(),
                        resolution_event_id: *resolution_event_id,
                        resolved_at: resolved_at.clone(),
                    },
                ));
            }
            TransactionValidation::Append(Box::new(JournalOperation::ResolveIncursion {
                incident_id,
            }))
        },
    )?;
    match outcome {
        LedgerTransactionOutcome::Appended { .. } => Ok(ResolvePayload::RecordedNow {
            reservation_id,
            incident_id,
        }),
        LedgerTransactionOutcome::Rejected(
            IncursionResolutionNotAppended::AlreadyRecordedBySameCoordinationActor,
        ) => Ok(ResolvePayload::AlreadyRecordedBySameCoordinationActor {
            reservation_id,
            incident_id,
        }),
        LedgerTransactionOutcome::Rejected(IncursionResolutionNotAppended::Rejected(rejection)) => {
            Err(RecoveryError::Rejected(rejection))
        },
    }
}

fn execute_reservation_resolution(
    resolve_request: ReservationResolutionRequest,
) -> Result<Enrollment<ResolvePayload>, RecoveryError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let journal_mutation_actor = ledger::resolve_identity(&worktree_context)?;
    let repository_root = worktree_context.repository_root();
    let berth_config = match BerthConfig::read(repository_root)? {
        Enrollment::Enrolled(berth_config) => berth_config,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let current_worktree_root = canonical_root(&worktree_context)?;
    let ledger = Ledger::open(repository_root)?;
    let outcome = ledger
        .transact_with_committed_action(
            journal_mutation_actor.worktree_id,
            journal_mutation_actor.coordination_run_id,
            |state| {
                let recovery_request = match validate_recovery_request(
                    repository_root,
                    &berth_config.trunk,
                    resolve_request.recovery,
                ) {
                    Ok(recovery_request) => recovery_request,
                    Err(error) => return CommittedActionValidation::Reject(error),
                };
                let reservations = match RetainedReservationSet::replay(state.events()) {
                    Ok(reservations) => reservations,
                    Err(error) => {
                        return CommittedActionValidation::Reject(RecoveryRejection::Replay(error));
                    },
                };
                let ordering_graph = match OrderingGraph::replay(state.events()) {
                    Ok(ordering_graph) => ordering_graph,
                    Err(error) => {
                        return CommittedActionValidation::Reject(RecoveryRejection::EdgeReplay(
                            error,
                        ));
                    },
                };
                let reservation = match reservations.reservation(resolve_request.reservation_id) {
                    Ok(reservation) => reservation,
                    Err(ReservationReplayError::UnknownReservation(_)) => {
                        return CommittedActionValidation::Reject(
                            RecoveryRejection::UnknownReservation,
                        );
                    },
                    Err(error) => {
                        return CommittedActionValidation::Reject(RecoveryRejection::Replay(error));
                    },
                };
                match recovery_operation(
                    reservation,
                    resolve_request.reservation_id,
                    recovery_request,
                    journal_mutation_actor.worktree_id,
                    current_worktree_root,
                    worktree_context.administrative_locator().clone(),
                ) {
                    Ok((operation, resolve_payload_seed, committed_action)) => {
                        let retention_deletions = match recovery_retention_deletions(
                            &operation,
                            &ordering_graph,
                            resolve_request.reservation_id,
                            &reservations,
                        ) {
                            Ok(retention_deletions) => retention_deletions,
                            Err(error) => return CommittedActionValidation::Reject(error),
                        };
                        CommittedActionValidation::Append {
                            operation: Box::new(operation),
                            action:    RecoveryCommittedAction {
                                committed_action,
                                resolve_payload_seed,
                                retention_deletions,
                            },
                        }
                    },
                    Err(error) => CommittedActionValidation::Reject(error),
                }
            },
            |committed_action| committed_action.commit(&worktree_context),
        )
        .map_err(|error| match error {
            LedgerCommittedActionError::Transaction(error) => RecoveryError::Transaction(error),
            LedgerCommittedActionError::Action(error) => error,
        })?;
    resolution_enrollment_from_outcome(outcome)
}

/// Convert one committed reservation-resolution transaction into its enrolled result.
fn resolution_enrollment_from_outcome(
    outcome: LedgerCommittedActionOutcome<RecoveryRejection, ResolvePayloadSeed>,
) -> Result<Enrollment<ResolvePayload>, RecoveryError> {
    match outcome {
        LedgerCommittedActionOutcome::Appended {
            output: resolve_payload_seed,
            session_mapping_publication,
        } => Ok(Enrollment::Enrolled(
            resolve_payload_seed.into_payload(session_mapping_publication),
        )),
        LedgerCommittedActionOutcome::Rejected(RecoveryRejection::Git(error)) => {
            Err(RecoveryError::Git(error))
        },
        LedgerCommittedActionOutcome::Rejected(rejection) => {
            Err(RecoveryError::Rejected(rejection))
        },
    }
}

fn recovery_retention_deletions(
    operation: &JournalOperation,
    ordering_graph: &OrderingGraph,
    terminal_successor: ReservationId,
    reservations: &RetainedReservationSet,
) -> Result<Vec<ReservationId>, RecoveryRejection> {
    if !matches!(operation, JournalOperation::Release { .. }) {
        return Ok(Vec::new());
    }
    ordering_graph
        .retention_refs_retired_by_terminal(terminal_successor, reservations)
        .map_err(RecoveryRejection::Replay)
}

fn execute_renewal(renew_request: RenewRequest) -> Result<(), RecoveryError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let journal_mutation_actor = ledger::resolve_identity(&worktree_context)?;
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let outcome = ledger.transact(
        journal_mutation_actor.worktree_id,
        journal_mutation_actor.coordination_run_id,
        |state| {
            let reservations = match RetainedReservationSet::replay(state.events()) {
                Ok(reservations) => reservations,
                Err(error) => {
                    return TransactionValidation::Reject(RecoveryRejection::Replay(error));
                },
            };
            let reservation = match reservations.reservation(renew_request.reservation_id) {
                Ok(reservation) => reservation,
                Err(ReservationReplayError::UnknownReservation(_)) => {
                    return TransactionValidation::Reject(RecoveryRejection::UnknownReservation);
                },
                Err(error) => {
                    return TransactionValidation::Reject(RecoveryRejection::Replay(error));
                },
            };
            if matches!(
                reservation.lifecycle(),
                ReservationLifecycle::Released { .. }
            ) {
                return TransactionValidation::Reject(RecoveryRejection::AlreadyResolved);
            }
            TransactionValidation::Append(Box::new(JournalOperation::Renew {
                reservation_id: renew_request.reservation_id,
            }))
        },
    )?;
    match outcome {
        LedgerTransactionOutcome::Appended { .. } => Ok(()),
        LedgerTransactionOutcome::Rejected(rejection) => Err(RecoveryError::Rejected(rejection)),
    }
}

fn validate_recovery_request(
    repository_root: &Path,
    trunk_branch: &str,
    recovery_request: ReservationRecoveryDecision,
) -> Result<ReservationRecoveryDecision, RecoveryRejection> {
    let ReservationRecoveryDecision::IntegratedAs(trunk_commit) = recovery_request else {
        return Ok(recovery_request);
    };
    let trunk_oid = reservation::current_trunk(repository_root, trunk_branch)
        .map_err(RecoveryRejection::Git)?;
    match git::reachability(repository_root, trunk_commit.as_ref(), &trunk_oid)
        .map_err(RecoveryRejection::Git)?
    {
        Reachability::Ancestor => Ok(ReservationRecoveryDecision::IntegratedAs(trunk_commit)),
        Reachability::NotAncestor | Reachability::ObjectUnknown => {
            Err(RecoveryRejection::UnreachableIntegrationEvidence)
        },
    }
}

fn recovery_operation(
    reservation: &Reservation,
    reservation_id: ReservationId,
    recovery_request: ReservationRecoveryDecision,
    current_worktree_id: WorktreeId,
    current_worktree_root: CanonicalWorktreeRoot,
    current_worktree_administrative_locator: WorktreeAdministrativeLocator,
) -> Result<(JournalOperation, ResolvePayloadSeed, RecoveryAction), RecoveryRejection> {
    match recovery_request {
        ReservationRecoveryDecision::Recovered => {
            if reservation.actor().worktree == current_worktree_id {
                return Err(RecoveryRejection::SameWorktreeRecovery);
            }
            let committed_action = match reservation.lifecycle() {
                ReservationLifecycle::Active => {
                    RecoveryAction::PublishMarker(reservation.actor().run)
                },
                ReservationLifecycle::Outstanding { .. } => RecoveryAction::None,
                ReservationLifecycle::Released { .. } => {
                    return Err(RecoveryRejection::AlreadyResolved);
                },
            };
            Ok((
                JournalOperation::RebindWorktree {
                    reservation_id,
                    previous_worktree_id: reservation.actor().worktree,
                    current_worktree_id,
                    current_worktree_root,
                    current_worktree_administrative_locator,
                },
                ResolvePayloadSeed::Recovered {
                    reservation_id,
                    worktree_id: current_worktree_id,
                },
                committed_action,
            ))
        },
        ReservationRecoveryDecision::IntegratedAs(trunk_commit) => {
            let disposition = ReleaseDisposition::RewrittenIntegration(trunk_commit);
            let evidence_state = reservation
                .evidence_state()
                .map_err(RecoveryRejection::Replay)?;
            let operation = match evidence_state {
                ReservationEvidenceState::Outstanding { .. } => JournalOperation::Release {
                    reservation_id,
                    disposition: disposition.clone(),
                },
                ReservationEvidenceState::Released {
                    disposition: superseded,
                    integration_status:
                        IntegrationEvidenceStatus::NotIntegrated
                        | IntegrationEvidenceStatus::TrunkRewritten
                        | IntegrationEvidenceStatus::ObjectUnknown,
                    ..
                } if !matches!(
                    superseded.revalidation_subject(),
                    ReleaseRevalidationSubject::None
                ) =>
                {
                    JournalOperation::ReplaceReleaseDisposition {
                        reservation_id,
                        superseded,
                        replacement: disposition.clone(),
                    }
                },
                ReservationEvidenceState::Released { .. }
                | ReservationEvidenceState::ReleasedWithoutCheckpoint { .. } => {
                    return Err(RecoveryRejection::AlreadyResolved);
                },
                ReservationEvidenceState::Active { .. } => {
                    return Err(RecoveryRejection::CheckpointRequired);
                },
            };
            Ok((
                operation,
                ResolvePayloadSeed::Released {
                    reservation_id,
                    disposition,
                },
                RecoveryAction::None,
            ))
        },
        ReservationRecoveryDecision::Abandon(reason) => disposition_operation(
            reservation,
            reservation_id,
            ReleaseDisposition::Abandoned(reason),
        ),
        ReservationRecoveryDecision::RetireOrphan(reason) => disposition_operation(
            reservation,
            reservation_id,
            ReleaseDisposition::RetiredOrphan(reason),
        ),
    }
}

fn disposition_operation(
    reservation: &Reservation,
    reservation_id: ReservationId,
    disposition: ReleaseDisposition,
) -> Result<(JournalOperation, ResolvePayloadSeed, RecoveryAction), RecoveryRejection> {
    match reservation.lifecycle() {
        ReservationLifecycle::Active | ReservationLifecycle::Outstanding { .. } => Ok((
            JournalOperation::Release {
                reservation_id,
                disposition: disposition.clone(),
            },
            ResolvePayloadSeed::Released {
                reservation_id,
                disposition,
            },
            RecoveryAction::None,
        )),
        ReservationLifecycle::Released { .. } => Err(RecoveryRejection::AlreadyResolved),
    }
}

fn canonical_root(
    worktree_context: &WorktreeContext,
) -> Result<CanonicalWorktreeRoot, RecoveryError> {
    worktree_context
        .repository_root()
        .to_str()
        .ok_or(RecoveryError::NonUtf8WorktreeRoot)?
        .parse()
        .map_err(|_| RecoveryError::InvalidCanonicalWorktreeRoot)
}

struct RecoveryCommittedAction {
    committed_action:     RecoveryAction,
    resolve_payload_seed: ResolvePayloadSeed,
    retention_deletions:  Vec<ReservationId>,
}

enum RecoveryAction {
    None,
    PublishMarker(CoordinationRunId),
}

impl RecoveryCommittedAction {
    fn commit(
        self,
        worktree_context: &WorktreeContext,
    ) -> Result<ResolvePayloadSeed, RecoveryError> {
        git::update_reservation_retention_refs(
            worktree_context.repository_root(),
            &[],
            &self.retention_deletions,
        )?;
        match self.committed_action {
            RecoveryAction::None => {},
            RecoveryAction::PublishMarker(coordination_run_id) => {
                worktree_context.publish_coordination_run_marker(coordination_run_id)?;
            },
        }
        Ok(self.resolve_payload_seed)
    }
}

enum ResolvePayloadSeed {
    Recovered {
        reservation_id: ReservationId,
        worktree_id:    WorktreeId,
    },
    Released {
        reservation_id: ReservationId,
        disposition:    ReleaseDisposition,
    },
}

impl ResolvePayloadSeed {
    fn into_payload(
        self,
        session_mapping_publication: SessionIdentityMappingPublication,
    ) -> ResolvePayload {
        match self {
            Self::Recovered {
                reservation_id,
                worktree_id,
            } => ResolvePayload::Recovered {
                reservation_id,
                worktree_id,
            },
            Self::Released {
                reservation_id,
                disposition,
            } => ResolvePayload::Released {
                reservation_id,
                disposition,
                session_mapping_publication,
            },
        }
    }
}

#[derive(Debug)]
enum RecoveryRejection {
    UnknownReservation,
    UnknownIncursionIncident(IncursionIncidentId),
    IncursionIncidentAlreadyResolvedByDifferentCoordinationActor {
        reservation_id:      ReservationId,
        incident_id:         IncursionIncidentId,
        resolving_actor:     JournalActor,
        resolution_event_id: EventId,
        resolved_at:         RecordedAt,
    },
    IncursionReservationMismatch {
        incident_id:    IncursionIncidentId,
        reservation_id: ReservationId,
    },
    NoOutstandingIncursion(ReservationId),
    Replay(ReservationReplayError),
    CheckpointRequired,
    AlreadyResolved,
    SameWorktreeRecovery,
    UnreachableIntegrationEvidence,
    Git(GitError),
    EdgeReplay(EdgeReplayError),
}

#[derive(Debug)]
enum IncursionResolutionNotAppended {
    AlreadyRecordedBySameCoordinationActor,
    Rejected(RecoveryRejection),
}

#[derive(Debug)]
enum RecoveryError {
    Io(Error),
    Config(ConfigError),
    Git(GitError),
    Ledger(LedgerError),
    Transaction(LedgerTransactionError),
    Rejected(RecoveryRejection),
    NonUtf8WorktreeRoot,
    InvalidCanonicalWorktreeRoot,
}

impl RecoveryError {
    fn into_output(self, command_verb: CommandVerb) -> OutputEnvelope {
        match self {
            Self::Transaction(LedgerTransactionError::LockContention) => {
                OutputEnvelope::contention(
                    command_verb,
                    &LedgerTransactionError::LockContention.to_string(),
                )
            },
            Self::Transaction(LedgerTransactionError::CorrectableInput(error)) => {
                OutputEnvelope::invalid_input(command_verb, &error.to_string())
            },
            Self::Rejected(RecoveryRejection::Replay(error)) => {
                OutputEnvelope::replay_failure(command_verb, &error)
            },
            Self::Rejected(RecoveryRejection::EdgeReplay(error)) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
            Self::Rejected(
                RecoveryRejection::IncursionIncidentAlreadyResolvedByDifferentCoordinationActor {
                    reservation_id,
                    incident_id,
                    resolving_actor,
                    resolution_event_id,
                    resolved_at,
                },
            ) => OutputEnvelope::incursion_resolution_recorded_by_different_actor(
                reservation_id,
                incident_id,
                resolving_actor.worktree,
                resolving_actor.run,
                resolution_event_id,
                resolved_at,
            ),
            Self::Rejected(rejection) => {
                OutputEnvelope::invalid_input(command_verb, &rejection.to_string())
            },
            Self::Io(error) => OutputEnvelope::ledger_unreadable(command_verb, &error.to_string()),
            Self::Config(error) => {
                OutputEnvelope::ledger_error(command_verb, &LedgerError::Config(error))
            },
            Self::Git(error) => OutputEnvelope::ledger_unreadable(command_verb, &error.to_string()),
            Self::Ledger(error)
            | Self::Transaction(LedgerTransactionError::LedgerUnreadable(error)) => {
                OutputEnvelope::ledger_error(command_verb, &error)
            },
            Self::NonUtf8WorktreeRoot => OutputEnvelope::ledger_unreadable(
                command_verb,
                "the replacement worktree root is not UTF-8",
            ),
            Self::InvalidCanonicalWorktreeRoot => OutputEnvelope::ledger_unreadable(
                command_verb,
                "the replacement worktree root is not canonical",
            ),
        }
    }
}

impl Display for RecoveryRejection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownReservation => formatter.write_str("the reservation does not exist"),
            Self::UnknownIncursionIncident(incident_id) => {
                write!(formatter, "incursion incident {incident_id} does not exist")
            },
            Self::IncursionIncidentAlreadyResolvedByDifferentCoordinationActor {
                incident_id,
                resolving_actor,
                resolution_event_id,
                resolved_at,
                ..
            } => write!(
                formatter,
                "incursion incident {incident_id} was already resolved by worktree {} in coordination run {}, event {resolution_event_id} at {resolved_at}",
                resolving_actor.worktree,
                resolving_actor.run
            ),
            Self::IncursionReservationMismatch {
                incident_id,
                reservation_id,
            } => write!(
                formatter,
                "incursion incident {incident_id} does not belong to reservation {reservation_id}"
            ),
            Self::NoOutstandingIncursion(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} has no outstanding incursion incident"
            ),
            Self::Replay(error) => error.fmt(formatter),
            Self::CheckpointRequired => formatter.write_str(
                "the reservation must have a protected checkpoint first; cargo-berth release records one",
            ),
            Self::AlreadyResolved => formatter.write_str("the reservation is already resolved"),
            Self::SameWorktreeRecovery => formatter
                .write_str("--recovered requires a replacement worktree with a new identity"),
            Self::UnreachableIntegrationEvidence => formatter.write_str(
                "the --integrated-as commit must resolve in this repository and be reachable from trunk",
            ),
            Self::Git(error) => error.fmt(formatter),
            Self::EdgeReplay(error) => error.fmt(formatter),
        }
    }
}

impl From<Error> for RecoveryError {
    fn from(error: Error) -> Self { Self::Io(error) }
}

impl From<ConfigError> for RecoveryError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<GitError> for RecoveryError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

impl From<LedgerError> for RecoveryError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}

impl From<LedgerTransactionError> for RecoveryError {
    fn from(error: LedgerTransactionError) -> Self { Self::Transaction(error) }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::CommandVerb;
    use super::RecoveryError;
    use super::RecoveryRejection;
    use super::ReservationId;
    use super::ReservationReplayError;

    #[test]
    fn recovery_replay_rejection_preserves_reason_and_subject() {
        let reservation_id = ReservationId::new();
        let output_envelope = RecoveryError::Rejected(RecoveryRejection::Replay(
            ReservationReplayError::UnknownReservation(reservation_id),
        ))
        .into_output(CommandVerb::Resolve);
        let value =
            serde_json::to_value(output_envelope).expect("recovery response should serialize");
        assert_eq!(value["payload"]["kind"], "replay_failure");
        assert_eq!(value["payload"]["data"]["reason"], "unknown_reservation");
        assert_eq!(
            value["payload"]["data"]["subject"],
            serde_json::json!({
                "kind": "reservation",
                "id": reservation_id.to_string(),
            })
        );
    }
}
