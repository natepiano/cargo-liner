//! `Focus` subsystem — owns `focused_pane`, `overlay_return`, and the
//! visited-pane set.
//!
//! Hosting at `crate::tui::focus` (outside `tui/app/`) lets methods
//! be `pub(crate)` per the post-Phase-4 location decision; mend's
//! `pub(crate)` policy forbids that visibility inside `tui/app/`.

use std::collections::HashSet;

use super::pane::PaneFocusState;
use super::panes::PaneId;

#[derive(Debug)]
pub(crate) struct Focus {
    focused_pane:   PaneId,
    overlay_return: Option<PaneId>,
    visited:        HashSet<PaneId>,
}

impl Focus {
    pub(crate) fn new(initial: PaneId) -> Self {
        Self {
            focused_pane:   initial,
            overlay_return: None,
            visited:        std::iter::once(initial).collect(),
        }
    }

    pub(crate) const fn current(&self) -> PaneId { self.focused_pane }

    pub(crate) const fn overlay_return(&self) -> Option<PaneId> { self.overlay_return }

    pub(crate) fn is(&self, pane: PaneId) -> bool { self.focused_pane == pane }

    /// Overlay-aware base focus. When an overlay is active, the
    /// `overlay_return` slot remembers the underlying pane.
    pub(crate) fn base(&self) -> PaneId {
        if self.focused_pane.is_overlay() {
            self.overlay_return.unwrap_or(PaneId::ProjectList)
        } else {
            self.focused_pane
        }
    }

    /// Focus a non-overlay pane. Marks it visited and clears
    /// `overlay_return`.
    pub(crate) fn set(&mut self, pane: PaneId) {
        self.focused_pane = pane;
        if !pane.is_overlay() {
            self.visited.insert(pane);
            self.overlay_return = None;
        }
    }

    /// Open an overlay pane, remembering the current base focus to
    /// restore on close.
    pub(crate) fn open_overlay(&mut self, pane: PaneId) {
        if !pane.is_overlay() {
            self.set(pane);
            return;
        }
        self.overlay_return = Some(self.base());
        self.focused_pane = pane;
    }

    /// Close any active overlay and restore the remembered focus.
    pub(crate) fn close_overlay(&mut self) {
        self.focused_pane = self.overlay_return.unwrap_or(PaneId::ProjectList);
        self.overlay_return = None;
    }

    /// Retarget the remembered return focus. Used by post-selection
    /// fixups when the recorded pane disappears from the tabbable set.
    pub(crate) const fn retarget_overlay_return(&mut self, pane: PaneId) {
        self.overlay_return = Some(pane);
    }

    /// Whether the currently-recorded `overlay_return` is one of `panes`.
    pub(crate) fn overlay_return_is_in(&self, panes: &[PaneId]) -> bool {
        self.overlay_return.is_some_and(|p| panes.contains(&p))
    }

    pub(crate) fn unvisit(&mut self, pane: PaneId) { self.visited.remove(&pane); }

    pub(crate) fn remembers_visited(&self, pane: PaneId) -> bool { self.visited.contains(&pane) }

    /// Resolve a pane to its [`PaneFocusState`] — `Active` if currently
    /// focused, `Remembered` if previously visited, otherwise `Inactive`.
    pub(crate) fn pane_state(&self, pane: PaneId) -> PaneFocusState {
        if self.is(pane) {
            PaneFocusState::Active
        } else if self.remembers_visited(pane) {
            PaneFocusState::Remembered
        } else {
            PaneFocusState::Inactive
        }
    }
}
