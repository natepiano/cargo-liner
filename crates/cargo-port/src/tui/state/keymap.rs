//! The `Keymap` subsystem.
//!
//! Owns App's keymap-file state: `current_keymap`, `keymap_path`,
//! `keymap_last_seen`, keymap diagnostics/warning toast ids, and
//! the current parsed keymap.
//! Composes [`tui_pane::WatchedFile<T>`] for the
//! load-watch-reload contract.

use std::path::Path;

use tui_pane::ToastId;
use tui_pane::WatchedFile;

use crate::config::CargoPortConfigurationPathResolution;
use crate::tui::keymap::ResolvedKeymap;

/// The keymap file path available to this subsystem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::tui) enum KeymapFilePath<'a> {
    Unavailable,
    Resolved(&'a Path),
}

/// The result of the consumed stamp check.
pub(in crate::tui) enum KeymapReloadCheck<'a> {
    NoReload,
    Changed(&'a Path),
}

/// The diagnostics toast identity retained by this subsystem.
#[derive(Debug, Eq, PartialEq)]
pub(in crate::tui) enum KeymapDiagnosticsNotice {
    Untracked,
    Tracked(ToastId),
}

/// The warning toast identity retained by this subsystem.
#[derive(Debug, Eq, PartialEq)]
pub(in crate::tui) enum KeymapWarningsNotice {
    Untracked,
    Tracked(ToastId),
}

/// Owns the parsed keymap plus the on-disk watch state and the
/// diagnostics-toast slot.
pub(in crate::tui) struct Keymap {
    file:           WatchedFile<ResolvedKeymap>,
    diagnostics_id: KeymapDiagnosticsNotice,
    warnings_id:    KeymapWarningsNotice,
}

impl Keymap {
    pub(in crate::tui) fn new(
        path_resolution: CargoPortConfigurationPathResolution,
        current: ResolvedKeymap,
    ) -> Self {
        Self {
            file:           WatchedFile::new(path_resolution.into_external_path(), current),
            diagnostics_id: KeymapDiagnosticsNotice::Untracked,
            warnings_id:    KeymapWarningsNotice::Untracked,
        }
    }

    #[cfg(test)]
    pub const fn current(&self) -> &ResolvedKeymap { &self.file.current }

    pub(in crate::tui) fn path(&self) -> KeymapFilePath<'_> {
        self.file
            .path()
            .map_or(KeymapFilePath::Unavailable, KeymapFilePath::Resolved)
    }

    /// Replace the parsed keymap (used by reload paths that parse
    /// the file themselves before consulting the stamp — the
    /// existing `App::maybe_reload_keymap_from_disk` path captures
    /// `result.keymap` from `keymap::load_keymap_from_str` and
    /// installs it directly).
    pub(in crate::tui) fn replace_current(&mut self, value: ResolvedKeymap) {
        self.file.current = value;
    }

    /// Refresh the cached stamp without re-parsing. Used after App
    /// itself writes the file (defaults written for missing
    /// actions) so the next reload doesn't see the self-write.
    pub(in crate::tui) fn sync_stamp(&mut self) { self.file.sync_stamp(); }

    /// Consume the framework stamp check without classifying filesystem errors.
    /// `App::maybe_reload_keymap_from_disk` uses `keymap::load_keymap_from_str`
    /// because `KeymapLoadResult` does not fit the
    /// `tui_pane::WatchedFile::take_stamp_change` signature.
    pub(in crate::tui) fn take_stamp_change(&mut self) -> KeymapReloadCheck<'_> {
        self.file
            .take_stamp_change()
            .map_or(KeymapReloadCheck::NoReload, KeymapReloadCheck::Changed)
    }

    pub(in crate::tui) const fn set_diagnostics_id(&mut self, id: ToastId) {
        self.diagnostics_id = KeymapDiagnosticsNotice::Tracked(id);
    }

    pub(in crate::tui) const fn take_diagnostics_id(&mut self) -> KeymapDiagnosticsNotice {
        std::mem::replace(&mut self.diagnostics_id, KeymapDiagnosticsNotice::Untracked)
    }

    pub(in crate::tui) const fn set_warnings_id(&mut self, id: ToastId) {
        self.warnings_id = KeymapWarningsNotice::Tracked(id);
    }

    pub(in crate::tui) const fn take_warnings_id(&mut self) -> KeymapWarningsNotice {
        std::mem::replace(&mut self.warnings_id, KeymapWarningsNotice::Untracked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_without_notice_ids() {
        let mut keymap = Keymap::new(
            CargoPortConfigurationPathResolution::PlatformDirectoryUnavailable,
            ResolvedKeymap::defaults(),
        );
        assert_eq!(
            keymap.take_diagnostics_id(),
            KeymapDiagnosticsNotice::Untracked
        );
        assert_eq!(keymap.take_warnings_id(), KeymapWarningsNotice::Untracked);
        assert_eq!(keymap.path(), KeymapFilePath::Unavailable);
    }

    #[test]
    fn diagnostics_id_round_trip_set_take() {
        let mut keymap = Keymap::new(
            CargoPortConfigurationPathResolution::PlatformDirectoryUnavailable,
            ResolvedKeymap::defaults(),
        );
        keymap.set_diagnostics_id(ToastId(42));
        let taken = keymap.take_diagnostics_id();
        assert_eq!(taken, KeymapDiagnosticsNotice::Tracked(ToastId(42)));
        assert_eq!(
            keymap.take_diagnostics_id(),
            KeymapDiagnosticsNotice::Untracked
        );
    }

    #[test]
    fn warnings_id_round_trip_set_take() {
        let mut keymap = Keymap::new(
            CargoPortConfigurationPathResolution::PlatformDirectoryUnavailable,
            ResolvedKeymap::defaults(),
        );
        keymap.set_warnings_id(ToastId(42));
        let taken = keymap.take_warnings_id();
        assert_eq!(taken, KeymapWarningsNotice::Tracked(ToastId(42)));
        assert_eq!(keymap.take_warnings_id(), KeymapWarningsNotice::Untracked);
    }

    #[test]
    fn replace_current_swaps_in_new_keymap() {
        let mut keymap = Keymap::new(
            CargoPortConfigurationPathResolution::PlatformDirectoryUnavailable,
            ResolvedKeymap::defaults(),
        );
        let next = ResolvedKeymap::defaults();
        keymap.replace_current(next);
        // The replacement preserves the watcher path and both notice states.
        // `ResolvedKeymap` has no structural comparison.
        // `replace_current` changes `file.current` in place without changing the
        // watched stamp, `diagnostics_id`, or `warnings_id`.
        assert_eq!(
            keymap.take_diagnostics_id(),
            KeymapDiagnosticsNotice::Untracked
        );
        assert_eq!(keymap.take_warnings_id(), KeymapWarningsNotice::Untracked);
        assert_eq!(keymap.path(), KeymapFilePath::Unavailable);
    }

    #[test]
    fn configuration_paths_without_a_filesystem_path_leave_the_watcher_unset() {
        for path_resolution in [
            CargoPortConfigurationPathResolution::PlatformDirectoryUnavailable,
            CargoPortConfigurationPathResolution::InvalidEmptyOverride,
        ] {
            let keymap = Keymap::new(path_resolution, ResolvedKeymap::defaults());
            assert_eq!(keymap.path(), KeymapFilePath::Unavailable);
        }
    }
}
