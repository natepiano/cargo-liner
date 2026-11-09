//! Compaction markers and native completion records for the local census.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::FixedOffset;
use serde::Deserialize;

use super::activity::Activity;
use super::activity::AgentActivity;
use super::hierarchy::FileRevision;
use super::transcript;
use super::transcript::JsonlCursor;
use super::transcript::RecordChange;
use crate::constants::TRANSCRIPT_TAIL_START;

/// `CompactionFamily` checks the family recorded by the start hook.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum CompactionFamily {
    Claude,
    Codex,
}

/// `CompactionTranscript` names the session file that can close a start marker.
#[derive(Clone, Copy)]
pub(super) enum CompactionTranscript<'a> {
    Unavailable,
    File(&'a Path),
}

/// The marker's state after validation and cached completion checks.
pub(super) enum CompactionMarkerStatus {
    Closed,
    Open { started: DateTime<FixedOffset> },
}

/// Whether a native end record closes the marker's start.
enum CompactionCompletion {
    Pending,
    Complete,
}

/// Whether a transcript line records compaction completion.
enum CompactionLineKind {
    Other,
    End,
}

/// `CompactionDirectory` separates a configured marker folder from an unavailable home.
#[derive(Debug, Default)]
enum CompactionDirectory {
    #[default]
    Unavailable,
    Folder(PathBuf),
}

/// `CompactionMarker` contains only a checked hook start or an absent marker.
#[derive(Clone, Copy, Debug, Default)]
enum CompactionMarker {
    #[default]
    Absent,
    Started {
        family: CompactionFamily,
        at:     DateTime<FixedOffset>,
    },
}

/// `CompactionMarkerPayload` accepts hook fields only at the file boundary.
#[derive(Deserialize)]
struct CompactionMarkerPayload {
    session_id: String,
    family:     CompactionFamily,
    started_at: String,
}

/// `CompactionEnd` retains the latest native completion observed for a session.
#[derive(Clone, Copy, Debug, Default)]
enum CompactionEnd {
    #[default]
    Unrecorded,
    Recorded(DateTime<FixedOffset>),
}

impl CompactionEnd {
    fn closes(self, start: DateTime<FixedOffset>) -> CompactionCompletion {
        match self {
            Self::Recorded(end) if end >= start => CompactionCompletion::Complete,
            Self::Unrecorded | Self::Recorded(_) => CompactionCompletion::Pending,
        }
    }

    fn observe(&mut self, end: DateTime<FixedOffset>) {
        if matches!(self.closes(end), CompactionCompletion::Pending) {
            *self = Self::Recorded(end);
        }
    }
}

/// `CompactionLine` reads completion identity and time without retaining conversation content.
#[derive(Deserialize)]
struct CompactionLine {
    #[serde(rename = "type", default)]
    kind:      String,
    #[serde(default)]
    subtype:   String,
    #[serde(default)]
    timestamp: String,
}

impl CompactionLine {
    fn is_end(&self, family: CompactionFamily) -> CompactionLineKind {
        match (family, self.kind.as_str(), self.subtype.as_str()) {
            (CompactionFamily::Claude, "system", "compact_boundary")
            | (CompactionFamily::Codex, "compacted", _) => CompactionLineKind::End,
            _ => CompactionLineKind::Other,
        }
    }
}

/// `CompactionRead` caches a marker revision and advances its native completion cursor.
#[derive(Debug)]
struct CompactionRead {
    marker_revision:     FileRevision,
    marker:              CompactionMarker,
    transcript_revision: FileRevision,
    cursor:              JsonlCursor,
    completed:           CompactionEnd,
}

/// `CompactionMarkers` reads hook starts and keeps ordinary activity separate from compaction.
#[derive(Debug, Default)]
pub(super) struct CompactionMarkers {
    directory: CompactionDirectory,
    reads:     HashMap<String, CompactionRead>,
    current:   HashSet<String>,
}

impl CompactionMarkers {
    /// `for_current_user` configures the production folder and keeps tests isolated.
    pub(super) fn for_current_user() -> Self {
        #[cfg(not(test))]
        {
            let mut markers = Self::default();
            if let Some(home) = dirs::home_dir() {
                markers.set_directory(home.join(".local/state/compaction"));
            }
            markers
        }
        #[cfg(test)]
        {
            Self::default()
        }
    }

    pub(super) fn set_directory(&mut self, directory: PathBuf) {
        if !matches!(&self.directory, CompactionDirectory::Folder(previous) if previous == &directory)
        {
            self.directory = CompactionDirectory::Folder(directory);
            self.reads.clear();
        }
    }

    pub(super) fn begin_scan(&mut self) { self.current.clear(); }

    pub(super) fn finish_scan(&mut self) {
        self.reads
            .retain(|session, _| self.current.contains(session));
    }

    /// Read the marker and exclude starts closed by cached native completions.
    pub(super) fn marker_status(
        &mut self,
        session: &str,
        family: CompactionFamily,
    ) -> CompactionMarkerStatus {
        let CompactionDirectory::Folder(directory) = &self.directory else {
            return CompactionMarkerStatus::Closed;
        };
        if Path::new(session)
            .file_name()
            .is_none_or(|name| name != session)
        {
            return CompactionMarkerStatus::Closed;
        }
        self.current.insert(session.to_owned());
        let read = self.reads.entry(session.to_owned()).or_default();
        read.read_marker(&directory.join(format!("{session}.json")), session);
        if let CompactionMarker::Started { family: marked, at } = read.marker
            && marked == family
            && matches!(read.completed.closes(at), CompactionCompletion::Pending)
        {
            CompactionMarkerStatus::Open { started: at }
        } else {
            CompactionMarkerStatus::Closed
        }
    }

    pub(super) fn activity(
        &mut self,
        session: &str,
        family: CompactionFamily,
        transcript: CompactionTranscript<'_>,
        ordinary: AgentActivity,
    ) -> AgentActivity {
        let CompactionMarkerStatus::Open { started } = self.marker_status(session, family) else {
            return ordinary;
        };
        let read = self.reads.entry(session.to_owned()).or_default();
        match read.completed(transcript, family, started) {
            CompactionCompletion::Pending => AgentActivity::Active(Activity::Compacting),
            CompactionCompletion::Complete => ordinary,
        }
    }
}

impl CompactionRead {
    const MARKER_BYTE_LIMIT: usize = 4096;

    fn read_marker(&mut self, path: &Path, session: &str) {
        let revision = fs::metadata(path).map_or(FileRevision::Missing, |metadata| {
            FileRevision::of(&metadata)
        });
        if revision == self.marker_revision {
            return;
        }
        self.marker_revision = revision;
        self.marker = CompactionMarker::Absent;
        let Ok(file) = fs::File::open(path) else {
            return;
        };
        let mut bytes = Vec::new();
        if file
            .take((Self::MARKER_BYTE_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() > Self::MARKER_BYTE_LIMIT
        {
            return;
        }
        let Ok(payload) = serde_json::from_slice::<CompactionMarkerPayload>(&bytes) else {
            return;
        };
        if payload.session_id != session {
            return;
        }
        if let Ok(at) = DateTime::parse_from_rfc3339(&payload.started_at) {
            self.marker = CompactionMarker::Started {
                family: payload.family,
                at,
            };
        }
    }

    fn completed(
        &mut self,
        transcript: CompactionTranscript<'_>,
        family: CompactionFamily,
        started: DateTime<FixedOffset>,
    ) -> CompactionCompletion {
        if matches!(
            self.completed.closes(started),
            CompactionCompletion::Complete
        ) {
            return CompactionCompletion::Complete;
        }
        let CompactionTranscript::File(path) = transcript else {
            return CompactionCompletion::Pending;
        };
        let revision = fs::metadata(path).map_or(FileRevision::Missing, |metadata| {
            FileRevision::of(&metadata)
        });
        match transcript::record_change(&self.transcript_revision, &revision) {
            RecordChange::Unchanged => return CompactionCompletion::Pending,
            RecordChange::Replaced => self.cursor = JsonlCursor::NeverRead,
            RecordChange::Appended => {},
        }
        self.transcript_revision = revision;
        let batch = self.cursor.read_until(path, TRANSCRIPT_TAIL_START, |span| {
            span.split(|byte| *byte == b'\n').any(|line| {
                let Ok(record) = serde_json::from_slice::<CompactionLine>(line) else {
                    return false;
                };
                DateTime::parse_from_rfc3339(&record.timestamp).is_ok_and(|at| {
                    at <= started
                        || (matches!(record.is_end(family), CompactionLineKind::End)
                            && at >= started)
                })
            })
        });
        for line in batch.lines {
            if let Ok(record) = serde_json::from_slice::<CompactionLine>(&line)
                && matches!(record.is_end(family), CompactionLineKind::End)
                && let Ok(at) = DateTime::parse_from_rfc3339(&record.timestamp)
            {
                self.completed.observe(at);
            }
        }
        self.completed.closes(started)
    }
}

impl Default for CompactionRead {
    fn default() -> Self {
        Self {
            marker_revision:     FileRevision::Missing,
            marker:              CompactionMarker::Absent,
            transcript_revision: FileRevision::Missing,
            cursor:              JsonlCursor::NeverRead,
            completed:           CompactionEnd::Unrecorded,
        }
    }
}
