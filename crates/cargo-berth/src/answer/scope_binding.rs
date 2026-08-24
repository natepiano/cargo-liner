//! Scope-only revisions and non-empty overlap bindings.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::ids::ReservationId;
use crate::reservation::ReservationConflict;
use crate::scope::PathCase;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;

/// A deterministic revision that changes only when a reservation's scopes change.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct OverlapScopeRevision(Vec<ReservationScope>);

/// The non-empty normalized scopes covered for one holder.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct AuthorizedOverlapScopeSet(ReservationScopeSet);

/// One exact holder and scope revision covered by an authorization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AuthorizedOverlap {
    /// The existing holder named by the authorization.
    pub(crate) reservation_id: ReservationId,
    /// The holder's scope-only revision when the authorization was shown.
    pub(crate) scope_revision: OverlapScopeRevision,
    /// The normalized overlap scopes that this answer covers.
    pub(crate) scopes:         AuthorizedOverlapScopeSet,
}

/// A non-empty set of holder-specific overlap bindings.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct AuthorizedOverlapSet(Vec<AuthorizedOverlap>);

impl From<&ReservationScopeSet> for OverlapScopeRevision {
    fn from(scopes: &ReservationScopeSet) -> Self {
        let mut canonical_scopes = scopes.as_slice().to_vec();
        canonical_scopes.sort_by(|left, right| {
            left.path
                .to_string()
                .cmp(&right.path.to_string())
                .then_with(|| left.kind.cmp(&right.kind))
        });
        Self(canonical_scopes)
    }
}

impl AuthorizedOverlapScopeSet {
    fn covers(&self, overlap_scope: &ReservationScope, path_case: PathCase) -> bool {
        self.0
            .as_slice()
            .iter()
            .any(|authorized_scope| authorized_scope.contains(overlap_scope, path_case))
    }
}

impl From<ReservationScopeSet> for AuthorizedOverlapScopeSet {
    fn from(scopes: ReservationScopeSet) -> Self { Self(scopes) }
}

impl From<&ReservationConflict> for AuthorizedOverlap {
    fn from(conflict: &ReservationConflict) -> Self {
        Self {
            reservation_id: conflict.reservation_id,
            scope_revision: conflict.overlap_scope_revision.clone(),
            scopes:         conflict.overlapping_scopes.clone().into(),
        }
    }
}

impl AuthorizedOverlap {
    pub(super) fn covers(
        &self,
        counterpart_id: ReservationId,
        counterpart_scope_revision: &OverlapScopeRevision,
        overlap_scope: &ReservationScope,
        path_case: PathCase,
    ) -> bool {
        self.reservation_id == counterpart_id
            && self.scope_revision == *counterpart_scope_revision
            && self.scopes.covers(overlap_scope, path_case)
    }
}

impl AuthorizedOverlapSet {
    /// Borrow the bindings without weakening the non-empty boundary.
    pub(crate) fn as_slice(&self) -> &[AuthorizedOverlap] { &self.0 }
}

impl From<AuthorizedOverlap> for AuthorizedOverlapSet {
    fn from(overlap: AuthorizedOverlap) -> Self { Self(vec![overlap]) }
}

impl TryFrom<Vec<AuthorizedOverlap>> for AuthorizedOverlapSet {
    type Error = EmptyAuthorizedOverlapSet;

    fn try_from(overlaps: Vec<AuthorizedOverlap>) -> Result<Self, Self::Error> {
        if overlaps.is_empty() {
            Err(EmptyAuthorizedOverlapSet)
        } else {
            Ok(Self(overlaps))
        }
    }
}

impl<'de> Deserialize<'de> for AuthorizedOverlapSet {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let overlaps = Vec::<AuthorizedOverlap>::deserialize(deserializer)?;
        Self::try_from(overlaps).map_err(serde::de::Error::custom)
    }
}

/// An error returned when an authorization contains no holder bindings.
#[derive(Debug)]
pub(crate) struct EmptyAuthorizedOverlapSet;

impl fmt::Display for EmptyAuthorizedOverlapSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an authorized overlap set cannot be empty")
    }
}

impl std::error::Error for EmptyAuthorizedOverlapSet {}
