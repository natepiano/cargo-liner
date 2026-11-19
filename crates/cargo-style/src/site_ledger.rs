//! Project the judgment evidence for an original site.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::rc::Rc;

use rusqlite::Connection;
use rusqlite::Row;
use rusqlite::Statement;

use crate::ledger_read;
use crate::ledger_read::LedgerReadFailure;
use crate::ledger_read::OriginalJudgment;
use crate::rule_snapshot::Family;
use crate::rule_snapshot::RuleFamily;
use crate::tip_view;

pub(crate) struct SiteEvidence {
    pub(crate) kind:     FindingKind,
    pub(crate) state:    FindingState,
    pub(crate) rule:     String,
    pub(crate) pin:      String,
    pub(crate) family:   PinnedFamily,
    pub(crate) original: OriginalJudgment,
    pub(crate) holder:   HolderEvidence,
    pub(crate) history:  Rc<[JudgmentHistory]>,
}

pub(crate) enum FindingState {
    Open,
    Closed,
}

pub(crate) enum FindingKind {
    Type { subject: String },
    Other,
}

pub(crate) enum PinnedFamily {
    Known(Family),
    Unspecified,
    Unreadable,
}

pub(crate) enum HolderEvidence {
    InactiveGeneration,
    NoActiveCoveringRow,
    NoCoveringRow,
    CoveringRow {
        row:     Rc<JudgmentRow>,
        history: Rc<[JudgmentHistory]>,
    },
}

pub(crate) struct JudgmentRow {
    pub(crate) path:        String,
    pub(crate) blob:        String,
    pub(crate) fingerprint: String,
    pub(crate) commit:      String,
    pub(crate) verdict:     JudgmentVerdict,
    pub(crate) scope:       InputScope,
    declared:               DeclaredScope,
    pub(crate) reads:       ReadScope,
}

pub(crate) struct JudgmentHistory {
    pub(crate) commit:  String,
    pub(crate) verdict: JudgmentVerdict,
}

pub(crate) enum JudgmentVerdict {
    Removed,
    Judged,
}

impl JudgmentVerdict {
    fn read(row: &Row<'_>, key: &str) -> Result<Self, LedgerReadFailure> {
        let verdict: String = ledger_read::read_column(row, "active_judgments", key, "verdict")?;
        Ok(if verdict == "removed" {
            Self::Removed
        } else {
            Self::Judged
        })
    }

    pub(crate) const fn is_removed(&self) -> bool { matches!(self, Self::Removed) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputScope {
    File,
    Module,
    Crate,
    Manifest,
}

enum DeclaredScope {
    Rule,
    File,
}

pub(crate) enum ReadScope {
    Inferred,
    Explicit(Vec<String>),
}

enum HolderGeneration {
    NoInactiveRows,
    HasInactiveRows,
}

struct HolderRows {
    generation: HolderGeneration,
    rows:       Vec<Rc<JudgmentRow>>,
}

pub(super) struct SiteRows<'a> {
    finding:  Statement<'a>,
    holder:   Statement<'a>,
    inactive: Statement<'a>,
    cell:     Statement<'a>,
    holders:  HashMap<(String, String), HolderRows>,
    cells:    HashMap<(String, String), Rc<[JudgmentHistory]>>,
}

impl<'a> SiteRows<'a> {
    pub(super) fn prepare(connection: &'a Connection) -> Result<Self, LedgerReadFailure> {
        Ok(Self {
            finding: connection.prepare(
                "SELECT h.kind,h.before,h.state,h.rule,h.rule_blob,p.text AS pinned_text \
                 FROM held_findings h LEFT JOIN rule_texts p ON p.blob=h.rule_blob \
                 WHERE h.finding_id=?1",
            )?,
            holder: connection.prepare(
                "SELECT path,rule_blob,input_fingerprint,\"commit\",verdict,input_scope,declared_scope,read_files \
                 FROM active_judgments WHERE rule=?1 AND ticket=?2 ORDER BY recorded_at DESC,id DESC",
            )?,
            inactive: connection.prepare(
                "SELECT EXISTS(SELECT 1 FROM judgments j WHERE j.rule=?1 AND j.ticket=?2 \
                 AND j.verdict!='removed' AND NOT EXISTS \
                 (SELECT 1 FROM active_judgments active WHERE active.id=j.id))",
            )?,
            cell: connection.prepare(
                "SELECT \"commit\",verdict FROM active_judgments \
                 WHERE rule=?1 AND path=?2 ORDER BY recorded_at DESC,id DESC",
            )?,
            holders: HashMap::new(),
            cells: HashMap::new(),
        })
    }

    pub(super) fn read(
        &mut self,
        id: &str,
        ticket: &str,
        path: &str,
        original: OriginalJudgment,
    ) -> Result<SiteEvidence, LedgerReadFailure> {
        let (kind, state, rule, pin, family) = {
            let mut rows = self.finding.query([id])?;
            let row = rows.next()?.ok_or_else(|| {
                ledger_read::invalid_row("held_findings", id, "finding_id", &"missing held finding")
            })?;
            let field =
                |column| ledger_read::read_column::<String>(row, "held_findings", id, column);
            let kind = if field("kind")? == "type" {
                FindingKind::Type {
                    subject: field("before")?,
                }
            } else {
                FindingKind::Other
            };
            let state = match field("state")?.as_str() {
                "unfiled" | "filed" => FindingState::Open,
                "waived" | "built" | "withdrawn" => FindingState::Closed,
                value => {
                    return Err(ledger_read::invalid_row(
                        "held_findings",
                        id,
                        "state",
                        &value,
                    ));
                },
            };
            let rule = field("rule")?;
            let pin = field("rule_blob")?;
            let family = read_family(row, &pin)?;
            (kind, state, rule, pin, family)
        };
        let holder = self.read_holder(&rule, ticket, path)?;
        let history = self.read_cell(&rule, path)?;
        Ok(SiteEvidence {
            kind,
            state,
            rule,
            pin,
            family,
            original,
            holder,
            history,
        })
    }

    fn read_holder(
        &mut self,
        rule: &str,
        ticket: &str,
        path: &str,
    ) -> Result<HolderEvidence, LedgerReadFailure> {
        let holder = match self.holders.entry((rule.to_owned(), ticket.to_owned())) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let mut query = self.holder.query([rule, ticket])?;
                let mut rows = Vec::new();
                while let Some(row) = query.next()? {
                    rows.push(Rc::new(JudgmentRow::read(row, ticket)?));
                }
                let generation = self.inactive.query_row([rule, ticket], |row| {
                    Ok(if row.get(0)? {
                        HolderGeneration::HasInactiveRows
                    } else {
                        HolderGeneration::NoInactiveRows
                    })
                })?;
                entry.insert(HolderRows { generation, rows })
            },
        };
        let Some(row) = holder
            .rows
            .iter()
            .rev()
            .filter(|row| tip_view::under(path, &row.path))
            .max_by_key(|row| row.path.chars().count())
            .cloned()
        else {
            return Ok(match holder.generation {
                HolderGeneration::HasInactiveRows if holder.rows.is_empty() => {
                    HolderEvidence::InactiveGeneration
                },
                HolderGeneration::HasInactiveRows => HolderEvidence::NoActiveCoveringRow,
                HolderGeneration::NoInactiveRows => HolderEvidence::NoCoveringRow,
            });
        };
        let history = self.read_cell(rule, &row.path)?;
        Ok(HolderEvidence::CoveringRow { row, history })
    }

    fn read_cell(
        &mut self,
        rule: &str,
        path: &str,
    ) -> Result<Rc<[JudgmentHistory]>, LedgerReadFailure> {
        let history = match self.cells.entry((rule.to_owned(), path.to_owned())) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let mut query = self.cell.query([rule, path])?;
                let mut history = Vec::new();
                while let Some(row) = query.next()? {
                    history.push(JudgmentHistory {
                        commit:  ledger_read::read_revision(
                            row,
                            "active_judgments",
                            path,
                            "commit",
                        )?,
                        verdict: JudgmentVerdict::read(row, path)?,
                    });
                }
                entry.insert(history.into())
            },
        };
        Ok(Rc::clone(history))
    }
}

fn read_family(row: &Row<'_>, pin: &str) -> Result<PinnedFamily, LedgerReadFailure> {
    let text: Option<Vec<u8>> = ledger_read::read_column(row, "rule_texts", pin, "pinned_text")?;
    Ok(match text.as_deref().map(std::str::from_utf8) {
        Some(Ok(text)) => match Family::from_pin(text) {
            RuleFamily::Known(family) => PinnedFamily::Known(family),
            RuleFamily::Unspecified => PinnedFamily::Unspecified,
        },
        Some(Err(_)) => PinnedFamily::Unreadable,
        None => PinnedFamily::Unspecified,
    })
}

impl JudgmentRow {
    fn read(row: &Row<'_>, ticket: &str) -> Result<Self, LedgerReadFailure> {
        const TABLE: &str = "active_judgments";
        let field = |column| ledger_read::read_column::<String>(row, TABLE, ticket, column);
        let scope = match field("input_scope")?.as_str() {
            "file" => InputScope::File,
            "module" => InputScope::Module,
            "crate" => InputScope::Crate,
            "manifest" => InputScope::Manifest,
            value => {
                return Err(ledger_read::invalid_row(
                    TABLE,
                    ticket,
                    "input_scope",
                    &value,
                ));
            },
        };
        let declared = match field("declared_scope")?.as_str() {
            "rule" => DeclaredScope::Rule,
            "file" => DeclaredScope::File,
            value => {
                return Err(ledger_read::invalid_row(
                    TABLE,
                    ticket,
                    "declared_scope",
                    &value,
                ));
            },
        };
        let reads =
            match field("read_files")?.as_str() {
                "null" => ReadScope::Inferred,
                files => ReadScope::Explicit(serde_json::from_str(files).map_err(|error| {
                    ledger_read::invalid_row(TABLE, ticket, "read_files", &error)
                })?),
            };
        Ok(Self {
            path: field("path")?,
            blob: field("rule_blob")?,
            fingerprint: field("input_fingerprint")?,
            commit: ledger_read::read_revision(row, TABLE, ticket, "commit")?,
            verdict: JudgmentVerdict::read(row, ticket)?,
            scope,
            declared,
            reads,
        })
    }

    pub(crate) const fn effective_scope(&self) -> InputScope {
        match self.declared {
            DeclaredScope::Rule => self.scope,
            DeclaredScope::File => InputScope::File,
        }
    }
}
