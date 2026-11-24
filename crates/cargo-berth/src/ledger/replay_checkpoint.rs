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
//! A replay can refresh the checkpoint after the byte or record interval. The write
//! budget grows with the complete journal bytes appended since the checkpoint.
//! One replay per journal file can publish at a time. The writer checks the current
//! header before serialization. It publishes by rename. A failed write never fails
//! the replay. Projection repair and reinitialization delete the checkpoint.
//!
//! Only the last checkpointed record is compared with the journal. An edit to, or corruption of,
//! an earlier record inside the checkpointed prefix goes undetected until the checkpoint is
//! deleted or invalidated, and the replay's fingerprint continues from the checkpointed one
//! instead of covering the edited bytes.

use std::fs;
use std::fs::File;
use std::fs::Metadata;
use std::fs::OpenOptions;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::constants::FOLD_FORMAT_VERSION;
use super::constants::REPLAY_CHECKPOINT_INTERVAL_BYTES;
use super::constants::REPLAY_CHECKPOINT_INTERVAL_RECORDS;
use super::constants::REPLAY_CHECKPOINT_MAXIMUM_WRITE_RATIO;
use super::journal::JournalError;
use super::journal::JournalFileIdentity;
use super::journal::JournalFingerprint;
use super::journal::JournalReplay;
use super::journal::JournalReplaySource;
use crate::ids::JournalByteOffset;
use crate::ids::RepoInstanceId;

/// A ledger's replay checkpoint file and the repository it belongs to.
pub(super) struct ReplayCheckpoint<'a> {
    path:             &'a Path,
    repo_instance_id: RepoInstanceId,
    interval:         CheckpointInterval,
    /// Writes the encoded checkpoint to its temporary file. Tests supply one that counts bytes.
    write_temporary:  fn(&mut File, &[u8]) -> io::Result<()>,
}

/// How far a replay folds past its start before it rewrites the checkpoint.
#[derive(Clone, Copy)]
struct CheckpointInterval {
    bytes:               u64,
    records:             u64,
    maximum_write_ratio: u64,
}

/// Where a replay starts, and the checkpoint header it read there.
enum ReplayStart {
    /// The checkpoint describes the journal, and the replay resumes at its end offset.
    Checkpoint {
        header: CheckpointHeader,
        /// The byte length of the checkpoint file.
        bytes:  u64,
    },
    /// The checkpoint does not describe the journal, and the replay starts at byte 0.
    Beginning { observed_header: ObservedHeader },
}

/// The header a replay read from the checkpoint file. A writer replaces the file only while the
/// file still holds this header or no readable header.
#[derive(PartialEq)]
enum ObservedHeader {
    /// The file is missing, has no header line, or its header line does not decode.
    Absent,
    /// The decoded header line, which can describe another journal.
    Present(CheckpointHeader),
}

/// How many bytes a checkpoint refresh can write.
#[derive(Clone, Copy)]
enum WriteBudget {
    /// The replay started at byte 0, so the refresh has no size limit.
    Unlimited,
    /// The journal bytes appended since the checkpoint, times the maximum write ratio.
    Limited(u64),
    /// The checkpoint is too large for the appended bytes, so the refresh waits for more records.
    Exceeded,
}

/// Whether a replay that holds the journal lock replaces the checkpoint file.
enum PublicationDecision {
    /// The journal path and the published header are as the replay saw them: write.
    Write,
    /// Another journal or another writer's checkpoint is now in place: keep the file.
    Skip,
}

/// What a checkpoint must match before its replay line is decoded.
#[derive(Debug, Deserialize, PartialEq, Serialize)]
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

impl<'a> ReplayCheckpoint<'a> {
    /// The checkpoint at `path` in the ledger of `repo_instance_id`.
    pub(super) const fn new(path: &'a Path, repo_instance_id: RepoInstanceId) -> Self {
        Self {
            path,
            repo_instance_id,
            interval: CheckpointInterval {
                bytes:               REPLAY_CHECKPOINT_INTERVAL_BYTES,
                records:             REPLAY_CHECKPOINT_INTERVAL_RECORDS,
                maximum_write_ratio: REPLAY_CHECKPOINT_MAXIMUM_WRITE_RATIO,
            },
            write_temporary: <File as Write>::write_all,
        }
    }

    /// Replay the complete records of the journal at `journal_path`, resuming after this
    /// checkpoint when it still describes that journal.
    ///
    /// Keep the source file open for as long as the caller can reuse the replay.
    pub(super) fn replay_journal(
        &self,
        journal_path: &Path,
    ) -> Result<(JournalReplay, JournalReplaySource), JournalError> {
        let mut journal_file = File::open(journal_path)?;
        let metadata = journal_file.metadata()?;
        let journal = JournalFileIdentity::from(&metadata);
        let (start, replay_start) = self.load(&mut journal_file, journal, metadata.len());
        let start_offset = u64::from(start.end_offset);
        let start_record_count = start.record_count;
        journal_file.seek(SeekFrom::Start(start_offset))?;
        let mut appended = Vec::new();
        journal_file.read_to_end(&mut appended)?;

        let (replay, complete_end) = start.extended_over(&appended)?;
        let growth = u64::from(replay.end_offset) - start_offset;
        let write_budget = replay_start.write_budget(growth, self.interval.maximum_write_ratio);
        if !matches!(write_budget, WriteBudget::Exceeded)
            && (growth >= self.interval.bytes
                || replay.record_count - start_record_count >= self.interval.records)
            && journal_file.try_lock().is_ok()
        {
            if matches!(
                self.publication_decision(
                    journal_path,
                    journal,
                    &replay_start.into_observed_header()
                ),
                PublicationDecision::Write
            ) {
                // A replay that cannot write the checkpoint is still complete.
                std::mem::drop(self.store(
                    &replay,
                    journal,
                    &appended[..complete_end],
                    write_budget,
                ));
            }
            journal_file.unlock()?;
        }
        Ok((replay, JournalReplaySource::new(journal_file, journal)))
    }

    /// The replay to start from and where it starts: the checkpointed replay when the checkpoint
    /// describes `journal_file`, or else the empty replay. Keep an invalid checkpoint's header
    /// so the writer can repair that file.
    fn load(
        &self,
        journal_file: &mut File,
        journal: JournalFileIdentity,
        journal_length: u64,
    ) -> (JournalReplay, ReplayStart) {
        let beginning = |observed_header| {
            (
                JournalReplay::empty(),
                ReplayStart::Beginning { observed_header },
            )
        };
        let Ok(contents) = fs::read(self.path) else {
            return beginning(ObservedHeader::Absent);
        };
        let Some(header_end) = contents.iter().position(|byte| *byte == b'\n') else {
            return beginning(ObservedHeader::Absent);
        };
        let Ok(header) = serde_json::from_slice::<CheckpointHeader>(&contents[..header_end]) else {
            return beginning(ObservedHeader::Absent);
        };
        match self.decode_replay(
            journal_file,
            journal,
            journal_length,
            &header,
            &contents[header_end + 1..],
        ) {
            Some(replay) => (
                replay,
                ReplayStart::Checkpoint {
                    header,
                    bytes: u64::try_from(contents.len()).unwrap_or(u64::MAX),
                },
            ),
            None => beginning(ObservedHeader::Present(header)),
        }
    }

    fn decode_replay(
        &self,
        journal_file: &mut File,
        journal: JournalFileIdentity,
        journal_length: u64,
        header: &CheckpointHeader,
        contents: &[u8],
    ) -> Option<JournalReplay> {
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
        #[cfg(test)]
        CHECKPOINT_DECODES.update(|decodes| decodes + 1);
        let replay = serde_json::from_slice::<JournalReplay>(contents).ok()?;
        (replay.end_offset == header.end_offset).then_some(replay)
    }

    /// Check the journal path and the published header while the writer holds the file lock.
    fn publication_decision(
        &self,
        journal_path: &Path,
        journal: JournalFileIdentity,
        observed_header: &ObservedHeader,
    ) -> PublicationDecision {
        let published_header = self.read_header();
        if fs::metadata(journal_path)
            .is_ok_and(|metadata| JournalFileIdentity::from(&metadata) == journal)
            && (published_header == ObservedHeader::Absent || published_header == *observed_header)
        {
            PublicationDecision::Write
        } else {
            PublicationDecision::Skip
        }
    }

    /// The header line of the published checkpoint file.
    fn read_header(&self) -> ObservedHeader {
        let Ok(file) = File::open(self.path) else {
            return ObservedHeader::Absent;
        };
        let mut reader = BufReader::new(file);
        let Ok(bytes) = reader.fill_buf() else {
            return ObservedHeader::Absent;
        };
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .and_then(|header_end| serde_json::from_slice(&bytes[..header_end]).ok())
            .map_or(ObservedHeader::Absent, ObservedHeader::Present)
    }

    /// Write `replay` as the checkpoint of the journal file `journal` describes, where `folded`
    /// holds complete records ending at the replay's end offset.
    ///
    /// The checkpoint follows that file through a rename, which keeps its device and inode.
    pub(super) fn store_for_journal_file(
        &self,
        replay: &JournalReplay,
        journal: &Metadata,
        folded: &[u8],
    ) -> io::Result<()> {
        self.store(
            replay,
            JournalFileIdentity::from(journal),
            folded,
            WriteBudget::Unlimited,
        )
    }

    /// Write `replay` as the checkpoint of the journal file `journal`, where `folded` holds the
    /// complete records the replay just folded, ending at its end offset.
    fn store(
        &self,
        replay: &JournalReplay,
        journal: JournalFileIdentity,
        folded: &[u8],
        write_budget: WriteBudget,
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
        if !write_budget.admits(u64::try_from(contents.len()).map_err(io::Error::other)?) {
            return Ok(());
        }

        let mut temporary_path = self.path.as_os_str().to_owned();
        temporary_path.push(format!(".{}.tmp", Uuid::now_v7()));
        let mut temporary = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)?;
        let publication = temporary.lock().and_then(|()| {
            (self.write_temporary)(&mut temporary, &contents)?;
            fs::rename(&temporary_path, self.path)
        });
        if publication.is_err() {
            std::mem::drop(fs::remove_file(&temporary_path));
        }
        publication
    }
}

#[cfg(test)]
thread_local! {
    static CHECKPOINT_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn count_checkpoint_decodes<Output>(action: impl FnOnce() -> Output) -> (Output, usize) {
    let before = CHECKPOINT_DECODES.get();
    let output = action();
    (output, CHECKPOINT_DECODES.get() - before)
}

impl ReplayStart {
    /// The bytes a refresh can write after the replay folds `growth` journal bytes past this start.
    const fn write_budget(&self, growth: u64, maximum_write_ratio: u64) -> WriteBudget {
        match self {
            Self::Checkpoint { bytes, .. } => {
                let limit = growth.saturating_mul(maximum_write_ratio);
                // The refresh writes the checkpoint again, with the appended records added.
                if *bytes <= limit.saturating_sub(growth) {
                    WriteBudget::Limited(limit)
                } else {
                    WriteBudget::Exceeded
                }
            },
            Self::Beginning { .. } => WriteBudget::Unlimited,
        }
    }

    /// The header a writer expects to find before it replaces the checkpoint file.
    const fn into_observed_header(self) -> ObservedHeader {
        match self {
            Self::Checkpoint { header, .. } => ObservedHeader::Present(header),
            Self::Beginning { observed_header } => observed_header,
        }
    }
}

impl WriteBudget {
    /// Whether a checkpoint of `bytes` fits this budget.
    const fn admits(self, bytes: u64) -> bool {
        match self {
            Self::Unlimited => true,
            Self::Limited(limit) => bytes <= limit,
            Self::Exceeded => false,
        }
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
mod tests {
    use std::cell::Cell;
    use std::error::Error;
    use std::fs;
    use std::fs::File;
    use std::fs::OpenOptions;
    use std::io;
    use std::io::Write;
    use std::path::PathBuf;

    use serde_json::Value;
    use serde_json::json;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::CheckpointHeader;
    use super::CheckpointInterval;
    use super::JournalFileIdentity;
    use super::ReplayCheckpoint;
    use super::ReplayStart;
    use crate::ids::EventId;
    use crate::ids::ProjectionGeneration;
    use crate::ids::RepoInstanceId;
    use crate::ledger::constants::FOLD_FORMAT_VERSION;
    use crate::ledger::constants::JOURNAL_FILE_NAME;
    use crate::ledger::constants::REPLAY_CHECKPOINT_FILE_NAME;
    use crate::ledger::constants::REPLAY_CHECKPOINT_INTERVAL_BYTES;
    use crate::ledger::constants::REPLAY_CHECKPOINT_INTERVAL_RECORDS;
    use crate::ledger::constants::REPLAY_CHECKPOINT_MAXIMUM_WRITE_RATIO;
    use crate::ledger::journal;
    use crate::ledger::journal::Journal;
    use crate::ledger::journal::JournalEvent;
    use crate::ledger::journal::JournalOperation;
    use crate::ledger::journal::JournalReplay;

    /// The audit records in the large checkpoint. The first refresh skips while the checkpoint
    /// is larger than `TEST_RECORD_INTERVAL * (ratio - 1)` appended records, and each record
    /// adds about one record of bytes to the checkpoint. Half again as many records keeps the
    /// first refresh over budget at every ratio from 4 to 256.
    const RETAINED_RECORDS: u64 =
        TEST_RECORD_INTERVAL * REPLAY_CHECKPOINT_MAXIMUM_WRITE_RATIO * 3 / 2;
    const TEST_RECORD_INTERVAL: u64 = 4;
    const PREVIOUS_FOLD_FORMAT_VERSION: u32 = 3;
    /// A scope revision digest for the `answer` record of the previous fold checkpoint.
    const PREVIOUS_FOLD_SCOPE_REVISION: &str =
        "abababababababababababababababababababababababababababababababab";
    /// The trunks of the two revalidations after the release in the previous fold checkpoint.
    const PREVIOUS_FOLD_SECOND_TRUNK: &str = "4444444444444444444444444444444444444444";
    const PREVIOUS_FOLD_THIRD_TRUNK: &str = "5555555555555555555555555555555555555555";

    thread_local! {
        static TEMPORARY_BYTES_WRITTEN: Cell<u64> = const { Cell::new(0) };
    }

    /// Write `contents` to the temporary checkpoint file and count the bytes written.
    fn counting_write(file: &mut File, contents: &[u8]) -> io::Result<()> {
        file.write_all(contents)?;
        let written = u64::try_from(contents.len()).map_err(io::Error::other)?;
        TEMPORARY_BYTES_WRITTEN.with(|bytes| bytes.set(bytes.get() + written));
        Ok(())
    }

    #[test]
    fn a_replay_resumed_from_a_checkpoint_equals_a_whole_journal_replay()
    -> Result<(), Box<dyn Error>> {
        let (events, _) = journal::round_trip_sequence_events()?;
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
        let events = journal::fold_sequence_events()?;
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
        let events = journal::fold_sequence_events()?;
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

    /// Version 3 keeps the proof commits and full answer records. Replay starts at byte 0 and
    /// replaces the checkpoint with one that matches a whole-journal replay.
    #[test]
    fn a_checkpoint_of_the_previous_fold_rebuilds_from_the_journal() -> Result<(), Box<dyn Error>> {
        assert_ne!(FOLD_FORMAT_VERSION, PREVIOUS_FOLD_FORMAT_VERSION);
        let fixture = CheckpointFixture::new()?;
        let (events, released_id) = previous_fold_events()?;
        fixture.journal.append_events(&events)?;
        let current = fixture.write_checkpoint()?;
        let previous_fold_replay = previous_fold_checkpoint(&fixture, &released_id)?;
        assert_ne!(
            previous_fold_replay,
            serde_json::to_value(&current)?,
            "the previous fold keeps content the current fold does not"
        );

        assert_eq!(fixture.load(&fixture.kept_checkpoint())?, None);
        let rebuilt =
            fixture.assert_replays_equal_whole_journal_replays(&fixture.rewritten_checkpoint())?;
        assert_eq!(rebuilt, current);
        assert_eq!(fixture.header()?.fold_format_version, FOLD_FORMAT_VERSION);
        assert_eq!(fixture.load(&fixture.kept_checkpoint())?, Some(current));
        Ok(())
    }

    /// The fold sequence up to its release, then two `ProtectedTipAncestor` revalidations of the
    /// released reservation and one `answer` record. Also returns the released reservation id.
    fn previous_fold_events() -> Result<(Vec<JournalEvent>, Value), Box<dyn Error>> {
        let mut events = journal::fold_sequence_events()?;
        let release_index = events
            .iter()
            .position(|event| matches!(event.operation, JournalOperation::Release { .. }))
            .ok_or("the fold sequence should release a reservation")?;
        events.truncate(release_index + 1);
        let release = events[release_index].clone();
        let released_id = serde_json::to_value(&release.operation)?["reservation_id"].clone();
        let operations = [PREVIOUS_FOLD_SECOND_TRUNK, PREVIOUS_FOLD_THIRD_TRUNK]
            .into_iter()
            .map(|trunk_oid| {
                json!({
                    "op": "evidence_revalidated",
                    "reservation_id": released_id,
                    "status": {
                        "status": "integrated",
                        "trunk_oid": trunk_oid,
                        "proof": "protected_tip_ancestor"
                    },
                    "edit_blocking_status": "clear",
                })
            })
            .chain(std::iter::once(json!({
                "op": "answer",
                "scopes": [{"path": "src", "kind": "tree"}],
                "authorizations": [previous_fold_defer(&released_id)],
            })));
        for (operation, projection_generation) in
            operations.zip(u64::from(release.projection_generation) + 1..)
        {
            let mut event = copies_with_new_event_ids(&release, 1)
                .pop()
                .ok_or("one copy of the release")?;
            event.operation = serde_json::from_value(operation)?;
            event.projection_generation = ProjectionGeneration::from(projection_generation);
            events.push(event);
        }
        Ok((events, released_id))
    }

    /// A `defer` answer to `holder` over the `src` tree.
    fn previous_fold_defer(holder: &Value) -> Value {
        json!({
            "kind": "defer",
            "overlaps": [{
                "reservation_id": holder,
                "scope_revision": PREVIOUS_FOLD_SCOPE_REVISION,
                "scopes": [{"path": "src", "kind": "tree"}],
            }],
            "blocker": holder,
            "reason": "decide the order later",
        })
    }

    /// Write a version-3 checkpoint with all proof commits and full answer records.
    /// Return its replay line.
    fn previous_fold_checkpoint(
        fixture: &CheckpointFixture,
        released_id: &Value,
    ) -> Result<Value, Box<dyn Error>> {
        let contents = fs::read(&fixture.checkpoint_path)?;
        let header_end = contents
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or("the checkpoint should hold a header line")?;
        let mut header = serde_json::from_slice::<CheckpointHeader>(&contents[..header_end])?;
        header.fold_format_version = PREVIOUS_FOLD_FORMAT_VERSION;
        let mut replay = serde_json::from_slice::<Value>(&contents[header_end + 1..])?;
        let retained = &mut replay["reservations"]["Ok"];
        let released = retained["reservations"]
            .as_array_mut()
            .and_then(|reservations| {
                reservations
                    .iter_mut()
                    .find(|reservation| reservation["id"] == *released_id)
            })
            .ok_or("the checkpoint should hold the released reservation")?;
        let target_proof_commits = released["target_proof_commits"]
            .as_array_mut()
            .filter(|commits| !commits.is_empty())
            .ok_or("the checkpoint should hold the proof recorded before release")?;
        target_proof_commits.extend(
            [PREVIOUS_FOLD_SECOND_TRUNK, PREVIOUS_FOLD_THIRD_TRUNK]
                .map(|trunk_oid| json!(trunk_oid)),
        );
        for answer_record in retained["answer_records"]
            .as_array_mut()
            .ok_or("the checkpoint should hold answer records")?
        {
            *answer_record = json!({
                "worktree": answer_record["worktree"].clone(),
                "authorizations": [previous_fold_defer(released_id)],
            });
        }
        let mut rewritten = serde_json::to_vec(&header)?;
        rewritten.push(b'\n');
        rewritten.extend(serde_json::to_vec(&replay)?);
        rewritten.push(b'\n');
        fs::write(&fixture.checkpoint_path, rewritten)?;
        Ok(replay)
    }

    #[test]
    fn a_replay_rewrites_the_checkpoint_once_it_folds_the_record_interval()
    -> Result<(), Box<dyn Error>> {
        let events = journal::fold_sequence_events()?;
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
        let events = journal::fold_sequence_events()?;
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
    fn a_large_checkpoint_limits_bytes_written_per_appended_record() -> Result<(), Box<dyn Error>> {
        let fixture = CheckpointFixture::new()?;
        let event = journal::fold_sequence_events()?
            .into_iter()
            .find(|event| matches!(event.operation, JournalOperation::Bypass { .. }))
            .ok_or("the sequence should contain a retained audit record")?;
        append_audit_records(&fixture, &event, 0, RETAINED_RECORDS)?;
        let initial = fixture.write_checkpoint()?;
        let initial_bytes = fs::metadata(&fixture.checkpoint_path)?.len();
        let checkpoint = ReplayCheckpoint {
            interval: CheckpointInterval {
                records: TEST_RECORD_INTERVAL,
                ..fixture.checkpoint().interval
            },
            write_temporary: counting_write,
            ..fixture.checkpoint()
        };

        TEMPORARY_BYTES_WRITTEN.with(|bytes| bytes.set(0));
        append_audit_records(&fixture, &event, initial.record_count, TEST_RECORD_INTERVAL)?;
        let skipped =
            Journal::replay_read_only_from_checkpoint(&fixture.journal_path, &checkpoint)?;
        assert_eq!(
            skipped.record_count,
            initial.record_count + TEST_RECORD_INTERVAL
        );
        let skipped_growth = u64::from(skipped.end_offset) - u64::from(initial.end_offset);
        assert!(
            initial_bytes > skipped_growth * (REPLAY_CHECKPOINT_MAXIMUM_WRITE_RATIO - 1),
            "the fixture checkpoint must exceed the first refresh's budget"
        );
        assert_eq!(fixture.header()?.end_offset, initial.end_offset);
        assert_eq!(
            TEMPORARY_BYTES_WRITTEN.with(Cell::get),
            0,
            "an over-budget refresh must write no temporary data"
        );

        let record_bytes = skipped_growth / TEST_RECORD_INTERVAL;
        let growth_needed = initial_bytes.div_ceil(REPLAY_CHECKPOINT_MAXIMUM_WRITE_RATIO - 1);
        let remaining_records = (growth_needed.div_ceil(record_bytes) + 1)
            .checked_sub(TEST_RECORD_INTERVAL)
            .ok_or("the records within budget must follow the skipped records")?;
        append_audit_records(&fixture, &event, skipped.record_count, remaining_records)?;
        let refreshed =
            Journal::replay_read_only_from_checkpoint(&fixture.journal_path, &checkpoint)?;
        assert_eq!(fixture.header()?.end_offset, refreshed.end_offset);

        let written = TEMPORARY_BYTES_WRITTEN.with(Cell::get);
        let published = fs::metadata(&fixture.checkpoint_path)?.len();
        let appended_records = refreshed.record_count - initial.record_count;
        let appended_bytes = u64::from(refreshed.end_offset) - u64::from(initial.end_offset);
        let budget = appended_bytes * REPLAY_CHECKPOINT_MAXIMUM_WRITE_RATIO;
        assert_eq!(
            written, published,
            "the refresh must write exactly one temporary checkpoint"
        );
        assert!(
            written <= budget,
            "the checkpoint must stay within the write budget"
        );
        eprintln!(
            "checkpoint wrote {written} bytes over {appended_records} records ({} bytes per record)",
            written.div_ceil(appended_records)
        );
        Ok(())
    }

    fn append_audit_records(
        fixture: &CheckpointFixture,
        template: &JournalEvent,
        previous_records: u64,
        count: u64,
    ) -> Result<(), Box<dyn Error>> {
        let mut events = copies_with_new_event_ids(template, usize::try_from(count)?);
        for (generation, event) in (previous_records + 1..).zip(&mut events) {
            event.projection_generation = ProjectionGeneration::from(generation);
        }
        fixture.journal.append_events(&events)?;
        Ok(())
    }

    #[test]
    fn a_failed_checkpoint_rename_counts_the_temporary_write() -> Result<(), Box<dyn Error>> {
        let fixture = CheckpointFixture::new()?;
        fixture
            .journal
            .append_events(&journal::fold_sequence_events()?)?;
        fs::create_dir(&fixture.checkpoint_path)?;

        TEMPORARY_BYTES_WRITTEN.with(|bytes| bytes.set(0));
        let replay = Journal::replay_read_only_from_checkpoint(
            &fixture.journal_path,
            &ReplayCheckpoint {
                write_temporary: counting_write,
                ..fixture.rewritten_checkpoint()
            },
        )?;
        assert!(replay.record_count > 0);
        assert!(
            TEMPORARY_BYTES_WRITTEN.with(Cell::get) > 0,
            "count the bytes before removing the temporary file"
        );
        assert!(fixture.checkpoint_path.is_dir());
        assert_eq!(
            fs::read_dir(fixture.directory.path())?.count(),
            2,
            "remove the temporary file"
        );
        Ok(())
    }

    #[test]
    fn a_checkpoint_writer_defers_to_a_held_journal_lock() -> Result<(), Box<dyn Error>> {
        let fixture = CheckpointFixture::new()?;
        let events = journal::fold_sequence_events()?;
        fixture.journal.append_events(&events)?;
        let holder = File::open(&fixture.journal_path)?;
        holder.try_lock()?;

        let checkpoint = fixture.rewritten_checkpoint();
        let replay = Journal::replay_read_only_from_checkpoint(&fixture.journal_path, &checkpoint)?;
        assert!(!fixture.checkpoint_path.exists());
        drop(holder);

        let refreshed =
            Journal::replay_read_only_from_checkpoint(&fixture.journal_path, &checkpoint)?;
        assert_eq!(replay, refreshed);
        assert_eq!(fixture.header()?.end_offset, replay.end_offset);
        Ok(())
    }

    #[test]
    fn a_checkpoint_that_cannot_be_written_leaves_the_replay_complete() -> Result<(), Box<dyn Error>>
    {
        let fixture = CheckpointFixture::new()?;
        fixture
            .journal
            .append_events(&journal::fold_sequence_events()?)?;
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
                    bytes:               1,
                    records:             1,
                    maximum_write_ratio: u64::MAX,
                },
                ..self.checkpoint()
            }
        }

        /// The checkpoint with an interval no replay reaches, so replays leave it unchanged.
        fn kept_checkpoint(&self) -> ReplayCheckpoint<'_> {
            ReplayCheckpoint {
                interval: CheckpointInterval {
                    bytes:               u64::MAX,
                    records:             u64::MAX,
                    maximum_write_ratio: u64::MAX,
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
            match checkpoint.load(
                &mut journal_file,
                JournalFileIdentity::from(&metadata),
                metadata.len(),
            ) {
                (replay, ReplayStart::Checkpoint { .. }) => Ok(Some(replay)),
                (_, ReplayStart::Beginning { .. }) => Ok(None),
            }
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
            let mut repaired =
                Journal::replay_read_only_from_checkpoint(&self.journal_path, checkpoint)?;
            self.journal.advance_repairing_tail(&mut repaired)?;
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
