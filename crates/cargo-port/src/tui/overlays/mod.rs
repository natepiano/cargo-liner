//! `Overlays` subsystem — owns `UiModes` (finder),
//! the transient `inline_error` UI feedback, the
//! transient `status_flash` slot, and overlay pane render
//! states.
//!
//! Lives at `crate::tui::overlays` (outside `tui/app/`) so methods can
//! be `pub(crate)`; mend's `pub(crate)` policy forbids that visibility
//! inside `tui/app/`.
//!
//! Module split:
//! - `mod.rs` (this file) — Finder mode state, inline-error / status-flash, plus the `Overlays`
//!   struct that owns all of the above.
//! - `render_state.rs` — the Finder pane render-state type.
//! - `pane_impls.rs` — `Pane` and `Hittable` impls for Finder and the framework panes.

mod pane_impls;
mod popup;
mod render_state;

use std::time::Instant;

pub(super) use popup::PopupFrame;
pub(crate) use render_state::FinderPane;
use tui_pane::FocusedPane;

use super::integration::framework_keymap::AppPaneId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FinderMode {
    #[default]
    Hidden,
    Visible,
}

#[derive(Default)]
pub(crate) struct Overlays {
    finder:          FinderMode,
    finder_return:   Option<FocusedPane<AppPaneId>>,
    inline_error:    Option<String>,
    status_flash:    Option<(String, Instant)>,
    pub finder_pane: FinderPane,
}

impl Overlays {
    pub(crate) fn new() -> Self { Self::default() }

    // ── finder ──────────────────────────────────────────────────────

    pub(crate) const fn is_finder_open(&self) -> bool { matches!(self.finder, FinderMode::Visible) }

    pub(crate) const fn open_finder(&mut self) { self.finder = FinderMode::Visible; }

    pub(crate) const fn close_finder(&mut self) { self.finder = FinderMode::Hidden; }

    pub(crate) const fn set_finder_return(&mut self, focus: FocusedPane<AppPaneId>) {
        self.finder_return = Some(focus);
    }

    pub(crate) const fn finder_return(&self) -> Option<FocusedPane<AppPaneId>> {
        self.finder_return
    }

    pub(crate) const fn take_finder_return(&mut self) -> Option<FocusedPane<AppPaneId>> {
        self.finder_return.take()
    }

    pub(crate) fn close_settings(&mut self) { self.inline_error = None; }

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
}
