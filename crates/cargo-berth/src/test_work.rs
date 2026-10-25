//! Per-test observations of expensive scope and history work at the operations themselves.

use std::cell::Cell;

/// Work that must stay bounded as retained holders, paths and history grow.
#[derive(Clone, Copy)]
pub(crate) enum Work {
    ScopeKey,
    ScopeComparison,
    IncursionReservation,
    AncestryParents,
}

/// Actual operation counts on this test thread.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WorkCounts {
    pub(crate) scope_keys:             usize,
    pub(crate) scope_comparisons:      usize,
    pub(crate) incursion_reservations: usize,
    pub(crate) ancestry_parents:       usize,
}

thread_local! {
    static COUNTS: Cell<WorkCounts> = const { Cell::new(WorkCounts {
        scope_keys: 0,
        scope_comparisons: 0,
        incursion_reservations: 0,
        ancestry_parents: 0,
    }) };
}

/// A fresh counting interval that restores its enclosing interval even on unwind.
pub(crate) struct WorkCounter(WorkCounts);

impl WorkCounter {
    pub(crate) fn start() -> Self { Self(COUNTS.replace(WorkCounts::default())) }

    pub(crate) fn finish(self) -> WorkCounts {
        let counts = COUNTS.get();
        drop(self);
        counts
    }
}

impl Drop for WorkCounter {
    fn drop(&mut self) { COUNTS.set(self.0); }
}

pub(crate) fn record(work: Work) {
    COUNTS.update(|mut counts| {
        match work {
            Work::ScopeKey => counts.scope_keys += 1,
            Work::ScopeComparison => counts.scope_comparisons += 1,
            Work::IncursionReservation => counts.incursion_reservations += 1,
            Work::AncestryParents => counts.ancestry_parents += 1,
        }
        counts
    });
}
