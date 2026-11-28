//! Read the synced tip and the legacy listed-site projection.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::Connection;
use rusqlite::Error;

use super::catalog::Catalog;
use super::catalog::Rule;
use crate::finding_source::CommitResolution;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::ledger_read::LedgerReadFailure;
use crate::ledger_read::LedgerSnapshot;

#[derive(Debug, thiserror::Error)]
pub(crate) enum SitesLedgerFailure {
    #[error(transparent)]
    Snapshot(#[from] LedgerReadFailure),
    #[error("cannot query the sites ledger")]
    Query(#[from] Error),
    #[error(
        "UNIQUE constraint failed: listed_rules.rule, listed_rules.rule_blob, listed_rules.tip"
    )]
    DuplicateListedRule,
    #[error("synced commit {0} is unavailable")]
    UnresolvedTip(String),
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
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
        LedgerSnapshot::read(path, |connection, revision| {
            let CommitResolution::Resolved(synced_tip) = source.resolve_commit(revision)? else {
                return Err(SitesLedgerFailure::UnresolvedTip(revision.to_owned()));
            };
            let history = if rule.listing.enumerates_sites()
                && matches!(
                    listing_source(connection, catalog, synced_tip)?,
                    ListedMembership::KeepRows
                ) {
                let mut statement = connection.prepare(
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
                                    .map_err(|_| Error::IntegralValueOutOfRange(1, line))?,
                                text: row.get(2)?,
                            })
                        })?
                        .collect::<Result<_, _>>()?,
                )
            } else {
                ListedHistory::Enumerate
            };
            Ok(Self {
                synced_tip: synced_tip.clone(),
                history,
            })
        })
    }
}

pub(super) enum ListedMembership {
    KeepRows,
    Enumerate,
}

pub(super) fn listing_source(
    connection: &Connection,
    catalog: &Catalog,
    tip: &str,
) -> Result<ListedMembership, Error> {
    let mut statement =
        connection.prepare("SELECT rule,rule_blob FROM listed_rules WHERE tip=?1")?;
    let markers = statement
        .query_map([tip], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<BTreeSet<_>, _>>()?;
    let pins: BTreeSet<_> = catalog.listing_pins().collect();
    if markers.is_empty() && pins.is_empty() {
        return Ok(ListedMembership::KeepRows);
    }
    if !markers
        .iter()
        .map(|(rule, blob)| (rule.as_str(), blob.as_str()))
        .eq(pins.iter().copied())
    {
        return Ok(ListedMembership::Enumerate);
    }
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='refill_candidates')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(ListedMembership::KeepRows);
    }
    let uncached = connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM refill_candidates
         WHERE CASE WHEN json_valid(catalog)
               THEN json_extract(catalog,'$[0]')=?1 AND json_extract(catalog,'$[1]')=?2
               ELSE 0 END)",
        [&catalog.checkout, &catalog.global],
        |row| row.get(0),
    )?;
    Ok(if uncached {
        ListedMembership::KeepRows
    } else {
        ListedMembership::Enumerate
    })
}

pub(super) fn validate_listing_keys(catalog: &Catalog) -> Result<(), SitesLedgerFailure> {
    let mut pins = BTreeSet::new();
    if catalog.listing_pins().any(|pin| !pins.insert(pin)) {
        return Err(SitesLedgerFailure::DuplicateListedRule);
    }
    Ok(())
}
