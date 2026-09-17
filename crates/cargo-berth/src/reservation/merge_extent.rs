//! Branch merge protection, kept independently of a run's declared editing scope.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::drift::WorkingTreeFingerprint;
use crate::ids::GitObjectId;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;

/// Every repository fact whose movement can change the branch's merge surface.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct MergeExtentKey {
    /// Trunk movement can integrate an otherwise unchanged holder.
    #[schemars(with = "String")]
    pub(crate) trunk:        GitObjectId,
    /// The branch tip whose net change will reach trunk.
    #[schemars(with = "String")]
    pub(crate) head:         GitObjectId,
    /// Uncommitted paths independently extend that net change.
    pub(crate) working_tree: WorkingTreeFingerprint,
}

impl MergeExtentKey {
    /// Whether the observation taken under this key found nothing at all left to integrate.
    ///
    /// An empty derivation already means `git::unmerged_branch_paths` returned no path, but that
    /// is a statement about paths. This is the stronger statement the key itself carries: the
    /// branch tip was trunk, and [`WorkingTreeFingerprint`] held no tracked and no untracked path.
    /// A holder proved that and then lost its worktree has no work anywhere for a later
    /// observation to find.
    pub(crate) fn proves_nothing_outstanding(&self) -> bool {
        self.head == self.trunk
            && self.working_tree.tracked_paths.is_empty()
            && self.working_tree.untracked_paths.is_empty()
    }
}

/// Evidence retained through failure, distinguishing an initial declaration from a successful
/// observation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum RetainedMergeEvidence {
    /// No Git observation has succeeded; keep the acquisition's conservative protection.
    NotDerived {
        /// The immutable acquisition scope protects until the first successful observation.
        protection: ReservationScopeSet,
    },
    /// A completed observation proved that the branch held no unmerged paths.
    Empty {
        /// The exact inputs that proved this empty answer.
        key: MergeExtentKey,
    },
    /// A completed observation established these exact unmerged paths.
    Protected {
        /// The exact inputs that established the protected surface.
        key:    MergeExtentKey,
        /// Net branch and dirty paths, independent of the run's declaration.
        scopes: ReservationScopeSet,
    },
}

/// The branch's independently observed merge protection, including inability to answer.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum MergeExtent {
    /// Replay of an acquisition predating derivation must still protect its declaration.
    NotDerived {
        /// The immutable acquisition scope protects until the first successful observation.
        protection: ReservationScopeSet,
    },
    /// Successful emptiness ends merge protection, and ends the run once it has done work.
    Empty {
        /// The exact inputs that proved this empty answer.
        key: MergeExtentKey,
    },
    /// Exactly the net branch change and its currently dirty paths are protected.
    Protected {
        /// The exact inputs that established the protected surface.
        key:    MergeExtentKey,
        /// Net branch and dirty paths, independent of the run's declaration.
        scopes: ReservationScopeSet,
    },
    /// Failure preserves the last answer and explains why no fresh answer is available.
    Unavailable {
        /// The preceding answer, or explicit evidence that none has yet succeeded.
        retained_evidence: RetainedMergeEvidence,
        /// Why the current observation could not establish a replacement answer.
        failure:           String,
    },
}

/// Whether the selected refusal ground currently protects any paths.
pub(crate) enum ReservationProtection<'scopes> {
    /// This ground refuses nothing.
    Clear,
    /// This ground protects this complete nonempty scope set.
    Protected(&'scopes ReservationScopeSet),
}

impl MergeExtent {
    /// Normalize a successfully derived surface, admitting an ordinary empty answer.
    pub(crate) fn derived(key: MergeExtentKey, scopes: Vec<ReservationScope>) -> Self {
        match ReservationScopeSet::try_from(scopes) {
            Ok(scopes) => Self::Protected { key, scopes },
            Err(_) => Self::Empty { key },
        }
    }

    /// Only an observation of all three unchanged inputs may reuse a successful answer.
    pub(crate) fn matches_key(&self, expected: &MergeExtentKey) -> bool {
        match self {
            Self::Empty { key } | Self::Protected { key, .. } => key == expected,
            Self::NotDerived { .. } | Self::Unavailable { .. } => false,
        }
    }

    /// Preserve evidence across failure without reading or growing the editing scope.
    pub(crate) fn unavailable(&self, failure: String) -> Self {
        let retained_evidence = match self {
            Self::NotDerived { protection } => RetainedMergeEvidence::NotDerived {
                protection: protection.clone(),
            },
            Self::Empty { key } => RetainedMergeEvidence::Empty { key: key.clone() },
            Self::Protected { key, scopes } => RetainedMergeEvidence::Protected {
                key:    key.clone(),
                scopes: scopes.clone(),
            },
            Self::Unavailable {
                retained_evidence, ..
            } => retained_evidence.clone(),
        };
        Self::Unavailable {
            retained_evidence,
            failure,
        }
    }

    /// Whether a completed observation last found unmerged paths, including through failure.
    ///
    /// An acquisition's declaration is not observed work, so `NotDerived` answers false.
    pub(crate) const fn observed_unmerged_work(&self) -> bool {
        matches!(
            self,
            Self::Protected { .. }
                | Self::Unavailable {
                    retained_evidence: RetainedMergeEvidence::Protected { .. },
                    ..
                }
        )
    }

    /// Read only this ground's own protection, including retained failure evidence.
    pub(crate) const fn protection(&self) -> ReservationProtection<'_> {
        match self {
            Self::Empty { .. }
            | Self::Unavailable {
                retained_evidence: RetainedMergeEvidence::Empty { .. },
                ..
            } => ReservationProtection::Clear,
            Self::NotDerived { protection }
            | Self::Unavailable {
                retained_evidence: RetainedMergeEvidence::NotDerived { protection },
                ..
            } => ReservationProtection::Protected(protection),
            Self::Protected { scopes, .. }
            | Self::Unavailable {
                retained_evidence: RetainedMergeEvidence::Protected { scopes, .. },
                ..
            } => ReservationProtection::Protected(scopes),
        }
    }

    /// The key of a completed observation that proved emptiness, including through a later failure.
    ///
    /// `NotDerived` has no key because no observation has succeeded, and a `Protected` ground
    /// proved the opposite, so both answer `None` and no caller can read a key that did not come
    /// from a proof of emptiness.
    pub(crate) const fn proved_empty_key(&self) -> Option<&MergeExtentKey> {
        match self {
            Self::Empty { key }
            | Self::Unavailable {
                retained_evidence: RetainedMergeEvidence::Empty { key },
                ..
            } => Some(key),
            Self::NotDerived { .. } | Self::Protected { .. } | Self::Unavailable { .. } => None,
        }
    }

    /// Whether "retains its previous merge protection" is true of this ground.
    ///
    /// [`Self::protection`] owns the mapping from evidence to refusal, so this asks it rather than
    /// reading the variants a second time. A failure over `RetainedMergeEvidence::Empty` retains a
    /// completed observation that the branch held no unmerged paths, which is retained emptiness
    /// and not retained protection, so `Alert::MergeExtentUnavailable` must not be raised for it.
    pub(crate) const fn retains_protection(&self) -> bool {
        matches!(self.protection(), ReservationProtection::Protected(_))
    }
}
