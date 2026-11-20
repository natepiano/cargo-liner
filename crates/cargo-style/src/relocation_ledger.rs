//! Read relocation evidence from `held_findings` and `tickets`.

use rusqlite::Row;

use crate::ledger_read;
use crate::ledger_read::LedgerReadFailure;

pub(crate) const WITH_ANCHOR: &str = "SELECT h.path, h.line, h.text, h.followed_at, h.anchor_commit, \
    h.ticket, t.id AS judging_ticket, t.checkpoint, t.base, h.state, h.build_ticket, \
    t.state AS judging_state, b.id AS build_id, b.state AS build_state \
    FROM held_findings h LEFT JOIN tickets t ON t.id = h.ticket \
    LEFT JOIN tickets b ON b.id = h.build_ticket \
    WHERE h.finding_id = ?1";

pub(crate) struct RelocationEvidence {
    pub(crate) finding: RelocationFindingState,
    pub(crate) judging: JudgingState,
    pub(crate) build:   LinkedBuild,
}

pub(crate) enum RelocationFindingState {
    Filed,
    Unfiled,
    Closed,
}

pub(crate) enum JudgingState {
    AssignedOrCheckpointed,
    OtherOrMissing,
}

pub(crate) enum LinkedBuild {
    NoOpenBuild,
    Open { ticket: String },
    Missing { ticket: String },
}

impl RelocationEvidence {
    pub(crate) fn read(row: &Row<'_>, id: &str) -> Result<Self, LedgerReadFailure> {
        let finding: String = ledger_read::read_column(row, "held_findings", id, "state")?;
        let finding = match finding.as_str() {
            "filed" => RelocationFindingState::Filed,
            "unfiled" => RelocationFindingState::Unfiled,
            _ => RelocationFindingState::Closed,
        };
        let judging_ticket: String = ledger_read::read_column(row, "held_findings", id, "ticket")?;
        let judging: Option<String> =
            ledger_read::read_column(row, "tickets", &judging_ticket, "judging_state")?;
        let judging = match judging.as_deref() {
            Some("assigned" | "checkpointed") => JudgingState::AssignedOrCheckpointed,
            _ => JudgingState::OtherOrMissing,
        };
        let ticket: String = ledger_read::read_column(row, "held_findings", id, "build_ticket")?;
        let build = read_build(row, ticket)?;
        Ok(Self {
            finding,
            judging,
            build,
        })
    }
}

fn read_build(row: &Row<'_>, ticket: String) -> Result<LinkedBuild, LedgerReadFailure> {
    if ticket.is_empty() {
        return Ok(LinkedBuild::NoOpenBuild);
    }
    let joined: Option<String> = ledger_read::read_column(row, "tickets", &ticket, "build_id")?;
    let Some(ticket) = joined else {
        return Ok(LinkedBuild::Missing { ticket });
    };
    let state: String = ledger_read::read_column(row, "tickets", &ticket, "build_state")?;
    Ok(match state.as_str() {
        "merged" | "dropped" => LinkedBuild::NoOpenBuild,
        _ => LinkedBuild::Open { ticket },
    })
}
