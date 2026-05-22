//! Runtime bookkeeping for an app's theme machinery.
//!
//! Holds the [`ThemesWatch`] for the user themes directory, the two
//! diagnostic-toast id slots used to dismiss "themes failed to load"
//! and "theme not found" toasts when the next reload/resolve clears
//! them, and the last OS [`Appearance`] reported by
//! [`crate::spawn_appearance_poller`].

use std::path::Path;
use std::path::PathBuf;

use super::Appearance;
use super::ThemesWatch;
use crate::toasts::ToastId;

/// Per-app theme runtime state — watch handle, two dismissable
/// toast-id slots, and last OS appearance.
pub struct ThemeRuntime {
    watch:          ThemesWatch,
    /// Toast id for the persistent "Themes reload errors" toast
    /// (parse failures under `themes/*.toml`). Dismissed when the
    /// next reload succeeds with zero failures.
    diagnostics_id: Option<ToastId>,
    /// Toast id for the persistent "Theme not found" toast emitted
    /// when the configured theme name is absent from the registry.
    /// Dismissed when the next resolve hits.
    miss_toast_id:  Option<ToastId>,
    /// Last OS appearance reported by the appearance poller. `None`
    /// until the poller emits its first event, or always `None` on
    /// platforms where detection fails.
    os_appearance:  Option<Appearance>,
}

impl ThemeRuntime {
    /// Build runtime state for the themes directory at `dir`. Pass
    /// `None` when the OS config dir cannot be resolved.
    #[must_use]
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            watch:          ThemesWatch::new(dir),
            diagnostics_id: None,
            miss_toast_id:  None,
            os_appearance:  None,
        }
    }

    /// Themes directory being watched.
    #[must_use]
    pub fn dir(&self) -> Option<&Path> { self.watch.dir() }

    /// Return `Some(dir)` if the themes directory's fingerprint has
    /// changed since the last check, swallowing the delta.
    pub fn take_change(&mut self) -> Option<&Path> { self.watch.take_change() }

    /// Set the diagnostics toast id (parse failures).
    pub const fn set_diagnostics_id(&mut self, id: Option<ToastId>) { self.diagnostics_id = id; }

    /// Take and clear the diagnostics toast id.
    pub const fn take_diagnostics_id(&mut self) -> Option<ToastId> { self.diagnostics_id.take() }

    /// Set the miss toast id (configured name not found).
    pub const fn set_miss_toast_id(&mut self, id: Option<ToastId>) { self.miss_toast_id = id; }

    /// Take and clear the miss toast id.
    pub const fn take_miss_toast_id(&mut self) -> Option<ToastId> { self.miss_toast_id.take() }

    /// Stash the last OS appearance reported by the poller.
    pub const fn set_os_appearance(&mut self, appearance: Option<Appearance>) {
        self.os_appearance = appearance;
    }

    /// Last OS appearance reported by the poller.
    #[must_use]
    pub const fn os_appearance(&self) -> Option<Appearance> { self.os_appearance }
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
        let mut runtime = ThemeRuntime::new(None);
        assert!(runtime.take_diagnostics_id().is_none());
        assert!(runtime.dir().is_none());
    }

    #[test]
    fn diagnostics_id_round_trip_set_take() {
        let mut runtime = ThemeRuntime::new(None);
        runtime.set_diagnostics_id(Some(ToastId(42)));
        assert_eq!(runtime.take_diagnostics_id(), Some(ToastId(42)));
        assert!(runtime.take_diagnostics_id().is_none());
    }

    #[test]
    fn miss_toast_id_round_trip_set_take() {
        let mut runtime = ThemeRuntime::new(None);
        runtime.set_miss_toast_id(Some(ToastId(7)));
        assert_eq!(runtime.take_miss_toast_id(), Some(ToastId(7)));
        assert!(runtime.take_miss_toast_id().is_none());
    }

    #[test]
    fn os_appearance_round_trip_set_get() {
        let mut runtime = ThemeRuntime::new(None);
        assert!(runtime.os_appearance().is_none());
        runtime.set_os_appearance(Some(Appearance::Light));
        assert_eq!(runtime.os_appearance(), Some(Appearance::Light));
        runtime.set_os_appearance(None);
        assert!(runtime.os_appearance().is_none());
    }
}
