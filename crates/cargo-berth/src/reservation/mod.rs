//! Reservation state derived solely from append-only journal events.

mod constants;
mod evidence;
mod lifecycle;

use std::collections::VecDeque;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::time::Duration;

pub(crate) use evidence::DeferredScopedPatchIntegrationStatus;
pub(crate) use evidence::IntegrationEvidenceObservation;
pub(crate) use evidence::PriorIntegrationStatus;
pub(crate) use evidence::ProtectedReservationTip;
pub(crate) use evidence::ScopedPatchComparisonObservation;
pub(crate) use evidence::current_head;
pub(crate) use evidence::current_trunk;
pub(crate) use evidence::integration_status;
pub(crate) use evidence::observe_integration_status;
pub(crate) use evidence::observe_outstanding_integration_status;
pub(crate) use evidence::outstanding_integration_status;
pub(crate) use evidence::retain_protected_tip;
pub(crate) use lifecycle::AbandonmentReason;
pub(crate) use lifecycle::EditBlockingStatus;
pub(crate) use lifecycle::IntegrationEvidenceStatus;
pub(crate) use lifecycle::IntegrationProof;
pub(crate) use lifecycle::LifecycleTransitionError;
pub(crate) use lifecycle::OrphanRetirementReason;
pub(crate) use lifecycle::ReleaseDisposition;
pub(crate) use lifecycle::ReleaseRevalidationSubject;
pub(crate) use lifecycle::ReservationLifecycle;
pub(crate) use lifecycle::RewrittenIntegrationTrunkCommit;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use self::constants::SCOPED_PATCH_TARGET_RETENTION_LIMIT;
use self::constants::SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT;
use crate::answer::AuthorizedOverlap;
use crate::answer::AuthorizedOverlapSet;
use crate::answer::ConflictAuthorization;
use crate::answer::OverlapScopeRevision;
use crate::ids::CoordinationRunId;
use crate::ids::EventId;
use crate::ids::GitObjectId;
use crate::ids::ProjectionGeneration;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::ReservationRevision;
use crate::ids::ReservationScopePath;
use crate::ids::WorktreeId;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::EditAuthorization;
use crate::ledger::ForeignReservationIdSet;
use crate::ledger::IncursionIncidentId;
use crate::ledger::IncursionPathSet;
use crate::ledger::JournalActor;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReservationPurpose;
use crate::ledger::ReservationScopeAdditionSet;
use crate::ledger::ReservationSnapshot;
use crate::ledger::TrunkObservationAtClaim;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::scope::PathCase;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;

/// The version of the baseline, protected content, and scopes used by a scoped proof.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct IntegrationProofSubjectRevision(u64);

impl IntegrationProofSubjectRevision {
    const INITIAL: Self = Self(1);
}

declare_wire_enum! {
    /// A definitive content verdict produced by scoped patch equivalence.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum ScopedPatchEquivalenceVerdict {
        /// The target contains the protected scoped change.
        Integrated => "integrated";
        /// The target does not contain an outstanding protected scoped change.
        NotIntegrated => "not_integrated";
        /// The target no longer contains a previously integrated scoped change.
        TrunkRewritten => "trunk_rewritten";
    }
}

/// An immutable scoped patch result that can be reused under a later integration context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DurableScopedPatchComparison {
    /// The target contains the protected scoped change.
    Equivalent,
    /// The target does not contain the protected scoped change.
    Different,
}

impl From<ScopedPatchEquivalenceVerdict> for DurableScopedPatchComparison {
    fn from(verdict: ScopedPatchEquivalenceVerdict) -> Self {
        match verdict {
            ScopedPatchEquivalenceVerdict::Integrated => Self::Equivalent,
            ScopedPatchEquivalenceVerdict::NotIntegrated
            | ScopedPatchEquivalenceVerdict::TrunkRewritten => Self::Different,
        }
    }
}

/// One definitive scoped patch verdict retained for an immutable target.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RetainedScopedPatchTargetVerdict {
    subject: IntegrationProofSubjectRevision,
    target:  GitObjectId,
    verdict: ScopedPatchEquivalenceVerdict,
}

/// Durable scoped patch verdicts retained for the most recently recorded reconciliation targets.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RetainedScopedPatchTargetVerdicts {
    entries: VecDeque<RetainedScopedPatchTargetVerdict>,
}

/// Whether a retained scoped patch verdict applies to one requested subject and target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScopedPatchTargetVerdictAvailability {
    /// The stored subject and target match the request.
    Hit(DurableScopedPatchComparison),
    /// No stored verdict applies to the request.
    Miss,
}

declare_wire_enum! {
    /// A definitive successor-incorporation verdict produced by scoped patch equivalence.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum SuccessorScopedPatchEquivalenceVerdict {
        /// The successor head contains the predecessor's protected scoped change.
        Equivalent => "equivalent";
        /// The successor head does not contain the predecessor's protected scoped change.
        Different => "different";
    }
}

/// One definitive successor-incorporation verdict retained for an immutable head.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RetainedSuccessorScopedPatchTargetVerdict {
    subject:        IntegrationProofSubjectRevision,
    successor_head: GitObjectId,
    verdict:        SuccessorScopedPatchEquivalenceVerdict,
}

/// Durable scoped patch verdicts retained for recently observed successor heads.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RetainedSuccessorScopedPatchTargetVerdicts {
    entries: VecDeque<RetainedSuccessorScopedPatchTargetVerdict>,
}

/// Whether a retained successor-incorporation verdict applies to one proof subject and head.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SuccessorScopedPatchTargetVerdictAvailability {
    /// The stored proof subject and successor head match the request.
    Hit(SuccessorScopedPatchEquivalenceVerdict),
    /// No stored successor-incorporation verdict applies to the request.
    Miss,
}

/// The scheduling order for scoped comparisons without a retained verdict at one trunk target.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ScopedPatchEvaluationPriority {
    /// This proof subject has not been compared with the target.
    NotAttempted,
    /// This generation last compared the proof subject with the target.
    LastAttemptedAt(ProjectionGeneration),
}

/// One comparison attempt retained for target-specific round-robin scheduling.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ScopedPatchComparisonAttempt {
    subject:    IntegrationProofSubjectRevision,
    target:     GitObjectId,
    generation: ProjectionGeneration,
}

/// The bounded evaluation schedule for the most recently recorded reconciliation targets.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ScopedPatchTargetEvaluationSchedule {
    entries: VecDeque<ScopedPatchComparisonAttempt>,
}

impl ScopedPatchTargetEvaluationSchedule {
    fn priority(
        &self,
        subject: IntegrationProofSubjectRevision,
        target: &GitObjectId,
    ) -> ScopedPatchEvaluationPriority {
        for attempt in &self.entries {
            if attempt.subject == subject && attempt.target == *target {
                return ScopedPatchEvaluationPriority::LastAttemptedAt(attempt.generation);
            }
        }
        ScopedPatchEvaluationPriority::NotAttempted
    }

    fn record(
        &mut self,
        subject: IntegrationProofSubjectRevision,
        target: &GitObjectId,
        generation: ProjectionGeneration,
    ) {
        self.entries
            .retain(|attempt| attempt.subject != subject || attempt.target != *target);
        if self.entries.len() == SCOPED_PATCH_TARGET_RETENTION_LIMIT {
            std::mem::drop(self.entries.pop_front());
        }
        self.entries.push_back(ScopedPatchComparisonAttempt {
            subject,
            target: target.clone(),
            generation,
        });
    }
}

/// Attempt generations for recent successor heads under the current proof subject.
///
/// The retention limit matches the retained successor verdict limit, so an unvisited retained
/// head sorts ahead of retried transient failures. Recording a new proof subject removes every
/// superseded subject before applying that limit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SuccessorScopedPatchTargetEvaluationSchedule {
    entries: VecDeque<ScopedPatchComparisonAttempt>,
}

impl SuccessorScopedPatchTargetEvaluationSchedule {
    fn priority(
        &self,
        subject: IntegrationProofSubjectRevision,
        successor_head: &GitObjectId,
    ) -> ScopedPatchEvaluationPriority {
        for attempt in &self.entries {
            if attempt.subject == subject && attempt.target == *successor_head {
                return ScopedPatchEvaluationPriority::LastAttemptedAt(attempt.generation);
            }
        }
        ScopedPatchEvaluationPriority::NotAttempted
    }

    fn record(
        &mut self,
        subject: IntegrationProofSubjectRevision,
        successor_head: &GitObjectId,
        generation: ProjectionGeneration,
    ) {
        self.entries
            .retain(|attempt| attempt.subject == subject && attempt.target != *successor_head);
        if self.entries.len() == SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT {
            std::mem::drop(self.entries.pop_front());
        }
        self.entries.push_back(ScopedPatchComparisonAttempt {
            subject,
            target: successor_head.clone(),
            generation,
        });
    }
}

impl RetainedScopedPatchTargetVerdicts {
    /// Look up a verdict only when both immutable proof inputs match.
    pub(crate) fn lookup(
        &self,
        subject: IntegrationProofSubjectRevision,
        target: &GitObjectId,
    ) -> ScopedPatchTargetVerdictAvailability {
        for entry in &self.entries {
            if entry.subject == subject && entry.target == *target {
                return ScopedPatchTargetVerdictAvailability::Hit(entry.verdict.into());
            }
        }
        ScopedPatchTargetVerdictAvailability::Miss
    }

    fn record(
        &mut self,
        subject: IntegrationProofSubjectRevision,
        target: &GitObjectId,
        verdict: ScopedPatchEquivalenceVerdict,
    ) {
        self.entries
            .retain(|entry| entry.subject != subject || entry.target != *target);
        if self.entries.len() == SCOPED_PATCH_TARGET_RETENTION_LIMIT {
            std::mem::drop(self.entries.pop_front());
        }
        self.entries.push_back(RetainedScopedPatchTargetVerdict {
            subject,
            target: target.clone(),
            verdict,
        });
    }
}

impl RetainedSuccessorScopedPatchTargetVerdicts {
    /// Look up a verdict only when both immutable successor-proof inputs match.
    pub(crate) fn lookup(
        &self,
        subject: IntegrationProofSubjectRevision,
        successor_head: &GitObjectId,
    ) -> SuccessorScopedPatchTargetVerdictAvailability {
        for entry in &self.entries {
            if entry.subject == subject && entry.successor_head == *successor_head {
                return SuccessorScopedPatchTargetVerdictAvailability::Hit(entry.verdict);
            }
        }
        SuccessorScopedPatchTargetVerdictAvailability::Miss
    }

    fn record(
        &mut self,
        subject: IntegrationProofSubjectRevision,
        successor_head: &GitObjectId,
        verdict: SuccessorScopedPatchEquivalenceVerdict,
    ) {
        self.entries
            .retain(|entry| entry.subject != subject || entry.successor_head != *successor_head);
        if self.entries.len() == SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT {
            std::mem::drop(self.entries.pop_front());
        }
        self.entries
            .push_back(RetainedSuccessorScopedPatchTargetVerdict {
                subject,
                successor_head: successor_head.clone(),
                verdict,
            });
    }
}

/// Every retained reservation after replaying the journal in append order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RetainedReservationSet {
    reservations:        Vec<Reservation>,
    incursion_incidents: Vec<IncursionIncident>,
}

/// One reservation retained for overlap, evidence, and audit decisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Reservation {
    id:                                                ReservationId,
    revision:                                          ReservationRevision,
    integration_proof_subject:                         IntegrationProofSubjectRevision,
    retained_scoped_patch_target_verdicts:             RetainedScopedPatchTargetVerdicts,
    scoped_patch_target_evaluation_schedule:           ScopedPatchTargetEvaluationSchedule,
    retained_successor_scoped_patch_verdicts:          RetainedSuccessorScopedPatchTargetVerdicts,
    successor_scoped_patch_target_evaluation_schedule: SuccessorScopedPatchTargetEvaluationSchedule,
    scopes:                                            ReservationScopeSet,
    authorizations:                                    Vec<ConflictAuthorization>,
    source:                                            ClaimSource,
    purpose:                                           ReservationPurpose,
    head_snapshot:                                     ClaimHeadSnapshot,
    phase_start_head:                                  ProtectedPhaseStartHead,
    actor:                                             JournalActor,
    lifecycle:                                         ReservationLifecycle,
    retained_protected_tip:                            RetainedProtectedTip,
    integration_trunk_snapshot:                        IntegrationTrunkSnapshot,
    integration_status:                                IntegrationEvidenceStatus,
    worktree_root:                                     CanonicalWorktreeRoot,
    worktree_locator:                                  WorktreeAdministrativeLocator,
    claimed_at:                                        RecordedAt,
    last_activity_at:                                  RecordedAt,
}

/// Whether a holder has explicitly demonstrated recent reservation activity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReservationFreshness {
    /// A claim, widen, renew, or checkpoint occurred inside the freshness window.
    Fresh { last_activity_at: RecordedAt },
    /// No owner activity event occurred inside the freshness window.
    Stale { last_activity_at: RecordedAt },
}

/// Whether a conflicting holder is still recording coordination activity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ReservationHolderActivity {
    /// The holder recorded a claim, widen, renew, or checkpoint inside the freshness window.
    Active { last_activity_at: RecordedAt },
    /// The holder has gone quiet beyond the freshness window.
    Quiet { last_activity_at: RecordedAt },
}

impl From<ReservationFreshness> for ReservationHolderActivity {
    fn from(freshness: ReservationFreshness) -> Self {
        match freshness {
            ReservationFreshness::Fresh { last_activity_at } => Self::Active { last_activity_at },
            ReservationFreshness::Stale { last_activity_at } => Self::Quiet { last_activity_at },
        }
    }
}

/// One incursion incident and its current replayed disposition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IncursionIncident {
    id:                      IncursionIncidentId,
    reservation_id:          ReservationId,
    foreign_reservation_ids: ForeignReservationIdSet,
    paths:                   IncursionPathSet,
    status:                  IncursionIncidentStatus,
}

/// Whether an incursion still requires a user disposition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum IncursionIncidentStatus {
    /// No disposition record has answered this incident.
    Outstanding,
    /// One later journal event recorded the incident's disposition.
    Resolved {
        /// The worktree coordination run that recorded the disposition.
        resolving_actor:     JournalActor,
        /// The journal append that answered the incident.
        resolution_event_id: EventId,
        /// When the disposition was recorded.
        resolved_at:         RecordedAt,
    },
}

/// What a drift observation adds to the incursion incidents replay already carries.
pub(crate) enum IncursionObservation {
    /// Every entered path already belongs to this unanswered incident, which still stands.
    AlreadyOutstanding {
        /// The incident the caller should be pointed at rather than a fresh one.
        incident_id: IncursionIncidentId,
        /// The entered paths that incident already covers.
        paths:       IncursionPathSet,
    },
    /// Every entered path was already answered, and must not be raised again.
    AlreadyAnswered,
    /// These paths are new to this overlap and need a freshly created incident.
    NewlyObserved {
        /// The identity issued for the new incident.
        incident_id: IncursionIncidentId,
        /// Only the entered paths no incident accounts for yet.
        paths:       IncursionPathSet,
    },
}

/// How the incidents replay carries already account for one entered path.
enum IncursionPathCoverage {
    /// An unanswered incident already names this path under the same holders.
    Outstanding(IncursionIncidentId),
    /// A disposition already answered this path under the same holders.
    Answered,
    /// No incident accounts for this path yet.
    Uncovered,
}

/// Whether replay has recorded a protected tip for this reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
enum RetainedProtectedTip {
    /// An active reservation has not checkpointed a commit.
    NotCheckpointed,
    /// The checkpoint commit remains available after release.
    Retained(ProtectedReservationTip),
}

/// The trunk comparison point retained for the reservation's current state.
#[derive(Clone, Debug, Eq, PartialEq)]
enum IntegrationTrunkSnapshot {
    /// The trunk commit observed when the reservation was acquired.
    AtClaim(TrunkObservationAtClaim),
    /// The trunk commit observed with the protected tip.
    AtCheckpoint(GitObjectId),
}

/// Borrowed fields from one replayed claim event.
#[derive(Clone, Copy)]
struct ReplayedClaim<'event> {
    id:               ReservationId,
    scopes:           &'event ReservationScopeSet,
    source:           &'event ClaimSource,
    purpose:          &'event ReservationPurpose,
    trunk_at_claim:   &'event TrunkObservationAtClaim,
    head_snapshot:    &'event ClaimHeadSnapshot,
    phase_start_head: &'event ProtectedPhaseStartHead,
    actor:            &'event JournalActor,
    worktree_root:    &'event CanonicalWorktreeRoot,
    worktree_locator: &'event WorktreeAdministrativeLocator,
    authorization:    &'event ConflictAuthorization,
    recorded_at:      &'event RecordedAt,
}

/// State-specific evidence exposed without an optional protected commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReservationEvidenceState {
    /// Active work has no protected integration subject.
    Active {
        /// The trunk commit observed when the reservation was acquired.
        trunk_at_claim: TrunkObservationAtClaim,
    },
    /// A protected checkpoint awaits or has gained integration evidence.
    Outstanding {
        /// The retained checkpoint commit.
        protected_tip:      ProtectedReservationTip,
        /// The trunk commit observed with the checkpoint or latest resnapshot.
        trunk_snapshot:     GitObjectId,
        /// The most recently materialized integration result.
        integration_status: IntegrationEvidenceStatus,
    },
    /// A disposition was recorded while its evidence remains revalidatable.
    Released {
        /// The retained checkpoint commit.
        protected_tip:      ProtectedReservationTip,
        /// The trunk commit observed with the checkpoint or latest resnapshot.
        trunk_snapshot:     GitObjectId,
        /// The recorded terminal disposition.
        disposition:        ReleaseDisposition,
        /// The most recently materialized integration result.
        integration_status: IntegrationEvidenceStatus,
    },
    /// A user-confirmed active-work retirement that has no checkpoint evidence.
    ReleasedWithoutCheckpoint {
        /// The abandonment or orphan-retirement decision that ended the work.
        disposition: ReleaseDisposition,
    },
}

/// A point-in-time reading of one reservation's lifecycle.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "reservation_lifecycle")]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReservationLifecycleSnapshot {
    /// Work remains active without a protected checkpoint.
    Active,
    /// A protected checkpoint awaits integration or terminal resolution.
    Outstanding {
        /// The retained checkpoint commit.
        protected_tip: ProtectedReservationTip,
    },
    /// A terminal disposition followed a protected checkpoint.
    ReleasedAfterCheckpoint {
        /// The retained checkpoint commit.
        protected_tip: ProtectedReservationTip,
        /// The recorded terminal disposition.
        disposition:   ReleaseDisposition,
    },
    /// A terminal disposition ended work that never reached a checkpoint.
    ReleasedWithoutCheckpoint {
        /// The recorded terminal disposition.
        disposition: ReleaseDisposition,
    },
}

impl From<ReservationEvidenceState> for ReservationLifecycleSnapshot {
    fn from(evidence_state: ReservationEvidenceState) -> Self {
        match evidence_state {
            ReservationEvidenceState::Active { .. } => Self::Active,
            ReservationEvidenceState::Outstanding { protected_tip, .. } => {
                Self::Outstanding { protected_tip }
            },
            ReservationEvidenceState::Released {
                protected_tip,
                disposition,
                ..
            } => Self::ReleasedAfterCheckpoint {
                protected_tip,
                disposition,
            },
            ReservationEvidenceState::ReleasedWithoutCheckpoint { disposition } => {
                Self::ReleasedWithoutCheckpoint { disposition }
            },
        }
    }
}

/// One foreign holder whose retained reservation intersects requested scopes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ReservationConflict {
    /// The durable reservation that holds the overlapping paths.
    pub(crate) reservation_id:         ReservationId,
    /// The holder revision against which the overlap was evaluated.
    reservation_revision:              ReservationRevision,
    /// The holder revision that changes only when its scopes change.
    pub(crate) overlap_scope_revision: OverlapScopeRevision,
    /// The worktree identity that acquired the reservation.
    holder_worktree_id:                WorktreeId,
    /// The coordination run that acquired the reservation.
    pub(crate) holder_run_id:          CoordinationRunId,
    /// The holder's attached branch or detached commit.
    head_snapshot:                     ClaimHeadSnapshot,
    /// The holder's typed acquisition provenance.
    pub(crate) source:                 ClaimSource,
    /// The holder's typed reason for protecting the paths.
    pub(crate) purpose:                ReservationPurpose,
    /// The holder scopes that intersect the requested scopes.
    pub(crate) overlapping_scopes:     ReservationScopeSet,
    /// When the holder acquired the reservation.
    claimed_at:                        RecordedAt,
    /// Whether the holder has recorded activity inside the freshness window.
    activity:                          ReservationHolderActivity,
}

/// How current edit-blocking reservations cover one drift path.
pub(crate) enum DriftBlockingCoverage {
    /// Another reservation from the same run and worktree already claims the path.
    SameIdentity,
    /// Reservations from another run or worktree currently block the path.
    Foreign(Vec<ReservationConflict>),
    /// No edit-blocking reservation claims the path.
    Unclaimed,
}

/// The result of re-binding every overlapping scope against existing answers for a proposed
/// widening.
pub(crate) enum WidenScopeBinding {
    /// The complete widened scope set is covered by this durable authorization result.
    Authorized(ConflictAuthorization),
    /// One or more foreign overlaps have no existing answer for their exact scopes.
    Blocked(Vec<ReservationConflict>),
}

/// The actor identity permitted to receive its reservation-specific overlap answers.
///
/// Every identified variant names a worktree, because the worktree is the coordination
/// unit. Two runs in one worktree share one filesystem, one index, and one branch, so
/// they cannot produce the merge collision a reservation exists to prevent.
#[derive(Clone, Copy)]
pub(crate) enum AuthorizedEditingIdentity {
    /// A live session mapping identifies one exact reservation.
    SessionReservation {
        coordination_run_id: CoordinationRunId,
        reservation_id:      ReservationId,
        worktree_id:         WorktreeId,
    },
    /// The environment, a validated marker, or a locked first-touch transaction
    /// identifies this coordination run in this worktree.
    Run {
        coordination_run_id: CoordinationRunId,
        worktree_id:         WorktreeId,
    },
    /// No coordination run can be proven for this edit.
    Unidentified,
}

impl RetainedReservationSet {
    /// Replay journal operations into the current retained reservation set.
    pub(crate) fn replay(events: &[JournalEvent]) -> Result<Self, ReservationReplayError> {
        let mut reservations = Self::default();
        for event in events {
            reservations.apply(event)?;
        }
        Ok(reservations)
    }

    /// Evaluate claim acquisition for one acting worktree.
    pub(crate) fn conflicts_for_claim(
        &self,
        candidate: &ReservationScopeSet,
        acting_worktree_id: WorktreeId,
        path_case: PathCase,
    ) -> Vec<ReservationConflict> {
        self.conflicts(candidate, path_case, |holder| {
            holder.actor.worktree != acting_worktree_id
        })
    }

    /// Evaluate changed paths against edit-blocking reservations of another worktree.
    fn conflicts_for_drift(
        &self,
        candidate: &ReservationScopeSet,
        acting_worktree_id: WorktreeId,
        path_case: PathCase,
    ) -> Vec<ReservationConflict> {
        self.conflicts(candidate, path_case, |holder| {
            holder.actor.worktree != acting_worktree_id
        })
    }

    /// Classify all blocking coverage of one changed path in drift-table order.
    pub(crate) fn blocking_coverage_for_drift(
        &self,
        candidate: &ReservationScopeSet,
        acting_worktree_id: WorktreeId,
        path_case: PathCase,
    ) -> DriftBlockingCoverage {
        if !self
            .conflicts_with_holders(candidate, path_case, |holder| {
                holder.actor.worktree == acting_worktree_id
            })
            .is_empty()
        {
            return DriftBlockingCoverage::SameIdentity;
        }
        let conflicts = self.conflicts_for_drift(candidate, acting_worktree_id, path_case);
        if conflicts.is_empty() {
            DriftBlockingCoverage::Unclaimed
        } else {
            DriftBlockingCoverage::Foreign(conflicts)
        }
    }

    /// Re-run exact overlap binding against one reservation's complete widened scope set.
    pub(crate) fn bind_widened_scopes(
        &self,
        subject: &Reservation,
        added_scopes: &ReservationScopeAdditionSet,
        path_case: PathCase,
    ) -> WidenScopeBinding {
        let mut widened_scopes = subject.scopes.as_slice().to_vec();
        widened_scopes.extend(added_scopes.as_slice().iter().cloned());
        let complete_scopes = ReservationScopeSet::try_from(widened_scopes).map_or_else(
            |_| subject.scopes.clone(),
            |scopes| scopes.minimal_antichain(path_case),
        );
        let conflicts = self.conflicts_with_holders(&complete_scopes, path_case, |holder| {
            holder.actor.run != subject.actor.run || holder.actor.worktree != subject.actor.worktree
        });
        let blocked =
            conflicts
                .iter()
                .filter(|(holder, conflict)| {
                    conflict.overlapping_scopes.as_slice().iter().any(|scope| {
                        !reservations_authorize_scope(subject, holder, scope, path_case)
                    })
                })
                .map(|(_, conflict)| conflict.clone())
                .collect::<Vec<_>>();
        if !blocked.is_empty() {
            return WidenScopeBinding::Blocked(blocked);
        }
        let overlaps = conflicts
            .iter()
            .map(|(_, conflict)| AuthorizedOverlap::from(conflict))
            .collect::<Vec<_>>();
        AuthorizedOverlapSet::try_from(overlaps).map_or(
            WidenScopeBinding::Authorized(ConflictAuthorization::NoConflict),
            |overlaps| {
                WidenScopeBinding::Authorized(
                    ConflictAuthorization::ExistingAnswersCoverEveryOverlap { overlaps },
                )
            },
        )
    }

    /// Evaluate an edit check using only authorization resolved by the process.
    pub(crate) fn conflicts_for_edit(
        &self,
        candidate: &ReservationScopeSet,
        edit_authorization: EditAuthorization,
        path_case: PathCase,
    ) -> Vec<ReservationConflict> {
        let authorized_editing_identity = self.resolve_editing_identity(edit_authorization);
        self.conflicts_for_authorized_edit(candidate, authorized_editing_identity, path_case)
    }

    /// Evaluate a locked first-touch claim for one exact reservation actor.
    pub(crate) fn conflicts_for_first_touch(
        &self,
        candidate: &ReservationScopeSet,
        coordination_run_id: CoordinationRunId,
        worktree_id: WorktreeId,
        path_case: PathCase,
    ) -> Vec<ReservationConflict> {
        self.conflicts_for_authorized_edit(
            candidate,
            AuthorizedEditingIdentity::Run {
                coordination_run_id,
                worktree_id,
            },
            path_case,
        )
    }

    fn conflicts_for_authorized_edit(
        &self,
        candidate: &ReservationScopeSet,
        authorized_editing_identity: AuthorizedEditingIdentity,
        path_case: PathCase,
    ) -> Vec<ReservationConflict> {
        let conflicts = self.conflicts_with_holders(candidate, path_case, |holder| {
            authorized_editing_identity.is_foreign(holder)
        });
        let mut unanswered_conflicts = Vec::new();
        for (holder, mut conflict) in conflicts {
            let unanswered_scopes = conflict
                .overlapping_scopes
                .as_slice()
                .iter()
                .filter(|overlap_scope| {
                    !authorized_editing_identity.authorizes(self, holder, overlap_scope, path_case)
                })
                .cloned()
                .collect::<Vec<_>>();
            if let Ok(overlapping_scopes) = ReservationScopeSet::try_from(unanswered_scopes) {
                conflict.overlapping_scopes = overlapping_scopes;
                unanswered_conflicts.push(conflict);
            }
        }
        unanswered_conflicts
    }

    /// Validate a process-resolved edit identity against retained active reservations.
    pub(crate) fn resolve_editing_identity(
        &self,
        edit_authorization: EditAuthorization,
    ) -> AuthorizedEditingIdentity {
        let session_is_active = match edit_authorization {
            EditAuthorization::Session {
                coordination_run_id,
                reservation_id,
                worktree_id,
            } => self.reservations.iter().any(|reservation| {
                reservation.id == reservation_id
                    && matches!(reservation.lifecycle, ReservationLifecycle::Active)
                    && reservation.actor.run == coordination_run_id
                    && reservation.actor.worktree == worktree_id
            }),
            EditAuthorization::Environment { .. }
            | EditAuthorization::Marker { .. }
            | EditAuthorization::Unidentified => false,
        };
        let marker_is_active = match edit_authorization {
            EditAuthorization::Marker {
                coordination_run_id,
                worktree_id,
            } => self.reservations.iter().any(|reservation| {
                matches!(reservation.lifecycle, ReservationLifecycle::Active)
                    && reservation.actor.run == coordination_run_id
                    && reservation.actor.worktree == worktree_id
            }),
            EditAuthorization::Session { .. }
            | EditAuthorization::Environment { .. }
            | EditAuthorization::Unidentified => false,
        };
        match edit_authorization {
            EditAuthorization::Session {
                coordination_run_id,
                reservation_id,
                worktree_id,
            } if session_is_active => AuthorizedEditingIdentity::SessionReservation {
                coordination_run_id,
                reservation_id,
                worktree_id,
            },
            EditAuthorization::Environment {
                coordination_run_id,
                worktree_id,
            } => AuthorizedEditingIdentity::Run {
                coordination_run_id,
                worktree_id,
            },
            EditAuthorization::Marker {
                coordination_run_id,
                worktree_id,
            } if marker_is_active => AuthorizedEditingIdentity::Run {
                coordination_run_id,
                worktree_id,
            },
            EditAuthorization::Session { .. }
            | EditAuthorization::Marker { .. }
            | EditAuthorization::Unidentified => AuthorizedEditingIdentity::Unidentified,
        }
    }

    /// Iterate over every reservation retained for constraints or audit history.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Reservation> { self.reservations.iter() }

    /// Find one retained reservation by its non-recyclable identity.
    pub(crate) fn reservation(
        &self,
        reservation_id: ReservationId,
    ) -> Result<&Reservation, ReservationReplayError> {
        self.reservations
            .iter()
            .find(|reservation| reservation.id == reservation_id)
            .ok_or(ReservationReplayError::UnknownReservation(reservation_id))
    }

    /// Find one retained incursion by its durable identity.
    pub(crate) fn incursion_incident(
        &self,
        incident_id: IncursionIncidentId,
    ) -> Result<&IncursionIncident, ReservationReplayError> {
        self.incursion_incidents
            .iter()
            .find(|incident| incident.id() == incident_id)
            .ok_or(ReservationReplayError::UnknownIncursionIncident(
                incident_id,
            ))
    }

    /// Classify an observed incursion against the incidents replay already carries.
    ///
    /// Coverage is decided one path at a time rather than by comparing whole sets. A
    /// straying edit is observed again on every drift run, so an observation that adds
    /// one path to an overlap already reported arrives as a superset of it; matching on
    /// set equality created a second incident that re-covered the first one's ground, and
    /// each then had to be answered separately.
    ///
    /// An answered path stays answered. The edit remains on disk after a disposition is
    /// recorded, so re-raising it would hand the caller a warning no answer can clear.
    pub(crate) fn observe_incursion(
        &self,
        reservation_id: ReservationId,
        foreign_reservation_ids: &ForeignReservationIdSet,
        paths: &IncursionPathSet,
    ) -> IncursionObservation {
        let mut outstanding = None;
        let mut outstanding_paths = Vec::new();
        let mut uncovered_paths = Vec::new();
        for path in paths.as_slice() {
            match self.incursion_path_coverage(reservation_id, foreign_reservation_ids, path) {
                IncursionPathCoverage::Outstanding(incident_id) => {
                    outstanding.get_or_insert(incident_id);
                    outstanding_paths.push(path.clone());
                },
                IncursionPathCoverage::Answered => {},
                IncursionPathCoverage::Uncovered => uncovered_paths.push(path.clone()),
            }
        }
        if let Ok(paths) = IncursionPathSet::try_from(uncovered_paths) {
            return IncursionObservation::NewlyObserved {
                incident_id: IncursionIncidentId::new(),
                paths,
            };
        }
        match (outstanding, IncursionPathSet::try_from(outstanding_paths)) {
            (Some(incident_id), Ok(paths)) => {
                IncursionObservation::AlreadyOutstanding { incident_id, paths }
            },
            _ => IncursionObservation::AlreadyAnswered,
        }
    }

    /// Decide whether any retained incident already accounts for one entered path.
    ///
    /// The holders are compared by containment rather than equality, matching the
    /// sibling suppression in drift classification: an incident naming every holder
    /// observed now already covers what this observation would report.
    fn incursion_path_coverage(
        &self,
        reservation_id: ReservationId,
        foreign_reservation_ids: &ForeignReservationIdSet,
        path: &ReservationScopePath,
    ) -> IncursionPathCoverage {
        let mut answered = false;
        for incident in &self.incursion_incidents {
            if incident.reservation_id() != reservation_id
                || !incident.paths().as_slice().contains(path)
                || !foreign_reservation_ids.as_slice().iter().all(|holder| {
                    incident
                        .foreign_reservation_ids()
                        .as_slice()
                        .contains(holder)
                })
            {
                continue;
            }
            match incident.status() {
                IncursionIncidentStatus::Outstanding => {
                    return IncursionPathCoverage::Outstanding(incident.id());
                },
                IncursionIncidentStatus::Resolved { .. } => answered = true,
            }
        }
        if answered {
            IncursionPathCoverage::Answered
        } else {
            IncursionPathCoverage::Uncovered
        }
    }

    /// Iterate over the incursion incidents that still require a disposition.
    pub(crate) fn outstanding_incursion_incidents(
        &self,
    ) -> impl Iterator<Item = &IncursionIncident> {
        self.incursion_incidents
            .iter()
            .filter(|incident| matches!(incident.status(), IncursionIncidentStatus::Outstanding))
    }

    /// Iterate every retained incident for outstanding and resolved audit sections.
    pub(crate) fn incursion_incidents(&self) -> impl Iterator<Item = &IncursionIncident> {
        self.incursion_incidents.iter()
    }

    /// Return whether the run still has another reservation in `Active`.
    pub(crate) fn has_other_active_reservation(
        &self,
        coordination_run_id: CoordinationRunId,
        excluded_reservation_id: ReservationId,
    ) -> bool {
        self.reservations.iter().any(|reservation| {
            reservation.id != excluded_reservation_id
                && reservation.actor.run == coordination_run_id
                && matches!(reservation.lifecycle, ReservationLifecycle::Active)
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one exhaustive operation match keeps replay dispatch visibly complete"
    )]
    fn apply(&mut self, event: &JournalEvent) -> Result<(), ReservationReplayError> {
        match &event.operation {
            JournalOperation::Claim {
                reservation_id,
                scopes,
                source,
                purpose,
                trunk_at_claim,
                head_snapshot,
                phase_start_head,
                worktree_root,
                worktree_administrative_locator,
                authorization,
                ..
            } => self.apply_claim(ReplayedClaim {
                id: *reservation_id,
                scopes,
                source,
                purpose,
                trunk_at_claim,
                head_snapshot,
                phase_start_head,
                actor: &event.actor,
                worktree_root,
                worktree_locator: worktree_administrative_locator,
                authorization,
                recorded_at: event.recorded_at(),
            })?,
            JournalOperation::Widen {
                reservation_id,
                added_scopes,
                authorization,
                ..
            } => self.apply_widen(
                *reservation_id,
                added_scopes,
                authorization,
                event.recorded_at(),
            )?,
            JournalOperation::Checkpoint {
                reservation_id,
                protected_tip,
                trunk_snapshot,
            } => self.apply_checkpoint(
                *reservation_id,
                protected_tip,
                trunk_snapshot,
                event.recorded_at(),
            )?,
            JournalOperation::Resnapshot {
                reservation_id,
                snapshot,
            } => self.apply_resnapshot(*reservation_id, snapshot)?,
            JournalOperation::Renew { reservation_id } => {
                let reservation = self.find_mut(*reservation_id)?;
                reservation.last_activity_at = event.recorded_at().clone();
                reservation.advance_revision()?;
            },
            JournalOperation::Release {
                reservation_id,
                disposition,
            } => self.apply_release(*reservation_id, disposition)?,
            JournalOperation::ReplaceReleaseDisposition {
                reservation_id,
                superseded,
                replacement,
            } => self.apply_replacement(*reservation_id, superseded, replacement)?,
            JournalOperation::EvidenceRevalidated {
                reservation_id,
                status,
                ..
            } => self.apply_evidence(*reservation_id, status)?,
            JournalOperation::ScopedPatchEquivalenceChecked {
                reservation_id,
                subject,
                target,
                verdict,
            } => self.apply_scoped_patch_equivalence_check(
                *reservation_id,
                *subject,
                target,
                *verdict,
                event.projection_generation(),
            )?,
            JournalOperation::ScopedPatchComparisonAttempted {
                reservation_id,
                subject,
                target,
            } => self.apply_scoped_patch_comparison_attempt(
                *reservation_id,
                *subject,
                target,
                event.projection_generation(),
            )?,
            JournalOperation::SuccessorScopedPatchEquivalenceChecked {
                predecessor_reservation_id,
                subject,
                successor_head,
                verdict,
            } => self.apply_successor_scoped_patch_equivalence_check(
                *predecessor_reservation_id,
                *subject,
                successor_head,
                *verdict,
                event.projection_generation(),
            )?,
            JournalOperation::SuccessorScopedPatchComparisonAttempted {
                predecessor_reservation_id,
                subject,
                successor_head,
            } => self.apply_successor_scoped_patch_comparison_attempt(
                *predecessor_reservation_id,
                *subject,
                successor_head,
                event.projection_generation(),
            )?,
            JournalOperation::RebindWorktree {
                reservation_id,
                previous_worktree_id,
                current_worktree_id,
                current_worktree_root,
                current_worktree_administrative_locator,
            } => self.apply_worktree_rebinding(
                *reservation_id,
                *previous_worktree_id,
                *current_worktree_id,
                current_worktree_root,
                current_worktree_administrative_locator,
            )?,
            JournalOperation::RelocateWorktree {
                reservation_id,
                worktree_id,
                previous_root,
                current_root,
            } => self.apply_worktree_relocation(
                *reservation_id,
                *worktree_id,
                previous_root,
                current_root,
            )?,
            JournalOperation::Incursion { .. } | JournalOperation::ResolveIncursion { .. } => {
                self.apply_incursion_journal_event(event)?;
            },
            JournalOperation::ResolveDefer { .. }
            | JournalOperation::ForcedIntegrationPermit { .. }
            | JournalOperation::ConsumeForcedIntegrationPermit { .. }
            | JournalOperation::Bypass { .. } => {},
        }
        Ok(())
    }

    fn apply_incursion_journal_event(
        &mut self,
        event: &JournalEvent,
    ) -> Result<(), ReservationReplayError> {
        match &event.operation {
            JournalOperation::Incursion {
                incident_id,
                reservation_id,
                foreign_reservation_ids,
                paths,
            } => self.apply_incursion(
                *incident_id,
                *reservation_id,
                foreign_reservation_ids,
                paths,
            ),
            JournalOperation::ResolveIncursion { incident_id } => self.apply_incursion_resolution(
                *incident_id,
                &event.actor,
                event.event_id(),
                event.recorded_at(),
            ),
            _ => Ok(()),
        }
    }

    fn apply_incursion(
        &mut self,
        incident_id: IncursionIncidentId,
        reservation_id: ReservationId,
        foreign_reservation_ids: &ForeignReservationIdSet,
        paths: &IncursionPathSet,
    ) -> Result<(), ReservationReplayError> {
        self.reservation(reservation_id)?;
        if self
            .incursion_incidents
            .iter()
            .any(|incident| incident.id == incident_id)
        {
            return Err(ReservationReplayError::DuplicateIncursionIncident(
                incident_id,
            ));
        }
        self.incursion_incidents.push(IncursionIncident {
            id: incident_id,
            reservation_id,
            foreign_reservation_ids: foreign_reservation_ids.clone(),
            paths: paths.clone(),
            status: IncursionIncidentStatus::Outstanding,
        });
        Ok(())
    }

    fn apply_incursion_resolution(
        &mut self,
        incident_id: IncursionIncidentId,
        resolving_actor: &JournalActor,
        resolution_event_id: EventId,
        resolved_at: &RecordedAt,
    ) -> Result<(), ReservationReplayError> {
        let incident = self
            .incursion_incidents
            .iter_mut()
            .find(|incident| incident.id == incident_id)
            .ok_or(ReservationReplayError::UnknownIncursionIncident(
                incident_id,
            ))?;
        if matches!(incident.status, IncursionIncidentStatus::Resolved { .. }) {
            return Err(ReservationReplayError::IncursionIncidentAlreadyResolved(
                incident_id,
            ));
        }
        incident.status = IncursionIncidentStatus::Resolved {
            resolving_actor: resolving_actor.clone(),
            resolution_event_id,
            resolved_at: resolved_at.clone(),
        };
        Ok(())
    }

    fn apply_worktree_rebinding(
        &mut self,
        reservation_id: ReservationId,
        previous_worktree_id: WorktreeId,
        current_worktree_id: WorktreeId,
        current_worktree_root: &CanonicalWorktreeRoot,
        current_worktree_locator: &WorktreeAdministrativeLocator,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        if reservation.actor.worktree != previous_worktree_id {
            return Err(ReservationReplayError::WorktreeRebindingMismatch(
                reservation_id,
            ));
        }
        reservation.actor.worktree = current_worktree_id;
        reservation.worktree_root = current_worktree_root.clone();
        reservation.worktree_locator = current_worktree_locator.clone();
        reservation.advance_revision()
    }

    fn apply_worktree_relocation(
        &mut self,
        reservation_id: ReservationId,
        worktree_id: WorktreeId,
        previous_root: &CanonicalWorktreeRoot,
        current_root: &CanonicalWorktreeRoot,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        if reservation.actor.worktree != worktree_id || reservation.worktree_root != *previous_root
        {
            return Err(ReservationReplayError::WorktreeRelocationMismatch(
                reservation_id,
            ));
        }
        reservation.worktree_root = current_root.clone();
        reservation.advance_revision()
    }

    fn apply_claim(
        &mut self,
        replayed_claim: ReplayedClaim<'_>,
    ) -> Result<(), ReservationReplayError> {
        if self
            .reservations
            .iter()
            .any(|reservation| reservation.id == replayed_claim.id)
        {
            return Err(ReservationReplayError::DuplicateClaim(replayed_claim.id));
        }
        self.reservations.push(Reservation {
            id:                                                replayed_claim.id,
            revision:                                          ReservationRevision::from(1),
            integration_proof_subject:
                IntegrationProofSubjectRevision::INITIAL,
            retained_scoped_patch_target_verdicts:
                RetainedScopedPatchTargetVerdicts::default(),
            scoped_patch_target_evaluation_schedule:
                ScopedPatchTargetEvaluationSchedule::default(),
            retained_successor_scoped_patch_verdicts:
                RetainedSuccessorScopedPatchTargetVerdicts::default(),
            successor_scoped_patch_target_evaluation_schedule:
                SuccessorScopedPatchTargetEvaluationSchedule::default(),
            scopes:                                            replayed_claim.scopes.clone(),
            authorizations:                                    vec![
                replayed_claim.authorization.clone(),
            ],
            source:                                            replayed_claim.source.clone(),
            purpose:                                           replayed_claim.purpose.clone(),
            head_snapshot:                                     replayed_claim.head_snapshot.clone(),
            phase_start_head:                                  replayed_claim
                .phase_start_head
                .clone(),
            actor:                                             replayed_claim.actor.clone(),
            lifecycle:                                         ReservationLifecycle::Active,
            retained_protected_tip:
                RetainedProtectedTip::NotCheckpointed,
            integration_trunk_snapshot:                        IntegrationTrunkSnapshot::AtClaim(
                replayed_claim.trunk_at_claim.clone(),
            ),
            integration_status:
                IntegrationEvidenceStatus::NotIntegrated,
            worktree_root:                                     replayed_claim.worktree_root.clone(),
            worktree_locator:                                  replayed_claim
                .worktree_locator
                .clone(),
            claimed_at:                                        replayed_claim.recorded_at.clone(),
            last_activity_at:                                  replayed_claim.recorded_at.clone(),
        });
        Ok(())
    }

    fn apply_widen(
        &mut self,
        reservation_id: ReservationId,
        added_scopes: &ReservationScopeAdditionSet,
        authorization: &ConflictAuthorization,
        recorded_at: &RecordedAt,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        if matches!(reservation.lifecycle, ReservationLifecycle::Released { .. }) {
            return Err(ReservationReplayError::WidenRequiresUnreleased(
                reservation_id,
            ));
        }
        let mut scopes = reservation.scopes.as_slice().to_vec();
        scopes.extend(added_scopes.as_slice().iter().cloned());
        reservation.scopes = ReservationScopeSet::try_from(scopes)
            .map_err(|_| ReservationReplayError::EmptyScopeSet(reservation_id))?;
        if matches!(
            reservation.lifecycle,
            ReservationLifecycle::Outstanding { .. }
        ) {
            reservation.integration_status = IntegrationEvidenceStatus::NotIntegrated;
        }
        reservation.advance_integration_proof_subject_revision()?;
        reservation.last_activity_at = recorded_at.clone();
        reservation.advance_revision()?;
        reservation.authorizations.push(authorization.clone());
        Ok(())
    }

    fn apply_checkpoint(
        &mut self,
        reservation_id: ReservationId,
        protected_tip: &ProtectedReservationTip,
        trunk_snapshot: &GitObjectId,
        recorded_at: &RecordedAt,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        reservation
            .lifecycle
            .checkpoint(protected_tip.clone())
            .map_err(|error| {
                ReservationReplayError::InvalidLifecycleTransition(reservation_id, error)
            })?;
        reservation.retained_protected_tip = RetainedProtectedTip::Retained(protected_tip.clone());
        reservation.integration_trunk_snapshot =
            IntegrationTrunkSnapshot::AtCheckpoint(trunk_snapshot.clone());
        reservation.integration_status = IntegrationEvidenceStatus::NotIntegrated;
        reservation.last_activity_at = recorded_at.clone();
        reservation.advance_revision()
    }

    fn apply_resnapshot(
        &mut self,
        reservation_id: ReservationId,
        snapshot: &ReservationSnapshot,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        match snapshot {
            ReservationSnapshot::Active { claim_snapshot } => {
                if !matches!(reservation.lifecycle, ReservationLifecycle::Active) {
                    return Err(ReservationReplayError::SnapshotStateMismatch(
                        reservation_id,
                    ));
                }
                reservation.phase_start_head =
                    ProtectedPhaseStartHead::from(claim_snapshot.clone());
            },
            ReservationSnapshot::Outstanding {
                protected_tip,
                trunk_oid,
            } => {
                if matches!(reservation.lifecycle, ReservationLifecycle::Released { .. }) {
                    reservation.advance_integration_proof_subject_revision()?;
                    return reservation.advance_revision();
                }
                reservation
                    .lifecycle
                    .resnapshot(protected_tip.clone())
                    .map_err(|error| {
                        ReservationReplayError::InvalidLifecycleTransition(reservation_id, error)
                    })?;
                reservation.retained_protected_tip =
                    RetainedProtectedTip::Retained(protected_tip.clone());
                reservation.integration_trunk_snapshot =
                    IntegrationTrunkSnapshot::AtCheckpoint(trunk_oid.clone());
                reservation.integration_status = IntegrationEvidenceStatus::NotIntegrated;
            },
        }
        reservation.advance_integration_proof_subject_revision()?;
        reservation.advance_revision()
    }

    fn apply_release(
        &mut self,
        reservation_id: ReservationId,
        disposition: &ReleaseDisposition,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        if matches!(disposition, ReleaseDisposition::Integrated)
            && !matches!(
                reservation.integration_status,
                IntegrationEvidenceStatus::Integrated { .. }
            )
        {
            return Err(ReservationReplayError::IntegratedReleaseWithoutEvidence(
                reservation_id,
            ));
        }
        if let ReleaseDisposition::RewrittenIntegration(trunk_commit) = disposition {
            reservation.integration_status = IntegrationEvidenceStatus::Integrated {
                trunk_oid: trunk_commit.as_ref().clone(),
                proof:     IntegrationProof::ProtectedTipAncestor,
            };
            reservation.advance_integration_proof_subject_revision()?;
        }
        match disposition {
            ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_) => reservation
                .lifecycle
                .release_after_user_confirmation(disposition.clone())
                .map_err(|error| {
                    ReservationReplayError::InvalidLifecycleTransition(reservation_id, error)
                })?,
            ReleaseDisposition::Integrated | ReleaseDisposition::RewrittenIntegration(_) => {
                reservation
                    .lifecycle
                    .release(disposition.clone())
                    .map_err(|error| {
                        ReservationReplayError::InvalidLifecycleTransition(reservation_id, error)
                    })?;
            },
        }
        reservation.advance_revision()
    }

    fn apply_evidence(
        &mut self,
        reservation_id: ReservationId,
        status: &IntegrationEvidenceStatus,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        match &reservation.lifecycle {
            ReservationLifecycle::Active => {
                return Err(ReservationReplayError::ActiveEvidenceRevalidation(
                    reservation_id,
                ));
            },
            ReservationLifecycle::Outstanding { .. } => {},
            ReservationLifecycle::Released { disposition } => {
                if matches!(
                    disposition.revalidation_subject(),
                    ReleaseRevalidationSubject::None
                ) {
                    return Err(ReservationReplayError::DecisionHasNoGitEvidence(
                        reservation_id,
                    ));
                }
            },
        }
        reservation.integration_status = status.clone();
        reservation.advance_revision()
    }

    fn apply_scoped_patch_equivalence_check(
        &mut self,
        reservation_id: ReservationId,
        subject: IntegrationProofSubjectRevision,
        target: &GitObjectId,
        verdict: ScopedPatchEquivalenceVerdict,
        generation: ProjectionGeneration,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.scoped_patch_comparison_subject_mut(reservation_id, subject)?;
        reservation
            .retained_scoped_patch_target_verdicts
            .record(subject, target, verdict);
        reservation
            .scoped_patch_target_evaluation_schedule
            .record(subject, target, generation);
        reservation.advance_revision()
    }

    fn apply_scoped_patch_comparison_attempt(
        &mut self,
        reservation_id: ReservationId,
        subject: IntegrationProofSubjectRevision,
        target: &GitObjectId,
        generation: ProjectionGeneration,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.scoped_patch_comparison_subject_mut(reservation_id, subject)?;
        reservation
            .scoped_patch_target_evaluation_schedule
            .record(subject, target, generation);
        reservation.advance_revision()
    }

    fn apply_successor_scoped_patch_equivalence_check(
        &mut self,
        predecessor_reservation_id: ReservationId,
        subject: IntegrationProofSubjectRevision,
        successor_head: &GitObjectId,
        verdict: SuccessorScopedPatchEquivalenceVerdict,
        generation: ProjectionGeneration,
    ) -> Result<(), ReservationReplayError> {
        let predecessor =
            self.scoped_patch_comparison_subject_mut(predecessor_reservation_id, subject)?;
        predecessor.retained_successor_scoped_patch_verdicts.record(
            subject,
            successor_head,
            verdict,
        );
        predecessor
            .successor_scoped_patch_target_evaluation_schedule
            .record(subject, successor_head, generation);
        predecessor.advance_revision()
    }

    fn apply_successor_scoped_patch_comparison_attempt(
        &mut self,
        predecessor_reservation_id: ReservationId,
        subject: IntegrationProofSubjectRevision,
        successor_head: &GitObjectId,
        generation: ProjectionGeneration,
    ) -> Result<(), ReservationReplayError> {
        let predecessor =
            self.scoped_patch_comparison_subject_mut(predecessor_reservation_id, subject)?;
        predecessor
            .successor_scoped_patch_target_evaluation_schedule
            .record(subject, successor_head, generation);
        predecessor.advance_revision()
    }

    fn scoped_patch_comparison_subject_mut(
        &mut self,
        reservation_id: ReservationId,
        subject: IntegrationProofSubjectRevision,
    ) -> Result<&mut Reservation, ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        match &reservation.lifecycle {
            ReservationLifecycle::Active => {
                return Err(ReservationReplayError::ActiveScopedPatchComparison(
                    reservation_id,
                ));
            },
            ReservationLifecycle::Outstanding { .. } => {},
            ReservationLifecycle::Released { disposition } => {
                if matches!(
                    disposition.revalidation_subject(),
                    ReleaseRevalidationSubject::None
                ) {
                    return Err(ReservationReplayError::DecisionHasNoGitEvidence(
                        reservation_id,
                    ));
                }
            },
        }
        if reservation.integration_proof_subject != subject {
            return Err(ReservationReplayError::IntegrationProofSubjectMismatch(
                reservation_id,
            ));
        }
        Ok(reservation)
    }

    fn apply_replacement(
        &mut self,
        reservation_id: ReservationId,
        superseded: &ReleaseDisposition,
        replacement: &ReleaseDisposition,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        if !matches!(replacement, ReleaseDisposition::RewrittenIntegration(_)) {
            return Err(ReservationReplayError::InvalidReplacementDisposition(
                reservation_id,
            ));
        }
        reservation
            .lifecycle
            .replace_release_disposition(superseded, replacement.clone())
            .map_err(|error| {
                ReservationReplayError::InvalidLifecycleTransition(reservation_id, error)
            })?;
        if let ReleaseDisposition::RewrittenIntegration(trunk_commit) = replacement {
            reservation.integration_status = IntegrationEvidenceStatus::Integrated {
                trunk_oid: trunk_commit.as_ref().clone(),
                proof:     IntegrationProof::ProtectedTipAncestor,
            };
        }
        reservation.advance_integration_proof_subject_revision()?;
        reservation.advance_revision()
    }

    fn find_mut(
        &mut self,
        reservation_id: ReservationId,
    ) -> Result<&mut Reservation, ReservationReplayError> {
        self.reservations
            .iter_mut()
            .find(|reservation| reservation.id == reservation_id)
            .ok_or(ReservationReplayError::UnknownReservation(reservation_id))
    }

    fn conflicts(
        &self,
        candidate: &ReservationScopeSet,
        path_case: PathCase,
        holder_is_foreign: impl Fn(&Reservation) -> bool,
    ) -> Vec<ReservationConflict> {
        self.conflicts_with_holders(candidate, path_case, holder_is_foreign)
            .into_iter()
            .map(|(_, conflict)| conflict)
            .collect()
    }

    fn conflicts_with_holders(
        &self,
        candidate: &ReservationScopeSet,
        path_case: PathCase,
        holder_is_foreign: impl Fn(&Reservation) -> bool,
    ) -> Vec<(&Reservation, ReservationConflict)> {
        let observed_at = RecordedAt::now();
        self.reservations
            .iter()
            .filter(|holder| holder.edit_blocking_status() == EditBlockingStatus::Blocking)
            .filter(|holder| holder_is_foreign(holder))
            .filter_map(|holder| {
                let overlapping_scopes = holder
                    .scopes
                    .as_slice()
                    .iter()
                    .flat_map(|held_scope| {
                        candidate
                            .as_slice()
                            .iter()
                            .filter(|candidate_scope| {
                                held_scope.overlaps(candidate_scope, path_case)
                            })
                            .map(|candidate_scope| {
                                if held_scope.contains(candidate_scope, path_case) {
                                    candidate_scope.clone()
                                } else {
                                    held_scope.clone()
                                }
                            })
                    })
                    .collect::<Vec<_>>();
                ReservationScopeSet::try_from(overlapping_scopes)
                    .ok()
                    .map(|overlapping_scopes| {
                        (
                            holder,
                            ReservationConflict {
                                reservation_id:         holder.id,
                                reservation_revision:   holder.revision,
                                overlap_scope_revision: OverlapScopeRevision::from(&holder.scopes),
                                holder_worktree_id:     holder.actor.worktree,
                                holder_run_id:          holder.actor.run,
                                head_snapshot:          holder.head_snapshot.clone(),
                                source:                 holder.source.clone(),
                                purpose:                holder.purpose.clone(),
                                overlapping_scopes:     overlapping_scopes
                                    .minimal_antichain(path_case),
                                claimed_at:             holder.claimed_at.clone(),
                                activity:               holder.freshness(&observed_at).into(),
                            },
                        )
                    })
            })
            .collect()
    }
}

impl AuthorizedEditingIdentity {
    /// Whether this holder belongs to another worktree, the only foreignness that blocks.
    ///
    /// A holder in the caller's own worktree is never foreign, however many coordination
    /// runs that worktree has issued. A run mismatch alone once blocked here, which let a
    /// worktree block itself with a reservation an earlier session in the same checkout
    /// had left behind.
    fn is_foreign(self, holder: &Reservation) -> bool {
        match self {
            Self::SessionReservation { worktree_id, .. } | Self::Run { worktree_id, .. } => {
                holder.actor.worktree != worktree_id
            },
            Self::Unidentified => true,
        }
    }

    fn authorizes(
        self,
        reservations: &RetainedReservationSet,
        holder: &Reservation,
        overlap_scope: &ReservationScope,
        path_case: PathCase,
    ) -> bool {
        reservations
            .reservations
            .iter()
            .filter(|requester| {
                self.identifies_requester(requester)
                    && requester.edit_blocking_status() == EditBlockingStatus::Blocking
                    && requester
                        .scopes
                        .as_slice()
                        .iter()
                        .any(|scope| scope.overlaps(overlap_scope, path_case))
            })
            .any(|requester| {
                reservations_authorize_scope(requester, holder, overlap_scope, path_case)
            })
    }

    /// Whether this reservation is one the caller's own worktree holds.
    ///
    /// Overlap answers bind the worktree that recorded them, so a later run in the same
    /// worktree inherits them along with the reservations they were recorded against.
    fn identifies_requester(self, requester: &Reservation) -> bool {
        match self {
            Self::SessionReservation { worktree_id, .. } | Self::Run { worktree_id, .. } => {
                requester.actor.worktree == worktree_id
            },
            Self::Unidentified => false,
        }
    }
}

fn reservations_authorize_scope(
    requester: &Reservation,
    holder: &Reservation,
    overlap_scope: &ReservationScope,
    path_case: PathCase,
) -> bool {
    let holder_scope_revision = OverlapScopeRevision::from(&holder.scopes);
    let requester_scope_revision = OverlapScopeRevision::from(&requester.scopes);
    requester.authorizations.iter().any(|authorization| {
        authorization.covers(holder.id, &holder_scope_revision, overlap_scope, path_case)
    }) || holder.authorizations.iter().any(|authorization| {
        authorization.covers(
            requester.id,
            &requester_scope_revision,
            overlap_scope,
            path_case,
        )
    })
}

impl Reservation {
    /// Return the reservation's durable identity.
    pub(crate) const fn id(&self) -> ReservationId { self.id }

    fn advance_revision(&mut self) -> Result<(), ReservationReplayError> {
        let revision: u64 = self.revision.into();
        self.revision = revision
            .checked_add(1)
            .map(ReservationRevision::from)
            .ok_or(ReservationReplayError::RevisionExhausted(self.id))?;
        Ok(())
    }

    fn advance_integration_proof_subject_revision(&mut self) -> Result<(), ReservationReplayError> {
        self.integration_proof_subject = self
            .integration_proof_subject
            .0
            .checked_add(1)
            .map(IntegrationProofSubjectRevision)
            .ok_or(ReservationReplayError::IntegrationProofSubjectRevisionExhausted(self.id))?;
        self.retained_scoped_patch_target_verdicts = RetainedScopedPatchTargetVerdicts::default();
        self.scoped_patch_target_evaluation_schedule =
            ScopedPatchTargetEvaluationSchedule::default();
        self.retained_successor_scoped_patch_verdicts =
            RetainedSuccessorScopedPatchTargetVerdicts::default();
        self.successor_scoped_patch_target_evaluation_schedule =
            SuccessorScopedPatchTargetEvaluationSchedule::default();
        Ok(())
    }

    /// Return the revision of the content subject used by scoped patch equivalence.
    pub(crate) const fn integration_proof_subject_revision(
        &self,
    ) -> IntegrationProofSubjectRevision {
        self.integration_proof_subject
    }

    /// Borrow the durable scoped patch verdict for this reservation.
    pub(crate) const fn retained_scoped_patch_target_verdicts(
        &self,
    ) -> &RetainedScopedPatchTargetVerdicts {
        &self.retained_scoped_patch_target_verdicts
    }

    /// Return this proof subject's scheduling priority for one trunk target.
    pub(crate) fn scoped_patch_evaluation_priority(
        &self,
        target: &GitObjectId,
    ) -> ScopedPatchEvaluationPriority {
        self.scoped_patch_target_evaluation_schedule
            .priority(self.integration_proof_subject, target)
    }

    /// Borrow retained scoped patch verdicts for successor incorporation.
    pub(crate) const fn retained_successor_scoped_patch_target_verdicts(
        &self,
    ) -> &RetainedSuccessorScopedPatchTargetVerdicts {
        &self.retained_successor_scoped_patch_verdicts
    }

    /// Return this proof subject's scheduling priority for one successor head.
    pub(crate) fn successor_scoped_patch_evaluation_priority(
        &self,
        successor_head: &GitObjectId,
    ) -> ScopedPatchEvaluationPriority {
        self.successor_scoped_patch_target_evaluation_schedule
            .priority(self.integration_proof_subject, successor_head)
    }

    /// Return the reservation's owning actor.
    pub(crate) const fn actor(&self) -> &JournalActor { &self.actor }

    /// Borrow the normalized scopes this reservation currently protects.
    pub(crate) const fn scopes(&self) -> &ReservationScopeSet { &self.scopes }

    /// Borrow the external provenance recorded when this reservation was claimed.
    pub(crate) const fn source(&self) -> &ClaimSource { &self.source }

    /// Borrow the caller's explanation of the work this reservation protects.
    pub(crate) const fn purpose(&self) -> &ReservationPurpose { &self.purpose }

    /// Return the canonical root last validated for the owning worktree.
    pub(crate) const fn worktree_root(&self) -> &CanonicalWorktreeRoot { &self.worktree_root }

    /// Return the common-directory-relative administrative locator recorded for the holder.
    pub(crate) const fn worktree_locator(&self) -> &WorktreeAdministrativeLocator {
        &self.worktree_locator
    }

    /// Return the reservation's progress state.
    pub(crate) const fn lifecycle(&self) -> &ReservationLifecycle { &self.lifecycle }

    /// Return the branch or detached commit recorded at acquisition.
    pub(crate) const fn head_snapshot(&self) -> &ClaimHeadSnapshot { &self.head_snapshot }

    /// Return the protected commit used as this active phase's drift baseline.
    pub(crate) const fn phase_start_head(&self) -> &ProtectedPhaseStartHead {
        &self.phase_start_head
    }

    /// Compute whether this reservation currently blocks foreign edits.
    pub(crate) const fn edit_blocking_status(&self) -> EditBlockingStatus {
        match self.lifecycle {
            ReservationLifecycle::Active => EditBlockingStatus::Blocking,
            ReservationLifecycle::Outstanding { .. } => {
                self.integration_status.edit_blocking_status()
            },
            ReservationLifecycle::Released { .. } => EditBlockingStatus::Clear,
        }
    }

    /// Classify freshness from owner activity events, never unrelated journal traffic.
    pub(crate) fn freshness(&self, observed_at: &RecordedAt) -> ReservationFreshness {
        const STALE_AFTER: Duration = Duration::from_hours(24);
        if self.last_activity_at.elapsed_until(observed_at) > STALE_AFTER {
            ReservationFreshness::Stale {
                last_activity_at: self.last_activity_at.clone(),
            }
        } else {
            ReservationFreshness::Fresh {
                last_activity_at: self.last_activity_at.clone(),
            }
        }
    }

    /// Return state-specific evidence without an optional protected tip.
    pub(crate) fn evidence_state(
        &self,
    ) -> Result<ReservationEvidenceState, ReservationReplayError> {
        match &self.lifecycle {
            ReservationLifecycle::Active => {
                let IntegrationTrunkSnapshot::AtClaim(trunk_at_claim) =
                    &self.integration_trunk_snapshot
                else {
                    return Err(ReservationReplayError::SnapshotStateMismatch(self.id));
                };
                Ok(ReservationEvidenceState::Active {
                    trunk_at_claim: trunk_at_claim.clone(),
                })
            },
            ReservationLifecycle::Outstanding { .. } => {
                let (protected_tip, trunk_snapshot) = self.checkpoint_evidence()?;
                Ok(ReservationEvidenceState::Outstanding {
                    protected_tip,
                    trunk_snapshot,
                    integration_status: self.integration_status.clone(),
                })
            },
            ReservationLifecycle::Released { disposition } => match &self.retained_protected_tip {
                RetainedProtectedTip::Retained(_) => {
                    let (protected_tip, trunk_snapshot) = self.checkpoint_evidence()?;
                    Ok(ReservationEvidenceState::Released {
                        protected_tip,
                        trunk_snapshot,
                        disposition: disposition.clone(),
                        integration_status: self.integration_status.clone(),
                    })
                },
                RetainedProtectedTip::NotCheckpointed
                    if matches!(
                        disposition,
                        ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_)
                    ) =>
                {
                    Ok(ReservationEvidenceState::ReleasedWithoutCheckpoint {
                        disposition: disposition.clone(),
                    })
                },
                RetainedProtectedTip::NotCheckpointed => {
                    Err(ReservationReplayError::MissingProtectedTip(self.id))
                },
            },
        }
    }

    fn checkpoint_evidence(
        &self,
    ) -> Result<(ProtectedReservationTip, GitObjectId), ReservationReplayError> {
        let RetainedProtectedTip::Retained(protected_tip) = &self.retained_protected_tip else {
            return Err(ReservationReplayError::MissingProtectedTip(self.id));
        };
        let IntegrationTrunkSnapshot::AtCheckpoint(trunk_snapshot) =
            &self.integration_trunk_snapshot
        else {
            return Err(ReservationReplayError::MissingTrunkSnapshot(self.id));
        };
        Ok((protected_tip.clone(), trunk_snapshot.clone()))
    }
}

impl RetainedReservationSet {
    /// Count reservations that have not received a terminal disposition.
    pub(crate) fn nonterminal_count(&self) -> usize {
        self.reservations
            .iter()
            .filter(|reservation| {
                !matches!(reservation.lifecycle, ReservationLifecycle::Released { .. })
            })
            .count()
    }
}

impl ReservationConflict {
    /// Return a compact display label for the holder's branch state.
    pub(crate) fn holder_branch(&self) -> String {
        match &self.head_snapshot {
            ClaimHeadSnapshot::Branch { full_ref, .. } => full_ref.to_string(),
            ClaimHeadSnapshot::Detached { head } => format!("detached at {}", head.as_ref()),
        }
    }
}

impl IncursionIncident {
    /// Return the incident's durable identity.
    pub(crate) const fn id(&self) -> IncursionIncidentId { self.id }

    /// Return the reservation whose worktree entered foreign scopes.
    pub(crate) const fn reservation_id(&self) -> ReservationId { self.reservation_id }

    /// Borrow the foreign reservations entered by this incident.
    pub(crate) const fn foreign_reservation_ids(&self) -> &ForeignReservationIdSet {
        &self.foreign_reservation_ids
    }

    /// Borrow the repository paths entered by this incident.
    pub(crate) const fn paths(&self) -> &IncursionPathSet { &self.paths }

    /// Return the incident's current replayed disposition.
    pub(crate) const fn status(&self) -> &IncursionIncidentStatus { &self.status }
}

/// A journal sequence that cannot represent valid reservation state.
#[derive(Debug)]
pub(crate) enum ReservationReplayError {
    /// Two claims reused one non-recyclable reservation identity.
    DuplicateClaim(ReservationId),
    /// Two incursion records reused one non-recyclable incident identity.
    DuplicateIncursionIncident(IncursionIncidentId),
    /// A replayed mutation referenced no retained reservation.
    UnknownReservation(ReservationId),
    /// A replayed disposition referenced no retained incursion incident.
    UnknownIncursionIncident(IncursionIncidentId),
    /// More than one disposition attempted to answer the same incursion.
    IncursionIncidentAlreadyResolved(IncursionIncidentId),
    /// A replayed widen somehow produced an empty scope set.
    EmptyScopeSet(ReservationId),
    /// A widen operation named a reservation that was no longer active.
    WidenRequiresUnreleased(ReservationId),
    /// A reservation revision counter can no longer advance.
    RevisionExhausted(ReservationId),
    /// An integration-proof subject revision counter can no longer advance.
    IntegrationProofSubjectRevisionExhausted(ReservationId),
    /// A lifecycle transition appeared in an invalid order.
    InvalidLifecycleTransition(ReservationId, LifecycleTransitionError),
    /// A snapshot variant disagreed with the reservation lifecycle.
    SnapshotStateMismatch(ReservationId),
    /// An ordinary integrated disposition lacked a preceding verified status.
    IntegratedReleaseWithoutEvidence(ReservationId),
    /// Git evidence was materialized for an active reservation.
    ActiveEvidenceRevalidation(ReservationId),
    /// A scoped patch comparison was recorded for an active reservation.
    ActiveScopedPatchComparison(ReservationId),
    /// A scoped patch verdict named a stale proof subject revision.
    IntegrationProofSubjectMismatch(ReservationId),
    /// A user decision that has no git subject received an evidence event.
    DecisionHasNoGitEvidence(ReservationId),
    /// A checkpointed or released reservation lost its protected tip during replay.
    MissingProtectedTip(ReservationId),
    /// An outstanding reservation lost its trunk comparison point during replay.
    MissingTrunkSnapshot(ReservationId),
    /// A relocation record disagreed with the holder identity or previous root.
    WorktreeRelocationMismatch(ReservationId),
    /// A rebinding record disagreed with the worktree that currently owns the reservation.
    WorktreeRebindingMismatch(ReservationId),
    /// A replacement record named a disposition other than rewritten integration.
    InvalidReplacementDisposition(ReservationId),
}

impl Display for ReservationReplayError {
    #[expect(
        clippy::too_many_lines,
        reason = "one exhaustive display match keeps every hard-stop replay diagnostic visible"
    )]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateClaim(reservation_id) => {
                write!(
                    formatter,
                    "duplicate claim for reservation {reservation_id}"
                )
            },
            Self::DuplicateIncursionIncident(incident_id) => {
                write!(formatter, "duplicate incursion incident {incident_id}")
            },
            Self::UnknownReservation(reservation_id) => {
                write!(
                    formatter,
                    "journal operation names unknown reservation {reservation_id}"
                )
            },
            Self::UnknownIncursionIncident(incident_id) => {
                write!(
                    formatter,
                    "journal operation names unknown incursion {incident_id}"
                )
            },
            Self::IncursionIncidentAlreadyResolved(incident_id) => {
                write!(
                    formatter,
                    "incursion incident {incident_id} is already resolved"
                )
            },
            Self::EmptyScopeSet(reservation_id) => {
                write!(
                    formatter,
                    "reservation {reservation_id} replayed with no scopes"
                )
            },
            Self::WidenRequiresUnreleased(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} cannot widen after release"
            ),
            Self::RevisionExhausted(reservation_id) => {
                write!(
                    formatter,
                    "reservation {reservation_id} revision is exhausted"
                )
            },
            Self::IntegrationProofSubjectRevisionExhausted(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} integration-proof subject revision is exhausted"
            ),
            Self::InvalidLifecycleTransition(reservation_id, error) => {
                write!(
                    formatter,
                    "reservation {reservation_id} lifecycle transition failed: {error}"
                )
            },
            Self::SnapshotStateMismatch(reservation_id) => {
                write!(
                    formatter,
                    "reservation {reservation_id} has a mismatched resnapshot"
                )
            },
            Self::IntegratedReleaseWithoutEvidence(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} was released as integrated without verified evidence"
            ),
            Self::ActiveEvidenceRevalidation(reservation_id) => write!(
                formatter,
                "active reservation {reservation_id} cannot have integration evidence"
            ),
            Self::ActiveScopedPatchComparison(reservation_id) => write!(
                formatter,
                "active reservation {reservation_id} cannot have a scoped patch comparison"
            ),
            Self::IntegrationProofSubjectMismatch(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} has a mismatched integration-proof subject"
            ),
            Self::DecisionHasNoGitEvidence(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} has no git evidence to revalidate"
            ),
            Self::MissingProtectedTip(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} is missing its protected tip"
            ),
            Self::MissingTrunkSnapshot(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} is missing its checkpoint trunk snapshot"
            ),
            Self::WorktreeRelocationMismatch(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} has a mismatched worktree relocation"
            ),
            Self::WorktreeRebindingMismatch(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} has a mismatched worktree rebinding"
            ),
            Self::InvalidReplacementDisposition(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} has an invalid replacement disposition"
            ),
        }
    }
}

impl std::error::Error for ReservationReplayError {}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use serde_json::json;

    use super::AuthorizedEditingIdentity;
    use super::DriftBlockingCoverage;
    use super::DurableScopedPatchComparison;
    use super::IncursionIncidentStatus;
    use super::IncursionObservation;
    use super::IntegrationEvidenceStatus;
    use super::IntegrationProofSubjectRevision;
    use super::ReservationEvidenceState;
    use super::RetainedReservationSet;
    use super::ScopedPatchEvaluationPriority;
    use super::ScopedPatchTargetVerdictAvailability;
    use super::SuccessorScopedPatchTargetEvaluationSchedule;
    use super::constants::SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT;
    use super::lifecycle::EditBlockingStatus;
    use crate::ids::CoordinationRunId;
    use crate::ids::GitObjectId;
    use crate::ids::ProjectionGeneration;
    use crate::ids::ReservationId;
    use crate::ids::ReservationScopePath;
    use crate::ids::WorktreeId;
    use crate::ledger::IncursionIncidentId;
    use crate::ledger::IncursionPathSet;
    use crate::ledger::JournalEvent;
    use crate::scope::PathCase;
    use crate::scope::ReservationScopeSet;
    use crate::scope::ScopeKind;

    const FOREIGN_RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a22";
    const INCIDENT_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a23";
    const PROTECTED_TIP: &str = "2222222222222222222222222222222222222222";
    const REPLACEMENT_TIP: &str = "3333333333333333333333333333333333333333";
    const RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f";
    const SECOND_RUN_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a20";
    const SECOND_TRUNK_OID: &str = "4444444444444444444444444444444444444444";
    const THIRD_TRUNK_OID: &str = "5555555555555555555555555555555555555555";
    const TRUNK_OID: &str = "1111111111111111111111111111111111111111";
    const WORKTREE_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";
    const SECOND_WORKTREE_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a21";

    #[test]
    fn replay_retains_active_outstanding_released_and_rewritten_states()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let [
            claim,
            checkpoint,
            integrated,
            release,
            rewritten,
            resnapshot,
        ] = lifecycle_events()?;

        let active = RetainedReservationSet::replay(std::slice::from_ref(&claim))?;
        assert!(matches!(
            active
                .reservation(reservation_id)
                .and_then(super::Reservation::evidence_state),
            Ok(ReservationEvidenceState::Active { .. })
        ));
        let outstanding = RetainedReservationSet::replay(&[claim.clone(), checkpoint.clone()])?;
        assert!(matches!(
            outstanding
                .reservation(reservation_id)
                .and_then(super::Reservation::evidence_state),
            Ok(ReservationEvidenceState::Outstanding { .. })
        ));
        let released = RetainedReservationSet::replay(&[
            claim.clone(),
            checkpoint.clone(),
            integrated.clone(),
            release.clone(),
        ])?;
        assert!(matches!(
            released
                .reservation(reservation_id)
                .and_then(super::Reservation::evidence_state),
            Ok(ReservationEvidenceState::Released {
                integration_status: IntegrationEvidenceStatus::Integrated { .. },
                ..
            })
        ));
        let lost_evidence = RetainedReservationSet::replay(&[
            claim.clone(),
            checkpoint.clone(),
            integrated.clone(),
            release.clone(),
            rewritten.clone(),
        ])?;
        assert!(matches!(
            lost_evidence
                .reservation(reservation_id)
                .and_then(super::Reservation::evidence_state),
            Ok(ReservationEvidenceState::Released {
                integration_status: IntegrationEvidenceStatus::TrunkRewritten,
                ..
            })
        ));
        assert_eq!(
            lost_evidence
                .reservation(reservation_id)?
                .edit_blocking_status(),
            EditBlockingStatus::Clear
        );
        let legacy_resnapshot = RetainedReservationSet::replay(&[
            claim, checkpoint, integrated, release, rewritten, resnapshot,
        ])?;
        assert!(matches!(
            legacy_resnapshot
                .reservation(reservation_id)
                .and_then(super::Reservation::evidence_state),
            Ok(ReservationEvidenceState::Released {
                protected_tip,
                integration_status: IntegrationEvidenceStatus::TrunkRewritten,
                ..
            }) if protected_tip.to_string() == PROTECTED_TIP
        ));
        Ok(())
    }

    #[test]
    fn replay_ignores_a_journaled_blocking_status_after_release()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let [
            claim,
            checkpoint,
            integrated,
            release,
            recorded_blocking_evidence,
            _,
        ] = lifecycle_events()?;

        let retained_reservations = RetainedReservationSet::replay(&[
            claim,
            checkpoint,
            integrated,
            release,
            recorded_blocking_evidence,
        ])?;

        assert_eq!(
            retained_reservations
                .reservation(reservation_id)?
                .edit_blocking_status(),
            EditBlockingStatus::Clear
        );
        Ok(())
    }

    #[test]
    fn replay_rejects_widen_after_release() -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let [claim, checkpoint, integrated, release, ..] = lifecycle_events()?;
        let widen = journal_event(
            5,
            &json!({
                "op": "widen",
                "reservation_id": RESERVATION_ID,
                "added_scopes": [{"path": "added.rs", "kind": "file"}],
                "cause": {"kind": "explicit", "reason": "legacy invalid sequence"},
                "authorization": {"kind": "no_conflict"},
                "edit_blocking_status": "blocking"
            }),
        )?;

        let Err(error) =
            RetainedReservationSet::replay(&[claim, checkpoint, integrated, release, widen])
        else {
            return Err(
                std::io::Error::other("release followed by widen should be rejected").into(),
            );
        };
        assert!(matches!(
            error,
            super::ReservationReplayError::WidenRequiresUnreleased(candidate)
                if candidate == reservation_id
        ));
        Ok(())
    }

    #[test]
    fn widening_outstanding_scopes_invalidates_scoped_patch_integration_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let [claim, checkpoint, ..] = lifecycle_events()?;
        let integrated = journal_event(
            3,
            &json!({
                "op": "evidence_revalidated",
                "reservation_id": RESERVATION_ID,
                "status": {
                    "status": "integrated",
                    "trunk_oid": TRUNK_OID,
                    "proof": "scoped_patch_equivalent"
                },
                "edit_blocking_status": "clear"
            }),
        )?;
        let widen = journal_event(
            4,
            &json!({
                "op": "widen",
                "reservation_id": RESERVATION_ID,
                "added_scopes": [{"path": "added.rs", "kind": "file"}],
                "cause": {"kind": "explicit", "reason": "reviewed scope expansion"},
                "authorization": {"kind": "no_conflict"},
                "edit_blocking_status": "clear"
            }),
        )?;

        let retained_reservations =
            RetainedReservationSet::replay(&[claim, checkpoint, integrated, widen])?;
        let reservation = retained_reservations.reservation(reservation_id)?;

        assert!(matches!(
            reservation.evidence_state(),
            Ok(ReservationEvidenceState::Outstanding {
                integration_status: IntegrationEvidenceStatus::NotIntegrated,
                ..
            })
        ));
        assert_eq!(
            reservation.edit_blocking_status(),
            EditBlockingStatus::Blocking
        );
        assert!(
            reservation
                .scopes()
                .as_slice()
                .iter()
                .any(|scope| scope.path.to_string() == "added.rs")
        );
        Ok(())
    }

    #[test]
    fn replay_retains_positive_and_negative_scoped_patch_verdicts()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let target = TRUNK_OID.parse::<GitObjectId>()?;
        let [claim, checkpoint, ..] = lifecycle_events()?;

        for (verdict_name, expected) in [
            ("integrated", DurableScopedPatchComparison::Equivalent),
            ("not_integrated", DurableScopedPatchComparison::Different),
            ("trunk_rewritten", DurableScopedPatchComparison::Different),
        ] {
            let checked = scoped_patch_equivalence_checked(3, 1, verdict_name)?;
            let reservations =
                RetainedReservationSet::replay(&[claim.clone(), checkpoint.clone(), checked])?;
            let reservation = reservations.reservation(reservation_id)?;

            assert_eq!(
                reservation
                    .retained_scoped_patch_target_verdicts()
                    .lookup(reservation.integration_proof_subject_revision(), &target,),
                ScopedPatchTargetVerdictAvailability::Hit(expected)
            );
        }
        Ok(())
    }

    #[test]
    fn retained_scoped_patch_verdicts_keep_two_targets_and_evict_the_oldest()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let first_target = TRUNK_OID.parse::<GitObjectId>()?;
        let second_target = SECOND_TRUNK_OID.parse::<GitObjectId>()?;
        let third_target = THIRD_TRUNK_OID.parse::<GitObjectId>()?;
        let [claim, checkpoint, ..] = lifecycle_events()?;
        let first = scoped_patch_equivalence_checked_at(3, TRUNK_OID, "integrated")?;
        let second = scoped_patch_equivalence_checked_at(4, SECOND_TRUNK_OID, "not_integrated")?;
        let first_two = RetainedReservationSet::replay(&[
            claim.clone(),
            checkpoint.clone(),
            first.clone(),
            second.clone(),
        ])?;
        let reservation = first_two.reservation(reservation_id)?;

        assert_eq!(
            reservation.retained_scoped_patch_target_verdicts().lookup(
                reservation.integration_proof_subject_revision(),
                &first_target
            ),
            ScopedPatchTargetVerdictAvailability::Hit(DurableScopedPatchComparison::Equivalent)
        );
        assert_eq!(
            reservation.retained_scoped_patch_target_verdicts().lookup(
                reservation.integration_proof_subject_revision(),
                &second_target,
            ),
            ScopedPatchTargetVerdictAvailability::Hit(DurableScopedPatchComparison::Different)
        );

        let third = scoped_patch_equivalence_checked_at(5, THIRD_TRUNK_OID, "integrated")?;
        let retained = RetainedReservationSet::replay(&[claim, checkpoint, first, second, third])?;
        let reservation = retained.reservation(reservation_id)?;

        assert_no_retained_scoped_patch_target_verdict(reservation, &first_target);
        assert_eq!(
            reservation.retained_scoped_patch_target_verdicts().lookup(
                reservation.integration_proof_subject_revision(),
                &second_target,
            ),
            ScopedPatchTargetVerdictAvailability::Hit(DurableScopedPatchComparison::Different)
        );
        assert_eq!(
            reservation.retained_scoped_patch_target_verdicts().lookup(
                reservation.integration_proof_subject_revision(),
                &third_target,
            ),
            ScopedPatchTargetVerdictAvailability::Hit(DurableScopedPatchComparison::Equivalent)
        );
        assert_eq!(
            reservation.scoped_patch_evaluation_priority(&first_target),
            ScopedPatchEvaluationPriority::NotAttempted
        );
        assert_eq!(
            reservation.scoped_patch_evaluation_priority(&second_target),
            ScopedPatchEvaluationPriority::LastAttemptedAt(ProjectionGeneration::from(4))
        );
        assert_eq!(
            reservation.scoped_patch_evaluation_priority(&third_target),
            ScopedPatchEvaluationPriority::LastAttemptedAt(ProjectionGeneration::from(5))
        );
        Ok(())
    }

    #[test]
    fn successor_scoped_patch_schedule_retains_only_bounded_current_subject_heads()
    -> Result<(), Box<dyn std::error::Error>> {
        let superseded_subject = IntegrationProofSubjectRevision::INITIAL;
        let current_subject = IntegrationProofSubjectRevision(2);
        let superseded_head = TRUNK_OID.parse::<GitObjectId>()?;
        let generation = ProjectionGeneration::from(3);
        let mut evaluation_schedule = SuccessorScopedPatchTargetEvaluationSchedule::default();
        evaluation_schedule.record(superseded_subject, &superseded_head, generation);

        for successor_number in 1..=SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT + 1 {
            let successor_head = format!("{successor_number:040x}").parse::<GitObjectId>()?;
            evaluation_schedule.record(current_subject, &successor_head, generation);
        }

        let evicted_head = format!("{:040x}", 1).parse::<GitObjectId>()?;
        let oldest_retained_head = format!("{:040x}", 2).parse::<GitObjectId>()?;
        assert_eq!(
            evaluation_schedule.entries.len(),
            SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT
        );
        assert_eq!(
            evaluation_schedule.priority(superseded_subject, &superseded_head),
            ScopedPatchEvaluationPriority::NotAttempted
        );
        assert_eq!(
            evaluation_schedule.priority(current_subject, &evicted_head),
            ScopedPatchEvaluationPriority::NotAttempted
        );
        assert_eq!(
            evaluation_schedule.priority(current_subject, &oldest_retained_head),
            ScopedPatchEvaluationPriority::LastAttemptedAt(generation)
        );
        Ok(())
    }

    #[test]
    fn proof_subject_changes_remove_retained_verdicts_after_widen_resnapshot_and_replacement()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let target = TRUNK_OID.parse::<GitObjectId>()?;
        let [claim, checkpoint, integrated, release, ..] = lifecycle_events()?;
        let checked = scoped_patch_equivalence_checked(3, 1, "integrated")?;
        let widen = journal_event(
            4,
            &json!({
                "op": "widen",
                "reservation_id": RESERVATION_ID,
                "added_scopes": [{"path": "added.rs", "kind": "file"}],
                "cause": {"kind": "explicit", "reason": "reviewed scope expansion"},
                "authorization": {"kind": "no_conflict"},
                "edit_blocking_status": "blocking"
            }),
        )?;
        let widened = RetainedReservationSet::replay(&[
            claim.clone(),
            checkpoint.clone(),
            checked.clone(),
            widen,
        ])?;
        assert_no_retained_scoped_patch_target_verdict(
            widened.reservation(reservation_id)?,
            &target,
        );

        let resnapshot = journal_event(
            4,
            &json!({
                "op": "resnapshot",
                "reservation_id": RESERVATION_ID,
                "snapshot": {
                    "stage": "outstanding",
                    "protected_tip": REPLACEMENT_TIP,
                    "trunk_oid": TRUNK_OID
                }
            }),
        )?;
        let resnapshotted = RetainedReservationSet::replay(&[
            claim.clone(),
            checkpoint.clone(),
            checked.clone(),
            resnapshot,
        ])?;
        assert_no_retained_scoped_patch_target_verdict(
            resnapshotted.reservation(reservation_id)?,
            &target,
        );

        let replacement = journal_event(
            6,
            &json!({
                "op": "replace_release_disposition",
                "reservation_id": RESERVATION_ID,
                "superseded": {"kind": "integrated"},
                "replacement": {
                    "kind": "rewritten_integration",
                    "evidence": REPLACEMENT_TIP
                }
            }),
        )?;
        let replaced = RetainedReservationSet::replay(&[
            claim,
            checkpoint,
            integrated,
            release,
            checked,
            replacement,
        ])?;
        assert_no_retained_scoped_patch_target_verdict(
            replaced.reservation(reservation_id)?,
            &target,
        );
        Ok(())
    }

    #[test]
    fn replay_preserves_phase_start_and_complete_widened_scope_kinds()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let [claim, ..] = lifecycle_events()?;
        let widen = journal_event(
            2,
            &json!({
                "op": "widen",
                "reservation_id": RESERVATION_ID,
                "added_scopes": [
                    {"path": "generated", "kind": "tree"},
                    {"path": "single.txt", "kind": "file"}
                ],
                "cause": {"kind": "explicit", "reason": "reviewed test expansion"},
                "authorization": {"kind": "no_conflict"},
                "edit_blocking_status": "blocking"
            }),
        )?;

        let reservations = RetainedReservationSet::replay(&[claim, widen])?;
        let reservation = reservations.reservation(reservation_id)?;
        assert_eq!(
            reservation.phase_start_head().as_ref().to_string(),
            TRUNK_OID
        );
        let Some(generated) = reservation
            .scopes()
            .as_slice()
            .iter()
            .find(|scope| scope.path.to_string() == "generated")
        else {
            return Err(std::io::Error::other("widened tree should replay").into());
        };
        let Some(single) = reservation
            .scopes()
            .as_slice()
            .iter()
            .find(|scope| scope.path.to_string() == "single.txt")
        else {
            return Err(std::io::Error::other("widened file should replay").into());
        };
        assert_eq!(generated.kind, ScopeKind::Tree);
        assert_eq!(single.kind, ScopeKind::File);
        assert!(generated.contains(
            &crate::scope::ReservationScope {
                path: "generated/child.rs".parse()?,
                kind: ScopeKind::File,
            },
            PathCase::Sensitive,
        ));
        assert!(!single.contains(
            &crate::scope::ReservationScope {
                path: "single.txt/child".parse()?,
                kind: ScopeKind::File,
            },
            PathCase::Sensitive,
        ));
        assert_eq!(
            reservation.edit_blocking_status(),
            EditBlockingStatus::Blocking
        );
        Ok(())
    }

    #[test]
    fn drift_blocking_coverage_follows_worktree_identity_and_ignores_the_run()
    -> Result<(), Box<dyn std::error::Error>> {
        let [claim, ..] = lifecycle_events()?;
        let reservations = RetainedReservationSet::replay(&[claim])?;
        let candidate = serde_json::from_value::<ReservationScopeSet>(json!([
            {"path": "src/lib.rs", "kind": "file"}
        ]))?;
        let worktree_id = WORKTREE_ID.parse::<WorktreeId>()?;
        let second_worktree_id = SECOND_WORKTREE_ID.parse::<WorktreeId>()?;

        assert!(matches!(
            reservations.blocking_coverage_for_drift(&candidate, worktree_id, PathCase::Sensitive),
            DriftBlockingCoverage::SameIdentity
        ));
        let DriftBlockingCoverage::Foreign(different_worktree) = reservations
            .blocking_coverage_for_drift(&candidate, second_worktree_id, PathCase::Sensitive)
        else {
            return Err(std::io::Error::other("another worktree should be foreign").into());
        };
        assert_eq!(
            different_worktree[0].reservation_id.to_string(),
            RESERVATION_ID
        );
        Ok(())
    }

    #[test]
    fn a_released_reservation_with_lost_evidence_never_blocks_edits()
    -> Result<(), Box<dyn std::error::Error>> {
        let [claim, checkpoint, integrated, release, rewritten, _] = lifecycle_events()?;
        let reservations =
            RetainedReservationSet::replay(&[claim, checkpoint, integrated, release, rewritten])?;
        let candidate = serde_json::from_value::<ReservationScopeSet>(json!([
            {"path": "src/lib.rs", "kind": "file"}
        ]))?;
        let second_run_id = SECOND_RUN_ID.parse::<CoordinationRunId>()?;
        let worktree_id = WORKTREE_ID.parse::<WorktreeId>()?;
        let second_worktree_id = SECOND_WORKTREE_ID.parse::<WorktreeId>()?;
        assert_eq!(
            reservations
                .reservation(RESERVATION_ID.parse::<ReservationId>()?)?
                .edit_blocking_status(),
            EditBlockingStatus::Clear
        );

        assert!(
            reservations
                .conflicts_for_first_touch(
                    &candidate,
                    second_run_id,
                    worktree_id,
                    PathCase::Sensitive,
                )
                .is_empty()
        );
        assert!(
            reservations
                .conflicts_for_claim(&candidate, worktree_id, PathCase::Sensitive)
                .is_empty()
        );
        let foreign_first_touch = reservations.conflicts_for_first_touch(
            &candidate,
            second_run_id,
            second_worktree_id,
            PathCase::Sensitive,
        );
        assert!(foreign_first_touch.is_empty());
        let foreign_claim =
            reservations.conflicts_for_claim(&candidate, second_worktree_id, PathCase::Sensitive);
        assert!(foreign_claim.is_empty());
        Ok(())
    }

    /// A worktree must never block itself with an active reservation it holds.
    ///
    /// An active holder blocks foreign worktrees, but a later coordination run in the same
    /// checkout remains the holder's identity. Deciding foreignness by run once made the
    /// worktree foreign to itself and offered an overlap negotiation where there was only
    /// one party.
    #[test]
    fn an_active_reservation_blocks_another_worktree_and_never_its_own()
    -> Result<(), Box<dyn std::error::Error>> {
        let [claim, ..] = lifecycle_events()?;
        let reservations = RetainedReservationSet::replay(&[claim])?;
        let candidate = serde_json::from_value::<ReservationScopeSet>(json!([
            {"path": "src/lib.rs", "kind": "file"}
        ]))?;
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let second_run_id = SECOND_RUN_ID.parse::<CoordinationRunId>()?;
        let worktree_id = WORKTREE_ID.parse::<WorktreeId>()?;
        let second_worktree_id = SECOND_WORKTREE_ID.parse::<WorktreeId>()?;
        let active_holder = reservations.reservation(reservation_id)?;
        assert_eq!(
            active_holder.edit_blocking_status(),
            EditBlockingStatus::Blocking
        );
        assert!(
            AuthorizedEditingIdentity::Run {
                coordination_run_id: second_run_id,
                worktree_id,
            }
            .identifies_requester(active_holder)
        );
        assert!(
            AuthorizedEditingIdentity::SessionReservation {
                coordination_run_id: second_run_id,
                reservation_id,
                worktree_id,
            }
            .identifies_requester(active_holder)
        );

        assert!(
            reservations
                .conflicts_for_first_touch(
                    &candidate,
                    second_run_id,
                    worktree_id,
                    PathCase::Sensitive,
                )
                .is_empty()
        );
        assert!(
            reservations
                .conflicts_for_claim(&candidate, worktree_id, PathCase::Sensitive)
                .is_empty()
        );
        let foreign_first_touch = reservations.conflicts_for_first_touch(
            &candidate,
            second_run_id,
            second_worktree_id,
            PathCase::Sensitive,
        );
        assert_eq!(
            foreign_first_touch
                .iter()
                .map(|conflict| conflict.reservation_id)
                .collect::<Vec<_>>(),
            [reservation_id]
        );
        let foreign_claim =
            reservations.conflicts_for_claim(&candidate, second_worktree_id, PathCase::Sensitive);
        assert_eq!(
            foreign_claim
                .iter()
                .map(|conflict| conflict.reservation_id)
                .collect::<Vec<_>>(),
            [reservation_id]
        );
        Ok(())
    }

    #[test]
    fn an_answered_incursion_is_never_raised_again_for_the_same_overlap()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let [claim, ..] = lifecycle_events()?;
        let incursion = journal_event(
            2,
            &json!({
                "op": "incursion",
                "incident_id": INCIDENT_ID,
                "reservation_id": RESERVATION_ID,
                "foreign_reservation_ids": [FOREIGN_RESERVATION_ID],
                "paths": ["src/lib.rs"],
            }),
        )?;
        let resolution = journal_event(
            3,
            &json!({"op": "resolve_incursion", "incident_id": INCIDENT_ID}),
        )?;
        let resolving_actor = resolution.actor.clone();

        let outstanding = RetainedReservationSet::replay(&[claim.clone(), incursion.clone()])?;
        let incident = outstanding
            .incursion_incidents()
            .next()
            .ok_or("replay should retain the incursion")?;
        let foreign_reservation_ids = incident.foreign_reservation_ids().clone();
        let paths = incident.paths().clone();
        assert!(matches!(
            outstanding.observe_incursion(reservation_id, &foreign_reservation_ids, &paths),
            IncursionObservation::AlreadyOutstanding { .. }
        ));

        let answered = RetainedReservationSet::replay(&[claim, incursion, resolution])?;
        let answered_incident =
            answered.incursion_incident(INCIDENT_ID.parse::<IncursionIncidentId>()?)?;
        let IncursionIncidentStatus::Resolved {
            resolving_actor: replayed_resolving_actor,
            ..
        } = answered_incident.status()
        else {
            return Err("the resolution event should answer the incident".into());
        };
        assert_eq!(replayed_resolving_actor, &resolving_actor);
        assert!(
            matches!(
                answered.observe_incursion(reservation_id, &foreign_reservation_ids, &paths),
                IncursionObservation::AlreadyAnswered
            ),
            "the straying edit stays on disk, so a fresh incident could never be cleared"
        );
        Ok(())
    }

    /// A second entered path must not re-raise the first one alongside it.
    #[test]
    fn an_incursion_adding_one_path_creates_an_incident_for_that_path_alone()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let [claim, ..] = lifecycle_events()?;
        let incursion = journal_event(
            2,
            &json!({
                "op": "incursion",
                "incident_id": INCIDENT_ID,
                "reservation_id": RESERVATION_ID,
                "foreign_reservation_ids": [FOREIGN_RESERVATION_ID],
                "paths": ["src/lib.rs"],
            }),
        )?;
        let outstanding = RetainedReservationSet::replay(&[claim, incursion])?;
        let incident = outstanding
            .incursion_incidents()
            .next()
            .ok_or("replay should retain the incursion")?;
        let foreign_reservation_ids = incident.foreign_reservation_ids().clone();
        let widened = IncursionPathSet::try_from(vec![
            "src/lib.rs".parse::<ReservationScopePath>()?,
            "src/other.rs".parse::<ReservationScopePath>()?,
        ])?;

        let observation =
            outstanding.observe_incursion(reservation_id, &foreign_reservation_ids, &widened);
        let IncursionObservation::NewlyObserved { paths, .. } = observation else {
            return Err("a genuinely new path must still raise an incident".into());
        };
        assert_eq!(
            paths
                .as_slice()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["src/other.rs".to_owned()],
            "the outstanding incident already covers src/lib.rs, so it must not be re-raised"
        );
        Ok(())
    }

    fn assert_no_retained_scoped_patch_target_verdict(
        reservation: &super::Reservation,
        target: &GitObjectId,
    ) {
        assert_eq!(
            reservation
                .retained_scoped_patch_target_verdicts()
                .lookup(reservation.integration_proof_subject_revision(), target,),
            ScopedPatchTargetVerdictAvailability::Miss
        );
    }

    fn scoped_patch_equivalence_checked(
        projection_generation: u64,
        subject: u64,
        verdict: &str,
    ) -> Result<JournalEvent, serde_json::Error> {
        journal_event(
            projection_generation,
            &json!({
                "op": "scoped_patch_equivalence_checked",
                "reservation_id": RESERVATION_ID,
                "subject": subject,
                "target": TRUNK_OID,
                "verdict": verdict
            }),
        )
    }

    fn scoped_patch_equivalence_checked_at(
        projection_generation: u64,
        target: &str,
        verdict: &str,
    ) -> Result<JournalEvent, serde_json::Error> {
        journal_event(
            projection_generation,
            &json!({
                "op": "scoped_patch_equivalence_checked",
                "reservation_id": RESERVATION_ID,
                "subject": 1,
                "target": target,
                "verdict": verdict
            }),
        )
    }

    fn lifecycle_events() -> Result<[JournalEvent; 6], serde_json::Error> {
        let claim = journal_event(
            1,
            &json!({
                "op": "claim",
                "reservation_id": RESERVATION_ID,
                "scopes": [{"path": "src", "kind": "tree"}],
                "source": {"kind": "explicit"},
                "purpose": {"kind": "not_provided_by_caller"},
                "trunk_at_claim": TRUNK_OID,
                "head_snapshot": {"kind": "branch", "full_ref": "refs/heads/phase", "head": PROTECTED_TIP},
                "phase_start_head": TRUNK_OID,
                "worktree_root": "/repo",
                "worktree_administrative_locator": ".",
                "authorization": {"kind": "no_conflict"},
            }),
        )?;
        let checkpoint = journal_event(
            2,
            &json!({
                "op": "checkpoint",
                "reservation_id": RESERVATION_ID,
                "protected_tip": PROTECTED_TIP,
                "trunk_snapshot": TRUNK_OID,
            }),
        )?;
        let integrated = journal_event(
            3,
            &json!({
                "op": "evidence_revalidated",
                "reservation_id": RESERVATION_ID,
                "status": {"status": "integrated", "trunk_oid": TRUNK_OID},
                "edit_blocking_status": "clear",
            }),
        )?;
        let release = journal_event(
            4,
            &json!({
                "op": "release",
                "reservation_id": RESERVATION_ID,
                "disposition": {"kind": "integrated"},
            }),
        )?;
        let rewritten = journal_event(
            5,
            &json!({
                "op": "evidence_revalidated",
                "reservation_id": RESERVATION_ID,
                "status": {"status": "trunk_rewritten"},
                "edit_blocking_status": "blocking",
            }),
        )?;
        let resnapshot = journal_event(
            6,
            &json!({
                "op": "resnapshot",
                "reservation_id": RESERVATION_ID,
                "snapshot": {
                    "stage": "outstanding",
                    "protected_tip": REPLACEMENT_TIP,
                    "trunk_oid": TRUNK_OID
                },
            }),
        )?;
        Ok([
            claim, checkpoint, integrated, release, rewritten, resnapshot,
        ])
    }

    fn journal_event(
        projection_generation: u64,
        operation: &Value,
    ) -> Result<JournalEvent, serde_json::Error> {
        let mut event = json!({
            "schema_version": 1,
            "event_id": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b",
            "actor": {
                "repository": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c",
                "worktree": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d",
                "run": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1e"
            },
            "at": "2026-08-23T17:34:54.123Z",
            "projection_generation": projection_generation,
        });
        if let (Some(event), Some(operation)) = (event.as_object_mut(), operation.as_object()) {
            event.extend(operation.clone());
        }
        serde_json::from_value(event)
    }
}
