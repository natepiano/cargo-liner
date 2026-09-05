//! One replayed reservation record and the readings taken from it.
//!
//! [`Reservation`] is the durable per-holder record replay accumulates: identity, revision,
//! scopes, provenance, lifecycle, and retained evidence. The remaining types are readings
//! taken from that record and handed to callers who must not see its fields --- freshness,
//! holder activity, evidence state, and the lifecycle snapshot the board and output contract
//! serialize.

use std::time::Duration;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::evidence::ProtectedReservationTip;
use super::lifecycle::EditBlockingStatus;
use super::lifecycle::IntegrationEvidenceStatus;
use super::lifecycle::ReleaseDisposition;
use super::lifecycle::ReservationLifecycle;
use super::replay::ReservationReplayError;
use super::retention::IntegrationTrunkSnapshot;
use super::retention::RetainedProtectedTip;
use super::scoped_patch_evaluation::IntegrationProofSubjectRevision;
use super::scoped_patch_evaluation::RetainedScopedPatchTargetVerdicts;
use super::scoped_patch_evaluation::RetainedSuccessorScopedPatchTargetVerdicts;
use super::scoped_patch_evaluation::ScopedPatchEvaluationPriority;
use super::scoped_patch_evaluation::ScopedPatchTargetEvaluationSchedule;
use super::scoped_patch_evaluation::SuccessorScopedPatchTargetEvaluationSchedule;
use crate::answer::ConflictAuthorization;
use crate::coordination_identity::CoordinationIdentityProvenance;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::ReservationRevision;
use crate::ids::WorktreeId;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::JournalActor;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReservationPurpose;
use crate::ledger::TrunkObservationAtClaim;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::scope::ReservationScopeSet;

/// One reservation retained for overlap, evidence, and audit decisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Reservation {
    pub(super) id:                                                ReservationId,
    pub(super) revision:                                          ReservationRevision,
    pub(super) integration_proof_subject:                         IntegrationProofSubjectRevision,
    pub(super) retained_scoped_patch_target_verdicts:             RetainedScopedPatchTargetVerdicts,
    pub(super) scoped_patch_target_evaluation_schedule: ScopedPatchTargetEvaluationSchedule,
    pub(super) retained_successor_scoped_patch_verdicts: RetainedSuccessorScopedPatchTargetVerdicts,
    pub(super) successor_scoped_patch_target_evaluation_schedule:
        SuccessorScopedPatchTargetEvaluationSchedule,
    pub(super) scopes:                                            ReservationScopeSet,
    pub(super) authorizations:                                    Vec<ConflictAuthorization>,
    pub(super) source:                                            ClaimSource,
    pub(super) purpose:                                           ReservationPurpose,
    pub(super) head_snapshot:                                     ClaimHeadSnapshot,
    pub(super) phase_start_head:                                  ProtectedPhaseStartHead,
    pub(super) actor:                                             JournalActor,
    pub(super) coordination_identity_provenance:                  CoordinationIdentityProvenance,
    pub(super) lifecycle:                                         ReservationLifecycle,
    pub(super) retained_protected_tip:                            RetainedProtectedTip,
    pub(super) integration_trunk_snapshot:                        IntegrationTrunkSnapshot,
    pub(super) integration_status:                                IntegrationEvidenceStatus,
    pub(super) worktree_root:                                     CanonicalWorktreeRoot,
    pub(super) worktree_locator:                                  WorktreeAdministrativeLocator,
    pub(super) claimed_at:                                        RecordedAt,
    pub(super) last_activity_at:                                  RecordedAt,
}

impl Reservation {
    /// Return the reservation's durable identity.
    pub(crate) const fn id(&self) -> ReservationId { self.id }

    pub(super) fn advance_revision(&mut self) -> Result<(), ReservationReplayError> {
        let revision: u64 = self.revision.into();
        self.revision = revision
            .checked_add(1)
            .map(ReservationRevision::from)
            .ok_or(ReservationReplayError::RevisionExhausted(self.id))?;
        Ok(())
    }

    pub(super) fn advance_integration_proof_subject_revision(
        &mut self,
    ) -> Result<(), ReservationReplayError> {
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

    /// Return whether this reservation is still in `Active`, whoever holds it.
    ///
    /// Every eligibility predicate below is this test plus identity terms, so the `Active`
    /// lifecycle test is written once for all of them. Lifecycle tests that ask a different
    /// question live on their own: [`Self::edit_blocking_status`] admits an `Outstanding`
    /// holder whose work has not reached the trunk, and eligibility never does.
    const fn is_active(&self) -> bool { matches!(self.lifecycle, ReservationLifecycle::Active) }

    /// Return whether this active reservation belongs to the named run, in any worktree.
    ///
    /// Constraining the run and not the worktree is deliberate, so a run holding live work in
    /// a second worktree still answers `true`. Callers deciding the fate of a run-scoped
    /// record want that reach; callers deciding what one worktree may edit want
    /// [`Self::is_active_for_coordination_run_and_worktree`].
    pub(crate) fn is_active_for_coordination_run(
        &self,
        coordination_run_id: CoordinationRunId,
    ) -> bool {
        self.actor.run == coordination_run_id && self.is_active()
    }

    /// Return whether this active reservation belongs to the named run and worktree.
    ///
    /// Delegates to [`Self::is_active_for_coordination_run`] and adds the worktree term, so the
    /// narrower predicate is structurally a subset of this one.
    pub(crate) fn is_active_for_coordination_run_and_worktree(
        &self,
        coordination_run_id: CoordinationRunId,
        worktree_id: WorktreeId,
    ) -> bool {
        self.is_active_for_coordination_run(coordination_run_id)
            && self.actor.worktree == worktree_id
    }

    /// Return whether this holder occupies the named worktree for some other coordination run.
    ///
    /// The worktree is the coordination unit, so one run occupies it at a time. `Active` only:
    /// an `Outstanding` holder has released and is awaiting integration, and counting it as an
    /// occupant would lock a worktree out of paths its own previous session released.
    ///
    /// Occupancy holds only between two coordination identities a caller actually presented.
    /// A holder recorded as [`CoordinationIdentityProvenance::NotPresented`] was claimed under
    /// an identity this engine created for itself --- post-commit drift first-touches that way
    /// --- and treating it as an occupant locks a checkout out against its own `--run`.
    ///
    /// Only the *holder's* provenance is read here, and the two chains that reach this
    /// predicate treat the acting side differently.
    ///
    /// The **occupancy chain** --- `coordination_identity::validate_worktree_occupancy`
    /// through `RetainedReservationSet::worktree_occupancy` --- carries its acting side in the
    /// type. Both hops take a
    /// [`PresentedCoordinationRun`](crate::coordination_identity::PresentedCoordinationRun),
    /// whose field is private and whose only two constructors name the `--run` argument and
    /// `CARGO_BERTH_RUN`, so a caller holding a bare [`CoordinationRunId`] cannot reach this
    /// question at all. `ClaimRunValidation::validate`,
    /// `DriftRunValidation::authorize_scope_acquisition`, and
    /// `check::validate_edit_worktree_occupancy` obtain theirs there, and a fourth site has no
    /// other way to obtain one --- which is why the rule is symmetric by construction rather
    /// than by a variant match each site is asked to remember.
    ///
    /// The **overlap chain** applies no acting-side term at all.
    /// [`Self::is_foreign_to_coordination_run_in_worktree`] reaches this predicate from
    /// `conflicts_for_drift`, `blocking_coverage_for_drift`, `bind_widened_scopes`, and ---
    /// through `AuthorizedEditingIdentity::is_foreign` --- `conflicts_for_edit` and
    /// `conflicts_for_first_touch`, and every hop carries a bare `CoordinationRunId` with no
    /// provenance beside it. That asymmetry is deliberate and stays: a same-worktree holder
    /// that presented an identity is foreign to any other run there, whatever that run
    /// presented.
    pub(super) fn occupies_worktree_for_another_coordination_run(
        &self,
        coordination_run_id: CoordinationRunId,
        worktree_id: WorktreeId,
    ) -> bool {
        self.actor.worktree == worktree_id
            && self.actor.run != coordination_run_id
            && self.is_active()
            && matches!(
                self.coordination_identity_provenance,
                CoordinationIdentityProvenance::Presented
            )
    }

    /// Return whether this holder is foreign to a run acting in the named worktree.
    ///
    /// Foreign is another worktree entirely, or this worktree while another run occupies it
    /// per [`Self::occupies_worktree_for_another_coordination_run`]. Both hook sites and the
    /// drift predicates ask exactly this question, so it is answered in one place.
    ///
    /// Occupancy carries its provenance term here by construction: a reservation this engine
    /// claimed under an identity it created for itself is never foreign inside its own
    /// worktree, so the hook, the drift conflict pass, and the drift coverage probe all read
    /// it as the acting identity's own work rather than someone else's.
    pub(super) fn is_foreign_to_coordination_run_in_worktree(
        &self,
        coordination_run_id: CoordinationRunId,
        worktree_id: WorktreeId,
    ) -> bool {
        self.actor.worktree != worktree_id
            || self.occupies_worktree_for_another_coordination_run(coordination_run_id, worktree_id)
    }

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
    /// Return when this reservation was claimed.
    pub(crate) const fn claimed_at(&self) -> &RecordedAt { &self.claimed_at }

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

/// Whether a holder has explicitly demonstrated recent reservation activity.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReservationFreshness {
    /// A claim, widen, renew, or checkpoint occurred inside the freshness window.
    Fresh { last_activity_at: RecordedAt },
    /// No owner activity event occurred inside the freshness window.
    Stale { last_activity_at: RecordedAt },
}

/// Whether a conflicting holder is still recording coordination activity.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum ReservationHolderActivity {
    /// The holder recorded a claim, widen, renew, or checkpoint inside the freshness window.
    Active {
        #[schemars(with = "String", length(min = 1))]
        last_activity_at: RecordedAt,
    },
    /// The holder has gone quiet beyond the freshness window.
    Quiet {
        #[schemars(with = "String", length(min = 1))]
        last_activity_at: RecordedAt,
    },
}

impl From<ReservationFreshness> for ReservationHolderActivity {
    fn from(freshness: ReservationFreshness) -> Self {
        match freshness {
            ReservationFreshness::Fresh { last_activity_at } => Self::Active { last_activity_at },
            ReservationFreshness::Stale { last_activity_at } => Self::Quiet { last_activity_at },
        }
    }
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
