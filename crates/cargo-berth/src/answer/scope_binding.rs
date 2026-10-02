//! Scope-only revisions and non-empty overlap bindings.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Write as _;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use sha2::Sha256;

use crate::ids::ReservationId;
use crate::ledger::ReservationScope;
use crate::ledger::ReservationScopeSet;
use crate::ledger::ScopeKind;
use crate::reservation::ReservationConflict;
use crate::scope::PathCase;

/// The length of a revision digest in lowercase hexadecimal digits.
const REVISION_DIGEST_HEX_LENGTH: usize = 64;

/// A deterministic revision that changes only when a reservation's scopes change.
///
/// The revision is the SHA-256 digest of the canonically ordered scopes, so a record naming a
/// holder's revision stays the same size however many scopes that holder protects.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct OverlapScopeRevision(#[schemars(pattern(r"^[0-9a-f]{64}$"))] String);

/// The recorded forms of an overlap scope revision.
#[derive(Deserialize)]
#[serde(untagged)]
enum RecordedOverlapScopeRevision {
    /// The digest written by current binaries.
    Digest(String),
    /// The canonical scope list written before the revision became a digest.
    Scopes(Vec<ReservationScope>),
}

/// The non-empty normalized scopes covered for one holder.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct AuthorizedOverlapScopeSet(ReservationScopeSet);

/// One exact holder and scope revision covered by an authorization.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct AuthorizedOverlap {
    /// The existing holder named by the authorization.
    pub(crate) reservation_id: ReservationId,
    /// The holder's scope-only revision when the authorization was shown.
    pub(crate) scope_revision: OverlapScopeRevision,
    /// The normalized overlap scopes that this answer covers.
    pub(crate) scopes:         AuthorizedOverlapScopeSet,
}

/// A non-empty set of holder-specific overlap bindings.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct AuthorizedOverlapSet(#[schemars(length(min = 1))] Vec<AuthorizedOverlap>);

impl From<&ReservationScopeSet> for OverlapScopeRevision {
    fn from(scopes: &ReservationScopeSet) -> Self { Self::of_scopes(scopes.as_slice()) }
}

impl OverlapScopeRevision {
    /// Digest scopes in path-then-kind order, so the revision ignores their listed order.
    fn of_scopes(scopes: &[ReservationScope]) -> Self {
        let mut canonical_scopes = scopes
            .iter()
            .map(|scope| (scope.path.to_string(), scope.kind))
            .collect::<Vec<_>>();
        canonical_scopes.sort();
        let mut hasher = Sha256::new();
        for (path, kind) in &canonical_scopes {
            hasher.update(match kind {
                ScopeKind::File => b"file\0",
                ScopeKind::Tree => b"tree\0",
            });
            hasher.update(path.as_bytes());
            hasher.update(b"\0");
        }
        let mut digest = String::with_capacity(REVISION_DIGEST_HEX_LENGTH);
        for byte in hasher.finalize() {
            let _ = write!(digest, "{byte:02x}");
        }
        Self(digest)
    }
}

impl<'de> Deserialize<'de> for OverlapScopeRevision {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        match RecordedOverlapScopeRevision::deserialize(deserializer)? {
            RecordedOverlapScopeRevision::Digest(digest) => {
                if digest.len() == REVISION_DIGEST_HEX_LENGTH
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    Ok(Self(digest))
                } else {
                    Err(serde::de::Error::custom(
                        "an overlap scope revision must be 64 lowercase hexadecimal digits",
                    ))
                }
            },
            RecordedOverlapScopeRevision::Scopes(scopes) if scopes.is_empty() => Err(
                serde::de::Error::custom("an overlap scope revision cannot be empty"),
            ),
            RecordedOverlapScopeRevision::Scopes(scopes) => Ok(Self::of_scopes(&scopes)),
        }
    }
}

impl AuthorizedOverlapScopeSet {
    /// Borrow the validated non-empty overlap scopes.
    pub(crate) fn as_slice(&self) -> &[ReservationScope] { self.0.as_slice() }

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
        self.scope_revision == *counterpart_scope_revision
            && self.covers_shared_scope(counterpart_id, overlap_scope, path_case)
    }

    /// Match recorded shared work independently of unrelated holder scope changes.
    pub(super) fn covers_shared_scope(
        &self,
        counterpart_id: ReservationId,
        overlap_scope: &ReservationScope,
        path_case: PathCase,
    ) -> bool {
        self.reservation_id == counterpart_id && self.scopes.covers(overlap_scope, path_case)
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

impl Display for EmptyAuthorizedOverlapSet {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("an authorized overlap set cannot be empty")
    }
}

impl Error for EmptyAuthorizedOverlapSet {}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::OverlapScopeRevision;
    use crate::ledger::ReservationScope;
    use crate::ledger::ReservationScopeSet;
    use crate::ledger::ScopeKind;

    #[test]
    fn recorded_scope_lists_decode_to_the_digest_of_their_scopes() -> Result<(), Box<dyn Error>> {
        let revision = OverlapScopeRevision::from(&ReservationScopeSet::try_from(vec![
            ReservationScope {
                path: "src".parse()?,
                kind: ScopeKind::Tree,
            },
            ReservationScope {
                path: "Cargo.toml".parse()?,
                kind: ScopeKind::File,
            },
        ])?);
        let digest = serde_json::to_value(&revision)?;
        assert_eq!(digest.as_str().map(str::len), Some(64));
        assert_eq!(
            serde_json::from_value::<OverlapScopeRevision>(digest)?,
            revision
        );

        let recorded = serde_json::from_value::<OverlapScopeRevision>(json!([
            {"path": "src", "kind": "tree"},
            {"path": "Cargo.toml", "kind": "file"},
        ]))?;
        assert_eq!(recorded, revision);

        assert!(serde_json::from_value::<OverlapScopeRevision>(json!([])).is_err());
        assert!(serde_json::from_value::<OverlapScopeRevision>(json!("ABC")).is_err());
        Ok(())
    }
}
