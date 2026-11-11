//! Spawn contention events for one observed test thread.

use std::cell::RefCell;
use std::io;
use std::sync::TryLockError;
use std::sync::mpsc::Sender;

use crate::spawn_lock;
use crate::spawn_lock::PROCESS_SPAWN;

/// The lock state at the observed spawn boundary.
#[derive(Debug, PartialEq, Eq)]
pub enum SpawnContention {
    /// The thread acquired the lock without contention.
    Acquired,
    /// Another thread holds the lock; this thread must wait.
    Waiting,
}

enum SpawnObserver {
    Unobserved,
    Observed(Sender<SpawnContention>),
}

thread_local! {
    static OBSERVER: RefCell<SpawnObserver> = const { RefCell::new(SpawnObserver::Unobserved) };
}

struct ObservationScope;

impl Drop for ObservationScope {
    fn drop(&mut self) {
        OBSERVER.with(|observer| *observer.borrow_mut() = SpawnObserver::Unobserved);
    }
}

/// Observe spawn contention only within this thread's operation.
pub fn observe_spawn_contention<T>(
    sender: Sender<SpawnContention>,
    operation: impl FnOnce() -> T,
) -> T {
    OBSERVER.with(|observer| *observer.borrow_mut() = SpawnObserver::Observed(sender));
    let scope = ObservationScope;
    let result = operation();
    drop(scope);
    result
}

/// Serialize pipe creation and report contention before the thread waits.
///
/// # Errors
/// Returns an error if the lock is poisoned or the observer has closed.
pub fn with_spawn_lock<T>(operation: impl FnOnce() -> T) -> io::Result<T> {
    let guard = match PROCESS_SPAWN.try_lock() {
        Ok(guard) => {
            report(SpawnContention::Acquired)?;
            guard
        },
        Err(TryLockError::WouldBlock) => {
            report(SpawnContention::Waiting)?;
            return spawn_lock::with_spawn_lock(operation);
        },
        Err(TryLockError::Poisoned(_)) => {
            return Err(io::Error::other("Git spawn lock was poisoned"));
        },
    };
    let result = operation();
    drop(guard);
    Ok(result)
}

fn report(contention: SpawnContention) -> io::Result<()> {
    OBSERVER.with(|observer| match &*observer.borrow() {
        SpawnObserver::Unobserved => Ok(()),
        SpawnObserver::Observed(sender) => sender.send(contention).map_err(io::Error::other),
    })
}
