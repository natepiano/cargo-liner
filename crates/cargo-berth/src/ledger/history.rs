//! The audit window and the dependency closure that keeps expired history still in use.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::btree_map::Entry;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Error;
use serde_json::Value;

use super::journal::JournalEvent;
use super::journal::JournalOperation;
use super::journal::JournalReplay;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::Reservation;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::RetainedReservationSet;

/// Finished audit history remains available for one day, unless a retained fact needs it.
pub(crate) const FINISHED_HISTORY_RETENTION: Duration = Duration::from_hours(24);

/// When reservations entered their current terminal state; evidence refreshes do not renew it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct FinishedHistory(BTreeMap<ReservationId, RecordedAt>);

/// A change to the journal record needed to reconstruct a terminal-state clock.
pub(super) enum HistoryWindowChange {
    /// This record establishes or replaces the terminal-state entry time.
    Started(ReservationId),
    /// The reservation has returned to a nonterminal state.
    Ended(ReservationId),
}

impl FinishedHistory {
    /// Observe only operations that can change terminality, after their reservation fold.
    pub(super) fn observe(
        &mut self,
        event: &JournalEvent,
        reservations: &RetainedReservationSet,
    ) -> Vec<HistoryWindowChange> {
        let ids = match &event.operation {
            JournalOperation::Release { reservation_id, .. }
            | JournalOperation::ReplaceReleaseDisposition { reservation_id, .. }
            | JournalOperation::Checkpoint { reservation_id, .. }
            | JournalOperation::Resnapshot { reservation_id, .. }
            | JournalOperation::MergeExtentObserved { reservation_id, .. } => vec![*reservation_id],
            JournalOperation::HolderMergeExtentObserved { reservations, .. } => reservations
                .as_slice()
                .iter()
                .map(|entry| entry.reservation_id)
                .collect(),
            JournalOperation::Claim { .. }
            | JournalOperation::Widen { .. }
            | JournalOperation::Retarget { .. }
            | JournalOperation::UnrecordedTargetsPinned { .. }
            | JournalOperation::Renew { .. }
            | JournalOperation::EvidenceRevalidated { .. }
            | JournalOperation::ScopedPatchEquivalenceChecked { .. }
            | JournalOperation::ScopedPatchComparisonAttempted { .. }
            | JournalOperation::SuccessorScopedPatchEquivalenceChecked { .. }
            | JournalOperation::SuccessorScopedPatchComparisonAttempted { .. }
            | JournalOperation::Answer { .. }
            | JournalOperation::OverlapRefused { .. }
            | JournalOperation::ResolveDefer { .. }
            | JournalOperation::Sequence { .. }
            | JournalOperation::Incursion { .. }
            | JournalOperation::ResolveIncursion { .. }
            | JournalOperation::ForcedIntegrationPermit { .. }
            | JournalOperation::ConsumeForcedIntegrationPermit { .. }
            | JournalOperation::Bypass { .. }
            | JournalOperation::RebindWorktree { .. }
            | JournalOperation::RelocateWorktree { .. } => Vec::new(),
        };
        let mut changes = Vec::new();
        for id in ids {
            if reservations
                .reservation(id)
                .is_ok_and(Reservation::is_terminal)
            {
                if matches!(
                    event.operation,
                    JournalOperation::Release { .. }
                        | JournalOperation::ReplaceReleaseDisposition { .. }
                ) {
                    self.0.insert(id, event.recorded_at().clone());
                    changes.push(HistoryWindowChange::Started(id));
                } else if let Entry::Vacant(entry) = self.0.entry(id) {
                    entry.insert(event.recorded_at().clone());
                    changes.push(HistoryWindowChange::Started(id));
                }
            } else if self.0.remove(&id).is_some() {
                changes.push(HistoryWindowChange::Ended(id));
            }
        }
        changes
    }

    /// Whether there is audit history old enough to attempt a dependency-checked compaction.
    pub(super) fn is_due(&self, now: &RecordedAt) -> bool {
        self.0
            .values()
            .any(|finished| finished.elapsed_until(now) > FINISHED_HISTORY_RETENTION)
    }
}

/// Expired reservation identities proven unnecessary to the remaining journal.
pub(super) struct ExpiredHistory {
    ids:        BTreeSet<ReservationId>,
    /// The reservation references of each complete record, in journal order.
    references: Vec<BTreeSet<ReservationId>>,
}

impl ExpiredHistory {
    /// Keep all live/recent reservations, open evidence alerts, and their transitive references.
    /// References are read from the complete wire operation, including nested authorizations,
    /// replaced edges, permits and incident holder coverage. Unknown/global facts are kept.
    pub(super) fn of(
        replay: &JournalReplay,
        events: &[JournalEvent],
        kept_extents: &[bool],
        now: &RecordedAt,
    ) -> Result<Self, Error> {
        let Ok(reservations) = &replay.reservations else {
            return Ok(Self {
                ids:        BTreeSet::new(),
                references: Vec::new(),
            });
        };
        let known = reservations
            .iter()
            .map(Reservation::id)
            .collect::<BTreeSet<_>>();
        let mut retained = known
            .iter()
            .copied()
            .filter(|id| {
                replay.finished_history.0.get(id).is_none_or(|finished| {
                    finished.elapsed_until(now) <= FINISHED_HISTORY_RETENTION
                })
            })
            .collect::<BTreeSet<_>>();
        for reservation in reservations.iter() {
            match reservation.evidence_state() {
                // The journal cannot establish worktree liveness. Even an empty extent
                // can leave a checkpoint needed by for_orphaned_outstanding's recovery alert.
                Ok(ReservationEvidenceState::Outstanding { .. }) | Err(_) => {
                    retained.insert(reservation.id());
                },
                Ok(ReservationEvidenceState::Released {
                    disposition,
                    integration_status,
                    ..
                }) if !matches!(
                    disposition.revalidation_subject(),
                    ReleaseRevalidationSubject::None
                ) && !matches!(
                    integration_status,
                    IntegrationEvidenceStatus::Integrated { .. }
                ) =>
                {
                    retained.insert(reservation.id());
                },
                Ok(
                    ReservationEvidenceState::Active { .. }
                    | ReservationEvidenceState::Released { .. }
                    | ReservationEvidenceState::ReleasedWithoutCheckpoint { .. },
                ) => {},
            }
        }
        let mut incident_references = HashMap::new();
        for incident in reservations.incursion_incidents() {
            let refs = reservation_references(&serde_json::to_value(incident)?, &known);
            if matches!(
                incident.status(),
                crate::reservation::IncursionIncidentStatus::Outstanding
            ) {
                retained.extend(&refs);
            }
            incident_references.insert(incident.id(), refs);
        }
        let references = events
            .iter()
            .map(|event| {
                if let JournalOperation::ResolveIncursion { incident_id } = &event.operation {
                    return Ok(incident_references
                        .get(incident_id)
                        .cloned()
                        .unwrap_or_default());
                }
                Ok(reservation_references(
                    &serde_json::to_value(&event.operation)?,
                    &known,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        // Keep the final record to preserve the monotonic generation even if its owner expired.
        if let Some(last) = references.last() {
            retained.extend(last);
        }
        loop {
            let before = retained.len();
            for (refs, keep) in references.iter().zip(kept_extents) {
                if *keep && !refs.is_disjoint(&retained) {
                    retained.extend(refs);
                }
            }
            if retained.len() == before {
                break;
            }
        }
        Ok(Self {
            ids: known.difference(&retained).copied().collect(),
            references,
        })
    }

    pub(super) fn is_empty(&self) -> bool { self.ids.is_empty() }

    pub(super) fn keeps(&self, ordinal: usize) -> bool {
        let refs = &self.references[ordinal];
        refs.is_empty() || !refs.is_subset(&self.ids)
    }

    /// The expected retained fold. Unresolved incidents already seeded the dependency closure.
    pub(super) fn prune(&self, replay: &mut JournalReplay) -> Result<(), Error> {
        if let Ok(reservations) = &mut replay.reservations {
            let mut value = serde_json::to_value(&*reservations)?;
            for field in ["reservations", "incursion_incidents", "answer_records"] {
                if let Some(entries) = value.get_mut(field).and_then(Value::as_array_mut) {
                    entries.retain(|entry| {
                        let refs = reservation_references(entry, &self.ids);
                        refs.is_empty()
                    });
                }
            }
            *reservations = serde_json::from_value(value)?;
        }
        let known = self.ids.clone();
        replay.coordination_events.retain(|event| {
            serde_json::to_value(&event.operation)
                .is_ok_and(|value| reservation_references(&value, &known).is_empty())
        });
        replay
            .finished_history
            .0
            .retain(|id, _| !self.ids.contains(id));
        Ok(())
    }
}

/// Conservatively recognize every retained reservation identity anywhere in a wire value.
fn reservation_references(
    value: &Value,
    known: &BTreeSet<ReservationId>,
) -> BTreeSet<ReservationId> {
    let mut references = BTreeSet::new();
    match value {
        Value::String(text) => {
            if let Ok(id) = text.parse::<ReservationId>()
                && known.contains(&id)
            {
                references.insert(id);
            }
        },
        Value::Array(values) => {
            for value in values {
                references.extend(reservation_references(value, known));
            }
        },
        Value::Object(fields) => {
            for value in fields.values() {
                references.extend(reservation_references(value, known));
            }
        },
        Value::Null | Value::Bool(_) | Value::Number(_) => {},
    }
    references
}
