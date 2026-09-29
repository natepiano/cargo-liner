//! Which merge extent observations a later observation supersedes, tracked as the journal replays,
//! and the compaction that rewrites the journal without them.
//!
//! An observation, in either record form, replaces the recorded merge extent of every reservation
//! it names, and no replay step reads a merge extent. So once every reservation an observation
//! names is named again by a later observation, the record leaves no trace in the fold, and
//! compacting the journal may drop it. An observation superseded for only some of its reservations
//! is kept whole. Every other record is kept.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;

use super::constants::JOURNAL_COMPACTION_TEMPORARY_FILE_NAME;
use super::constants::MUTATING_VERB_CONTENTION_TOLERANCE;
use super::error::LedgerError;
use super::handle::Ledger;
use super::identity;
use super::journal::Journal;
use super::journal::JournalError;
use super::journal::JournalOperation;
use super::journal::JournalReplay;
use super::lock::MutationLock;
use super::lock::MutationLockError;
use super::projection::Projection;
use super::projection::ProjectionError;
use super::replay_checkpoint::ReplayCheckpoint;
use crate::ids::RepoInstanceId;
use crate::ids::ReservationId;
use crate::reservation::ReservationReplayError;

/// The merge extent observations of a replayed journal that later observations supersede.
///
/// Observations are identified by ordinal, the zero-based position of their record in the
/// journal. Holds one entry per observed reservation and one per observation still named.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct SupersededExtents {
    /// The ordinal of the latest observation naming each reservation.
    latest:                     BTreeMap<ReservationId, u64>,
    /// Every observation that is the latest for at least one reservation, by ordinal.
    live:                       BTreeMap<u64, LiveExtent>,
    /// The total byte length of the observations no reservation names as its latest.
    pub(super) droppable_bytes: u64,
}

/// An observation that is the latest for at least one reservation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct LiveExtent {
    /// The byte length of the observation's record, its newline included.
    bytes: u64,
    /// How many reservations name the observation as their latest.
    names: usize,
}

impl SupersededExtents {
    /// Record the operation of the record at `ordinal`, `bytes` long, the next after every record
    /// already observed.
    pub(super) fn observe(&mut self, ordinal: u64, bytes: u64, operation: &JournalOperation) {
        let Some(reservations) = operation.merge_extent_reservations() else {
            return;
        };
        self.live.insert(
            ordinal,
            LiveExtent {
                bytes,
                names: reservations.len(),
            },
        );
        for reservation_id in reservations {
            let Some(previous) = self.latest.insert(reservation_id, ordinal) else {
                continue;
            };
            if let Entry::Occupied(mut entry) = self.live.entry(previous) {
                entry.get_mut().names -= 1;
                if entry.get().names == 0 {
                    self.droppable_bytes += entry.remove().bytes;
                }
            }
        }
    }

    /// Whether compaction keeps the record at `ordinal`, a merge extent observation when
    /// `merge_extent_observation` holds: every record except an observation that no reservation
    /// names as its latest.
    fn keeps(&self, ordinal: u64, merge_extent_observation: bool) -> bool {
        !merge_extent_observation || self.live.contains_key(&ordinal)
    }
}

/// What one journal compaction did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JournalCompaction {
    /// No merge extent observation is superseded, so the journal is untouched.
    NothingToCompact,
    /// The journal was rewritten without its superseded merge extent observations.
    Compacted {
        /// The records dropped.
        removed:   JournalRecords,
        /// The records the compacted journal holds.
        remaining: JournalRecords,
    },
}

/// A number of journal records and their byte length, newlines included.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct JournalRecords {
    /// The number of records.
    records: u64,
    /// The byte length of the records.
    bytes:   u64,
}

/// Why a journal compaction did not complete.
#[derive(Debug)]
pub(crate) enum JournalCompactionError {
    /// The ledger could not be locked, read, or written. The journal is untouched unless the
    /// failure followed the rename that replaces it.
    Ledger(LedgerError),
    /// The journal's replay does not permit a rewrite, and the journal is untouched.
    Refused(CompactionRefusal),
}

/// A journal that compaction declines to rewrite.
#[derive(Debug)]
pub(crate) enum CompactionRefusal {
    /// Reservation replay rejects a record, so no fold confirms what the rewrite must preserve.
    ReservationReplay(ReservationReplayError),
    /// A record names a repository other than the ledger's.
    RepositoryValidation(LedgerError),
    /// Bytes follow the journal's final newline.
    IncompleteFinalRecord,
    /// The compacted journal replays to a different reservation set, coordination record list,
    /// repository set, or generation than the journal it would replace.
    ReplayMismatch,
}

/// A boundary around the two publications that make a compacted journal visible, where a test
/// observes lock-free reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationBoundary {
    /// The compacted journal is written and verified, and the ledger still shows the original.
    BeforeProjection,
    /// The compacted journal's projection sits beside the original journal.
    BeforeRename,
    /// The compacted journal and its projection are both published.
    AfterRename,
}

/// The journal file an automatic compaction last failed on, which holds automatic compaction back
/// until that file grows past its length then by the compaction threshold, so a defect costs one
/// attempt per threshold of appended bytes instead of one per transaction.
#[derive(Debug, Deserialize, Serialize)]
struct CompactionRefusalMarker {
    /// The device of the journal file.
    device:     u64,
    /// The inode of the journal file.
    inode:      u64,
    /// The journal's length when the compaction failed.
    end_offset: u64,
    /// Why the compaction failed.
    cause:      String,
}

/// A journal's replay and the records its compaction keeps.
struct CompactionPlan {
    /// The replay of the whole journal.
    replay: JournalReplay,
    /// The kept records, byte for byte, in append order.
    kept:   Vec<u8>,
}

impl Ledger {
    /// Rewrite the journal without the merge extent observations that later observations
    /// supersede, holding the mutation lock throughout.
    ///
    /// Replays the whole journal, writes the kept records to a temporary file, replays that file,
    /// and refuses unless both replays agree. Then stores the replay checkpoint for the temporary
    /// file, publishes its projection, and renames it over the journal. The projection publishes
    /// first because lock-free readers replay the journal and then read the projection: a reader
    /// of the original journal meets the new projection as one behind its replay, a rebuild to do,
    /// where the reverse order would show a reader of the compacted journal the original
    /// projection, ahead of its replay.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the compaction trigger and its init verb are not wired yet"
        )
    )]
    pub(crate) fn compact_journal(&self) -> Result<JournalCompaction, JournalCompactionError> {
        self.compact_journal_observing(|_| {})
    }

    /// [`Self::compact_journal`], calling `at_boundary` at each [`PublicationBoundary`] it
    /// reaches.
    fn compact_journal_observing(
        &self,
        mut at_boundary: impl FnMut(PublicationBoundary),
    ) -> Result<JournalCompaction, JournalCompactionError> {
        self.require_existing()?;
        let _lock = MutationLock::acquire(&self.paths.lock, MUTATING_VERB_CONTENTION_TOLERANCE)?;
        let repo_instance_id = identity::read_repo_instance_id(&self.paths.repo_instance_id)?;
        self.compact_locked_journal(repo_instance_id, replays_agree, &mut at_boundary)
    }

    /// Compact the journal after a transaction whose replay held `droppable_bytes` of superseded
    /// records, once they reach the ledger's compaction threshold.
    ///
    /// Skips when another holder has the mutation lock rather than wait, and never fails: like a
    /// checkpoint that cannot be written, a failed compaction leaves the journal whole. Its
    /// [`CompactionRefusalMarker`] holds the next attempt back until the journal grows.
    pub(super) fn compact_if_due(&self, droppable_bytes: u64) {
        std::mem::drop(self.attempt_due_compaction(droppable_bytes, replays_agree));
    }

    /// [`Self::compact_if_due`], keeping a compacted journal only when `agree` holds of the
    /// original and compacted replays. Returns `None` when it skips the compaction.
    fn attempt_due_compaction(
        &self,
        droppable_bytes: u64,
        agree: fn(&JournalReplay, &JournalReplay) -> bool,
    ) -> Result<Option<JournalCompaction>, JournalCompactionError> {
        if droppable_bytes < self.compaction_threshold_bytes || self.compaction_held_back() {
            return Ok(None);
        }
        let _lock = match MutationLock::acquire(&self.paths.lock, Duration::ZERO) {
            Ok(lock) => lock,
            Err(MutationLockError::AcquisitionTimedOut) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let compaction = self.compact_locked_journal_if_due(agree);
        if let Err(error) = &compaction {
            // Without a marker, the next transaction past the threshold attempts again.
            std::mem::drop(self.record_refusal(error));
        }
        compaction
    }

    /// Replay the journal again under the mutation lock the caller holds, and compact it when its
    /// superseded records still reach the threshold after any concurrent compaction.
    fn compact_locked_journal_if_due(
        &self,
        agree: fn(&JournalReplay, &JournalReplay) -> bool,
    ) -> Result<Option<JournalCompaction>, JournalCompactionError> {
        let repo_instance_id = identity::read_repo_instance_id(&self.paths.repo_instance_id)?;
        let replay =
            Journal::open_existing(&self.paths.journal)?.replay_repairing_tail_from_checkpoint(
                &ReplayCheckpoint::new(&self.paths.replay_checkpoint, repo_instance_id),
            )?;
        if replay.superseded.droppable_bytes < self.compaction_threshold_bytes {
            return Ok(None);
        }
        self.compact_locked_journal(repo_instance_id, agree, &mut |_| {})
            .map(Some)
    }

    /// Whether a failed automatic compaction of the current journal file holds the next one back:
    /// the journal has not grown past its length at that failure by the compaction threshold.
    fn compaction_held_back(&self) -> bool {
        let (Ok(marker), Ok(journal)) = (
            fs::read(&self.paths.compaction_refusal),
            fs::metadata(&self.paths.journal),
        ) else {
            return false;
        };
        serde_json::from_slice::<CompactionRefusalMarker>(&marker).is_ok_and(|marker| {
            marker.device == journal.dev()
                && marker.inode == journal.ino()
                && journal.len()
                    <= marker
                        .end_offset
                        .saturating_add(self.compaction_threshold_bytes)
        })
    }

    /// Store the [`CompactionRefusalMarker`] of an automatic compaction that failed with `error`,
    /// naming the journal file and its length now.
    fn record_refusal(&self, error: &JournalCompactionError) -> std::io::Result<()> {
        let journal = fs::metadata(&self.paths.journal)?;
        let marker = CompactionRefusalMarker {
            device:     journal.dev(),
            inode:      journal.ino(),
            end_offset: journal.len(),
            cause:      error.to_string(),
        };
        fs::write(&self.paths.compaction_refusal, serde_json::to_vec(&marker)?)
    }

    /// Compact the journal of the ledger of `repo_instance_id` under the mutation lock the caller
    /// holds, keeping the compacted journal only when `agree` holds of the original and compacted
    /// replays, and calling `at_boundary` at each [`PublicationBoundary`] it reaches.
    fn compact_locked_journal(
        &self,
        repo_instance_id: RepoInstanceId,
        agree: fn(&JournalReplay, &JournalReplay) -> bool,
        at_boundary: &mut impl FnMut(PublicationBoundary),
    ) -> Result<JournalCompaction, JournalCompactionError> {
        let temporary_path = self
            .paths
            .directory
            .join(JOURNAL_COMPACTION_TEMPORARY_FILE_NAME);
        match fs::remove_file(&temporary_path) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
        let Some(plan) = CompactionPlan::of(&fs::read(&self.paths.journal)?, repo_instance_id)?
        else {
            return Ok(JournalCompaction::NothingToCompact);
        };
        let publication =
            self.publish_compacted(&temporary_path, &plan, repo_instance_id, agree, at_boundary);
        if publication.is_err() {
            // Before the rename the journal is untouched, and the next compaction would remove
            // the temporary file anyway.
            std::mem::drop(fs::remove_file(&temporary_path));
        }
        let compacted = publication?;
        // A marker left behind names the replaced journal file, so it holds nothing back.
        std::mem::drop(remove_refusal(&self.paths.compaction_refusal));
        Ok(JournalCompaction::Compacted {
            removed:   JournalRecords {
                records: plan.replay.record_count - compacted.record_count,
                bytes:   u64::from(plan.replay.end_offset) - u64::from(compacted.end_offset),
            },
            remaining: JournalRecords {
                records: compacted.record_count,
                bytes:   u64::from(compacted.end_offset),
            },
        })
    }

    /// Write `plan`'s kept records to `temporary_path`, verify with `agree` that their replay
    /// matches the plan's, and publish them as the journal. Returns the compacted journal's replay.
    fn publish_compacted(
        &self,
        temporary_path: &Path,
        plan: &CompactionPlan,
        repo_instance_id: RepoInstanceId,
        agree: fn(&JournalReplay, &JournalReplay) -> bool,
        at_boundary: &mut impl FnMut(PublicationBoundary),
    ) -> Result<JournalReplay, JournalCompactionError> {
        let mut temporary_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temporary_path)?;
        temporary_file.write_all(&plan.kept)?;
        temporary_file.sync_all()?;
        let compacted = Journal::replay_read_only(temporary_path)?;
        if !agree(&plan.replay, &compacted) {
            return Err(CompactionRefusal::ReplayMismatch.into());
        }
        // A checkpoint that cannot be written leaves the next replay to start from byte 0.
        std::mem::drop(
            ReplayCheckpoint::new(&self.paths.replay_checkpoint, repo_instance_id)
                .store_for_journal_file(&compacted, &temporary_file.metadata()?, &plan.kept),
        );
        at_boundary(PublicationBoundary::BeforeProjection);
        Projection::from_replay(repo_instance_id, &compacted)
            .publish(&self.paths.directory, &self.paths.projection)?;
        at_boundary(PublicationBoundary::BeforeRename);
        fs::rename(temporary_path, &self.paths.journal)?;
        fs::File::open(&self.paths.directory)?.sync_all()?;
        at_boundary(PublicationBoundary::AfterRename);
        Ok(compacted)
    }
}

impl CompactionPlan {
    /// The plan for compacting `journal`, the whole journal of the ledger of `repo_instance_id`,
    /// or `None` when no record is superseded.
    fn of(
        journal: &[u8],
        repo_instance_id: RepoInstanceId,
    ) -> Result<Option<Self>, JournalCompactionError> {
        let mut merge_extent_observations = Vec::new();
        let (replay, complete_end) =
            JournalReplay::empty().extended_over_visiting(journal, |operation| {
                merge_extent_observations.push(operation.is_merge_extent_observation());
            })?;
        if let Err(error) = &replay.reservations {
            return Err(CompactionRefusal::ReservationReplay(error.clone()).into());
        }
        identity::validate_journal_repository(repo_instance_id, &replay)
            .map_err(CompactionRefusal::RepositoryValidation)?;
        if complete_end != journal.len() {
            return Err(CompactionRefusal::IncompleteFinalRecord.into());
        }
        if replay.superseded.droppable_bytes == 0 {
            return Ok(None);
        }
        let mut kept = Vec::with_capacity(journal.len());
        let records = (0_u64..).zip(journal.split_inclusive(|byte| *byte == b'\n'));
        for ((ordinal, record), merge_extent_observation) in records.zip(merge_extent_observations)
        {
            if replay.superseded.keeps(ordinal, merge_extent_observation) {
                kept.extend_from_slice(record);
            }
        }
        Ok(Some(Self { replay, kept }))
    }
}

/// Delete the [`CompactionRefusalMarker`] at `path`, so the next transaction past the threshold
/// compacts the journal.
pub(super) fn remove_refusal(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Whether `compacted` replays to everything `original` does: its reservations, coordination
/// records, repositories, and generation.
fn replays_agree(original: &JournalReplay, compacted: &JournalReplay) -> bool {
    original.reservations == compacted.reservations
        && original.coordination_events == compacted.coordination_events
        && original.repositories == compacted.repositories
        && original.generation == compacted.generation
}

impl Display for JournalCompactionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ledger(error) => write!(formatter, "journal compaction failed: {error}"),
            Self::Refused(refusal) => write!(formatter, "journal compaction refused: {refusal}"),
        }
    }
}

impl std::error::Error for JournalCompactionError {}

impl Display for CompactionRefusal {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservationReplay(error) => {
                write!(formatter, "reservation replay rejects a record: {error}")
            },
            Self::RepositoryValidation(error) => error.fmt(formatter),
            Self::IncompleteFinalRecord => {
                formatter.write_str("bytes follow the journal's final newline")
            },
            Self::ReplayMismatch => formatter
                .write_str("the compacted journal replays differently from the original journal"),
        }
    }
}

impl From<CompactionRefusal> for JournalCompactionError {
    fn from(refusal: CompactionRefusal) -> Self { Self::Refused(refusal) }
}

impl From<LedgerError> for JournalCompactionError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}

impl From<std::io::Error> for JournalCompactionError {
    fn from(error: std::io::Error) -> Self { Self::Ledger(LedgerError::Io(error)) }
}

impl From<JournalError> for JournalCompactionError {
    fn from(error: JournalError) -> Self { Self::Ledger(LedgerError::from(error)) }
}

impl From<MutationLockError> for JournalCompactionError {
    fn from(error: MutationLockError) -> Self { Self::Ledger(LedgerError::from(error)) }
}

impl From<ProjectionError> for JournalCompactionError {
    fn from(error: ProjectionError) -> Self { Self::Ledger(LedgerError::from(error)) }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;
    use std::fs;
    use std::num::TryFromIntError;
    use std::ops::RangeInclusive;
    use std::os::unix::fs::MetadataExt;
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::thread;
    use std::time::Duration;
    use std::time::Instant;

    use serde_json::Value;
    use tempfile::TempDir;

    use super::CompactionPlan;
    use super::CompactionRefusal;
    use super::CompactionRefusalMarker;
    use super::JOURNAL_COMPACTION_TEMPORARY_FILE_NAME;
    use super::JournalCompaction;
    use super::JournalCompactionError;
    use super::Ledger;
    use super::MUTATING_VERB_CONTENTION_TOLERANCE;
    use super::MutationLock;
    use super::PublicationBoundary;
    use super::SupersededExtents;
    use super::replays_agree;
    use crate::ids::CoordinationRunId;
    use crate::ids::EventId;
    use crate::ids::ProjectionGeneration;
    use crate::ids::RepoInstanceId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::LedgerTransactionOutcome;
    use crate::ledger::TransactionValidation;
    use crate::ledger::journal;
    use crate::ledger::journal::Journal;
    use crate::ledger::journal::JournalEvent;
    use crate::ledger::journal::JournalOperation;
    use crate::ledger::journal::JournalReplay;
    use crate::ledger::projection;
    use crate::ledger::projection::ProjectionError;
    use crate::ledger::replay_checkpoint::ReplayCheckpoint;
    use crate::ledger::test_support;
    use crate::reservation::RetainedReservationSet;

    /// The merge extent observations inserted into each generated journal, before reservation
    /// replay rejects some of them.
    const GENERATED_OBSERVATIONS: usize = 12;
    /// The seeds of the generated journals.
    const GENERATED_SEEDS: RangeInclusive<u64> = 1..=8;
    /// The renewals one thread appends while another appends observations and compacts.
    const RACING_RENEWALS: usize = 10;
    /// The compactions that run while renewals append, each after one superseding observation.
    const RACING_COMPACTIONS: usize = 5;

    #[test]
    fn only_fully_superseded_extent_observations_drop() -> Result<(), Box<dyn Error>> {
        let first = ReservationId::new();
        let second = ReservationId::new();
        let operations = [
            legacy_extent(first)?,
            holder_extent(&[first, second])?,
            operation(&serde_json::json!({"op": "renew", "reservation_id": first.to_string()}))?,
            legacy_extent(second)?,
            holder_extent(&[first])?,
        ];
        let lengths = [10, 20, 30, 40, 50];
        let mut superseded = SupersededExtents::default();
        let kept = |superseded: &SupersededExtents, observed: &[JournalOperation]| {
            (0_u64..)
                .zip(observed)
                .map(|(ordinal, operation)| {
                    superseded.keeps(ordinal, operation.is_merge_extent_observation())
                })
                .collect::<Vec<_>>()
        };

        for (ordinal, (bytes, operation)) in (0_u64..).zip(lengths.iter().zip(&operations)).take(4)
        {
            superseded.observe(ordinal, *bytes, operation);
        }
        assert_eq!(
            kept(&superseded, &operations[..4]),
            [false, true, true, true],
            "the holder record still names the first reservation"
        );
        assert_eq!(superseded.droppable_bytes, 10);

        superseded.observe(4, lengths[4], &operations[4]);
        assert_eq!(
            kept(&superseded, &operations),
            [false, false, true, true, true]
        );
        assert_eq!(superseded.droppable_bytes, 10 + 20);
        Ok(())
    }

    #[test]
    fn the_final_record_and_generation_survive() -> Result<(), Box<dyn Error>> {
        let (events, _) = journal::round_trip_sequence_events()?;
        let (bytes, record_ends) = journal_bytes(&events)?;

        for (prefix_length, record_end) in record_ends.into_iter().enumerate().skip(1) {
            let prefix = &events[..prefix_length];
            let (replay, _) = JournalReplay::empty().extended_over(&bytes[..record_end])?;
            let last_extent = (0_u64..)
                .zip(prefix)
                .filter(|(_, event)| event.operation.merge_extent_reservations().is_some())
                .last();
            let final_ordinal = u64::try_from(prefix_length - 1)?;
            let final_event = &prefix[prefix_length - 1];

            assert!(replay.superseded.keeps(
                final_ordinal,
                final_event.operation.is_merge_extent_observation()
            ));
            assert_eq!(replay.generation, final_event.projection_generation);
            if let Some((ordinal, event)) = last_extent {
                assert!(
                    replay
                        .superseded
                        .keeps(ordinal, event.operation.is_merge_extent_observation())
                );
            }
        }
        Ok(())
    }

    #[test]
    fn droppable_bytes_equal_across_a_checkpoint_resume() -> Result<(), Box<dyn Error>> {
        let (events, _) = journal::round_trip_sequence_events()?;
        let (bytes, record_ends) = journal_bytes(&events)?;
        let (whole, _) = JournalReplay::empty().extended_over(&bytes)?;

        for record_end in record_ends {
            let (checkpointed, appended) = bytes.split_at(record_end);
            let (checkpointed, _) = JournalReplay::empty().extended_over(checkpointed)?;
            let decoded =
                serde_json::from_slice::<JournalReplay>(&serde_json::to_vec(&checkpointed)?)?;
            let (resumed, _) = decoded.extended_over(appended)?;

            assert_eq!(resumed.superseded, whole.superseded);
        }
        let expected = superseded_record_bytes(&events)?;
        assert!(expected > 0, "the sequence supersedes an observation");
        assert_eq!(whole.superseded.droppable_bytes, expected);
        Ok(())
    }

    #[test]
    fn compaction_preserves_every_replay_output() -> Result<(), Box<dyn Error>> {
        let repository = RepoInstanceId::new();
        let fold = journal::fold_sequence_events()?;
        let (round_trip, rejected) = journal::round_trip_sequence_events()?;
        let base = &round_trip[..rejected];
        let mut journals = vec![accepted_prefix(&fold).to_vec(), base.to_vec()];
        for seed in GENERATED_SEEDS {
            journals.push(generated_journal(base, seed)?);
        }
        let mut compacted_journals = 0;

        for mut events in journals {
            name_repository(&mut events, repository);
            let (bytes, _) = journal_bytes(&events)?;
            let Some(plan) = CompactionPlan::of(&bytes, repository)? else {
                assert_eq!(superseded_record_bytes(&events)?, 0);
                continue;
            };
            let (compacted, _) = JournalReplay::empty().extended_over(&plan.kept)?;

            assert_eq!(compacted.reservations, plan.replay.reservations);
            assert_eq!(
                compacted.coordination_events,
                plan.replay.coordination_events
            );
            assert_eq!(compacted.repositories, plan.replay.repositories);
            assert_eq!(compacted.generation, plan.replay.generation);
            assert!(is_line_subsequence(&plan.kept, &bytes));
            compacted_journals += 1;
        }
        assert!(
            compacted_journals > 1,
            "the round-trip and generated journals supersede observations"
        );
        Ok(())
    }

    #[test]
    fn a_second_compaction_drops_nothing() -> Result<(), Box<dyn Error>> {
        let fixture = CompactionFixture::round_trip()?;
        let original = fixture.journal()?;
        let JournalCompaction::Compacted { removed, remaining } =
            fixture.ledger.compact_journal()?
        else {
            return Err("the round-trip sequence supersedes two observations".into());
        };
        let compacted = fixture.journal()?;
        let inode = fs::metadata(&fixture.ledger.paths.journal)?.ino();

        assert_eq!(removed.records, 2);
        assert_eq!(
            removed.bytes + remaining.bytes,
            u64::try_from(original.len())?
        );
        assert_eq!(remaining.bytes, u64::try_from(compacted.len())?);
        assert_eq!(
            fixture.ledger.compact_journal()?,
            JournalCompaction::NothingToCompact
        );
        assert_eq!(fixture.journal()?, compacted);
        assert_eq!(fs::metadata(&fixture.ledger.paths.journal)?.ino(), inode);
        Ok(())
    }

    #[test]
    fn a_journal_whose_replay_fails_is_never_compacted() -> Result<(), Box<dyn Error>> {
        let (events, rejected) = journal::round_trip_sequence_events()?;
        assert!(superseded_record_bytes(&events)? > 0);
        let fixture = CompactionFixture::holding(events.clone())?;
        let original = fixture.journal()?;

        let refusal = fixture.ledger.compact_journal();
        assert!(
            matches!(
                refusal,
                Err(JournalCompactionError::Refused(
                    CompactionRefusal::ReservationReplay(_)
                ))
            ),
            "{refusal:?}"
        );
        assert_eq!(fixture.journal()?, original);
        assert!(
            !fixture
                .ledger
                .paths
                .directory
                .join(JOURNAL_COMPACTION_TEMPORARY_FILE_NAME)
                .exists()
        );

        let mut accepted = events[..rejected].to_vec();
        let repository = RepoInstanceId::new();
        name_repository(&mut accepted, repository);
        let (bytes, _) = journal_bytes(&accepted)?;
        let incomplete = [bytes.as_slice(), b"{\"schema_version\""].concat();
        assert!(matches!(
            CompactionPlan::of(&incomplete, repository),
            Err(JournalCompactionError::Refused(
                CompactionRefusal::IncompleteFinalRecord
            ))
        ));
        assert!(matches!(
            CompactionPlan::of(&bytes, RepoInstanceId::new()),
            Err(JournalCompactionError::Refused(
                CompactionRefusal::RepositoryValidation(_)
            ))
        ));
        Ok(())
    }

    #[test]
    fn superseded_bytes_equal_the_bytes_compaction_removes() -> Result<(), Box<dyn Error>> {
        let (events, rejected) = journal::round_trip_sequence_events()?;
        let mut events = events[..rejected].to_vec();
        let repository = RepoInstanceId::new();
        name_repository(&mut events, repository);
        let (bytes, record_ends) = journal_bytes(&events)?;
        let removed_bytes = |journal: &[u8]| -> Result<u64, Box<dyn Error>> {
            let kept = CompactionPlan::of(journal, repository)?
                .map_or(journal.len(), |plan| plan.kept.len());
            Ok(u64::try_from(journal.len() - kept)?)
        };

        for (length, record_end) in record_ends.iter().enumerate() {
            assert_eq!(
                removed_bytes(&bytes[..*record_end])?,
                superseded_record_bytes(&events[..length])?
            );
        }
        let removed = removed_bytes(&bytes)?;
        assert!(removed > 0, "the sequence supersedes an observation");
        for record_end in record_ends {
            let (checkpointed, appended) = bytes.split_at(record_end);
            let (checkpointed, _) = JournalReplay::empty().extended_over(checkpointed)?;
            let decoded =
                serde_json::from_slice::<JournalReplay>(&serde_json::to_vec(&checkpointed)?)?;
            let (resumed, _) = decoded.extended_over(appended)?;

            assert_eq!(resumed.superseded.droppable_bytes, removed);
        }
        Ok(())
    }

    #[test]
    fn appends_racing_compaction_all_survive() -> Result<(), Box<dyn Error>> {
        let (events, rejected) = journal::round_trip_sequence_events()?;
        let (renewal, observation) = renewal_and_observation(&events[..rejected])?;
        let fixture = Arc::new(CompactionFixture::holding(events[..rejected].to_vec())?);
        let start = Arc::new(Barrier::new(2));
        let renewals = {
            let fixture = Arc::clone(&fixture);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                (0..RACING_RENEWALS)
                    .map(|_| append(&fixture.ledger, renewal.clone()))
                    .collect::<Result<Vec<_>, _>>()
            })
        };
        start.wait();
        let mut observations = Vec::new();
        for _ in 0..RACING_COMPACTIONS {
            observations.push(append(&fixture.ledger, observation.clone())?);
            let compaction = fixture.ledger.compact_journal()?;
            assert!(
                matches!(compaction, JournalCompaction::Compacted { .. }),
                "{compaction:?}"
            );
        }
        let renewals = renewals
            .join()
            .map_err(|_| "the renewal thread panicked")??;
        let survivors = journal::complete_record_events(&fixture.journal()?)?
            .into_iter()
            .map(|event| event.event_id)
            .collect::<BTreeSet<_>>();

        assert!(renewals.iter().all(|event_id| survivors.contains(event_id)));
        let (superseded, latest) = observations.split_at(RACING_COMPACTIONS - 1);
        assert!(
            superseded
                .iter()
                .all(|event_id| !survivors.contains(event_id))
        );
        assert!(latest.iter().all(|event_id| survivors.contains(event_id)));
        fixture.ledger.read_validated_journal()?;
        Ok(())
    }

    #[test]
    fn lock_free_reads_never_see_cache_ahead_across_compaction() -> Result<(), Box<dyn Error>> {
        let fixture = CompactionFixture::round_trip()?;
        let paths = &fixture.ledger.paths;
        let checkpoint = ReplayCheckpoint::new(&paths.replay_checkpoint, fixture.repo_instance_id);
        let original_projection = fixture.repository.path().join("original-projection.json");
        let mut replays = Vec::new();
        let mut failures = Vec::new();

        // At each boundary a reader replays the journal, and every replay taken so far reads the
        // projection, as a reader that replayed at an earlier boundary would.
        let compaction = fixture.ledger.compact_journal_observing(|boundary| {
            if boundary == PublicationBoundary::BeforeProjection
                && let Err(error) = fs::copy(&paths.projection, &original_projection)
            {
                failures.push(format!("{boundary:?}: {error}"));
            }
            match Journal::replay_read_only_from_checkpoint(&paths.journal, &checkpoint) {
                Ok(replay) => replays.push(replay),
                Err(error) => failures.push(format!("{boundary:?}: {error}")),
            }
            for replay in &replays {
                if let Err(error) =
                    projection::read_validated(&paths.projection, fixture.repo_instance_id, replay)
                {
                    failures.push(format!("{boundary:?}: {error}"));
                }
            }
        })?;

        assert!(matches!(compaction, JournalCompaction::Compacted { .. }));
        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!(replays.len(), 3);
        // The reverse order would pair the compacted journal with the original projection.
        assert!(matches!(
            projection::read_validated(&original_projection, fixture.repo_instance_id, &replays[2]),
            Err(ProjectionError::CacheAhead)
        ));
        Ok(())
    }

    #[test]
    fn compaction_leaves_a_checkpoint_for_the_new_journal() -> Result<(), Box<dyn Error>> {
        let fixture = CompactionFixture::round_trip()?;
        let paths = &fixture.ledger.paths;

        assert!(matches!(
            fixture.ledger.compact_journal()?,
            JournalCompaction::Compacted { .. }
        ));
        let contents = fs::read(&paths.replay_checkpoint)?;
        let header_end = contents
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or("the checkpoint holds a header line")?;
        let header = serde_json::from_slice::<Value>(&contents[..header_end])?;
        let checkpointed = serde_json::from_slice::<JournalReplay>(&contents[header_end + 1..])?;
        let journal = fs::metadata(&paths.journal)?;

        assert_eq!(header["journal"]["device"], journal.dev());
        assert_eq!(header["journal"]["inode"], journal.ino());
        assert_eq!(header["end_offset"], journal.len());
        assert_eq!(checkpointed, Journal::replay_read_only(&paths.journal)?);
        Ok(())
    }

    #[test]
    fn a_pre_compaction_checkpoint_is_ignored() -> Result<(), Box<dyn Error>> {
        let fixture = CompactionFixture::round_trip()?;
        let paths = &fixture.ledger.paths;
        let checkpoint = ReplayCheckpoint::new(&paths.replay_checkpoint, fixture.repo_instance_id);
        checkpoint.store_for_journal_file(
            &Journal::replay_read_only(&paths.journal)?,
            &fs::metadata(&paths.journal)?,
            &fixture.journal()?,
        )?;
        let pre_compaction = fs::read(&paths.replay_checkpoint)?;

        assert!(matches!(
            fixture.ledger.compact_journal()?,
            JournalCompaction::Compacted { .. }
        ));
        // A lock-free reader of the original journal stores its checkpoint after compaction
        // stores the new journal's.
        fs::write(&paths.replay_checkpoint, pre_compaction)?;

        assert_eq!(
            Journal::replay_read_only_from_checkpoint(&paths.journal, &checkpoint)?,
            Journal::replay_read_only(&paths.journal)?
        );
        Ok(())
    }

    #[test]
    fn a_transaction_compacts_the_journal_once_superseded_records_reach_the_threshold()
    -> Result<(), Box<dyn Error>> {
        let (events, rejected) = journal::round_trip_sequence_events()?;
        let (renewal, observation) = renewal_and_observation(&events[..rejected])?;
        let mut fixture = CompactionFixture::holding(events[..rejected].to_vec())?;
        let droppable = superseded_bytes(&fixture.ledger)?;
        fixture.ledger.compaction_threshold_bytes = droppable + 1;
        let inode = fs::metadata(&fixture.ledger.paths.journal)?.ino();

        let renewed = append(&fixture.ledger, renewal)?;
        assert_eq!(fs::metadata(&fixture.ledger.paths.journal)?.ino(), inode);
        assert_eq!(superseded_bytes(&fixture.ledger)?, droppable);

        let observed = append(&fixture.ledger, observation)?;
        let survivors = journal::complete_record_events(&fixture.journal()?)?
            .into_iter()
            .map(|event| event.event_id)
            .collect::<BTreeSet<_>>();
        assert_ne!(fs::metadata(&fixture.ledger.paths.journal)?.ino(), inode);
        assert_eq!(superseded_bytes(&fixture.ledger)?, 0);
        assert!(survivors.contains(&renewed) && survivors.contains(&observed));
        fixture.ledger.read_validated_journal()?;
        Ok(())
    }

    #[test]
    fn a_due_compaction_skips_a_held_lock_without_waiting() -> Result<(), Box<dyn Error>> {
        let mut fixture = CompactionFixture::round_trip()?;
        let droppable = superseded_bytes(&fixture.ledger)?;
        fixture.ledger.compaction_threshold_bytes = droppable;
        let original = fixture.journal()?;
        let _holder = MutationLock::acquire(
            &fixture.ledger.paths.lock,
            MUTATING_VERB_CONTENTION_TOLERANCE,
        )?;

        let started_at = Instant::now();
        let compaction = fixture
            .ledger
            .attempt_due_compaction(droppable, replays_agree)?;
        // A compaction that waited for the lock would spend the whole contention tolerance.
        assert!(started_at.elapsed() < Duration::from_secs(1));
        assert_eq!(compaction, None);
        assert_eq!(fixture.journal()?, original);
        assert!(!fixture.ledger.paths.compaction_refusal.exists());
        Ok(())
    }

    #[test]
    fn a_refused_compaction_waits_for_a_threshold_of_growth() -> Result<(), Box<dyn Error>> {
        let (events, rejected) = journal::round_trip_sequence_events()?;
        let (renewal, _) = renewal_and_observation(&events[..rejected])?;
        let mut fixture = CompactionFixture::holding(events[..rejected].to_vec())?;
        let droppable = superseded_bytes(&fixture.ledger)?;
        fixture.ledger.compaction_threshold_bytes = droppable;
        let fixture = fixture;
        let ledger = &fixture.ledger;
        let original = fixture.journal()?;
        let disagree = |_: &JournalReplay, _: &JournalReplay| false;

        let refusal = ledger.attempt_due_compaction(droppable, disagree);
        assert!(
            matches!(
                refusal,
                Err(JournalCompactionError::Refused(
                    CompactionRefusal::ReplayMismatch
                ))
            ),
            "{refusal:?}"
        );
        assert_eq!(fixture.journal()?, original);
        Ledger::repair_projection(fixture.repository.path())?;
        assert!(!ledger.paths.compaction_refusal.exists());

        ledger
            .attempt_due_compaction(droppable, disagree)
            .err()
            .ok_or("the disagreeing compaction completed")?;
        let refused = fs::metadata(&ledger.paths.journal)?;
        let marker = serde_json::from_slice::<CompactionRefusalMarker>(&fs::read(
            &ledger.paths.compaction_refusal,
        )?)?;
        assert_eq!(
            (marker.device, marker.inode, marker.end_offset),
            (refused.dev(), refused.ino(), refused.len())
        );
        assert_eq!(
            ledger.attempt_due_compaction(droppable, replays_agree)?,
            None
        );
        assert_eq!(fixture.journal()?, original);

        let limit = refused.len() + droppable;
        let mut appends = 0;
        loop {
            append(ledger, renewal.clone())?;
            appends += 1;
            let journal = fs::metadata(&ledger.paths.journal)?;
            if journal.ino() != refused.ino() {
                break;
            }
            assert!(
                journal.len() <= limit,
                "a journal grown to {} bytes, past {limit}, stayed uncompacted",
                journal.len()
            );
        }
        assert!(appends > 1, "one renewal outgrew the threshold");
        assert!(!ledger.paths.compaction_refusal.exists());
        assert_eq!(superseded_bytes(ledger)?, 0);
        Ok(())
    }

    /// The byte length, newline included, of every merge extent observation whose every
    /// reservation a later observation names.
    fn superseded_record_bytes(events: &[JournalEvent]) -> Result<u64, Box<dyn Error>> {
        let mut total = 0;
        for (index, event) in events.iter().enumerate() {
            let Some(reservations) = event.operation.merge_extent_reservations() else {
                continue;
            };
            let named_later = events[index + 1..]
                .iter()
                .filter_map(|later| later.operation.merge_extent_reservations())
                .flatten()
                .collect::<BTreeSet<_>>();
            if reservations.is_subset(&named_later) {
                total += u64::try_from(serde_json::to_vec(event)?.len() + 1)?;
            }
        }
        Ok(total)
    }

    /// The journal holding `events`, and the byte offset after each record, starting at zero.
    fn journal_bytes(events: &[JournalEvent]) -> Result<(Vec<u8>, Vec<usize>), serde_json::Error> {
        let mut bytes = Vec::new();
        let mut record_ends = vec![0];
        for event in events {
            bytes.extend(serde_json::to_vec(event)?);
            bytes.push(b'\n');
            record_ends.push(bytes.len());
        }
        Ok((bytes, record_ends))
    }

    fn legacy_extent(reservation_id: ReservationId) -> Result<JournalOperation, serde_json::Error> {
        operation(&serde_json::json!({
            "op": "merge_extent_observed",
            "reservation_id": reservation_id.to_string(),
            "extent": empty_extent(),
            "run_status": "editing",
        }))
    }

    fn holder_extent(
        reservation_ids: &[ReservationId],
    ) -> Result<JournalOperation, serde_json::Error> {
        let reservations = reservation_ids
            .iter()
            .map(|reservation_id| {
                serde_json::json!({
                    "reservation_id": reservation_id.to_string(),
                    "run_status": "editing",
                })
            })
            .collect::<Vec<_>>();
        operation(&serde_json::json!({
            "op": "holder_merge_extent_observed",
            "extent": empty_extent(),
            "reservations": reservations,
        }))
    }

    fn empty_extent() -> Value {
        serde_json::json!({
            "status": "empty",
            "key": {
                "trunk": "1111111111111111111111111111111111111111",
                "head": "2222222222222222222222222222222222222222",
                "working_tree": {"tracked_paths": [], "untracked_paths": []},
            },
        })
    }

    fn operation(operation: &Value) -> Result<JournalOperation, serde_json::Error> {
        serde_json::from_value(operation.clone())
    }

    /// A ledger in a scratch repository whose journal holds chosen events, beside that journal's
    /// projection.
    struct CompactionFixture {
        repository:       TempDir,
        ledger:           Ledger,
        repo_instance_id: RepoInstanceId,
    }

    impl CompactionFixture {
        /// A ledger whose journal holds `events`, each rewritten to name the ledger's repository.
        fn holding(mut events: Vec<JournalEvent>) -> Result<Self, Box<dyn Error>> {
            let repository = test_support::scratch_repository();
            Ledger::initialize(repository.path())?;
            let ledger = Ledger::open(repository.path())?;
            let repo_instance_id = ledger.repository_identity()?;
            name_repository(&mut events, repo_instance_id);
            fs::write(&ledger.paths.journal, journal_bytes(&events)?.0)?;
            Ledger::repair_projection(repository.path())?;
            Ok(Self {
                repository,
                ledger,
                repo_instance_id,
            })
        }

        /// A ledger whose journal holds the round-trip sequence before its rejected record,
        /// which supersedes two observations.
        fn round_trip() -> Result<Self, Box<dyn Error>> {
            let (events, rejected) = journal::round_trip_sequence_events()?;
            Self::holding(events[..rejected].to_vec())
        }

        fn journal(&self) -> std::io::Result<Vec<u8>> { fs::read(&self.ledger.paths.journal) }
    }

    /// A renewal of the reservation of the latest legacy merge extent observation in `events`,
    /// and that observation, whose append supersedes it.
    fn renewal_and_observation(
        events: &[JournalEvent],
    ) -> Result<(JournalOperation, JournalOperation), Box<dyn Error>> {
        let (reservation_id, observation) = events
            .iter()
            .rev()
            .find_map(|event| match &event.operation {
                JournalOperation::MergeExtentObserved { reservation_id, .. } => {
                    Some((*reservation_id, event.operation.clone()))
                },
                _ => None,
            })
            .ok_or("the events record a legacy observation")?;
        Ok((JournalOperation::Renew { reservation_id }, observation))
    }

    /// The superseded record bytes of `ledger`'s journal.
    fn superseded_bytes(ledger: &Ledger) -> Result<u64, Box<dyn Error>> {
        let droppable = Journal::replay_read_only(&ledger.paths.journal)?
            .superseded
            .droppable_bytes;
        Ok(droppable)
    }

    /// Append `operation` through a ledger transaction and return its event id.
    fn append(ledger: &Ledger, operation: JournalOperation) -> Result<EventId, String> {
        match ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
            TransactionValidation::<()>::Append(Box::new(operation))
        }) {
            Ok(LedgerTransactionOutcome::Appended { event, .. }) => Ok(event.event_id),
            Ok(LedgerTransactionOutcome::Rejected(())) => Err("the append was rejected".to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Rewrite every event to name `repository`, as a journal a compaction accepts does.
    fn name_repository(events: &mut [JournalEvent], repository: RepoInstanceId) {
        for event in events {
            event.actor.repository = repository;
        }
    }

    /// The longest prefix of `events` that reservation replay accepts.
    fn accepted_prefix(events: &[JournalEvent]) -> &[JournalEvent] {
        let accepted = (0..=events.len())
            .rev()
            .find(|length| RetainedReservationSet::replay(&events[..*length]).is_ok())
            .unwrap_or(0);
        &events[..accepted]
    }

    /// `base` with up to [`GENERATED_OBSERVATIONS`] merge extent observations inserted, each at a
    /// position `seed` selects, naming one or two claimed reservations in either record form, and
    /// kept only where reservation replay accepts it.
    fn generated_journal(
        base: &[JournalEvent],
        seed: u64,
    ) -> Result<Vec<JournalEvent>, Box<dyn Error>> {
        let claimed = base
            .iter()
            .filter_map(|event| match &event.operation {
                JournalOperation::Claim { reservation_id, .. } => Some(*reservation_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut random = Xorshift(seed);
        let mut events = base.to_vec();
        for _ in 0..GENERATED_OBSERVATIONS {
            let first = claimed[random.below(claimed.len())?];
            let second = claimed[random.below(claimed.len())?];
            let operation = match (first == second, random.below(2)? == 0) {
                (true, true) => legacy_extent(first)?,
                (true, false) => holder_extent(&[first])?,
                (false, _) => holder_extent(&[first, second])?,
            };
            let mut event = events[0].clone();
            event.event_id = EventId::new();
            event.operation = operation;
            let position = 1 + random.below(events.len())?;
            events.insert(position, event);
            if RetainedReservationSet::replay(&events).is_err() {
                events.remove(position);
            }
        }
        if events.len() == base.len() {
            return Err(format!("seed {seed} inserted no observation").into());
        }
        for (generation, event) in (1_u64..).zip(&mut events) {
            event.projection_generation = ProjectionGeneration::from(generation);
        }
        Ok(events)
    }

    /// A xorshift generator, so each seed selects the same journal on every run.
    struct Xorshift(u64);

    impl Xorshift {
        /// The next value below `bound`.
        fn below(&mut self, bound: usize) -> Result<usize, TryFromIntError> {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            usize::try_from(self.0 % u64::try_from(bound)?)
        }
    }

    /// Whether every line of `kept` appears, byte for byte and in order, among the lines of
    /// `journal`.
    fn is_line_subsequence(kept: &[u8], journal: &[u8]) -> bool {
        let mut lines = journal.split_inclusive(|byte| *byte == b'\n');
        kept.split_inclusive(|byte| *byte == b'\n')
            .all(|kept_line| lines.any(|line| line == kept_line))
    }
}
