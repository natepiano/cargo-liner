//! The replayed ordering graph, its deferrals, and edge-declaration validation.

use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use super::DeferralOrigin;
use super::EdgeDeclaration;
use super::IntegrationConstraintProjection;
use super::IntegrationDeferralConstraint;
use super::IntegrationDeferralStatus;
use super::IntegrationHold;
use super::IntegrationOrderingConstraint;
use super::IntegrationReservationFacts;
use super::IntegrationSubject;
use super::MissingReadinessFact;
use super::OrderingEdge;
use super::OrderingOverlapScopeSet;
use super::OrderingReason;
use super::RepositoryReservationEvidence;
use super::RepositorySnapshot;
use super::cycle;
use crate::answer::ConflictAuthorization;
use crate::answer::OverlapAuthorizationReason;
use crate::ids::EdgeId;
use crate::ids::EventId;
use crate::ids::ProjectionGeneration;
use crate::ids::ReservationId;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::OrderingDirection;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::worktree::WorktreeHead;

#[derive(Clone, Debug, Eq, PartialEq)]
struct DeferredOverlap {
    declaration_event_id: EventId,
    deferred:             ReservationId,
    blocker:              ReservationId,
    scopes:               OrderingOverlapScopeSet,
    reason:               OverlapAuthorizationReason,
    origin:               DeferralOrigin,
    resolved:             DeferralResolution,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeferralResolution {
    Pending,
    Resolved,
}

#[derive(Clone, Copy)]
struct DeferredOverlapEndpoints {
    deferred: ReservationId,
    blocker:  ReservationId,
}

/// Adjacency and unresolved deferrals rebuilt from append-only facts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OrderingGraph {
    vertices:         HashSet<ReservationId>,
    edges:            Vec<OrderingEdge>,
    edge_ids:         HashSet<EdgeId>,
    endpoint_pairs:   HashSet<(ReservationId, ReservationId)>,
    adjacency:        HashMap<ReservationId, Vec<ReservationId>>,
    deferrals:        Vec<DeferredOverlap>,
    deferral_indices: HashMap<(ReservationId, ReservationId), Vec<usize>>,
}

impl OrderingGraph {
    /// Rebuild vertices, unresolved deferrals, edges, and adjacency in append order.
    pub(crate) fn replay(events: &[JournalEvent]) -> Result<Self, EdgeReplayError> {
        let mut graph = Self::default();
        for event in events {
            match &event.operation {
                JournalOperation::Claim {
                    reservation_id,
                    authorization,
                    ..
                } => {
                    graph.add_vertex(*reservation_id);
                    graph.apply_authorization(*reservation_id, authorization, event.event_id())?;
                },
                JournalOperation::Widen {
                    reservation_id,
                    authorization,
                    ..
                } => {
                    graph.apply_authorization(*reservation_id, authorization, event.event_id())?;
                },
                JournalOperation::ResolveDefer {
                    deferred_reservation_id,
                    blocker_reservation_id,
                    edge_id,
                    direction,
                    reason,
                } => graph.apply_resolution(
                    *deferred_reservation_id,
                    *blocker_reservation_id,
                    *edge_id,
                    *direction,
                    reason.clone(),
                    event.event_id(),
                )?,
                JournalOperation::Checkpoint { .. }
                | JournalOperation::Resnapshot { .. }
                | JournalOperation::Retarget { .. }
                | JournalOperation::UnrecordedTargetsPinned { .. }
                | JournalOperation::Renew { .. }
                | JournalOperation::Release { .. }
                | JournalOperation::ReplaceReleaseDisposition { .. }
                | JournalOperation::EvidenceRevalidated { .. }
                | JournalOperation::ScopedPatchEquivalenceChecked { .. }
                | JournalOperation::ScopedPatchComparisonAttempted { .. }
                | JournalOperation::SuccessorScopedPatchEquivalenceChecked { .. }
                | JournalOperation::SuccessorScopedPatchComparisonAttempted { .. }
                | JournalOperation::Incursion { .. }
                | JournalOperation::ResolveIncursion { .. }
                | JournalOperation::ForcedIntegrationPermit { .. }
                | JournalOperation::ConsumeForcedIntegrationPermit { .. }
                | JournalOperation::Bypass { .. }
                | JournalOperation::RebindWorktree { .. }
                | JournalOperation::MergeExtentObserved { .. }
                | JournalOperation::RelocateWorktree { .. } => {},
            }
        }
        for edge in &graph.edges {
            if !graph.vertices.contains(&edge.before) {
                return Err(EdgeReplayError::UnknownEndpoint(edge.before));
            }
            if !graph.vertices.contains(&edge.after) {
                return Err(EdgeReplayError::UnknownEndpoint(edge.after));
            }
        }
        if cycle::contains_cycle(&graph.adjacency) {
            return Err(EdgeReplayError::Cycle);
        }
        Ok(graph)
    }

    /// Return the number of durable ordering relationships.
    pub(crate) const fn edge_count(&self) -> usize { self.edges.len() }

    /// Iterate each distinct predecessor exactly once with its direct successors.
    pub(crate) fn predecessors(&self) -> impl Iterator<Item = GraphPredecessor<'_>> {
        self.adjacency
            .iter()
            .filter(|(_, successors)| !successors.is_empty())
            .map(|(reservation_id, successors)| GraphPredecessor {
                reservation_id: *reservation_id,
                successors,
            })
    }

    /// Build the gate-and-board projection without exposing graph internals.
    pub(crate) fn integration_constraints(
        &self,
        reservations: &RetainedReservationSet,
        repository_snapshot: &RepositorySnapshot,
        generation: ProjectionGeneration,
    ) -> Result<IntegrationConstraintProjection, MissingReadinessFact> {
        let reservation_facts = reservations
            .iter()
            .map(|reservation| {
                let snapshot = repository_snapshot.reservation(reservation.id())?;
                let subject = match &snapshot.evidence {
                    RepositoryReservationEvidence::Active => match &snapshot.worktree_head {
                        WorktreeHead::Resolved(object_id) => IntegrationSubject::Commit {
                            object_id: object_id.clone(),
                        },
                        WorktreeHead::Unavailable => IntegrationSubject::WorktreeHeadUnavailable,
                    },
                    RepositoryReservationEvidence::Outstanding { protected_tip, .. }
                    | RepositoryReservationEvidence::Released { protected_tip, .. } => {
                        IntegrationSubject::Commit {
                            object_id: protected_tip.as_ref().clone(),
                        }
                    },
                    RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => {
                        IntegrationSubject::NotApplicable
                    },
                };
                Ok(IntegrationReservationFacts {
                    reservation_id: reservation.id(),
                    actor: reservation.actor().clone(),
                    source: reservation.source().clone(),
                    purpose: reservation.purpose().clone(),
                    scopes: reservation.scopes().clone(),
                    lifecycle: reservation.lifecycle().clone(),
                    subject,
                })
            })
            .collect::<Result<Vec<_>, MissingReadinessFact>>()?;
        let mut holds = Vec::new();
        let mut ordering_constraints = Vec::new();
        for edge in &self.edges {
            let readiness = edge.readiness(repository_snapshot)?;
            ordering_constraints.push(IntegrationOrderingConstraint {
                edge_id: edge.edge_id,
                predecessor: edge.before,
                successor: edge.after,
                ordering_target: edge.ordering_target(repository_snapshot),
                scopes: edge.scopes.0.clone(),
                reason: edge.reason.clone(),
                readiness,
                declaration: edge.declaration,
                declaration_event_id: edge.declaration_event_id,
            });
            if readiness.holds_successor() {
                holds.push(IntegrationHold::OrderingEdge {
                    edge_id: edge.edge_id,
                    predecessor: edge.before,
                    successor: edge.after,
                    ordering_target: edge.ordering_target(repository_snapshot),
                    scopes: edge.scopes.0.clone(),
                    reason: edge.reason.clone(),
                    readiness,
                });
            }
        }
        holds.extend(
            self.deferrals
                .iter()
                .filter(|deferral| deferral.resolved == DeferralResolution::Pending)
                .map(|deferral| IntegrationHold::DeferredOverlap {
                    declaration_event_id: deferral.declaration_event_id,
                    deferred:             deferral.deferred,
                    blocker:              deferral.blocker,
                    scopes:               deferral.scopes.0.clone(),
                    reason:               deferral.reason.clone(),
                }),
        );
        let deferrals = self
            .deferrals
            .iter()
            .map(|deferral| IntegrationDeferralConstraint {
                declaration_event_id: deferral.declaration_event_id,
                deferred:             deferral.deferred,
                blocker:              deferral.blocker,
                scopes:               deferral.scopes.0.clone(),
                reason:               deferral.reason.clone(),
                origin:               deferral.origin,
                status:               match deferral.resolved {
                    DeferralResolution::Pending => IntegrationDeferralStatus::Unresolved,
                    DeferralResolution::Resolved => IntegrationDeferralStatus::Resolved,
                },
            })
            .collect();
        Ok(IntegrationConstraintProjection {
            generation,
            reservations: reservation_facts,
            holds,
            ordering_constraints,
            deferrals,
        })
    }

    /// Validate and prepare one edge that resolves an existing deferral.
    pub(crate) fn prepare_deferred_edge(
        &self,
        before: ReservationId,
        after: ReservationId,
        reason: OrderingReason,
    ) -> Result<PreparedOrderingEdge, EdgeDeclarationRejection> {
        if before == after {
            return Err(EdgeDeclarationRejection::SameEndpoint);
        }
        if !self.vertices.contains(&before) {
            return Err(EdgeDeclarationRejection::UnknownEndpoint(before));
        }
        if !self.vertices.contains(&after) {
            return Err(EdgeDeclarationRejection::UnknownEndpoint(after));
        }
        if self.endpoint_pairs.contains(&(before, after)) {
            return Err(EdgeDeclarationRejection::Duplicate);
        }
        if cycle::would_create_cycle(&self.adjacency, before, after) {
            return Err(EdgeDeclarationRejection::Cycle);
        }
        let resolution = self.deferred_overlap_between(before, after)?;
        Ok(PreparedOrderingEdge {
            edge_id: EdgeId::new(),
            before,
            after,
            scopes: resolution.scopes,
            reason,
            deferral: resolution.endpoints,
        })
    }

    /// Return whether this predecessor still has a nonterminal dependent successor.
    pub(crate) fn has_nonterminal_dependent(
        &self,
        predecessor: ReservationId,
        reservations: &RetainedReservationSet,
    ) -> Result<bool, ReservationReplayError> {
        for successor in self.adjacency.get(&predecessor).into_iter().flatten() {
            let reservation = reservations.reservation(*successor)?;
            if !matches!(
                reservation.lifecycle(),
                ReservationLifecycle::Released { .. }
            ) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Find released predecessors whose final nonterminal successor is ending now.
    pub(crate) fn retention_refs_retired_by_terminal(
        &self,
        terminal_successor: ReservationId,
        reservations: &RetainedReservationSet,
    ) -> Result<Vec<ReservationId>, ReservationReplayError> {
        let mut predecessors = Vec::new();
        'predecessors: for edge in self
            .edges
            .iter()
            .filter(|edge| edge.after == terminal_successor)
        {
            let predecessor = reservations.reservation(edge.before)?;
            if !matches!(
                predecessor.lifecycle(),
                ReservationLifecycle::Released { .. }
            ) {
                continue;
            }
            for successor in self.adjacency.get(&edge.before).into_iter().flatten() {
                if *successor == terminal_successor {
                    continue;
                }
                if !matches!(
                    reservations.reservation(*successor)?.lifecycle(),
                    ReservationLifecycle::Released { .. }
                ) {
                    continue 'predecessors;
                }
            }
            if !predecessors.contains(&edge.before) {
                predecessors.push(edge.before);
            }
        }
        Ok(predecessors)
    }

    fn add_vertex(&mut self, reservation_id: ReservationId) {
        self.vertices.insert(reservation_id);
        self.adjacency.entry(reservation_id).or_default();
    }

    fn apply_authorization(
        &mut self,
        requester: ReservationId,
        authorization: &ConflictAuthorization,
        event_id: EventId,
    ) -> Result<(), EdgeReplayError> {
        match authorization {
            ConflictAuthorization::NoConflict
            | ConflictAuthorization::Override { .. }
            | ConflictAuthorization::ExistingAnswersCoverEveryOverlap { .. } => Ok(()),
            ConflictAuthorization::Enrollment { overlaps } => {
                let reason = OverlapAuthorizationReason::enrollment();
                let mut counterparts = HashSet::new();
                for overlap in overlaps.as_slice() {
                    let blocker = overlap.reservation_id;
                    if counterparts.insert(blocker) {
                        let scopes =
                            OrderingOverlapScopeSet::from_authorized_overlaps(blocker, overlaps)?;
                        self.add_deferral(
                            requester,
                            blocker,
                            scopes,
                            reason.clone(),
                            event_id,
                            DeferralOrigin::Enrollment,
                        );
                    }
                }
                Ok(())
            },
            ConflictAuthorization::Defer {
                overlaps,
                blocker,
                reason,
            } => {
                let scopes = OrderingOverlapScopeSet::from_authorized_overlaps(*blocker, overlaps)?;
                self.add_deferral(
                    requester,
                    *blocker,
                    scopes,
                    reason.clone(),
                    event_id,
                    DeferralOrigin::UserAnswer,
                );
                Ok(())
            },
            ConflictAuthorization::Sequence {
                overlaps,
                blocker,
                direction,
                edge_id,
                reason,
            } => {
                let (before, after) = directed_endpoints(requester, *blocker, *direction);
                let scopes = OrderingOverlapScopeSet::from_authorized_overlaps(*blocker, overlaps)?;
                self.add_edge(OrderingEdge {
                    edge_id: *edge_id,
                    before,
                    after,
                    scopes,
                    reason: OrderingReason::from(reason),
                    declaration_event_id: event_id,
                    declaration: EdgeDeclaration::Acquisition,
                })
            },
        }
    }

    fn add_deferral(
        &mut self,
        deferred: ReservationId,
        blocker: ReservationId,
        scopes: OrderingOverlapScopeSet,
        reason: OverlapAuthorizationReason,
        event_id: EventId,
        origin: DeferralOrigin,
    ) {
        let deferral_index = self.deferrals.len();
        self.deferrals.push(DeferredOverlap {
            declaration_event_id: event_id,
            deferred,
            blocker,
            scopes,
            reason,
            origin,
            resolved: DeferralResolution::Pending,
        });
        self.deferral_indices
            .entry((deferred, blocker))
            .or_default()
            .push(deferral_index);
    }

    fn apply_resolution(
        &mut self,
        deferred: ReservationId,
        blocker: ReservationId,
        edge_id: EdgeId,
        direction: OrderingDirection,
        reason: OrderingReason,
        event_id: EventId,
    ) -> Result<(), EdgeReplayError> {
        let matching = self
            .deferral_indices
            .get(&(deferred, blocker))
            .into_iter()
            .flatten()
            .copied()
            .filter(|index| self.deferrals[*index].resolved == DeferralResolution::Pending)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            return Err(EdgeReplayError::MissingDeferral { deferred, blocker });
        }
        let scopes = OrderingOverlapScopeSet::combine(
            matching.iter().map(|index| &self.deferrals[*index].scopes),
        )
        .map_err(|()| EdgeReplayError::MissingAuthorizedScopes(blocker))?;
        for index in matching {
            self.deferrals[index].resolved = DeferralResolution::Resolved;
        }
        let (before, after) = directed_endpoints(deferred, blocker, direction);
        self.add_edge(OrderingEdge {
            edge_id,
            before,
            after,
            scopes,
            reason,
            declaration_event_id: event_id,
            declaration: EdgeDeclaration::DeferredResolution,
        })
    }

    fn add_edge(&mut self, edge: OrderingEdge) -> Result<(), EdgeReplayError> {
        if !self.edge_ids.insert(edge.edge_id) {
            return Err(EdgeReplayError::DuplicateEdgeId(edge.edge_id));
        }
        if !self.endpoint_pairs.insert((edge.before, edge.after)) {
            return Err(EdgeReplayError::DuplicateEdge {
                before: edge.before,
                after:  edge.after,
            });
        }
        self.adjacency
            .entry(edge.before)
            .or_default()
            .push(edge.after);
        self.edges.push(edge);
        Ok(())
    }

    fn deferred_overlap_between(
        &self,
        first: ReservationId,
        second: ReservationId,
    ) -> Result<DeferredOverlapResolution, EdgeDeclarationRejection> {
        let first_orientation = self
            .deferrals
            .iter()
            .filter(|deferral| {
                deferral.deferred == first
                    && deferral.blocker == second
                    && deferral.resolved == DeferralResolution::Pending
            })
            .collect::<Vec<_>>();
        let second_orientation = self
            .deferrals
            .iter()
            .filter(|deferral| {
                deferral.deferred == second
                    && deferral.blocker == first
                    && deferral.resolved == DeferralResolution::Pending
            })
            .collect::<Vec<_>>();
        let matching = match (first_orientation.is_empty(), second_orientation.is_empty()) {
            (true, true) => return Err(EdgeDeclarationRejection::MissingDeferral),
            (false, false) => return Err(EdgeDeclarationRejection::AmbiguousDeferral),
            (false, true) => first_orientation,
            (true, false) => second_orientation,
        };
        let endpoints = DeferredOverlapEndpoints {
            deferred: matching[0].deferred,
            blocker:  matching[0].blocker,
        };
        let scopes =
            OrderingOverlapScopeSet::combine(matching.into_iter().map(|deferral| &deferral.scopes))
                .map_err(|()| EdgeDeclarationRejection::MissingDeferral)?;
        Ok(DeferredOverlapResolution { endpoints, scopes })
    }
}

/// One distinct graph predecessor and its direct successors.
pub(crate) struct GraphPredecessor<'graph> {
    /// The reservation that must be incorporated first.
    pub(crate) reservation_id: ReservationId,
    /// Every reservation directly held by this predecessor.
    pub(crate) successors:     &'graph [ReservationId],
}

/// An edge validated for one locked `ResolveDefer` append.
pub(crate) struct PreparedOrderingEdge {
    edge_id:  EdgeId,
    before:   ReservationId,
    after:    ReservationId,
    scopes:   OrderingOverlapScopeSet,
    reason:   OrderingReason,
    deferral: DeferredOverlapEndpoints,
}

impl PreparedOrderingEdge {
    /// Build the sole journal operation that turns this deferral into an edge.
    pub(crate) fn operation(&self) -> JournalOperation {
        let direction = if self.before == self.deferral.deferred {
            OrderingDirection::RequesterBeforeHolder
        } else {
            OrderingDirection::HolderBeforeRequester
        };
        JournalOperation::ResolveDefer {
            deferred_reservation_id: self.deferral.deferred,
            blocker_reservation_id: self.deferral.blocker,
            edge_id: self.edge_id,
            direction,
            reason: self.reason.clone(),
        }
    }

    /// Pair the stable edge identity with the event that committed its declaration.
    pub(crate) fn into_edge(self, declaration_event_id: EventId) -> OrderingEdge {
        OrderingEdge {
            edge_id: self.edge_id,
            before: self.before,
            after: self.after,
            scopes: self.scopes,
            reason: self.reason,
            declaration_event_id,
            declaration: EdgeDeclaration::DeferredResolution,
        }
    }
}

struct DeferredOverlapResolution {
    endpoints: DeferredOverlapEndpoints,
    scopes:    OrderingOverlapScopeSet,
}

/// A request to create an edge cannot be admitted to the current graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EdgeDeclarationRejection {
    /// At least one endpoint does not name a retained reservation.
    UnknownEndpoint(ReservationId),
    /// One reservation cannot precede itself.
    SameEndpoint,
    /// The exact directed relationship already exists.
    Duplicate,
    /// Adding the relationship would make the graph cyclic.
    Cycle,
    /// No unresolved deferral joins the two endpoints.
    MissingDeferral,
    /// Deferrals in both requester directions make the journal operation ambiguous.
    AmbiguousDeferral,
}

impl Display for EdgeDeclarationRejection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEndpoint(reservation_id) => {
                write!(formatter, "reservation {reservation_id} does not exist")
            },
            Self::SameEndpoint => formatter.write_str("an ordering edge requires two reservations"),
            Self::Duplicate => formatter.write_str("that ordering edge already exists"),
            Self::Cycle => formatter.write_str("that ordering edge would create a cycle"),
            Self::MissingDeferral => formatter.write_str(
                "sequence can only resolve an existing defer answer between these reservations",
            ),
            Self::AmbiguousDeferral => formatter.write_str(
                "both reservations recorded defer answers; the ordering resolution is ambiguous",
            ),
        }
    }
}

/// Journal facts cannot be reconstructed as a valid ordering graph.
#[derive(Debug)]
pub(crate) enum EdgeReplayError {
    /// An edge identifier appeared more than once.
    DuplicateEdgeId(EdgeId),
    /// A directed endpoint pair appeared more than once.
    DuplicateEdge {
        /// The predecessor endpoint.
        before: ReservationId,
        /// The successor endpoint.
        after:  ReservationId,
    },
    /// The complete replayed graph contains a directed cycle.
    Cycle,
    /// An authorization did not retain scopes for its named blocker.
    MissingAuthorizedScopes(ReservationId),
    /// A resolution did not match a preceding deferral.
    MissingDeferral {
        /// The reservation that recorded the defer answer.
        deferred: ReservationId,
        /// The blocker named by that answer.
        blocker:  ReservationId,
    },
    /// An edge names a reservation that no claim created.
    UnknownEndpoint(ReservationId),
}

impl Display for EdgeReplayError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateEdgeId(edge_id) => {
                write!(
                    formatter,
                    "ordering edge id {edge_id} appears more than once"
                )
            },
            Self::DuplicateEdge { before, after } => {
                write!(
                    formatter,
                    "ordering edge {before} -> {after} appears more than once"
                )
            },
            Self::Cycle => formatter.write_str("replayed ordering edges contain a cycle"),
            Self::MissingAuthorizedScopes(blocker) => write!(
                formatter,
                "ordering authorization for blocker {blocker} has no matching scopes"
            ),
            Self::MissingDeferral { deferred, blocker } => write!(
                formatter,
                "defer resolution {deferred} against {blocker} has no pending deferral"
            ),
            Self::UnknownEndpoint(reservation_id) => {
                write!(
                    formatter,
                    "ordering edge names unknown reservation {reservation_id}"
                )
            },
        }
    }
}

impl Error for EdgeReplayError {}

const fn directed_endpoints(
    requester: ReservationId,
    blocker: ReservationId,
    direction: OrderingDirection,
) -> (ReservationId, ReservationId) {
    match direction {
        OrderingDirection::RequesterBeforeHolder => (requester, blocker),
        OrderingDirection::HolderBeforeRequester => (blocker, requester),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io;

    use super::DeferralResolution;
    use super::OrderingGraph;
    use crate::answer::AuthorizedOverlap;
    use crate::answer::AuthorizedOverlapSet;
    use crate::answer::ConflictAuthorization;
    use crate::answer::OverlapScopeRevision;
    use crate::edge::DeferralOrigin;
    use crate::edge::EdgeDeclaration;
    use crate::ids::CoordinationRunId;
    use crate::ids::EventId;
    use crate::ids::RepoInstanceId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::ClaimSource;
    use crate::ledger::JournalEvent;
    use crate::ledger::JournalOperation;
    use crate::ledger::ReservationScope;
    use crate::ledger::ReservationScopeSet;
    use crate::ledger::ScopeKind;

    #[test]
    fn enrolled_claim_replay_defers_each_counterpart_once() -> Result<(), Box<dyn Error>> {
        let first = ReservationId::new();
        let second = ReservationId::new();
        let third = ReservationId::new();
        let events = [
            enrolled_claim(first, &ConflictAuthorization::NoConflict)?,
            enrolled_claim(second, &enrollment(&[first])?)?,
            enrolled_claim(third, &enrollment(&[first, second])?)?,
        ];

        let graph = OrderingGraph::replay(&events)?;

        assert_eq!(graph.edge_count(), 0);
        assert_eq!(graph.vertices.len(), 3);
        assert_eq!(graph.deferrals.len(), 3);
        for (deferred, blocker, declaration_event_id) in [
            (second, first, events[1].event_id()),
            (third, first, events[2].event_id()),
            (third, second, events[2].event_id()),
        ] {
            let matching = graph
                .deferrals
                .iter()
                .filter(|deferral| deferral.deferred == deferred && deferral.blocker == blocker)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1);
            let deferral = matching[0];
            assert_eq!(deferral.declaration_event_id, declaration_event_id);
            assert_eq!(deferral.origin, DeferralOrigin::Enrollment);
            assert_eq!(deferral.resolved, DeferralResolution::Pending);
            assert_eq!(deferral.scopes.0, shared_scopes()?);
            assert!(deferral.reason.to_string().starts_with("Enrollment"));
        }
        Ok(())
    }

    #[test]
    fn enrollment_deferral_resolves_with_either_integration_order() -> Result<(), Box<dyn Error>> {
        let counterpart = ReservationId::new();
        let enrolled = ReservationId::new();
        let other_counterpart = ReservationId::new();
        let events = [
            enrolled_claim(counterpart, &ConflictAuthorization::NoConflict)?,
            enrolled_claim(other_counterpart, &ConflictAuthorization::NoConflict)?,
            enrolled_claim(enrolled, &enrollment(&[counterpart, other_counterpart])?)?,
        ];
        for (before, after) in [(enrolled, counterpart), (counterpart, enrolled)] {
            let mut graph = OrderingGraph::replay(&events)?;
            let prepared = graph
                .prepare_deferred_edge(before, after, "land in this order".parse()?)
                .map_err(|error| io::Error::other(error.to_string()))?;
            let JournalOperation::ResolveDefer {
                deferred_reservation_id,
                blocker_reservation_id,
                edge_id,
                direction,
                reason,
            } = prepared.operation()
            else {
                return Err(io::Error::other("expected a deferral resolution").into());
            };
            let resolution_event_id = EventId::new();

            graph.apply_resolution(
                deferred_reservation_id,
                blocker_reservation_id,
                edge_id,
                direction,
                reason,
                resolution_event_id,
            )?;

            assert_eq!(graph.edge_count(), 1);
            let edge = &graph.edges[0];
            assert_eq!((edge.before, edge.after), (before, after));
            assert_eq!(edge.scopes.0, shared_scopes()?);
            assert_eq!(edge.declaration, EdgeDeclaration::DeferredResolution);
            assert_eq!(edge.declaration_event_id, resolution_event_id);
            assert_eq!(graph.deferrals[0].resolved, DeferralResolution::Resolved);
            assert_eq!(graph.deferrals[1].resolved, DeferralResolution::Pending);
            assert!(graph.deferred_overlap_between(before, after).is_err());
            assert!(
                graph
                    .deferred_overlap_between(enrolled, other_counterpart)
                    .is_ok()
            );
        }
        Ok(())
    }

    fn shared_scopes() -> Result<ReservationScopeSet, Box<dyn Error>> {
        Ok(ReservationScopeSet::try_from(vec![ReservationScope {
            path: "shared.rs".parse()?,
            kind: ScopeKind::File,
        }])?)
    }

    fn enrollment(counterparts: &[ReservationId]) -> Result<ConflictAuthorization, Box<dyn Error>> {
        let scopes = shared_scopes()?;
        let overlaps = counterparts
            .iter()
            .map(|counterpart| AuthorizedOverlap {
                reservation_id: *counterpart,
                scope_revision: OverlapScopeRevision::from(&scopes),
                scopes:         scopes.clone().into(),
            })
            .collect::<Vec<_>>();
        Ok(ConflictAuthorization::Enrollment {
            overlaps: AuthorizedOverlapSet::try_from(overlaps)?,
        })
    }

    fn enrolled_claim(
        reservation_id: ReservationId,
        authorization: &ConflictAuthorization,
    ) -> Result<JournalEvent, Box<dyn Error>> {
        Ok(serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "event_id": EventId::new(),
            "actor": {
                "repository": RepoInstanceId::new(),
                "worktree": WorktreeId::new(),
                "run": CoordinationRunId::new(),
            },
            "at": "2026-09-15T15:46:38.647Z",
            "projection_generation": 1,
            "op": "claim",
            "reservation_id": reservation_id,
            "scopes": shared_scopes()?,
            "source": ClaimSource::Enrolled,
            "purpose": { "kind": "not_provided_by_caller" },
            "trunk_at_claim": "1111111111111111111111111111111111111111",
            "head_snapshot": {
                "kind": "branch",
                "full_ref": "refs/heads/enrolled",
                "head": "1111111111111111111111111111111111111111",
            },
            "phase_start_head": "1111111111111111111111111111111111111111",
            "worktree_root": "/tmp/enrolled",
            "worktree_administrative_locator": "worktrees/enrolled",
            "authorization": authorization,
            "coordination_identity_provenance": "not_presented",
        }))?)
    }
}
