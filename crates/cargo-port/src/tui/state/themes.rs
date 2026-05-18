//! The `Themes` subsystem.
//!
//! Owns App's user-themes directory watch state, the diagnostics-toast
//! slot used to dismiss any previous parse-error toast when the
//! registry reloads cleanly, the persistent "Theme not found" toast id
//! slot from theme-resolution misses, and the last-known OS appearance
//! reported by the Phase 5 `dark-light` poller (`None` until that
//! poller ships or on platforms where detection fails). Wraps
//! [`crate::themes::ThemesWatch`].

use std::path::Path;
use std::path::PathBuf;

use tui_pane::Appearance;
use tui_pane::ToastId;

use crate::themes::ThemesWatch;

/// Owns the themes-directory watch plus the diagnostics-toast slots
/// and the cached OS appearance state.
pub struct Themes {
    watch:          ThemesWatch,
    /// Toast id for the persistent "Themes reload errors" toast
    /// (parse failures under `themes/*.toml`). Dismissed when the
    /// next reload succeeds with zero failures.
    diagnostics_id: Option<ToastId>,
    /// Toast id for the persistent "Theme not found" toast emitted
    /// when `appearance.{light,dark}_theme` references an id absent
    /// from the registry. Dismissed when the next resolve hits.
    miss_toast_id:  Option<ToastId>,
    /// Last OS appearance reported by the Phase 5 poller. `None`
    /// until the poller emits its first `BackgroundMsg::AppearanceChanged`
    /// (or always `None` on platforms where detection fails).
    os_appearance:  Option<Appearance>,
}

impl Themes {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            watch:          ThemesWatch::new(dir),
            diagnostics_id: None,
            miss_toast_id:  None,
            os_appearance:  None,
        }
    }

    pub fn dir(&self) -> Option<&Path> { self.watch.dir() }

    /// Return `Some(dir)` if the themes directory's fingerprint has
    /// changed since the last check, swallowing the delta.
    pub fn take_change(&mut self) -> Option<&Path> { self.watch.take_change() }

    pub const fn set_diagnostics_id(&mut self, id: Option<ToastId>) { self.diagnostics_id = id; }

    pub const fn take_diagnostics_id(&mut self) -> Option<ToastId> { self.diagnostics_id.take() }

    pub const fn set_miss_toast_id(&mut self, id: Option<ToastId>) { self.miss_toast_id = id; }

    pub const fn take_miss_toast_id(&mut self) -> Option<ToastId> { self.miss_toast_id.take() }

    pub const fn set_os_appearance(&mut self, appearance: Option<Appearance>) {
        self.os_appearance = appearance;
    }

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
        let mut themes = Themes::new(None);
        assert!(themes.take_diagnostics_id().is_none());
        assert!(themes.dir().is_none());
    }

    #[test]
    fn diagnostics_id_round_trip_set_take() {
        let mut themes = Themes::new(None);
        themes.set_diagnostics_id(Some(ToastId(42)));
        assert_eq!(themes.take_diagnostics_id(), Some(ToastId(42)));
        assert!(themes.take_diagnostics_id().is_none());
    }

    #[test]
    fn miss_toast_id_round_trip_set_take() {
        let mut themes = Themes::new(None);
        themes.set_miss_toast_id(Some(ToastId(7)));
        assert_eq!(themes.take_miss_toast_id(), Some(ToastId(7)));
        assert!(themes.take_miss_toast_id().is_none());
    }

    #[test]
    fn os_appearance_round_trip_set_get() {
        let mut themes = Themes::new(None);
        assert!(themes.os_appearance().is_none());
        themes.set_os_appearance(Some(Appearance::Light));
        assert_eq!(themes.os_appearance(), Some(Appearance::Light));
        themes.set_os_appearance(None);
        assert!(themes.os_appearance().is_none());
    }
}
