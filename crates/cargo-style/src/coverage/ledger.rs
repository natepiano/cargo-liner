//! Read only the ledger rows that can supply coverage.

use std::path::Path;

use rusqlite::Connection;
use rusqlite::Row;
use rusqlite::Statement;

use super::CoverageError;
use crate::ledger_read;
use crate::ledger_read::LedgerReadFailure;
use crate::ledger_read::LedgerSnapshot;
use crate::site_ledger::InputScope;
use crate::site_ledger::ReadScope;

const ACTIVE_JUDGMENTS: &str = "active_judgments";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DeclaredScope {
    Rule,
    File,
}

impl DeclaredScope {
    pub(super) const fn comparison(self, scope: InputScope) -> InputScope {
        match self {
            Self::Rule => scope,
            Self::File => InputScope::File,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    Held,
    Removed,
    Other,
}

pub(super) struct Judgment {
    pub(super) pin:         String,
    pub(super) commit:      String,
    pub(super) fingerprint: String,
    pub(super) scope:       InputScope,
    pub(super) declared:    DeclaredScope,
    pub(super) reads:       ReadScope,
    pub(super) verdict:     Verdict,
}

impl Judgment {
    fn read(row: &Row<'_>) -> Result<Self, LedgerReadFailure> {
        let key = row.get::<_, i64>("id")?.to_string();
        let text = |name| ledger_read::read_column::<String>(row, ACTIVE_JUDGMENTS, &key, name);
        let invalid =
            |name, value: &str| ledger_read::invalid_row(ACTIVE_JUDGMENTS, &key, name, &value);
        let scope = match text("input_scope")?.as_str() {
            "file" => InputScope::File,
            "module" => InputScope::Module,
            "crate" => InputScope::Crate,
            "manifest" => InputScope::Manifest,
            value => return Err(invalid("input_scope", value)),
        };
        let declared = match text("declared_scope")?.as_str() {
            "rule" => DeclaredScope::Rule,
            "file" => DeclaredScope::File,
            value => return Err(invalid("declared_scope", value)),
        };
        let verdict = match text("verdict")?.as_str() {
            "held" => Verdict::Held,
            "removed" => Verdict::Removed,
            "clean" | "fixed" | "exception" | "waived" => Verdict::Other,
            value => return Err(invalid("verdict", value)),
        };
        let reads = match text("read_files")?.as_str() {
            "null" => ReadScope::Inferred,
            serialized => {
                let paths: Vec<String> = serde_json::from_str(serialized).map_err(|error| {
                    ledger_read::invalid_row(ACTIVE_JUDGMENTS, &key, "read_files", &error)
                })?;
                if paths.is_empty() {
                    return Err(invalid("read_files", "expected at least one path"));
                }
                ReadScope::Explicit(paths)
            },
        };
        Ok(Self {
            pin: text("rule_blob")?,
            commit: ledger_read::read_revision(row, ACTIVE_JUDGMENTS, &key, "commit")?,
            fingerprint: text("input_fingerprint")?,
            scope,
            declared,
            reads,
            verdict,
        })
    }
}

pub(super) enum PriorGeneration {
    InactiveJudgment,
    NoInactiveJudgment,
}

#[derive(Clone)]
pub(super) enum StyleBranch {
    Saved(String),
    Unspecified,
}

pub(super) struct SyncPoint {
    pub(super) commit:       String,
    pub(super) style_branch: StyleBranch,
}

pub(super) struct CoverageLedger<'a> {
    pub(super) sync:       SyncPoint,
    judgments:             Statement<'a>,
    inactive:              Statement<'a>,
    pub(super) connection: &'a Connection,
}

impl<'a> CoverageLedger<'a> {
    fn read(connection: &'a Connection, commit: &str) -> Result<Self, LedgerReadFailure> {
        let branch: String =
            connection.query_row("SELECT style_branch FROM sync WHERE id=1", [], |row| {
                row.get(0)
            })?;
        let style_branch = if branch.is_empty() {
            StyleBranch::Unspecified
        } else {
            StyleBranch::Saved(branch)
        };
        Ok(Self {
            sync: SyncPoint { commit: commit.to_owned(), style_branch },
            judgments: connection.prepare(
                "SELECT id,rule_blob,input_fingerprint,\"commit\",verdict,input_scope,declared_scope,read_files \
                 FROM active_judgments WHERE rule=?1 AND path=?2 ORDER BY recorded_at DESC,id DESC",
            )?,
            inactive: connection.prepare(
                "SELECT EXISTS(SELECT 1 FROM judgments j WHERE j.rule=?1 AND j.path=?2 \
                 AND j.verdict!='removed' AND NOT EXISTS \
                 (SELECT 1 FROM active_judgments active WHERE active.id=j.id))",
            )?,
            connection,
        })
    }

    pub(super) fn judgments(
        &mut self,
        rule: &str,
        path: &str,
    ) -> Result<Vec<Judgment>, LedgerReadFailure> {
        let mut result = Vec::new();
        let mut rows = self.judgments.query([rule, path])?;
        while let Some(row) = rows.next()? {
            result.push(Judgment::read(row)?);
        }
        Ok(result)
    }

    pub(super) fn prior_generation(
        &mut self,
        rule: &str,
        path: &str,
    ) -> Result<PriorGeneration, LedgerReadFailure> {
        Ok(
            if self
                .inactive
                .query_row([rule, path], |row| row.get::<_, bool>(0))?
            {
                PriorGeneration::InactiveJudgment
            } else {
                PriorGeneration::NoInactiveJudgment
            },
        )
    }
}

pub(super) fn read_snapshot<T>(
    path: &Path,
    read: impl Fn(CoverageLedger<'_>) -> Result<T, CoverageError>,
) -> Result<T, CoverageError> {
    LedgerSnapshot::read(path, |connection, commit| {
        read(CoverageLedger::read(connection, commit)?)
    })
}
