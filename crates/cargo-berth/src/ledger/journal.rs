//! The append-only journal and its complete version-one operation union.

use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;

use super::constants::CURRENT_SCHEMA_VERSION;
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
use crate::ids::WorktreeId;

/// One append-only fact in the shared coordination journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct JournalEvent {
    /// The schema version required to interpret this fact.
    pub(super) schema_version:        SchemaVersion,
    /// The non-recyclable identity of this append.
    pub(super) event_id:              EventId,
    /// The coordination actor that recorded this fact.
    pub(super) actor:                 JournalActor,
    /// The time this fact was recorded.
    pub(super) at:                    RecordedAt,
    /// The cache generation this append publishes.
    pub(super) projection_generation: ProjectionGeneration,
    /// The state transition this fact records.
    #[serde(flatten)]
    pub(super) operation:             JournalOperation,
}

impl JournalEvent {
    /// Build a new v1 journal fact for one mutation transaction.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "No stateful verb creates journal facts through the transaction wrapper yet."
        )
    )]
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
        reservation_id: ReservationId,
        /// The paths claimed atomically by this reservation.
        scopes:         Vec<ReservationScopePath>,
        /// How the claimant described the work that needs these paths.
        source:         ClaimSource,
        /// The reason the claimant gave for this reservation.
        reason:         String,
        /// The overlap result that authorized this acquisition.
        authorization:  ConflictAuthorization,
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
        result_head:    GitObjectId,
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
        /// The user-confirmed outcome of this release.
        disposition:    ReleaseDisposition,
        /// The human explanation retained for an irreversible disposition.
        reason:         String,
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
        plan:  String,
        /// The plan-local phase number.
        phase: u32,
    },
    /// A direct caller-specified reservation.
    Explicit,
}

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
        protected_tip: GitObjectId,
        /// The current trunk comparison point.
        trunk_oid:     GitObjectId,
    },
}

/// A user-confirmed terminal reservation outcome.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReleaseDisposition {
    /// Git proved the protected work reached trunk.
    Integrated,
    /// The user supplied alternate evidence after rewritten integration.
    RewrittenIntegration,
    /// The user deliberately discarded the reservation's work.
    Abandoned,
    /// The user confirmed an orphaned reservation can retire.
    RetiredOrphan,
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

    /// Replay every complete record and repair one incomplete final record.
    pub(super) fn replay_repairing_tail(&self) -> Result<JournalReplay, JournalError> {
        let bytes = fs::read(&self.path)?;
        let complete_end = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let complete_records = &bytes[..complete_end];
        let mut events = Vec::new();
        for (line_index, record) in complete_records.split(|byte| *byte == b'\n').enumerate() {
            if record.is_empty() {
                if line_index + 1 == complete_records.split(|byte| *byte == b'\n').count() {
                    continue;
                }
                return Err(JournalError::CorruptInteriorRecord {
                    line:  line_index + 1,
                    error: "blank journal record".to_owned(),
                });
            }
            let record = std::str::from_utf8(record).map_err(|error| {
                JournalError::CorruptInteriorRecord {
                    line:  line_index + 1,
                    error: error.to_string(),
                }
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

        if complete_end != bytes.len() {
            let journal_file = OpenOptions::new().write(true).open(&self.path)?;
            journal_file
                .set_len(u64::try_from(complete_end).map_err(JournalError::JournalTooLarge)?)?;
            journal_file.sync_all()?;
        }

        let repaired_bytes = if complete_end == bytes.len() {
            bytes
        } else {
            bytes[..complete_end].to_vec()
        };
        let generation = events.last().map_or_else(
            || ProjectionGeneration::from(0),
            |event| event.projection_generation,
        );
        Ok(JournalReplay {
            events,
            end_offset: JournalByteOffset::from(
                u64::try_from(repaired_bytes.len()).map_err(JournalError::JournalTooLarge)?,
            ),
            fingerprint: JournalFingerprint::from_bytes(&repaired_bytes),
            generation,
        })
    }

    /// Append exactly one complete JSON record and sync it before cache publication.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "No stateful verb appends through this journal writer yet."
        )
    )]
    pub(super) fn append(&self, event: &JournalEvent) -> Result<(), JournalError> {
        let mut record = serde_json::to_vec(event).map_err(JournalError::Serialization)?;
        record.push(b'\n');
        if record.len() > MAXIMUM_JOURNAL_RECORD_BYTES {
            return Err(JournalError::RecordTooLarge {
                bytes: record.len(),
            });
        }

        let mut journal_file = OpenOptions::new().append(true).open(&self.path)?;
        journal_file.write_all(&record)?;
        journal_file.sync_all()?;
        Ok(())
    }
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
    /// A serialized record would exceed the configured journal record limit.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "The writer constructs this error when a mutation exceeds the append limit; none reaches it yet."
        )
    )]
    RecordTooLarge {
        /// The serialized record length, including its newline.
        bytes: usize,
    },
    /// Serializing a journal event failed.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "The writer constructs this error when a journal event cannot serialize; no writer path reaches it yet."
        )
    )]
    Serialization(serde_json::Error),
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
            Self::RecordTooLarge { bytes } => {
                write!(
                    formatter,
                    "journal record exceeds the configured limit: {bytes} bytes"
                )
            },
            Self::Serialization(error) => {
                write!(formatter, "could not serialize journal record: {error}")
            },
        }
    }
}

impl std::error::Error for JournalError {}

impl From<std::io::Error> for JournalError {
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
    use super::ClaimSource;
    use super::ConflictAuthorization;
    use super::Journal;
    use super::JournalActor;
    use super::JournalEvent;
    use super::JournalOperation;
    use super::OrderingDirection;
    use super::ProjectionGeneration;
    use crate::ids::CoordinationRunId;
    use crate::ids::EdgeId;
    use crate::ids::EventId;
    use crate::ids::RecordedAt;
    use crate::ids::RepoInstanceId;
    use crate::ids::ReservationId;
    use crate::ids::ReservationRevision;
    use crate::ids::ReservationScopePath;
    use crate::ids::SchemaVersion;
    use crate::ids::WorktreeId;

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
                    "crates/cargo-berth/src",
                    "crates/cargo-berth/tests",
                    "docs/berth-plan.md",
                ],
                "source": {
                    "kind": "work_plan",
                    "plan": "docs/berth-plan.md",
                    "phase": 2,
                },
                "reason": "The ledger records durable coordination facts before a worktree claims paths.",
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
                reservation_id: "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f"
                    .parse::<ReservationId>()
                    .expect("reservation identifier should parse"),
                scopes:         [
                    "crates/cargo-berth/src",
                    "crates/cargo-berth/tests",
                    "docs/berth-plan.md",
                ]
                .into_iter()
                .map(parse_scope_path)
                .collect(),
                source:         ClaimSource::WorkPlan {
                    plan:  "docs/berth-plan.md".to_owned(),
                    phase: 2,
                },
                reason:
                    "The ledger records durable coordination facts before a worktree claims paths."
                        .to_owned(),
                authorization:  ConflictAuthorization::Sequence {
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
}
