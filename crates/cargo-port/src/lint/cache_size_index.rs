//! Persistent count of total bytes under the lint cache root.
//!
//! Avoids walking ~8000 files on every cache-usage refresh. The lint
//! runtime updates this file as runs are archived, history lines are
//! appended, and old runs are pruned. Reads are O(1).
//!
//! Drift policy: if the file is missing or unparseable, the caller
//! re-walks the cache and rewrites the index. Concurrent writers can
//! race (rare — the lint runtime serializes its own writes); a lost
//! update produces a stale value that the next walk-and-rewrite or
//! prune corrects. Users can clear lint state at any time, so a small
//! amount of drift is acceptable.

use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

/// Hidden filename at the cache root holding the byte count as plain
/// text decimal (one line, no trailing newline required).
const INDEX_FILENAME: &str = ".cache_size";

/// Serializes [`adjust`] calls within the process. Two lint workers running
/// in parallel for different projects both call adjust, and without this
/// mutex their read-modify-write sequences race and lose updates. The
/// process is the only writer (cross-process races aren't a concern —
/// only one cargo-port runs at a time).
static ADJUST_LOCK: Mutex<()> = Mutex::new(());

fn index_path(cache_root: &Path) -> PathBuf { cache_root.join(INDEX_FILENAME) }

pub(super) fn read(cache_root: &Path) -> Option<u64> {
    let raw = fs::read_to_string(index_path(cache_root)).ok()?;
    raw.trim().parse::<u64>().ok()
}

pub(super) fn write(cache_root: &Path, bytes: u64) -> io::Result<()> {
    fs::create_dir_all(cache_root)?;
    let final_path = index_path(cache_root);
    let tmp_path = final_path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp_path)?;
        write!(file, "{bytes}")?;
        file.sync_all()?;
    }
    fs::rename(tmp_path, final_path)
}

/// Stat `target_path` (returns 0 on missing/error). Pair with
/// [`apply_write_delta`] around any write that overwrites or creates
/// a file inside the cache, so the index tracks the size change.
pub(super) fn file_size_or_zero(target_path: &Path) -> u64 {
    fs::metadata(target_path).map_or(0, |meta| meta.len())
}

/// Adjust the index by `new_size - old_size` (in bytes). Pair with a
/// preceding [`file_size_or_zero`] call before the write.
pub(super) fn apply_write_delta(cache_root: &Path, old_size: u64, new_size: u64) {
    if new_size == old_size {
        return;
    }
    let delta = i128::from(new_size) - i128::from(old_size);
    let clamped = i64::try_from(delta).unwrap_or(if delta < 0 { i64::MIN } else { i64::MAX });
    adjust(cache_root, clamped);
}

/// Best-effort signed adjustment of the stored byte count. If the
/// index is missing the call is a no-op — the next
/// [`crate::lint::retained_cache_usage`] read will walk and seed it.
/// `delta` saturates at `u64::MIN`/`u64::MAX`. Serialized through
/// [`ADJUST_LOCK`] so concurrent lint workers can't lose updates.
pub(super) fn adjust(cache_root: &Path, delta: i64) {
    let _guard = ADJUST_LOCK.lock();
    let Some(current) = read(cache_root) else {
        return;
    };
    let next = if delta >= 0 {
        current.saturating_add(u64::try_from(delta).unwrap_or(u64::MAX))
    } else {
        current.saturating_sub(delta.unsigned_abs())
    };
    let _ = write(cache_root, next);
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn read_missing_returns_none() {
        let dir = tempdir().expect("tempdir");
        assert_eq!(read(dir.path()), None);
    }

    #[test]
    fn write_then_read_roundtrips() {
        let dir = tempdir().expect("tempdir");
        write(dir.path(), 12_345).expect("write");
        assert_eq!(read(dir.path()), Some(12_345));
    }

    #[test]
    fn adjust_increments_and_decrements() {
        let dir = tempdir().expect("tempdir");
        write(dir.path(), 1_000).expect("write");
        adjust(dir.path(), 250);
        assert_eq!(read(dir.path()), Some(1_250));
        adjust(dir.path(), -500);
        assert_eq!(read(dir.path()), Some(750));
    }

    #[test]
    fn adjust_saturates_at_zero() {
        let dir = tempdir().expect("tempdir");
        write(dir.path(), 100).expect("write");
        adjust(dir.path(), -500);
        assert_eq!(read(dir.path()), Some(0));
    }

    #[test]
    fn adjust_no_op_when_missing() {
        let dir = tempdir().expect("tempdir");
        adjust(dir.path(), 250);
        assert_eq!(read(dir.path()), None);
    }
}
