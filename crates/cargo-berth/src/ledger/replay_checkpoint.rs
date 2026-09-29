//! The disposable replay checkpoint, so a replay decodes only the records appended after it.
//!
//! `replay-checkpoint.json` holds two JSON lines: a [`CheckpointHeader`], then the
//! [`JournalReplay`] of the journal's first [`CheckpointHeader::end_offset`] bytes, which always
//! end at a complete record. The header is decoded first, and the replay line only when the header
//! still describes the journal: this build's [`FOLD_FORMAT_VERSION`], this repository, the same
//! journal file by device and inode, a journal at least `end_offset` bytes long, and an unchanged
//! last checkpointed record, compared by the FNV-1a digest of its bytes at its offset. A checkpoint
//! that is missing, unreadable, or undecodable, or that describes anything else, is ignored and the
//! journal replays from byte 0.
//!
//! A replay that folds [`REPLAY_CHECKPOINT_INTERVAL_BYTES`] or
//! [`REPLAY_CHECKPOINT_INTERVAL_RECORDS`] past its start writes a new checkpoint to a uniquely
//! named temporary file and renames it into place, without a sync. Locked and lock-free replays
//! both write it, and a failed write never fails the replay. Projection repair and reinitialization
//! delete it.
//!
//! Only the last checkpointed record is compared with the journal. An edit to, or corruption of,
//! an earlier record inside the checkpointed prefix goes undetected until the checkpoint is
//! deleted or invalidated, and the replay's fingerprint continues from the checkpointed one
//! instead of covering the edited bytes.

use std::fs;
use std::fs::File;
use std::fs::Metadata;
use std::io;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::constants::FOLD_FORMAT_VERSION;
use super::constants::REPLAY_CHECKPOINT_INTERVAL_BYTES;
use super::constants::REPLAY_CHECKPOINT_INTERVAL_RECORDS;
use super::journal::JournalError;
use super::journal::JournalFingerprint;
use super::journal::JournalReplay;
use crate::ids::JournalByteOffset;
use crate::ids::RepoInstanceId;

/// A ledger's replay checkpoint file and the repository it belongs to.
pub(super) struct ReplayCheckpoint<'a> {
    path:             &'a Path,
    repo_instance_id: RepoInstanceId,
    interval:         CheckpointInterval,
}

/// How far a replay folds past its start before it rewrites the checkpoint.
#[derive(Clone, Copy)]
struct CheckpointInterval {
    bytes:   u64,
    records: u64,
}

/// What a checkpoint must match before its replay line is decoded.
#[derive(Debug, Deserialize, Serialize)]
struct CheckpointHeader {
    /// The [`FOLD_FORMAT_VERSION`] of the build that wrote the replay line.
    fold_format_version: u32,
    /// The repository whose ledger holds the checkpoint.
    repo_instance_id:    RepoInstanceId,
    /// The journal file the checkpointed replay was read from.
    journal:             JournalFileIdentity,
    /// The journal length the checkpointed replay covers.
    end_offset:          JournalByteOffset,
    /// The journal offset of the last checkpointed record.
    last_record_offset:  JournalByteOffset,
    /// The FNV-1a digest of the last checkpointed record's bytes, newline included.
    last_record_digest:  JournalFingerprint,
}

/// The file behind the journal path, which a journal replaced by a copy of its bytes does not
/// share.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct JournalFileIdentity {
    device: u64,
    inode:  u64,
}

impl From<&Metadata> for JournalFileIdentity {
    fn from(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode:  metadata.ino(),
        }
    }
}

impl<'a> ReplayCheckpoint<'a> {
    /// The checkpoint at `path` in the ledger of `repo_instance_id`.
    pub(super) const fn new(path: &'a Path, repo_instance_id: RepoInstanceId) -> Self {
        Self {
            path,
            repo_instance_id,
            interval: CheckpointInterval {
                bytes:   REPLAY_CHECKPOINT_INTERVAL_BYTES,
                records: REPLAY_CHECKPOINT_INTERVAL_RECORDS,
            },
        }
    }

    /// Replay the complete records of the journal at `journal_path`, resuming after this
    /// checkpoint when it still describes that journal.
    ///
    /// Also returns the journal length read, which passes the replay's end offset by an incomplete
    /// final record. Rewrites the checkpoint when the replay folds a whole interval past its start.
    pub(super) fn replay_journal(
        &self,
        journal_path: &Path,
    ) -> Result<(JournalReplay, JournalByteOffset), JournalError> {
        let mut journal_file = File::open(journal_path)?;
        let metadata = journal_file.metadata()?;
        let journal = JournalFileIdentity::from(&metadata);
        let start = self
            .load(&mut journal_file, journal, metadata.len())
            .unwrap_or_else(JournalReplay::empty);
        let start_offset = u64::from(start.end_offset);
        let start_record_count = start.record_count;
        journal_file.seek(SeekFrom::Start(start_offset))?;
        let mut appended = Vec::new();
        journal_file.read_to_end(&mut appended)?;

        let (replay, complete_end) = start.extended_over(&appended)?;
        if u64::from(replay.end_offset) - start_offset >= self.interval.bytes
            || replay.record_count - start_record_count >= self.interval.records
        {
            // A replay that cannot write the checkpoint is still complete.
            std::mem::drop(self.store(&replay, journal, &appended[..complete_end]));
        }
        let appended_length =
            u64::try_from(appended.len()).map_err(JournalError::JournalTooLarge)?;
        Ok((
            replay,
            JournalByteOffset::from(start_offset + appended_length),
        ))
    }

    /// The checkpointed replay, when the checkpoint still describes `journal_file`: the journal
    /// file `journal`, `journal_length` bytes long.
    fn load(
        &self,
        journal_file: &mut File,
        journal: JournalFileIdentity,
        journal_length: u64,
    ) -> Option<JournalReplay> {
        let contents = fs::read(self.path).ok()?;
        let header_end = contents.iter().position(|byte| *byte == b'\n')?;
        let header = serde_json::from_slice::<CheckpointHeader>(&contents[..header_end]).ok()?;
        let end_offset = u64::from(header.end_offset);
        if header.fold_format_version != FOLD_FORMAT_VERSION
            || header.repo_instance_id != self.repo_instance_id
            || header.journal != journal
            || journal_length < end_offset
        {
            return None;
        }
        let last_record_offset = u64::from(header.last_record_offset);
        let last_record_length = end_offset.checked_sub(last_record_offset)?;
        let mut last_record = vec![0; usize::try_from(last_record_length).ok()?];
        journal_file
            .seek(SeekFrom::Start(last_record_offset))
            .ok()?;
        journal_file.read_exact(&mut last_record).ok()?;
        if JournalFingerprint::EMPTY.continued(&last_record) != header.last_record_digest {
            return None;
        }
        let replay = serde_json::from_slice::<JournalReplay>(&contents[header_end + 1..]).ok()?;
        (replay.end_offset == header.end_offset).then_some(replay)
    }

    /// Write `replay` as the checkpoint of the journal file `journal`, where `folded` holds the
    /// complete records the replay just folded, ending at its end offset.
    fn store(
        &self,
        replay: &JournalReplay,
        journal: JournalFileIdentity,
        folded: &[u8],
    ) -> io::Result<()> {
        let Some((_, before_final_newline)) = folded.split_last() else {
            return Ok(());
        };
        let last_record_start = before_final_newline
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let last_record = &folded[last_record_start..];
        let last_record_length = u64::try_from(last_record.len()).map_err(io::Error::other)?;
        let header = CheckpointHeader {
            fold_format_version: FOLD_FORMAT_VERSION,
            repo_instance_id: self.repo_instance_id,
            journal,
            end_offset: replay.end_offset,
            last_record_offset: JournalByteOffset::from(
                u64::from(replay.end_offset) - last_record_length,
            ),
            last_record_digest: JournalFingerprint::EMPTY.continued(last_record),
        };
        let mut contents = serde_json::to_vec(&header)?;
        contents.push(b'\n');
        serde_json::to_writer(&mut contents, replay)?;
        contents.push(b'\n');

        let mut temporary_path = self.path.as_os_str().to_owned();
        temporary_path.push(format!(".{}.tmp", Uuid::now_v7()));
        let publication = fs::write(&temporary_path, &contents)
            .and_then(|()| fs::rename(&temporary_path, self.path));
        if publication.is_err() {
            std::mem::drop(fs::remove_file(&temporary_path));
        }
        publication
    }
}

/// Delete the checkpoint at `path`, so the next replay starts from byte 0.
pub(super) fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::fs::File;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::PathBuf;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::CheckpointHeader;
    use super::CheckpointInterval;
    use super::JournalFileIdentity;
    use super::ReplayCheckpoint;
    use crate::ids::EventId;
    use crate::ids::RepoInstanceId;
    use crate::ledger::constants::JOURNAL_FILE_NAME;
    use crate::ledger::constants::REPLAY_CHECKPOINT_FILE_NAME;
    use crate::ledger::constants::REPLAY_CHECKPOINT_INTERVAL_BYTES;
    use crate::ledger::constants::REPLAY_CHECKPOINT_INTERVAL_RECORDS;
    use crate::ledger::journal;
    use crate::ledger::journal::Journal;
    use crate::ledger::journal::JournalEvent;
    use crate::ledger::journal::JournalReplay;

    #[test]
    fn a_replay_resumed_from_a_checkpoint_equals_a_whole_journal_replay()
    -> Result<(), Box<dyn Error>> {
        let (events, _) = journal::tests::round_trip_sequence_events()?;
        for checkpointed_records in 1..=events.len() {
            let fixture = CheckpointFixture::new()?;
            let (checkpointed, appended) = events.split_at(checkpointed_records);
            fixture.journal.append_events(checkpointed)?;
            let checkpointed_replay = fixture.write_checkpoint()?;
            fixture.journal.append_events(appended)?;
            fixture.append_incomplete_record()?;

            assert_eq!(
                fixture.load(&fixture.kept_checkpoint())?,
                Some(checkpointed_replay),
                "the replays below resume from the checkpoint"
            );
            fixture.assert_replays_equal_whole_journal_replays(&fixture.kept_checkpoint())?;
        }
        Ok(())
    }

    #[test]
    fn a_checkpoint_rewritten_after_every_append_keeps_replays_equal_to_whole_journal_replays()
    -> Result<(), Box<dyn Error>> {
        let events = journal::tests::fold_sequence_events()?;
        let fixture = CheckpointFixture::new()?;
        for (appended_records, event) in events.iter().enumerate() {
            assert_eq!(
                fixture.load(&fixture.rewritten_checkpoint())?.is_some(),
                appended_records > 0,
                "every replay after the first resumes from the checkpoint"
            );
            fixture.journal.append(event)?;
            let replay = fixture
                .assert_replays_equal_whole_journal_replays(&fixture.rewritten_checkpoint())?;
            assert_eq!(fixture.header()?.end_offset, replay.end_offset);
        }
        Ok(())
    }

    #[test]
    fn a_checkpoint_that_no_longer_describes_the_journal_is_ignored() -> Result<(), Box<dyn Error>>
    {
        let events = journal::tests::fold_sequence_events()?;
        let (checkpointed, appended) = events.split_at(events.len() / 2);
        for invalidation in [
            Invalidation::Missing,
            Invalidation::Unparsable,
            Invalidation::FoldFormatVersion,
            Invalidation::OtherRepository,
            Invalidation::JournalReplacedByCopy,
            Invalidation::JournalShortened,
            Invalidation::LastRecordRewritten,
        ] {
            let fixture = CheckpointFixture::new()?;
            fixture.journal.append_events(checkpointed)?;
            fixture.write_checkpoint()?;
            fixture.journal.append_events(appended)?;
            assert!(fixture.load(&fixture.kept_checkpoint())?.is_some());

            let checkpoint = fixture.invalidate(invalidation)?;
            assert_eq!(fixture.load(&checkpoint)?, None, "{invalidation:?}");
            fixture.assert_replays_equal_whole_journal_replays(&checkpoint)?;
        }
        Ok(())
    }

    #[test]
    fn a_replay_rewrites_the_checkpoint_once_it_folds_the_record_interval()
    -> Result<(), Box<dyn Error>> {
        let events = journal::tests::fold_sequence_events()?;
        let smallest = events
            .iter()
            .min_by_key(|event| serde_json::to_vec(event).map_or(usize::MAX, |record| record.len()))
            .ok_or("the fold sequence should hold records")?;
        let interval = usize::try_from(REPLAY_CHECKPOINT_INTERVAL_RECORDS)?;
        let fixture = CheckpointFixture::new()?;

        fixture
            .journal
            .append_events(&copies_with_new_event_ids(smallest, interval - 1))?;
        fixture.replay()?;
        assert!(!fixture.checkpoint_path.exists());

        fixture
            .journal
            .append_events(&copies_with_new_event_ids(smallest, 1))?;
        let checkpointed = fixture.replay()?;
        assert!(
            u64::from(checkpointed.end_offset) < REPLAY_CHECKPOINT_INTERVAL_BYTES,
            "the record interval is reached before the byte interval"
        );
        assert_eq!(fixture.header()?.end_offset, checkpointed.end_offset);

        fixture
            .journal
            .append_events(&copies_with_new_event_ids(smallest, interval - 1))?;
        fixture.replay()?;
        assert_eq!(
            fixture.header()?.end_offset,
            checkpointed.end_offset,
            "records count from the loaded checkpoint"
        );

        fixture
            .journal
            .append_events(&copies_with_new_event_ids(smallest, 1))?;
        let rewritten = fixture.replay()?;
        assert_eq!(fixture.header()?.end_offset, rewritten.end_offset);
        assert_eq!(
            fs::read_dir(fixture.directory.path())?.count(),
            2,
            "only the journal and the checkpoint remain"
        );
        Ok(())
    }

    #[test]
    fn a_replay_rewrites_the_checkpoint_once_it_folds_the_byte_interval()
    -> Result<(), Box<dyn Error>> {
        let events = journal::tests::fold_sequence_events()?;
        let largest = events
            .iter()
            .max_by_key(|event| serde_json::to_vec(event).map_or(0, |record| record.len()))
            .ok_or("the fold sequence should hold records")?;
        let record_length = u64::try_from(serde_json::to_vec(largest)?.len() + 1)?;
        let records_to_interval = REPLAY_CHECKPOINT_INTERVAL_BYTES.div_ceil(record_length);
        assert!(
            records_to_interval < REPLAY_CHECKPOINT_INTERVAL_RECORDS,
            "the byte interval is reached before the record interval"
        );
        let fixture = CheckpointFixture::new()?;

        fixture.journal.append_events(&copies_with_new_event_ids(
            largest,
            usize::try_from(records_to_interval - 1)?,
        ))?;
        fixture.replay()?;
        assert!(!fixture.checkpoint_path.exists());

        fixture
            .journal
            .append_events(&copies_with_new_event_ids(largest, 1))?;
        let checkpointed = fixture.replay()?;
        assert_eq!(fixture.header()?.end_offset, checkpointed.end_offset);
        Ok(())
    }

    #[test]
    fn a_checkpoint_that_cannot_be_written_leaves_the_replay_complete() -> Result<(), Box<dyn Error>>
    {
        let fixture = CheckpointFixture::new()?;
        fixture
            .journal
            .append_events(&journal::tests::fold_sequence_events()?)?;
        let unwritable_path = fixture
            .directory
            .path()
            .join("missing")
            .join(REPLAY_CHECKPOINT_FILE_NAME);
        let checkpoint = ReplayCheckpoint {
            path: &unwritable_path,
            ..fixture.rewritten_checkpoint()
        };

        assert_eq!(
            Journal::replay_read_only_from_checkpoint(&fixture.journal_path, &checkpoint)?,
            Journal::replay_read_only(&fixture.journal_path)?
        );
        assert!(!unwritable_path.exists());
        Ok(())
    }

    /// A change after which a replay must ignore the checkpoint.
    #[derive(Clone, Copy, Debug)]
    enum Invalidation {
        Missing,
        Unparsable,
        FoldFormatVersion,
        OtherRepository,
        JournalReplacedByCopy,
        JournalShortened,
        LastRecordRewritten,
    }

    /// A journal and its checkpoint in a temporary ledger directory.
    struct CheckpointFixture {
        directory:        TempDir,
        journal:          Journal,
        journal_path:     PathBuf,
        checkpoint_path:  PathBuf,
        repo_instance_id: RepoInstanceId,
    }

    impl CheckpointFixture {
        fn new() -> Result<Self, Box<dyn Error>> {
            let directory = tempdir()?;
            let journal_path = directory.path().join(JOURNAL_FILE_NAME);
            let (journal, _) = Journal::open_or_create(&journal_path)?;
            Ok(Self {
                checkpoint_path: directory.path().join(REPLAY_CHECKPOINT_FILE_NAME),
                directory,
                journal,
                journal_path,
                repo_instance_id: RepoInstanceId::new(),
            })
        }

        /// The checkpoint with the interval production replays use.
        fn checkpoint(&self) -> ReplayCheckpoint<'_> {
            ReplayCheckpoint::new(&self.checkpoint_path, self.repo_instance_id)
        }

        /// The checkpoint with an interval every replay of one record reaches.
        fn rewritten_checkpoint(&self) -> ReplayCheckpoint<'_> {
            ReplayCheckpoint {
                interval: CheckpointInterval {
                    bytes:   1,
                    records: 1,
                },
                ..self.checkpoint()
            }
        }

        /// The checkpoint with an interval no replay reaches, so replays leave it unchanged.
        fn kept_checkpoint(&self) -> ReplayCheckpoint<'_> {
            ReplayCheckpoint {
                interval: CheckpointInterval {
                    bytes:   u64::MAX,
                    records: u64::MAX,
                },
                ..self.checkpoint()
            }
        }

        /// Replay the journal as a lock-free production read does.
        fn replay(&self) -> Result<JournalReplay, Box<dyn Error>> {
            Ok(Journal::replay_read_only_from_checkpoint(
                &self.journal_path,
                &self.checkpoint(),
            )?)
        }

        /// Write a checkpoint at the end of the journal's complete records.
        fn write_checkpoint(&self) -> Result<JournalReplay, Box<dyn Error>> {
            Ok(Journal::replay_read_only_from_checkpoint(
                &self.journal_path,
                &self.rewritten_checkpoint(),
            )?)
        }

        /// The replay `checkpoint` resumes from, or `None` when a replay starts at byte 0.
        fn load(
            &self,
            checkpoint: &ReplayCheckpoint<'_>,
        ) -> Result<Option<JournalReplay>, Box<dyn Error>> {
            let mut journal_file = File::open(&self.journal_path)?;
            let metadata = journal_file.metadata()?;
            Ok(checkpoint.load(
                &mut journal_file,
                JournalFileIdentity::from(&metadata),
                metadata.len(),
            ))
        }

        fn header(&self) -> Result<CheckpointHeader, Box<dyn Error>> {
            let contents = fs::read(&self.checkpoint_path)?;
            let header_line = contents
                .split(|byte| *byte == b'\n')
                .next()
                .ok_or("the checkpoint should hold a header line")?;
            Ok(serde_json::from_slice(header_line)?)
        }

        fn append_incomplete_record(&self) -> Result<(), Box<dyn Error>> {
            OpenOptions::new()
                .append(true)
                .open(&self.journal_path)?
                .write_all(b"{\"op\":")?;
            Ok(())
        }

        /// Apply `invalidation` and return the checkpoint a replay then reads.
        fn invalidate(
            &self,
            invalidation: Invalidation,
        ) -> Result<ReplayCheckpoint<'_>, Box<dyn Error>> {
            let mut checkpoint = self.kept_checkpoint();
            match invalidation {
                Invalidation::Missing => fs::remove_file(&self.checkpoint_path)?,
                Invalidation::Unparsable => fs::write(&self.checkpoint_path, b"not json\n")?,
                Invalidation::FoldFormatVersion => {
                    let contents = fs::read(&self.checkpoint_path)?;
                    let header_end = contents
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .ok_or("the checkpoint should hold a header line")?;
                    let mut header =
                        serde_json::from_slice::<CheckpointHeader>(&contents[..header_end])?;
                    header.fold_format_version += 1;
                    let mut rewritten = serde_json::to_vec(&header)?;
                    rewritten.extend_from_slice(&contents[header_end..]);
                    fs::write(&self.checkpoint_path, rewritten)?;
                },
                Invalidation::OtherRepository => {
                    checkpoint.repo_instance_id = RepoInstanceId::new();
                },
                Invalidation::JournalReplacedByCopy => {
                    let copy_path = self.directory.path().join("journal copy");
                    fs::copy(&self.journal_path, &copy_path)?;
                    fs::rename(&copy_path, &self.journal_path)?;
                },
                Invalidation::JournalShortened => {
                    OpenOptions::new()
                        .write(true)
                        .open(&self.journal_path)?
                        .set_len(u64::from(self.header()?.last_record_offset))?;
                },
                Invalidation::LastRecordRewritten => {
                    let journal_bytes = fs::read(&self.journal_path)?;
                    let last_record_offset = u64::from(self.header()?.last_record_offset);
                    let rewritten = journal::complete_record_events(
                        &journal_bytes[usize::try_from(last_record_offset)?..],
                    )?
                    .iter()
                    .flat_map(|event| copies_with_new_event_ids(event, 1))
                    .collect::<Vec<_>>();
                    OpenOptions::new()
                        .write(true)
                        .open(&self.journal_path)?
                        .set_len(last_record_offset)?;
                    self.journal.append_events(&rewritten)?;
                    assert_eq!(
                        fs::read(&self.journal_path)?.len(),
                        journal_bytes.len(),
                        "the rewritten records keep their length"
                    );
                },
            }
            Ok(checkpoint)
        }

        /// Require that the locked and lock-free replays through `checkpoint` equal checkpoint-free
        /// replays of the whole journal, and return the locked replay.
        fn assert_replays_equal_whole_journal_replays(
            &self,
            checkpoint: &ReplayCheckpoint<'_>,
        ) -> Result<JournalReplay, Box<dyn Error>> {
            let whole = Journal::replay_read_only(&self.journal_path)?;
            assert_eq!(
                Journal::replay_read_only_from_checkpoint(&self.journal_path, checkpoint)?,
                whole
            );
            let repaired = self
                .journal
                .replay_repairing_tail_from_checkpoint(checkpoint)?;
            assert_eq!(repaired, whole);
            assert_eq!(repaired, self.journal.replay_repairing_tail()?);
            assert_eq!(
                fs::metadata(&self.journal_path)?.len(),
                u64::from(repaired.end_offset),
                "the incomplete final record is repaired"
            );
            Ok(repaired)
        }
    }

    fn copies_with_new_event_ids(event: &JournalEvent, count: usize) -> Vec<JournalEvent> {
        (0..count)
            .map(|_| {
                let mut copy = event.clone();
                copy.event_id = EventId::new();
                copy
            })
            .collect()
    }
}
