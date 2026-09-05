//! The disposable generation-stamped projection of the append-only journal.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;

use super::constants::CURRENT_PROJECTION_SCHEMA_VERSION;
use super::constants::PROJECTION_TEMPORARY_FILE_NAME;
use super::journal::JournalFingerprint;
use super::journal::JournalReplay;
use crate::ids::JournalByteOffset;
use crate::ids::ProjectionGeneration;
use crate::ids::RepoInstanceId;
use crate::ids::SchemaVersion;

/// The version field that determines whether this binary can decode a projection cache.
#[derive(Deserialize)]
struct ProjectionSchemaHeader {
    schema_version: SchemaVersion,
}

/// The serialized cache reconstructed solely from the journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Projection {
    /// The projection schema used to create this cache.
    schema_version:      SchemaVersion,
    /// The clone identity that owns this ledger.
    repo_instance_id:    RepoInstanceId,
    /// The byte length of the journal represented here.
    journal_end_offset:  JournalByteOffset,
    /// The generation published by the last journal mutation.
    generation:          ProjectionGeneration,
    /// A digest that detects journal changes without trusting cache contents.
    journal_fingerprint: JournalFingerprint,
}

impl Projection {
    /// Derive a projection from a complete replay.
    pub(super) fn from_replay(repo_instance_id: RepoInstanceId, replay: &JournalReplay) -> Self {
        Self {
            schema_version: SchemaVersion::from(CURRENT_PROJECTION_SCHEMA_VERSION),
            repo_instance_id,
            journal_end_offset: replay.end_offset,
            generation: replay.generation,
            journal_fingerprint: replay.fingerprint,
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
        validate_projection_schema_version(self.schema_version)?;
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

    fn uses_current_schema(&self) -> bool {
        self.schema_version == SchemaVersion::from(CURRENT_PROJECTION_SCHEMA_VERSION)
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
///
/// An unsupported projection schema requires a rebuild because the journal replay is independent
/// of this disposable cache. Malformed projection bytes and repository identity mismatches remain
/// errors because they do not establish that the file is a projection for this repository.
pub(super) fn read_validated(
    projection_path: &Path,
    repo_instance_id: RepoInstanceId,
    replay: &JournalReplay,
) -> Result<ProjectionSynchronization, ProjectionError> {
    match read_once(projection_path) {
        Ok(ProjectionRead::Present(projection)) => {
            projection.validate_against(repo_instance_id, replay)?;
            if projection.matches_replay_point(replay) && projection.uses_current_schema() {
                Ok(ProjectionSynchronization::Current)
            } else {
                Ok(ProjectionSynchronization::RebuildRequired)
            }
        },
        Ok(ProjectionRead::Missing) | Err(ProjectionError::UnsupportedSchemaVersion(_)) => {
            Ok(ProjectionSynchronization::RebuildRequired)
        },
        Err(error) => Err(error),
    }
}

fn read_once(projection_path: &Path) -> Result<ProjectionRead, ProjectionError> {
    let contents = match fs::read(projection_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(ProjectionRead::Missing);
        },
        Err(error) => return Err(ProjectionError::Io(error)),
    };
    let schema_header = serde_json::from_slice::<ProjectionSchemaHeader>(&contents)
        .map_err(ProjectionError::Deserialization)?;
    validate_projection_schema_version(schema_header.schema_version)?;
    serde_json::from_slice(&contents)
        .map(ProjectionRead::Present)
        .map_err(ProjectionError::Deserialization)
}

fn validate_projection_schema_version(
    schema_version: SchemaVersion,
) -> Result<(), ProjectionError> {
    if schema_version != SchemaVersion::from(CURRENT_PROJECTION_SCHEMA_VERSION) {
        return Err(ProjectionError::UnsupportedSchemaVersion(schema_version));
    }
    Ok(())
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
    /// The projection names an unsupported cache schema.
    UnsupportedSchemaVersion(SchemaVersion),
    /// The projection belongs to a different repository instance.
    RepositoryIdentityMismatch,
    /// The projection is ahead of, or inconsistent with, the journal truth.
    CacheAhead,
    /// The projection describes the current generation but not the journal bytes it claims to
    /// cache.
    JournalFingerprintMismatch,
}

impl Display for ProjectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
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
            Self::CacheAhead => formatter.write_str(
                "projection is ahead of the journal; run cargo-berth init --repair-projection to rebuild only the cache from journal truth",
            ),
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

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::ProjectionError;
    use super::read_once;
    use crate::ids::SchemaVersion;
    use crate::ledger::constants::CURRENT_PROJECTION_SCHEMA_VERSION;
    use crate::ledger::constants::PROJECTION_FILE_NAME;

    #[test]
    fn unsupported_schema_precedes_full_projection_decoding()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary_directory = tempdir()?;
        let projection_path = temporary_directory.path().join(PROJECTION_FILE_NAME);
        let unsupported_schema_version = CURRENT_PROJECTION_SCHEMA_VERSION + 1;
        fs::write(
            &projection_path,
            format!(r#"{{"schema_version":{unsupported_schema_version}}}"#),
        )?;

        assert!(matches!(
            read_once(&projection_path),
            Err(ProjectionError::UnsupportedSchemaVersion(version))
                if version == SchemaVersion::from(unsupported_schema_version)
        ));
        Ok(())
    }
}
