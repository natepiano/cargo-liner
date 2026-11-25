//! The compaction that rewrites the journal without superseded observations and expired history.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::File;
use std::fs::Metadata;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;

use uuid::Uuid;

use super::maintenance;
use super::maintenance::HistoryCompactionNoProgress;
use crate::ids::RecordedAt;
use crate::ids::RepoInstanceId;
use crate::ledger::constants::JOURNAL_COMPACTION_TEMPORARY_FILE_NAME;
use crate::ledger::constants::MUTATING_VERB_CONTENTION_TOLERANCE;
use crate::ledger::error::LedgerError;
use crate::ledger::handle::Ledger;
use crate::ledger::history::ExpiredHistory;
use crate::ledger::history::HistoryCompactionBasis;
use crate::ledger::identity;
use crate::ledger::journal::Journal;
use crate::ledger::journal::JournalError;
use crate::ledger::journal::JournalEvent;
use crate::ledger::journal::JournalReplay;
use crate::ledger::lock::MutationLockError;
use crate::ledger::preparation;
use crate::ledger::projection::Projection;
use crate::ledger::projection::ProjectionError;
use crate::ledger::replay_checkpoint::ReplayCheckpoint;
use crate::reservation::ReservationReplayError;

/// What one journal compaction did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JournalCompaction {
    /// No superseded observation or expired, unreferenced history can be removed.
    NothingToCompact,
    /// The journal was rewritten without superseded observations and expired, unreferenced history.
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
    pub(crate) records: u64,
    /// The byte length of the records.
    pub(crate) bytes:   u64,
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
pub(super) enum PublicationBoundary {
    /// The compacted journal is written and verified, and the ledger still shows the original.
    BeforeProjection,
    /// The compacted journal's projection sits beside the original journal.
    BeforeRename,
    /// The compacted journal and its projection are both published.
    AfterRename,
}

/// A journal's replay and the records its compaction keeps.
struct CompactionPlan {
    /// The replay of the whole journal.
    replay:         JournalReplay,
    /// The kept records, byte for byte, in append order.
    kept:           Vec<u8>,
    /// The original byte and record counts, before pruning its expected fold.
    original:       JournalRecords,
    /// Whether new records can follow this verified prefix without changing its pruning plan.
    source_changes: CompactionSourceChanges,
}

/// A plan to replace the journal or the facts that ruled out a replacement.
enum CompactionPlanning {
    Rewrite(Box<CompactionPlan>),
    Unchanged(HistoryCompactionBasis),
}

/// Which source changes preserve a verified compaction prefix.
#[derive(Clone, Copy)]
enum CompactionSourceChanges {
    /// Superseded observations leave no replay state; later records can follow either prefix.
    AppendedRecords,
    /// New references can make expired history necessary again, requiring a new pruning plan.
    ExactJournal,
}

/// A verified rewrite bound to the exact journal file it replaces.
pub(super) struct PreparedJournalCompaction {
    source:            Metadata,
    /// Byte length actually read and verified, including appends racing the source metadata read.
    source_end:        u64,
    source_changes:    CompactionSourceChanges,
    journal_path:      PathBuf,
    checkpoint_path:   PathBuf,
    temporary_file:    File,
    replay:            JournalReplay,
    pub(super) result: JournalCompaction,
}

/// Whether the journal has a verified rewrite ready for publication.
pub(super) enum CompactionPreparation {
    /// No observation or expired history can be removed.
    Unchanged(Option<HistoryCompactionNoProgress>),
    /// A verified replacement bound to its source journal.
    Ready(Box<PreparedJournalCompaction>),
}

/// The lock on the journal file that a compaction replaces. A replay checkpoint writer takes the
/// same lock, so no replay of that file publishes a checkpoint while the compaction publishes.
pub(super) struct OriginalJournalLock {
    _journal: File,
}

/// Whether automatic compaction holds the [`OriginalJournalLock`] without waiting.
pub(super) enum OriginalJournalLockAttempt {
    /// The compaction holds the lock and can publish.
    Acquired(OriginalJournalLock),
    /// A replay checkpoint writer holds the lock, so the compaction gives up this attempt.
    Busy,
}

impl OriginalJournalLock {
    /// Wait until no replay checkpoint writer holds the journal at `journal_path`, then lock it.
    fn acquire(journal_path: &Path) -> std::io::Result<Self> {
        let journal = File::open(journal_path)?;
        journal.lock()?;
        Ok(Self { _journal: journal })
    }

    /// Lock the journal at `journal_path` without waiting.
    pub(super) fn try_acquire(journal_path: &Path) -> std::io::Result<OriginalJournalLockAttempt> {
        let journal = File::open(journal_path)?;
        match journal.try_lock() {
            Ok(()) => Ok(OriginalJournalLockAttempt::Acquired(Self {
                _journal: journal,
            })),
            Err(TryLockError::WouldBlock) => Ok(OriginalJournalLockAttempt::Busy),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

impl PreparedJournalCompaction {
    /// Include only the newly committed tail while holding the mutation lock. A rewrite that
    /// prunes expired history needs a new dependency plan instead.
    pub(super) fn include_committed_tail(
        &mut self,
        path: &Path,
    ) -> Result<bool, JournalCompactionError> {
        if self.still_current(path)? {
            return Ok(true);
        }
        if matches!(self.source_changes, CompactionSourceChanges::ExactJournal) {
            return Ok(false);
        }
        let mut source = fs::File::open(path)?;
        let current = source.metadata()?;
        if current.dev() != self.source.dev()
            || current.ino() != self.source.ino()
            || current.len() < self.source_end
        {
            return Ok(false);
        }
        source.seek(SeekFrom::Start(self.source_end))?;
        let mut appended = Vec::new();
        source.read_to_end(&mut appended)?;
        let (replay, complete_end) =
            std::mem::replace(&mut self.replay, JournalReplay::empty()).extended_over(&appended)?;
        if complete_end != appended.len() {
            return Err(CompactionRefusal::IncompleteFinalRecord.into());
        }
        if let Err(error) = &replay.reservations {
            return Err(CompactionRefusal::ReservationReplay(error.clone()).into());
        }
        let mut replacement = OpenOptions::new().append(true).open(&self.journal_path)?;
        replacement.write_all(&appended)?;
        replacement.sync_all()?;
        if let JournalCompaction::Compacted { remaining, .. } = &mut self.result {
            remaining.records = replay.record_count;
            remaining.bytes = u64::from(replay.end_offset);
        }
        self.replay = replay;
        // The checkpoint still certifies its unchanged prefix. Its reader advances this tail.
        self.source_end = current.len();
        self.source = current;
        Ok(true)
    }

    fn still_current(&self, path: &Path) -> std::io::Result<bool> {
        let current = fs::metadata(path)?;
        let stamp = |metadata: &Metadata| {
            (
                metadata.dev(),
                metadata.ino(),
                metadata.len(),
                metadata.mtime(),
                metadata.mtime_nsec(),
            )
        };
        Ok(self.source_end == current.len() && stamp(&self.source) == stamp(&current))
    }
}

impl Drop for PreparedJournalCompaction {
    fn drop(&mut self) {
        std::mem::drop(fs::remove_file(&self.journal_path));
        std::mem::drop(fs::remove_file(&self.checkpoint_path));
    }
}

impl Ledger {
    /// Rewrite the journal without superseded observations and expired, unreferenced history,
    /// preparing the rewrite before taking the mutation lock for publication.
    ///
    /// Replays the whole journal, writes the kept records to a temporary file, replays that file,
    /// and refuses unless both replays agree. Then stores the replay checkpoint for the temporary
    /// file, publishes its projection, and renames it over the journal. The projection publishes
    /// first because lock-free readers replay the journal and then read the projection: a reader
    /// of the original journal meets the new projection as one behind its replay, a rebuild to do,
    /// where the reverse order would show a reader of the compacted journal the original
    /// projection, ahead of its replay.
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
        self.remove_abandoned_compaction_files()?;
        let repo_instance_id = identity::read_repo_instance_id(&self.paths.repo_instance_id)?;
        let now = RecordedAt::now();
        let (lock, (), preparation) = preparation::prepare_with_lock(
            &self.paths.lock,
            MUTATING_VERB_CONTENTION_TOLERANCE,
            (),
            |()| self.prepare_compaction(repo_instance_id, replays_agree, &now),
            |(), preparation| match preparation {
                CompactionPreparation::Unchanged(_) => Ok(true),
                CompactionPreparation::Ready(prepared) => {
                    prepared.include_committed_tail(&self.paths.journal)
                },
            },
        )?;
        let result = match preparation {
            CompactionPreparation::Unchanged(_) => Ok(JournalCompaction::NothingToCompact),
            CompactionPreparation::Ready(prepared) => {
                OriginalJournalLock::acquire(&self.paths.journal)
                    .map_err(JournalCompactionError::from)
                    .and_then(|original_journal| {
                        self.publish_compacted(
                            &prepared,
                            original_journal,
                            repo_instance_id,
                            &mut at_boundary,
                        )
                    })
                    .map(|()| prepared.result)
            },
        };
        std::mem::drop(lock);
        result
    }

    /// Read, prune, write and verify a rewrite without owning the mutation lock.
    pub(super) fn prepare_compaction(
        &self,
        repo_instance_id: RepoInstanceId,
        agree: fn(&JournalReplay, &JournalReplay) -> bool,
        now: &RecordedAt,
    ) -> Result<CompactionPreparation, JournalCompactionError> {
        let mut journal = File::open(&self.paths.journal)?;
        let source = journal.metadata()?;
        let mut bytes = Vec::new();
        journal.read_to_end(&mut bytes)?;
        let plan = match CompactionPlan::of(&bytes, repo_instance_id, now)? {
            CompactionPlanning::Rewrite(plan) => plan,
            CompactionPlanning::Unchanged(basis) => {
                let marker = HistoryCompactionNoProgress::for_source(
                    &source,
                    &journal.metadata()?,
                    u64::try_from(bytes.len()).map_err(std::io::Error::other)?,
                    basis,
                );
                return Ok(CompactionPreparation::Unchanged(marker));
            },
        };
        drop(bytes);
        let journal_path = self.paths.directory.join(format!(
            "{JOURNAL_COMPACTION_TEMPORARY_FILE_NAME}.{}",
            Uuid::now_v7()
        ));
        let checkpoint_path = journal_path.with_added_extension("checkpoint");
        let temporary_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&journal_path)?;
        let mut prepared = PreparedJournalCompaction {
            source,
            source_end: plan.original.bytes,
            source_changes: plan.source_changes,
            journal_path,
            checkpoint_path,
            temporary_file,
            replay: JournalReplay::empty(),
            result: JournalCompaction::NothingToCompact,
        };
        prepared.temporary_file.lock()?;
        let mut temporary_file = &prepared.temporary_file;
        temporary_file.write_all(&plan.kept)?;
        temporary_file.sync_all()?;
        let compacted = Journal::replay_read_only(&prepared.journal_path)?;
        if !agree(&plan.replay, &compacted) {
            return Err(CompactionRefusal::ReplayMismatch.into());
        }
        std::mem::drop(
            ReplayCheckpoint::new(&prepared.checkpoint_path, repo_instance_id)
                .store_for_journal_file(&compacted, &temporary_file.metadata()?, &plan.kept),
        );
        prepared.result = JournalCompaction::Compacted {
            removed:   JournalRecords {
                records: plan.original.records - compacted.record_count,
                bytes:   plan.original.bytes - u64::from(compacted.end_offset),
            },
            remaining: JournalRecords {
                records: compacted.record_count,
                bytes:   u64::from(compacted.end_offset),
            },
        };
        prepared.replay = compacted;
        Ok(CompactionPreparation::Ready(Box::new(prepared)))
    }

    /// Publish an already verified rewrite while its source file is still current.
    ///
    /// `original_journal` holds the source file, so a replay of it finishes publishing its
    /// checkpoint before either rename, and none starts until both renames are done.
    pub(super) fn publish_compacted(
        &self,
        prepared: &PreparedJournalCompaction,
        original_journal: OriginalJournalLock,
        repo_instance_id: RepoInstanceId,
        at_boundary: &mut impl FnMut(PublicationBoundary),
    ) -> Result<(), JournalCompactionError> {
        if prepared.checkpoint_path.is_file() {
            fs::rename(&prepared.checkpoint_path, &self.paths.replay_checkpoint)?;
        }
        at_boundary(PublicationBoundary::BeforeProjection);
        Projection::from_replay(repo_instance_id, &prepared.replay)
            .publish(&self.paths.directory, &self.paths.projection)?;
        fs::File::open(&self.paths.directory)?.sync_all()?;
        at_boundary(PublicationBoundary::BeforeRename);
        fs::rename(&prepared.journal_path, &self.paths.journal)?;
        fs::File::open(&self.paths.directory)?.sync_all()?;
        at_boundary(PublicationBoundary::AfterRename);
        std::mem::drop(original_journal);
        std::mem::drop(maintenance::remove_refusal(&self.paths.compaction_refusal));
        Ok(())
    }
}

impl CompactionPlan {
    /// Plan a replacement or record why the journal has no records to remove.
    fn of(
        journal: &[u8],
        repo_instance_id: RepoInstanceId,
        now: &RecordedAt,
    ) -> Result<CompactionPlanning, JournalCompactionError> {
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
        let events = journal
            .split_inclusive(|byte| *byte == b'\n')
            .map(serde_json::from_slice)
            .collect::<Result<Vec<JournalEvent>, _>>()
            .map_err(LedgerError::JournalEncoding)?;
        let kept_extents = merge_extent_observations
            .into_iter()
            .zip(0_u64..)
            .map(|(observation, ordinal)| replay.superseded.keeps(ordinal, observation))
            .collect::<Vec<_>>();
        let expired = ExpiredHistory::of(&replay, &events, &kept_extents, now)
            .map_err(LedgerError::JournalEncoding)?;
        if replay.superseded.droppable_bytes == 0 && expired.is_empty() {
            return Ok(CompactionPlanning::Unchanged(expired.basis));
        }
        let original = JournalRecords {
            records: replay.record_count,
            bytes:   u64::from(replay.end_offset),
        };
        let mut kept = Vec::with_capacity(journal.len());
        for (ordinal, record) in journal.split_inclusive(|byte| *byte == b'\n').enumerate() {
            if kept_extents[ordinal] && expired.keeps(ordinal) {
                kept.extend_from_slice(record);
            }
        }
        let source_changes = if expired.is_empty() {
            CompactionSourceChanges::AppendedRecords
        } else {
            CompactionSourceChanges::ExactJournal
        };
        let mut replay = replay;
        expired
            .prune(&mut replay)
            .map_err(LedgerError::JournalEncoding)?;
        Ok(CompactionPlanning::Rewrite(Box::new(Self {
            replay,
            kept,
            original,
            source_changes,
        })))
    }
}

/// Whether `compacted` replays to everything `original` does: its reservations, coordination
/// records, repositories, and generation.
pub(super) fn replays_agree(original: &JournalReplay, compacted: &JournalReplay) -> bool {
    original.reservations == compacted.reservations
        && original.finished_history == compacted.finished_history
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
    use std::io::Write;
    use std::num::TryFromIntError;
    use std::ops::RangeInclusive;
    use std::os::unix::fs::MetadataExt;
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::thread;

    use serde_json::Value;

    use super::CompactionPlan;
    use super::CompactionPlanning;
    use super::CompactionRefusal;
    use super::JOURNAL_COMPACTION_TEMPORARY_FILE_NAME;
    use super::JournalCompaction;
    use super::JournalCompactionError;
    use super::Ledger;
    use super::MUTATING_VERB_CONTENTION_TOLERANCE;
    use super::OriginalJournalLock;
    use super::PublicationBoundary;
    use super::replays_agree;
    use crate::answer::ConflictAuthorization;
    use crate::ids::CoordinationRunId;
    use crate::ids::EventId;
    use crate::ids::ProjectionGeneration;
    use crate::ids::RecordedAt;
    use crate::ids::RepoInstanceId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::LedgerReconciliationOutcome;
    use crate::ledger::ReconciliationValidation;
    use crate::ledger::WorktreeContext;
    use crate::ledger::compaction::fixtures;
    use crate::ledger::compaction::fixtures::CompactionFixture;
    use crate::ledger::compaction::fixtures::empty_extent;
    use crate::ledger::journal;
    use crate::ledger::journal::Journal;
    use crate::ledger::journal::JournalEvent;
    use crate::ledger::journal::JournalOperation;
    use crate::ledger::journal::JournalReplay;
    use crate::ledger::lock::MutationLock;
    use crate::ledger::projection;
    use crate::ledger::projection::ProjectionError;
    use crate::ledger::replay_checkpoint::ReplayCheckpoint;
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
    fn expired_finished_history_compacts_without_superseded_extents() -> Result<(), Box<dyn Error>>
    {
        let (events, expired, live) = fixtures::finished_history_events()?;
        let fixture = CompactionFixture::holding(events)?;
        assert_eq!(fixtures::superseded_bytes(&fixture.ledger)?, 0);
        let before = fixture.journal()?.len();
        fixtures::append(
            &fixture.ledger,
            &JournalOperation::Renew {
                reservation_id: live,
            },
        )?;
        let replay = Journal::replay_read_only(&fixture.ledger.paths.journal)?;
        let retained = replay.reservations?;
        assert!(
            retained.reservation(expired).is_err(),
            "expired finished reservation leaves the fold"
        );
        assert!(
            retained.reservation(live).is_ok(),
            "live reservation survives"
        );
        assert!(
            fixture.journal()?.len() < before,
            "the journal shrinks independently of the extent threshold"
        );
        let checkpoint = Journal::replay_read_only_from_checkpoint(
            &fixture.ledger.paths.journal,
            &ReplayCheckpoint::new(
                &fixture.ledger.paths.replay_checkpoint,
                fixture.repo_instance_id,
            ),
        )?;
        assert!(checkpoint.reservations?.reservation(expired).is_err());
        Ok(())
    }

    #[test]
    fn history_pruning_keeps_live_edge_proofs_open_alerts_and_recent_dispositions()
    -> Result<(), Box<dyn Error>> {
        let template = journal::fold_sequence_events()?
            .into_iter()
            .find(|event| matches!(event.operation, JournalOperation::Claim { .. }))
            .ok_or("claim fixture")?;
        let predecessor = ReservationId::new();
        let alert = ReservationId::new();
        let recent = ReservationId::new();
        let live = ReservationId::new();
        let expired = ReservationId::new();
        let old = "2020-01-01T00:00:00.000Z".parse::<RecordedAt>()?;
        let mut events = Vec::new();
        for id in [predecessor, alert, recent, live, expired] {
            let mut claim = template.clone();
            claim.at = if id == recent {
                crate::ids::RecordedAt::now()
            } else {
                old.clone()
            };
            claim.event_id = EventId::new();
            if let JournalOperation::Claim {
                reservation_id,
                authorization,
                ..
            } = &mut claim.operation
            {
                *reservation_id = id;
                **authorization = ConflictAuthorization::NoConflict;
            }
            events.push(claim);
            if id == live {
                continue;
            }
            if id == predecessor || id == alert {
                for value in [
                    serde_json::json!({"op":"checkpoint","reservation_id":id,"protected_tip":"2222222222222222222222222222222222222222","trunk_snapshot":"1111111111111111111111111111111111111111"}),
                    serde_json::json!({"op":"evidence_revalidated","reservation_id":id,"status":serde_json::json!({"status":"integrated","trunk_oid":"1111111111111111111111111111111111111111"}),"edit_blocking_status":"clear"}),
                ] {
                    let mut event = template.clone();
                    event.at = old.clone();
                    event.event_id = EventId::new();
                    event.operation = fixtures::operation(&value)?;
                    events.push(event);
                }
            }
            let mut release = template.clone();
            release.at = if id == recent {
                crate::ids::RecordedAt::now()
            } else {
                old.clone()
            };
            release.event_id = EventId::new();
            release.operation = fixtures::operation(
                &serde_json::json!({"op":"release","reservation_id":id,"disposition":if id == predecessor || id == alert { serde_json::json!({"kind":"integrated"}) } else { serde_json::json!({"kind":"retired_orphan","evidence":"finished audit fixture"}) }}),
            )?;
            events.push(release);
            if id == alert {
                let mut lost = events.last().ok_or("release fixture")?.clone();
                lost.event_id = EventId::new();
                lost.operation = fixtures::operation(
                    &serde_json::json!({"op":"evidence_revalidated","reservation_id":id,"status":{"status":"not_integrated"},"edit_blocking_status":"clear"}),
                )?;
                events.push(lost);
            }
        }
        let mut sequence = template;
        sequence.operation = fixtures::operation(
            &serde_json::json!({"op":"sequence","predecessor":predecessor,"successor":live,"edge_id":crate::ids::EdgeId::new(),"scopes":[{"path":"src","kind":"tree"}],"reason":"preserve predecessor proof"}),
        )?;
        events.push(sequence);
        for (index, event) in events.iter_mut().enumerate() {
            event.projection_generation = ProjectionGeneration::from(u64::try_from(index)? + 1);
        }
        let fixture = CompactionFixture::holding(events)?;
        let before = Journal::replay_read_only(&fixture.ledger.paths.journal)?.reservations?;
        fixture.ledger.compact_journal()?;
        let after = Journal::replay_read_only(&fixture.ledger.paths.journal)?.reservations?;
        assert!(after.reservation(expired).is_err());
        for id in [predecessor, alert, recent, live] {
            assert_eq!(
                after.reservation(id)?,
                before.reservation(id)?,
                "retained dependency keeps all historical integration witnesses and scoped-patch evidence"
            );
        }
        Ok(())
    }

    #[test]
    fn expired_empty_outstanding_checkpoint_keeps_its_recovery_evidence()
    -> Result<(), Box<dyn Error>> {
        let mut claim = journal::fold_sequence_events()?
            .into_iter()
            .find(|event| matches!(event.operation, JournalOperation::Claim { .. }))
            .ok_or("claim fixture")?;
        claim.at = "2020-01-01T00:00:00.000Z".parse()?;
        let outstanding = ReservationId::new();
        if let JournalOperation::Claim {
            reservation_id,
            authorization,
            ..
        } = &mut claim.operation
        {
            *reservation_id = outstanding;
            **authorization = ConflictAuthorization::NoConflict;
        }
        let mut checkpoint = claim.clone();
        checkpoint.event_id = EventId::new();
        checkpoint.operation = fixtures::operation(
            &serde_json::json!({"op":"checkpoint", "reservation_id":outstanding, "protected_tip":"2222222222222222222222222222222222222222", "trunk_snapshot":"1111111111111111111111111111111111111111"}),
        )?;
        let mut observed = claim.clone();
        observed.event_id = EventId::new();
        observed.operation = fixtures::operation(
            &serde_json::json!({"op":"merge_extent_observed", "reservation_id":outstanding, "extent":empty_extent(), "run_status":"ended"}),
        )?;
        let mut anchor = claim.clone();
        anchor.at = RecordedAt::now();
        anchor.event_id = EventId::new();
        if let JournalOperation::Claim { reservation_id, .. } = &mut anchor.operation {
            *reservation_id = ReservationId::new();
        }
        let mut events = vec![claim, checkpoint, observed, anchor];
        for (index, event) in events.iter_mut().enumerate() {
            event.projection_generation = ProjectionGeneration::from(u64::try_from(index)? + 1);
        }
        let fixture = CompactionFixture::holding(events)?;
        let before = Journal::replay_read_only(&fixture.ledger.paths.journal)?.reservations?;
        assert!(before.reservation(outstanding)?.is_terminal());
        fixture.ledger.compact_journal()?;
        let after = Journal::replay_read_only(&fixture.ledger.paths.journal)?.reservations?;
        assert_eq!(
            after.reservation(outstanding)?,
            before.reservation(outstanding)?,
            "an orphan alert can still need an outstanding checkpoint's protected tip"
        );
        Ok(())
    }

    #[test]
    fn compaction_preserves_the_first_terminal_observation_clock() -> Result<(), Box<dyn Error>> {
        let mut claim = journal::fold_sequence_events()?
            .into_iter()
            .find(|event| matches!(event.operation, JournalOperation::Claim { .. }))
            .ok_or("claim fixture")?;
        let id = ReservationId::new();
        if let JournalOperation::Claim {
            reservation_id,
            authorization,
            ..
        } = &mut claim.operation
        {
            *reservation_id = id;
            **authorization = ConflictAuthorization::NoConflict;
        }
        let mut events = vec![claim.clone()];
        let mut checkpoint = claim.clone();
        checkpoint.event_id = EventId::new();
        checkpoint.operation = fixtures::operation(
            &serde_json::json!({"op":"checkpoint", "reservation_id":id, "protected_tip":"2222222222222222222222222222222222222222", "trunk_snapshot":"1111111111111111111111111111111111111111"}),
        )?;
        events.push(checkpoint);
        for at in [
            "2020-01-01T00:00:00.000Z".parse()?,
            crate::ids::RecordedAt::now(),
            crate::ids::RecordedAt::now(),
        ] {
            let mut observed = claim.clone();
            observed.at = at;
            observed.event_id = EventId::new();
            observed.operation = fixtures::operation(
                &serde_json::json!({"op":"merge_extent_observed", "reservation_id":id, "extent":empty_extent(), "run_status":"ended"}),
            )?;
            events.push(observed);
        }
        // The final record keeps this endpoint as the generation anchor, despite its age.
        for (index, event) in events.iter_mut().enumerate() {
            event.projection_generation = ProjectionGeneration::from(u64::try_from(index)? + 1);
        }
        let fixture = CompactionFixture::holding(events)?;
        let before = Journal::replay_read_only(&fixture.ledger.paths.journal)?;
        assert!(
            before
                .finished_history
                .has_expired_record(id, &crate::ids::RecordedAt::now())
        );
        assert!(matches!(
            fixture.ledger.compact_journal()?,
            JournalCompaction::Compacted { .. }
        ));
        let after = Journal::replay_read_only(&fixture.ledger.paths.journal)?;
        assert_eq!(before.finished_history, after.finished_history);
        assert_eq!(after.superseded.droppable_bytes, 0);
        assert!(after.record_count < before.record_count);
        Ok(())
    }

    #[test]
    fn compaction_preserves_every_replay_output() -> Result<(), Box<dyn Error>> {
        let repository = RepoInstanceId::new();
        let fold = journal::fold_sequence_events()?;
        let (round_trip, rejected) = fixtures::recent_round_trip_events()?;
        let base = &round_trip[..rejected];
        let mut journals = vec![accepted_prefix(&fold).to_vec(), base.to_vec()];
        for seed in GENERATED_SEEDS {
            journals.push(generated_journal(base, seed)?);
        }
        let mut compacted_journals = 0;

        for mut events in journals {
            fixtures::name_repository(&mut events, repository);
            let (bytes, _) = fixtures::journal_bytes(&events)?;
            let CompactionPlanning::Rewrite(plan) =
                CompactionPlan::of(&bytes, repository, &RecordedAt::now())?
            else {
                assert_eq!(fixtures::superseded_record_bytes(&events)?, 0);
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
    fn an_event_after_compaction_preparation_invalidates_the_rewrite() -> Result<(), Box<dyn Error>>
    {
        let fixture = CompactionFixture::round_trip()?;
        let super::CompactionPreparation::Ready(prepared) = fixture.ledger.prepare_compaction(
            fixture.repo_instance_id,
            replays_agree,
            &RecordedAt::now(),
        )?
        else {
            return Err("round-trip fixture must supersede observations".into());
        };
        let super::CompactionPreparation::Ready(overlapping) = fixture.ledger.prepare_compaction(
            fixture.repo_instance_id,
            replays_agree,
            &RecordedAt::now(),
        )?
        else {
            return Err("round-trip fixture must still supersede observations".into());
        };
        let events = journal::complete_record_events(&fixture.journal()?)?;
        let (renewal, _) = fixtures::renewal_and_observation(&events)?;
        let appended = fixtures::append(&fixture.ledger, &renewal)?;
        assert!(
            !prepared.still_current(&fixture.ledger.paths.journal)?,
            "a rewrite prepared before the append must not replace its journal"
        );
        drop(prepared);
        assert!(
            overlapping.checkpoint_path.is_file(),
            "discarding one preparation must leave the other's checkpoint intact"
        );
        drop(overlapping);
        fixture.ledger.compact_journal()?;
        assert!(
            journal::complete_record_events(&fixture.journal()?)?
                .iter()
                .any(|event| event.event_id == appended),
            "re-preparation must retain the intervening event"
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
        let (events, rejected) = fixtures::recent_round_trip_events()?;
        assert!(fixtures::superseded_record_bytes(&events)? > 0);
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
        fixtures::name_repository(&mut accepted, repository);
        let (bytes, _) = fixtures::journal_bytes(&accepted)?;
        let incomplete = [bytes.as_slice(), b"{\"schema_version\""].concat();
        assert!(matches!(
            CompactionPlan::of(&incomplete, repository, &RecordedAt::now()),
            Err(JournalCompactionError::Refused(
                CompactionRefusal::IncompleteFinalRecord
            ))
        ));
        assert!(matches!(
            CompactionPlan::of(&bytes, RepoInstanceId::new(), &RecordedAt::now()),
            Err(JournalCompactionError::Refused(
                CompactionRefusal::RepositoryValidation(_)
            ))
        ));
        Ok(())
    }

    #[test]
    fn superseded_bytes_equal_the_bytes_compaction_removes() -> Result<(), Box<dyn Error>> {
        let (events, rejected) = fixtures::recent_round_trip_events()?;
        let mut events = events[..rejected].to_vec();
        let repository = RepoInstanceId::new();
        fixtures::name_repository(&mut events, repository);
        let (bytes, record_ends) = fixtures::journal_bytes(&events)?;
        let removed_bytes = |journal: &[u8]| -> Result<u64, Box<dyn Error>> {
            let kept = match CompactionPlan::of(journal, repository, &RecordedAt::now())? {
                CompactionPlanning::Rewrite(plan) => plan.kept.len(),
                CompactionPlanning::Unchanged(_) => journal.len(),
            };
            Ok(u64::try_from(journal.len() - kept)?)
        };

        for (length, record_end) in record_ends.iter().enumerate() {
            assert_eq!(
                removed_bytes(&bytes[..*record_end])?,
                fixtures::superseded_record_bytes(&events[..length])?
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
    fn compaction_keeps_appends_that_arrive_after_its_prefix_was_verified()
    -> Result<(), Box<dyn Error>> {
        let (events, rejected) = fixtures::recent_round_trip_events()?;
        let (renewal, observation) = fixtures::renewal_and_observation(&events[..rejected])?;
        let fixture = CompactionFixture::holding(events[..rejected].to_vec())?;
        fixtures::append(&fixture.ledger, &observation)?;
        fixtures::append(&fixture.ledger, &observation)?;
        let super::CompactionPreparation::Ready(mut prepared) = fixture.ledger.prepare_compaction(
            fixture.repo_instance_id,
            replays_agree,
            &RecordedAt::now(),
        )?
        else {
            return Err("superseded observations must prepare a rewrite".into());
        };
        let appended = (0..RACING_RENEWALS)
            .map(|_| fixtures::append(&fixture.ledger, &renewal))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let lock = MutationLock::acquire(
            &fixture.ledger.paths.lock,
            MUTATING_VERB_CONTENTION_TOLERANCE,
        )?;
        assert!(prepared.include_committed_tail(&fixture.ledger.paths.journal)?);
        fixture.ledger.publish_compacted(
            &prepared,
            OriginalJournalLock::acquire(&fixture.ledger.paths.journal)?,
            fixture.repo_instance_id,
            &mut |_| {},
        )?;
        drop(lock);
        let survivors = journal::complete_record_events(&fixture.journal()?)?
            .into_iter()
            .map(|event| event.event_id)
            .collect::<BTreeSet<_>>();
        assert!(appended.is_subset(&survivors));
        fixture.ledger.read_validated_journal()?;
        let checkpoint = ReplayCheckpoint::new(
            &fixture.ledger.paths.replay_checkpoint,
            fixture.repo_instance_id,
        );
        let resumed =
            Journal::replay_read_only_from_checkpoint(&fixture.ledger.paths.journal, &checkpoint)?;
        let whole = Journal::replay_read_only(&fixture.ledger.paths.journal)?;
        assert!(replays_agree(&whole, &resumed));
        assert_eq!(whole.end_offset, resumed.end_offset);
        Ok(())
    }

    #[test]
    fn automatic_compaction_includes_an_append_during_verification() -> Result<(), Box<dyn Error>> {
        thread_local! {
            static APPEND: std::cell::RefCell<(std::path::PathBuf, Vec<u8>)> =
                const { std::cell::RefCell::new((std::path::PathBuf::new(), Vec::new())) };
        }
        fn agree_and_append(original: &JournalReplay, compacted: &JournalReplay) -> bool {
            APPEND.with(|append| {
                let append = append.borrow();
                let result = fs::OpenOptions::new()
                    .append(true)
                    .open(&append.0)
                    .and_then(|mut file| file.write_all(&append.1));
                assert!(result.is_ok(), "racing append: {result:?}");
            });
            replays_agree(original, compacted)
        }
        let (events, rejected) = fixtures::recent_round_trip_events()?;
        let (renewal, observation) = fixtures::renewal_and_observation(&events[..rejected])?;
        let mut fixture = CompactionFixture::holding(events[..rejected].to_vec())?;
        fixtures::append(&fixture.ledger, &observation)?;
        fixtures::append(&fixture.ledger, &observation)?;
        let mut event = events[0].clone();
        event.actor.repository = fixture.repo_instance_id;
        event.event_id = EventId::new();
        event.operation = renewal;
        let replay = Journal::replay_read_only(&fixture.ledger.paths.journal)?;
        event.projection_generation = ProjectionGeneration::from(u64::from(replay.generation) + 1);
        let mut bytes = serde_json::to_vec(&event)?;
        bytes.push(b'\n');
        APPEND.with(|append| *append.borrow_mut() = (fixture.ledger.paths.journal.clone(), bytes));
        fixture.ledger.compaction_threshold_bytes = 1;
        let result =
            fixture
                .ledger
                .prepare_due_compaction(agree_and_append, None, &RecordedAt::now())?;
        assert!(
            matches!(result, Some(JournalCompaction::Compacted { .. })),
            "append must not starve automatic compaction: {result:?}"
        );
        let replay = Journal::replay_read_only(&fixture.ledger.paths.journal)?;
        assert!(replay.reservations.is_ok());
        assert!(
            journal::complete_record_events(&fixture.journal()?)?
                .iter()
                .any(|kept| kept.event_id == event.event_id)
        );
        Ok(())
    }

    #[test]
    fn appends_racing_compaction_all_survive() -> Result<(), Box<dyn Error>> {
        let (events, rejected) = fixtures::recent_round_trip_events()?;
        let (renewal, observation) = fixtures::renewal_and_observation(&events[..rejected])?;
        let fixture = Arc::new(CompactionFixture::holding(events[..rejected].to_vec())?);
        let start = Arc::new(Barrier::new(2));
        let renewals = {
            let fixture = Arc::clone(&fixture);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                (0..RACING_RENEWALS)
                    .map(|_| fixtures::append(&fixture.ledger, &renewal))
                    .collect::<Result<Vec<_>, _>>()
            })
        };
        start.wait();
        let mut observations = Vec::new();
        for _ in 0..RACING_COMPACTIONS {
            observations.push(fixtures::append(&fixture.ledger, &observation)?);
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
        let checkpoint = ReplayCheckpoint::new(&paths.replay_checkpoint, fixture.repo_instance_id);
        let (deferred, _) = checkpoint
            .deferred_edit_start(&paths.journal)?
            .ok_or("compaction must checkpoint the replacement journal")?;
        let checkpointed = deferred.complete().ok_or("checkpoint parts must decode")?;
        let journal = fs::metadata(&paths.journal)?;

        assert_eq!(header["journal"]["device"], journal.dev());
        assert_eq!(header["journal"]["inode"], journal.ino());
        assert_eq!(header["end_offset"], journal.len());
        assert_eq!(checkpointed, Journal::replay_read_only(&paths.journal)?);
        Ok(())
    }

    #[test]
    fn checkpoint_writers_cannot_use_the_original_journal_during_compaction()
    -> Result<(), Box<dyn Error>> {
        let fixture = CompactionFixture::round_trip()?;
        let original = fs::File::open(&fixture.ledger.paths.journal)?;
        let mut observed = Vec::new();
        let result = fixture.ledger.compact_journal_observing(|boundary| {
            observed.push(boundary);
            assert!(
                matches!(original.try_lock(), Err(fs::TryLockError::WouldBlock)),
                "the original journal must exclude checkpoint writers at {boundary:?}"
            );
        })?;
        assert!(matches!(result, JournalCompaction::Compacted { .. }));
        assert_eq!(observed.len(), 3);
        original.try_lock()?;
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
        let (events, rejected) = fixtures::recent_round_trip_events()?;
        let (renewal, observation) = fixtures::renewal_and_observation(&events[..rejected])?;
        let mut fixture = CompactionFixture::holding(events[..rejected].to_vec())?;
        let droppable = fixtures::superseded_bytes(&fixture.ledger)?;
        fixture.ledger.compaction_threshold_bytes = droppable + 1;
        let inode = fs::metadata(&fixture.ledger.paths.journal)?.ino();

        let renewed = fixtures::append(&fixture.ledger, &renewal)?;
        assert_eq!(fs::metadata(&fixture.ledger.paths.journal)?.ino(), inode);
        assert_eq!(fixtures::superseded_bytes(&fixture.ledger)?, droppable);

        let observed = fixtures::append(&fixture.ledger, &observation)?;
        let survivors = journal::complete_record_events(&fixture.journal()?)?
            .into_iter()
            .map(|event| event.event_id)
            .collect::<BTreeSet<_>>();
        assert_ne!(fs::metadata(&fixture.ledger.paths.journal)?.ino(), inode);
        assert_eq!(fixtures::superseded_bytes(&fixture.ledger)?, 0);
        assert!(survivors.contains(&renewed) && survivors.contains(&observed));
        fixture.ledger.read_validated_journal()?;
        Ok(())
    }

    #[test]
    fn a_ref_hook_defers_due_compaction_to_the_next_ordinary_mutation() -> Result<(), Box<dyn Error>>
    {
        let (events, rejected) = fixtures::recent_round_trip_events()?;
        let (renewal, observation) = fixtures::renewal_and_observation(&events[..rejected])?;
        let mut fixture = CompactionFixture::holding(events[..rejected].to_vec())?;
        let threshold = fixtures::superseded_bytes(&fixture.ledger)? + 1;
        fixture.ledger.compaction_threshold_bytes = threshold;
        let context = WorktreeContext::discover(fixture.repository.path())?;
        let mut hook = Ledger::open_for_reference_transaction(&context)?;
        hook.compaction_threshold_bytes = threshold;
        let original = fixture.journal()?;
        let inode = fs::metadata(&hook.paths.journal)?.ino();

        let outcome = hook.transact_reconciliation(
            WorktreeId::new(),
            CoordinationRunId::new(),
            |_| ReconciliationValidation::<(), ()>::Apply {
                operations:             vec![observation.clone()],
                cover_actors:           std::collections::HashMap::new(),
                recoverable_operations: Vec::new(),
                action:                 (),
            },
            |(), _, _| Ok::<(), std::io::Error>(()),
        )?;
        assert!(matches!(
            outcome,
            LedgerReconciliationOutcome::Appended { .. }
        ));
        let appended = fixture.journal()?;
        assert!(appended.starts_with(&original) && appended.len() > original.len());
        assert_eq!(fs::metadata(&hook.paths.journal)?.ino(), inode);
        assert!(fixtures::superseded_bytes(&hook)? >= threshold);

        let renewed = fixtures::append(&fixture.ledger, &renewal)?;
        assert_ne!(fs::metadata(&hook.paths.journal)?.ino(), inode);
        assert_eq!(fixtures::superseded_bytes(&fixture.ledger)?, 0);
        assert!(
            journal::complete_record_events(&fixture.journal()?)?
                .iter()
                .any(|event| event.event_id == renewed)
        );
        fixture.ledger.read_validated_journal()?;
        Ok(())
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
                (true, true) => fixtures::legacy_extent(first)?,
                (true, false) => fixtures::holder_extent(&[first])?,
                (false, _) => fixtures::holder_extent(&[first, second])?,
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
