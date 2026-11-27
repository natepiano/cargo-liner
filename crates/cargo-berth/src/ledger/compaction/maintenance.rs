//! Periodic journal compaction and temporary file cleanup.

use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::fs::FileTimes;
use std::fs::Metadata;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::journal_compaction::CompactionPreparation;
use super::journal_compaction::JournalCompaction;
use super::journal_compaction::JournalCompactionError;
use super::journal_compaction::OriginalJournalLock;
use super::journal_compaction::OriginalJournalLockAttempt;
use super::journal_compaction::replays_agree;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ledger::constants::HISTORY_COMPACTION_CHECK_FILE_NAME;
use crate::ledger::constants::HISTORY_COMPACTION_CHECK_INTERVAL;
use crate::ledger::constants::JOURNAL_COMPACTION_TEMPORARY_FILE_NAME;
use crate::ledger::constants::JOURNAL_COMPACTION_TEMPORARY_GRACE;
use crate::ledger::constants::REPLAY_CHECKPOINT_FILE_NAME;
use crate::ledger::handle::Ledger;
use crate::ledger::history::FinishedHistory;
use crate::ledger::history::HistoryCompactionBasis;
use crate::ledger::identity;
use crate::ledger::journal::JournalReplay;
use crate::ledger::lock::MutationLock;
use crate::ledger::lock::MutationLockError;

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

/// The journal and candidates that produced a history plan with no removable records.
#[derive(Debug, Deserialize, Serialize)]
pub(super) struct HistoryCompactionNoProgress {
    device:     u64,
    inode:      u64,
    end_offset: u64,
    modified:   SystemTime,
    basis:      HistoryCompactionBasis,
}

impl HistoryCompactionNoProgress {
    /// Store a plan only for the complete file it inspected.
    pub(super) fn for_source(
        before: &Metadata,
        after: &Metadata,
        end_offset: u64,
        basis: HistoryCompactionBasis,
    ) -> Option<Self> {
        let modified = before.modified().ok()?;
        (before.len() == end_offset
            && after.len() == end_offset
            && after.modified().ok()? == modified)
            .then_some(Self {
                device: before.dev(),
                inode: before.ino(),
                end_offset,
                modified,
                basis,
            })
    }

    /// The bytes appended after the plan, if the journal and candidates still match it.
    fn covered_growth(
        &self,
        journal: &Metadata,
        candidates: &BTreeSet<ReservationId>,
        threshold: u64,
    ) -> Option<u64> {
        let growth = journal.len().checked_sub(self.end_offset)?;
        (self.device == journal.dev()
            && self.inode == journal.ino()
            && candidates.is_subset(&self.basis.candidates)
            && growth < threshold
            && (growth != 0 || journal.modified().ok()? == self.modified))
            .then_some(growth)
    }

    /// Reuse the plan unless an append can remove a root or create a superseded record.
    fn still_applies(
        &self,
        path: &Path,
        replay: &JournalReplay,
        candidates: &BTreeSet<ReservationId>,
        threshold: u64,
    ) -> Option<bool> {
        if replay.superseded.droppable_bytes != 0 {
            return Some(false);
        }
        let mut journal = File::open(path).ok()?;
        let metadata = journal.metadata().ok()?;
        if metadata.len() != u64::from(replay.end_offset) {
            return Some(false);
        }
        let growth = self.covered_growth(&metadata, candidates, threshold)?;
        if growth == 0 {
            return Some(true);
        }
        journal.seek(SeekFrom::Start(self.end_offset)).ok()?;
        let mut appended = Vec::new();
        journal.take(growth).read_to_end(&mut appended).ok()?;
        Some(
            u64::try_from(appended.len()).ok()? == growth
                && appended.ends_with(b"\n")
                && self.basis.appends_keep_roots(&appended, replay, candidates),
        )
    }
}

/// A temporary file that a compaction or a replay checkpoint write leaves in the ledger directory.
enum TemporaryFile {
    /// A compacted journal, `journal.ndjson.compact.tmp.<uuid>`, which its writer locks.
    CompactedJournal,
    /// The checkpoint or a checkpoint write of a compacted journal. The writer locks the
    /// compacted journal named `owner`, not this file.
    CompactionSidecar { owner: String },
    /// A replay checkpoint write, `replay-checkpoint.json.<uuid>.tmp`, which its writer locks.
    CheckpointWrite,
}

/// A ledger directory entry that is not a [`TemporaryFile`].
struct OtherFile;

impl TemporaryFile {
    /// The file whose lock shows that a writer still uses the temporary file at `path` in
    /// `directory`.
    fn owner(&self, path: &Path, directory: &Path) -> PathBuf {
        match self {
            Self::CompactedJournal | Self::CheckpointWrite => path.to_path_buf(),
            Self::CompactionSidecar { owner } => directory.join(owner),
        }
    }
}

impl TryFrom<&str> for TemporaryFile {
    type Error = OtherFile;

    /// The kind of the temporary file named `name`.
    fn try_from(name: &str) -> Result<Self, OtherFile> {
        let is_uuid = |id: &str| Uuid::parse_str(id).is_ok();
        if let Some(suffix) = name
            .strip_prefix(JOURNAL_COMPACTION_TEMPORARY_FILE_NAME)
            .and_then(|rest| rest.strip_prefix('.'))
        {
            let (journal_id, sidecar) = suffix.split_once('.').unwrap_or((suffix, ""));
            if !is_uuid(journal_id) {
                return Err(OtherFile);
            }
            let owner = format!("{JOURNAL_COMPACTION_TEMPORARY_FILE_NAME}.{journal_id}");
            return match sidecar {
                "" => Ok(Self::CompactedJournal),
                "checkpoint" => Ok(Self::CompactionSidecar { owner }),
                write => write
                    .strip_prefix("checkpoint.")
                    .and_then(|write| write.strip_suffix(".tmp"))
                    .filter(|write_id| is_uuid(write_id))
                    .map(|_| Self::CompactionSidecar { owner })
                    .ok_or(OtherFile),
            };
        }
        name.strip_prefix(REPLAY_CHECKPOINT_FILE_NAME)
            .and_then(|rest| rest.strip_prefix('.'))
            .and_then(|rest| rest.strip_suffix(".tmp"))
            .filter(|write_id| is_uuid(write_id))
            .map(|_| Self::CheckpointWrite)
            .ok_or(OtherFile)
    }
}

impl Ledger {
    /// Check for abandoned files and compact the journal when the transaction found work.
    pub(in crate::ledger) fn maintain_compaction(&self, replay: &JournalReplay) {
        self.maintain_compaction_with_clock(replay, RecordedAt::now);
    }

    fn maintain_compaction_with_clock(
        &self,
        replay: &JournalReplay,
        now: impl FnOnce() -> RecordedAt,
    ) {
        let mut check = self.begin_compaction_check();
        if check.is_some() {
            std::mem::drop(self.remove_abandoned_compaction_files());
        }

        let droppable_bytes = replay.superseded.droppable_bytes;
        if check.is_none() && droppable_bytes < self.compaction_threshold_bytes {
            return;
        }
        let now = now();
        let mut candidates =
            FinishedHistory::compaction_candidates(replay, &now, &self.paths.repository_root);
        if droppable_bytes >= self.compaction_threshold_bytes {
            if check.is_some() || candidates.next().is_none() {
                std::mem::drop(self.attempt_due_compaction(
                    droppable_bytes,
                    replays_agree,
                    None,
                    &now,
                ));
            }
        } else if let Some(check) = &mut check {
            let candidates: BTreeSet<_> = candidates.collect();
            if !candidates.is_empty() {
                self.compact_expired_history(check, replay, &candidates, &now);
            }
        }
    }

    /// Lock the periodic maintenance marker without waiting. Stamp it before expensive work.
    fn begin_compaction_check(&self) -> Option<File> {
        let checked = self
            .paths
            .directory
            .join(HISTORY_COMPACTION_CHECK_FILE_NAME);
        if fs::metadata(&checked).is_ok_and(|metadata| recently_checked(&metadata)) {
            return None;
        }
        let mut marker = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(checked)
            .ok()?;
        marker.try_lock().ok()?;
        let metadata = marker.metadata().ok()?;
        if recently_checked(&metadata) {
            return None;
        }

        if metadata.len() == 0 {
            marker.write_all(b"null").ok()?;
            marker.seek(SeekFrom::Start(0)).ok()?;
        } else {
            marker
                .set_times(FileTimes::new().set_modified(SystemTime::now()))
                .ok()?;
        }
        Some(marker)
    }

    /// Use the saved plan if the journal tail and candidates keep its retention roots.
    fn compact_expired_history(
        &self,
        marker: &mut File,
        replay: &JournalReplay,
        candidates: &BTreeSet<ReservationId>,
        now: &RecordedAt,
    ) {
        if serde_json::from_reader::<_, HistoryCompactionNoProgress>(&mut *marker).is_ok_and(
            |plan| {
                plan.still_applies(
                    &self.paths.journal,
                    replay,
                    candidates,
                    self.compaction_threshold_bytes,
                ) == Some(true)
            },
        ) {
            return;
        }
        std::mem::drop(self.attempt_due_compaction(
            self.compaction_threshold_bytes,
            replays_agree,
            Some(marker),
            now,
        ));
    }

    /// Try a due compaction without waiting for the mutation lock.
    fn attempt_due_compaction(
        &self,
        droppable_bytes: u64,
        agree: fn(&JournalReplay, &JournalReplay) -> bool,
        marker: Option<&mut File>,
        now: &RecordedAt,
    ) -> Result<Option<JournalCompaction>, JournalCompactionError> {
        if droppable_bytes < self.compaction_threshold_bytes || self.compaction_held_back() {
            return Ok(None);
        }
        let compaction = self.prepare_due_compaction(agree, marker, now);
        if let Err(error) = &compaction {
            std::mem::drop(self.record_refusal(error));
        }
        compaction
    }

    /// Build and verify the rewrite, then publish it only when the mutation lock and the original
    /// journal lock are free at once. A held lock gives up this attempt without an error.
    pub(super) fn prepare_due_compaction(
        &self,
        agree: fn(&JournalReplay, &JournalReplay) -> bool,
        marker: Option<&mut File>,
        now: &RecordedAt,
    ) -> Result<Option<JournalCompaction>, JournalCompactionError> {
        let repo_instance_id = identity::read_repo_instance_id(&self.paths.repo_instance_id)?;
        let mut prepared = match self.prepare_compaction(repo_instance_id, agree, now)? {
            CompactionPreparation::Ready(prepared) => prepared,
            CompactionPreparation::Unchanged(plan) => {
                if let (Some(marker), Some(plan)) = (marker, plan) {
                    std::mem::drop(write_history_compaction_plan(marker, &plan));
                }
                return Ok(Some(JournalCompaction::NothingToCompact));
            },
        };
        let lock = match MutationLock::acquire(&self.paths.lock, Duration::ZERO) {
            Ok(lock) => lock,
            Err(MutationLockError::AcquisitionTimedOut) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !prepared.include_committed_tail(&self.paths.journal)? {
            return Ok(None);
        }
        let OriginalJournalLockAttempt::Acquired(original_journal) =
            OriginalJournalLock::try_acquire(&self.paths.journal)?
        else {
            return Ok(None);
        };
        let result =
            self.publish_compacted(&prepared, original_journal, repo_instance_id, &mut |_| {});
        std::mem::drop(lock);
        result.map(|()| Some(prepared.result))
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

    /// Reclaim old temporary files whose writers released their locks.
    pub(super) fn remove_abandoned_compaction_files(&self) -> std::io::Result<()> {
        for entry in fs::read_dir(&self.paths.directory)?.flatten() {
            let Some(Ok(temporary_file)) = entry.file_name().to_str().map(TemporaryFile::try_from)
            else {
                continue;
            };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file()
                || !metadata
                    .modified()
                    .and_then(|time| time.elapsed().map_err(std::io::Error::other))
                    .is_ok_and(|age| age >= JOURNAL_COMPACTION_TEMPORARY_GRACE)
            {
                continue;
            }
            let path = entry.path();
            let _lock =
                match open_temporary_owner(&temporary_file.owner(&path, &self.paths.directory)) {
                    Ok(file) if file.try_lock().is_ok() => Some(file),
                    Err(error) if error.kind() == ErrorKind::NotFound => None,
                    _ => continue,
                };
            std::mem::drop(remove_if_exists(&path));
            if matches!(temporary_file, TemporaryFile::CompactedJournal) {
                std::mem::drop(remove_if_exists(&path.with_added_extension("checkpoint")));
            }
        }
        Ok(())
    }
}

/// Whether the last maintenance check still prevents a new check.
fn recently_checked(metadata: &Metadata) -> bool {
    metadata.len() != 0
        && metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|elapsed| elapsed < HISTORY_COMPACTION_CHECK_INTERVAL)
}

fn write_history_compaction_plan(
    marker: &mut File,
    plan: &HistoryCompactionNoProgress,
) -> std::io::Result<()> {
    let contents = serde_json::to_vec(plan)?;
    marker.seek(SeekFrom::Start(0))?;
    marker.write_all(&contents)?;
    marker.set_len(u64::try_from(contents.len()).map_err(std::io::Error::other)?)
}

fn open_temporary_owner(path: &Path) -> std::io::Result<File> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "temporary owner is not a file",
        ));
    }
    File::open(path)
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Delete the refusal marker so the next due transaction retries compaction.
pub(in crate::ledger) fn remove_refusal(path: &Path) -> std::io::Result<()> {
    remove_if_exists(path)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::fs::File;
    use std::fs::FileTimes;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use std::time::Instant;
    use std::time::SystemTime;

    use uuid::Uuid;

    use super::CompactionRefusalMarker;
    use super::replays_agree;
    use crate::ids::ProjectionGeneration;
    use crate::ids::RecordedAt;
    use crate::ledger::compaction::fixtures;
    use crate::ledger::compaction::fixtures::CompactionFixture;
    use crate::ledger::compaction::journal_compaction::CompactionRefusal;
    use crate::ledger::compaction::journal_compaction::JournalCompaction;
    use crate::ledger::compaction::journal_compaction::JournalCompactionError;
    use crate::ledger::constants::HISTORY_COMPACTION_CHECK_FILE_NAME;
    use crate::ledger::constants::HISTORY_COMPACTION_CHECK_INTERVAL;
    use crate::ledger::constants::JOURNAL_COMPACTION_TEMPORARY_FILE_NAME;
    use crate::ledger::constants::JOURNAL_COMPACTION_TEMPORARY_GRACE;
    use crate::ledger::constants::MUTATING_VERB_CONTENTION_TOLERANCE;
    use crate::ledger::constants::REPLAY_CHECKPOINT_FILE_NAME;
    use crate::ledger::handle::Ledger;
    use crate::ledger::history::FinishedHistory;
    use crate::ledger::journal::Journal;
    use crate::ledger::journal::JournalReplay;
    use crate::ledger::lock::MutationLock;

    /// How long the test's original journal lock holder waits for its release. A compaction that
    /// waited for that lock would take this long.
    const JOURNAL_LOCK_HOLD_LIMIT: Duration = Duration::from_secs(5);

    #[test]
    fn history_compaction_throttles_an_unpublished_attempt() -> Result<(), Box<dyn Error>> {
        let (events, _, _) = fixtures::finished_history_events()?;
        let fixture = CompactionFixture::holding(events)?;
        let ledger = &fixture.ledger;
        let original = fixture.journal()?;
        let replay = Journal::replay_read_only(&ledger.paths.journal)?;
        assert!(
            FinishedHistory::compaction_candidates(
                &replay,
                &RecordedAt::now(),
                &ledger.paths.repository_root
            )
            .next()
            .is_some()
        );
        assert_eq!(replay.superseded.droppable_bytes, 0);

        let holder = MutationLock::acquire(&ledger.paths.lock, Duration::ZERO)?;
        ledger.maintain_compaction(&replay);
        drop(holder);

        let marker = ledger
            .paths
            .directory
            .join(HISTORY_COMPACTION_CHECK_FILE_NAME);
        assert!(
            marker.is_file(),
            "a failed publication must start the cooldown"
        );
        ledger.maintain_compaction(&replay);
        assert_eq!(
            fixture.journal()?,
            original,
            "a recent attempt must prevent a retry"
        );

        age_file(&marker, HISTORY_COMPACTION_CHECK_INTERVAL)?;
        ledger.maintain_compaction(&replay);
        assert!(
            fixture.journal()?.len() < original.len(),
            "history must compact after the cooldown"
        );
        Ok(())
    }

    #[test]
    fn combined_compaction_obeys_the_marker_lock_and_cooldown() -> Result<(), Box<dyn Error>> {
        let (mut history, _, _) = fixtures::finished_history_events()?;
        let (events, rejected) = fixtures::recent_round_trip_events()?;
        history.truncate(2);
        history.extend_from_slice(&events[..rejected]);
        for (generation, event) in (1_u64..).zip(&mut history) {
            event.projection_generation = ProjectionGeneration::from(generation);
        }
        let mut fixture = CompactionFixture::holding(history)?;
        fixture.ledger.compaction_threshold_bytes = 1;
        let ledger = &fixture.ledger;
        let original = fixture.journal()?;
        let replay = Journal::replay_read_only(&ledger.paths.journal)?;
        assert!(
            FinishedHistory::compaction_candidates(
                &replay,
                &RecordedAt::now(),
                &ledger.paths.repository_root
            )
            .next()
            .is_some()
        );
        assert!(replay.superseded.droppable_bytes >= ledger.compaction_threshold_bytes);

        let holder = ledger
            .begin_compaction_check()
            .ok_or("the marker should be available")?;
        let marker = ledger
            .paths
            .directory
            .join(HISTORY_COMPACTION_CHECK_FILE_NAME);
        age_file(&marker, HISTORY_COMPACTION_CHECK_INTERVAL)?;
        ledger.maintain_compaction(&replay);
        assert_eq!(
            fixture.journal()?,
            original,
            "the held marker must prevent compaction"
        );
        drop(holder);

        File::options()
            .write(true)
            .open(&marker)?
            .set_times(FileTimes::new().set_modified(SystemTime::now()))?;
        ledger.maintain_compaction(&replay);
        assert_eq!(
            fixture.journal()?,
            original,
            "the cooldown must prevent compaction"
        );

        age_file(&marker, HISTORY_COMPACTION_CHECK_INTERVAL)?;
        ledger.maintain_compaction(&replay);
        assert!(
            fixture.journal()?.len() < original.len(),
            "both triggers must compact after the cooldown"
        );
        Ok(())
    }

    #[test]
    fn compaction_retries_only_when_history_conditions_change() -> Result<(), Box<dyn Error>> {
        let (mut events, _, _) = fixtures::finished_history_events()?;
        events.pop().ok_or("the fixture has a live claim")?;
        let fixture = CompactionFixture::holding(events)?;
        let ledger = &fixture.ledger;
        let original = fixture.journal()?;
        let replay = Journal::replay_read_only(&ledger.paths.journal)?;
        ledger.maintain_compaction(&replay);
        assert_eq!(fixture.journal()?, original);

        let marker = ledger
            .paths
            .directory
            .join(HISTORY_COMPACTION_CHECK_FILE_NAME);
        let mut saved: serde_json::Value = serde_json::from_slice(&fs::read(&marker)?)?;
        assert!(saved.is_object());
        if let Some(planned_at) = saved.get_mut("planned_at") {
            *planned_at = serde_json::to_value(SystemTime::UNIX_EPOCH)?;
        }
        saved["test_sentinel"] = serde_json::Value::Bool(true);
        let saved = serde_json::to_vec(&saved)?;
        fs::write(&marker, &saved)?;
        age_file(&marker, HISTORY_COMPACTION_CHECK_INTERVAL)?;
        ledger.maintain_compaction(&replay);
        assert_eq!(fs::read(&marker)?, saved, "skip the unchanged plan");

        assert_eq!(fixture.journal()?, original);
        Ok(())
    }

    #[test]
    fn a_small_append_can_reuse_the_history_plan() -> Result<(), Box<dyn Error>> {
        let (mut events, expired, _) = fixtures::finished_history_events()?;
        events.pop().ok_or("the fixture has a live claim")?;
        let fixture = CompactionFixture::holding(events)?;
        let ledger = &fixture.ledger;
        let replay = Journal::replay_read_only(&ledger.paths.journal)?;
        ledger.maintain_compaction(&replay);

        let marker = ledger
            .paths
            .directory
            .join(HISTORY_COMPACTION_CHECK_FILE_NAME);
        let mut saved: serde_json::Value = serde_json::from_slice(&fs::read(&marker)?)?;
        assert!(saved.is_object());
        saved["test_sentinel"] = serde_json::Value::Bool(true);
        let saved = serde_json::to_vec(&saved)?;
        fs::write(&marker, &saved)?;
        age_file(&marker, HISTORY_COMPACTION_CHECK_INTERVAL)?;

        let operation = crate::ledger::journal::JournalOperation::Renew {
            reservation_id: expired,
        };
        fixtures::append(ledger, &operation)?;
        assert_eq!(
            fs::read(&marker)?,
            saved,
            "reuse the plan while the final record still keeps the same reservation"
        );
        Ok(())
    }

    #[test]
    fn compaction_retries_when_the_final_record_releases_history() -> Result<(), Box<dyn Error>> {
        let (mut events, _, _) = fixtures::finished_history_events()?;
        let live_claim = events.pop().ok_or("the fixture has a live claim")?;
        let fixture = CompactionFixture::holding(events)?;
        let ledger = &fixture.ledger;
        let inode = fs::metadata(&ledger.paths.journal)?.ino();
        let replay = Journal::replay_read_only(&ledger.paths.journal)?;
        ledger.maintain_compaction(&replay);
        assert_eq!(fs::metadata(&ledger.paths.journal)?.ino(), inode);

        let marker = ledger
            .paths
            .directory
            .join(HISTORY_COMPACTION_CHECK_FILE_NAME);
        age_file(&marker, HISTORY_COMPACTION_CHECK_INTERVAL)?;
        fixtures::append(ledger, &live_claim.operation)?;
        assert_ne!(fs::metadata(&ledger.paths.journal)?.ino(), inode);
        Ok(())
    }

    #[test]
    fn a_history_plan_needs_a_new_scan_when_another_reservation_expires()
    -> Result<(), Box<dyn Error>> {
        let (mut older, anchored, _) = fixtures::finished_history_events()?;
        older.pop().ok_or("expected a live claim")?;
        let (mut events, expires, _) = fixtures::finished_history_events()?;
        events.pop().ok_or("expected a live claim")?;
        for event in &mut events {
            event.at = "2020-01-02T00:00:00.000Z".parse()?;
        }
        events.extend(older);
        for (index, event) in events.iter_mut().enumerate() {
            event.projection_generation = ProjectionGeneration::from(u64::try_from(index)? + 1);
        }

        let fixture = CompactionFixture::holding(events)?;
        let ledger = &fixture.ledger;
        let marker = ledger
            .paths
            .directory
            .join(HISTORY_COMPACTION_CHECK_FILE_NAME);
        let replay = Journal::replay_read_only(&ledger.paths.journal)?;
        let original = fixture.journal()?;
        let before_expiry = "2020-01-03T00:00:00.000Z".parse()?;
        ledger.maintain_compaction_with_clock(&replay, || before_expiry);
        assert_eq!(fixture.journal()?, original);
        assert!(
            fs::read(&marker)?.starts_with(b"{"),
            "save the unchanged plan"
        );

        age_file(&marker, HISTORY_COMPACTION_CHECK_INTERVAL)?;
        let after_expiry = "2020-01-03T00:00:00.001Z".parse()?;
        ledger.maintain_compaction_with_clock(&replay, || after_expiry);
        assert!(fixture.journal()?.len() < original.len());
        let compacted = Journal::replay_read_only(&ledger.paths.journal)?
            .reservations
            .map_err(|error| error.to_string())?;
        assert!(compacted.reservation(anchored).is_ok());
        assert!(compacted.reservation(expires).is_err());
        Ok(())
    }

    #[test]
    fn maintenance_reclaims_abandoned_temporary_files() -> Result<(), Box<dyn Error>> {
        assert_temporary_cleanup(|ledger| {
            let replay = Journal::replay_read_only(&ledger.paths.journal)?;
            ledger.maintain_compaction(&replay);
            Ok(())
        })
    }

    #[test]
    fn manual_compaction_reclaims_abandoned_temporary_files() -> Result<(), Box<dyn Error>> {
        assert_temporary_cleanup(|ledger| {
            assert!(matches!(
                ledger.compact_journal()?,
                JournalCompaction::Compacted { .. }
            ));
            Ok(())
        })
    }

    fn assert_temporary_cleanup(
        cleanup: impl FnOnce(&Ledger) -> Result<(), Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let fixture = CompactionFixture::round_trip()?;
        let directory = &fixture.ledger.paths.directory;
        let temporary_journal = || {
            directory.join(format!(
                "{JOURNAL_COMPACTION_TEMPORARY_FILE_NAME}.{}",
                Uuid::now_v7()
            ))
        };
        let temporary_checkpoint = || {
            directory.join(format!(
                "{REPLAY_CHECKPOINT_FILE_NAME}.{}.tmp",
                Uuid::now_v7()
            ))
        };
        let stale = temporary_journal();
        let sidecar = stale.with_added_extension("checkpoint");
        let orphaned =
            temporary_journal().with_added_extension(format!("checkpoint.{}.tmp", Uuid::now_v7()));
        let active = temporary_journal();
        let active_sidecar =
            active.with_added_extension(format!("checkpoint.{}.tmp", Uuid::now_v7()));
        let recent = temporary_journal();
        let bad_owner = temporary_journal();
        let bad_sidecar = bad_owner.with_added_extension("checkpoint");
        let stale_checkpoint = temporary_checkpoint();
        let live_checkpoint = temporary_checkpoint();
        let recent_checkpoint = temporary_checkpoint();
        fs::create_dir(&bad_owner)?;
        for path in [
            &stale,
            &sidecar,
            &orphaned,
            &active,
            &active_sidecar,
            &bad_sidecar,
            &stale_checkpoint,
            &live_checkpoint,
        ] {
            fs::write(path, b"temporary")?;
            age_file(path, JOURNAL_COMPACTION_TEMPORARY_GRACE)?;
        }
        fs::write(&recent, b"recent journal")?;
        fs::write(&recent_checkpoint, b"recent checkpoint")?;
        let journal_writer = File::open(&active)?;
        let checkpoint_writer = File::open(&live_checkpoint)?;
        journal_writer.try_lock()?;
        checkpoint_writer.try_lock()?;

        cleanup(&fixture.ledger)?;
        assert!(!stale.exists(), "remove the abandoned journal");
        assert!(
            !sidecar.exists(),
            "remove the abandoned journal's checkpoint"
        );
        assert!(
            !orphaned.exists(),
            "remove the abandoned checkpoint write without a journal"
        );
        assert!(active.exists(), "keep the locked temporary journal");
        assert!(
            active_sidecar.exists(),
            "keep the locked journal's checkpoint write"
        );
        assert!(recent.exists(), "keep the recent temporary journal");
        assert!(bad_owner.is_dir(), "keep the unexpected owner directory");
        assert!(
            bad_sidecar.exists(),
            "skip the sidecar with the unexpected owner"
        );
        assert!(
            !stale_checkpoint.exists(),
            "remove the abandoned replay checkpoint write"
        );
        assert!(
            live_checkpoint.exists(),
            "keep the locked replay checkpoint write"
        );
        assert!(
            recent_checkpoint.exists(),
            "keep the recent replay checkpoint write"
        );
        Ok(())
    }

    fn age_file(path: &Path, age: Duration) -> std::io::Result<()> {
        File::options().write(true).open(path)?.set_times(
            FileTimes::new().set_modified(SystemTime::now() - age - Duration::from_secs(1)),
        )
    }

    #[test]
    fn a_due_compaction_skips_a_held_lock_without_waiting() -> Result<(), Box<dyn Error>> {
        let mut fixture = CompactionFixture::round_trip()?;
        let droppable = fixtures::superseded_bytes(&fixture.ledger)?;
        fixture.ledger.compaction_threshold_bytes = droppable;
        let original = fixture.journal()?;
        let _holder = MutationLock::acquire(
            &fixture.ledger.paths.lock,
            MUTATING_VERB_CONTENTION_TOLERANCE,
        )?;

        let started_at = Instant::now();
        let compaction = fixture.ledger.attempt_due_compaction(
            droppable,
            replays_agree,
            None,
            &RecordedAt::now(),
        )?;
        // A compaction that waited for the lock would spend the whole contention tolerance.
        assert!(started_at.elapsed() < Duration::from_secs(1));
        assert_eq!(compaction, None);
        assert_eq!(fixture.journal()?, original);
        assert!(!fixture.ledger.paths.compaction_refusal.exists());
        Ok(())
    }

    #[test]
    fn a_due_compaction_skips_a_held_original_journal_lock_without_waiting()
    -> Result<(), Box<dyn Error>> {
        let mut fixture = CompactionFixture::round_trip()?;
        let droppable = fixtures::superseded_bytes(&fixture.ledger)?;
        fixture.ledger.compaction_threshold_bytes = droppable;
        let original = fixture.journal()?;
        let journal_path = fixture.ledger.paths.journal.clone();
        let (locked_sender, locked) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let holder = thread::spawn(move || -> std::io::Result<()> {
            // A replay checkpoint writer holds the journal it replays.
            let journal = File::open(journal_path)?;
            journal.lock()?;
            locked_sender.send(()).map_err(std::io::Error::other)?;
            let _ = released.recv_timeout(JOURNAL_LOCK_HOLD_LIMIT);
            Ok(())
        });
        locked.recv()?;

        let started_at = Instant::now();
        let compaction = fixture.ledger.attempt_due_compaction(
            droppable,
            replays_agree,
            None,
            &RecordedAt::now(),
        );
        let elapsed = started_at.elapsed();
        let _ = release.send(());
        holder
            .join()
            .map_err(|_| "the journal lock holder panicked")??;
        assert!(
            elapsed < Duration::from_secs(1),
            "automatic compaction waited {elapsed:?} for the original journal lock"
        );
        assert_eq!(compaction?, None);
        assert_eq!(fixture.journal()?, original);
        assert!(!fixture.ledger.paths.compaction_refusal.exists());
        Ok(())
    }

    #[test]
    fn a_refused_compaction_waits_for_a_threshold_of_growth() -> Result<(), Box<dyn Error>> {
        let (events, rejected) = fixtures::recent_round_trip_events()?;
        let (renewal, _) = fixtures::renewal_and_observation(&events[..rejected])?;
        let mut fixture = CompactionFixture::holding(events[..rejected].to_vec())?;
        let droppable = fixtures::superseded_bytes(&fixture.ledger)?;
        fixture.ledger.compaction_threshold_bytes = droppable;
        let fixture = fixture;
        let ledger = &fixture.ledger;
        let original = fixture.journal()?;
        let disagree = |_: &JournalReplay, _: &JournalReplay| false;

        let refusal = ledger.attempt_due_compaction(droppable, disagree, None, &RecordedAt::now());
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
            .attempt_due_compaction(droppable, disagree, None, &RecordedAt::now())
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
            ledger.attempt_due_compaction(droppable, replays_agree, None, &RecordedAt::now())?,
            None
        );
        assert_eq!(fixture.journal()?, original);

        let limit = refused.len() + droppable;
        let mut appends = 0;
        loop {
            fixtures::append(ledger, &renewal)?;
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
        assert_eq!(fixtures::superseded_bytes(ledger)?, 0);
        Ok(())
    }
}
