//! The foreign holder that blocks a requested scope, described for the caller.
//!
//! A conflict is a snapshot of one blocking holder taken at the moment overlap was evaluated:
//! the holder's identity and revision, the scopes that actually intersect, and enough
//! provenance for the caller to decide whether to wait, ask, or widen. It is a reported value
//! rather than retained state, so it never appears in a reservation's field list.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::record::ReservationHolderActivity;
use crate::answer::OverlapScopeRevision;
use crate::ids::CoordinationRunId;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::ReservationRevision;
use crate::ids::WorktreeId;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::ReservationPurpose;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;

/// One foreign holder whose retained reservation intersects requested scopes.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct ReservationConflict {
    /// The durable reservation that holds the overlapping paths.
    pub(crate) reservation_id:         ReservationId,
    /// The holder revision against which the overlap was evaluated.
    pub(super) reservation_revision:   ReservationRevision,
    /// The holder revision that changes only when its scopes change.
    #[schemars(with = "Vec<ReservationScope>", length(min = 1))]
    pub(crate) overlap_scope_revision: OverlapScopeRevision,
    /// The worktree identity that acquired the reservation.
    pub(super) holder_worktree_id:     WorktreeId,
    /// The coordination run that acquired the reservation.
    pub(crate) holder_run_id:          CoordinationRunId,
    /// The holder's attached branch or detached commit.
    pub(super) head_snapshot:          ClaimHeadSnapshot,
    /// The holder's typed acquisition provenance.
    pub(crate) source:                 ClaimSource,
    /// The holder's typed reason for protecting the paths.
    pub(crate) purpose:                ReservationPurpose,
    /// The holder scopes that intersect the requested scopes.
    pub(crate) overlapping_scopes:     ReservationScopeSet,
    /// When the holder acquired the reservation.
    #[schemars(with = "String", length(min = 1))]
    pub(super) claimed_at:             RecordedAt,
    /// Whether the holder has recorded activity inside the freshness window.
    pub(super) activity:               ReservationHolderActivity,
}

impl ReservationConflict {
    /// Return a compact display label for the holder's branch state.
    pub(crate) fn holder_branch(&self) -> String {
        match &self.head_snapshot {
            ClaimHeadSnapshot::Branch { full_ref, .. } => full_ref.to_string(),
            ClaimHeadSnapshot::Detached { head } => format!("detached at {}", head.as_ref()),
        }
    }

    /// Return the worktree that owns the conflicting reservation.
    pub(crate) const fn holder_worktree_id(&self) -> WorktreeId { self.holder_worktree_id }

    /// Return when the holder acquired the conflicting reservation.
    pub(crate) const fn claimed_at(&self) -> &RecordedAt { &self.claimed_at }

    /// Describe whether the holder remains active and when it last recorded activity.
    pub(crate) fn holder_activity_description(&self) -> String {
        match &self.activity {
            ReservationHolderActivity::Active { last_activity_at } => {
                format!("active; last activity at {last_activity_at}")
            },
            ReservationHolderActivity::Quiet { last_activity_at } => {
                format!("gone quiet; last activity at {last_activity_at}")
            },
        }
    }
}
