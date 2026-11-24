//! Read the synced tip and the legacy listed-site projection.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use rusqlite::Connection;

use super::catalog::Catalog;
use super::catalog::Rule;
use super::constants::SITES_LEDGER_SCHEMA_VERSION;
use crate::constants::LEDGER_SCHEMA_VERSION;
use crate::finding_source::CommitResolution;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::sqlite_read;
use crate::sqlite_read::DatabaseFiles;
use crate::sqlite_read::DatabaseReadMode;

const READ_ATTEMPTS: usize = 3;

#[derive(Debug, thiserror::Error)]
pub(crate) enum SitesLedgerFailure {
    #[error("cannot read the sites ledger")]
    File(#[from] io::Error),
    #[error("cannot query the sites ledger")]
    Query(#[from] rusqlite::Error),
    #[error("unsupported ledger schema {0}")]
    Schema(i64),
    #[error("no synced tip; run set-tip first")]
    NoTip,
    #[error("synced commit {0} is unavailable")]
    UnresolvedTip(String),
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    #[error("the ledger must have one schema version")]
    VersionRows,
}

/// One listed-site coordinate and its display text.
#[derive(Ord, PartialOrd, Eq, PartialEq)]
pub(super) struct ListedCoordinate {
    pub(super) path: String,
    pub(super) line: usize,
    pub(super) text: String,
}

pub(super) enum ListedHistory {
    Enumerate,
    UseLegacy(Vec<ListedCoordinate>),
}

pub(super) struct SitesLedger {
    pub(super) synced_tip: String,
    pub(super) history:    ListedHistory,
}

impl SitesLedger {
    pub(super) fn read(
        path: &Path,
        catalog: &Catalog,
        rule: &Rule,
        source: &mut FindingSource<'_>,
    ) -> Result<Self, SitesLedgerFailure> {
        let path = fs::canonicalize(path)?;
        for _ in 0..READ_ATTEMPTS {
            let files = DatabaseFiles::read(&path)?;
            if files.is_quiet() {
                let read =
                    Self::read_snapshot(&path, DatabaseReadMode::Immutable, catalog, rule, source);
                if files == DatabaseFiles::read(&path)? {
                    return read;
                }
            } else if files.uses_wal() {
                let temporary = tempfile::tempdir()?;
                let copy = temporary.path().join("ledger.sqlite");
                let copied = files.copy_wal_database(&path, &copy);
                if files == DatabaseFiles::read(&path)? {
                    copied?;
                    return Self::read_snapshot(
                        &copy,
                        DatabaseReadMode::Locked,
                        catalog,
                        rule,
                        source,
                    );
                }
            } else {
                return Self::read_snapshot(&path, DatabaseReadMode::Locked, catalog, rule, source);
            }
        }
        Err(io::Error::other("the ledger files changed during the read").into())
    }

    fn read_snapshot(
        path: &Path,
        mode: DatabaseReadMode,
        catalog: &Catalog,
        rule: &Rule,
        source: &mut FindingSource<'_>,
    ) -> Result<Self, SitesLedgerFailure> {
        let mut connection = sqlite_read::open_read_only(path, mode)?;
        let snapshot = connection.transaction()?;
        let version = read_version(&snapshot)?;
        if !matches!(version, LEDGER_SCHEMA_VERSION | SITES_LEDGER_SCHEMA_VERSION) {
            return Err(SitesLedgerFailure::Schema(version));
        }
        let revision: String = snapshot
            .query_row("SELECT tip FROM sync WHERE id = 1", [], |row| row.get(0))
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => SitesLedgerFailure::NoTip,
                error => error.into(),
            })?;
        let CommitResolution::Resolved(synced_tip) = source.resolve_commit(&revision)? else {
            return Err(SitesLedgerFailure::UnresolvedTip(revision));
        };
        let history = if rule.is_regex() && legacy_markers(&snapshot, catalog, synced_tip)? {
            let mut statement = snapshot.prepare(
                "SELECT path,line,text FROM listed_sites WHERE rule=?1 AND tip=?2
                 ORDER BY path,line,text",
            )?;
            ListedHistory::UseLegacy(
                statement
                    .query_map([rule.id.as_str(), synced_tip.as_str()], |row| {
                        let line: i64 = row.get(1)?;
                        Ok(ListedCoordinate {
                            path: row.get(0)?,
                            line: usize::try_from(line)
                                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, line))?,
                            text: row.get(2)?,
                        })
                    })?
                    .collect::<Result<_, _>>()?,
            )
        } else {
            ListedHistory::Enumerate
        };
        snapshot.commit()?;
        Ok(Self {
            synced_tip: synced_tip.clone(),
            history,
        })
    }
}

fn read_version(connection: &Connection) -> Result<i64, SitesLedgerFailure> {
    let mut statement = connection.prepare("SELECT version FROM schema_version")?;
    let mut rows = statement.query([])?;
    let version = rows
        .next()?
        .ok_or(SitesLedgerFailure::VersionRows)?
        .get(0)?;
    if rows.next()?.is_some() {
        return Err(SitesLedgerFailure::VersionRows);
    }
    Ok(version)
}

fn legacy_markers(
    connection: &Connection,
    catalog: &Catalog,
    tip: &str,
) -> Result<bool, rusqlite::Error> {
    let mut statement =
        connection.prepare("SELECT rule,rule_blob FROM listed_rules WHERE tip=?1")?;
    let markers = statement
        .query_map([tip], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<BTreeSet<_>, _>>()?;
    if markers
        .iter()
        .map(|(rule, blob)| (rule.as_str(), blob.as_str()))
        .collect::<BTreeSet<_>>()
        != catalog.pins()
    {
        return Ok(false);
    }
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='refill_candidates')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(true);
    }
    connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM refill_candidates
         WHERE CASE WHEN json_valid(catalog)
               THEN json_extract(catalog,'$[0]')=?1 AND json_extract(catalog,'$[1]')=?2
               ELSE 0 END)",
        [&catalog.checkout, &catalog.global],
        |row| row.get(0),
    )
}
