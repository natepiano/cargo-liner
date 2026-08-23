//! Live reservation state derived solely from append-only journal events.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::ReservationRevision;
use crate::ids::WorktreeId;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::EditAuthorization;
use crate::ledger::JournalActor;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::ReservationPurpose;
use crate::scope::PathCase;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;

/// Every reservation still live after replaying the journal in append order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LiveReservationSet {
    reservations: Vec<LiveReservation>,
}

/// One reservation that has not received a terminal release event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LiveReservation {
    reservation_id: ReservationId,
    revision:       ReservationRevision,
    scopes:         ReservationScopeSet,
    source:         ClaimSource,
    purpose:        ReservationPurpose,
    head_snapshot:  ClaimHeadSnapshot,
    actor:          JournalActor,
}

/// One foreign holder whose live reservation intersects requested scopes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ReservationConflict {
    /// The durable reservation that holds the overlapping paths.
    pub(crate) reservation_id:       ReservationId,
    /// The holder revision against which the overlap was evaluated.
    pub(crate) reservation_revision: ReservationRevision,
    /// The worktree identity that acquired the reservation.
    pub(crate) holder_worktree_id:   WorktreeId,
    /// The coordination run that acquired the reservation.
    pub(crate) holder_run_id:        CoordinationRunId,
    /// The holder's attached branch or detached commit.
    pub(crate) head_snapshot:        ClaimHeadSnapshot,
    /// The holder's typed plan provenance.
    pub(crate) source:               ClaimSource,
    /// The holder's typed reason for protecting the paths.
    pub(crate) purpose:              ReservationPurpose,
    /// The holder scopes that intersect the requested scopes.
    pub(crate) overlapping_scopes:   ReservationScopeSet,
}

impl LiveReservationSet {
    /// Replay journal operations into the current live reservation set.
    pub(crate) fn replay(events: &[JournalEvent]) -> Result<Self, ReservationReplayError> {
        let mut live_reservations = Self::default();
        for event in events {
            live_reservations.apply(event)?;
        }
        Ok(live_reservations)
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
        self.conflicts(candidate, path_case, |holder| match edit_authorization {
            EditAuthorization::Identified(coordination_run_id) => {
                holder.actor.run != coordination_run_id
            },
            EditAuthorization::Unidentified => true,
        })
    }

    fn apply(&mut self, event: &JournalEvent) -> Result<(), ReservationReplayError> {
        match &event.operation {
            JournalOperation::Claim {
                reservation_id,
                scopes,
                source,
                purpose,
                head_snapshot,
                ..
            } => {
                if self
                    .reservations
                    .iter()
                    .any(|reservation| reservation.reservation_id == *reservation_id)
                {
                    return Err(ReservationReplayError::DuplicateClaim(*reservation_id));
                }
                self.reservations.push(LiveReservation {
                    reservation_id: *reservation_id,
                    revision:       ReservationRevision::from(1),
                    scopes:         scopes.clone(),
                    source:         source.clone(),
                    purpose:        purpose.clone(),
                    head_snapshot:  head_snapshot.clone(),
                    actor:          event.actor.clone(),
                });
            },
            JournalOperation::Widen {
                reservation_id,
                added_scopes,
                ..
            } => {
                let reservation = self.find_mut(*reservation_id)?;
                let mut scopes = reservation.scopes.as_slice().to_vec();
                scopes.extend(added_scopes.iter().cloned().map(|path| ReservationScope {
                    path,
                    kind: ScopeKind::File,
                }));
                reservation.scopes = ReservationScopeSet::try_from(scopes)
                    .map_err(|_| ReservationReplayError::EmptyScopeSet(*reservation_id))?;
                reservation.advance_revision()?;
            },
            JournalOperation::Checkpoint { reservation_id, .. }
            | JournalOperation::Resnapshot { reservation_id, .. }
            | JournalOperation::Renew { reservation_id } => {
                self.find_mut(*reservation_id)?.advance_revision()?;
            },
            JournalOperation::Release { reservation_id, .. } => {
                let original_len = self.reservations.len();
                self.reservations
                    .retain(|reservation| reservation.reservation_id != *reservation_id);
                if self.reservations.len() == original_len {
                    return Err(ReservationReplayError::UnknownReservation(*reservation_id));
                }
            },
            JournalOperation::RebindWorktree {
                reservation_id,
                current_worktree_id,
                ..
            } => {
                let reservation = self.find_mut(*reservation_id)?;
                reservation.actor.worktree = *current_worktree_id;
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

    fn find_mut(
        &mut self,
        reservation_id: ReservationId,
    ) -> Result<&mut LiveReservation, ReservationReplayError> {
        self.reservations
            .iter_mut()
            .find(|reservation| reservation.reservation_id == reservation_id)
            .ok_or(ReservationReplayError::UnknownReservation(reservation_id))
    }

    fn conflicts(
        &self,
        candidate: &ReservationScopeSet,
        path_case: PathCase,
        holder_is_foreign: impl Fn(&LiveReservation) -> bool,
    ) -> Vec<ReservationConflict> {
        self.reservations
            .iter()
            .filter(|holder| holder_is_foreign(holder))
            .filter_map(|holder| {
                let overlapping_scopes = holder
                    .scopes
                    .as_slice()
                    .iter()
                    .filter(|held_scope| {
                        candidate
                            .as_slice()
                            .iter()
                            .any(|candidate_scope| held_scope.overlaps(candidate_scope, path_case))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                ReservationScopeSet::try_from(overlapping_scopes)
                    .ok()
                    .map(|overlapping_scopes| ReservationConflict {
                        reservation_id:       holder.reservation_id,
                        reservation_revision: holder.revision,
                        holder_worktree_id:   holder.actor.worktree,
                        holder_run_id:        holder.actor.run,
                        head_snapshot:        holder.head_snapshot.clone(),
                        source:               holder.source.clone(),
                        purpose:              holder.purpose.clone(),
                        overlapping_scopes:   overlapping_scopes.minimal_antichain(path_case),
                    })
            })
            .collect()
    }
}

impl LiveReservation {
    fn advance_revision(&mut self) -> Result<(), ReservationReplayError> {
        let revision: u64 = self.revision.into();
        self.revision = revision
            .checked_add(1)
            .map(ReservationRevision::from)
            .ok_or(ReservationReplayError::RevisionExhausted(
                self.reservation_id,
            ))?;
        Ok(())
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

/// A journal sequence that cannot represent valid live reservation state.
#[derive(Debug)]
pub(crate) enum ReservationReplayError {
    /// Two live claims reused one non-recyclable reservation identity.
    DuplicateClaim(ReservationId),
    /// A replayed mutation referenced no live reservation.
    UnknownReservation(ReservationId),
    /// A replayed widen somehow produced an empty scope set.
    EmptyScopeSet(ReservationId),
    /// A reservation revision counter can no longer advance.
    RevisionExhausted(ReservationId),
}

impl fmt::Display for ReservationReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateClaim(reservation_id) => {
                write!(
                    formatter,
                    "duplicate live claim for reservation {reservation_id}"
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
        }
    }
}

impl std::error::Error for ReservationReplayError {}
