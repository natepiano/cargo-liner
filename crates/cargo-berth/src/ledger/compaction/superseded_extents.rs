//! The superseded merge extent observations of a replayed journal.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use serde::Deserialize;
use serde::Serialize;

use crate::ids::ReservationId;
use crate::ledger::history::HistoryWindowChange;
use crate::ledger::journal::JournalOperation;

/// The merge extent observations of a replayed journal that later observations supersede.
///
/// Observations are identified by ordinal, the zero-based position of their record in the
/// journal. Holds one entry per observed reservation and one per observation still named.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(in crate::ledger) struct SupersededExtents {
    /// The ordinal of the latest observation naming each reservation.
    latest:                     BTreeMap<ReservationId, u64>,
    /// Every observation that is the latest for at least one reservation, by ordinal.
    live:                       BTreeMap<u64, LiveExtent>,
    /// The total byte length of the observations no reservation names as its latest.
    pub(super) droppable_bytes: u64,
    /// Observations that also establish a retained terminal-state clock.
    history_roots:              BTreeMap<ReservationId, u64>,
}

/// An observation that is the latest for at least one reservation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct LiveExtent {
    /// The byte length of the observation's record, its newline included.
    bytes: u64,
    /// How many reservations name the observation as their latest.
    names: usize,
}

impl SupersededExtents {
    /// Record the operation of the record at `ordinal`, `bytes` long, the next after every record
    /// already observed.
    pub(in crate::ledger) fn observe(
        &mut self,
        ordinal: u64,
        bytes: u64,
        operation: &JournalOperation,
    ) {
        let Some(reservations) = operation.merge_extent_reservations() else {
            return;
        };
        self.live.insert(
            ordinal,
            LiveExtent {
                bytes,
                names: reservations.len(),
            },
        );
        for reservation_id in reservations {
            let Some(previous) = self.latest.insert(reservation_id, ordinal) else {
                continue;
            };
            if let Entry::Occupied(mut entry) = self.live.entry(previous) {
                entry.get_mut().names -= 1;
                if entry.get().names == 0 {
                    self.droppable_bytes += entry.remove().bytes;
                }
            }
        }
    }

    /// Retain the first terminal observation until a later disposition replaces its clock.
    pub(in crate::ledger) fn observe_history(
        &mut self,
        changes: Vec<HistoryWindowChange>,
        ordinal: u64,
        is_extent: bool,
    ) {
        for change in changes {
            let (id, starts) = match change {
                HistoryWindowChange::Started(id) => (id, true),
                HistoryWindowChange::Ended(id) => (id, false),
            };
            if let Some(previous) = self.history_roots.remove(&id)
                && let Entry::Occupied(mut entry) = self.live.entry(previous)
            {
                entry.get_mut().names -= 1;
                if entry.get().names == 0 {
                    self.droppable_bytes += entry.remove().bytes;
                }
            }
            if starts && is_extent {
                self.history_roots.insert(id, ordinal);
                if let Some(entry) = self.live.get_mut(&ordinal) {
                    entry.names += 1;
                }
            }
        }
    }

    /// Whether compaction keeps the record at `ordinal`, a merge extent observation when
    /// `merge_extent_observation` holds: every record except an observation that no reservation
    /// names as its latest.
    pub(super) fn keeps(&self, ordinal: u64, merge_extent_observation: bool) -> bool {
        !merge_extent_observation || self.live.contains_key(&ordinal)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::SupersededExtents;
    use crate::ids::ReservationId;
    use crate::ledger::compaction::fixtures;
    use crate::ledger::journal::JournalOperation;
    use crate::ledger::journal::JournalReplay;

    #[test]
    fn only_fully_superseded_extent_observations_drop() -> Result<(), Box<dyn Error>> {
        let first = ReservationId::new();
        let second = ReservationId::new();
        let operations = [
            fixtures::legacy_extent(first)?,
            fixtures::holder_extent(&[first, second])?,
            fixtures::operation(
                &serde_json::json!({"op": "renew", "reservation_id": first.to_string()}),
            )?,
            fixtures::legacy_extent(second)?,
            fixtures::holder_extent(&[first])?,
        ];
        let lengths = [10, 20, 30, 40, 50];
        let mut superseded = SupersededExtents::default();
        let kept = |superseded: &SupersededExtents, observed: &[JournalOperation]| {
            (0_u64..)
                .zip(observed)
                .map(|(ordinal, operation)| {
                    superseded.keeps(ordinal, operation.is_merge_extent_observation())
                })
                .collect::<Vec<_>>()
        };

        for (ordinal, (bytes, operation)) in (0_u64..).zip(lengths.iter().zip(&operations)).take(4)
        {
            superseded.observe(ordinal, *bytes, operation);
        }
        assert_eq!(
            kept(&superseded, &operations[..4]),
            [false, true, true, true],
            "the holder record still names the first reservation"
        );
        assert_eq!(superseded.droppable_bytes, 10);

        superseded.observe(4, lengths[4], &operations[4]);
        assert_eq!(
            kept(&superseded, &operations),
            [false, false, true, true, true]
        );
        assert_eq!(superseded.droppable_bytes, 10 + 20);
        Ok(())
    }

    #[test]
    fn the_final_record_and_generation_survive() -> Result<(), Box<dyn Error>> {
        let (events, _) = fixtures::recent_round_trip_events()?;
        let (bytes, record_ends) = fixtures::journal_bytes(&events)?;

        for (prefix_length, record_end) in record_ends.into_iter().enumerate().skip(1) {
            let prefix = &events[..prefix_length];
            let (replay, _) = JournalReplay::empty().extended_over(&bytes[..record_end])?;
            let last_extent = (0_u64..)
                .zip(prefix)
                .filter(|(_, event)| event.operation.merge_extent_reservations().is_some())
                .last();
            let final_ordinal = u64::try_from(prefix_length - 1)?;
            let final_event = &prefix[prefix_length - 1];

            assert!(replay.superseded.keeps(
                final_ordinal,
                final_event.operation.is_merge_extent_observation()
            ));
            assert_eq!(replay.generation, final_event.projection_generation);
            if let Some((ordinal, event)) = last_extent {
                assert!(
                    replay
                        .superseded
                        .keeps(ordinal, event.operation.is_merge_extent_observation())
                );
            }
        }
        Ok(())
    }

    #[test]
    fn droppable_bytes_equal_across_a_checkpoint_resume() -> Result<(), Box<dyn Error>> {
        let (events, _) = fixtures::recent_round_trip_events()?;
        let (bytes, record_ends) = fixtures::journal_bytes(&events)?;
        let (whole, _) = JournalReplay::empty().extended_over(&bytes)?;

        for record_end in record_ends {
            let (checkpointed, appended) = bytes.split_at(record_end);
            let (checkpointed, _) = JournalReplay::empty().extended_over(checkpointed)?;
            let decoded =
                serde_json::from_slice::<JournalReplay>(&serde_json::to_vec(&checkpointed)?)?;
            let (resumed, _) = decoded.extended_over(appended)?;

            assert_eq!(resumed.superseded, whole.superseded);
        }
        let expected = fixtures::superseded_record_bytes(&events)?;
        assert!(expected > 0, "the sequence supersedes an observation");
        assert_eq!(whole.superseded.droppable_bytes, expected);
        Ok(())
    }
}
