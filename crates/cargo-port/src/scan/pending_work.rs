use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::Mutex;

/// Keep one pending replacement for each owner with a scheduled worker.
pub(super) struct PendingWork<K, T> {
    pending:   HashMap<K, T>,
    scheduled: HashSet<K>,
}

impl<K, T> Default for PendingWork<K, T> {
    fn default() -> Self {
        Self {
            pending:   HashMap::new(),
            scheduled: HashSet::new(),
        }
    }
}

pub(super) enum WorkSubmission {
    StartWorker,
    JoinedWorker,
}

pub(super) enum NextWork<T> {
    Run(T),
    Finished,
}

impl<K: Clone + Eq + Hash, T> PendingWork<K, T> {
    pub(super) fn submit(&mut self, owner: K, work: T) -> WorkSubmission {
        self.pending.insert(owner.clone(), work);
        if self.scheduled.insert(owner) {
            WorkSubmission::StartWorker
        } else {
            WorkSubmission::JoinedWorker
        }
    }

    pub(super) fn next(&mut self, owner: &K) -> NextWork<T> {
        if let Some(work) = self.pending.remove(owner) {
            NextWork::Run(work)
        } else {
            self.scheduled.remove(owner);
            NextWork::Finished
        }
    }
}

/// Remove pending work if its worker is cancelled before completion.
pub(super) struct PendingWorker<K: Eq + Hash, T> {
    queue: Arc<Mutex<PendingWork<K, T>>>,
    owner: K,
    state: WorkerState,
}

enum WorkerState {
    Scheduled,
    Finished,
}

impl<K: Clone + Eq + Hash, T> PendingWorker<K, T> {
    pub(super) const fn new(queue: Arc<Mutex<PendingWork<K, T>>>, owner: K) -> Self {
        Self {
            queue,
            owner,
            state: WorkerState::Scheduled,
        }
    }

    pub(super) const fn owner(&self) -> &K { &self.owner }

    pub(super) fn next(&mut self) -> NextWork<T> {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let next = queue.next(&self.owner);
        if matches!(next, NextWork::Finished) {
            self.state = WorkerState::Finished;
        }
        drop(queue);
        next
    }
}

impl<K: Eq + Hash, T> Drop for PendingWorker<K, T> {
    fn drop(&mut self) {
        if matches!(self.state, WorkerState::Scheduled) {
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            queue.pending.remove(&self.owner);
            queue.scheduled.remove(&self.owner);
        }
    }
}
