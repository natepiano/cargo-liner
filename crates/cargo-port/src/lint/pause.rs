//! Persisted global and project-scoped lint-pause flags.
//!
//! Global pause is in-memory app state (`Lint::is_paused`), but it must survive
//! a restart. Project pauses use a marker inside that project's lint-cache
//! directory, so a workspace member resolves to the same pause state as the
//! workspace lint run that owns it. Markers keep these transient toggles out of
//! the user-edited settings TOML and off the settings-save path, which would
//! respawn the lint runtime on every press.

use std::path::Path;

use super::paths;
use crate::cache_paths;
use crate::config::CargoPortConfig;
use crate::project::AbsolutePath;

/// Write the pause marker. Best-effort: a failure to persist leaves the
/// in-memory pause in effect for this session and is not surfaced.
pub(crate) fn record_paused(cargo_port_config: &CargoPortConfig) {
    let marker = cache_paths::lint_pause_marker_for(cargo_port_config);
    if let Some(parent) = marker.as_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(marker.as_path(), []);
}

/// Remove the pause marker. A missing marker is already the resumed state, so
/// a `NotFound` error is success.
pub(crate) fn record_resumed(cargo_port_config: &CargoPortConfig) {
    let _ = std::fs::remove_file(cache_paths::lint_pause_marker_for(cargo_port_config).as_path());
}

/// Whether the persisted pause marker is set. Read once at startup to decide
/// whether to resume the session paused.
pub(crate) fn is_set(cargo_port_config: &CargoPortConfig) -> bool {
    cache_paths::lint_pause_marker_for(cargo_port_config)
        .as_path()
        .is_file()
}

/// Write a project-scoped pause marker. Best-effort, like the global marker.
pub(crate) fn record_project_paused(cargo_port_config: &CargoPortConfig, project_root: &Path) {
    let cache_root = cache_paths::lint_runs_root_for(cargo_port_config);
    let marker = project_pause_marker_under(cache_root.as_path(), project_root);
    if let Some(parent) = marker.as_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(marker.as_path(), []);
}

/// Remove a project-scoped pause marker. Missing means resumed.
pub(crate) fn record_project_resumed(cargo_port_config: &CargoPortConfig, project_root: &Path) {
    let cache_root = cache_paths::lint_runs_root_for(cargo_port_config);
    let marker = project_pause_marker_under(cache_root.as_path(), project_root);
    let _ = std::fs::remove_file(marker.as_path());
}

/// Whether a project's persisted pause marker is set under an explicit cache
/// root. The lint supervisor uses this when it creates or replaces workers.
pub(crate) fn is_project_set_under(cache_root: &Path, project_root: &Path) -> bool {
    project_pause_marker_under(cache_root, project_root)
        .as_path()
        .is_file()
}

fn project_pause_marker_under(cache_root: &Path, project_root: &Path) -> AbsolutePath {
    paths::project_dir_under(cache_root, project_root)
        .join(crate::constants::LINTS_PAUSED_MARKER)
        .into()
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
        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config.cache.root = root.to_string_lossy().to_string();
        cargo_port_config
    }

    #[test]
    fn marker_round_trips_through_record_and_clear() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let cargo_port_config = config_with_cache_root(cache_dir.path());

        assert!(
            !is_set(&cargo_port_config),
            "no marker before recording pause"
        );

        record_paused(&cargo_port_config);
        assert!(
            is_set(&cargo_port_config),
            "marker present after recording pause"
        );

        record_resumed(&cargo_port_config);
        assert!(
            !is_set(&cargo_port_config),
            "marker gone after recording resume"
        );
    }

    #[test]
    fn record_resumed_is_a_noop_when_marker_absent() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let cargo_port_config = config_with_cache_root(cache_dir.path());

        record_resumed(&cargo_port_config);
        assert!(!is_set(&cargo_port_config));
    }

    #[test]
    fn project_marker_round_trips_without_affecting_other_projects() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("project dir");
        let other_project_dir = tempfile::tempdir().expect("other project dir");
        let cargo_port_config = config_with_cache_root(cache_dir.path());

        record_project_paused(&cargo_port_config, project_dir.path());
        let cache_root = cache_paths::lint_runs_root_for(&cargo_port_config);
        assert!(is_project_set_under(
            cache_root.as_path(),
            project_dir.path()
        ));
        assert!(!is_project_set_under(
            cache_root.as_path(),
            other_project_dir.path()
        ));

        record_project_resumed(&cargo_port_config, project_dir.path());
        assert!(!is_project_set_under(
            cache_root.as_path(),
            project_dir.path()
        ));
    }
}
