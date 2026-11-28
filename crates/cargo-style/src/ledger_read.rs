//! Read the ledger rows that a finding proof needs.

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
use rusqlite::types::FromSql;
use serde::Serialize;

use crate::applied_tickets::AppliedTickets;
use crate::applied_tickets::BuildComparison;
use crate::binding;
use crate::binding::Pair;
use crate::constants::LEDGER_SCHEMA_VERSIONS;
use crate::relocation_ledger::RelocationEvidence;
use crate::site_ledger::SiteEvidence;
use crate::site_ledger::SiteRows;
use crate::sqlite_read;
use crate::sqlite_read::DatabaseFiles;
use crate::sqlite_read::DatabaseReadMode;

const LEDGER_READ_ATTEMPTS: usize = 3;

/// One read of the sync row, ledger revision, requested findings and the rows a request reads
/// beside them.
pub(crate) struct LedgerSnapshot<T, B = ()> {
    pub(crate) synced_tip:      String,
    pub(crate) ledger_revision: LedgerRevision,
    pub(crate) findings:        Vec<FindingLookup<T>>,
    pub(crate) beside:          B,
}

/// One held finding in state `built`, as its built rename reads it.
pub(crate) struct BuiltRow {
    pub(crate) id:       String,
    pub(crate) path:     String,
    pub(crate) line:     usize,
    pub(crate) edit:     FindingEdit,
    pub(crate) proposal: String,
    pub(crate) follow:   FindingFollow,
    pub(crate) history:  FindingSourceHistory,
}

type FindingRead<T, B> = (Vec<FindingLookup<T>>, B);

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LedgerRevision {
    Absent,
    Revision(u64),
}

pub(crate) enum FindingLookup<T> {
    Held(T),
    Unknown,
}

pub(crate) struct HeldSite {
    pub(crate) finding:    HeldRename,
    pub(crate) site:       SiteEvidence,
    pub(crate) relocation: RelocationEvidence,
}

pub(crate) struct HeldRename {
    pub(crate) anchor:     HeldAnchor,
    pub(crate) edit:       FindingEdit,
    pub(crate) needs:      Vec<String>,
    pub(crate) rule:       String,
    pub(crate) rule_blob:  String,
    pub(crate) ticket:     String,
    pub(crate) follow:     FindingFollow,
    pub(crate) history:    FindingSourceHistory,
    pub(crate) comparison: BuildComparison,
}

pub(crate) struct FindingEdit {
    pub(crate) kind:   FindingKind,
    pub(crate) before: String,
    pub(crate) after:  String,
}

pub(crate) enum FindingKind {
    Rename,
    Restructure,
    Type,
    Other(String),
}

impl FindingKind {
    pub(crate) const fn as_str(&self) -> &str {
        match self {
            Self::Rename => "rename",
            Self::Restructure => "restructure",
            Self::Type => "type",
            Self::Other(kind) => kind.as_str(),
        }
    }
}

impl From<String> for FindingKind {
    fn from(kind: String) -> Self {
        match kind.as_str() {
            "rename" => Self::Rename,
            "restructure" => Self::Restructure,
            "type" => Self::Type,
            _ => Self::Other(kind),
        }
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum FindingFollow {
    NotFollowed {},
    Followed {
        commit:    String,
        from_line: i64,
        from_path: String,
    },
}

#[derive(Serialize)]
pub(crate) struct FindingSourceHistory {
    anchor_commit:        String,
    judging:              Vec<[String; 2]>,
    previous_site_commit: String,
}

/// Whether a finding's judging ticket names a commit.
pub(crate) enum JudgingHistory {
    Recorded,
    Empty,
}

impl FindingSourceHistory {
    fn read(row: &Row<'_>, id: &str, judging: [String; 2]) -> Result<Self, LedgerReadFailure> {
        let table = "held_findings";
        Ok(Self {
            anchor_commit:        read_revision(row, table, id, "anchor_commit")?,
            judging:              if judging.iter().all(String::is_empty) {
                Vec::new()
            } else {
                vec![judging]
            },
            previous_site_commit: read_revision(row, table, id, "followed_from_commit")?,
        })
    }

    pub(crate) fn anchor_commit(&self) -> &str { &self.anchor_commit }

    pub(crate) fn previous_site_commit(&self) -> &str { &self.previous_site_commit }

    pub(crate) const fn judging(&self) -> JudgingHistory {
        if self.judging.is_empty() {
            JudgingHistory::Empty
        } else {
            JudgingHistory::Recorded
        }
    }

    pub(crate) fn judging_commits(&self) -> impl Iterator<Item = &str> {
        self.judging
            .iter()
            .flatten()
            .map(String::as_str)
            .filter(|commit| !commit.is_empty())
    }
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

pub(crate) enum OriginalJudgment {
    Recorded(String),
    Anchor,
    PriorTicket(Vec<String>),
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
    #[error("cannot read the {projection} projection from the ledger database")]
    TicketProjection {
        projection: &'static str,
        source:     SqliteError,
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

impl LedgerSnapshot<HeldRename> {
    pub(crate) fn read_renames(path: &Path, ids: &[String]) -> Result<Self, LedgerReadFailure> {
        Self::read_findings_snapshot(path, |connection, tip| {
            let mut tickets = AppliedTickets::new(connection, tip);
            let findings = read_findings(connection, ids, |row, id| {
                read_rename(connection, row, id, &mut tickets)
            })?;
            Ok((findings, ()))
        })
    }
}

impl LedgerSnapshot<HeldSite, Vec<BuiltRow>> {
    pub(crate) fn read_sites(path: &Path, ids: &[String]) -> Result<Self, LedgerReadFailure> {
        Self::read_findings_snapshot(path, |connection, tip| {
            let mut tickets = AppliedTickets::new(connection, tip);
            let mut sites = SiteRows::prepare(connection)?;
            let findings = read_findings(connection, ids, |row, id| {
                let finding = read_rename(connection, row, id, &mut tickets)?;
                let original = read_original(row, id, &finding.ticket, &finding.anchor.evidence)?;
                let site = sites.read(id, &finding.ticket, &finding.anchor.path, original)?;
                let relocation = RelocationEvidence::read(row, id)?;
                Ok(HeldSite {
                    finding,
                    site,
                    relocation,
                })
            })?;
            Ok((findings, read_built(connection)?))
        })
    }
}

impl<T, B> LedgerSnapshot<T, B> {
    fn read_findings_snapshot(
        path: &Path,
        findings: impl Fn(&Connection, &str) -> Result<FindingRead<T, B>, LedgerReadFailure>,
    ) -> Result<Self, LedgerReadFailure> {
        LedgerSnapshot::read(path, |connection, synced_tip| {
            let ledger_revision = if has_table(connection, "ledger_revision")? {
                let revision = one_integer(connection, "ledger_revision", "revision")?;
                LedgerRevision::Revision(
                    revision
                        .try_into()
                        .map_err(|error| invalid_row("ledger_revision", "1", "revision", &error))?,
                )
            } else {
                LedgerRevision::Absent
            };
            let (findings, beside) = findings(connection, synced_tip)?;
            Ok(Self {
                synced_tip: synced_tip.to_owned(),
                ledger_revision,
                findings,
                beside,
            })
        })
    }
}

impl LedgerSnapshot<()> {
    pub(crate) fn read<R, E: From<LedgerReadFailure>>(
        path: &Path,
        mut read: impl FnMut(&Connection, &str) -> Result<R, E>,
    ) -> Result<R, E> {
        let absolute = fs::canonicalize(path).map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                LedgerReadFailure::Missing(path.to_owned())
            } else {
                error.into()
            }
        })?;
        for _ in 0..LEDGER_READ_ATTEMPTS {
            let files = DatabaseFiles::read(&absolute).map_err(LedgerReadFailure::from)?;
            if files.is_quiet() {
                let result = Self::read_database(&absolute, DatabaseReadMode::Immutable, &mut read);
                if files == DatabaseFiles::read(&absolute).map_err(LedgerReadFailure::from)? {
                    return result;
                }
            } else if files.uses_wal() {
                let directory = tempfile::tempdir().map_err(LedgerReadFailure::from)?;
                let copy = directory.path().join("ledger.sqlite");
                let copied = files.copy_wal_database(&absolute, &copy);
                if files == DatabaseFiles::read(&absolute).map_err(LedgerReadFailure::from)? {
                    copied.map_err(LedgerReadFailure::from)?;
                    return Self::read_database(&copy, DatabaseReadMode::Locked, &mut read);
                }
            } else {
                return Self::read_database(&absolute, DatabaseReadMode::Locked, &mut read);
            }
        }
        Err(
            LedgerReadFailure::from(io::Error::other("the ledger files changed during the read"))
                .into(),
        )
    }

    fn read_database<R, E: From<LedgerReadFailure>>(
        path: &Path,
        mode: DatabaseReadMode,
        read: &mut impl FnMut(&Connection, &str) -> Result<R, E>,
    ) -> Result<R, E> {
        let mut connection =
            sqlite_read::open_read_only(path, mode).map_err(LedgerReadFailure::from)?;
        let snapshot = connection.transaction().map_err(LedgerReadFailure::from)?;
        check_schema(&snapshot)?;
        let synced_tip = read_synced_tip(&snapshot)?;
        let result = read(&snapshot, &synced_tip)?;
        snapshot.commit().map_err(LedgerReadFailure::from)?;
        Ok(result)
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
    if !LEDGER_SCHEMA_VERSIONS.contains(&version) {
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

fn read_findings<T>(
    connection: &Connection,
    ids: &[String],
    mut read: impl FnMut(&Row<'_>, &str) -> Result<T, LedgerReadFailure>,
) -> Result<Vec<FindingLookup<T>>, LedgerReadFailure> {
    let mut statement = connection.prepare(
        "SELECT h.path, h.line, h.text, h.followed_at, h.anchor_commit, h.ticket, \
                h.rule, h.rule_blob, h.kind, h.before, h.after, h.followed_from, \
                h.followed_from_line, h.followed_from_commit, h.build_ticket, h.state, h.proposal, \
                t.id AS judging_ticket, t.checkpoint, t.base, t.state AS judging_state, \
                b.id AS build_id, b.state AS build_state \
         FROM held_findings h LEFT JOIN tickets t ON t.id = h.ticket \
         LEFT JOIN tickets b ON b.id = h.build_ticket \
         WHERE h.finding_id = ?1",
    )?;
    ids.iter()
        .map(|id| {
            let mut rows = statement.query([id])?;
            rows.next()?.map_or(Ok(FindingLookup::Unknown), |row| {
                read(row, id).map(FindingLookup::Held)
            })
        })
        .collect()
}

fn read_rename(
    connection: &Connection,
    row: &Row<'_>,
    id: &str,
    tickets: &mut AppliedTickets<'_>,
) -> Result<HeldRename, LedgerReadFailure> {
    let anchor = read_anchor(row, id)?;
    let table = "held_findings";
    let ticket: String = read_column(row, table, id, "ticket")?;
    let followed = read_revision(row, table, id, "followed_at")?;
    let follow = FindingFollow::read(row, id, followed)?;
    let history = FindingSourceHistory::read(row, id, read_judging(row, &ticket)?)?;
    let edit = FindingEdit::read(row, id)?;
    let comparison = match binding::read_pair(
        edit.kind.as_str(),
        &edit.before,
        &edit.after,
        anchor.line,
        &anchor.text,
    ) {
        Pair::Local(rename) => {
            let build_ticket: String = read_column(row, table, id, "build_ticket")?;
            tickets.read(id, &anchor.path, &build_ticket, &rename)?
        },
        Pair::NonLocal => BuildComparison::NotBinding,
    };
    Ok(HeldRename {
        anchor,
        edit,
        needs: read_needs(connection, id)?,
        rule: read_column(row, table, id, "rule")?,
        rule_blob: read_column(row, table, id, "rule_blob")?,
        ticket,
        follow,
        history,
        comparison,
    })
}

impl FindingFollow {
    fn read(row: &Row<'_>, id: &str, followed: String) -> Result<Self, LedgerReadFailure> {
        let table = "held_findings";
        Ok(if followed.is_empty() {
            Self::NotFollowed {}
        } else {
            Self::Followed {
                commit:    followed,
                from_line: read_column(row, table, id, "followed_from_line")?,
                from_path: read_column(row, table, id, "followed_from")?,
            }
        })
    }
}

impl FindingEdit {
    fn read(row: &Row<'_>, id: &str) -> Result<Self, LedgerReadFailure> {
        let table = "held_findings";
        let kind: String = read_column(row, table, id, "kind")?;
        Ok(Self {
            kind:   kind.into(),
            before: read_column(row, table, id, "before")?,
            after:  read_column(row, table, id, "after")?,
        })
    }
}

fn read_judging(row: &Row<'_>, ticket: &str) -> Result<[String; 2], LedgerReadFailure> {
    let checkpoint: Option<String> = read_column(row, "tickets", ticket, "checkpoint")?;
    let base: Option<String> = read_column(row, "tickets", ticket, "base")?;
    Ok([checkpoint.unwrap_or_default(), base.unwrap_or_default()])
}

/// The built findings in Python `_held_rows` order.
fn read_built(connection: &Connection) -> Result<Vec<BuiltRow>, LedgerReadFailure> {
    let mut statement = connection.prepare(
        "SELECT h.finding_id, h.path, h.line, h.kind, h.before, h.after, h.proposal, \
                h.ticket, h.followed_at, h.followed_from, h.followed_from_line, \
                h.followed_from_commit, h.anchor_commit, t.checkpoint, t.base \
         FROM held_findings h LEFT JOIN tickets t ON t.id = h.ticket \
         WHERE h.state = 'built' ORDER BY h.created_at, h.finding_id",
    )?;
    let mut rows = statement.query([])?;
    let mut built = Vec::new();
    while let Some(row) = rows.next()? {
        let table = "held_findings";
        let id: String = read_column(row, table, "built", "finding_id")?;
        let ticket: String = read_column(row, table, &id, "ticket")?;
        let judging = read_judging(row, &ticket)?;
        built.push(BuiltRow {
            path: read_column(row, table, &id, "path")?,
            line: read_finding_line(row, &id)?,
            edit: FindingEdit::read(row, &id)?,
            proposal: read_column(row, table, &id, "proposal")?,
            follow: FindingFollow::read(row, &id, read_revision(row, table, &id, "followed_at")?)?,
            history: FindingSourceHistory::read(row, &id, judging)?,
            id,
        });
    }
    Ok(built)
}

fn read_needs(connection: &Connection, id: &str) -> Result<Vec<String>, LedgerReadFailure> {
    let sql = if has_table(connection, "blocked_finding_needs")? {
        "SELECT path FROM finding_needs WHERE finding_id = ?1 \
         UNION SELECT path FROM blocked_finding_needs WHERE finding_id = ?1 ORDER BY path"
    } else {
        "SELECT path FROM finding_needs WHERE finding_id = ?1 ORDER BY path"
    };
    let mut statement = connection.prepare_cached(sql)?;
    let mut rows = statement.query([id])?;
    let mut paths = Vec::new();
    while let Some(row) = rows.next()? {
        paths.push(read_column(row, "finding_needs", id, "path")?);
    }
    Ok(paths)
}

fn read_anchor(row: &Row<'_>, id: &str) -> Result<HeldAnchor, LedgerReadFailure> {
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
    let line = read_finding_line(row, id)?;
    let text = read_column(row, "held_findings", id, "text")?;
    let evidence = read_evidence(row, id)?;
    Ok(HeldAnchor {
        path,
        line,
        text,
        evidence,
    })
}

pub(crate) fn read_finding_line(row: &Row<'_>, id: &str) -> Result<usize, LedgerReadFailure> {
    let stored_line: i64 = read_column(row, "held_findings", id, "line")?;
    usize::try_from(stored_line)
        .ok()
        .filter(|&line| line > 0)
        .ok_or_else(|| {
            invalid_row(
                "held_findings",
                id,
                "line",
                &"expected a positive line number",
            )
        })
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

fn read_original(
    row: &Row<'_>,
    id: &str,
    ticket: &str,
    anchor: &AnchorEvidence,
) -> Result<OriginalJudgment, LedgerReadFailure> {
    match anchor {
        AnchorEvidence::Recorded(commit) => Ok(OriginalJudgment::Recorded(commit.clone())),
        AnchorEvidence::Judging(_) | AnchorEvidence::NoJudgingTicket => {
            Ok(OriginalJudgment::Anchor)
        },
        AnchorEvidence::Followed(_) => {
            let recorded = read_revision(row, "held_findings", id, "anchor_commit")?;
            if !recorded.is_empty() {
                return Ok(OriginalJudgment::Recorded(recorded));
            }
            let mut commits = Vec::new();
            for column in ["checkpoint", "base"] {
                let commit: Option<String> = read_column(row, "tickets", ticket, column)?;
                if let Some(commit) = commit.filter(|commit| !commit.is_empty()) {
                    commits.push(commit);
                }
            }
            Ok(OriginalJudgment::PriorTicket(commits))
        },
    }
}

pub(crate) fn read_revision(
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

pub(crate) fn read_column<T: FromSql>(
    row: &Row<'_>,
    table: &'static str,
    key: &str,
    column: &'static str,
) -> Result<T, LedgerReadFailure> {
    row.get(column)
        .map_err(|error| invalid_row(table, key, column, &error))
}

pub(crate) fn invalid_row(
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
