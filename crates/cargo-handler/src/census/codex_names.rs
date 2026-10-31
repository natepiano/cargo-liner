//! Names Codex records for its own sessions, read once and then by appended lines.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use super::codex::CodexThread;
use super::transcript::JsonlCursor;
use super::tree;
use crate::constants::CODEX_SESSION_INDEX_FILENAME;

/// A recorded user name, distinct from the pid fallback chosen by the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SessionName {
    Unnamed,
    Named(String),
}

impl From<Option<String>> for SessionName {
    fn from(name: Option<String>) -> Self {
        name.map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|name| !name.is_empty())
            .map_or(Self::Unnamed, Self::Named)
    }
}

/// A file generation prevents replaced or truncated indexes retaining stale names.
#[derive(Debug, Eq, PartialEq)]
struct IndexGeneration {
    device: u64,
    inode:  u64,
}

#[derive(Debug, Default)]
struct SessionIndex {
    cursor:     JsonlCursor,
    names:      HashMap<String, String>,
    generation: IndexRead,
    bytes_read: u64,
}

#[derive(Debug, Default)]
enum IndexRead {
    #[default]
    Unread,
    Following(IndexGeneration),
}

#[derive(Deserialize)]
struct NameRecord {
    id:          String,
    thread_name: String,
}

/// Separate caches for the separate Codex homes; names never cross accounts.
#[derive(Debug, Default)]
pub(super) struct SessionNames(HashMap<PathBuf, SessionIndex>);

impl SessionNames {
    /// Read the complete index initially, then only appended complete records.
    pub(super) fn read(&mut self, codex_home: &Path) {
        let index = self.0.entry(codex_home.to_path_buf()).or_default();
        let path = codex_home.join(CODEX_SESSION_INDEX_FILENAME);
        let Ok(metadata) = fs::metadata(&path) else {
            *index = SessionIndex::default();
            return;
        };
        let generation = IndexGeneration {
            device: metadata.dev(),
            inode:  metadata.ino(),
        };
        let same_file =
            matches!(&index.generation, IndexRead::Following(previous) if *previous == generation);
        let truncated = matches!(&index.cursor, JsonlCursor::Following { offset, .. } if *offset > metadata.len());
        if !same_file || truncated {
            *index = SessionIndex::default();
        }
        // Opening separately makes an unreadable index clear the previous names too.
        if fs::File::open(&path).is_err() {
            *index = SessionIndex::default();
            return;
        }
        index.generation = IndexRead::Following(generation);
        let batch = index
            .cursor
            .read_until(&path, metadata.len().max(1), |_| true);
        index.bytes_read = batch.bytes_read;
        for line in batch.lines {
            if let Ok(record) = serde_json::from_slice::<NameRecord>(&line) {
                if record.thread_name.trim().is_empty() {
                    index.names.remove(&record.id);
                } else {
                    index.names.insert(
                        record.id,
                        record
                            .thread_name
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                }
            }
        }
    }

    /// The index name from this thread's account, else its database name.
    pub(super) fn name(&self, thread: &CodexThread) -> SessionName {
        tree::thread_id(&thread.rollout)
            .and_then(|id| self.0.get(thread.account_home.as_ref())?.names.get(&id))
            .map_or_else(
                || thread.name.clone(),
                |name| SessionName::Named(name.clone()),
            )
    }

    /// File traffic from the most recent read, for incremental-reader regressions.
    #[cfg(test)]
    pub(super) fn bytes_read(&self, codex_home: &Path) -> u64 {
        self.0.get(codex_home).map_or(0, |index| index.bytes_read)
    }
}
