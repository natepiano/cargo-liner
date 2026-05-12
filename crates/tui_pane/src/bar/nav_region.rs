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

use super::BarPalette;
use super::support;
use crate::AppContext;
use crate::BarRegion;
use crate::GlobalAction;
use crate::Keymap;
use crate::Mode;
use crate::ShortcutState;
use crate::Visibility;
use crate::keymap::RenderedSlot;

pub(super) fn render<Ctx: AppContext + 'static>(
    mode: Option<&Mode<Ctx>>,
    keymap: &Keymap<Ctx>,
    pane_slots: &[RenderedSlot],
    palette: &BarPalette,
) -> Vec<Span<'static>> {
    if !matches!(mode, Some(Mode::Navigable)) {
        return Vec::new();
    }

    let mut spans: Vec<Span<'static>> = Vec::new();

    // Navigation scope: the status bar advertises the compact row the
    // legacy binary used ("↑/↓ nav"), not every scroll/navigation key.
    let navigation_slots = keymap.render_navigation_slots();
    if let (Some(up), Some(down)) = (
        slot_by_label(&navigation_slots, "up"),
        slot_by_label(&navigation_slots, "down"),
    ) {
        support::push_paired(&mut spans, up.key, down.key, "nav", palette);
    }

    // Pane-emitted nav slots (rare — most panes leave nav to the
    // navigation scope, but `bar_slots` is allowed to add extras).
    for slot in pane_slots.iter().filter(|s| s.region == BarRegion::Nav) {
        support::push_slot(&mut spans, slot, palette);
    }

    // Pane-cycle row from the framework globals. The pre-refactor bar
    // advertised the forward key only ("Tab pane") even though
    // Shift+Tab also works.
    if let Some(&next) = keymap.framework_globals().key_for(GlobalAction::NextPane) {
        let slot = RenderedSlot {
            region:        BarRegion::Nav,
            label:         "pane",
            key:           next,
            state:         ShortcutState::Enabled,
            visibility:    Visibility::Visible,
            secondary_key: None,
        };
        support::push_slot(&mut spans, &slot, palette);
    }

    spans
}

fn slot_by_label<'a>(slots: &'a [RenderedSlot], label: &str) -> Option<&'a RenderedSlot> {
    slots.iter().find(|slot| slot.label == label)
}
