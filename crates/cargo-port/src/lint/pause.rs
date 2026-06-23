//! Persisted lint-pause flag.
//!
//! Pause is in-memory app state (`Lint::is_paused`), but it must survive a
//! restart: a session paused when the app exits comes back up paused. The flag
//! is a marker file at the lint cache root — present iff paused — written on
//! the pause/resume toggle and read once at startup. A marker (rather than the
//! settings TOML) keeps this transient toggle out of the user-edited config and
//! off the settings-save path, which would respawn the lint runtime on every
//! press.

use crate::cache_paths;
use crate::config::CargoPortConfig;

/// Write the pause marker. Best-effort: a failure to persist leaves the
/// in-memory pause in effect for this session and is not surfaced.
pub(crate) fn record_paused(config: &CargoPortConfig) {
    let marker = cache_paths::lint_pause_marker_for(config);
    if let Some(parent) = marker.as_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(marker.as_path(), []);
}

/// Remove the pause marker. A missing marker is already the resumed state, so
/// a `NotFound` error is success.
pub(crate) fn record_resumed(config: &CargoPortConfig) {
    let _ = std::fs::remove_file(cache_paths::lint_pause_marker_for(config).as_path());
}

/// Whether the persisted pause marker is set. Read once at startup to decide
/// whether to resume the session paused.
pub(crate) fn is_set(config: &CargoPortConfig) -> bool {
    cache_paths::lint_pause_marker_for(config)
        .as_path()
        .is_file()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::Path;

    use super::*;

    fn config_with_cache_root(root: &Path) -> CargoPortConfig {
        let mut config = CargoPortConfig::default();
        config.cache.root = root.to_string_lossy().to_string();
        config
    }

    #[test]
    fn marker_round_trips_through_record_and_clear() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let config = config_with_cache_root(cache_dir.path());

        assert!(!is_set(&config), "no marker before recording pause");

        record_paused(&config);
        assert!(is_set(&config), "marker present after recording pause");

        record_resumed(&config);
        assert!(!is_set(&config), "marker gone after recording resume");
    }

    #[test]
    fn record_resumed_is_a_noop_when_marker_absent() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let config = config_with_cache_root(cache_dir.path());

        record_resumed(&config);
        assert!(!is_set(&config));
    }
}
