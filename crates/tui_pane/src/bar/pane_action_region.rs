//! `PaneAction` region of the bar.
//!
//! Filters the focused pane's pre-resolved slots for
//! [`BarRegion::PaneAction`](crate::BarRegion::PaneAction) and emits
//! one slot per entry. Renders for [`Some(Mode::Navigable)`](crate::Mode::Navigable)
//! and [`Some(Mode::Static)`](crate::Mode::Static); suppressed for
//! [`Some(Mode::TextInput(_))`](crate::Mode::TextInput) and `None`.

use ratatui::text::Span;

use super::support;
use crate::AppContext;
use crate::BarRegion;
use crate::Mode;
use crate::keymap::RenderedSlot;

pub(super) fn render<Ctx: AppContext>(
    mode: Option<&Mode<Ctx>>,
    pane_slots: &[RenderedSlot],
) -> Vec<Span<'static>> {
    let render_pane_actions = match mode {
        Some(Mode::Navigable | Mode::Static) => true,
        Some(Mode::TextInput(_)) | None => false,
    };
    if !render_pane_actions {
        return Vec::new();
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    for slot in pane_slots
        .iter()
        .filter(|s| s.region == BarRegion::PaneAction)
    {
        support::push_slot(&mut spans, slot);
    }
    spans
}
