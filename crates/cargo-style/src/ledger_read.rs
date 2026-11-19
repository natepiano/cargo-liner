//! Read the ledger rows for finding proofs.

use std::error::Error;
use std::fmt::Display;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use rusqlite::Connection;
use rusqlite::Error as SqliteError;
use rusqlite::Row;
use rusqlite::Statement;
use rusqlite::types::FromSql;
use serde::Serialize;

use crate::constants::LEDGER_SCHEMA_VERSION;
use crate::sqlite_read;
use crate::sqlite_read::DatabaseFiles;
use crate::sqlite_read::DatabaseReadMode;

const LEDGER_READ_ATTEMPTS: usize = 3;

/// One read of the sync row, ledger revision, and requested findings.
pub(crate) struct LedgerSnapshot {
    pub(crate) synced_tip:      String,
    pub(crate) ledger_revision: LedgerRevision,
    pub(crate) findings:        Vec<FindingLookup>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LedgerRevision {
    Absent,
    Revision(u64),
}

pub(crate) enum FindingLookup {
    Held(HeldFinding),
    Unknown,
}

pub(crate) struct HeldFinding {
    pub(crate) anchor: HeldAnchor,
    pub(crate) edit:   FindingEdit,
    pub(crate) needs:  Vec<String>,
}

pub(crate) enum FindingKind {
    Rename,
    Restructure,
    Other,
}

impl From<&str> for FindingKind {
    fn from(kind: &str) -> Self {
        match kind {
            "rename" => Self::Rename,
            "restructure" => Self::Restructure,
            _ => Self::Other,
        }
    }
}

pub(crate) struct FindingEdit {
    pub(crate) kind:   FindingKind,
    pub(crate) before: String,
    pub(crate) after:  String,
}

pub(crate) struct HeldAnchor {
    pub(crate) path:     String,
    pub(crate) line:     usize,
    pub(crate) text:     String,
    pub(crate) evidence: AnchorEvidence,
}

pub(crate) enum AnchorEvidence {
    Followed(String),
    Recorded(String),
    Judging(JudgingRevisions),
    NoJudgingTicket,
}

pub(crate) struct JudgingRevisions(JudgingCommits);

enum JudgingCommits {
    One(String),
    CheckpointAndBase([String; 2]),
}

impl JudgingRevisions {
    pub(crate) const fn candidates(&self) -> &[String] {
        match &self.0 {
            JudgingCommits::One(revision) => std::slice::from_ref(revision),
            JudgingCommits::CheckpointAndBase(revisions) => revisions,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum LedgerReadFailure {
    #[error("missing ledger at {0}")]
    Missing(PathBuf),
    #[error("unsupported ledger schema: {0}")]
    UnsupportedSchema(String),
    #[error("invalid row in {table} at key {key:?}: {detail}")]
    InvalidRow {
        table:  &'static str,
        key:    String,
        detail: String,
    },
    #[error("cannot read the ledger database")]
    Database(#[source] Box<dyn Error + Send + Sync>),
}

impl From<SqliteError> for LedgerReadFailure {
    fn from(error: SqliteError) -> Self { Self::Database(Box::new(error)) }
}

impl From<io::Error> for LedgerReadFailure {
    fn from(error: io::Error) -> Self { Self::Database(Box::new(error)) }
}

impl LedgerSnapshot {
    pub(crate) fn read(path: &Path, ids: &[String]) -> Result<Self, LedgerReadFailure> {
        let absolute = fs::canonicalize(path).map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                LedgerReadFailure::Missing(path.to_owned())
            } else {
                error.into()
            }
        })?;
        for _ in 0..LEDGER_READ_ATTEMPTS {
            let files = DatabaseFiles::read(&absolute)?;
            if files.is_quiet() {
                let read = Self::read_database(&absolute, DatabaseReadMode::Immutable, ids);
                if files == DatabaseFiles::read(&absolute)? {
                    return read;
                }
            } else if files.uses_wal() {
                let directory = tempfile::tempdir()?;
                let copy = directory.path().join("ledger.sqlite");
                let copied = files.copy_wal_database(&absolute, &copy);
                if files == DatabaseFiles::read(&absolute)? {
                    copied?;
                    return Self::read_database(&copy, DatabaseReadMode::Locked, ids);
                }
            } else {
                return Self::read_database(&absolute, DatabaseReadMode::Locked, ids);
            }
        }
        Err(io::Error::other("the ledger files changed during the read").into())
    }

    fn read_database(
        path: &Path,
        mode: DatabaseReadMode,
        ids: &[String],
    ) -> Result<Self, LedgerReadFailure> {
        Self::read_transaction(&mut sqlite_read::open_read_only(path, mode)?, ids)
    }

    fn read_transaction(
        connection: &mut Connection,
        ids: &[String],
    ) -> Result<Self, LedgerReadFailure> {
        let snapshot = connection.transaction()?;
        check_schema(&snapshot)?;
        let synced_tip = read_synced_tip(&snapshot)?;
        let ledger_revision = if has_table(&snapshot, "ledger_revision")? {
            let revision = one_integer(&snapshot, "ledger_revision", "revision")?;
            LedgerRevision::Revision(
                revision
                    .try_into()
                    .map_err(|error| invalid_row("ledger_revision", "1", "revision", &error))?,
            )
        } else {
            LedgerRevision::Absent
        };
        let findings = {
            let mut statement = snapshot.prepare(
                "SELECT h.path, h.line, h.text, h.followed_at, h.anchor_commit, h.ticket, \
                        t.id AS judging_ticket, t.checkpoint, t.base, h.kind, h.before, h.after \
                 FROM held_findings h LEFT JOIN tickets t ON t.id = h.ticket \
                 WHERE h.finding_id = ?1",
            )?;
            let mut needs = snapshot
                .prepare("SELECT path FROM finding_needs WHERE finding_id = ?1 ORDER BY path")?;
            ids.iter()
                .map(|id| read_finding(&mut statement, &mut needs, id))
                .collect::<Result<_, _>>()?
        };
        snapshot.commit()?;
        Ok(Self {
            synced_tip,
            ledger_revision,
            findings,
        })
    }
}

fn has_table(connection: &Connection, table: &str) -> Result<bool, SqliteError> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

fn check_schema(connection: &Connection) -> Result<(), LedgerReadFailure> {
    if !has_table(connection, "schema_version")? {
        return Err(LedgerReadFailure::UnsupportedSchema(
            "no schema_version table".to_owned(),
        ));
    }
    let version = one_integer(connection, "schema_version", "version")?;
    if version != LEDGER_SCHEMA_VERSION {
        return Err(LedgerReadFailure::UnsupportedSchema(version.to_string()));
    }
    Ok(())
}

fn one_integer(
    connection: &Connection,
    table: &'static str,
    column: &'static str,
) -> Result<i64, LedgerReadFailure> {
    let mut statement = connection.prepare(&format!("SELECT {column} FROM {table}"))?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Err(invalid_row(table, "1", column, &"missing row"));
    };
    let value = read_column(row, table, "1", column)?;
    if rows.next()?.is_some() {
        return Err(invalid_row(table, "1", column, &"expected one row"));
    }
    Ok(value)
}

fn read_synced_tip(connection: &Connection) -> Result<String, LedgerReadFailure> {
    let mut statement = connection.prepare("SELECT tip FROM sync WHERE id = 1")?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Err(invalid_row("sync", "1", "tip", &"no synced tip"));
    };
    let tip: String = read_column(row, "sync", "1", "tip")?;
    if !matches!(tip.len(), 40 | 64) || !tip.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_row(
            "sync",
            "1",
            "tip",
            &"expected a full Git object ID",
        ));
    }
    Ok(tip)
}

fn read_finding(
    statement: &mut Statement<'_>,
    needs: &mut Statement<'_>,
    id: &str,
) -> Result<FindingLookup, LedgerReadFailure> {
    let mut rows = statement.query([id])?;
    let Some(row) = rows.next()? else {
        return Ok(FindingLookup::Unknown);
    };
    let path: String = read_column(row, "held_findings", id, "path")?;
    if path.is_empty()
        || path.contains('\0')
        || path.starts_with('/')
        || path.split('/').any(|part| part == "..")
    {
        return Err(invalid_row(
            "held_findings",
            id,
            "path",
            &"expected a path relative to the repository",
        ));
    }
    let stored_line: i64 = read_column(row, "held_findings", id, "line")?;
    let line = usize::try_from(stored_line)
        .ok()
        .filter(|&line| line > 0)
        .ok_or_else(|| {
            invalid_row(
                "held_findings",
                id,
                "line",
                &"expected a positive line number",
            )
        })?;
    let text = read_column(row, "held_findings", id, "text")?;
    let evidence = read_evidence(row, id)?;
    let kind: String = read_column(row, "held_findings", id, "kind")?;
    let edit = FindingEdit {
        kind:   FindingKind::from(kind.as_str()),
        before: read_column(row, "held_findings", id, "before")?,
        after:  read_column(row, "held_findings", id, "after")?,
    };
    let mut paths = needs.query([id])?;
    let mut needed_paths = Vec::new();
    while let Some(row) = paths.next()? {
        needed_paths.push(read_column(row, "finding_needs", id, "path")?);
    }
    Ok(FindingLookup::Held(HeldFinding {
        anchor: HeldAnchor {
            path,
            line,
            text,
            evidence,
        },
        edit,
        needs: needed_paths,
    }))
}

fn read_evidence(row: &Row<'_>, id: &str) -> Result<AnchorEvidence, LedgerReadFailure> {
    let followed = read_revision(row, "held_findings", id, "followed_at")?;
    if !followed.is_empty() {
        return Ok(AnchorEvidence::Followed(followed));
    }
    let recorded = read_revision(row, "held_findings", id, "anchor_commit")?;
    if !recorded.is_empty() {
        return Ok(AnchorEvidence::Recorded(recorded));
    }
    let ticket: String = read_column(row, "held_findings", id, "ticket")?;
    let judging: Option<String> = read_column(row, "tickets", &ticket, "judging_ticket")?;
    if judging.is_some() {
        let checkpoint = read_revision(row, "tickets", &ticket, "checkpoint")?;
        let base = read_revision(row, "tickets", &ticket, "base")?;
        let revisions = match (checkpoint.is_empty(), base.is_empty()) {
            (true, true) => return Ok(AnchorEvidence::NoJudgingTicket),
            (true, false) => JudgingCommits::One(base),
            (false, true) => JudgingCommits::One(checkpoint),
            (false, false) => JudgingCommits::CheckpointAndBase([checkpoint, base]),
        };
        return Ok(AnchorEvidence::Judging(JudgingRevisions(revisions)));
    }
    Ok(AnchorEvidence::NoJudgingTicket)
}

fn read_revision(
    row: &Row<'_>,
    table: &'static str,
    key: &str,
    column: &'static str,
) -> Result<String, LedgerReadFailure> {
    let revision: String = read_column(row, table, key, column)?;
    if revision.contains('\0') {
        return Err(invalid_row(table, key, column, &"expected a Git revision"));
    }
    Ok(revision)
}

fn read_column<T: FromSql>(
    row: &Row<'_>,
    table: &'static str,
    key: &str,
    column: &'static str,
) -> Result<T, LedgerReadFailure> {
    row.get(column)
        .map_err(|error| invalid_row(table, key, column, &error))
}

fn invalid_row(
    table: &'static str,
    key: &str,
    column: &'static str,
    reason: &impl Display,
) -> LedgerReadFailure {
    LedgerReadFailure::InvalidRow {
        table,
        key: key.to_owned(),
        detail: format!("{column}: {reason}"),
    }
}
