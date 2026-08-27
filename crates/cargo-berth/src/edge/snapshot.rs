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

/// What proves whether one successor head has incorporated its predecessor's protected work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SuccessorIncorporationEvidence {
    /// The successor head contains the predecessor's protected tip as an ancestor.
    ProtectedTipAncestor,
    /// The successor head contains equivalent rewritten protected content.
    ScopedPatchEquivalent,
    /// Neither ancestry nor scoped content proves incorporation.
    NotIncorporated,
    /// A required object or comparison result was unavailable.
    ObjectUnknown,
}

/// Successor-incorporation results for one protected graph predecessor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PredecessorSuccessorIncorporation {
    /// Every resolvable successor head received its independent result.
    Classified(HashMap<GitObjectId, SuccessorIncorporationEvidence>),
    /// The predecessor's protected tip does not resolve as a commit.
    PredecessorObjectUnknown,
    /// Git could not complete the grouped ancestry query.
    QueryFailed,
}

/// One complete repository observation used for every edge-readiness decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RepositorySnapshot {
    trunk:                   RepositoryTrunk,
    reservations:            HashMap<ReservationId, RepositoryReservationSnapshot>,
    successor_incorporation: HashMap<ReservationId, PredecessorSuccessorIncorporation>,
}

impl RepositorySnapshot {
    /// Assemble one repository observation from its complete typed facts.
    pub(crate) fn new(
        trunk: RepositoryTrunk,
        reservations: Vec<RepositoryReservationSnapshot>,
        successor_incorporation: Vec<(ReservationId, PredecessorSuccessorIncorporation)>,
    ) -> Self {
        Self {
            trunk,
            reservations: reservations
                .into_iter()
                .map(|snapshot| (snapshot.reservation_id, snapshot))
                .collect(),
            successor_incorporation: successor_incorporation.into_iter().collect(),
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

    pub(super) fn successor_incorporation_evidence(
        &self,
        predecessor: ReservationId,
        successor: ReservationId,
    ) -> Result<SuccessorIncorporationEvidence, MissingReadinessFact> {
        let successor = self.reservation(successor)?;
        let WorktreeHead::Resolved(successor_head) = &successor.worktree_head else {
            return Ok(SuccessorIncorporationEvidence::ObjectUnknown);
        };
        let predecessor_incorporation = self
            .successor_incorporation
            .get(&predecessor)
            .ok_or(MissingReadinessFact::PredecessorIncorporation(predecessor))?;
        match predecessor_incorporation {
            PredecessorSuccessorIncorporation::Classified(successor_heads) => successor_heads
                .get(successor_head)
                .copied()
                .ok_or(MissingReadinessFact::SuccessorIncorporation {
                    predecessor,
                    successor: successor.reservation_id,
                }),
            PredecessorSuccessorIncorporation::PredecessorObjectUnknown
            | PredecessorSuccessorIncorporation::QueryFailed => {
                Ok(SuccessorIncorporationEvidence::ObjectUnknown)
            },
        }
    }
}

/// A repository snapshot lacks a fact required to classify an edge.
#[derive(Debug)]
pub(crate) enum MissingReadinessFact {
    /// The snapshot contains no entry for this reservation.
    Reservation(ReservationId),
    /// The snapshot contains no incorporation result for this protected predecessor.
    PredecessorIncorporation(ReservationId),
    /// A classified predecessor omitted one of its direct successor heads.
    SuccessorIncorporation {
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
            Self::PredecessorIncorporation(reservation_id) => write!(
                formatter,
                "repository snapshot has no successor-incorporation evidence for {reservation_id}"
            ),
            Self::SuccessorIncorporation {
                predecessor,
                successor,
            } => write!(
                formatter,
                "repository snapshot has no incorporation evidence from predecessor {predecessor} to successor {successor}"
            ),
        }
    }
}

impl Error for MissingReadinessFact {}
