//! Read ticket membership and batch companions in the finding snapshot.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::rc::Rc;

use rusqlite::CachedStatement;
use rusqlite::Connection;
use rusqlite::Error as SqliteError;
use rusqlite::Row;
use rusqlite::params;
use serde_json::Error as JsonError;

use crate::facts;
use crate::ledger_read;
use crate::ledger_read::LedgerReadFailure;
use crate::rename_admission::BindingRename;
use crate::rename_admission::OwnedRename;

pub(crate) enum BuildComparison {
    NotBinding,
    NoBase,
    MissingTicket(String),
    Ticket {
        ticket: Rc<TicketComparison>,
        rename: OwnedRename,
    },
}

pub(crate) struct TicketComparison {
    pub(crate) id:      String,
    pub(crate) base:    String,
    pub(crate) members: Vec<TicketRename>,
}

pub(crate) struct TicketRename {
    pub(crate) kind:   String,
    pub(crate) before: String,
    pub(crate) after:  String,
    pub(crate) line:   usize,
    pub(crate) text:   String,
}

pub(crate) struct AppliedTickets<'a> {
    connection:  &'a Connection,
    target:      &'a str,
    comparisons: BTreeMap<(String, String), Rc<TicketComparison>>,
}

struct EligibleTicket {
    id:         String,
    base:       String,
    branch:     String,
    checkpoint: String,
    rules:      String,
}

enum Projection {
    DirectTicket,
    LinkedTicket,
    Members,
}

impl Projection {
    fn prepare<'connection>(
        &self,
        connection: &'connection Connection,
    ) -> Result<CachedStatement<'connection>, LedgerReadFailure> {
        let sql = match self {
            Self::DirectTicket => {
                "SELECT id, base, branch, checkpoint, rules FROM tickets WHERE id = ?1"
            },
            Self::LinkedTicket => {
                "SELECT id, base, branch, checkpoint, rules FROM tickets t \
                 WHERE state = 'merged' AND checkpoint = ?2 AND base <> '' \
                   AND EXISTS (SELECT 1 FROM json_each(t.finding_ids) f WHERE f.value = ?1) \
                 ORDER BY id LIMIT 1"
            },
            Self::Members => {
                "SELECT h.finding_id, h.kind, h.before, h.after, h.line, h.text \
                 FROM tickets t JOIN json_each(t.finding_ids) f \
                 JOIN held_findings h ON h.finding_id = f.value \
                 WHERE h.path = ?1 AND (t.id = ?2 OR (\
                     ?3 AND t.id IN (SELECT ticket FROM bulk_workspace_tickets WHERE batch = \
                         (SELECT batch FROM bulk_workspace_tickets WHERE ticket = ?2)) \
                     AND t.state IN ('checkpointed', 'merged') AND t.base = ?4 \
                     AND t.branch = ?5 AND t.checkpoint = ?6 AND t.rules = ?7)) \
                 ORDER BY t.id, h.finding_id"
            },
        };
        connection
            .prepare_cached(sql)
            .map_err(|source| self.failure(source))
    }

    const fn failure(&self, source: SqliteError) -> LedgerReadFailure {
        let projection = match self {
            Self::DirectTicket => "direct build ticket",
            Self::LinkedTicket => "linked build ticket",
            Self::Members => "build ticket members",
        };
        LedgerReadFailure::TicketProjection { projection, source }
    }
}

impl<'a> AppliedTickets<'a> {
    pub(crate) const fn new(connection: &'a Connection, target: &'a str) -> Self {
        Self {
            connection,
            target,
            comparisons: BTreeMap::new(),
        }
    }

    pub(crate) fn read(
        &mut self,
        finding: &str,
        path: &str,
        build_ticket: &str,
        rename: &BindingRename<'_>,
    ) -> Result<BuildComparison, LedgerReadFailure> {
        let projection = if build_ticket.is_empty() {
            Projection::LinkedTicket
        } else {
            Projection::DirectTicket
        };
        let ticket = {
            let mut statement = projection.prepare(self.connection)?;
            let rows = if build_ticket.is_empty() {
                statement.query(params![finding, self.target])
            } else {
                statement.query([build_ticket])
            };
            let mut rows = rows.map_err(|source| projection.failure(source))?;
            let Some(row) = rows.next().map_err(|source| projection.failure(source))? else {
                return Ok(match projection {
                    Projection::LinkedTicket => BuildComparison::NoBase,
                    _ => BuildComparison::MissingTicket(build_ticket.to_owned()),
                });
            };
            EligibleTicket::read(row, &projection)?
        };
        if ticket.base.is_empty() {
            return Ok(BuildComparison::NoBase);
        }
        let key = (ticket.id.clone(), path.to_owned());
        let comparison = match self.comparisons.entry(key) {
            Entry::Occupied(entry) => Rc::clone(entry.get()),
            Entry::Vacant(entry) => {
                let at_checkpoint = ticket.checkpoint == self.target;
                let rules = if at_checkpoint {
                    ticket.serialized_rules()?
                } else {
                    String::new()
                };
                let mut statement = Projection::Members.prepare(self.connection)?;
                let mut rows = statement
                    .query(params![
                        path,
                        ticket.id,
                        at_checkpoint,
                        ticket.base,
                        ticket.branch,
                        self.target,
                        rules
                    ])
                    .map_err(|source| Projection::Members.failure(source))?;
                let mut members = Vec::new();
                while let Some(row) = rows
                    .next()
                    .map_err(|source| Projection::Members.failure(source))?
                {
                    members.push(TicketRename::read(row)?);
                }
                let comparison = Rc::new(TicketComparison {
                    id: ticket.id,
                    base: ticket.base,
                    members,
                });
                Rc::clone(entry.insert(comparison))
            },
        };
        Ok(BuildComparison::Ticket {
            ticket: comparison,
            rename: OwnedRename::from(rename),
        })
    }
}

impl EligibleTicket {
    fn read(row: &Row<'_>, projection: &Projection) -> Result<Self, LedgerReadFailure> {
        let id: String = row.get("id").map_err(|source| projection.failure(source))?;
        let field = |name| ledger_read::read_column(row, "tickets", &id, name);
        let base = field("base")?;
        let branch = field("branch")?;
        let checkpoint = field("checkpoint")?;
        let rules = field("rules")?;
        Ok(Self {
            id,
            base,
            branch,
            checkpoint,
            rules,
        })
    }

    fn serialized_rules(&self) -> Result<String, LedgerReadFailure> {
        let invalid =
            |error: JsonError| ledger_read::invalid_row("tickets", &self.id, "rules", &error);
        let rules: Vec<String> = serde_json::from_str(&self.rules).map_err(invalid)?;
        let parts = rules
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(invalid)?;
        facts::python_ascii_json(&format!("[{}]", parts.join(", "))).map_err(invalid)
    }
}

impl TicketRename {
    fn read(row: &Row<'_>) -> Result<Self, LedgerReadFailure> {
        let id: String = row
            .get("finding_id")
            .map_err(|source| Projection::Members.failure(source))?;
        let field = |name| ledger_read::read_column(row, "held_findings", &id, name);
        Ok(Self {
            kind:   field("kind")?,
            before: field("before")?,
            after:  field("after")?,
            line:   ledger_read::read_finding_line(row, &id)?,
            text:   field("text")?,
        })
    }
}
