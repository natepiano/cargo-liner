//! Cycle detection over replayed ordering adjacency: in-degree peeling for a
//! complete graph, reverse reachability for one proposed `before -> after` edge.

use std::collections::HashMap;
use std::collections::HashSet;

use crate::ids::ReservationId;

/// Return whether a completely rebuilt adjacency map contains a directed cycle.
pub(super) fn contains_cycle(adjacency: &HashMap<ReservationId, Vec<ReservationId>>) -> bool {
    let mut incoming_edges = adjacency
        .keys()
        .map(|reservation_id| (*reservation_id, 0_usize))
        .collect::<HashMap<_, _>>();
    for successors in adjacency.values() {
        for successor in successors {
            *incoming_edges.entry(*successor).or_default() += 1;
        }
    }
    let mut ready = incoming_edges
        .iter()
        .filter_map(|(reservation_id, incoming)| (*incoming == 0).then_some(*reservation_id))
        .collect::<Vec<_>>();
    let mut visited = 0_usize;
    while let Some(reservation_id) = ready.pop() {
        visited += 1;
        for successor in adjacency.get(&reservation_id).into_iter().flatten() {
            let Some(incoming) = incoming_edges.get_mut(successor) else {
                continue;
            };
            *incoming -= 1;
            if *incoming == 0 {
                ready.push(*successor);
            }
        }
    }
    visited != incoming_edges.len()
}

/// Return whether adding `before -> after` would create a directed cycle.
pub(super) fn would_create_cycle(
    adjacency: &HashMap<ReservationId, Vec<ReservationId>>,
    before: ReservationId,
    after: ReservationId,
) -> bool {
    if before == after {
        return true;
    }
    let mut pending = vec![after];
    let mut visited = HashSet::new();
    while let Some(reservation_id) = pending.pop() {
        if reservation_id == before {
            return true;
        }
        if !visited.insert(reservation_id) {
            continue;
        }
        if let Some(successors) = adjacency.get(&reservation_id) {
            pending.extend(successors.iter().copied());
        }
    }
    false
}
