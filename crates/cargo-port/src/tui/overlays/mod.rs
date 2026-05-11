//! `Overlays` subsystem — owns `UiModes` (finder / settings / keymap),
//! the transient `inline_error` UI feedback, the
//! transient `status_flash` slot, and the three overlay pane render
//! states.
//!
//! Lives at `crate::tui::overlays` (outside `tui/app/`) so methods can
//! be `pub(crate)`; mend's `pub(crate)` policy forbids that visibility
//! inside `tui/app/`.
//!
//! Module split:
//! - `mod.rs` (this file) — mode state (Finder / Settings / Keymap), inline-error / status-flash,
//!   plus the `Overlays` struct that owns all of the above.
//! - `render_state.rs` — the three pane-render-state types (`KeymapPane`, `SettingsPane`,
//!   `FinderPane`) plus the accessor `impl Overlays` block. They live with `Overlays` because
//!   `Overlays` already owns the open/closed mode state for each.
//! - `pane_impls.rs` — `Pane` and `Hittable` impls for the three render-state types.

mod pane_impls;
mod render_state;

use std::time::Instant;

pub(crate) use render_state::FinderPane;
pub(crate) use render_state::KeymapPane;
pub(crate) use render_state::SettingsPane;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FinderMode {
    #[default]
    Hidden,
    Visible,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SettingsMode {
    #[default]
    Hidden,
    Browsing,
    Editing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum KeymapMode {
    #[default]
    Hidden,
    Browsing,
    AwaitingKey,
}

#[derive(Default)]
pub(crate) struct Overlays {
    finder:            FinderMode,
    settings:          SettingsMode,
    keymap:            KeymapMode,
    inline_error:      Option<String>,
    status_flash:      Option<(String, Instant)>,
    pub keymap_pane:   KeymapPane,
    pub settings_pane: SettingsPane,
    pub finder_pane:   FinderPane,
}

impl Overlays {
    pub(crate) fn new() -> Self { Self::default() }

    // ── finder ──────────────────────────────────────────────────────

    pub(crate) const fn is_finder_open(&self) -> bool { matches!(self.finder, FinderMode::Visible) }

    pub(crate) const fn open_finder(&mut self) { self.finder = FinderMode::Visible; }

    pub(crate) const fn close_finder(&mut self) { self.finder = FinderMode::Hidden; }

    // ── settings ────────────────────────────────────────────────────

    pub(crate) const fn is_settings_open(&self) -> bool {
        !matches!(self.settings, SettingsMode::Hidden)
    }

    pub(crate) const fn is_settings_editing(&self) -> bool {
        matches!(self.settings, SettingsMode::Editing)
    }

    #[cfg(test)]
    pub(crate) const fn open_settings(&mut self) { self.settings = SettingsMode::Browsing; }

    pub(crate) fn close_settings(&mut self) {
        self.settings = SettingsMode::Hidden;
        self.inline_error = None;
    }

    pub(crate) fn begin_settings_editing(&mut self) {
        self.settings = SettingsMode::Editing;
        self.inline_error = None;
    }

    pub(crate) fn end_settings_editing(&mut self) {
        self.settings = SettingsMode::Browsing;
        self.inline_error = None;
    }

    // ── keymap ──────────────────────────────────────────────────────

    pub(crate) const fn is_keymap_open(&self) -> bool { !matches!(self.keymap, KeymapMode::Hidden) }

    pub(crate) const fn keymap_is_awaiting(&self) -> bool {
        matches!(self.keymap, KeymapMode::AwaitingKey)
    }

    pub(crate) fn close_keymap(&mut self) {
        self.keymap = KeymapMode::Hidden;
        self.inline_error = None;
    }

    pub(crate) fn keymap_begin_awaiting(&mut self) {
        self.keymap = KeymapMode::AwaitingKey;
        self.inline_error = None;
    }

    pub(crate) fn keymap_end_awaiting(&mut self) {
        self.keymap = KeymapMode::Browsing;
        self.inline_error = None;
    }

    // ── inline error ────────────────────────────────────────────────

    pub(crate) const fn inline_error(&self) -> Option<&String> { self.inline_error.as_ref() }

    pub(crate) fn set_inline_error(&mut self, error: impl Into<String>) {
        self.inline_error = Some(error.into());
    }

    pub(crate) fn clear_inline_error(&mut self) { self.inline_error = None; }

    // ── status flash ────────────────────────────────────────────────

    pub(crate) const fn status_flash(&self) -> Option<&(String, Instant)> {
        self.status_flash.as_ref()
    }

    pub(crate) fn set_status_flash(&mut self, message: String, at: Instant) {
        self.status_flash = Some((message, at));
    }

    // ── render-state accessors ──────────────────────────────────────
    //
    // Each accessor returns the small viewport-holding struct for one
    // overlay pane. The three structs live in `render_state.rs`; their
    // `Pane` / `Hittable` impls live in `pane_impls.rs`.
}
