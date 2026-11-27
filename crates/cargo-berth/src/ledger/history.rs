//! The audit window and the dependency closure that keeps expired history still in use.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::btree_map::Entry;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Error;
use serde_json::Value;

use super::journal::JournalEvent;
use super::journal::JournalOperation;
use super::journal::JournalReplay;
use crate::git;
use crate::git::CommitCandidateReachability;
use crate::git::CommitTargetReachability;
use crate::git::ReferenceLookup;
use crate::git::RepositorySnapshot;
use crate::ids::GitObjectId;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::KeptWork;
use crate::reservation::RecordedTarget;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::Reservation;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::ReservationLifecycle;
use crate::reservation::RetainedProtectedTip;
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
            | JournalOperation::IntegrateAbsentHolder { reservation_id, .. }
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
            | JournalOperation::RetireCyclicOrderingEdge { .. }
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

    #[cfg(test)]
    pub(super) fn has_expired_record(&self, id: ReservationId, now: &RecordedAt) -> bool {
        self.0
            .get(&id)
            .is_some_and(|finished| finished.elapsed_until(now) > FINISHED_HISTORY_RETENTION)
    }

    /// Expired reservations that do not directly protect evidence.
    pub(super) fn compaction_candidates<'a>(
        replay: &'a JournalReplay,
        now: &'a RecordedAt,
        repository_root: &'a Path,
    ) -> impl Iterator<Item = ReservationId> + 'a {
        replay.reservations.iter().flat_map(move |reservations| {
            reservations.iter().filter_map(move |reservation| {
                let id = reservation.id();
                let finished = replay.finished_history.0.get(&id)?;
                (finished.elapsed_until(now) > FINISHED_HISTORY_RETENTION
                    && !keeps_evidence(reservation, reservations, repository_root))
                .then_some(id)
            })
        })
    }
}

/// Whether compaction must keep the reservation's evidence.
fn keeps_evidence(
    reservation: &Reservation,
    reservations: &RetainedReservationSet,
    repository_root: &Path,
) -> bool {
    if let ReservationLifecycle::Released {
        disposition: ReleaseDisposition::KeptOnBranch(kept),
    } = reservation.lifecycle()
    {
        match &kept.work {
            KeptWork::Known(tip)
                if !kept_work_can_expire(repository_root, reservation, tip, reservations) =>
            {
                return true;
            },
            KeptWork::Unknown => return true,
            KeptWork::HoldTransfer | KeptWork::Known(_) => {},
        }
    }
    match reservation.evidence_state() {
        // An outstanding checkpoint may be needed for an orphan alert.
        Ok(ReservationEvidenceState::Outstanding { .. }) | Err(_) => true,
        Ok(ReservationEvidenceState::Released {
            disposition,
            integration_status,
            ..
        }) => {
            !matches!(
                disposition.revalidation_subject(),
                ReleaseRevalidationSubject::None
            ) && !matches!(
                integration_status,
                IntegrationEvidenceStatus::Integrated { .. }
            )
        },
        Ok(ReservationEvidenceState::IntegrationVerifiedWithoutCheckpoint {
            integration_status,
            ..
        }) => !matches!(
            integration_status,
            IntegrationEvidenceStatus::Integrated { .. }
        ),
        Ok(
            ReservationEvidenceState::Active { .. }
            | ReservationEvidenceState::ReleasedWithoutCheckpoint { .. },
        ) => false,
    }
}

/// Expire kept work only when each live holder uses the same target and Git proves landing.
fn kept_work_can_expire(
    repository_root: &Path,
    reservation: &Reservation,
    tip: &GitObjectId,
    reservations: &RetainedReservationSet,
) -> bool {
    let RecordedTarget::Recorded { target, .. } = reservation.target() else {
        return false;
    };
    if reservations.iter().any(|holder| {
        !matches!(holder.lifecycle(), ReservationLifecycle::Released { .. })
            && !matches!(holder.target(), RecordedTarget::Recorded { target: other, .. } if other == target)
    }) {
        return false;
    }
    if !matches!(
        git::reference_lookup(repository_root, &git::kept_work_ref_name(reservation.id())),
        Ok(ReferenceLookup::Missing)
    ) {
        return false;
    }
    // An absent ref can also result from manual deletion. Check the target independently.
    let Ok(repository) = RepositorySnapshot::discover(repository_root) else {
        return false;
    };
    let mut tips = vec![tip.clone()];
    if let RetainedProtectedTip::Retained(checkpoint) = reservation.protected_checkpoint()
        && checkpoint.as_ref() != tip
    {
        tips.push(checkpoint.as_ref().clone());
    }
    git::branch_commit_reachability(&repository, target.short_name(), &tips)
        .is_ok_and(|observation| {
            matches!(
                observation.reachability,
                CommitTargetReachability::Resolved { candidates, .. }
                    if candidates.iter().all(|candidate| *candidate == CommitCandidateReachability::Ancestor)
            )
        })
}

/// The retention facts used by a compaction plan.
#[derive(Debug, Deserialize, Serialize)]
pub(super) struct HistoryCompactionBasis {
    /// The expired reservations that could be removed if no dependency keeps them.
    pub(super) candidates:  BTreeSet<ReservationId>,
    /// The candidates that the final record can keep as the generation anchor.
    pub(super) final_roots: BTreeSet<ReservationId>,
}

impl HistoryCompactionBasis {
    /// Check that appends keep the roots from the previous plan.
    pub(super) fn appends_keep_roots(
        &self,
        bytes: &[u8],
        replay: &JournalReplay,
        candidates: &BTreeSet<ReservationId>,
    ) -> bool {
        let Ok(reservations) = &replay.reservations else {
            return false;
        };
        let mut last = None;
        for record in bytes.split_inclusive(|byte| *byte == b'\n') {
            let Ok(event) = serde_json::from_slice::<JournalEvent>(record) else {
                return false;
            };
            if let JournalOperation::ResolveIncursion { incident_id } = &event.operation {
                let Some(incident) = reservations
                    .incursion_incidents()
                    .find(|incident| incident.id() == *incident_id)
                else {
                    return false;
                };
                let Ok(value) = serde_json::to_value(incident) else {
                    return false;
                };
                if !reservation_references(&value, candidates).is_empty() {
                    return false;
                }
            }
            last = Some(event.operation);
        }
        if self.final_roots.is_disjoint(candidates) {
            return true;
        }
        last.and_then(|operation| serde_json::to_value(operation).ok())
            .is_some_and(|value| {
                let refs = reservation_references(&value, candidates);
                self.final_roots
                    .intersection(candidates)
                    .all(|id| refs.contains(id))
            })
    }
}

/// Expired reservation identities proven unnecessary to the remaining journal.
pub(super) struct ExpiredHistory {
    ids:              BTreeSet<ReservationId>,
    pub(super) basis: HistoryCompactionBasis,
    /// The reservation references of each complete record, in journal order.
    references:       Vec<BTreeSet<ReservationId>>,
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
        repository_root: &Path,
    ) -> Result<Self, Error> {
        let Ok(reservations) = &replay.reservations else {
            return Ok(Self {
                ids:        BTreeSet::new(),
                basis:      HistoryCompactionBasis {
                    candidates:  BTreeSet::new(),
                    final_roots: BTreeSet::new(),
                },
                references: Vec::new(),
            });
        };
        let known = reservations
            .iter()
            .map(Reservation::id)
            .collect::<BTreeSet<_>>();
        let candidates = FinishedHistory::compaction_candidates(replay, now, repository_root)
            .collect::<BTreeSet<_>>();
        let mut retained = known
            .difference(&candidates)
            .copied()
            .collect::<BTreeSet<_>>();
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
        // A record that refers to a root is already kept by the dependency closure.
        let final_roots = references
            .last()
            .filter(|last| last.is_disjoint(&retained))
            .cloned()
            .unwrap_or_default();
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
            basis: HistoryCompactionBasis {
                candidates,
                final_roots,
            },
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
