//! The disposable generation-stamped projection of the append-only journal.

use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;

use super::constants::CURRENT_SCHEMA_VERSION;
use super::constants::PROJECTION_TEMPORARY_FILE_NAME;
use super::journal::JournalEvent;
use super::journal::JournalFingerprint;
use super::journal::JournalReplay;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RepoInstanceId;
use crate::ids::SchemaVersion;

/// The serialized cache reconstructed solely from the journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Projection {
    /// The journal schema used to create this projection.
    schema_version:      SchemaVersion,
    /// The clone identity that owns this ledger.
    repo_instance_id:    RepoInstanceId,
    /// The byte length of the journal represented here.
    journal_end_offset:  JournalByteOffset,
    /// The generation published by the last journal mutation.
    generation:          ProjectionGeneration,
    /// A digest that detects journal changes without trusting cache contents.
    journal_fingerprint: JournalFingerprint,
    /// The replayed facts, including materialized edit-blocking evidence.
    events:              Vec<JournalEvent>,
}

impl Projection {
    /// Derive a projection from a complete replay.
    pub(super) fn from_replay(repo_instance_id: RepoInstanceId, replay: &JournalReplay) -> Self {
        Self {
            schema_version: SchemaVersion::from(CURRENT_SCHEMA_VERSION),
            repo_instance_id,
            journal_end_offset: replay.end_offset,
            generation: replay.generation,
            journal_fingerprint: replay.fingerprint,
            events: replay.events.clone(),
        }
    }

    /// Publish this projection through a synced temporary file and atomic rename.
    pub(super) fn publish(
        &self,
        ledger_directory: &Path,
        projection_path: &Path,
    ) -> Result<(), ProjectionError> {
        let temporary_path = ledger_directory.join(PROJECTION_TEMPORARY_FILE_NAME);
        let serialized_projection =
            serde_json::to_vec_pretty(self).map_err(ProjectionError::Serialization)?;
        let mut temporary_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary_path)?;
        temporary_file.write_all(&serialized_projection)?;
        temporary_file.write_all(b"\n")?;
        temporary_file.sync_all()?;
        fs::rename(temporary_path, projection_path)?;
        sync_directory(ledger_directory)?;
        Ok(())
    }

    /// Verify that this cache does not claim facts the journal cannot replay.
    pub(super) fn validate_against(
        &self,
        repo_instance_id: RepoInstanceId,
        replay: &JournalReplay,
    ) -> Result<(), ProjectionError> {
        let expected_schema_version = SchemaVersion::from(CURRENT_SCHEMA_VERSION);
        if self.schema_version != expected_schema_version {
            return Err(ProjectionError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        if self.repo_instance_id != repo_instance_id {
            return Err(ProjectionError::RepositoryIdentityMismatch);
        }
        if self.has_newer_generation_than(replay) || self.claims_more_journal_bytes_than(replay) {
            return Err(ProjectionError::CacheAhead);
        }
        if self.matches_replay_point(replay) && self.has_different_journal_fingerprint_than(replay)
        {
            return Err(ProjectionError::JournalFingerprintMismatch);
        }
        Ok(())
    }

    fn has_newer_generation_than(&self, replay: &JournalReplay) -> bool {
        self.generation > replay.generation
    }

    fn claims_more_journal_bytes_than(&self, replay: &JournalReplay) -> bool {
        self.journal_end_offset > replay.end_offset
    }

    fn matches_replay_point(&self, replay: &JournalReplay) -> bool {
        let generation_matches = self.generation == replay.generation;
        let byte_offset_matches = self.journal_end_offset == replay.end_offset;
        generation_matches && byte_offset_matches
    }

    fn has_different_journal_fingerprint_than(&self, replay: &JournalReplay) -> bool {
        self.journal_fingerprint != replay.fingerprint
    }
}

/// The cache's observed state at one point in time.
enum ProjectionRead {
    /// A complete projection was present.
    Present(Projection),
    /// No projection has been published yet.
    Missing,
}

/// The publication work required to make a valid projection current.
#[derive(Clone, Copy)]
pub(super) enum ProjectionSynchronization {
    /// The published projection already represents the complete journal replay.
    Current,
    /// The projection is absent or behind and must be rebuilt from the replay.
    RebuildRequired,
}

/// Read the projection once and validate it against the locked journal replay.
pub(super) fn read_validated(
    projection_path: &Path,
    repo_instance_id: RepoInstanceId,
    replay: &JournalReplay,
) -> Result<ProjectionSynchronization, ProjectionError> {
    match read_once(projection_path)? {
        ProjectionRead::Present(projection) => {
            projection.validate_against(repo_instance_id, replay)?;
            if projection.matches_replay_point(replay) {
                Ok(ProjectionSynchronization::Current)
            } else {
                Ok(ProjectionSynchronization::RebuildRequired)
            }
        },
        ProjectionRead::Missing => Ok(ProjectionSynchronization::RebuildRequired),
    }
}

fn read_once(projection_path: &Path) -> Result<ProjectionRead, ProjectionError> {
    let contents = match fs::read(projection_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProjectionRead::Missing);
        },
        Err(error) => return Err(ProjectionError::Io(error)),
    };
    serde_json::from_slice(&contents)
        .map(ProjectionRead::Present)
        .map_err(ProjectionError::Deserialization)
}

fn sync_directory(directory: &Path) -> Result<(), ProjectionError> {
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

/// A projection cache failure that means the journal cannot be trusted through it.
#[derive(Debug)]
pub(crate) enum ProjectionError {
    /// Filesystem access failed.
    Io(std::io::Error),
    /// The projection could not be serialized.
    Serialization(serde_json::Error),
    /// The projection could not be decoded.
    Deserialization(serde_json::Error),
    /// The projection names an unsupported journal schema.
    UnsupportedSchemaVersion(SchemaVersion),
    /// The projection belongs to a different repository instance.
    RepositoryIdentityMismatch,
    /// The projection is ahead of, or inconsistent with, the journal truth.
    CacheAhead,
    /// The projection describes the current generation but not the journal bytes it claims to
    /// cache.
    JournalFingerprintMismatch,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "projection I/O failed: {error}"),
            Self::Serialization(error) => {
                write!(formatter, "could not serialize projection: {error}")
            },
            Self::Deserialization(error) => {
                write!(formatter, "could not decode projection: {error}")
            },
            Self::UnsupportedSchemaVersion(version) => {
                write!(
                    formatter,
                    "projection schema version {version} is unsupported"
                )
            },
            Self::RepositoryIdentityMismatch => {
                formatter.write_str("projection belongs to a different repository instance")
            },
            Self::CacheAhead => formatter.write_str("projection is ahead of the journal"),
            Self::JournalFingerprintMismatch => {
                formatter.write_str("projection fingerprint does not match the current journal")
            },
        }
    }
}

impl std::error::Error for ProjectionError {}

impl From<std::io::Error> for ProjectionError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}
