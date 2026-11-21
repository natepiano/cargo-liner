//! Finds the Rust blobs of commits and reads their facts on every CPU.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::env;
use std::io::Write;
use std::path::Path;

use gix::Blob;
use gix::ObjectId;
use gix::Repository;
use gix::ThreadSafeRepository;
use gix::odb::pack::index;
use gix::odb::pack::multi_index;
use gix::odb::store::structure::IndexState;
use gix::odb::store::structure::Record;
use rayon::prelude::*;
use serde_json::Error as JsonError;

use crate::cache_constants::PARSE_COUNT_ENV;
use crate::constants::BLOB_BATCH_SIZE;
use crate::error::ReaderError;
use crate::fact_store::CacheFailure;
use crate::fact_store::CachedFact;
use crate::fact_store::FactCache;
use crate::fact_store::NewFactRow;
use crate::facts::BlobSource;
use crate::facts::FactReading;
use crate::module_sites::ModuleSitesQuery;
use crate::request::FactKind;
use crate::request::NameWordClass;
use crate::tree_inventory::TreeInventory;

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
            .map(|commit| TreeInventory::read(&repository, commit))
            .collect()
    }

    pub(crate) fn existing_fact_cache(&self) -> FactCache {
        FactCache::open_existing(self.repository.to_thread_local().common_dir())
    }

    pub(crate) fn source(
        &self,
        blob: &str,
        cache: &mut FactCache,
    ) -> Result<BlobSource, ReaderError> {
        let id =
            ObjectId::from_hex(blob.as_bytes()).map_err(|source| ReaderError::QueryBlobId {
                blob: blob.to_owned(),
                source,
            })?;
        let mut repository = self.repository.to_thread_local();
        repository.objects.ignore_replacements = true;
        if let CachedFact::Found(encoded) = cache.lookup(id) {
            match validated_cached_blob(&repository, id, &encoded)? {
                Ok(reading) => return Ok(reading.into_source()),
                Err(error) => cache.disable(CacheFailure::from(error)),
            }
        }
        Ok(
            read_blob_object(&repository, id)?.map_or(BlobSource::Missing, |blob| {
                std::str::from_utf8(&blob.data).map_or(BlobSource::NotUtf8, |text| {
                    BlobSource::Utf8(text.to_owned())
                })
            }),
        )
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
        if all.is_empty() {
            return Ok(());
        }
        let repository = self.repository.to_thread_local();
        let mut cache = FactCache::open(repository.common_dir());
        let mut parsed = 0;
        for batch in all.chunks(BLOB_BATCH_SIZE) {
            let lookups: Vec<_> = batch
                .iter()
                .map(|&blob| (blob, cache.lookup(blob)))
                .collect();
            let lines = lookups
                .par_iter()
                .map_init(
                    || {
                        let mut repository = self.repository.to_thread_local();
                        repository.objects.ignore_replacements = true;
                        repository
                    },
                    |repository, (blob, cached)| {
                        let kinds = if requested.contains(blob) { kinds } else { &[] };
                        let queries = grouped.get(blob).map_or(&[][..], Vec::as_slice);
                        read_and_render(repository, *blob, cached, kinds, queries, word_class)
                    },
                )
                .collect::<Result<Vec<_>, _>>()?;
            let mut rows = Vec::new();
            for line in lines {
                match line.outcome {
                    BlobReadOutcome::NotParsed => {},
                    BlobReadOutcome::Parsed => parsed += 1,
                    BlobReadOutcome::ParsedWithRow(row) => {
                        parsed += 1;
                        rows.push(row);
                    },
                    BlobReadOutcome::NotParsedWithCacheFailure(reason) => cache.disable(reason),
                    BlobReadOutcome::ParsedWithCacheFailure(reason) => {
                        parsed += 1;
                        cache.disable(reason);
                    },
                }
                output
                    .write_all(line.json.as_bytes())
                    .map_err(ReaderError::Write)?;
            }
            cache.store_batch(&rows);
        }
        cache.finish();
        report_parse_count(parsed);
        Ok(())
    }
}

/// An output line group and any cache update from reading its blob.
struct BlobOutput {
    json:    String,
    outcome: BlobReadOutcome,
}

enum BlobReadOutcome {
    NotParsed,
    Parsed,
    ParsedWithRow(NewFactRow),
    NotParsedWithCacheFailure(CacheFailure),
    ParsedWithCacheFailure(CacheFailure),
}

fn read_and_render(
    repository: &Repository,
    blob: ObjectId,
    cached: &CachedFact,
    kinds: &[FactKind],
    queries: &[&ModuleSitesQuery],
    word_class: NameWordClass,
) -> Result<BlobOutput, ReaderError> {
    let serialize_error = |source| ReaderError::Serialize { blob, source };
    let cache_validation = match cached {
        CachedFact::Found(encoded) => match validated_cached_blob(repository, blob, encoded)? {
            Ok(reading) => {
                return Ok(BlobOutput {
                    json:    reading
                        .render(blob, kinds, queries, word_class)
                        .map_err(serialize_error)?,
                    outcome: BlobReadOutcome::NotParsed,
                });
            },
            Err(error) => Err(CacheFailure::from(error)),
        },
        CachedFact::Missing | CachedFact::Unavailable => Ok(()),
    };

    let reading = read_blob(repository, blob)?;
    let outcome = match (&reading, cache_validation, cached) {
        (FactReading::MissingBlob, Err(reason), _) => {
            BlobReadOutcome::NotParsedWithCacheFailure(reason)
        },
        (FactReading::MissingBlob, Ok(()), _) => BlobReadOutcome::NotParsed,
        (_, Err(reason), _) => BlobReadOutcome::ParsedWithCacheFailure(reason),
        (_, Ok(()), CachedFact::Missing) => match NewFactRow::encode(blob, &reading) {
            Ok(row) => BlobReadOutcome::ParsedWithRow(row),
            Err(reason) => BlobReadOutcome::ParsedWithCacheFailure(reason),
        },
        (_, Ok(()), _) => BlobReadOutcome::Parsed,
    };
    Ok(BlobOutput {
        json: reading
            .render(blob, kinds, queries, word_class)
            .map_err(serialize_error)?,
        outcome,
    })
}

fn validated_cached_blob(
    repository: &Repository,
    blob: ObjectId,
    encoded: &str,
) -> Result<Result<FactReading, JsonError>, ReaderError> {
    let header = repository
        .try_find_header(blob)
        .map_err(|source| ReaderError::Blob {
            blob,
            source: Box::new(source),
        })?;
    if header.is_some() {
        Ok(serde_json::from_str(encoded))
    } else {
        check_unloaded_pack_indexes(repository, blob)?;
        Ok(Ok(FactReading::MissingBlob))
    }
}

fn read_blob(repository: &Repository, blob: ObjectId) -> Result<FactReading, ReaderError> {
    let Some(object) = read_blob_object(repository, blob)? else {
        return Ok(FactReading::MissingBlob);
    };
    FactReading::parse(&object.data).map_err(|source| ReaderError::Serialize { blob, source })
}

fn read_blob_object(
    repository: &Repository,
    blob: ObjectId,
) -> Result<Option<Blob<'_>>, ReaderError> {
    let object = repository
        .try_find_object(blob)
        .map_err(|source| ReaderError::Blob {
            blob,
            source: Box::new(source),
        })?;
    let Some(object) = object else {
        check_unloaded_pack_indexes(repository, blob)?;
        return Ok(None);
    };
    object
        .try_into_blob()
        .map(Some)
        .map_err(|source| ReaderError::Blob {
            blob,
            source: Box::new(source),
        })
}

fn check_unloaded_pack_indexes(repository: &Repository, blob: ObjectId) -> Result<(), ReaderError> {
    // gix can skip an unreadable index and report a missing object.
    let store = repository.objects.store_ref();
    let metrics = store.metrics();
    if metrics.open_reachable_indices == metrics.known_reachable_indices {
        return Ok(());
    }

    let read_error = |source| ReaderError::Blob {
        blob,
        source: Box::new(source),
    };
    for record in store.structure().map_err(read_error)? {
        match record {
            Record::Index {
                path,
                state: IndexState::Unloaded,
            } => {
                let _ = index::File::at(path, repository.object_hash()).map_err(read_error)?;
            },
            Record::MultiIndex {
                path,
                state: IndexState::Unloaded,
            } => {
                let _ = multi_index::File::at(path, None).map_err(read_error)?;
            },
            _ => {},
        }
    }
    Ok(())
}

fn report_parse_count(parsed: usize) {
    if cfg!(debug_assertions) && env::var_os(PARSE_COUNT_ENV).is_some_and(|value| value == "1") {
        eprintln!("cargo-style: parsed {parsed} blobs");
    }
}
