//! The borrowed claim record replay consumes and the faults that stop it.
//!
//! Replay reads an append-only journal, so every fault here describes a sequence that
//! cannot represent valid reservation state rather than an operation that merely failed.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use super::lifecycle::LifecycleTransitionError;
use crate::answer::ConflictAuthorization;
use crate::coordination_identity::CoordinationIdentityProvenance;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::IncursionIncidentId;
use crate::ledger::JournalActor;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReservationPurpose;
use crate::ledger::TrunkObservationAtClaim;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::scope::ReservationScopeSet;

/// Borrowed fields from one replayed claim event.
#[derive(Clone, Copy)]
pub(super) struct ReplayedClaim<'event> {
    pub(super) id:                               ReservationId,
    pub(super) scopes:                           &'event ReservationScopeSet,
    pub(super) source:                           &'event ClaimSource,
    pub(super) purpose:                          &'event ReservationPurpose,
    pub(super) trunk_at_claim:                   &'event TrunkObservationAtClaim,
    pub(super) head_snapshot:                    &'event ClaimHeadSnapshot,
    pub(super) phase_start_head:                 &'event ProtectedPhaseStartHead,
    pub(super) actor:                            &'event JournalActor,
    pub(super) worktree_root:                    &'event CanonicalWorktreeRoot,
    pub(super) worktree_locator:                 &'event WorktreeAdministrativeLocator,
    pub(super) authorization:                    &'event ConflictAuthorization,
    pub(super) recorded_at:                      &'event RecordedAt,
    /// Whether a caller presented the coordination identity this claim was made under.
    pub(super) coordination_identity_provenance: CoordinationIdentityProvenance,
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

impl ReservationReplayError {
    /// Render one fault against the reservation whose replay could not continue.
    fn write_holder_fault(
        formatter: &mut Formatter<'_>,
        reservation_id: &ReservationId,
        fault: &str,
    ) -> fmt::Result {
        write!(formatter, "reservation {reservation_id} {fault}")
    }

    /// Render one fault that only an unreleased reservation can commit.
    fn write_active_holder_fault(
        formatter: &mut Formatter<'_>,
        reservation_id: &ReservationId,
        fault: &str,
    ) -> fmt::Result {
        write!(formatter, "active reservation {reservation_id} {fault}")
    }
}

impl Display for ReservationReplayError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateClaim(reservation_id) => write!(
                formatter,
                "duplicate claim for reservation {reservation_id}"
            ),
            Self::DuplicateIncursionIncident(incident_id) => {
                write!(formatter, "duplicate incursion incident {incident_id}")
            },
            Self::UnknownReservation(reservation_id) => write!(
                formatter,
                "journal operation names unknown reservation {reservation_id}"
            ),
            Self::UnknownIncursionIncident(incident_id) => write!(
                formatter,
                "journal operation names unknown incursion {incident_id}"
            ),
            Self::IncursionIncidentAlreadyResolved(incident_id) => write!(
                formatter,
                "incursion incident {incident_id} is already resolved"
            ),
            Self::EmptyScopeSet(reservation_id) => {
                Self::write_holder_fault(formatter, reservation_id, "replayed with no scopes")
            },
            Self::WidenRequiresUnreleased(reservation_id) => {
                Self::write_holder_fault(formatter, reservation_id, "cannot widen after release")
            },
            Self::RevisionExhausted(reservation_id) => {
                Self::write_holder_fault(formatter, reservation_id, "revision is exhausted")
            },
            Self::IntegrationProofSubjectRevisionExhausted(reservation_id) => {
                Self::write_holder_fault(
                    formatter,
                    reservation_id,
                    "integration-proof subject revision is exhausted",
                )
            },
            Self::InvalidLifecycleTransition(reservation_id, error) => write!(
                formatter,
                "reservation {reservation_id} lifecycle transition failed: {error}"
            ),
            Self::SnapshotStateMismatch(reservation_id) => {
                Self::write_holder_fault(formatter, reservation_id, "has a mismatched resnapshot")
            },
            Self::IntegratedReleaseWithoutEvidence(reservation_id) => Self::write_holder_fault(
                formatter,
                reservation_id,
                "was released as integrated without verified evidence",
            ),
            Self::ActiveEvidenceRevalidation(reservation_id) => Self::write_active_holder_fault(
                formatter,
                reservation_id,
                "cannot have integration evidence",
            ),
            Self::ActiveScopedPatchComparison(reservation_id) => Self::write_active_holder_fault(
                formatter,
                reservation_id,
                "cannot have a scoped patch comparison",
            ),
            Self::IntegrationProofSubjectMismatch(reservation_id) => Self::write_holder_fault(
                formatter,
                reservation_id,
                "has a mismatched integration-proof subject",
            ),
            Self::DecisionHasNoGitEvidence(reservation_id) => Self::write_holder_fault(
                formatter,
                reservation_id,
                "has no git evidence to revalidate",
            ),
            Self::MissingProtectedTip(reservation_id) => {
                Self::write_holder_fault(formatter, reservation_id, "is missing its protected tip")
            },
            Self::MissingTrunkSnapshot(reservation_id) => Self::write_holder_fault(
                formatter,
                reservation_id,
                "is missing its checkpoint trunk snapshot",
            ),
            Self::WorktreeRelocationMismatch(reservation_id) => Self::write_holder_fault(
                formatter,
                reservation_id,
                "has a mismatched worktree relocation",
            ),
            Self::WorktreeRebindingMismatch(reservation_id) => Self::write_holder_fault(
                formatter,
                reservation_id,
                "has a mismatched worktree rebinding",
            ),
            Self::InvalidReplacementDisposition(reservation_id) => Self::write_holder_fault(
                formatter,
                reservation_id,
                "has an invalid replacement disposition",
            ),
        }
    }
}

impl Error for ReservationReplayError {}
