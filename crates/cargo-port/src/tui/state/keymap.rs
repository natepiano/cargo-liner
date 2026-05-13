//! The `Keymap` subsystem.
//!
//! Owns App's keymap-file state: `current_keymap`, `keymap_path`,
//! `keymap_last_seen`, and `keymap_diagnostics_id` (the toast id
//! used to dismiss diagnostics from a previous parse failure).
//! Composes [`super::support::WatchedFile<T>`] for the
//! load-watch-reload contract.

use std::path::Path;
use std::path::PathBuf;

use tui_pane::ToastId;

use crate::keymap::ResolvedKeymap;
use crate::tui::support::WatchedFile;

/// Owns the parsed keymap plus the on-disk watch state and the
/// diagnostics-toast slot.
pub struct Keymap {
    file:           WatchedFile<ResolvedKeymap>,
    diagnostics_id: Option<ToastId>,
}

impl Keymap {
    pub fn new(path: Option<PathBuf>, current: ResolvedKeymap) -> Self {
        Self {
            file:           WatchedFile::new(path, current),
            diagnostics_id: None,
        }
    }

    #[cfg(test)]
    pub const fn current(&self) -> &ResolvedKeymap { &self.file.current }

    pub fn path(&self) -> Option<&Path> { self.file.path() }

    /// Replace the parsed keymap (used by reload paths that parse
    /// the file themselves before consulting the stamp — the
    /// existing `App::maybe_reload_keymap_from_disk` path captures
    /// `result.keymap` from `keymap::load_keymap_from_str` and
    /// installs it directly).
    pub fn replace_current(&mut self, value: ResolvedKeymap) { self.file.current = value; }

    /// Refresh the cached stamp without re-parsing. Used after App
    /// itself writes the file (defaults written for missing
    /// actions) so the next reload doesn't see the self-write.
    pub fn sync_stamp(&mut self) { self.file.sync_stamp(); }

    /// Return `Some(path)` if the keymap file's stamp has changed
    /// since the last seen value, swallowing the stamp delta.
    /// Used by `App::maybe_reload_keymap_from_disk`, which drives
    /// its own rich parser (`keymap::load_keymap_from_str`) whose
    /// `KeymapLoadResult` doesn't fit
    /// [`crate::tui::support::WatchedFile::try_reload`]'s
    /// `Result<T, String>` signature.
    pub fn take_stamp_change(&mut self) -> Option<&Path> { self.file.take_stamp_change() }

    pub const fn set_diagnostics_id(&mut self, id: Option<ToastId>) { self.diagnostics_id = id; }

    pub const fn take_diagnostics_id(&mut self) -> Option<ToastId> { self.diagnostics_id.take() }
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
    fn new_seeds_diagnostics_id_to_none() {
        let mut keymap = Keymap::new(None, ResolvedKeymap::defaults());
        assert!(keymap.take_diagnostics_id().is_none());
        assert!(keymap.path().is_none());
    }

    #[test]
    fn diagnostics_id_round_trip_set_take() {
        let mut keymap = Keymap::new(None, ResolvedKeymap::defaults());
        keymap.set_diagnostics_id(Some(ToastId(42)));
        let taken = keymap.take_diagnostics_id();
        assert_eq!(taken, Some(ToastId(42)));
        assert!(
            keymap.take_diagnostics_id().is_none(),
            "take must clear the slot"
        );
    }

    #[test]
    fn replace_current_swaps_in_new_keymap() {
        let mut keymap = Keymap::new(None, ResolvedKeymap::defaults());
        let next = ResolvedKeymap::defaults();
        keymap.replace_current(next);
        // We can't easily compare ResolvedKeymap structurally — the
        // contract is just that replace_current mutates in place
        // without touching the stamp or diagnostics_id.
        assert!(keymap.take_diagnostics_id().is_none());
        assert!(keymap.path().is_none());
    }
}
