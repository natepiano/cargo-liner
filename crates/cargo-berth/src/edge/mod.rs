//! Durable ordering edges and the readiness they derive from a repository snapshot.

mod cycle;
mod graph;
mod snapshot;

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

pub(crate) use graph::EdgeDeclarationRejection;
pub(crate) use graph::EdgeReplayError;
pub(crate) use graph::OrderingGraph;
pub(crate) use graph::PreparedOrderingEdge;
use serde::Deserialize;
use serde::Serialize;
pub(crate) use snapshot::MissingReadinessFact;
pub(crate) use snapshot::PredecessorReachability;
pub(crate) use snapshot::RepositoryReservationEvidence;
pub(crate) use snapshot::RepositoryReservationSnapshot;
pub(crate) use snapshot::RepositorySnapshot;
pub(crate) use snapshot::RepositoryTrunk;
use snapshot::SnapshotReachability;
pub(crate) use snapshot::SuccessorHeadReachability;

use crate::answer::AuthorizedOverlapSet;
use crate::answer::OverlapAuthorizationReason;
use crate::ids::EdgeId;
use crate::ids::EventId;
use crate::ids::GitObjectId;
use crate::ids::ProjectionGeneration;
use crate::ids::ReservationId;
use crate::ledger::ClaimSource;
use crate::ledger::JournalActor;
use crate::ledger::ReservationPurpose;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReservationLifecycle;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;

/// Why one reservation was declared to precede another.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct OrderingReason(String);

impl FromStr for OrderingReason {
    type Err = EmptyOrderingReason;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let reason = value.trim();
        if reason.is_empty() {
            Err(EmptyOrderingReason)
        } else {
            Ok(Self(reason.to_owned()))
        }
    }
}

impl<'de> Deserialize<'de> for OrderingReason {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl Display for OrderingReason {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { formatter.write_str(&self.0) }
}

impl From<&OverlapAuthorizationReason> for OrderingReason {
    fn from(reason: &OverlapAuthorizationReason) -> Self { Self(reason.to_string()) }
}

/// A validated non-empty set of paths covered by one ordering edge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
struct OrderingOverlapScopeSet(ReservationScopeSet);

impl OrderingOverlapScopeSet {
    fn from_authorized_overlaps(
        blocker: ReservationId,
        overlaps: &AuthorizedOverlapSet,
    ) -> Result<Self, EdgeReplayError> {
        let scopes = overlaps
            .as_slice()
            .iter()
            .filter(|overlap| overlap.reservation_id == blocker)
            .flat_map(|overlap| overlap.scopes.as_slice().iter().cloned())
            .collect::<Vec<_>>();
        Self::from_scopes(scopes).map_err(|()| EdgeReplayError::MissingAuthorizedScopes(blocker))
    }

    fn combine<'scope>(scope_sets: impl Iterator<Item = &'scope Self>) -> Result<Self, ()> {
        let mut scopes = scope_sets
            .flat_map(|scope_set| scope_set.0.as_slice().iter().cloned())
            .collect::<Vec<_>>();
        scopes.sort_by(|left, right| {
            left.path
                .to_string()
                .cmp(&right.path.to_string())
                .then_with(|| left.kind.cmp(&right.kind))
        });
        scopes.dedup();
        Self::from_scopes(scopes)
    }

    fn from_scopes(scopes: Vec<ReservationScope>) -> Result<Self, ()> {
        ReservationScopeSet::try_from(scopes)
            .map(Self)
            .map_err(|_| ())
    }
}

/// Whether an edge was born with an acquisition or resolved a prior deferral.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum EdgeDeclaration {
    /// A claim or widen carried the ordering decision itself.
    Acquisition,
    /// A later `sequence` operation converted a deferral into an order.
    DeferredResolution,
}

/// One persistent ordering relationship reconstructed from journal truth.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OrderingEdge {
    /// The stable relationship identity rendered and referenced by later facts.
    pub(crate) edge_id:   EdgeId,
    /// The reservation whose protected work must be incorporated first.
    pub(crate) before:    ReservationId,
    /// The reservation held by this relationship.
    pub(crate) after:     ReservationId,
    /// The exact overlap paths that justified the order.
    scopes:               OrderingOverlapScopeSet,
    /// Why the user selected this order.
    reason:               OrderingReason,
    /// The journal fact that first recorded or resolved this edge.
    declaration_event_id: EventId,
    /// How the ordering relationship entered the journal.
    declaration:          EdgeDeclaration,
}

/// One read-only view of every integration constraint at a journal generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct IntegrationConstraintProjection {
    /// The journal generation from which every reservation and hold was rebuilt.
    pub(crate) generation:   ProjectionGeneration,
    /// The retained reservation facts required by a denial or board row.
    pub(crate) reservations: Vec<IntegrationReservationFacts>,
    /// Only relationships that currently hold at the accompanying repository snapshot.
    pub(crate) holds:        Vec<IntegrationHold>,
}

/// Reservation material shared by the trunk gate and the later board renderer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct IntegrationReservationFacts {
    /// The reservation these facts describe.
    pub(crate) reservation_id: ReservationId,
    /// The actor that acquired the reservation.
    pub(crate) actor:          JournalActor,
    /// The plan-and-phase or explicit provenance supplied at acquisition.
    pub(crate) source:         ClaimSource,
    /// The caller's explanation of the protected work.
    pub(crate) purpose:        ReservationPurpose,
    /// The complete normalized footprint, not only one edge's overlap.
    pub(crate) scopes:         ReservationScopeSet,
    /// The reservation's replayed progress state.
    pub(crate) lifecycle:      ReservationLifecycle,
    /// The commit whose newly-reachable appearance identifies this reservation.
    pub(crate) subject:        IntegrationSubject,
}

/// The commit identity available for matching a reservation to a proposed ref update.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum IntegrationSubject {
    /// This commit is the reservation's current integration subject.
    Commit { object_id: GitObjectId },
    /// The holder worktree had no resolvable current head.
    WorktreeHeadUnavailable,
    /// A terminal reservation without a checkpoint has no integration subject.
    NotApplicable,
}

/// A directed edge or symmetric deferral that currently prevents integration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum IntegrationHold {
    /// A derived ordering edge still holds its successor.
    OrderingEdge {
        /// The stable edge identity.
        edge_id:     EdgeId,
        /// The reservation that must be incorporated first.
        predecessor: ReservationId,
        /// The reservation this hold blocks.
        successor:   ReservationId,
        /// The exact approved overlap scopes that justified this edge.
        scopes:      ReservationScopeSet,
        /// Why the order was selected.
        reason:      OrderingReason,
        /// The structurally holding readiness value and its precise recovery case.
        readiness:   EdgeReadiness,
    },
    /// A defer answer holds both named endpoints until a direction is selected.
    DeferredOverlap {
        /// The claim event that first recorded this unresolved answer.
        declaration_event_id: EventId,
        /// The reservation whose claim carried the answer.
        deferred:             ReservationId,
        /// The exact counterpart named by that answer.
        blocker:              ReservationId,
        /// The exact approved overlap scopes.
        scopes:               ReservationScopeSet,
        /// Why the ordering decision was deferred.
        reason:               OverlapAuthorizationReason,
    },
}

impl IntegrationConstraintProjection {
    /// Find the retained facts for one reservation.
    pub(crate) fn reservation(
        &self,
        reservation_id: ReservationId,
    ) -> Result<&IntegrationReservationFacts, MissingReadinessFact> {
        self.reservations
            .iter()
            .find(|reservation| reservation.reservation_id == reservation_id)
            .ok_or(MissingReadinessFact::Reservation(reservation_id))
    }

    /// Iterate only holds that prevent this reservation from integrating now.
    pub(crate) fn holds_for(
        &self,
        reservation_id: ReservationId,
    ) -> impl Iterator<Item = &IntegrationHold> {
        self.holds
            .iter()
            .filter(move |hold| hold.blocks(reservation_id))
    }
}

impl IntegrationHold {
    /// Return whether this relationship currently blocks the supplied reservation.
    pub(crate) fn blocks(&self, reservation_id: ReservationId) -> bool {
        match self {
            Self::OrderingEdge { successor, .. } => *successor == reservation_id,
            Self::DeferredOverlap {
                deferred, blocker, ..
            } => *deferred == reservation_id || *blocker == reservation_id,
        }
    }
}

impl OrderingEdge {
    /// Derive this edge's current state without running git or consulting liveness.
    pub(crate) fn readiness(
        &self,
        repository_snapshot: &RepositorySnapshot,
    ) -> Result<EdgeReadiness, MissingReadinessFact> {
        let successor = repository_snapshot.reservation(self.after)?;
        match &successor.evidence {
            RepositoryReservationEvidence::Active
            | RepositoryReservationEvidence::Outstanding { .. } => {},
            RepositoryReservationEvidence::Released { disposition, .. }
            | RepositoryReservationEvidence::ReleasedWithoutCheckpoint { disposition } => {
                return Ok(match disposition {
                    ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_) => {
                        EdgeReadiness::Cancelled
                    },
                    ReleaseDisposition::Integrated
                    | ReleaseDisposition::RewrittenIntegration(_) => EdgeReadiness::Fulfilled,
                });
            },
        }
        let predecessor = repository_snapshot.reservation(self.before)?;
        match &predecessor.evidence {
            RepositoryReservationEvidence::Active => Ok(EdgeReadiness::Holding {
                hold: EdgeHold::AwaitingPredecessorCheckpoint,
            }),
            RepositoryReservationEvidence::ReleasedWithoutCheckpoint { disposition }
            | RepositoryReservationEvidence::Released { disposition, .. }
                if matches!(
                    disposition,
                    ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_)
                ) =>
            {
                Ok(EdgeReadiness::Cancelled)
            },
            RepositoryReservationEvidence::Outstanding {
                integration_status, ..
            }
            | RepositoryReservationEvidence::Released {
                integration_status, ..
            } => match integration_status {
                IntegrationEvidenceStatus::Integrated { .. } => {
                    match repository_snapshot.successor_reachability(self.before, self.after)? {
                        SnapshotReachability::Ancestor => Ok(EdgeReadiness::Fulfilled),
                        SnapshotReachability::NotAncestor | SnapshotReachability::ObjectUnknown => {
                            Ok(EdgeReadiness::Holding {
                                hold: EdgeHold::AwaitingSuccessorIncorporation,
                            })
                        },
                    }
                },
                IntegrationEvidenceStatus::NotIntegrated => Ok(EdgeReadiness::Holding {
                    hold: EdgeHold::PredecessorNotOnTrunk {
                        evidence: UnintegratedPredecessorEvidence::NotIntegrated,
                    },
                }),
                IntegrationEvidenceStatus::TrunkRewritten => Ok(EdgeReadiness::Holding {
                    hold: EdgeHold::PredecessorNotOnTrunk {
                        evidence: UnintegratedPredecessorEvidence::TrunkRewritten,
                    },
                }),
                IntegrationEvidenceStatus::ObjectUnknown => Ok(EdgeReadiness::Holding {
                    hold: EdgeHold::PredecessorNotOnTrunk {
                        evidence: UnintegratedPredecessorEvidence::ObjectUnknown,
                    },
                }),
            },
            RepositoryReservationEvidence::ReleasedWithoutCheckpoint { .. } => {
                Ok(EdgeReadiness::Cancelled)
            },
        }
    }
}

/// The derived state of one ordering edge at a repository snapshot.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum EdgeReadiness {
    /// The edge still prevents its successor from integrating.
    Holding {
        /// What the successor is waiting on.
        hold: EdgeHold,
    },
    /// A user-confirmed abandonment or orphan retirement ended the constraint.
    Cancelled,
    /// The successor's current head contains the predecessor's protected tip.
    Fulfilled,
}

/// Why an ordering edge still holds its successor back.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub(crate) enum EdgeHold {
    /// The predecessor has no protected checkpoint to be reachable from.
    AwaitingPredecessorCheckpoint,
    /// The predecessor has a protected tip that current trunk does not prove.
    PredecessorNotOnTrunk {
        /// Which unproven case applies, and therefore how it is resolved.
        evidence: UnintegratedPredecessorEvidence,
    },
    /// Trunk contains the predecessor; the successor has not incorporated it.
    AwaitingSuccessorIncorporation,
}

/// Why current trunk does not prove a predecessor's protected evidence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UnintegratedPredecessorEvidence {
    /// Trunk does not contain the protected tip yet; wait for it to land.
    NotIntegrated,
    /// A trunk rewrite invalidated the recorded evidence; re-record it with
    /// `resolve --integrated-as <trunk-oid>`.
    TrunkRewritten,
    /// A commit involved in the check does not resolve; repair the repository.
    ObjectUnknown,
}

impl EdgeReadiness {
    /// Return whether this state still prevents the successor from integrating.
    pub(crate) const fn holds_successor(self) -> bool { matches!(self, Self::Holding { .. }) }
}

/// An ordering reason cannot contain only whitespace.
#[derive(Debug)]
pub(crate) struct EmptyOrderingReason;

impl Display for EmptyOrderingReason {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("an ordering reason cannot be empty")
    }
}

impl Error for EmptyOrderingReason {}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::EdgeDeclaration;
    use super::EdgeHold;
    use super::EdgeReadiness;
    use super::OrderingEdge;
    use super::OrderingOverlapScopeSet;
    use super::OrderingReason;
    use super::RepositoryReservationEvidence;
    use super::RepositoryReservationSnapshot;
    use super::RepositorySnapshot;
    use super::RepositoryTrunk;
    use crate::ids::EdgeId;
    use crate::ids::EventId;
    use crate::ids::GitObjectId;
    use crate::ids::ReservationId;
    use crate::ids::ReservationScopePath;
    use crate::reservation::ReleaseDisposition;
    use crate::scope::ReservationScope;
    use crate::scope::ReservationScopeSet;
    use crate::scope::ScopeKind;
    use crate::worktree::WorktreeHead;
    use crate::worktree::WorktreeLiveness;

    const LIVE_HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
    const RETAINED_SCOPE_PATH: &str = "src/shared.rs";

    #[test]
    fn recovered_predecessor_edge_is_awaiting_checkpoint_until_abandoned()
    -> Result<(), Box<dyn Error>> {
        let predecessor = ReservationId::new();
        let successor = ReservationId::new();
        let live_head = LIVE_HEAD.parse::<GitObjectId>()?;
        let edge = OrderingEdge {
            edge_id:              EdgeId::new(),
            before:               predecessor,
            after:                successor,
            scopes:               OrderingOverlapScopeSet(ReservationScopeSet::try_from(vec![
                ReservationScope {
                    path: RETAINED_SCOPE_PATH.parse::<ReservationScopePath>()?,
                    kind: ScopeKind::File,
                },
            ])?),
            reason:               OrderingReason("predecessor lands first".to_owned()),
            declaration_event_id: EventId::new(),
            declaration:          EdgeDeclaration::DeferredResolution,
        };
        let snapshot_with_predecessor_evidence = |evidence| {
            RepositorySnapshot::new(
                RepositoryTrunk::ObjectUnknown,
                vec![
                    RepositoryReservationSnapshot {
                        reservation_id: predecessor,
                        worktree_liveness: WorktreeLiveness::Live,
                        worktree_head: WorktreeHead::Resolved(live_head.clone()),
                        evidence,
                    },
                    RepositoryReservationSnapshot {
                        reservation_id:    successor,
                        worktree_liveness: WorktreeLiveness::Live,
                        worktree_head:     WorktreeHead::Resolved(live_head.clone()),
                        evidence:          RepositoryReservationEvidence::Active,
                    },
                ],
                Vec::new(),
            )
        };

        let recovered_readiness = edge.readiness(&snapshot_with_predecessor_evidence(
            RepositoryReservationEvidence::Active,
        ))?;
        assert_eq!(
            recovered_readiness,
            EdgeReadiness::Holding {
                hold: EdgeHold::AwaitingPredecessorCheckpoint,
            }
        );
        assert!(recovered_readiness.holds_successor());

        let abandoned_readiness = edge.readiness(&snapshot_with_predecessor_evidence(
            RepositoryReservationEvidence::ReleasedWithoutCheckpoint {
                disposition: ReleaseDisposition::Abandoned(
                    "predecessor work was discarded".parse()?,
                ),
            },
        ))?;
        assert_eq!(abandoned_readiness, EdgeReadiness::Cancelled);
        assert!(!abandoned_readiness.holds_successor());
        Ok(())
    }
}
