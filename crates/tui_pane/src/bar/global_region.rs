//! `Global` region of the bar.
//!
//! Walks the framework globals first, then the app globals. The
//! framework's pane-cycle pair ([`GlobalAction::NextPane`](crate::GlobalAction::NextPane) /
//! [`GlobalAction::PrevPane`](crate::GlobalAction::PrevPane)) is owned by
//! `nav_region` and dropped here.
//!
//! Suppressed when the focused pane's mode is
//! [`Some(Mode::TextInput(_))`](crate::Mode::TextInput) — text-entry
//! contexts let the embedded handler own every printable key, and the
//! global strip would advertise unreachable bindings. `None` (no
//! registered pane) likewise returns empty: nothing to render
//! against.

use ratatui::text::Span;

use super::BarPalette;
use super::support;
use crate::Action;
use crate::AppContext;
use crate::GlobalAction;
use crate::Keymap;
use crate::Mode;

pub(super) fn render<Ctx: AppContext + 'static>(
    mode: Option<&Mode<Ctx>>,
    keymap: &Keymap<Ctx>,
    palette: &BarPalette,
) -> Vec<Span<'static>> {
    let render_globals = match mode {
        Some(Mode::Navigable | Mode::Static) => true,
        Some(Mode::TextInput(_)) | None => false,
    };
    if !render_globals {
        return Vec::new();
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    for slot in keymap.render_framework_globals_slots() {
        // NextPane / PrevPane render in the nav region's pane-cycle
        // row — drop them here so they don't render twice.
        if matches!(
            framework_action_for_label(slot.label),
            Some(GlobalAction::NextPane | GlobalAction::PrevPane)
        ) {
            continue;
        }
        support::push_slot(&mut spans, &slot, palette);
    }
    for slot in keymap.render_app_globals_slots() {
        support::push_slot(&mut spans, &slot, palette);
    }
    spans
}

/// Reverse-lookup helper: the rendered slot carries only the
/// `bar_label` static string, but we need to recognize the cycle
/// variants to drop them. This costs one match per global slot;
/// accepted trade for not introducing a typed variant tag on
/// `RenderedSlot`.
fn framework_action_for_label(label: &'static str) -> Option<GlobalAction> {
    GlobalAction::ALL
        .iter()
        .copied()
        .find(|a| crate::Action::bar_label(*a) == label)
}
