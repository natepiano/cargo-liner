//! The `Config` subsystem.
//!
//! Owns App's `cargo-port.toml` state: `current_config`,
//! `config_path`, and `config_last_seen`. Composes
//! [`tui_pane::WatchedFile<T>`] for the
//! load-watch-reload contract.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::time::Duration;

use tui_pane::WatchedFile;

use crate::config::CargoPortConfig;
use crate::config::CargoPortConfigurationPathResolution;
use crate::config::EdgeScroll;
use crate::config::NavigationKeys;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;
use crate::scan::ExcludeDirs;

/// Owns the parsed config plus the on-disk watch state.
pub(crate) struct Config {
    file: WatchedFile<CargoPortConfig>,
}

impl Config {
    pub(crate) fn new(
        path_resolution: CargoPortConfigurationPathResolution,
        current: CargoPortConfig,
    ) -> Self {
        Self {
            file: WatchedFile::new(path_resolution.into_external_path(), current),
        }
    }

    pub(crate) const fn current(&self) -> &CargoPortConfig { &self.file.current }

    pub(crate) const fn current_mut(&mut self) -> &mut CargoPortConfig { &mut self.file.current }

    pub(crate) fn path(&self) -> Option<&Path> { self.file.path() }

    /// Refresh the cached stamp without re-parsing. Used after App
    /// itself writes the file (saving settings) so the next
    /// `try_reload` doesn't see the self-write as an external
    /// change.
    pub(crate) fn sync_stamp(&mut self) { self.file.sync_stamp(); }

    /// Return `Some(path)` if the config file's stamp has changed
    /// since the last seen value, swallowing the stamp delta. Used
    /// by `App::maybe_reload_config_from_disk`, which reloads
    /// through the framework settings store and applies its own
    /// rescan / toast logic on the outcome.
    pub(crate) fn take_stamp_change(&mut self) -> Option<&Path> { self.file.take_stamp_change() }

    // ── flag accessors ──────────────────────────────────────────────

    pub(crate) const fn lint_enabled(&self) -> bool { self.current().lint.enabled.is_enabled() }

    pub(crate) const fn invert_scroll(&self) -> ScrollDirection {
        self.current().mouse.invert_scroll
    }

    pub(crate) const fn include_non_rust(&self) -> NonRustInclusion {
        self.current().tui.include_non_rust
    }

    pub(crate) const fn ci_run_count(&self) -> u32 { self.current().tui.ci_run_count }

    pub(crate) fn exclude_dirs(&self) -> ExcludeDirs {
        ExcludeDirs::from(self.current().tui.exclude_dirs.as_slice())
    }

    pub(crate) const fn navigation_keys(&self) -> NavigationKeys {
        self.current().tui.navigation_keys
    }

    pub(crate) const fn edge_scroll(&self) -> EdgeScroll { self.current().tui.edge_scroll }

    pub(crate) fn editor(&self) -> &str { &self.current().tui.editor }

    pub(crate) fn terminal_command(&self) -> &str { &self.current().tui.terminal_command }

    pub(crate) fn discovery_shimmer_enabled(&self) -> bool {
        self.current().tui.discovery_shimmer_secs > 0.0
    }

    pub(crate) fn discovery_shimmer_duration(&self) -> Duration {
        Duration::from_secs_f64(self.current().tui.discovery_shimmer_secs)
    }

    /// Test-only — point the watch at a new path and clear the
    /// cached stamp so the next `take_stamp_change` sees a fresh
    /// reload. Production paths construct `Config` once at startup.
    #[cfg(test)]
    pub fn force_reload_from(&mut self, path: PathBuf) {
        let current = self.file.current.clone();
        self.file = WatchedFile::new(Some(path), current);
        // Replace WatchedFile constructor sets stamp to whatever's
        // on disk now; clear it so the next take_stamp_change sees
        // a delta and triggers reload.
        self.file.clear_stamp_for_test();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_new_seeds_current() {
        let cargo_port_config = CargoPortConfig::default();
        let config = Config::new(
            CargoPortConfigurationPathResolution::PlatformDirectoryUnavailable,
            cargo_port_config,
        );
        assert!(config.path().is_none());
    }

    #[test]
    fn configuration_paths_without_a_filesystem_path_leave_the_watcher_unset() {
        for path_resolution in [
            CargoPortConfigurationPathResolution::PlatformDirectoryUnavailable,
            CargoPortConfigurationPathResolution::InvalidEmptyOverride,
        ] {
            let config = Config::new(path_resolution, CargoPortConfig::default());
            assert!(config.path().is_none());
        }
    }
}
