//! The append-only journal and its complete version-one operation union.

use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

use super::constants::CURRENT_SCHEMA_VERSION;
use super::constants::DELETE_CONTROL_BYTE;
use super::constants::MAXIMUM_JOURNAL_RECORD_BYTES;
use crate::config::InitializationState;
use crate::ids::CoordinationRunId;
use crate::ids::EdgeId;
use crate::ids::EventId;
use crate::ids::GitObjectId;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RecordedAt;
use crate::ids::RepoInstanceId;
use crate::ids::ReservationId;
use crate::ids::ReservationRevision;
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
        reservation_id: ReservationId,
        /// The new paths added by this mutation.
        added_scopes:   Vec<ReservationScopePath>,
        /// Why the footprint expanded.
        cause:          WidenCause,
        /// The overlap result that authorized this widening.
        authorization:  ConflictAuthorization,
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
        reason:                  String,
    },
    /// Declare an ordering edge that did not begin as a deferral.
    DeclareOrderingEdge {
        /// The durable identity for this edge.
        edge_id: EdgeId,
        /// The reservation required to integrate first.
        before:  ReservationId,
        /// The reservation held until `before` is satisfied.
        after:   ReservationId,
        /// The overlap scopes that make this edge relevant.
        scopes:  Vec<ReservationScopePath>,
        /// The reason for the ordering.
        reason:  String,
    },
    /// Record a write that entered scopes reserved by another worktree.
    Incursion {
        /// The reservation whose worktree made the write.
        reservation_id:          ReservationId,
        /// The foreign reservations whose scopes were entered.
        foreign_reservation_ids: Vec<ReservationId>,
        /// The paths written without coverage.
        paths:                   Vec<ReservationScopePath>,
    },
    /// Issue a one-use permit for a confirmed forced integration.
    ForcedIntegrationPermit {
        /// The opaque identity of the one-use permit.
        permit_id:      EventId,
        /// The reservation allowed to integrate past a hold.
        reservation_id: ReservationId,
        /// The reason the user accepted this exception.
        reason:         String,
    },
    /// Consume a previously issued forced-integration permit.
    ConsumeForcedIntegrationPermit {
        /// The permit that cannot be used again.
        permit_id:      EventId,
        /// The reservation that consumed it.
        reservation_id: ReservationId,
    },
    /// Record an explicit escape-hatch bypass without changing edge state.
    Bypass {
        /// The action permitted outside normal ledger validation.
        action: BypassedAction,
        /// The explanation for accepting the bypass.
        reason: String,
    },
    /// Move a reservation's ownership to a replacement worktree.
    RebindWorktree {
        /// The recovered reservation.
        reservation_id:       ReservationId,
        /// The opaque worktree identity that no longer holds the work.
        previous_worktree_id: WorktreeId,
        /// The opaque worktree identity now holding the work.
        current_worktree_id:  WorktreeId,
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

impl fmt::Display for FullRefName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
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

impl fmt::Display for InvalidFullRefName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "a full git reference name must begin with refs/ and satisfy git reference rules",
        )
    }
}

impl std::error::Error for InvalidFullRefName {}

/// The declared file-versus-tree meaning of one reserved repository path.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

impl fmt::Display for EmptyReservationScopeSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a reservation scope set cannot be empty")
    }
}

impl std::error::Error for EmptyReservationScopeSet {}

/// A canonical, absolute, UTF-8 worktree root stored for identity validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalWorktreeRoot(String);

impl fmt::Display for CanonicalWorktreeRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
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

impl fmt::Display for InvalidCanonicalWorktreeRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a canonical worktree root must be an absolute normalized UTF-8 path")
    }
}

impl std::error::Error for InvalidCanonicalWorktreeRoot {}

/// The common-directory-relative locator of a worktree administrative directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeAdministrativeLocator(String);

impl fmt::Display for WorktreeAdministrativeLocator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
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

impl fmt::Display for InvalidWorktreeAdministrativeLocator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a worktree administrative locator must be normalized and relative")
    }
}

impl std::error::Error for InvalidWorktreeAdministrativeLocator {}

/// Why an existing reservation received more scopes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum WidenCause {
    /// Reconciliation observed paths not covered by the claim.
    Drift {
        /// The newly observed paths.
        observed_paths: Vec<ReservationScopePath>,
    },
    /// The caller deliberately expanded the reservation.
    Explicit {
        /// The caller's explanation.
        reason: String,
    },
}

/// The complete overlap decision recorded within a claim or widen transaction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ConflictAuthorization {
    /// No foreign overlap existed when the transaction acquired these scopes.
    NoConflict,
    /// An ordering edge authorizes this exact observed overlap set.
    Sequence {
        /// The exact overlaps and generations shown to the user.
        overlaps:  Vec<AuthorizedOverlap>,
        /// The requested ordering direction.
        direction: OrderingDirection,
        /// The edge born with this acquisition.
        edge_id:   EdgeId,
        /// The approved reason for selecting an order.
        reason:    String,
    },
    /// Editing can proceed while integration remains held pending an order.
    Defer {
        /// The exact overlaps and generations shown to the user.
        overlaps: Vec<AuthorizedOverlap>,
        /// The approved reason for delaying the order.
        reason:   String,
    },
    /// Editing can proceed without declaring an ordering relationship.
    Override {
        /// The exact overlaps and generations shown to the user.
        overlaps: Vec<AuthorizedOverlap>,
        /// The approved reason for accepting the conflict.
        reason:   String,
    },
}

/// One exact holder and reservation generation covered by an authorization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AuthorizedOverlap {
    /// The existing holder named by the authorization.
    pub(crate) reservation_id:       ReservationId,
    /// The holder's revision when the authorization was shown.
    pub(crate) reservation_revision: ReservationRevision,
    /// The normalized overlap paths that this answer covers.
    pub(crate) scopes:               Vec<ReservationScopePath>,
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
    path: std::path::PathBuf,
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
        let event = serde_json::from_str::<JournalEvent>(record).map_err(|error| {
            JournalError::CorruptInteriorRecord {
                line:  line_index + 1,
                error: error.to_string(),
            }
        })?;
        let supported_schema_version = SchemaVersion::from(CURRENT_SCHEMA_VERSION);
        if event.schema_version != supported_schema_version {
            return Err(JournalError::UnsupportedSchemaVersion(event.schema_version));
        }
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
    JournalTooLarge(std::num::TryFromIntError),
}

impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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

impl fmt::Display for JournalAppendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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

    use super::AuthorizedOverlap;
    use super::CanonicalWorktreeRoot;
    use super::ClaimHeadCommit;
    use super::ClaimHeadSnapshot;
    use super::ClaimSource;
    use super::ConflictAuthorization;
    use super::FullRefName;
    use super::Journal;
    use super::JournalActor;
    use super::JournalEvent;
    use super::JournalOperation;
    use super::NonEmptyReservationPurpose;
    use super::OrderingDirection;
    use super::ProjectionGeneration;
    use super::ProtectedPhaseStartHead;
    use super::ReservationPurpose;
    use super::ReservationScope;
    use super::ReservationScopeSet;
    use super::ScopeKind;
    use super::TrunkCommitAtClaim;
    use super::WorkPlanReference;
    use super::WorktreeAdministrativeLocator;
    use crate::ids::CoordinationRunId;
    use crate::ids::EdgeId;
    use crate::ids::EventId;
    use crate::ids::GitObjectId;
    use crate::ids::RecordedAt;
    use crate::ids::RepoInstanceId;
    use crate::ids::ReservationId;
    use crate::ids::ReservationRevision;
    use crate::ids::ReservationScopePath;
    use crate::ids::SchemaVersion;
    use crate::ids::WorkPlanPhase;
    use crate::ids::WorktreeId;

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
                        "reservation_revision": 3,
                        "scopes": ["crates/cargo-berth/src", "docs/berth-plan.md"],
                    }],
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
                    overlaps:  vec![AuthorizedOverlap {
                        reservation_id:       "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a20"
                            .parse::<ReservationId>()
                            .expect("holder reservation identifier should parse"),
                        reservation_revision: ReservationRevision::from(3),
                        scopes:               ["crates/cargo-berth/src", "docs/berth-plan.md"]
                            .into_iter()
                            .map(parse_scope_path)
                            .collect(),
                    }],
                    direction: OrderingDirection::RequesterBeforeHolder,
                    edge_id:   "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a21"
                        .parse::<EdgeId>()
                        .expect("edge identifier should parse"),
                    reason:
                        "The implementation must precede the dependent documentation update."
                            .to_owned(),
                },
            },
        }
    }

    fn parse_scope_path(path: &str) -> ReservationScopePath {
        path.parse()
            .expect("repository-relative reservation scope path should parse")
    }

    fn reservation_scope(path: &str, kind: ScopeKind) -> ReservationScope {
        ReservationScope {
            path: parse_scope_path(path),
            kind,
        }
    }
}
