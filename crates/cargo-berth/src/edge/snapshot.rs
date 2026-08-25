//! One repository observation and the reachability facts edge readiness consumes.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::worktree::WorktreeHead;
use crate::worktree::WorktreeLiveness;

/// Whether the configured trunk resolved during repository observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RepositoryTrunk {
    /// The configured branch resolved to this commit.
    Resolved(GitObjectId),
    /// Git could not resolve the configured branch.
    ObjectUnknown,
}

/// Lifecycle and git evidence captured for one retained reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RepositoryReservationEvidence {
    /// Active work has no protected integration subject.
    Active,
    /// A protected checkpoint has not received a terminal disposition.
    Outstanding {
        /// The commit whose reachability controls the edge.
        protected_tip:      ProtectedReservationTip,
        /// What the one current trunk observation proves.
        integration_status: IntegrationEvidenceStatus,
    },
    /// A disposition exists and retains a protected checkpoint.
    Released {
        /// The commit whose reachability controls the edge.
        protected_tip:      ProtectedReservationTip,
        /// The recorded user or git-backed disposition.
        disposition:        ReleaseDisposition,
        /// What the one current trunk observation proves now.
        integration_status: IntegrationEvidenceStatus,
    },
    /// A user-confirmed retirement occurred before any checkpoint.
    ReleasedWithoutCheckpoint {
        /// The confirmed terminal decision.
        disposition: ReleaseDisposition,
    },
}

/// Repository facts for one retained reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RepositoryReservationSnapshot {
    /// The reservation these point-in-time facts describe.
    pub(crate) reservation_id:    ReservationId,
    /// The holder observation kept separate from edge readiness.
    pub(crate) worktree_liveness: WorktreeLiveness,
    /// The holder commit reported by the one worktree-list snapshot.
    pub(crate) worktree_head:     WorktreeHead,
    /// Current lifecycle-specific integration evidence.
    pub(crate) evidence:          RepositoryReservationEvidence,
}

/// Whether one successor head contains its predecessor's protected tip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SuccessorHeadReachability {
    /// The successor head contains the predecessor tip.
    ContainsPredecessor,
    /// The successor head resolves without containing the predecessor tip.
    DoesNotContainPredecessor,
    /// This successor head does not resolve as a commit.
    ObjectUnknown,
}

/// Reachability results for one protected graph predecessor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PredecessorReachability {
    /// Every resolvable successor head received its independent result.
    Classified(HashMap<GitObjectId, SuccessorHeadReachability>),
    /// The predecessor's protected tip does not resolve as a commit.
    ObjectUnknown,
    /// Git could not complete the grouped ancestry query.
    QueryFailed,
}

/// One complete repository observation used for every edge-readiness decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RepositorySnapshot {
    trunk:                    RepositoryTrunk,
    reservations:             HashMap<ReservationId, RepositoryReservationSnapshot>,
    predecessor_reachability: HashMap<ReservationId, PredecessorReachability>,
}

impl RepositorySnapshot {
    /// Assemble one repository observation from its complete typed facts.
    pub(crate) fn new(
        trunk: RepositoryTrunk,
        reservations: Vec<RepositoryReservationSnapshot>,
        predecessor_reachability: Vec<(ReservationId, PredecessorReachability)>,
    ) -> Self {
        Self {
            trunk,
            reservations: reservations
                .into_iter()
                .map(|snapshot| (snapshot.reservation_id, snapshot))
                .collect(),
            predecessor_reachability: predecessor_reachability.into_iter().collect(),
        }
    }

    pub(crate) fn reservation(
        &self,
        reservation_id: ReservationId,
    ) -> Result<&RepositoryReservationSnapshot, MissingReadinessFact> {
        self.reservations
            .get(&reservation_id)
            .ok_or(MissingReadinessFact::Reservation(reservation_id))
    }

    /// Borrow the one resolved-or-unknown trunk observation used for this snapshot.
    pub(crate) const fn trunk(&self) -> &RepositoryTrunk { &self.trunk }

    /// Iterate the one grouped reachability result recorded per graph predecessor.
    pub(crate) fn predecessor_reachability(
        &self,
    ) -> impl Iterator<Item = (&ReservationId, &PredecessorReachability)> {
        self.predecessor_reachability.iter()
    }

    pub(super) fn successor_reachability(
        &self,
        predecessor: ReservationId,
        successor: ReservationId,
    ) -> Result<SnapshotReachability, MissingReadinessFact> {
        let successor = self.reservation(successor)?;
        let WorktreeHead::Resolved(successor_head) = &successor.worktree_head else {
            return Ok(SnapshotReachability::ObjectUnknown);
        };
        let predecessor_reachability = self
            .predecessor_reachability
            .get(&predecessor)
            .ok_or(MissingReadinessFact::PredecessorReachability(predecessor))?;
        match predecessor_reachability {
            PredecessorReachability::Classified(successor_heads) => {
                match successor_heads.get(successor_head).ok_or(
                    MissingReadinessFact::SuccessorReachability {
                        predecessor,
                        successor: successor.reservation_id,
                    },
                )? {
                    SuccessorHeadReachability::ContainsPredecessor => {
                        Ok(SnapshotReachability::Ancestor)
                    },
                    SuccessorHeadReachability::DoesNotContainPredecessor => {
                        Ok(SnapshotReachability::NotAncestor)
                    },
                    SuccessorHeadReachability::ObjectUnknown => {
                        Ok(SnapshotReachability::ObjectUnknown)
                    },
                }
            },
            PredecessorReachability::ObjectUnknown | PredecessorReachability::QueryFailed => {
                Ok(SnapshotReachability::ObjectUnknown)
            },
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum SnapshotReachability {
    Ancestor,
    NotAncestor,
    ObjectUnknown,
}

/// A repository snapshot lacks a fact required to classify an edge.
#[derive(Debug)]
pub(crate) enum MissingReadinessFact {
    /// The snapshot contains no entry for this reservation.
    Reservation(ReservationId),
    /// The snapshot contains no ancestry result for this protected predecessor.
    PredecessorReachability(ReservationId),
    /// A classified predecessor omitted one of its direct successor heads.
    SuccessorReachability {
        /// The protected predecessor whose classification omitted a head.
        predecessor: ReservationId,
        /// The direct successor whose head received no result.
        successor:   ReservationId,
    },
}

impl Display for MissingReadinessFact {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reservation(reservation_id) => write!(
                formatter,
                "repository snapshot has no reservation {reservation_id}"
            ),
            Self::PredecessorReachability(reservation_id) => write!(
                formatter,
                "repository snapshot has no protected-tip reachability for {reservation_id}"
            ),
            Self::SuccessorReachability {
                predecessor,
                successor,
            } => write!(
                formatter,
                "repository snapshot has no reachability from predecessor {predecessor} to successor {successor}"
            ),
        }
    }
}

impl Error for MissingReadinessFact {}
