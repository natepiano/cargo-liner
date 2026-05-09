//! `StatusBar`: the framework bar renderer's output value.
//!
//! Three flat span sequences — one per [`BarRegion`](super::BarRegion).
//! The binary positions them however the application's status line
//! wants (typically left / center / right); the framework owns
//! intra-region spacing and per-slot styling.

use ratatui::text::Span;

/// Joined output of [`bar::render`](super::render).
///
/// Each field is a flat [`Span`] sequence ready to be wrapped in a
/// [`ratatui::text::Line`] and drawn. `nav` carries the navigation /
/// pane-cycle row, `pane_action` carries the focused pane's local
/// actions, and `global` carries the framework + app globals strip.
///
/// Empty `Vec`s indicate region suppression — the renderer applies
/// the [`Mode`](crate::Mode) suppression rules itself; the binary
/// does not need to re-check them.
#[derive(Clone, Debug, Default)]
pub struct StatusBar {
    /// Navigation + pane-cycle slots.
    pub nav:         Vec<Span<'static>>,
    /// Focused pane's `PaneAction` slots.
    pub pane_action: Vec<Span<'static>>,
    /// Framework + app globals slots.
    pub global:      Vec<Span<'static>>,
}

impl StatusBar {
    /// Empty bar — every region suppressed. Used by the renderer when
    /// the focused pane has no registered mode (the registry returns
    /// `None`).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            nav:         Vec::new(),
            pane_action: Vec::new(),
            global:      Vec::new(),
        }
    }

    /// `true` when every region is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.nav.is_empty() && self.pane_action.is_empty() && self.global.is_empty()
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::text::Span;

    use super::StatusBar;

    #[test]
    fn empty_round_trips() {
        let bar = StatusBar::empty();
        assert!(bar.is_empty());
        assert!(bar.nav.is_empty());
        assert!(bar.pane_action.is_empty());
        assert!(bar.global.is_empty());
    }

    #[test]
    fn non_empty_when_any_region_populated() {
        let mut bar = StatusBar::empty();
        bar.nav.push(Span::raw("x"));
        assert!(!bar.is_empty());
    }
}
