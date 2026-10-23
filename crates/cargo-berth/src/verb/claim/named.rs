//! Claims that continue an explicitly selected reservation.

use std::convert::Infallible;
use std::sync::Arc;

use super::AnswerKind;
use super::AnsweringWorktree;
use super::ClaimAnswerPolicy;
use super::ClaimError;
use super::ClaimExecution;
use super::ClaimRejection;
use super::ClaimRequest;
use super::ClaimReservationSelection;
use super::ClaimValidationContext;
use super::CommittedReservationAcquisition;
use super::FirstTouchConflictOutcome;
use super::FirstTouchReservationSelection;
use super::OrderingContext;
use super::ReservationAcquisition;
use super::ReservationAcquisitionKind;
use super::bind_answer_authorizations;
use super::bind_claim_authorization;
use super::checked_claim_operation;
use super::claim_rejection_execution;
use super::default_answer_request;
use super::locked_claim_reservations;
use super::normalized_claim_scopes;
use super::publish_coordination_run_marker;
use super::record_refusal;
use super::sibling_held_scopes;
use crate::answer::ClaimSequenceBinding;
use crate::answer::ConflictAuthorization;
use crate::answer::OverlapAuthorizationRequest;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::coordination_identity::RecoveryCommandLine;
use crate::edge::CycleFreeOrderingOperation;
use crate::git;
use crate::ids::ReservationId;
use crate::ledger;
use crate::ledger::CommittedActionValidation;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::ReplayedLedgerState;
use crate::ledger::ReservationScope;
use crate::ledger::ReservationScopeAdditionSet;
use crate::ledger::ReservationScopeSet;
use crate::ledger::WidenCause;
use crate::ledger::WorktreeContext;
use crate::output::TargetView;
use crate::reservation::RecordedTarget;
use crate::reservation::Reservation;
use crate::reservation::ReservationConflict;
use crate::reservation::RetainedReservationSet;
use crate::scope::PathCase;
use crate::session;
use crate::session::PreparedSessionMappingPublication;

/// Resolve identity once, then bind the answer and scope change under the ledger lock.
pub(super) fn acquire(
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
            match validate(&state, reservation_id, scopes.clone(), &context) {
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
            consume(outcome, &context, &config)
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
fn consume(
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
fn validate(
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
            bind_claim_authorization(
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
            bind_answer_authorizations(AnswerKind::Defer, (**request).clone(), conflicts)
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
        ClaimAnswerPolicy::RepositoryDefault(default_answer) => bind_answer_authorizations(
            AnswerKind::Override,
            default_answer_request(*default_answer, &conflicts),
            conflicts,
        )
        .map(NamedAuthorization::Answer),
        ClaimAnswerPolicy::CallerRequest(OverlapAuthorizationRequest::Absent) => {
            Err(ClaimRejection::Conflict(conflicts))
        },
    }
}
