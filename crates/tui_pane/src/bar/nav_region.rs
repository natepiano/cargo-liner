//! `Nav` region of the bar.
//!
//! Walks two sources in this order, then concatenates:
//!
//! 1. The framework's pane-cycle row — paired
//!    [`GlobalAction::NextPane`](crate::GlobalAction::NextPane) +
//!    [`GlobalAction::PrevPane`](crate::GlobalAction::PrevPane), labelled `pane`.
//! 2. Slots tagged [`BarRegion::Nav`](crate::BarRegion::Nav) drawn from the focused pane's
//!    `pane_slots` and from the navigation scope's resolved slots
//!    ([`Keymap::render_navigation_slots`](crate::Keymap::render_navigation_slots)).
//!
//! Suppression: empty `Vec` whenever the focused pane's mode is
//! anything other than [`Some(Mode::Navigable)`](crate::Mode::Navigable) — `Static`,
//! `TextInput(_)`, and `None` (no registered pane) all return empty.

use ratatui::text::Span;

use super::support;
use crate::AppContext;
use crate::BarRegion;
use crate::GlobalAction;
use crate::Keymap;
use crate::Mode;
use crate::keymap::RenderedSlot;

pub(super) fn render<Ctx: AppContext + 'static>(
    mode: Option<&Mode<Ctx>>,
    keymap: &Keymap<Ctx>,
    pane_slots: &[RenderedSlot],
) -> Vec<Span<'static>> {
    if !matches!(mode, Some(Mode::Navigable)) {
        return Vec::new();
    }

    let mut spans: Vec<Span<'static>> = Vec::new();

    // Pane-cycle row from the framework globals — emit only when both
    // halves are bound.
    let framework = keymap.framework_globals();
    if let (Some(&next), Some(&prev)) = (
        framework.key_for(GlobalAction::NextPane),
        framework.key_for(GlobalAction::PrevPane),
    ) {
        support::push_paired(&mut spans, next, prev, "pane");
    }

    // Navigation scope's bar slots (Up/Down/Left/Right/Home/End). The
    // first two are the standard "↑/↓ nav" indicator; the bar emits
    // every entry the binary's Navigation impl produced, in the order
    // returned by the keymap.
    for slot in keymap.render_navigation_slots() {
        support::push_slot(&mut spans, &slot);
    }

    // Pane-emitted nav slots (rare — most panes leave nav to the
    // navigation scope, but `bar_slots` is allowed to add extras).
    for slot in pane_slots.iter().filter(|s| s.region == BarRegion::Nav) {
        support::push_slot(&mut spans, slot);
    }

    spans
}
