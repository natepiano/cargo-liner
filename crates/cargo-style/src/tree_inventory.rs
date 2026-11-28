//! Read the complete recursive inventory of a Git tree.

use std::io;
use std::io::Error;
use std::path::Path;

use gix::ObjectId;
use gix::Repository;
use gix::objs::tree::EntryKind;
use gix::traverse::tree::Recorder;
use serde::Serialize;
use serde::Serializer;
use serde::ser::Error as _;

use crate::error::ReaderError;
use crate::git_objects;

type InventoryEntry = (Vec<u8>, String, String, String);

/// The complete recursive inventory of one commit or tree.
#[derive(Serialize)]
pub(crate) struct TreeInventory {
    commit:             String,
    tree:               String,
    #[serde(serialize_with = "serialize_entries")]
    pub(crate) entries: Vec<InventoryEntry>,
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

    pub(crate) fn read_git(repo: &Path, commit: &str) -> Result<Self, ReaderError> {
        let _ = ObjectId::from_hex(commit.as_bytes()).map_err(|source| ReaderError::CommitId {
            commit: commit.to_owned(),
            source,
        })?;
        let tree_error = |source: Error| ReaderError::Tree {
            commit: commit.to_owned(),
            source: Box::new(source),
        };
        let tree = git_objects::read(
            repo,
            &["rev-parse", "--verify", &format!("{commit}^{{tree}}")],
        )
        .and_then(|output| String::from_utf8(output).map_err(io::Error::other))
        .map_err(tree_error)?;
        let listed = git_objects::read(repo, &["ls-tree", "-rzt", "--full-tree", commit])
            .map_err(tree_error)?;
        let entries = listed
            .split(|&byte| byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(listed_entry)
            .collect::<Result<_, _>>()
            .map_err(tree_error)?;
        Ok(Self {
            commit: commit.to_owned(),
            tree: tree.trim_end_matches('\n').to_owned(),
            entries,
        })
    }

    pub(crate) fn into_entries(self) -> (String, Vec<InventoryEntry>) { (self.tree, self.entries) }
}

fn listed_entry(bytes: &[u8]) -> io::Result<InventoryEntry> {
    let invalid = || io::Error::other("unexpected git ls-tree entry");
    let mut fields = bytes.splitn(2, |&byte| byte == b'\t');
    let header =
        std::str::from_utf8(fields.next().unwrap_or_default()).map_err(io::Error::other)?;
    let path = fields.next().ok_or_else(invalid)?;
    let mut parts = header.split(' ');
    let (Some(mode), Some(kind), Some(object), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    Ok((
        path.to_vec(),
        mode.to_owned(),
        kind.to_owned(),
        object.to_owned(),
    ))
}

/// Decode tree paths only when the caller requests a text inventory.
fn serialize_entries<S: Serializer>(
    entries: &[InventoryEntry],
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
