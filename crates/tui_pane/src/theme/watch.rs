//! Polled directory-change detector for `themes/*.toml`.
//!
//! Mirrors [`crate::WatchedFile`] in spirit, but watches a *set* of
//! files. The fingerprint hashes each `*.toml` entry's filename +
//! modified time + length; any addition, removal, or content change
//! flips the fingerprint so [`ThemesWatch::take_change`] reports a
//! delta on the next tick.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

/// Polled directory-change detector for a directory of `*.toml` theme
/// files. Caches a fingerprint of the directory's entries and reports
/// a delta whenever the next read disagrees.
pub struct ThemesWatch {
    dir:         Option<PathBuf>,
    fingerprint: u64,
}

impl ThemesWatch {
    /// Build a watch for `dir`. The initial fingerprint is captured
    /// now so the next call to [`Self::take_change`] only fires on a
    /// subsequent change.
    #[must_use]
    pub fn new(dir: Option<PathBuf>) -> Self {
        let fingerprint = dir.as_deref().map_or(0, directory_fingerprint);
        Self { dir, fingerprint }
    }

    /// Directory being watched, or `None` if no source is configured.
    #[must_use]
    pub fn dir(&self) -> Option<&Path> { self.dir.as_deref() }

    /// Return `Some(dir)` if the themes directory's fingerprint has
    /// changed since the last check, updating the cached fingerprint
    /// before returning. Polled per-tick from the main loop.
    pub fn take_change(&mut self) -> Option<&Path> {
        let dir = self.dir.as_deref()?;
        let current = directory_fingerprint(dir);
        if current == self.fingerprint {
            return None;
        }
        self.fingerprint = current;
        Some(dir)
    }
}

fn directory_fingerprint(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut items: Vec<(String, u64, Option<SystemTime>)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
            {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            let metadata = fs::metadata(&path).ok()?;
            let modified = metadata.modified().ok();
            Some((name, metadata.len(), modified))
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = DefaultHasher::new();
    items.hash(&mut hasher);
    hasher.finish()
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

    use super::*;

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "tui_pane_themes_watch_{label}_{n}_{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp themes dir");
        dir
    }

    fn write_file(path: &Path, contents: &str) {
        let mut f = fs::File::create(path).expect("create temp file");
        f.write_all(contents.as_bytes()).expect("write temp file");
        f.sync_all().expect("sync temp file");
    }

    const MINIMAL_DARK_FAMILY: &str = include_str!("../../themes/default_dark.toml");

    #[test]
    fn themes_watch_reports_initial_no_change() {
        let dir = temp_dir("watch_initial");
        write_file(&dir.join("a.toml"), MINIMAL_DARK_FAMILY);
        let mut watch = ThemesWatch::new(Some(dir));
        assert!(
            watch.take_change().is_none(),
            "first call should not see a change"
        );
    }

    #[test]
    fn themes_watch_detects_new_file() {
        let dir = temp_dir("watch_new");
        let mut watch = ThemesWatch::new(Some(dir.clone()));
        assert!(watch.take_change().is_none());
        write_file(&dir.join("new.toml"), MINIMAL_DARK_FAMILY);
        assert!(watch.take_change().is_some(), "addition should fire");
        assert!(
            watch.take_change().is_none(),
            "second call should see no change"
        );
    }

    #[test]
    fn themes_watch_ignores_non_toml() {
        let dir = temp_dir("watch_nontoml");
        let mut watch = ThemesWatch::new(Some(dir.clone()));
        write_file(&dir.join("notes.md"), "ignore me");
        assert!(
            watch.take_change().is_none(),
            "non-toml additions should not fire"
        );
    }
}
