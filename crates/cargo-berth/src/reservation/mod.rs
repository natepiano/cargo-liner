//! Reservation state derived solely from append-only journal events.

mod evidence;
mod lifecycle;

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::time::Duration;

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

use crate::answer::AuthorizedOverlap;
use crate::answer::AuthorizedOverlapSet;
use crate::answer::ConflictAuthorization;
use crate::answer::OverlapScopeRevision;
use crate::ids::CoordinationRunId;
use crate::ids::EventId;
use crate::ids::GitObjectId;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::ReservationRevision;
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
use crate::ledger::TrunkCommitAtClaim;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::scope::PathCase;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;

/// Every retained reservation after replaying the journal in append order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RetainedReservationSet {
    reservations:        Vec<Reservation>,
    incursion_incidents: Vec<IncursionIncident>,
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
    phase_start_head:           ProtectedPhaseStartHead,
    actor:                      JournalActor,
    lifecycle:                  ReservationLifecycle,
    retained_protected_tip:     RetainedProtectedTip,
    integration_trunk_snapshot: IntegrationTrunkSnapshot,
    integration_status:         IntegrationEvidenceStatus,
    edit_blocking_status:       EditBlockingStatus,
    worktree_root:              CanonicalWorktreeRoot,
    worktree_locator:           WorktreeAdministrativeLocator,
    last_activity_at:           RecordedAt,
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
        /// The journal append that answered the incident.
        resolution_event_id: EventId,
        /// When the disposition was recorded.
        resolved_at:         RecordedAt,
    },
}

/// Whether a drift observation matches an already-outstanding incident.
pub(crate) enum IncursionObservation {
    /// Replay already carries the same unanswered incident.
    AlreadyOutstanding(IncursionIncidentId),
    /// This observation requires a newly minted incident record.
    NewlyObserved(IncursionIncidentId),
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
    reservation_revision:              ReservationRevision,
    /// The holder revision that changes only when its scopes change.
    pub(crate) overlap_scope_revision: OverlapScopeRevision,
    /// The worktree identity that acquired the reservation.
    holder_worktree_id:                WorktreeId,
    /// The coordination run that acquired the reservation.
    pub(crate) holder_run_id:          CoordinationRunId,
    /// The holder's attached branch or detached commit.
    head_snapshot:                     ClaimHeadSnapshot,
    /// The holder's typed plan provenance.
    pub(crate) source:                 ClaimSource,
    /// The holder's typed reason for protecting the paths.
    pub(crate) purpose:                ReservationPurpose,
    /// The holder scopes that intersect the requested scopes.
    pub(crate) overlapping_scopes:     ReservationScopeSet,
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

/// The run identity permitted to receive its reservation-specific overlap answers.
#[derive(Clone, Copy)]
pub(crate) enum AuthorizedEditingIdentity {
    /// A live session mapping identifies one exact reservation.
    SessionReservation {
        coordination_run_id: CoordinationRunId,
        reservation_id:      ReservationId,
    },
    /// The process or validated marker identifies this coordination run.
    Run(CoordinationRunId),
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

    /// Evaluate changed paths against edit-blocking reservations of another acting identity.
    pub(crate) fn conflicts_for_drift(
        &self,
        candidate: &ReservationScopeSet,
        acting_run_id: CoordinationRunId,
        acting_worktree_id: WorktreeId,
        path_case: PathCase,
    ) -> Vec<ReservationConflict> {
        self.conflicts(candidate, path_case, |holder| {
            holder.actor.run != acting_run_id || holder.actor.worktree != acting_worktree_id
        })
    }

    /// Classify all blocking coverage of one changed path in drift-table order.
    pub(crate) fn blocking_coverage_for_drift(
        &self,
        candidate: &ReservationScopeSet,
        acting_run_id: CoordinationRunId,
        acting_worktree_id: WorktreeId,
        path_case: PathCase,
    ) -> DriftBlockingCoverage {
        if !self
            .conflicts_with_holders(candidate, path_case, |holder| {
                holder.actor.run == acting_run_id && holder.actor.worktree == acting_worktree_id
            })
            .is_empty()
        {
            return DriftBlockingCoverage::SameIdentity;
        }
        let conflicts =
            self.conflicts_for_drift(candidate, acting_run_id, acting_worktree_id, path_case);
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
                WidenScopeBinding::Authorized(ConflictAuthorization::Revalidated { overlaps })
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
            EditAuthorization::Environment(_)
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
            | EditAuthorization::Environment(_)
            | EditAuthorization::Unidentified => false,
        };
        match edit_authorization {
            EditAuthorization::Session {
                coordination_run_id,
                reservation_id,
                ..
            } if session_is_active => AuthorizedEditingIdentity::SessionReservation {
                coordination_run_id,
                reservation_id,
            },
            EditAuthorization::Environment(coordination_run_id) => {
                AuthorizedEditingIdentity::Run(coordination_run_id)
            },
            EditAuthorization::Marker {
                coordination_run_id,
                ..
            } if marker_is_active => AuthorizedEditingIdentity::Run(coordination_run_id),
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

    /// Classify an observed incursion against unanswered incidents from replay.
    pub(crate) fn observe_incursion(
        &self,
        reservation_id: ReservationId,
        foreign_reservation_ids: &ForeignReservationIdSet,
        paths: &IncursionPathSet,
    ) -> IncursionObservation {
        self.incursion_incidents
            .iter()
            .find(|incident| {
                incident.reservation_id() == reservation_id
                    && incident.foreign_reservation_ids() == foreign_reservation_ids
                    && incident.paths() == paths
                    && matches!(incident.status(), IncursionIncidentStatus::Outstanding)
            })
            .map_or_else(
                || IncursionObservation::NewlyObserved(IncursionIncidentId::new()),
                |incident| IncursionObservation::AlreadyOutstanding(incident.id()),
            )
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
                edit_blocking_status,
                ..
            } => self.apply_widen(
                *reservation_id,
                added_scopes,
                authorization,
                *edit_blocking_status,
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
                edit_blocking_status,
            } => self.apply_evidence(*reservation_id, status, *edit_blocking_status)?,
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
            JournalOperation::ResolveIncursion { incident_id } => {
                self.apply_incursion_resolution(*incident_id, event.event_id(), event.recorded_at())
            },
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
            id:                         replayed_claim.id,
            revision:                   ReservationRevision::from(1),
            scopes:                     replayed_claim.scopes.clone(),
            authorizations:             vec![replayed_claim.authorization.clone()],
            source:                     replayed_claim.source.clone(),
            purpose:                    replayed_claim.purpose.clone(),
            head_snapshot:              replayed_claim.head_snapshot.clone(),
            phase_start_head:           replayed_claim.phase_start_head.clone(),
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
            last_activity_at:           replayed_claim.recorded_at.clone(),
        });
        Ok(())
    }

    fn apply_widen(
        &mut self,
        reservation_id: ReservationId,
        added_scopes: &ReservationScopeAdditionSet,
        authorization: &ConflictAuthorization,
        edit_blocking_status: EditBlockingStatus,
        recorded_at: &RecordedAt,
    ) -> Result<(), ReservationReplayError> {
        let reservation = self.find_mut(reservation_id)?;
        let mut scopes = reservation.scopes.as_slice().to_vec();
        scopes.extend(added_scopes.as_slice().iter().cloned());
        reservation.scopes = ReservationScopeSet::try_from(scopes)
            .map_err(|_| ReservationReplayError::EmptyScopeSet(reservation_id))?;
        reservation.edit_blocking_status = edit_blocking_status;
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
        reservation.lifecycle.checkpoint(protected_tip.clone())?;
        reservation.retained_protected_tip = RetainedProtectedTip::Retained(protected_tip.clone());
        reservation.integration_trunk_snapshot =
            IntegrationTrunkSnapshot::AtCheckpoint(trunk_snapshot.clone());
        reservation.integration_status = IntegrationEvidenceStatus::NotIntegrated;
        reservation.edit_blocking_status = EditBlockingStatus::Blocking;
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

impl AuthorizedEditingIdentity {
    fn is_foreign(self, holder: &Reservation) -> bool {
        match self {
            Self::SessionReservation {
                coordination_run_id,
                ..
            }
            | Self::Run(coordination_run_id) => holder.actor.run != coordination_run_id,
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

    fn identifies_requester(self, requester: &Reservation) -> bool {
        match self {
            Self::SessionReservation {
                coordination_run_id,
                ..
            }
            | Self::Run(coordination_run_id) => requester.actor.run == coordination_run_id,
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

    /// Return the materialized edit decision.
    pub(crate) const fn edit_blocking_status(&self) -> EditBlockingStatus {
        self.edit_blocking_status
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

impl Display for ReservationReplayError {
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

    use super::DriftBlockingCoverage;
    use super::IntegrationEvidenceStatus;
    use super::ReservationEvidenceState;
    use super::RetainedReservationSet;
    use super::lifecycle::EditBlockingStatus;
    use crate::ids::CoordinationRunId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::JournalEvent;
    use crate::scope::PathCase;
    use crate::scope::ReservationScopeSet;
    use crate::scope::ScopeKind;

    const PROTECTED_TIP: &str = "2222222222222222222222222222222222222222";
    const REPLACEMENT_TIP: &str = "3333333333333333333333333333333333333333";
    const RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f";
    const RUN_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1e";
    const SECOND_RUN_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a20";
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
    fn drift_blocking_coverage_requires_both_run_and_worktree_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let [claim, ..] = lifecycle_events()?;
        let reservations = RetainedReservationSet::replay(&[claim])?;
        let candidate = serde_json::from_value::<ReservationScopeSet>(json!([
            {"path": "src/lib.rs", "kind": "file"}
        ]))?;
        let run_id = RUN_ID.parse::<CoordinationRunId>()?;
        let second_run_id = SECOND_RUN_ID.parse::<CoordinationRunId>()?;
        let worktree_id = WORKTREE_ID.parse::<WorktreeId>()?;
        let second_worktree_id = SECOND_WORKTREE_ID.parse::<WorktreeId>()?;

        assert!(matches!(
            reservations.blocking_coverage_for_drift(
                &candidate,
                run_id,
                worktree_id,
                PathCase::Sensitive,
            ),
            DriftBlockingCoverage::SameIdentity
        ));
        let DriftBlockingCoverage::Foreign(different_run) = reservations
            .blocking_coverage_for_drift(
                &candidate,
                second_run_id,
                worktree_id,
                PathCase::Sensitive,
            )
        else {
            return Err(std::io::Error::other("another run should be foreign").into());
        };
        assert_eq!(different_run[0].reservation_id.to_string(), RESERVATION_ID);
        let DriftBlockingCoverage::Foreign(different_worktree) = reservations
            .blocking_coverage_for_drift(
                &candidate,
                run_id,
                second_worktree_id,
                PathCase::Sensitive,
            )
        else {
            return Err(std::io::Error::other("another worktree should be foreign").into());
        };
        assert_eq!(
            different_worktree[0].reservation_id.to_string(),
            RESERVATION_ID
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
