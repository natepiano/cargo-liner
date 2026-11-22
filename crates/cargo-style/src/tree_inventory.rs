//! Read the complete recursive inventory of a Git tree.

use gix::ObjectId;
use gix::Repository;
use gix::objs::tree::EntryKind;
use gix::traverse::tree::Recorder;
use serde::Serialize;
use serde::Serializer;
use serde::ser::Error as _;

use crate::error::ReaderError;

/// The complete recursive inventory of one commit or tree.
#[derive(Serialize)]
pub(crate) struct TreeInventory {
    commit:             String,
    tree:               String,
    #[serde(serialize_with = "serialize_entries")]
    pub(crate) entries: Vec<(Vec<u8>, String, String, String)>,
}

impl TreeInventory {
    pub(crate) fn read(repository: &Repository, commit: &str) -> Result<Self, ReaderError> {
        let id = ObjectId::from_hex(commit.as_bytes()).map_err(|source| ReaderError::CommitId {
            commit: commit.to_owned(),
            source,
        })?;
        let tree_error = |source: Box<dyn std::error::Error + Send + Sync>| ReaderError::Tree {
            commit: commit.to_owned(),
            source,
        };
        let tree = repository
            .find_object(id)
            .map_err(|source| tree_error(Box::new(source)))?
            .peel_to_tree()
            .map_err(|source| tree_error(Box::new(source)))?;
        let mut recorder = Recorder::default();
        tree.traverse()
            .breadthfirst(&mut recorder)
            .map_err(|source| tree_error(Box::new(source)))?;
        let mut entries = Vec::with_capacity(recorder.records.len());
        for entry in recorder.records {
            let path = entry.filepath.to_vec();
            let kind = match entry.mode.kind() {
                EntryKind::Tree => "tree",
                EntryKind::Commit => "commit",
                EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link => "blob",
            };
            entries.push((
                path,
                format!("{:06o}", entry.mode.value()),
                kind.to_owned(),
                entry.oid.to_string(),
            ));
        }
        entries.sort_by_cached_key(|(path, _, kind, _)| {
            if kind == "tree" {
                let mut directory = path.clone();
                directory.push(b'/');
                directory
            } else {
                path.clone()
            }
        });
        Ok(Self {
            commit: commit.to_owned(),
            tree: tree.id.to_string(),
            entries,
        })
    }
}

/// Decode tree paths only when the caller requests a text inventory.
fn serialize_entries<S: Serializer>(
    entries: &[(Vec<u8>, String, String, String)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let decoded: Result<Vec<_>, _> = entries
        .iter()
        .map(|(path, mode, kind, blob)| {
            std::str::from_utf8(path).map(|path| (path, mode, kind, blob))
        })
        .collect();
    decoded.map_err(S::Error::custom)?.serialize(serializer)
}
