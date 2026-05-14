//! Generic "load from disk + watch stamp + try-reload" lifecycle.
//!
//! The primitive captures the load-watch-reload contract once: it
//! owns the path, a stamp (modified time + len), and the most
//! recently parsed value. Callers invoke [`WatchedFile::try_reload`]
//! every tick; if the file's stamp has not changed it short-circuits
//! without touching disk content.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

/// Modified-time + length pair used to detect on-disk changes
/// without re-reading the file when nothing has changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len:      u64,
}

/// A value of type `T` parsed from a watched file on disk.
///
/// Construct with [`Self::new`]; refresh on each tick with
/// `take_stamp_change`. The path may be `None` (no on-disk source)
/// in which case `take_stamp_change` always returns `None`.
pub struct WatchedFile<T> {
    path:        Option<PathBuf>,
    stamp:       Option<Stamp>,
    /// Most recently parsed value. Callers replace this directly
    /// after their own parse step.
    pub current: T,
}

impl<T> WatchedFile<T> {
    /// Build a `WatchedFile` for an already-parsed value. Captures
    /// the on-disk stamp at this moment so the next `take_stamp_change`
    /// short-circuits unless the file changes again.
    pub fn new(path: Option<PathBuf>, current: T) -> Self {
        let stamp = path.as_deref().and_then(read_stamp);
        Self {
            path,
            stamp,
            current,
        }
    }

    /// Path being watched, or `None` for an in-memory-only value.
    pub fn path(&self) -> Option<&Path> { self.path.as_deref() }

    /// Refresh the cached stamp without re-parsing. Used after the
    /// caller writes the file itself so the next `take_stamp_change`
    /// does not see the caller's own write as an external change.
    pub fn sync_stamp(&mut self) { self.stamp = self.path.as_deref().and_then(read_stamp); }

    /// Drop the cached stamp so the next
    /// [`Self::take_stamp_change`] always sees a delta. Test-only
    /// support: used when swapping the watched path so the new
    /// file's identical mtime doesn't get treated as unchanged.
    pub const fn clear_stamp_for_test(&mut self) { self.stamp = None; }

    /// Return `Some(path)` if the file's on-disk stamp has changed
    /// since the last seen value, updating the cached stamp before
    /// returning.
    pub fn take_stamp_change(&mut self) -> Option<&Path> {
        let path = self.path.as_deref()?;
        let current = read_stamp(path);
        if current == self.stamp {
            return None;
        }
        self.stamp = current;
        Some(path)
    }
}

fn read_stamp(path: &Path) -> Option<Stamp> {
    let metadata = fs::metadata(path).ok()?;
    Some(Stamp {
        modified: metadata.modified().ok(),
        len:      metadata.len(),
    })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use super::*;

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// Allocate a unique path under the system temp dir for an
    /// individual test. Files are cleaned up by the OS — and tests
    /// never overlap because the sequence is process-global.
    fn temp_path(label: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "watched_file_{label}_{n}_{}.txt",
            std::process::id()
        ))
    }

    fn write_synced_file(path: &Path, content: &str) {
        let mut f = std::fs::File::create(path).expect("create temp file");
        f.write_all(content.as_bytes()).expect("write temp file");
        f.sync_all().expect("sync temp file");
    }

    #[test]
    fn take_stamp_change_with_no_path_is_none() {
        let mut wf: WatchedFile<String> = WatchedFile::new(None, "default".to_string());
        assert!(wf.take_stamp_change().is_none());
    }

    #[test]
    fn take_stamp_change_returns_path_then_swallows_until_next_change() {
        let path = temp_path("take_stamp");
        write_synced_file(&path, "v0");
        let mut wf = WatchedFile::new(Some(path.clone()), "seed".to_string());
        // No change yet.
        assert!(wf.take_stamp_change().is_none());
        std::thread::sleep(Duration::from_millis(20));
        write_synced_file(&path, "v1");
        // First call after change returns the path and updates the
        // cached stamp.
        assert_eq!(wf.take_stamp_change(), Some(path.as_path()));
        // Subsequent call sees the same stamp again.
        assert!(wf.take_stamp_change().is_none());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn sync_stamp_marks_caller_owned_writes_as_unchanged() {
        let path = temp_path("sync");
        write_synced_file(&path, "before");
        let mut wf = WatchedFile::new(Some(path.clone()), "before".to_string());
        std::thread::sleep(Duration::from_millis(20));
        // Caller writes the file itself and updates the stamp so
        // the next take_stamp_change doesn't see its own write.
        write_synced_file(&path, "self-written");
        wf.sync_stamp();
        assert!(wf.take_stamp_change().is_none());
        std::fs::remove_file(&path).ok();
    }
}
