//! One process-wide boundary for pipe setup and child execution.

use std::io;
use std::sync::Mutex;

// Keep dependency-owned probes and our pipe creation inside the same spawn boundary.
static PROCESS_SPAWN: Mutex<()> = Mutex::new(());

/// Serialize pipe creation and child spawn.
///
/// # Errors
/// Returns an error if an earlier spawn panicked while holding the lock.
pub fn with_spawn_lock<T>(operation: impl FnOnce() -> T) -> io::Result<T> {
    let guard = PROCESS_SPAWN
        .lock()
        .map_err(|_| io::Error::other("Git spawn lock was poisoned"))?;
    let result = operation();
    drop(guard);
    Ok(result)
}
