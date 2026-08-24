//! Protected commit roles and git-backed integration evidence.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

use super::lifecycle::IntegrationEvidenceStatus;
use crate::git;
use crate::git::GitError;
use crate::git::Reachability;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::ReservationId;

/// The fixed checkpoint commit used for ordinary integration evidence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct ProtectedReservationTip(GitObjectId);

impl Display for ProtectedReservationTip {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
}

impl From<GitObjectId> for ProtectedReservationTip {
    fn from(git_object_id: GitObjectId) -> Self { Self(git_object_id) }
}

impl AsRef<GitObjectId> for ProtectedReservationTip {
    fn as_ref(&self) -> &GitObjectId { &self.0 }
}

impl FromStr for ProtectedReservationTip {
    type Err = InvalidGitObjectId;

    fn from_str(value: &str) -> Result<Self, Self::Err> { value.parse::<GitObjectId>().map(Self) }
}

/// Whether this reservation previously had verified integration evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PriorIntegrationStatus {
    /// No earlier stateful check proved integration.
    Unproven,
    /// An earlier stateful check proved integration.
    Proven,
}

/// Read the full commit currently named by `HEAD`.
pub(crate) fn current_head(repository_root: &Path) -> Result<GitObjectId, GitError> {
    git::head_object_id(repository_root)
}

/// Read the full commit currently named by the configured trunk branch.
pub(crate) fn current_trunk(
    repository_root: &Path,
    trunk_branch: &str,
) -> Result<GitObjectId, GitError> {
    git::branch_object_id(repository_root, trunk_branch)
}

/// Revalidate a protected tip against current trunk.
pub(crate) fn integration_status(
    repository_root: &Path,
    protected_tip: &ProtectedReservationTip,
    trunk_oid: &GitObjectId,
    prior_integration_status: PriorIntegrationStatus,
) -> Result<IntegrationEvidenceStatus, GitError> {
    match git::reachability(repository_root, protected_tip.as_ref(), trunk_oid)? {
        Reachability::Ancestor => Ok(IntegrationEvidenceStatus::Integrated {
            trunk_oid: trunk_oid.clone(),
        }),
        Reachability::NotAncestor => match prior_integration_status {
            PriorIntegrationStatus::Unproven => Ok(IntegrationEvidenceStatus::NotIntegrated),
            PriorIntegrationStatus::Proven => Ok(IntegrationEvidenceStatus::TrunkRewritten),
        },
        Reachability::ObjectUnknown => Ok(IntegrationEvidenceStatus::ObjectUnknown),
    }
}

/// Revalidate an outstanding tip and distinguish trunk replacement from ordinary non-integration.
pub(crate) fn outstanding_integration_status(
    repository_root: &Path,
    protected_tip: &ProtectedReservationTip,
    previous_trunk_oid: &GitObjectId,
    current_trunk_oid: &GitObjectId,
) -> Result<IntegrationEvidenceStatus, GitError> {
    let status = integration_status(
        repository_root,
        protected_tip,
        current_trunk_oid,
        PriorIntegrationStatus::Unproven,
    )?;
    if !matches!(status, IntegrationEvidenceStatus::NotIntegrated) {
        return Ok(status);
    }
    match git::reachability(repository_root, previous_trunk_oid, current_trunk_oid)? {
        Reachability::Ancestor => Ok(IntegrationEvidenceStatus::NotIntegrated),
        Reachability::NotAncestor => Ok(IntegrationEvidenceStatus::TrunkRewritten),
        Reachability::ObjectUnknown => Ok(IntegrationEvidenceStatus::ObjectUnknown),
    }
}

/// Create or update the ref that keeps a protected tip reachable.
pub(crate) fn retain_protected_tip(
    repository_root: &Path,
    reservation_id: ReservationId,
    protected_tip: &ProtectedReservationTip,
) -> Result<(), GitError> {
    git::write_reservation_retention_ref(repository_root, reservation_id, protected_tip.as_ref())
}
