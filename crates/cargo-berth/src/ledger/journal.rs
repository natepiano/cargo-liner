//! The append-only journal and its complete version-one operation union.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::num::TryFromIntError;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::constants::CURRENT_SCHEMA_VERSION;
use super::constants::DELETE_CONTROL_BYTE;
use super::constants::MAXIMUM_JOURNAL_RECORD_BYTES;
use super::constants::MINIMUM_SUPPORTED_SCHEMA_VERSION;
use crate::answer::ConflictAuthorization;
use crate::config::InitializationState;
use crate::edge::OrderingReason;
use crate::ids::CoordinationRunId;
use crate::ids::EdgeId;
use crate::ids::EventId;
use crate::ids::ForcedIntegrationPermitId;
use crate::ids::GitObjectId;
use crate::ids::InvalidUuidV7;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RecordedAt;
use crate::ids::RepoInstanceId;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ids::SchemaVersion;
use crate::ids::WorkPlanPhase;
use crate::ids::WorktreeId;
use crate::reservation::EditBlockingStatus;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;

/// One append-only fact in the shared coordination journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct JournalEvent {
    /// The schema version required to interpret this fact.
    pub(super) schema_version:        SchemaVersion,
    /// The non-recyclable identity of this append.
    pub(super) event_id:              EventId,
    /// The coordination actor that recorded this fact.
    pub(crate) actor:                 JournalActor,
    /// The time this fact was recorded.
    pub(super) at:                    RecordedAt,
    /// The cache generation this append publishes.
    pub(super) projection_generation: ProjectionGeneration,
    /// The state transition this fact records.
    #[serde(flatten)]
    pub(crate) operation:             JournalOperation,
}

/// The version field that determines whether this binary can decode a journal record.
#[derive(Deserialize)]
struct JournalSchemaHeader {
    schema_version: SchemaVersion,
}

impl JournalEvent {
    /// Build a new v1 journal fact for one mutation transaction.
    pub(super) fn for_operation(
        actor: JournalActor,
        projection_generation: ProjectionGeneration,
        operation: JournalOperation,
    ) -> Self {
        Self {
            schema_version: SchemaVersion::from(CURRENT_SCHEMA_VERSION),
            event_id: EventId::new(),
            actor,
            at: RecordedAt::now(),
            projection_generation,
            operation,
        }
    }

    /// Return the durable identity of this journal fact.
    pub(crate) const fn event_id(&self) -> EventId { self.event_id }

    /// Return when this journal fact was recorded.
    pub(crate) const fn recorded_at(&self) -> &RecordedAt { &self.at }
}

/// The durable identity of the actor that made a journal mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct JournalActor {
    /// The clone-wide opaque repository identity.
    pub(crate) repository: RepoInstanceId,
    /// The opaque identity of the worktree making the mutation.
    pub(crate) worktree:   WorktreeId,
    /// The active coordination run in that worktree.
    pub(crate) run:        CoordinationRunId,
}

/// The opaque UUID-v7 identity of one incursion incident.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct IncursionIncidentId(Uuid);

impl IncursionIncidentId {
    /// Mint a new non-recyclable incident identity.
    pub(crate) fn new() -> Self { Self(Uuid::now_v7()) }
}

impl Display for IncursionIncidentId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { self.0.fmt(formatter) }
}

impl FromStr for IncursionIncidentId {
    type Err = InvalidUuidV7;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let identifier = value.parse::<Uuid>().map_err(InvalidUuidV7::InvalidUuid)?;
        if identifier.get_version_num() != 7 {
            return Err(InvalidUuidV7::WrongVersion(identifier));
        }
        if identifier.get_variant() != uuid::Variant::RFC4122 {
            return Err(InvalidUuidV7::WrongVariant(identifier));
        }
        Ok(Self(identifier))
    }
}

impl Serialize for IncursionIncidentId {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for IncursionIncidentId {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// Every v1 operation a journal can contain.
///
/// New behavior must use one of these variants. Older binaries reject an
/// unknown operation rather than silently replaying an incomplete state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum JournalOperation {
    /// Acquire a new reservation and any conflict answer that authorized it.
    Claim {
        /// The newly minted reservation identity.
        reservation_id:                  ReservationId,
        /// The paths claimed atomically by this reservation.
        scopes:                          ReservationScopeSet,
        /// How the claimant described the work that needs these paths.
        source:                          ClaimSource,
        /// The claimant's non-empty explanation of the work being protected.
        purpose:                         ReservationPurpose,
        /// The trunk commit against which later movement is measured.
        trunk_at_claim:                  TrunkCommitAtClaim,
        /// The branch or detached head observed when the reservation was acquired.
        head_snapshot:                   ClaimHeadSnapshot,
        /// The phase-start commit protected for later drift comparison.
        phase_start_head:                ProtectedPhaseStartHead,
        /// The canonical root used to validate the worktree during reconciliation.
        worktree_root:                   CanonicalWorktreeRoot,
        /// The locator used to find the worktree's administrative directory again.
        worktree_administrative_locator: WorktreeAdministrativeLocator,
        /// The overlap result that authorized this acquisition.
        authorization:                   ConflictAuthorization,
    },
    /// Enlarge an existing reservation and any conflict answer that authorized it.
    Widen {
        /// The reservation receiving additional scopes.
        reservation_id:       ReservationId,
        /// The non-empty complete scopes added by this mutation.
        added_scopes:         ReservationScopeAdditionSet,
        /// Why the footprint expanded.
        cause:                WidenCause,
        /// The overlap result that authorized this widening.
        authorization:        ConflictAuthorization,
        /// The edit decision resulting from the enlarged footprint.
        edit_blocking_status: EditBlockingStatus,
    },
    /// Record the phase result used to protect an outstanding reservation.
    Checkpoint {
        /// The reservation entering its outstanding state.
        reservation_id: ReservationId,
        /// The completed work's protected commit.
        protected_tip:  ProtectedReservationTip,
        /// The trunk commit observed at the checkpoint.
        trunk_snapshot: GitObjectId,
    },
    /// Replace the comparison points after a rebase or trunk rewrite.
    Resnapshot {
        /// The reservation whose comparison data changed.
        reservation_id: ReservationId,
        /// The replacement state-specific comparison points.
        snapshot:       ReservationSnapshot,
    },
    /// Mark a still-live reservation as recently active.
    Renew {
        /// The reservation receiving this renewal.
        reservation_id: ReservationId,
    },
    /// Record a confirmed terminal disposition for a reservation.
    Release {
        /// The reservation receiving this disposition.
        reservation_id: ReservationId,
        /// The verified or user-confirmed outcome of this release.
        disposition:    ReleaseDisposition,
    },
    /// Replace a released reservation's disposition after its git evidence was invalidated.
    ReplaceReleaseDisposition {
        /// The reservation receiving corrected rewritten-integration evidence.
        reservation_id: ReservationId,
        /// The disposition retained as immutable history before this correction.
        superseded:     ReleaseDisposition,
        /// The newly verified disposition used by current replay state.
        replacement:    ReleaseDisposition,
    },
    /// Materialize a git evidence result for mutation-free edit checks.
    EvidenceRevalidated {
        /// The reservation whose evidence was revalidated.
        reservation_id:       ReservationId,
        /// The point-in-time result produced by git.
        status:               IntegrationEvidenceStatus,
        /// The edit decision produced when this evidence was recorded.
        edit_blocking_status: EditBlockingStatus,
    },
    /// Convert a previously recorded defer answer into an ordering edge.
    ResolveDefer {
        /// The reservation that had deferred the overlap decision.
        deferred_reservation_id: ReservationId,
        /// The reservation it deferred against.
        blocker_reservation_id:  ReservationId,
        /// The durable edge created by resolving the deferral.
        edge_id:                 EdgeId,
        /// The ordering chosen for the two reservations.
        direction:               OrderingDirection,
        /// The reason for choosing this ordering now.
        reason:                  OrderingReason,
    },
    /// Record a write that entered scopes reserved by another worktree.
    Incursion {
        /// The durable identity used to answer this incident.
        incident_id:             IncursionIncidentId,
        /// The reservation whose worktree made the write.
        reservation_id:          ReservationId,
        /// The foreign reservations whose scopes were entered.
        foreign_reservation_ids: ForeignReservationIdSet,
        /// The paths written without coverage.
        paths:                   IncursionPathSet,
    },
    /// Record the user disposition that answers one incursion incident.
    ResolveIncursion {
        /// The incident leaving outstanding state.
        incident_id: IncursionIncidentId,
    },
    /// Issue a one-use permit for a confirmed forced integration.
    ForcedIntegrationPermit {
        /// The opaque identity of the one-use permit.
        permit_id:      ForcedIntegrationPermitId,
        /// The reservation allowed to integrate past a hold.
        reservation_id: ReservationId,
        /// The reason the user accepted this exception.
        reason:         ForcedIntegrationReason,
        /// Every ordering or deferral hold this one update may skip.
        skipped_holds:  SkippedIntegrationHoldSet,
    },
    /// Consume a previously issued forced-integration permit.
    ConsumeForcedIntegrationPermit {
        /// The permit that cannot be used again.
        permit_id:      ForcedIntegrationPermitId,
        /// The reservation that consumed it.
        reservation_id: ReservationId,
    },
    /// Record an explicit escape-hatch bypass without changing edge state.
    Bypass {
        /// The action permitted outside normal ledger validation.
        action:          BypassedAction,
        /// The typed release valve that permitted the action.
        cause:           BypassCause,
        /// When the bypass occurred, kept distinct from a delayed marker import.
        #[serde(default)]
        occurrence_time: BypassOccurrenceTime,
        /// Whether this record was written directly or recovered from one marker.
        #[serde(default)]
        recording:       BypassRecording,
    },
    /// Move a reservation's ownership to a replacement worktree.
    RebindWorktree {
        /// The recovered reservation.
        reservation_id:                          ReservationId,
        /// The opaque worktree identity that no longer holds the work.
        previous_worktree_id:                    WorktreeId,
        /// The opaque worktree identity now holding the work.
        current_worktree_id:                     WorktreeId,
        /// The replacement worktree's canonical root.
        current_worktree_root:                   CanonicalWorktreeRoot,
        /// The replacement worktree's administrative locator.
        current_worktree_administrative_locator: WorktreeAdministrativeLocator,
    },
    /// Update a moved worktree's root while preserving its opaque identity.
    RelocateWorktree {
        /// The reservation whose holder moved.
        reservation_id: ReservationId,
        /// The unchanged opaque worktree identity.
        worktree_id:    WorktreeId,
        /// The root recorded by the preceding claim, rebind, or relocation.
        previous_root:  CanonicalWorktreeRoot,
        /// The canonical root now linked from the same administrative directory.
        current_root:   CanonicalWorktreeRoot,
    },
}

/// How a claim named the work it reserves.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ClaimSource {
    /// A reservation supplied by an external work-plan integration.
    WorkPlan {
        /// The plan's identifying path.
        plan:  WorkPlanReference,
        /// The plan-local opaque phase label.
        phase: WorkPlanPhase,
    },
    /// A reservation acquired atomically when an edit first touched its paths.
    FirstTouch,
    /// A direct caller-specified reservation.
    Explicit,
}

macro_rules! git_commit_role {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(GitObjectId);

        impl From<GitObjectId> for $name {
            fn from(object_id: GitObjectId) -> Self { Self(object_id) }
        }

        impl AsRef<GitObjectId> for $name {
            fn as_ref(&self) -> &GitObjectId { &self.0 }
        }

        impl FromStr for $name {
            type Err = crate::ids::InvalidGitObjectId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse::<GitObjectId>().map(Self)
            }
        }
    };
}

git_commit_role!(
    TrunkCommitAtClaim,
    "The trunk commit observed when a reservation was acquired."
);
git_commit_role!(
    ClaimHeadCommit,
    "The worktree HEAD commit observed when a reservation was acquired."
);
git_commit_role!(
    ProtectedPhaseStartHead,
    "The phase-start commit retained for active-work drift comparison."
);

macro_rules! normalize_nonempty_claim_text {
    (trim, $value:ident) => {
        $value.trim()
    };
    (preserve, $value:ident) => {
        $value
    };
}

macro_rules! nonempty_claim_text {
    (
        $normalization:ident,
        $name:ident,
        $error:ident,
        $documentation:literal,
        $error_message:literal
    ) => {
        #[doc = $documentation]
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub(crate) struct $name(String);

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = $error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let value = normalize_nonempty_claim_text!($normalization, value);
                if value.is_empty() {
                    Err($error)
                } else {
                    Ok(Self(value.to_owned()))
                }
            }
        }

        impl Serialize for $name {
            fn serialize<SerializerType>(
                &self,
                serializer: SerializerType,
            ) -> Result<SerializerType::Ok, SerializerType::Error>
            where
                SerializerType: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<DeserializerType>(
                deserializer: DeserializerType,
            ) -> Result<Self, DeserializerType::Error>
            where
                DeserializerType: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(serde::de::Error::custom)
            }
        }

        #[doc = concat!("An error returned when constructing `", stringify!($name), "` from empty text.")]
        #[derive(Debug)]
        pub(crate) struct $error;

        impl fmt::Display for $error {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str($error_message)
            }
        }

        impl std::error::Error for $error {}
    };
}

nonempty_claim_text!(
    trim,
    NonEmptyReservationPurpose,
    EmptyNonEmptyReservationPurpose,
    "A non-empty explanation of the work protected by a reservation.",
    "a reservation purpose cannot be empty"
);
nonempty_claim_text!(
    trim,
    ForcedIntegrationReason,
    EmptyForcedIntegrationReason,
    "A non-empty explanation for one forced integration.",
    "a forced-integration reason cannot be empty"
);
nonempty_claim_text!(
    trim,
    ExplicitWidenReason,
    EmptyExplicitWidenReason,
    "A non-empty explanation for one explicit reservation widening.",
    "an explicit widen reason cannot be empty"
);
nonempty_claim_text!(
    preserve,
    WorkPlanReference,
    EmptyWorkPlanReference,
    "An opaque, non-empty reference to the work plan that originated a claim.",
    "a work-plan reference cannot be empty"
);

/// Whether the caller supplied an explanation for the protected work.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "explanation", rename_all = "snake_case")]
pub(crate) enum ReservationPurpose {
    /// The caller supplied a non-empty explanation.
    Explained(NonEmptyReservationPurpose),
    /// The caller omitted `--why`.
    NotProvidedByCaller,
}

/// The full branch reference and commit, or detached commit, observed at claim time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ClaimHeadSnapshot {
    /// The worktree was attached to a branch.
    Branch {
        /// The full `refs/...` name, retained without short-name ambiguity.
        full_ref: FullRefName,
        /// The commit to which the branch resolved.
        head:     ClaimHeadCommit,
    },
    /// The worktree had a detached HEAD.
    Detached {
        /// The detached commit.
        head: ClaimHeadCommit,
    },
}

/// A non-empty full git reference name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FullRefName(String);

impl Display for FullRefName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { formatter.write_str(&self.0) }
}

impl FromStr for FullRefName {
    type Err = InvalidFullRefName;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let suffix = value.strip_prefix("refs/").unwrap_or_default();
        let has_disallowed_character = value.bytes().any(|byte| {
            byte <= b' '
                || byte == DELETE_CONTROL_BYTE
                || matches!(byte, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
        });
        let has_invalid_component = suffix.split('/').any(|component| {
            let has_lock_extension = Path::new(component)
                .extension()
                .is_some_and(|extension| extension == "lock");
            component.is_empty() || component.starts_with('.') || has_lock_extension
        });
        if suffix.is_empty()
            || has_invalid_component
            || value.contains("..")
            || value.contains("@{")
            || value.ends_with('.')
            || has_disallowed_character
        {
            Err(InvalidFullRefName)
        } else {
            Ok(Self(value.to_owned()))
        }
    }
}

impl Serialize for FullRefName {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for FullRefName {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// An error returned when a full git reference name is empty or not rooted at `refs/`.
#[derive(Debug)]
pub(crate) struct InvalidFullRefName;

impl Display for InvalidFullRefName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "a full git reference name must begin with refs/ and satisfy git reference rules",
        )
    }
}

impl std::error::Error for InvalidFullRefName {}

/// The declared file-versus-tree meaning of one reserved repository path.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScopeKind {
    /// Reserve exactly one path.
    File,
    /// Reserve a path and all component descendants.
    Tree,
}

/// One repository path paired with its declared reservation semantics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ReservationScope {
    /// The normalized repository-relative path.
    pub(crate) path: ReservationScopePath,
    /// Whether the path denotes one file or a whole tree.
    pub(crate) kind: ScopeKind,
}

/// The non-empty atomic footprint protected by one reservation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct ReservationScopeSet(Vec<ReservationScope>);

impl ReservationScopeSet {
    /// Borrow the scopes without weakening the non-empty construction boundary.
    pub(crate) fn as_slice(&self) -> &[ReservationScope] { &self.0 }
}

impl TryFrom<Vec<ReservationScope>> for ReservationScopeSet {
    type Error = EmptyReservationScopeSet;

    fn try_from(scopes: Vec<ReservationScope>) -> Result<Self, Self::Error> {
        if scopes.is_empty() {
            Err(EmptyReservationScopeSet)
        } else {
            Ok(Self(scopes))
        }
    }
}

impl<'de> Deserialize<'de> for ReservationScopeSet {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let scopes = Vec::<ReservationScope>::deserialize(deserializer)?;
        Self::try_from(scopes).map_err(serde::de::Error::custom)
    }
}

/// An error returned when a reservation footprint contains no scopes.
#[derive(Debug)]
pub(crate) struct EmptyReservationScopeSet;

impl Display for EmptyReservationScopeSet {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a reservation scope set cannot be empty")
    }
}

impl std::error::Error for EmptyReservationScopeSet {}

macro_rules! nonempty_journal_set {
    ($name:ident, $item:ty, $error:ident, $documentation:literal, $error_message:literal) => {
        #[doc = $documentation]
        #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(Vec<$item>);

        impl $name {
            #[doc = concat!("Borrow the values in this `", stringify!($name), "`.")]
            pub(crate) fn as_slice(&self) -> &[$item] { &self.0 }
        }

        impl TryFrom<Vec<$item>> for $name {
            type Error = $error;

            fn try_from(values: Vec<$item>) -> Result<Self, Self::Error> {
                if values.is_empty() {
                    Err($error)
                } else {
                    Ok(Self(values))
                }
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<DeserializerType>(
                deserializer: DeserializerType,
            ) -> Result<Self, DeserializerType::Error>
            where
                DeserializerType: serde::Deserializer<'de>,
            {
                let values = Vec::<$item>::deserialize(deserializer)?;
                Self::try_from(values).map_err(serde::de::Error::custom)
            }
        }

        #[doc = concat!("An error returned when constructing an empty `", stringify!($name), "`.")]
        #[derive(Debug)]
        pub(crate) struct $error;

        impl Display for $error {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str($error_message)
            }
        }

        impl std::error::Error for $error {}
    };
}

nonempty_journal_set!(
    ReservationScopeAdditionSet,
    ReservationScope,
    EmptyReservationScopeAdditionSet,
    "A non-empty set of complete scopes added by one widening.",
    "a reservation scope addition set cannot be empty"
);
nonempty_journal_set!(
    ForeignReservationIdSet,
    ReservationId,
    EmptyForeignReservationIdSet,
    "The non-empty foreign-holder set proven by one incursion.",
    "an incursion must name at least one foreign reservation"
);
nonempty_journal_set!(
    IncursionPathSet,
    ReservationScopePath,
    EmptyIncursionPathSet,
    "The non-empty repository path set entered by one incursion.",
    "an incursion must name at least one path"
);
nonempty_journal_set!(
    CollisionPathSet,
    ReservationScopePath,
    EmptyCollisionPathSet,
    "The non-empty repository path set refused by one drift collision.",
    "a collision must name at least one path"
);

/// A canonical, absolute, UTF-8 worktree root stored for identity validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalWorktreeRoot(String);

impl Display for CanonicalWorktreeRoot {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { formatter.write_str(&self.0) }
}

impl AsRef<Path> for CanonicalWorktreeRoot {
    fn as_ref(&self) -> &Path { Path::new(&self.0) }
}

impl FromStr for CanonicalWorktreeRoot {
    type Err = InvalidCanonicalWorktreeRoot;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let path = Path::new(value);
        let normalized: PathBuf = path.components().collect();
        let has_only_absolute_components = path.components().all(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        });
        let has_normalized_spelling = normalized.to_str() == Some(value);
        if path.is_absolute() && has_normalized_spelling && has_only_absolute_components {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidCanonicalWorktreeRoot)
        }
    }
}

impl Serialize for CanonicalWorktreeRoot {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CanonicalWorktreeRoot {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// An error returned when a worktree root is not a canonical absolute path.
#[derive(Debug)]
pub(crate) struct InvalidCanonicalWorktreeRoot;

impl Display for InvalidCanonicalWorktreeRoot {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a canonical worktree root must be an absolute normalized UTF-8 path")
    }
}

impl std::error::Error for InvalidCanonicalWorktreeRoot {}

/// The common-directory-relative locator of a worktree administrative directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeAdministrativeLocator(String);

impl Display for WorktreeAdministrativeLocator {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { formatter.write_str(&self.0) }
}

impl FromStr for WorktreeAdministrativeLocator {
    type Err = InvalidWorktreeAdministrativeLocator;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let is_common_directory = value == ".";
        let path = Path::new(value);
        let normalized: PathBuf = path.components().collect();
        let has_only_relative_components = path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        let has_path_text = !value.is_empty();
        let is_relative_path = path.is_relative();
        let has_normalized_spelling = normalized.to_str() == Some(value);
        let is_linked_worktree = has_path_text
            && is_relative_path
            && has_normalized_spelling
            && has_only_relative_components;
        if is_common_directory || is_linked_worktree {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidWorktreeAdministrativeLocator)
        }
    }
}

impl Serialize for WorktreeAdministrativeLocator {
    fn serialize<SerializerType>(
        &self,
        serializer: SerializerType,
    ) -> Result<SerializerType::Ok, SerializerType::Error>
    where
        SerializerType: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for WorktreeAdministrativeLocator {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// An error returned when an administrative locator is not common-directory relative.
#[derive(Debug)]
pub(crate) struct InvalidWorktreeAdministrativeLocator;

impl Display for InvalidWorktreeAdministrativeLocator {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a worktree administrative locator must be normalized and relative")
    }
}

impl std::error::Error for InvalidWorktreeAdministrativeLocator {}

/// Why an existing reservation received more scopes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum WidenCause {
    /// Reconciliation observed paths not covered by the claim.
    Drift,
    /// The caller deliberately expanded the reservation.
    Explicit {
        /// The caller's explanation.
        reason: ExplicitWidenReason,
    },
}

/// The ordering direction selected for two conflicting reservations.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OrderingDirection {
    /// The requesting reservation must integrate before the holder.
    RequesterBeforeHolder,
    /// The holder must integrate before the requesting reservation.
    HolderBeforeRequester,
}

/// The state-specific data replaced by a resnapshot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub(crate) enum ReservationSnapshot {
    /// Fresh active-work comparison data.
    Active {
        /// The new phase-start commit.
        claim_snapshot: GitObjectId,
    },
    /// Fresh outstanding-work integration evidence.
    Outstanding {
        /// The reservation's current protected commit.
        protected_tip: ProtectedReservationTip,
        /// The current trunk comparison point.
        trunk_oid:     GitObjectId,
    },
}

/// The operation a bypass deliberately allowed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BypassedAction {
    /// An integration was allowed despite ordinary gate state.
    Integration,
    /// An editing decision was allowed despite an unreadable ledger.
    Editing,
}

/// Why ordinary integration validation was deliberately bypassed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum BypassCause {
    /// `CARGO_BERTH_BYPASS=1` was present before the hook read any ledger state.
    EnvironmentOverride {
        /// The bypassed git integration whose reference transactions share one audit row.
        bypassed_merge: BypassedMergeIdentity,
    },
    /// A one-use forced-integration permit authorized exactly its recorded holds.
    ForcedIntegration {
        /// The permit consumed by the bypassed update.
        permit_id: ForcedIntegrationPermitId,
        /// The user's non-empty explanation retained with the permit.
        reason:    ForcedIntegrationReason,
    },
}

/// The write-time identity shared by every reference transaction from one bypassed merge.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct BypassedMergeIdentity(String);

impl BypassedMergeIdentity {
    /// Retain a hook-supplied merge token that needs no shell or JSON escaping.
    pub(crate) fn from_hook_token(token: &str) -> Result<Self, InvalidBypassedMergeIdentity> {
        let token = token.trim();
        if token.is_empty() {
            return Err(InvalidBypassedMergeIdentity::Empty);
        }
        if let Some(character) = token
            .chars()
            .find(|character| !bypassed_merge_identity_character_is_supported(*character))
        {
            return Err(InvalidBypassedMergeIdentity::UnsupportedCharacter(
                character,
            ));
        }
        Ok(Self(token.to_owned()))
    }
}

impl From<CoordinationRunId> for BypassedMergeIdentity {
    fn from(run_id: CoordinationRunId) -> Self {
        let token = format!("direct-{run_id}");
        debug_assert!(
            token
                .chars()
                .all(bypassed_merge_identity_character_is_supported),
            "coordination-run bypass identities must satisfy the hook token character set"
        );
        Self(token)
    }
}

impl<'de> Deserialize<'de> for BypassedMergeIdentity {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let token = String::deserialize(deserializer)?;
        Self::from_hook_token(&token).map_err(serde::de::Error::custom)
    }
}

const fn bypassed_merge_identity_character_is_supported(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
}

/// Why a hook-supplied bypass merge identity was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InvalidBypassedMergeIdentity {
    /// The token contained no characters after trimming.
    Empty,
    /// The token contained a character outside ASCII alphanumeric, hyphen, and underscore.
    UnsupportedCharacter(char),
}

impl Display for InvalidBypassedMergeIdentity {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("a bypassed merge identity cannot be empty"),
            Self::UnsupportedCharacter(character) => write!(
                formatter,
                "a bypassed merge identity contains unsupported character {character:?}; use only ASCII letters, digits, hyphens, and underscores"
            ),
        }
    }
}

impl std::error::Error for InvalidBypassedMergeIdentity {}

/// The occurrence time retained for a directly written or recovered bypass.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum BypassOccurrenceTime {
    /// The journal event time is also the bypass occurrence time.
    #[default]
    EventRecordedAt,
    /// A pending marker retained the actual occurrence time.
    Known { at: RecordedAt },
    /// A fallback marker could not capture a parseable time.
    Unavailable,
}

/// The durable source identity used to deduplicate recovered marker imports.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum BypassRecording {
    /// The bypass reached the journal at the time it occurred.
    #[default]
    Direct,
    /// Reconciliation imported one common-directory marker.
    PendingMarker { marker_id: PendingBypassMarkerId },
}

/// A stable pending-bypass identity derived from its unique filename.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct PendingBypassMarkerId(String);

impl PendingBypassMarkerId {
    /// Retain a marker filename as its non-recycled import identity.
    pub(crate) const fn from_file_name(file_name: String) -> Self { Self(file_name) }

    /// Borrow the filename that serves as this marker's durable identity.
    pub(crate) fn file_name(&self) -> &str { &self.0 }
}

/// One ordering relationship deliberately skipped by a forced integration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SkippedOrderingEdge {
    /// The durable relationship that remained holding when the permit was issued.
    pub(crate) edge_id:     EdgeId,
    /// The predecessor whose ordered work had not yet been incorporated.
    pub(crate) predecessor: ReservationId,
}

/// One unresolved symmetric deferral deliberately skipped by a forced integration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SkippedDeferral {
    /// The claim event that first recorded the unresolved deferral.
    pub(crate) declaration_event_id: EventId,
    /// The reservation whose acquisition carried the defer answer.
    pub(crate) deferred:             ReservationId,
    /// The exact counterpart named by that answer.
    pub(crate) blocker:              ReservationId,
}

/// A non-empty set of integration holds authorized for one forced update.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SkippedIntegrationHoldSet {
    /// One or more ordering edges, with no unresolved deferral.
    OrderingEdges {
        /// The non-empty edge set.
        edges: Vec<SkippedOrderingEdge>,
    },
    /// One or more unresolved deferrals, with no ordering edge.
    Deferrals {
        /// The non-empty deferral set.
        deferrals: Vec<SkippedDeferral>,
    },
    /// Both kinds of hold were present.
    OrderingEdgesAndDeferrals {
        /// The non-empty edge set.
        edges:     Vec<SkippedOrderingEdge>,
        /// The non-empty deferral set.
        deferrals: Vec<SkippedDeferral>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SkippedIntegrationHoldRepresentation {
    OrderingEdges {
        edges: Vec<SkippedOrderingEdge>,
    },
    Deferrals {
        deferrals: Vec<SkippedDeferral>,
    },
    OrderingEdgesAndDeferrals {
        edges:     Vec<SkippedOrderingEdge>,
        deferrals: Vec<SkippedDeferral>,
    },
}

impl<'de> Deserialize<'de> for SkippedIntegrationHoldSet {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        match SkippedIntegrationHoldRepresentation::deserialize(deserializer)? {
            SkippedIntegrationHoldRepresentation::OrderingEdges { edges } => {
                Self::new(edges, Vec::new()).map_err(serde::de::Error::custom)
            },
            SkippedIntegrationHoldRepresentation::Deferrals { deferrals } => {
                Self::new(Vec::new(), deferrals).map_err(serde::de::Error::custom)
            },
            SkippedIntegrationHoldRepresentation::OrderingEdgesAndDeferrals {
                edges,
                deferrals,
            } if !edges.is_empty() && !deferrals.is_empty() => {
                Ok(Self::OrderingEdgesAndDeferrals { edges, deferrals })
            },
            SkippedIntegrationHoldRepresentation::OrderingEdgesAndDeferrals { .. } => {
                Err(serde::de::Error::custom(EmptySkippedIntegrationHoldSet))
            },
        }
    }
}

impl SkippedIntegrationHoldSet {
    /// Construct a non-empty typed set without representable empty or half-empty variants.
    pub(crate) fn new(
        edges: Vec<SkippedOrderingEdge>,
        deferrals: Vec<SkippedDeferral>,
    ) -> Result<Self, EmptySkippedIntegrationHoldSet> {
        match (edges.is_empty(), deferrals.is_empty()) {
            (false, true) => Ok(Self::OrderingEdges { edges }),
            (true, false) => Ok(Self::Deferrals { deferrals }),
            (false, false) => Ok(Self::OrderingEdgesAndDeferrals { edges, deferrals }),
            (true, true) => Err(EmptySkippedIntegrationHoldSet),
        }
    }

    /// Return whether this permit covers every supplied edge and deferral hold.
    pub(crate) fn covers(&self, edge_ids: &[EdgeId], deferral_event_ids: &[EventId]) -> bool {
        let (edges, deferrals): (&[SkippedOrderingEdge], &[SkippedDeferral]) = match self {
            Self::OrderingEdges { edges } => (edges, &[]),
            Self::Deferrals { deferrals } => (&[], deferrals),
            Self::OrderingEdgesAndDeferrals { edges, deferrals } => (edges, deferrals),
        };
        edge_ids.len() == edges.len()
            && edge_ids
                .iter()
                .all(|edge_id| edges.iter().any(|edge| edge.edge_id == *edge_id))
            && deferral_event_ids.len() == deferrals.len()
            && deferral_event_ids.iter().all(|event_id| {
                deferrals
                    .iter()
                    .any(|deferral| deferral.declaration_event_id == *event_id)
            })
    }
}

/// An attempted forced integration had no hold to bypass.
#[derive(Debug)]
pub(crate) struct EmptySkippedIntegrationHoldSet;

impl Display for EmptySkippedIntegrationHoldSet {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a forced-integration permit must skip at least one hold")
    }
}

impl std::error::Error for EmptySkippedIntegrationHoldSet {}

/// A replayed journal and the metadata needed to validate its cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct JournalReplay {
    /// Every fully parsed journal event in append order.
    pub(super) events:      Vec<JournalEvent>,
    /// The byte length of the repaired journal.
    pub(super) end_offset:  JournalByteOffset,
    /// A deterministic digest of the exact journal bytes.
    pub(super) fingerprint: JournalFingerprint,
    /// The generation represented by the final event, or zero for an empty journal.
    pub(super) generation:  ProjectionGeneration,
}

/// A deterministic fingerprint over a journal's bytes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(super) struct JournalFingerprint(pub(super) u64);

/// The journal file opened at its fixed ledger path.
pub(super) struct Journal {
    path: PathBuf,
}

impl Journal {
    /// Open the append-only journal, creating its empty file when initialization needs it.
    pub(super) fn open_or_create(path: &Path) -> Result<(Self, InitializationState), JournalError> {
        let initialization_state = if path.exists() {
            InitializationState::Existing
        } else {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)?;
            file.sync_all()?;
            InitializationState::Created
        };
        Ok((
            Self {
                path: path.to_owned(),
            },
            initialization_state,
        ))
    }

    /// Open an initialized journal without creating any missing ledger state.
    pub(super) fn open_existing(path: &Path) -> Result<Self, JournalError> {
        OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Self {
            path: path.to_owned(),
        })
    }

    /// Replay every complete record and repair one incomplete final record.
    pub(super) fn replay_repairing_tail(&self) -> Result<JournalReplay, JournalError> {
        let bytes = fs::read(&self.path)?;
        let (replay, complete_end) = replay_complete_records(&bytes)?;

        if complete_end != bytes.len() {
            let journal_file = OpenOptions::new().write(true).open(&self.path)?;
            journal_file
                .set_len(u64::try_from(complete_end).map_err(JournalError::JournalTooLarge)?)?;
            journal_file.sync_all()?;
        }
        Ok(replay)
    }

    /// Replay complete records without opening the journal for mutation.
    pub(super) fn replay_read_only(path: &Path) -> Result<JournalReplay, JournalError> {
        let bytes = fs::read(path)?;
        replay_complete_records(&bytes).map(|(replay, _)| replay)
    }

    /// Append exactly one complete JSON record and sync it before cache publication.
    pub(super) fn append(&self, event: &JournalEvent) -> Result<(), JournalAppendError> {
        let mut record = serde_json::to_vec(event).map_err(JournalAppendError::Serialization)?;
        record.push(b'\n');
        if record.len() > MAXIMUM_JOURNAL_RECORD_BYTES {
            return Err(JournalAppendError::RecordTooLarge {
                bytes: record.len(),
            });
        }

        let mut journal_file = OpenOptions::new().append(true).open(&self.path)?;
        journal_file.write_all(&record)?;
        journal_file.sync_all()?;
        Ok(())
    }

    /// Discard every journal byte after explicit reinitialization confirmation.
    pub(super) fn truncate(&self) -> Result<(), JournalError> {
        let journal_file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        journal_file.sync_all()?;
        Ok(())
    }
}

fn replay_complete_records(bytes: &[u8]) -> Result<(JournalReplay, usize), JournalError> {
    let complete_end = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let complete_records = &bytes[..complete_end];
    let records = complete_records.split(|byte| *byte == b'\n');
    let record_count = records.clone().count();
    let mut events = Vec::new();
    for (line_index, record) in records.enumerate() {
        if record.is_empty() {
            if line_index + 1 == record_count {
                continue;
            }
            return Err(JournalError::CorruptInteriorRecord {
                line:  line_index + 1,
                error: "blank journal record".to_owned(),
            });
        }
        let record =
            std::str::from_utf8(record).map_err(|error| JournalError::CorruptInteriorRecord {
                line:  line_index + 1,
                error: error.to_string(),
            })?;
        let schema_header =
            serde_json::from_str::<JournalSchemaHeader>(record).map_err(|error| {
                JournalError::CorruptInteriorRecord {
                    line:  line_index + 1,
                    error: error.to_string(),
                }
            })?;
        let minimum_schema_version = SchemaVersion::from(MINIMUM_SUPPORTED_SCHEMA_VERSION);
        let current_schema_version = SchemaVersion::from(CURRENT_SCHEMA_VERSION);
        if schema_header.schema_version < minimum_schema_version
            || schema_header.schema_version > current_schema_version
        {
            return Err(JournalError::UnsupportedSchemaVersion(
                schema_header.schema_version,
            ));
        }
        let event = serde_json::from_str::<JournalEvent>(record).map_err(|error| {
            JournalError::CorruptInteriorRecord {
                line:  line_index + 1,
                error: error.to_string(),
            }
        })?;
        events.push(event);
    }

    let generation = events.last().map_or_else(
        || ProjectionGeneration::from(0),
        |event| event.projection_generation,
    );
    let complete_bytes = &bytes[..complete_end];
    Ok((
        JournalReplay {
            events,
            end_offset: JournalByteOffset::from(
                u64::try_from(complete_end).map_err(JournalError::JournalTooLarge)?,
            ),
            fingerprint: JournalFingerprint::from_bytes(complete_bytes),
            generation,
        },
        complete_end,
    ))
}

impl JournalFingerprint {
    fn from_bytes(bytes: &[u8]) -> Self {
        const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
        const FNV_PRIME: u64 = 1_099_511_628_211;

        Self(bytes.iter().fold(FNV_OFFSET_BASIS, |fingerprint, byte| {
            (fingerprint ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        }))
    }
}

/// A journal failure that prevents a reliable replay.
#[derive(Debug)]
pub(crate) enum JournalError {
    /// Filesystem access failed.
    Io(std::io::Error),
    /// A complete interior record could not be decoded.
    CorruptInteriorRecord {
        /// The one-based record number that is invalid.
        line:  usize,
        /// The decoding failure.
        error: String,
    },
    /// A record names a schema this binary cannot safely interpret.
    UnsupportedSchemaVersion(SchemaVersion),
    /// The journal length cannot fit in the stored offset type.
    JournalTooLarge(TryFromIntError),
}

impl Display for JournalError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "journal I/O failed: {error}"),
            Self::CorruptInteriorRecord { line, error } => {
                write!(formatter, "journal record {line} is corrupt: {error}")
            },
            Self::UnsupportedSchemaVersion(version) => {
                write!(formatter, "journal schema version {version} is unsupported")
            },
            Self::JournalTooLarge(error) => write!(formatter, "journal is too large: {error}"),
        }
    }
}

impl std::error::Error for JournalError {}

impl From<std::io::Error> for JournalError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

/// A failure while encoding or appending one proposed journal fact.
#[derive(Debug)]
pub(super) enum JournalAppendError {
    /// Filesystem access failed after transaction validation approved the append.
    Io(std::io::Error),
    /// The proposed fact could not be encoded.
    Serialization(serde_json::Error),
    /// The proposed fact exceeds the bounded record format.
    RecordTooLarge {
        /// The serialized record length, including its newline.
        bytes: usize,
    },
}

impl Display for JournalAppendError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "journal append failed: {error}"),
            Self::Serialization(error) => {
                write!(formatter, "could not serialize journal record: {error}")
            },
            Self::RecordTooLarge { bytes } => {
                write!(
                    formatter,
                    "proposed journal record is too large: {bytes} bytes"
                )
            },
        }
    }
}

impl std::error::Error for JournalAppendError {}

impl From<std::io::Error> for JournalAppendError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::io::Write;

    use tempfile::tempdir;

    use super::BypassedMergeIdentity;
    use super::CanonicalWorktreeRoot;
    use super::ClaimHeadCommit;
    use super::ClaimHeadSnapshot;
    use super::ClaimSource;
    use super::CollisionPathSet;
    use super::ExplicitWidenReason;
    use super::ForeignReservationIdSet;
    use super::FullRefName;
    use super::IncursionPathSet;
    use super::InvalidBypassedMergeIdentity;
    use super::Journal;
    use super::JournalActor;
    use super::JournalError;
    use super::JournalEvent;
    use super::JournalOperation;
    use super::NonEmptyReservationPurpose;
    use super::OrderingDirection;
    use super::ProjectionGeneration;
    use super::ProtectedPhaseStartHead;
    use super::ReservationPurpose;
    use super::ReservationScope;
    use super::ReservationScopeAdditionSet;
    use super::ReservationScopeSet;
    use super::ScopeKind;
    use super::TrunkCommitAtClaim;
    use super::WidenCause;
    use super::WorkPlanReference;
    use super::WorktreeAdministrativeLocator;
    use super::replay_complete_records;
    use crate::answer::AuthorizedOverlap;
    use crate::answer::AuthorizedOverlapScopeSet;
    use crate::answer::AuthorizedOverlapSet;
    use crate::answer::ConflictAuthorization;
    use crate::answer::OverlapAuthorizationReason;
    use crate::answer::OverlapScopeRevision;
    use crate::ids::CoordinationRunId;
    use crate::ids::EdgeId;
    use crate::ids::EventId;
    use crate::ids::GitObjectId;
    use crate::ids::RecordedAt;
    use crate::ids::RepoInstanceId;
    use crate::ids::ReservationId;
    use crate::ids::ReservationScopePath;
    use crate::ids::SchemaVersion;
    use crate::ids::WorkPlanPhase;
    use crate::ids::WorktreeId;

    const HOLDER_RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a20";

    #[test]
    fn bypassed_merge_identity_rejects_shell_and_json_metacharacters() {
        assert!(matches!(
            BypassedMergeIdentity::from_hook_token("merge-\"quoted"),
            Err(InvalidBypassedMergeIdentity::UnsupportedCharacter('"'))
        ));
        assert!(matches!(
            BypassedMergeIdentity::from_hook_token("merge-\\escaped"),
            Err(InvalidBypassedMergeIdentity::UnsupportedCharacter('\\'))
        ));

        let generated = BypassedMergeIdentity::from(CoordinationRunId::new());
        assert!(BypassedMergeIdentity::from_hook_token(&generated.0).is_ok());
    }

    #[test]
    fn reservation_purpose_rejects_whitespace_and_stores_trimmed_text() {
        assert!(" \t\n".parse::<NonEmptyReservationPurpose>().is_err());
        let reservation_purpose = "  protected work  "
            .parse::<NonEmptyReservationPurpose>()
            .expect("non-whitespace purpose should parse");

        assert_eq!(reservation_purpose.to_string(), "protected work");
    }

    #[test]
    fn a_truncated_final_record_is_removed_before_replay() {
        let temporary_directory = tempdir().expect("temporary directory should exist");
        let journal_path = temporary_directory.path().join("journal.ndjson");
        let (journal, _) = Journal::open_or_create(&journal_path).expect("journal should open");
        let actor = test_actor();
        journal
            .append(&super::JournalEvent::for_operation(
                actor,
                ProjectionGeneration::from(1),
                JournalOperation::Renew {
                    reservation_id: ReservationId::new(),
                },
            ))
            .expect("record should append");
        fs::OpenOptions::new()
            .append(true)
            .open(&journal_path)
            .expect("journal should open for test tail")
            .write_all(b"{\"op\":")
            .expect("test tail should write");

        let replay = journal.replay_repairing_tail().expect("tail should repair");

        assert_eq!(replay.events.len(), 1);
        assert!(
            fs::read(&journal_path)
                .expect("journal should read")
                .ends_with(b"\n")
        );
    }

    #[test]
    fn a_corrupt_complete_record_is_not_repaired_away() {
        let temporary_directory = tempdir().expect("temporary directory should exist");
        let journal_path = temporary_directory.path().join("journal.ndjson");
        fs::write(&journal_path, b"not-json\n{}\n").expect("test journal should write");
        let (journal, _) = Journal::open_or_create(&journal_path).expect("journal should open");

        assert!(journal.replay_repairing_tail().is_err());
    }

    #[test]
    fn an_unsupported_schema_precedes_version_specific_operation_decoding() {
        let future_record = b"{\"schema_version\":3,\"op\":\"future_operation\"}\n";

        assert!(matches!(
            replay_complete_records(future_record),
            Err(JournalError::UnsupportedSchemaVersion(version))
                if version == SchemaVersion::from(3)
        ));
    }

    #[test]
    fn first_touch_source_round_trips_in_the_current_schema() {
        let encoded = serde_json::to_string(&ClaimSource::FirstTouch)
            .expect("first-touch source should encode");
        let decoded = serde_json::from_str::<ClaimSource>(&encoded)
            .expect("first-touch source should decode");

        assert_eq!(decoded, ClaimSource::FirstTouch);
    }

    #[test]
    fn a_malformed_supported_schema_record_remains_corrupt() {
        let malformed_v1_record = b"{\"schema_version\":1,\"op\":\"future_operation\"}\n";

        assert!(matches!(
            replay_complete_records(malformed_v1_record),
            Err(JournalError::CorruptInteriorRecord { line: 1, .. })
        ));
    }

    #[test]
    fn fully_populated_claim_fits_the_journal_record_limit() {
        let temporary_directory = tempdir().expect("temporary directory should exist");
        let journal_path = temporary_directory.path().join("journal.ndjson");
        let (journal, _) = Journal::open_or_create(&journal_path).expect("journal should open");
        let journal_event = fully_populated_claim_event();
        assert!(matches!(
            &journal_event.operation,
            JournalOperation::Claim { .. }
        ));
        let JournalOperation::Claim { scopes, .. } = &journal_event.operation else {
            return;
        };
        assert_eq!(scopes.as_slice().len(), 3);

        journal
            .append(&journal_event)
            .expect("fully populated claim should append");

        assert_eq!(
            journal
                .replay_repairing_tail()
                .expect("appended claim should replay")
                .events,
            vec![journal_event]
        );
    }

    #[test]
    fn fully_populated_claim_preserves_the_v1_json_scalars() {
        let serialized = serde_json::to_value(fully_populated_claim_event())
            .expect("journal event should serialize");

        assert_eq!(
            serialized,
            serde_json::json!({
                "schema_version": 1,
                "event_id": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b",
                "actor": {
                    "repository": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c",
                    "worktree": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d",
                    "run": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1e",
                },
                "at": "2026-08-23T17:34:54.123Z",
                "projection_generation": 9,
                "op": "claim",
                "reservation_id": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f",
                "scopes": [
                    {"path": "crates/cargo-berth/src", "kind": "tree"},
                    {"path": "crates/cargo-berth/tests", "kind": "tree"},
                    {"path": "docs/berth-plan.md", "kind": "file"},
                ],
                "source": {
                    "kind": "work_plan",
                    "plan": "docs/berth-plan.md",
                    "phase": "3b",
                },
                "purpose": {
                    "kind": "explained",
                    "explanation": "The ledger records durable coordination facts before a worktree claims paths.",
                },
                "trunk_at_claim": "1111111111111111111111111111111111111111",
                "head_snapshot": {
                    "kind": "branch",
                    "full_ref": "refs/heads/feature/ledger-transactions",
                    "head": "2222222222222222222222222222222222222222",
                },
                "phase_start_head": "3333333333333333333333333333333333333333",
                "worktree_root": "/Users/example/rust/cargo-berth-init",
                "worktree_administrative_locator": "worktrees/cargo-berth-init",
                "authorization": {
                    "kind": "sequence",
                    "overlaps": [{
                        "reservation_id": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a20",
                        "scope_revision": [
                            {"path": "crates/cargo-berth", "kind": "tree"},
                        ],
                        "scopes": [
                            {"path": "crates/cargo-berth/src", "kind": "tree"},
                            {"path": "docs/berth-plan.md", "kind": "file"},
                        ],
                    }],
                    "blocker": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a20",
                    "direction": "requester_before_holder",
                    "edge_id": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a21",
                    "reason": "The implementation must precede the dependent documentation update.",
                },
            })
        );
    }

    #[test]
    fn reservation_scope_sets_cannot_be_empty() {
        assert!(ReservationScopeSet::try_from(Vec::new()).is_err());
        assert!(serde_json::from_str::<ReservationScopeSet>("[]").is_err());
    }

    #[test]
    fn drift_journal_inputs_reject_empty_domain_values() {
        assert!(" \t\n".parse::<ExplicitWidenReason>().is_err());
        assert!(serde_json::from_str::<ExplicitWidenReason>("\"\"").is_err());
        assert!(ReservationScopeAdditionSet::try_from(Vec::new()).is_err());
        assert!(ForeignReservationIdSet::try_from(Vec::new()).is_err());
        assert!(IncursionPathSet::try_from(Vec::new()).is_err());
        assert!(CollisionPathSet::try_from(Vec::new()).is_err());
        assert!(serde_json::from_str::<ReservationScopeAdditionSet>("[]").is_err());
        assert!(serde_json::from_str::<ForeignReservationIdSet>("[]").is_err());
        assert!(serde_json::from_str::<IncursionPathSet>("[]").is_err());
        assert!(serde_json::from_str::<CollisionPathSet>("[]").is_err());

        let reason = "  reviewed expansion  "
            .parse::<ExplicitWidenReason>()
            .expect("non-empty widen reason should parse");
        let cause = WidenCause::Explicit { reason };
        assert_eq!(
            serde_json::to_value(cause).expect("widen cause should serialize"),
            serde_json::json!({"kind": "explicit", "reason": "reviewed expansion"})
        );
    }

    #[test]
    fn full_ref_names_enforce_git_reference_rules() {
        for invalid_ref in [
            "heads/main",
            "refs/",
            "refs//heads/main",
            "refs/heads/.hidden",
            "refs/heads/bad.lock",
            "refs/heads/bad..name",
            "refs/heads/bad name",
            "refs/heads/bad\u{0001}name",
            "refs/heads/bad\u{007f}name",
            "refs/heads/bad~name",
            "refs/heads/bad^name",
            "refs/heads/bad:name",
            "refs/heads/bad?name",
            "refs/heads/bad*name",
            "refs/heads/bad[name",
            "refs/heads/bad@{name",
            "refs/heads/bad\\name",
            "refs/heads/bad.",
        ] {
            assert!(invalid_ref.parse::<FullRefName>().is_err(), "{invalid_ref}");
        }

        for valid_ref in ["refs/heads/main", "refs/tags/v1.0.0"] {
            assert!(valid_ref.parse::<FullRefName>().is_ok(), "{valid_ref}");
        }
    }

    #[test]
    fn canonical_worktree_roots_require_normalized_absolute_spelling() {
        for invalid_root in [
            "/repo//tree",
            "/repo/tree/",
            "/repo/./tree",
            "/repo/../tree",
            "repo/tree",
        ] {
            assert!(
                invalid_root.parse::<CanonicalWorktreeRoot>().is_err(),
                "{invalid_root}"
            );
        }

        for valid_root in ["/", "/repo/tree"] {
            assert!(
                valid_root.parse::<CanonicalWorktreeRoot>().is_ok(),
                "{valid_root}"
            );
        }
    }

    #[test]
    fn worktree_administrative_locators_require_normalized_relative_spelling() {
        for invalid_locator in [
            "",
            "worktrees//tree",
            "worktrees/tree/",
            "worktrees/./tree",
            "worktrees/../tree",
            "/repo/tree",
        ] {
            assert!(
                invalid_locator
                    .parse::<WorktreeAdministrativeLocator>()
                    .is_err(),
                "{invalid_locator}"
            );
        }

        for valid_locator in [".", "worktrees/tree"] {
            assert!(
                valid_locator
                    .parse::<WorktreeAdministrativeLocator>()
                    .is_ok(),
                "{valid_locator}"
            );
        }
    }

    fn test_actor() -> JournalActor {
        JournalActor {
            repository: RepoInstanceId::new(),
            worktree:   WorktreeId::new(),
            run:        CoordinationRunId::new(),
        }
    }

    fn fully_populated_claim_event() -> JournalEvent {
        JournalEvent {
            schema_version:        SchemaVersion::from(1),
            event_id:              "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b"
                .parse::<EventId>()
                .expect("event identifier should parse"),
            actor:                 JournalActor {
                repository: "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c"
                    .parse::<RepoInstanceId>()
                    .expect("repository identifier should parse"),
                worktree:   "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d"
                    .parse::<WorktreeId>()
                    .expect("worktree identifier should parse"),
                run:        "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1e"
                    .parse::<CoordinationRunId>()
                    .expect("coordination run identifier should parse"),
            },
            at:                    "2026-08-23T17:34:54.123Z"
                .parse::<RecordedAt>()
                .expect("recorded timestamp should parse"),
            projection_generation: ProjectionGeneration::from(9),
            operation:             JournalOperation::Claim {
                reservation_id:                  "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f"
                    .parse::<ReservationId>()
                    .expect("reservation identifier should parse"),
                scopes:                          ReservationScopeSet::try_from(vec![
                    reservation_scope("crates/cargo-berth/src", ScopeKind::Tree),
                    reservation_scope("crates/cargo-berth/tests", ScopeKind::Tree),
                    reservation_scope("docs/berth-plan.md", ScopeKind::File),
                ])
                .expect("claim footprint should be non-empty"),
                source:                          ClaimSource::WorkPlan {
                    plan:  "docs/berth-plan.md"
                        .parse::<WorkPlanReference>()
                        .expect("work-plan reference should parse"),
                    phase: "3b"
                        .parse::<WorkPlanPhase>()
                        .expect("opaque work-plan phase should parse"),
                },
                purpose:
                    "The ledger records durable coordination facts before a worktree claims paths."
                        .parse::<NonEmptyReservationPurpose>()
                        .map(ReservationPurpose::Explained)
                        .expect("reservation purpose should parse"),
                trunk_at_claim:                  "1111111111111111111111111111111111111111"
                    .parse::<GitObjectId>()
                    .map(TrunkCommitAtClaim::from)
                    .expect("trunk commit should parse"),
                head_snapshot:                   ClaimHeadSnapshot::Branch {
                    full_ref: "refs/heads/feature/ledger-transactions"
                        .parse::<FullRefName>()
                        .expect("full branch reference should parse"),
                    head:     "2222222222222222222222222222222222222222"
                        .parse::<GitObjectId>()
                        .map(ClaimHeadCommit::from)
                        .expect("claim head should parse"),
                },
                phase_start_head:                "3333333333333333333333333333333333333333"
                    .parse::<GitObjectId>()
                    .map(ProtectedPhaseStartHead::from)
                    .expect("phase-start head should parse"),
                worktree_root:                   "/Users/example/rust/cargo-berth-init"
                    .parse::<CanonicalWorktreeRoot>()
                    .expect("canonical worktree root should parse"),
                worktree_administrative_locator: "worktrees/cargo-berth-init"
                    .parse::<WorktreeAdministrativeLocator>()
                    .expect("worktree administrative locator should parse"),
                authorization:                   ConflictAuthorization::Sequence {
                    overlaps:  AuthorizedOverlapSet::try_from(vec![AuthorizedOverlap {
                        reservation_id: parse_reservation_id(HOLDER_RESERVATION_ID),
                        scope_revision: OverlapScopeRevision::from(
                            &ReservationScopeSet::try_from(vec![reservation_scope(
                                "crates/cargo-berth",
                                ScopeKind::Tree,
                            )])
                            .expect("holder scopes should be non-empty"),
                        ),
                        scopes:         AuthorizedOverlapScopeSet::from(
                            ReservationScopeSet::try_from(vec![
                                reservation_scope("crates/cargo-berth/src", ScopeKind::Tree),
                                reservation_scope("docs/berth-plan.md", ScopeKind::File),
                            ])
                            .expect("authorized scopes should be non-empty"),
                        ),
                    }])
                    .expect("authorized holders should be non-empty"),
                    blocker:   parse_reservation_id(HOLDER_RESERVATION_ID),
                    direction: OrderingDirection::RequesterBeforeHolder,
                    edge_id:   "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a21"
                        .parse::<EdgeId>()
                        .expect("edge identifier should parse"),
                    reason:
                        "The implementation must precede the dependent documentation update."
                            .parse::<OverlapAuthorizationReason>()
                            .expect("overlap authorization reason should parse"),
                },
            },
        }
    }

    fn parse_scope_path(path: &str) -> ReservationScopePath {
        path.parse()
            .expect("repository-relative reservation scope path should parse")
    }

    fn parse_reservation_id(value: &str) -> ReservationId {
        value
            .parse()
            .expect("holder reservation identifier should parse")
    }

    fn reservation_scope(path: &str, kind: ScopeKind) -> ReservationScope {
        ReservationScope {
            path: parse_scope_path(path),
            kind,
        }
    }
}
