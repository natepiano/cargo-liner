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
//! - `pane.rs` — `Pane` and `Hittable` impls for Finder and the framework panes.

mod pane_impls;
mod render_state;

use std::time::Instant;

pub(crate) use render_state::FinderPane;
use tui_pane::FocusedPane;
pub(super) use tui_pane::PopupFrame;

use super::integration::AppPaneId;
use super::sccache::SccachePane;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FinderMode {
    #[default]
    Hidden,
    Visible,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SccacheMode {
    #[default]
    Hidden,
    Visible,
}

#[derive(Default)]
pub(super) struct Overlays {
    finder:           FinderMode,
    sccache:          SccacheMode,
    finder_return:    Option<FocusedPane<AppPaneId>>,
    inline_error:     Option<String>,
    status_flash:     Option<(String, Instant)>,
    pub finder_pane:  FinderPane,
    pub sccache_pane: SccachePane,
}

impl Overlays {
    pub(super) fn new() -> Self { Self::default() }

    // ── finder ──────────────────────────────────────────────────────

    pub(super) const fn is_finder_open(&self) -> bool { matches!(self.finder, FinderMode::Visible) }

    pub(super) const fn open_finder(&mut self) { self.finder = FinderMode::Visible; }

    pub(super) const fn close_finder(&mut self) { self.finder = FinderMode::Hidden; }

    pub(super) const fn is_sccache_open(&self) -> bool {
        matches!(self.sccache, SccacheMode::Visible)
    }

    pub(super) const fn open_sccache(&mut self) { self.sccache = SccacheMode::Visible; }

    pub(super) const fn close_sccache(&mut self) { self.sccache = SccacheMode::Hidden; }

    pub(super) const fn set_finder_return(&mut self, focus: FocusedPane<AppPaneId>) {
        self.finder_return = Some(focus);
    }

    pub(super) const fn finder_return(&self) -> Option<FocusedPane<AppPaneId>> {
        self.finder_return
    }

    pub(super) const fn take_finder_return(&mut self) -> Option<FocusedPane<AppPaneId>> {
        self.finder_return.take()
    }

    pub(super) fn close_settings(&mut self) { self.inline_error = None; }

    // ── inline error ────────────────────────────────────────────────

    pub(super) const fn inline_error(&self) -> Option<&String> { self.inline_error.as_ref() }

    pub(super) fn set_inline_error(&mut self, error: impl Into<String>) {
        self.inline_error = Some(error.into());
    }

    pub(super) fn clear_inline_error(&mut self) { self.inline_error = None; }

    // ── status flash ────────────────────────────────────────────────

    pub(super) const fn status_flash(&self) -> Option<&(String, Instant)> {
        self.status_flash.as_ref()
    }

    pub(super) fn set_status_flash(&mut self, message: String, at: Instant) {
        self.status_flash = Some((message, at));
    }

    // ── render-state accessors ──────────────────────────────────────
}
