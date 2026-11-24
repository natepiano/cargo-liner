//! Journal fixtures shared by the unit tests of the compaction submodules.

use std::collections::BTreeSet;
use std::error::Error;
use std::fs;

use serde_json::Value;
use tempfile::TempDir;

use crate::answer::ConflictAuthorization;
use crate::ids::CoordinationRunId;
use crate::ids::EventId;
use crate::ids::ProjectionGeneration;
use crate::ids::RecordedAt;
use crate::ids::RepoInstanceId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::TransactionValidation;
use crate::ledger::handle::Ledger;
use crate::ledger::journal;
use crate::ledger::journal::Journal;
use crate::ledger::journal::JournalEvent;
use crate::ledger::journal::JournalOperation;
use crate::ledger::test_support;

/// The byte length, newline included, of every merge extent observation whose every
/// reservation a later observation names.
pub(super) fn superseded_record_bytes(events: &[JournalEvent]) -> Result<u64, Box<dyn Error>> {
    let mut total = 0;
    for (index, event) in events.iter().enumerate() {
        let Some(reservations) = event.operation.merge_extent_reservations() else {
            continue;
        };
        let named_later = events[index + 1..]
            .iter()
            .filter_map(|later| later.operation.merge_extent_reservations())
            .flatten()
            .collect::<BTreeSet<_>>();
        if reservations.is_subset(&named_later) {
            total += u64::try_from(serde_json::to_vec(event)?.len() + 1)?;
        }
    }
    Ok(total)
}

/// The journal holding `events`, and the byte offset after each record, starting at zero.
pub(super) fn journal_bytes(
    events: &[JournalEvent],
) -> Result<(Vec<u8>, Vec<usize>), serde_json::Error> {
    let mut bytes = Vec::new();
    let mut record_ends = vec![0];
    for event in events {
        bytes.extend(serde_json::to_vec(event)?);
        bytes.push(b'\n');
        record_ends.push(bytes.len());
    }
    Ok((bytes, record_ends))
}

pub(super) fn legacy_extent(
    reservation_id: ReservationId,
) -> Result<JournalOperation, serde_json::Error> {
    operation(&serde_json::json!({
        "op": "merge_extent_observed",
        "reservation_id": reservation_id.to_string(),
        "extent": empty_extent(),
        "run_status": "editing",
    }))
}

pub(super) fn holder_extent(
    reservation_ids: &[ReservationId],
) -> Result<JournalOperation, serde_json::Error> {
    let reservations = reservation_ids
        .iter()
        .map(|reservation_id| {
            serde_json::json!({
                "reservation_id": reservation_id.to_string(),
                "run_status": "editing",
            })
        })
        .collect::<Vec<_>>();
    operation(&serde_json::json!({
        "op": "holder_merge_extent_observed",
        "extent": empty_extent(),
        "reservations": reservations,
    }))
}

pub(super) fn empty_extent() -> Value {
    serde_json::json!({
        "status": "empty",
        "key": {
            "trunk": "1111111111111111111111111111111111111111",
            "head": "2222222222222222222222222222222222222222",
            "working_tree": {"tracked_paths": [], "untracked_paths": []},
        },
    })
}

pub(super) fn finished_history_events()
-> Result<(Vec<JournalEvent>, ReservationId, ReservationId), Box<dyn Error>> {
    let old = "2020-01-01T00:00:00.000Z".parse::<RecordedAt>()?;
    let mut claim = journal::fold_sequence_events()?
        .into_iter()
        .find(|event| matches!(event.operation, JournalOperation::Claim { .. }))
        .ok_or("claim fixture")?;
    claim.at = old;
    let expired = ReservationId::new();
    if let JournalOperation::Claim {
        reservation_id,
        authorization,
        ..
    } = &mut claim.operation
    {
        *reservation_id = expired;
        **authorization = ConflictAuthorization::NoConflict;
    }
    let mut release = claim.clone();
    release.event_id = EventId::new();
    release.operation = operation(
        &serde_json::json!({"op":"release", "reservation_id":expired, "disposition":{"kind":"retired_orphan", "evidence":"finished audit fixture"}}),
    )?;
    let mut live_claim = claim.clone();
    let live = ReservationId::new();
    live_claim.event_id = EventId::new();
    live_claim.at = crate::ids::RecordedAt::now();
    if let JournalOperation::Claim { reservation_id, .. } = &mut live_claim.operation {
        *reservation_id = live;
    }
    let mut events = vec![claim, release, live_claim];
    for (index, event) in events.iter_mut().enumerate() {
        event.projection_generation = ProjectionGeneration::from(u64::try_from(index)? + 1);
    }
    Ok((events, expired, live))
}

pub(super) fn recent_round_trip_events() -> Result<(Vec<JournalEvent>, usize), Box<dyn Error>> {
    let (mut events, rejected) = journal::round_trip_sequence_events()?;
    for event in &mut events {
        event.at = crate::ids::RecordedAt::now();
    }
    Ok((events, rejected))
}

pub(super) fn operation(operation: &Value) -> Result<JournalOperation, serde_json::Error> {
    serde_json::from_value(operation.clone())
}

/// A ledger in a scratch repository whose journal holds chosen events, beside that journal's
/// projection.
pub(super) struct CompactionFixture {
    pub(super) repository:       TempDir,
    pub(super) ledger:           Ledger,
    pub(super) repo_instance_id: RepoInstanceId,
}

impl CompactionFixture {
    /// A ledger whose journal holds `events`, each rewritten to name the ledger's repository.
    pub(super) fn holding(mut events: Vec<JournalEvent>) -> Result<Self, Box<dyn Error>> {
        let repository = test_support::scratch_repository();
        Ledger::initialize(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        let repo_instance_id = ledger.repository_identity()?;
        name_repository(&mut events, repo_instance_id);
        fs::write(&ledger.paths.journal, journal_bytes(&events)?.0)?;
        Ledger::repair_projection(repository.path())?;
        Ok(Self {
            repository,
            ledger,
            repo_instance_id,
        })
    }

    /// A ledger whose journal holds the round-trip sequence before its rejected record,
    /// which supersedes two observations.
    pub(super) fn round_trip() -> Result<Self, Box<dyn Error>> {
        let (events, rejected) = recent_round_trip_events()?;
        Self::holding(events[..rejected].to_vec())
    }

    pub(super) fn journal(&self) -> std::io::Result<Vec<u8>> {
        fs::read(&self.ledger.paths.journal)
    }
}

/// A renewal of the reservation of the latest legacy merge extent observation in `events`,
/// and that observation, whose append supersedes it.
pub(super) fn renewal_and_observation(
    events: &[JournalEvent],
) -> Result<(JournalOperation, JournalOperation), Box<dyn Error>> {
    let (reservation_id, observation) = events
        .iter()
        .rev()
        .find_map(|event| match &event.operation {
            JournalOperation::MergeExtentObserved { reservation_id, .. } => {
                Some((*reservation_id, event.operation.clone()))
            },
            _ => None,
        })
        .ok_or("the events record a legacy observation")?;
    Ok((JournalOperation::Renew { reservation_id }, observation))
}

/// The superseded record bytes of `ledger`'s journal.
pub(super) fn superseded_bytes(ledger: &Ledger) -> Result<u64, Box<dyn Error>> {
    let droppable = Journal::replay_read_only(&ledger.paths.journal)?
        .superseded
        .droppable_bytes;
    Ok(droppable)
}

/// Append `operation` through a ledger transaction and return its event id.
pub(super) fn append(ledger: &Ledger, operation: &JournalOperation) -> Result<EventId, String> {
    match ledger.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
        TransactionValidation::<()>::Append(Box::new(operation.clone()))
    }) {
        Ok(LedgerTransactionOutcome::Appended { event, .. }) => Ok(event.event_id),
        Ok(LedgerTransactionOutcome::Rejected(())) => Err("the append was rejected".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// Rewrite every event to name `repository`, as a journal a compaction accepts does.
pub(super) fn name_repository(events: &mut [JournalEvent], repository: RepoInstanceId) {
    for event in events {
        event.actor.repository = repository;
    }
}
