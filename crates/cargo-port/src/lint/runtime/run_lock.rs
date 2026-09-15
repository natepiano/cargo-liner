//! Cross-process exclusion for one project's lint runs.
//!
//! Every cargo-port instance sharing a cache root watches the same files, so a
//! single edit triggers the same lint in each of them. The instance that takes
//! the project's run lock runs the commands. The others wait for the lock and
//! adopt the result it wrote, running only when that result started before the
//! change they were asked to lint. The OS releases the lock when its holder's
//! file closes — including when the process exits or crashes — so no lock is
//! ever left behind.

use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;

use super::Path;
use super::io;
use super::paths;
use crate::constants::LINTS_RUN_LOCK;

/// Held for the duration of one lint run. Dropping it closes the file, which
/// releases the lock.
pub(super) struct RunLock {
    _file: Option<File>,
}

impl RunLock {
    /// A run that proceeds without cross-process exclusion, because the lock
    /// file could not be opened or locked. Duplicate work beats no lint at all.
    pub(super) const fn unlocked() -> Self { Self { _file: None } }
}

/// Take the project's run lock without blocking. `Ok(None)` means another
/// holder has it.
pub(super) fn try_acquire(cache_root: &Path, project_root: &Path) -> io::Result<Option<RunLock>> {
    let project_dir = paths::project_dir_under(cache_root, project_root);
    std::fs::create_dir_all(&project_dir)?;
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(project_dir.join(LINTS_RUN_LOCK))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(RunLock { _file: Some(file) })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(err)) => Err(err),
    }
}

/// Whether a run holds the project's lock right now. Never creates the lock
/// file: a project without one has no run in flight.
pub(super) fn is_held(cache_root: &Path, project_root: &Path) -> bool {
    let path = paths::project_dir_under(cache_root, project_root).join(LINTS_RUN_LOCK);
    let Ok(file) = OpenOptions::new().write(true).open(path) else {
        return false;
    };
    matches!(file.try_lock(), Err(TryLockError::WouldBlock))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn second_holder_is_refused_until_the_first_drops() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        let first = try_acquire(cache_dir.path(), project_dir.path())
            .expect("lock io")
            .expect("free lock is acquired");
        assert!(is_held(cache_dir.path(), project_dir.path()));
        assert!(
            try_acquire(cache_dir.path(), project_dir.path())
                .expect("lock io")
                .is_none(),
            "a held lock must refuse a second holder"
        );

        drop(first);
        assert!(!is_held(cache_dir.path(), project_dir.path()));
        assert!(
            try_acquire(cache_dir.path(), project_dir.path())
                .expect("lock io")
                .is_some(),
            "a released lock must be acquirable again"
        );
    }

    #[test]
    fn is_held_does_not_create_the_lock_file() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        assert!(!is_held(cache_dir.path(), project_dir.path()));
        assert!(
            !paths::project_dir_under(cache_dir.path(), project_dir.path())
                .join(LINTS_RUN_LOCK)
                .exists()
        );
    }
}
