//! Reservation state derived solely from append-only journal events.

mod evidence;
mod lifecycle;

use std::fmt;

pub(crate) use evidence::PriorIntegrationStatus;
pub(crate) use evidence::ProtectedReservationTip;
pub(crate) use evidence::current_head;
pub(crate) use evidence::current_trunk;
pub(crate) use evidence::integration_status;
pub(crate) use evidence::outstanding_integration_status;
pub(crate) use evidence::retain_protected_tip;
pub(crate) use lifecycle::AbandonmentReason;
pub(crate) use lifecycle::EditBlockingStatus;
pub(crate) use lifecycle::IntegrationEvidenceStatus;
pub(crate) use lifecycle::LifecycleTransitionError;
pub(crate) use lifecycle::OrphanRetirementReason;
pub(crate) use lifecycle::ReleaseDisposition;
pub(crate) use lifecycle::ReleaseRevalidationSubject;
pub(crate) use lifecycle::ReservationLifecycle;
pub(crate) use lifecycle::RewrittenIntegrationTrunkCommit;
use serde::Deserialize;
use serde::Serialize;

use crate::answer::ConflictAuthorization;
use crate::answer::OverlapScopeRevision;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::ReservationRevision;
use crate::ids::WorktreeId;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::EditAuthorization;
use crate::ledger::JournalActor;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::ReservationPurpose;
use crate::ledger::ReservationSnapshot;
use crate::ledger::TrunkCommitAtClaim;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::scope::PathCase;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;

/// Every retained reservation after replaying the journal in append order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RetainedReservationSet {
    reservations: Vec<Reservation>,
}

/// One reservation retained for overlap, evidence, and audit decisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Reservation {
    id:                         ReservationId,
    revision:                   ReservationRevision,
    scopes:                     ReservationScopeSet,
    authorizations:             Vec<ConflictAuthorization>,
    source:                     ClaimSource,
    purpose:                    ReservationPurpose,
    head_snapshot:              ClaimHeadSnapshot,
    actor:                      JournalActor,
    lifecycle:                  ReservationLifecycle,
    retained_protected_tip:     RetainedProtectedTip,
    integration_trunk_snapshot: IntegrationTrunkSnapshot,
    integration_status:         IntegrationEvidenceStatus,
    edit_blocking_status:       EditBlockingStatus,
    worktree_root:              CanonicalWorktreeRoot,
    worktree_locator:           WorktreeAdministrativeLocator,
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
    AtClaim(TrunkCommitAtClaim),
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
    trunk_at_claim:   &'event TrunkCommitAtClaim,
    head_snapshot:    &'event ClaimHeadSnapshot,
    actor:            &'event JournalActor,
    worktree_root:    &'event CanonicalWorktreeRoot,
    worktree_locator: &'event WorktreeAdministrativeLocator,
    authorization:    &'event ConflictAuthorization,
}

/// State-specific evidence exposed without an optional protected commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReservationEvidenceState {
    /// Active work has no protected integration subject.
    Active {
        /// The trunk commit observed when the reservation was acquired.
        trunk_at_claim: TrunkCommitAtClaim,
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

/// One foreign holder whose retained reservation intersects requested scopes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ReservationConflict {
    /// The durable reservation that holds the overlapping paths.
    pub(crate) reservation_id:         ReservationId,
    /// The holder revision against which the overlap was evaluated.
    pub(crate) reservation_revision:   ReservationRevision,
    /// The holder revision that changes only when its scopes change.
    pub(crate) overlap_scope_revision: OverlapScopeRevision,
    /// The worktree identity that acquired the reservation.
    pub(crate) holder_worktree_id:     WorktreeId,
    /// The coordination run that acquired the reservation.
    pub(crate) holder_run_id:          CoordinationRunId,
    /// The holder's attached branch or detached commit.
    pub(crate) head_snapshot:          ClaimHeadSnapshot,
    /// The holder's typed plan provenance.
    pub(crate) source:                 ClaimSource,
    /// The holder's typed reason for protecting the paths.
    pub(crate) purpose:                ReservationPurpose,
    /// The holder scopes that intersect the requested scopes.
    pub(crate) overlapping_scopes:     ReservationScopeSet,
}

/// The run identity permitted to receive its reservation-specific overlap answers.
#[derive(Clone, Copy)]
enum AuthorizedEditingRun {
    /// The process or validated marker identifies this coordination run.
    Identified(CoordinationRunId),
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

    /// Evaluate claim acquisition for one coordination run.
    pub(crate) fn conflicts_for_claim(
        &self,
        candidate: &ReservationScopeSet,
        coordination_run_id: CoordinationRunId,
        path_case: PathCase,
    ) -> Vec<ReservationConflict> {
        self.conflicts(candidate, path_case, |holder| {
            holder.actor.run != coordination_run_id
        })
    }

    /// Evaluate an edit check using only authorization resolved by the process.
    pub(crate) fn conflicts_for_edit(
        &self,
        candidate: &ReservationScopeSet,
        edit_authorization: EditAuthorization,
        path_case: PathCase,
    ) -> Vec<ReservationConflict> {
        let marker_is_active = match edit_authorization {
            EditAuthorization::Marker {
                coordination_run_id,
                worktree_id,
            } => self.reservations.iter().any(|reservation| {
                matches!(reservation.lifecycle, ReservationLifecycle::Active)
                    && reservation.actor.run == coordination_run_id
                    && reservation.actor.worktree == worktree_id
            }),
            EditAuthorization::Environment(_) | EditAuthorization::Unidentified => false,
        };
        let authorized_editing_run = match edit_authorization {
            EditAuthorization::Environment(coordination_run_id) => {
                AuthorizedEditingRun::Identified(coordination_run_id)
            },
            EditAuthorization::Marker {
                coordination_run_id,
                ..
            } if marker_is_active => AuthorizedEditingRun::Identified(coordination_run_id),
            EditAuthorization::Marker { .. } | EditAuthorization::Unidentified => {
                AuthorizedEditingRun::Unidentified
            },
        };
        let conflicts = self.conflicts_with_holders(candidate, path_case, |holder| {
            authorized_editing_run.is_foreign(holder)
        });
        let mut unanswered_conflicts = Vec::new();
        for (holder, mut conflict) in conflicts {
            let unanswered_scopes = conflict
                .overlapping_scopes
                .as_slice()
                .iter()
                .filter(|overlap_scope| {
                    !authorized_editing_run.authorizes(self, holder, overlap_scope, path_case)
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

    fn apply(&mut self, event: &JournalEvent) -> Result<(), ReservationReplayError> {
        match &event.operation {
            JournalOperation::Claim {
                reservation_id,
                scopes,
                source,
                purpose,
                trunk_at_claim,
                head_snapshot,
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
                actor: &event.actor,
                worktree_root,
                worktree_locator: worktree_administrative_locator,
                authorization,
            })?,
            JournalOperation::Widen {
                reservation_id,
                added_scopes,
                authorization,
                ..
            } => self.apply_widen(*reservation_id, added_scopes, authorization)?,
            JournalOperation::Checkpoint {
                reservation_id,
                protected_tip,
                trunk_snapshot,
            } => self.apply_checkpoint(*reservation_id, protected_tip, trunk_snapshot)?,
            JournalOperation::Resnapshot {
                reservation_id,
                snapshot,
            } => self.apply_resnapshot(*reservation_id, snapshot)?,
            JournalOperation::Renew { reservation_id } => {
                self.find_mut(*reservation_id)?.advance_revision()?;
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
                edit_blocking_status,
            } => self.apply_evidence(*reservation_id, status, *edit_blocking_status)?,
            JournalOperation::RebindWorktree {
                reservation_id,
                previous_worktree_id,
                current_worktree_id,
                current_worktree_root,
                current_worktree_administrative_locator,
            } => {
                let reservation = self.find_mut(*reservation_id)?;
                if reservation.actor.worktree != *previous_worktree_id {
                    return Err(ReservationReplayError::WorktreeRebindingMismatch(
                        *reservation_id,
                    ));
                }
                reservation.actor.worktree = *current_worktree_id;
                reservation.worktree_root = current_worktree_root.clone();
                reservation.worktree_locator = current_worktree_administrative_locator.clone();
                reservation.advance_revision()?;
            },
            JournalOperation::RelocateWorktree {
                reservation_id,
                worktree_id,
                previous_root,
                current_root,
            } => {
                let reservation = self.find_mut(*reservation_id)?;
                if reservation.actor.worktree != *worktree_id
                    || reservation.worktree_root != *previous_root
                {
                    return Err(ReservationReplayError::WorktreeRelocationMismatch(
                        *reservation_id,
                    ));
                }
                reservation.worktree_root = current_root.clone();
                reservation.advance_revision()?;
            },
            JournalOperation::ResolveDefer { .. }
            | JournalOperation::DeclareOrderingEdge { .. }
            | JournalOperation::Incursion { .. }
            | JournalOperation::ForcedIntegrationPermit { .. }
            | JournalOperation::ConsumeForcedIntegrationPermit { .. }
            | JournalOperation::Bypass { .. } => {},
        }
        Ok(())
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
            id:                         replayed_claim.id,
            revision:                   ReservationRevision::from(1),
            scopes:                     replayed_claim.scopes.clone(),
            authorizations:             vec![replayed_claim.authorization.clone()],
            source:                     replayed_claim.source.clone(),
            purpose:                    replayed_claim.purpose.clone(),
            head_snapshot:              replayed_claim.head_snapshot.clone(),
            actor:                      replayed_claim.actor.clone(),
            lifecycle:                  ReservationLifecycle::Active,
            retained_protected_tip:     RetainedProtectedTip::NotCheckpointed,
            integration_trunk_snapshot: IntegrationTrunkSnapshot::AtClaim(
                replayed_claim.trunk_at_claim.clone(),
            ),
            integration_status:         IntegrationEvidenceStatus::NotIntegrated,
            edit_blocking_status:       EditBlockingStatus::Blocking,
            worktree_root:              replayed_claim.worktree_root.clone(),
            worktree_locator:           replayed_claim.worktree_locator.clone(),
        });
        Ok(())
    }

    fn apply_widen(
        &mut self,
        reservation_id: ReservationId,
        added_scopes: &[crate::ids::ReservationScopePath],
        authorization: &ConflictAuthorization,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        let mut scopes = reservation.scopes.as_slice().to_vec();
        scopes.extend(added_scopes.iter().cloned().map(|path| ReservationScope {
            path,
            kind: ScopeKind::File,
        }));
        reservation.scopes = ReservationScopeSet::try_from(scopes)
            .map_err(|_| ReservationReplayError::EmptyScopeSet(reservation_id))?;
        reservation.advance_revision()?;
        reservation.authorizations.push(authorization.clone());
        Ok(())
    }

    fn apply_checkpoint(
        &mut self,
        reservation_id: ReservationId,
        protected_tip: &ProtectedReservationTip,
        trunk_snapshot: &GitObjectId,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        reservation.lifecycle.checkpoint(protected_tip.clone())?;
        reservation.retained_protected_tip = RetainedProtectedTip::Retained(protected_tip.clone());
        reservation.integration_trunk_snapshot =
            IntegrationTrunkSnapshot::AtCheckpoint(trunk_snapshot.clone());
        reservation.integration_status = IntegrationEvidenceStatus::NotIntegrated;
        reservation.edit_blocking_status = EditBlockingStatus::Blocking;
        reservation.advance_revision()
    }

    fn apply_resnapshot(
        &mut self,
        reservation_id: ReservationId,
        snapshot: &ReservationSnapshot,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        match snapshot {
            ReservationSnapshot::Active { .. } => {
                if !matches!(reservation.lifecycle, ReservationLifecycle::Active) {
                    return Err(ReservationReplayError::SnapshotStateMismatch(
                        reservation_id,
                    ));
                }
            },
            ReservationSnapshot::Outstanding {
                protected_tip,
                trunk_oid,
            } => {
                reservation.lifecycle.resnapshot(protected_tip.clone())?;
                reservation.retained_protected_tip =
                    RetainedProtectedTip::Retained(protected_tip.clone());
                reservation.integration_trunk_snapshot =
                    IntegrationTrunkSnapshot::AtCheckpoint(trunk_oid.clone());
                reservation.integration_status = IntegrationEvidenceStatus::NotIntegrated;
                reservation.edit_blocking_status = EditBlockingStatus::Blocking;
            },
        }
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
            };
        }
        match disposition {
            ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_) => reservation
                .lifecycle
                .release_after_user_confirmation(disposition.clone())?,
            ReleaseDisposition::Integrated | ReleaseDisposition::RewrittenIntegration(_) => {
                reservation.lifecycle.release(disposition.clone())?;
            },
        }
        reservation.edit_blocking_status = EditBlockingStatus::Clear;
        reservation.advance_revision()
    }

    fn apply_evidence(
        &mut self,
        reservation_id: ReservationId,
        status: &IntegrationEvidenceStatus,
        edit_blocking_status: EditBlockingStatus,
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
        reservation.edit_blocking_status = edit_blocking_status;
        reservation.advance_revision()
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
            .replace_release_disposition(superseded, replacement.clone())?;
        if let ReleaseDisposition::RewrittenIntegration(trunk_commit) = replacement {
            reservation.integration_status = IntegrationEvidenceStatus::Integrated {
                trunk_oid: trunk_commit.as_ref().clone(),
            };
        }
        reservation.edit_blocking_status = EditBlockingStatus::Clear;
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
        self.reservations
            .iter()
            .filter(|holder| holder.edit_blocking_status == EditBlockingStatus::Blocking)
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
                            },
                        )
                    })
            })
            .collect()
    }
}

impl AuthorizedEditingRun {
    fn is_foreign(self, holder: &Reservation) -> bool {
        match self {
            Self::Identified(coordination_run_id) => holder.actor.run != coordination_run_id,
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
        let Self::Identified(coordination_run_id) = self else {
            return false;
        };
        reservations
            .reservations
            .iter()
            .filter(|requester| {
                requester.actor.run == coordination_run_id
                    && requester.edit_blocking_status == EditBlockingStatus::Blocking
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

    /// Return the reservation's owning actor.
    pub(crate) const fn actor(&self) -> &JournalActor { &self.actor }

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

    /// Return the materialized edit decision.
    pub(crate) const fn edit_blocking_status(&self) -> EditBlockingStatus {
        self.edit_blocking_status
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

impl ReservationConflict {
    /// Return a compact display label for the holder's branch state.
    pub(crate) fn holder_branch(&self) -> String {
        match &self.head_snapshot {
            ClaimHeadSnapshot::Branch { full_ref, .. } => full_ref.to_string(),
            ClaimHeadSnapshot::Detached { head } => format!("detached at {}", head.as_ref()),
        }
    }
}

/// A journal sequence that cannot represent valid reservation state.
#[derive(Debug)]
pub(crate) enum ReservationReplayError {
    /// Two claims reused one non-recyclable reservation identity.
    DuplicateClaim(ReservationId),
    /// A replayed mutation referenced no retained reservation.
    UnknownReservation(ReservationId),
    /// A replayed widen somehow produced an empty scope set.
    EmptyScopeSet(ReservationId),
    /// A reservation revision counter can no longer advance.
    RevisionExhausted(ReservationId),
    /// A lifecycle transition appeared in an invalid order.
    InvalidLifecycleTransition(LifecycleTransitionError),
    /// A snapshot variant disagreed with the reservation lifecycle.
    SnapshotStateMismatch(ReservationId),
    /// An ordinary integrated disposition lacked a preceding verified status.
    IntegratedReleaseWithoutEvidence(ReservationId),
    /// Git evidence was materialized for an active reservation.
    ActiveEvidenceRevalidation(ReservationId),
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

impl fmt::Display for ReservationReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateClaim(reservation_id) => {
                write!(
                    formatter,
                    "duplicate claim for reservation {reservation_id}"
                )
            },
            Self::UnknownReservation(reservation_id) => {
                write!(
                    formatter,
                    "journal operation names unknown reservation {reservation_id}"
                )
            },
            Self::EmptyScopeSet(reservation_id) => {
                write!(
                    formatter,
                    "reservation {reservation_id} replayed with no scopes"
                )
            },
            Self::RevisionExhausted(reservation_id) => {
                write!(
                    formatter,
                    "reservation {reservation_id} revision is exhausted"
                )
            },
            Self::InvalidLifecycleTransition(error) => error.fmt(formatter),
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

impl From<LifecycleTransitionError> for ReservationReplayError {
    fn from(error: LifecycleTransitionError) -> Self { Self::InvalidLifecycleTransition(error) }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use serde_json::json;

    use super::IntegrationEvidenceStatus;
    use super::ReservationEvidenceState;
    use super::RetainedReservationSet;
    use super::lifecycle::EditBlockingStatus;
    use crate::ids::ReservationId;
    use crate::ledger::JournalEvent;

    const PROTECTED_TIP: &str = "2222222222222222222222222222222222222222";
    const REPLACEMENT_TIP: &str = "3333333333333333333333333333333333333333";
    const RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f";
    const TRUNK_OID: &str = "1111111111111111111111111111111111111111";

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
        let reblocked = RetainedReservationSet::replay(&[
            claim.clone(),
            checkpoint.clone(),
            integrated.clone(),
            release.clone(),
            rewritten.clone(),
        ])?;
        assert!(matches!(
            reblocked
                .reservation(reservation_id)
                .and_then(super::Reservation::evidence_state),
            Ok(ReservationEvidenceState::Released {
                integration_status: IntegrationEvidenceStatus::TrunkRewritten,
                ..
            })
        ));
        let recovered = RetainedReservationSet::replay(&[
            claim, checkpoint, integrated, release, rewritten, resnapshot,
        ])?;
        assert!(matches!(
            recovered
                .reservation(reservation_id)
                .and_then(super::Reservation::evidence_state),
            Ok(ReservationEvidenceState::Outstanding {
                protected_tip,
                integration_status: IntegrationEvidenceStatus::NotIntegrated,
                ..
            }) if protected_tip.to_string() == REPLACEMENT_TIP
        ));
        Ok(())
    }

    #[test]
    fn replay_reads_the_journaled_edit_blocking_status() -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = RESERVATION_ID.parse::<ReservationId>()?;
        let [claim, checkpoint, ..] = lifecycle_events()?;
        let recorded_blocking_evidence = journal_event(
            3,
            &json!({
                "op": "evidence_revalidated",
                "reservation_id": RESERVATION_ID,
                "status": {"status": "integrated", "trunk_oid": TRUNK_OID},
                "edit_blocking_status": "blocking",
            }),
        )?;

        let retained_reservations =
            RetainedReservationSet::replay(&[claim, checkpoint, recorded_blocking_evidence])?;

        assert_eq!(
            retained_reservations
                .reservation(reservation_id)?
                .edit_blocking_status,
            EditBlockingStatus::Blocking
        );
        Ok(())
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
