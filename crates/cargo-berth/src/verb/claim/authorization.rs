//! Overlap answers and ordering authorization.

use std::collections::HashSet;

use super::failure::ClaimRejection;
use super::incomplete_answer::IncompleteClaimAnswer;
use super::incomplete_answer::IncompleteClaimAnswerRejection;
use super::request::ClaimReservationSelection;
use crate::answer::AuthorizedOverlap;
use crate::answer::AuthorizedOverlapSet;
use crate::answer::ClaimSequenceBinding;
use crate::answer::ConflictAuthorization;
use crate::answer::ContradictedOrdering;
use crate::answer::DeferAnswerRequest;
use crate::answer::OrderingReplacement;
use crate::answer::PermissiveOverlapAnswer;
use crate::answer::PermissiveOverlapAuthorizationRequest;
use crate::config::DefaultAnswer;
use crate::edge::OrderingGraph;
use crate::ids::CoordinationRunId;
use crate::ids::WireOrderedReservationIds;
use crate::ids::WorktreeId;
use crate::ledger::JournalOperation;
use crate::ledger::ReservationScopeSet;
use crate::ledger::TransactionValidation;
use crate::reservation::Reservation;
use crate::reservation::ReservationConflict;
use crate::reservation::RetainedReservationSet;

/// The answer the repository's default answer records: one answer to every holder in
/// `conflicts`, with the engine's reason naming the default.
pub(super) fn default_answer_request(
    default_answer: DefaultAnswer,
    conflicts: &[ReservationConflict],
) -> DeferAnswerRequest {
    DeferAnswerRequest {
        blockers: conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect(),
        reason:   default_answer.into(),
    }
}

/// The acting run and worktree a `--defer` claim or the default answer answers for.
#[derive(Clone, Copy)]
pub(super) struct AnsweringWorktree {
    pub(super) coordination_run_id: CoordinationRunId,
    pub(super) worktree_id:         WorktreeId,
}

/// The [`ConflictAuthorization`] [`validate_answer`] records against every holder.
#[derive(Clone, Copy)]
pub(super) enum AnswerKind {
    /// [`ConflictAuthorization::Defer`]: a `--defer` claim; integration waits for an order.
    Defer,
    /// [`ConflictAuthorization::Override`]: the repository's default answer; no order is
    /// recorded, so whichever checkpoint is ready first merges first.
    Override,
}

/// Record one answer to every current holder as one [`JournalOperation::Answer`].
///
/// The named blockers must be exactly the holders `conflicts_for_claim` reports, one per foreign
/// worktree, so one call answers every holder of the requested paths. The answer acquires no
/// reservation, so it protects nothing from any lane. When the acting run holds an active
/// reservation in this worktree, the oldest one is named: the ordering graph holds it against
/// each blocker of a [`AnswerKind::Defer`] at integration, and adds nothing for an
/// [`AnswerKind::Override`]. With no active reservation the answer is recorded with
/// `reservation_id: None`; the ordering graph and the board ignore it, and an override has
/// nothing to hold anyway.
pub(super) fn validate_answer(
    answer_kind: AnswerKind,
    request: DeferAnswerRequest,
    conflicts: Vec<ReservationConflict>,
    reservations: &RetainedReservationSet,
    answering_worktree: AnsweringWorktree,
    scopes: ReservationScopeSet,
) -> TransactionValidation<ClaimRejection> {
    let authorizations = match bind_answer_authorizations(answer_kind, request, conflicts) {
        Ok(authorizations) => authorizations,
        Err(rejection) => return TransactionValidation::Reject(rejection),
    };
    let AnsweringWorktree {
        coordination_run_id,
        worktree_id,
    } = answering_worktree;
    let reservation_id = reservations
        .iter()
        .find(|reservation| {
            reservation
                .is_active_for_coordination_run_and_worktree(coordination_run_id, worktree_id)
        })
        .map(Reservation::id);
    TransactionValidation::Append(Box::new(JournalOperation::Answer {
        reservation_id,
        scopes,
        authorizations,
    }))
}

/// Validate the named holders and bind answer-only authorizations to the locked overlaps.
pub(super) fn bind_answer_authorizations(
    answer_kind: AnswerKind,
    request: DeferAnswerRequest,
    conflicts: Vec<ReservationConflict>,
) -> Result<Vec<ConflictAuthorization>, ClaimRejection> {
    if conflicts.is_empty() {
        return Err(ClaimRejection::AnsweredWithoutOverlap(request.blockers));
    }
    let named_blockers = request.blockers.iter().copied().collect::<HashSet<_>>();
    let holders = conflicts
        .iter()
        .map(|conflict| conflict.reservation_id)
        .collect::<HashSet<_>>();
    if named_blockers != holders {
        return Err(ClaimRejection::IncompleteAnswer(Box::new(
            IncompleteClaimAnswerRejection::new(conflicts, IncompleteClaimAnswer::Defer(request)),
        )));
    }
    let reason = request.reason;
    Ok(conflicts
        .iter()
        .map(|conflict| {
            let overlaps = AuthorizedOverlap::from(conflict).into();
            let blocker = conflict.reservation_id;
            let reason = reason.clone();
            match answer_kind {
                AnswerKind::Defer => ConflictAuthorization::Defer {
                    overlaps,
                    blocker,
                    reason,
                },
                AnswerKind::Override => ConflictAuthorization::Override {
                    overlaps,
                    blocker,
                    reason,
                },
            }
        })
        .collect())
}

/// The holders a recorded [`JournalOperation::Answer`] named, by deferral or override.
pub(super) fn answered_blockers(
    authorizations: &[ConflictAuthorization],
) -> WireOrderedReservationIds {
    WireOrderedReservationIds::sorted_and_deduplicated(
        authorizations
            .iter()
            .filter_map(|authorization| match authorization {
                ConflictAuthorization::Defer { blocker, .. }
                | ConflictAuthorization::Override { blocker, .. } => Some(*blocker),
                ConflictAuthorization::NoConflict
                | ConflictAuthorization::Enrollment { .. }
                | ConflictAuthorization::Sequence { .. }
                | ConflictAuthorization::SequencePerHolder { .. }
                | ConflictAuthorization::SequenceWithExistingEdges { .. }
                | ConflictAuthorization::ExistingAnswersCoverEveryOverlap { .. } => None,
            })
            .collect(),
    )
}

/// Bind an overlap answer independently of whether it appends or widens a reservation.
pub(super) fn bind_claim_authorization(
    request: PermissiveOverlapAuthorizationRequest,
    conflicts: Vec<ReservationConflict>,
    ordering_context: &OrderingContext<'_>,
    maximum_ordering_edges: u32,
) -> Result<ConflictAuthorization, ClaimRejection> {
    let PermissiveOverlapAuthorizationRequest { answer, reason } = request;
    let Ok(overlaps) = AuthorizedOverlapSet::try_from(
        conflicts
            .iter()
            .map(AuthorizedOverlap::from)
            .collect::<Vec<_>>(),
    ) else {
        return Err(ClaimRejection::AnsweredWithoutOverlap(answer.blockers()));
    };
    let named_blockers = answer.blockers().into_iter().collect::<HashSet<_>>();
    let holders = conflicts
        .iter()
        .map(|conflict| conflict.reservation_id)
        .collect::<HashSet<_>>();
    if named_blockers != holders {
        return Err(ClaimRejection::IncompleteAnswer(Box::new(
            IncompleteClaimAnswerRejection::new(
                conflicts,
                IncompleteClaimAnswer::Permissive(PermissiveOverlapAuthorizationRequest {
                    answer,
                    reason,
                }),
            ),
        )));
    }
    let replaced = replaced_orderings(&answer, ordering_context)?;
    let authorization = match (ordering_context.reservation_selection, &answer) {
        (
            ClaimReservationSelection::Existing(requester),
            PermissiveOverlapAnswer::Sequence { blockers, .. },
        ) => ConflictAuthorization::sequence_with_bindings(
            overlaps,
            blockers
                .iter()
                .map(|blocker| {
                    ordering_context
                        .ordering_graph
                        .bind_claim_sequence(requester, *blocker, &reason)
                })
                .collect(),
            reason,
            replaced,
        ),
        _ => ConflictAuthorization::answered(&answer, overlaps, reason, replaced),
    };
    let new_edge_count = match &authorization {
        ConflictAuthorization::Sequence { .. } => 1,
        ConflictAuthorization::SequencePerHolder { holders, .. } => holders.len(),
        ConflictAuthorization::SequenceWithExistingEdges { holders, .. } => holders
            .iter()
            .filter(|binding| matches!(binding, ClaimSequenceBinding::NewEdge { .. }))
            .count(),
        ConflictAuthorization::NoConflict
        | ConflictAuthorization::Enrollment { .. }
        | ConflictAuthorization::Defer { .. }
        | ConflictAuthorization::Override { .. }
        | ConflictAuthorization::ExistingAnswersCoverEveryOverlap { .. } => 0,
    };
    if new_edge_count != 0
        && additions_exceed_limit(
            ordering_context
                .ordering_graph
                .edge_count()
                .saturating_sub(authorization.replaced().len()),
            new_edge_count,
            maximum_ordering_edges,
        )
    {
        return Err(ClaimRejection::OrderingEdgeLimitReached(
            maximum_ordering_edges,
        ));
    }
    Ok(authorization)
}

/// The locked facts a sequence answer is checked against for contradicted live edges.
pub(super) struct OrderingContext<'state> {
    pub(super) reservation_selection: ClaimReservationSelection,
    pub(super) ordering_graph:        &'state OrderingGraph,
    pub(super) reservations:          &'state RetainedReservationSet,
    pub(super) answering_worktree:    AnsweringWorktree,
}

/// Return the live edges a sequence answer retires, or the rejection that stops it.
///
/// The claimant's own reservations are the nonterminal ones of its coordination run and
/// worktree. An edge between one of them and a named holder that points opposite to the order
/// the answer chose is a contradiction: without `--replace` the claim is refused, so two live
/// edges never order the same work both ways; with it the edges are recorded as retired. A
/// `--replace` that contradicts nothing is refused too, so a stale re-ruling never records
/// silently. An override orders nothing, so it retires nothing.
fn replaced_orderings(
    answer: &PermissiveOverlapAnswer,
    ordering_context: &OrderingContext<'_>,
) -> Result<Vec<ContradictedOrdering>, ClaimRejection> {
    let PermissiveOverlapAnswer::Sequence {
        blockers,
        replacement,
    } = answer
    else {
        return Ok(Vec::new());
    };
    let AnsweringWorktree {
        coordination_run_id,
        worktree_id,
    } = ordering_context.answering_worktree;
    let requesters = ordering_context
        .reservations
        .iter()
        .filter(|reservation| {
            matches!(ordering_context.reservation_selection, ClaimReservationSelection::NewReservation)
                || matches!(ordering_context.reservation_selection, ClaimReservationSelection::Existing(id) if reservation.id() == id)
        })
        .filter(|reservation| {
            !reservation.is_terminal()
                && reservation.actor().run == coordination_run_id
                && reservation.actor().worktree == worktree_id
        })
        .map(Reservation::id)
        .collect::<HashSet<_>>();
    let contradicted = blockers
        .iter()
        .flat_map(|sequenced_blocker| {
            ordering_context
                .ordering_graph
                .contradicted_orderings(*sequenced_blocker, &requesters)
        })
        .collect::<Vec<_>>();
    match (replacement, contradicted.is_empty()) {
        (OrderingReplacement::Keep, true) | (OrderingReplacement::Replace, false) => {
            Ok(contradicted)
        },
        (OrderingReplacement::Keep, false) => {
            Err(ClaimRejection::ContradictsLiveOrdering(contradicted))
        },
        (OrderingReplacement::Replace, true) => {
            Err(ClaimRejection::NothingToReplace(answer.blockers()))
        },
    }
}

/// Whether adding `additions` to `count` would exceed `maximum`.
fn additions_exceed_limit(count: usize, additions: usize, maximum: u32) -> bool {
    u64::try_from(count.saturating_add(additions)).map_or(true, |total| total > u64::from(maximum))
}

pub(super) fn count_reaches_limit(count: usize, maximum: u32) -> bool {
    u64::try_from(count).map_or(true, |count| count >= u64::from(maximum))
}
