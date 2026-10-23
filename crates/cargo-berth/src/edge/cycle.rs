//! Cycle detection over replayed ordering adjacency: in-degree peeling for a
//! complete graph, reverse reachability for one proposed `before -> after` edge.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Display;
use std::fmt::Formatter;

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

/// A closed traversal naming every reservation in one refused ordering cycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OrderingCycle(Vec<ReservationId>);

impl Display for OrderingCycle {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        for (index, reservation) in self.0.iter().enumerate() {
            if index != 0 {
                formatter.write_str(" -> ")?;
            }
            reservation.fmt(formatter)?;
        }
        Ok(())
    }
}

/// Reject a cyclic adjacency with a closed traversal, using an iterative depth-first search.
pub(super) fn ensure_acyclic(
    adjacency: &HashMap<ReservationId, Vec<ReservationId>>,
) -> Result<(), OrderingCycle> {
    let mut visited = HashSet::new();
    let mut active = HashMap::new();
    let mut path = Vec::new();
    for root in adjacency.keys() {
        if !visited.insert(*root) {
            continue;
        }
        let mut pending = vec![(*root, 0)];
        active.insert(*root, path.len());
        path.push(*root);
        while let Some((vertex, next)) = pending.last_mut() {
            let successors = adjacency.get(vertex).map_or(&[][..], Vec::as_slice);
            if let Some(successor) = successors.get(*next) {
                *next += 1;
                if let Some(start) = active.get(successor) {
                    let mut cycle = path[*start..].to_vec();
                    cycle.push(*successor);
                    return Err(OrderingCycle(cycle));
                }
                if visited.insert(*successor) {
                    active.insert(*successor, path.len());
                    path.push(*successor);
                    pending.push((*successor, 0));
                }
            } else {
                active.remove(vertex);
                path.pop();
                pending.pop();
            }
        }
    }
    Ok(())
}
