//! The `Config` subsystem.
//!
//! Owns App's `cargo-port.toml` state: `current_config`,
//! `config_path`, and `config_last_seen`. Composes
//! [`tui_pane::WatchedFile<T>`] for the
//! load-watch-reload contract.

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use tui_pane::WatchedFile;

use crate::config::CargoPortConfig;
use crate::config::NavigationKeys;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;

/// Owns the parsed config plus the on-disk watch state.
pub(crate) struct Config {
    file: WatchedFile<CargoPortConfig>,
}

impl Config {
    pub fn new(path: Option<PathBuf>, current: CargoPortConfig) -> Self {
        Self {
            file: WatchedFile::new(path, current),
        }
    }

    pub const fn current(&self) -> &CargoPortConfig { &self.file.current }

    pub const fn current_mut(&mut self) -> &mut CargoPortConfig { &mut self.file.current }

    pub fn path(&self) -> Option<&Path> { self.file.path() }

    /// Refresh the cached stamp without re-parsing. Used after App
    /// itself writes the file (saving settings) so the next
    /// `try_reload` doesn't see the self-write as an external
    /// change.
    pub fn sync_stamp(&mut self) { self.file.sync_stamp(); }

    /// Return `Some(path)` if the config file's stamp has changed
    /// since the last seen value, swallowing the stamp delta. Used
    /// by `App::maybe_reload_config_from_disk`, which reloads
    /// through the framework settings store and applies its own
    /// rescan / toast logic on the outcome.
    pub fn take_stamp_change(&mut self) -> Option<&Path> { self.file.take_stamp_change() }

    // ── flag accessors ──────────────────────────────────────────────

    pub const fn lint_enabled(&self) -> bool { self.current().lint.enabled }

    pub const fn invert_scroll(&self) -> ScrollDirection { self.current().mouse.invert_scroll }

    pub const fn include_non_rust(&self) -> NonRustInclusion { self.current().tui.include_non_rust }

    pub const fn ci_run_count(&self) -> u32 { self.current().tui.ci_run_count }

    pub const fn navigation_keys(&self) -> NavigationKeys { self.current().tui.navigation_keys }

    pub fn editor(&self) -> &str { &self.current().tui.editor }

    pub fn terminal_command(&self) -> &str { &self.current().tui.terminal_command }

    pub fn discovery_shimmer_enabled(&self) -> bool {
        self.current().tui.discovery_shimmer_secs > 0.0
    }

    pub fn discovery_shimmer_duration(&self) -> Duration {
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
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn config_new_seeds_current() {
        let cfg = CargoPortConfig::default();
        let config = Config::new(None, cfg);
        assert!(config.path().is_none());
    }
}
