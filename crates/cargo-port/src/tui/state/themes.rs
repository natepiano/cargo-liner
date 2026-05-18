//! The `Themes` subsystem.
//!
//! Owns App's user-themes directory watch state and the
//! diagnostics-toast slot used to dismiss any previous parse-error
//! toast when the registry reloads cleanly. Wraps
//! [`crate::themes::ThemesWatch`].

use std::path::Path;
use std::path::PathBuf;

use tui_pane::ToastId;

use crate::themes::ThemesWatch;

/// Owns the themes-directory watch plus the diagnostics-toast slot.
pub struct Themes {
    watch:          ThemesWatch,
    diagnostics_id: Option<ToastId>,
}

impl Themes {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            watch:          ThemesWatch::new(dir),
            diagnostics_id: None,
        }
    }

    pub fn dir(&self) -> Option<&Path> { self.watch.dir() }

    /// Return `Some(dir)` if the themes directory's fingerprint has
    /// changed since the last check, swallowing the delta.
    pub fn take_change(&mut self) -> Option<&Path> { self.watch.take_change() }

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
}
