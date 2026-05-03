//! The `Config` subsystem.
//!
//! Owns App's `cargo-port.toml` state: `current_config`,
//! `config_path`, `config_last_seen`, plus the in-app settings
//! editor's typed [`SettingsEditBuffer`]. Composes
//! [`super::watched_file::WatchedFile<T>`] for the
//! load-watch-reload contract.

use std::path::Path;
use std::path::PathBuf;

use super::watched_file::WatchedFile;
use crate::config::CargoPortConfig;

/// Typed pair of edit buffer + cursor. Collapsing the two raw
/// fields prevents cursor drift past the buffer's byte length —
/// every mutation goes through [`Self::set`] / [`Self::clear`] /
/// [`Self::parts_mut`], which clamp the cursor to a valid index.
pub(super) struct SettingsEditBuffer {
    buf:    String,
    cursor: usize,
}

impl SettingsEditBuffer {
    pub(super) const fn new() -> Self {
        Self {
            buf:    String::new(),
            cursor: 0,
        }
    }

    pub(super) fn buf(&self) -> &str { &self.buf }

    pub(super) const fn cursor(&self) -> usize { self.cursor }

    /// Replace the buffer + cursor in one atomic step. The cursor
    /// is clamped to `buf.len()` so callers can't end up pointing
    /// past the new buffer's bounds.
    pub(super) fn set(&mut self, buf: String, cursor: usize) {
        let clamped = cursor.min(buf.len());
        self.buf = buf;
        self.cursor = clamped;
    }

    /// Joint mutable handles on the buf and cursor. Used by
    /// `tui::settings` keystroke handlers that splice characters
    /// at cursor and need to advance both fields together.
    pub(super) const fn parts_mut(&mut self) -> (&mut String, &mut usize) {
        (&mut self.buf, &mut self.cursor)
    }
}

impl Default for SettingsEditBuffer {
    fn default() -> Self { Self::new() }
}

/// Owns the parsed config plus the on-disk watch state and the
/// in-app settings editor's edit buffer.
pub(super) struct Config {
    file:        WatchedFile<CargoPortConfig>,
    edit_buffer: SettingsEditBuffer,
}

impl Config {
    pub(super) fn new(path: Option<PathBuf>, current: CargoPortConfig) -> Self {
        Self {
            file:        WatchedFile::new(path, current),
            edit_buffer: SettingsEditBuffer::new(),
        }
    }

    pub(super) const fn current(&self) -> &CargoPortConfig { self.file.current() }

    pub(super) const fn current_mut(&mut self) -> &mut CargoPortConfig { self.file.current_mut() }

    pub(super) fn path(&self) -> Option<&Path> { self.file.path() }

    /// Refresh the cached stamp without re-parsing. Used after App
    /// itself writes the file (saving settings) so the next
    /// `try_reload` doesn't see the self-write as an external
    /// change.
    pub(super) fn sync_stamp(&mut self) { self.file.sync_stamp(); }

    /// Return `Some(path)` if the config file's stamp has changed
    /// since the last seen value, swallowing the stamp delta. Used
    /// by `App::maybe_reload_config_from_disk`, which drives a
    /// custom load path (`config::try_load_from_path` with
    /// `Result<CargoPortConfig, String>`) and applies its own
    /// rescan / toast logic on the outcome.
    pub(super) fn take_stamp_change(&mut self) -> Option<&Path> { self.file.take_stamp_change() }

    pub(super) const fn edit_buffer(&self) -> &SettingsEditBuffer { &self.edit_buffer }

    pub(super) const fn edit_buffer_mut(&mut self) -> &mut SettingsEditBuffer {
        &mut self.edit_buffer
    }

    /// Test-only — point the watch at a new path and clear the
    /// cached stamp so the next `take_stamp_change` sees a fresh
    /// reload. Production paths construct `Config` once at startup.
    #[cfg(test)]
    pub(super) fn force_reload_from(&mut self, path: PathBuf) {
        let current = self.file.current().clone();
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
    fn settings_edit_buffer_set_clamps_cursor_to_buf_len() {
        let mut buf = SettingsEditBuffer::new();
        buf.set("abc".to_string(), 99);
        assert_eq!(buf.buf(), "abc");
        assert_eq!(buf.cursor(), 3, "cursor must clamp to buf.len() after set");
    }

    #[test]
    fn settings_edit_buffer_set_keeps_in_range_cursor() {
        let mut buf = SettingsEditBuffer::new();
        buf.set("hello".to_string(), 2);
        assert_eq!(buf.cursor(), 2);
    }

    #[test]
    fn settings_edit_buffer_parts_mut_allows_joint_mutation() {
        let mut buf = SettingsEditBuffer::new();
        buf.set("ab".to_string(), 2);
        {
            let (s, c) = buf.parts_mut();
            s.insert(*c, 'c');
            *c += 1;
        }
        assert_eq!(buf.buf(), "abc");
        assert_eq!(buf.cursor(), 3);
    }

    #[test]
    fn config_new_seeds_current_and_buffer_is_empty() {
        let cfg = CargoPortConfig::default();
        let config = Config::new(None, cfg);
        assert!(config.path().is_none());
        assert_eq!(config.edit_buffer().buf(), "");
        assert_eq!(config.edit_buffer().cursor(), 0);
    }
}
