//! Which merge extent observations a later observation supersedes, tracked as the journal replays.
//!
//! An observation, in either record form, replaces the recorded merge extent of every reservation
//! it names, and no replay step reads a merge extent. So once every reservation an observation
//! names is named again by a later observation, the record leaves no trace in the fold, and
//! compacting the journal may drop it. An observation superseded for only some of its reservations
//! is kept whole. Every other record is kept.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use serde::Deserialize;
use serde::Serialize;

use super::journal::JournalOperation;
use crate::ids::ReservationId;

/// The merge extent observations of a replayed journal that later observations supersede.
///
/// Observations are identified by ordinal, the zero-based position of their record in the
/// journal. Holds one entry per observed reservation and one per observation still named.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct SupersededExtents {
    /// The ordinal of the latest observation naming each reservation.
    latest:          BTreeMap<ReservationId, u64>,
    /// Every observation that is the latest for at least one reservation, by ordinal.
    live:            BTreeMap<u64, LiveExtent>,
    /// The total byte length of the observations no reservation names as its latest.
    droppable_bytes: u64,
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
    pub(super) fn observe(&mut self, ordinal: u64, bytes: u64, operation: &JournalOperation) {
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

    /// Whether compaction keeps the record at `ordinal` holding `operation`: every record except
    /// a merge extent observation that no reservation names as its latest.
    #[cfg(test)]
    pub(super) fn keeps(&self, ordinal: u64, operation: &JournalOperation) -> bool {
        operation.merge_extent_reservations().is_none() || self.live.contains_key(&ordinal)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;

    use serde_json::Value;

    use super::SupersededExtents;
    use crate::ids::ReservationId;
    use crate::ledger::journal;
    use crate::ledger::journal::JournalEvent;
    use crate::ledger::journal::JournalOperation;
    use crate::ledger::journal::JournalReplay;

    #[test]
    fn only_fully_superseded_extent_observations_drop() -> Result<(), Box<dyn Error>> {
        let first = ReservationId::new();
        let second = ReservationId::new();
        let operations = [
            legacy_extent(first)?,
            holder_extent(&[first, second])?,
            operation(&serde_json::json!({"op": "renew", "reservation_id": first.to_string()}))?,
            legacy_extent(second)?,
            holder_extent(&[first])?,
        ];
        let lengths = [10, 20, 30, 40, 50];
        let mut superseded = SupersededExtents::default();
        let kept = |superseded: &SupersededExtents, observed: &[JournalOperation]| {
            (0_u64..)
                .zip(observed)
                .map(|(ordinal, operation)| superseded.keeps(ordinal, operation))
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
        let (events, _) = journal::round_trip_sequence_events()?;
        let (bytes, record_ends) = journal_bytes(&events)?;

        for (prefix_length, record_end) in record_ends.into_iter().enumerate().skip(1) {
            let prefix = &events[..prefix_length];
            let (replay, _) = JournalReplay::empty().extended_over(&bytes[..record_end])?;
            let last_extent = (0_u64..)
                .zip(prefix)
                .filter(|(_, event)| event.operation.merge_extent_reservations().is_some())
                .last();
            let final_ordinal = u64::try_from(prefix_length - 1)?;
            let final_event = &prefix[prefix_length - 1];

            assert!(
                replay
                    .superseded
                    .keeps(final_ordinal, &final_event.operation)
            );
            assert_eq!(replay.generation, final_event.projection_generation);
            if let Some((ordinal, event)) = last_extent {
                assert!(replay.superseded.keeps(ordinal, &event.operation));
            }
        }
        Ok(())
    }

    #[test]
    fn droppable_bytes_equal_across_a_checkpoint_resume() -> Result<(), Box<dyn Error>> {
        let (events, _) = journal::round_trip_sequence_events()?;
        let (bytes, record_ends) = journal_bytes(&events)?;
        let (whole, _) = JournalReplay::empty().extended_over(&bytes)?;

        for record_end in record_ends {
            let (checkpointed, appended) = bytes.split_at(record_end);
            let (checkpointed, _) = JournalReplay::empty().extended_over(checkpointed)?;
            let decoded =
                serde_json::from_slice::<JournalReplay>(&serde_json::to_vec(&checkpointed)?)?;
            let (resumed, _) = decoded.extended_over(appended)?;

            assert_eq!(resumed.superseded, whole.superseded);
        }
        let expected = superseded_record_bytes(&events)?;
        assert!(expected > 0, "the sequence supersedes an observation");
        assert_eq!(whole.superseded.droppable_bytes, expected);
        Ok(())
    }

    /// The byte length, newline included, of every merge extent observation whose every
    /// reservation a later observation names.
    fn superseded_record_bytes(events: &[JournalEvent]) -> Result<u64, Box<dyn Error>> {
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
    fn journal_bytes(events: &[JournalEvent]) -> Result<(Vec<u8>, Vec<usize>), serde_json::Error> {
        let mut bytes = Vec::new();
        let mut record_ends = vec![0];
        for event in events {
            bytes.extend(serde_json::to_vec(event)?);
            bytes.push(b'\n');
            record_ends.push(bytes.len());
        }
        Ok((bytes, record_ends))
    }

    fn legacy_extent(reservation_id: ReservationId) -> Result<JournalOperation, serde_json::Error> {
        operation(&serde_json::json!({
            "op": "merge_extent_observed",
            "reservation_id": reservation_id.to_string(),
            "extent": empty_extent(),
            "run_status": "editing",
        }))
    }

    fn holder_extent(
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

    fn empty_extent() -> Value {
        serde_json::json!({
            "status": "empty",
            "key": {
                "trunk": "1111111111111111111111111111111111111111",
                "head": "2222222222222222222222222222222222222222",
                "working_tree": {"tracked_paths": [], "untracked_paths": []},
            },
        })
    }

    fn operation(operation: &Value) -> Result<JournalOperation, serde_json::Error> {
        serde_json::from_value(operation.clone())
    }
}
