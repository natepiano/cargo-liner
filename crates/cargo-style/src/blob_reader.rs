//! Finds the Rust blobs of commits and reads their facts on every CPU.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

use gix::ObjectId;
use gix::ThreadSafeRepository;
use gix::objs::tree::EntryKind;
use gix::traverse::tree::Recorder;
use rayon::prelude::*;
use serde::Serialize;
use serde::Serializer;
use serde::ser::Error as _;

use crate::constants::BLOB_BATCH_SIZE;
use crate::error::ReaderError;
use crate::facts;
use crate::module_sites::ModuleSitesQuery;
use crate::request::FactKind;
use crate::request::NameWordClass;

/// One repository, shared by the reader threads.
pub(crate) struct BlobReader {
    repository: ThreadSafeRepository,
}

impl BlobReader {
    pub(crate) fn open(path: &Path) -> Result<Self, ReaderError> {
        let repository = gix::open(path).map_err(|source| ReaderError::Open {
            path: path.to_owned(),
            source,
        })?;
        Ok(Self {
            repository: repository.into_sync(),
        })
    }

    /// Read each tree once, including directories, links, and submodules.
    pub(crate) fn inventories(
        &self,
        commits: &[String],
    ) -> Result<Vec<TreeInventory>, ReaderError> {
        let repository = self.repository.to_thread_local();
        commits
            .iter()
            .map(|commit| {
                let id = ObjectId::from_hex(commit.as_bytes()).map_err(|source| {
                    ReaderError::CommitId {
                        commit: commit.clone(),
                        source,
                    }
                })?;
                let tree_error =
                    |source: Box<dyn std::error::Error + Send + Sync>| ReaderError::Tree {
                        commit: commit.clone(),
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
                Ok(TreeInventory {
                    commit: commit.clone(),
                    tree: tree.id.to_string(),
                    entries,
                })
            })
            .collect()
    }

    /// Write all facts and queries in bounded batches, including queried blobs outside the commits.
    pub(crate) fn write_request(
        &self,
        blobs: &[ObjectId],
        kinds: &[FactKind],
        queries: &[ModuleSitesQuery],
        word_class: NameWordClass,
        output: &mut impl Write,
    ) -> Result<(), ReaderError> {
        let mut grouped: BTreeMap<ObjectId, Vec<&ModuleSitesQuery>> = BTreeMap::new();
        for query in queries {
            let id = ObjectId::from_hex(query.blob.as_bytes()).map_err(|source| {
                ReaderError::QueryBlobId {
                    blob: query.blob.clone(),
                    source,
                }
            })?;
            grouped.entry(id).or_default().push(query);
        }
        let all: Vec<_> = blobs
            .iter()
            .copied()
            .chain(grouped.keys().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let requested: BTreeSet<_> = blobs.iter().copied().collect();
        all.chunks(BLOB_BATCH_SIZE).try_for_each(|batch| {
            let lines: Result<Vec<_>, _> = batch
                .par_iter()
                .map_init(
                    || self.repository.to_thread_local(),
                    |repository, &blob| {
                        let blob_kinds = if requested.contains(&blob) {
                            kinds
                        } else {
                            &[]
                        };
                        let blob_queries = grouped.get(&blob).map_or(&[][..], Vec::as_slice);
                        let object = repository.try_find_object(blob).map_err(|source| {
                            ReaderError::Blob {
                                blob,
                                source: Box::new(source),
                            }
                        })?;
                        let Some(object) = object else {
                            return facts::missing_readings(blob, blob_kinds, blob_queries)
                                .map_err(|source| ReaderError::Serialize { blob, source });
                        };
                        let object =
                            object.try_into_blob().map_err(|source| ReaderError::Blob {
                                blob,
                                source: Box::new(source),
                            })?;
                        facts::blob_readings(
                            blob,
                            &object.data,
                            blob_kinds,
                            blob_queries,
                            word_class,
                        )
                        .map_err(|source| ReaderError::Serialize { blob, source })
                    },
                )
                .collect();
            lines?.iter().try_for_each(|line| {
                output
                    .write_all(line.as_bytes())
                    .map_err(ReaderError::Write)
            })
        })
    }
}

/// The complete recursive inventory of one commit or tree.
#[derive(Serialize)]
pub(crate) struct TreeInventory {
    commit:             String,
    tree:               String,
    #[serde(serialize_with = "serialize_entries")]
    pub(crate) entries: Vec<(Vec<u8>, String, String, String)>,
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
