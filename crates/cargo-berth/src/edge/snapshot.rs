//! One repository observation and the reachability facts edge readiness consumes.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ledger::IntegrationTarget;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::worktree::WorktreeHead;
use crate::worktree::WorktreeLiveness;

/// Commit availability at the branch where a reservation is judged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum JudgedTargetTip {
    /// The judging branch resolved to this commit.
    Resolved(GitObjectId),
    /// Git could not resolve the judging branch.
    ObjectUnknown,
}

impl JudgedTargetTip {
    /// Read the repository trunk's tip from its branch observation.
    pub(crate) fn repository_trunk(
        targets: &BTreeMap<IntegrationTarget, TargetObservation>,
        trunk: &IntegrationTarget,
    ) -> Self {
        match targets.get(trunk) {
            Some(TargetObservation::Resolved(commit)) => Self::Resolved(commit.clone()),
            Some(TargetObservation::Missing | TargetObservation::ObjectUnknown) | None => {
                Self::ObjectUnknown
            },
        }
    }
}

/// The branch identity that judges a reservation's integration evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EdgeJudgingTarget {
    /// A resolved recorded branch, or the trunk replacing a missing branch.
    Branch(IntegrationTarget),
    /// The recorded branch could not be classified.
    Unavailable,
}

/// The branch an ordering edge requires, with its relationship to the repository trunk.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum EdgeOrderingTarget {
    RepositoryTrunk(IntegrationTarget),
    SharedTarget(IntegrationTarget),
    CrossTarget {
        predecessor: IntegrationTarget,
        trunk:       IntegrationTarget,
    },
    #[default]
    Unavailable,
}

impl EdgeOrderingTarget {
    pub(crate) fn branch_name(&self) -> &str {
        match self {
            Self::RepositoryTrunk(target) | Self::SharedTarget(target) => target.short_name(),
            Self::CrossTarget { trunk, .. } => trunk.short_name(),
            Self::Unavailable => "ordering target",
        }
    }

    pub(crate) fn wait_name(&self) -> &str {
        match self {
            Self::RepositoryTrunk(_) | Self::CrossTarget { .. } => "trunk",
            Self::SharedTarget(target) => target.short_name(),
            Self::Unavailable => "ordering target",
        }
    }
}

/// Whether a protected predecessor or its integration proof has reached the repository trunk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CrossTargetPredecessorReachability {
    /// The predecessor's work is reachable at this observed trunk commit.
    OnTrunk(GitObjectId),
    /// Neither protected evidence commit is reachable from the trunk.
    NotOnTrunk,
    /// A required commit or the trunk observation could not be classified.
    ObjectUnknown,
}

/// Trunk-level evidence for graph predecessors, keyed by reservation identity.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CrossTargetPredecessorEvidence(
    HashMap<ReservationId, CrossTargetPredecessorReachability>,
);

impl CrossTargetPredecessorEvidence {
    pub(crate) const fn new(
        evidence: HashMap<ReservationId, CrossTargetPredecessorReachability>,
    ) -> Self {
        Self(evidence)
    }

    fn for_predecessor(
        &self,
        reservation_id: ReservationId,
    ) -> Result<&CrossTargetPredecessorReachability, MissingReadinessFact> {
        self.0
            .get(&reservation_id)
            .ok_or(MissingReadinessFact::CrossTargetPredecessor(reservation_id))
    }
}

/// The direct observation of one recorded integration branch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetObservation {
    /// The branch resolves to a commit.
    Resolved(GitObjectId),
    /// The branch ref is absent; its reservations are judged at the repository trunk.
    Missing,
    /// Git could not classify the branch.
    ObjectUnknown,
}

impl TargetObservation {
    /// The commit shown on the board for this branch.
    pub(crate) fn commit(&self) -> String {
        match self {
            Self::Resolved(commit) => commit.to_string(),
            Self::Missing | Self::ObjectUnknown => "unresolved".to_owned(),
        }
    }
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
    /// The successor head contains the trunk commit the predecessor's integration proof names.
    ///
    /// The predecessor's work reached trunk there, so a successor holding that commit holds the
    /// work -- whatever became of the protected tip, which an amending push rewrites away on every
    /// push while the commit it landed as stays in trunk forever.
    IntegratedTrunkAncestor,
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
    repository_trunk:             IntegrationTarget,
    targets:                      BTreeMap<IntegrationTarget, TargetObservation>,
    judged_targets:               BTreeMap<IntegrationTarget, JudgedTargetTip>,
    repository_trunk_observation: JudgedTargetTip,
    reservation_targets:          HashMap<ReservationId, IntegrationTarget>,
    reservations:                 HashMap<ReservationId, RepositoryReservationSnapshot>,
    successor_incorporation:      HashMap<ReservationId, PredecessorSuccessorIncorporation>,
    cross_target_predecessors:    CrossTargetPredecessorEvidence,
}

impl RepositorySnapshot {
    /// Assemble one repository observation from its complete typed facts.
    pub(crate) fn new(
        repository_trunk: IntegrationTarget,
        targets: BTreeMap<IntegrationTarget, TargetObservation>,
        reservation_targets: HashMap<ReservationId, IntegrationTarget>,
        reservations: Vec<RepositoryReservationSnapshot>,
        successor_incorporation: Vec<(ReservationId, PredecessorSuccessorIncorporation)>,
        cross_target_predecessors: CrossTargetPredecessorEvidence,
    ) -> Self {
        let trunk_observation = JudgedTargetTip::repository_trunk(&targets, &repository_trunk);
        let judged_targets = targets
            .iter()
            .map(|(target, observation)| {
                let judged = match observation {
                    TargetObservation::Resolved(commit) => {
                        JudgedTargetTip::Resolved(commit.clone())
                    },
                    TargetObservation::Missing if target != &repository_trunk => {
                        trunk_observation.clone()
                    },
                    TargetObservation::Missing | TargetObservation::ObjectUnknown => {
                        JudgedTargetTip::ObjectUnknown
                    },
                };
                (target.clone(), judged)
            })
            .collect();
        Self {
            repository_trunk,
            targets,
            judged_targets,
            repository_trunk_observation: trunk_observation,
            reservation_targets,
            reservations: reservations
                .into_iter()
                .map(|snapshot| (snapshot.reservation_id, snapshot))
                .collect(),
            successor_incorporation: successor_incorporation.into_iter().collect(),
            cross_target_predecessors,
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

    /// Borrow the configured repository trunk branch.
    pub(crate) const fn repository_trunk_target(&self) -> &IntegrationTarget {
        &self.repository_trunk
    }

    /// Borrow every recorded branch observation, including the repository trunk.
    pub(crate) const fn targets(&self) -> &BTreeMap<IntegrationTarget, TargetObservation> {
        &self.targets
    }

    /// Borrow a reservation's recorded target branch.
    pub(crate) fn recorded_target(&self, reservation_id: ReservationId) -> &IntegrationTarget {
        self.reservation_targets
            .get(&reservation_id)
            .unwrap_or(&self.repository_trunk)
    }

    /// Borrow the observation at which this reservation is judged.
    pub(crate) fn target_for(&self, reservation_id: ReservationId) -> &JudgedTargetTip {
        let target = self.recorded_target(reservation_id);
        self.judged_targets
            .get(target)
            .unwrap_or_else(|| self.repository_trunk())
    }

    /// Borrow the direct repository trunk observation for trunk-level rules.
    pub(crate) const fn repository_trunk(&self) -> &JudgedTargetTip {
        &self.repository_trunk_observation
    }

    /// Return the branch identity used to judge this reservation, independent of its tip.
    pub(crate) fn judging_target(&self, reservation_id: ReservationId) -> EdgeJudgingTarget {
        let target = self.recorded_target(reservation_id);
        match self.targets.get(target) {
            Some(TargetObservation::Resolved(_)) => EdgeJudgingTarget::Branch(target.clone()),
            Some(TargetObservation::Missing) => {
                EdgeJudgingTarget::Branch(self.repository_trunk.clone())
            },
            Some(TargetObservation::ObjectUnknown) | None => EdgeJudgingTarget::Unavailable,
        }
    }

    pub(super) fn cross_target_predecessor(
        &self,
        reservation_id: ReservationId,
    ) -> Result<&CrossTargetPredecessorReachability, MissingReadinessFact> {
        self.cross_target_predecessors
            .for_predecessor(reservation_id)
    }

    /// Retain actual trunk reachability while a non-trunk target is proposed.
    pub(crate) const fn cross_target_predecessor_evidence(
        &self,
    ) -> &CrossTargetPredecessorEvidence {
        &self.cross_target_predecessors
    }

    /// Carry all target facts into a successor snapshot.
    pub(crate) fn target_facts(
        &self,
    ) -> (
        IntegrationTarget,
        BTreeMap<IntegrationTarget, TargetObservation>,
        HashMap<ReservationId, IntegrationTarget>,
    ) {
        (
            self.repository_trunk.clone(),
            self.targets.clone(),
            self.reservation_targets.clone(),
        )
    }

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
    /// The snapshot lacks trunk reachability for a cross-target predecessor.
    CrossTargetPredecessor(ReservationId),
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
            Self::CrossTargetPredecessor(reservation_id) => write!(
                formatter,
                "repository snapshot has no trunk evidence for predecessor {reservation_id}"
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::HashMap;
    use std::error::Error;

    use super::CrossTargetPredecessorEvidence;
    use super::EdgeJudgingTarget;
    use super::RepositorySnapshot;
    use super::TargetObservation;
    use crate::ids::GitObjectId;
    use crate::ids::ReservationId;
    use crate::ledger::IntegrationTarget;

    #[test]
    fn judging_target_uses_branch_identity_and_missing_target_fallback()
    -> Result<(), Box<dyn Error>> {
        let trunk = IntegrationTarget::from_branch_argument("main")?;
        let first = IntegrationTarget::from_branch_argument("integration-first")?;
        let second = IntegrationTarget::from_branch_argument("integration-second")?;
        let missing = IntegrationTarget::from_branch_argument("deleted")?;
        let unknown = IntegrationTarget::from_branch_argument("unreadable")?;
        let shared_tip: GitObjectId = "0123456789abcdef0123456789abcdef01234567".parse()?;
        let trunk_id = ReservationId::new();
        let first_id = ReservationId::new();
        let second_id = ReservationId::new();
        let missing_id = ReservationId::new();
        let unknown_id = ReservationId::new();
        let snapshot = RepositorySnapshot::new(
            trunk.clone(),
            BTreeMap::from([
                (
                    trunk.clone(),
                    TargetObservation::Resolved(shared_tip.clone()),
                ),
                (
                    first.clone(),
                    TargetObservation::Resolved(shared_tip.clone()),
                ),
                (second.clone(), TargetObservation::Resolved(shared_tip)),
                (missing.clone(), TargetObservation::Missing),
                (unknown.clone(), TargetObservation::ObjectUnknown),
            ]),
            HashMap::from([
                (trunk_id, trunk.clone()),
                (first_id, first.clone()),
                (second_id, second.clone()),
                (missing_id, missing),
                (unknown_id, unknown),
            ]),
            Vec::new(),
            Vec::new(),
            CrossTargetPredecessorEvidence::default(),
        );
        assert_eq!(
            snapshot.judging_target(trunk_id),
            EdgeJudgingTarget::Branch(trunk.clone())
        );
        assert_eq!(
            snapshot.judging_target(missing_id),
            EdgeJudgingTarget::Branch(trunk)
        );
        assert_eq!(
            snapshot.judging_target(unknown_id),
            EdgeJudgingTarget::Unavailable
        );
        assert_eq!(
            snapshot.judging_target(first_id),
            EdgeJudgingTarget::Branch(first)
        );
        assert_eq!(
            snapshot.judging_target(second_id),
            EdgeJudgingTarget::Branch(second)
        );
        assert_ne!(
            snapshot.judging_target(first_id),
            snapshot.judging_target(second_id)
        );
        Ok(())
    }
}
