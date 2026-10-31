//! Observational work counts at native repository opens and uncached ancestor traversals.
//! Each worker owns its counts; these never participate in a Git decision.

use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ReadWorkCounts {
    pub(super) repository_opens: usize,
    pub(super) ancestor_sets:    usize,
}

thread_local! {
    static COUNTS: Cell<ReadWorkCounts> = const { Cell::new(ReadWorkCounts {
        repository_opens: 0,
        ancestor_sets: 0,
    }) };
}

pub(super) fn record_repository_open() {
    COUNTS.update(|mut counts| {
        counts.repository_opens += 1;
        counts
    });
}

pub(super) fn record_ancestor_set() {
    COUNTS.update(|mut counts| {
        counts.ancestor_sets += 1;
        counts
    });
}

#[cfg(test)]
pub(super) fn counts() -> ReadWorkCounts { COUNTS.get() }
