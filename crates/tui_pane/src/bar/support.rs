//! Shared row builders for the bar region modules.
//!
//! Each region module emits `Vec<Span<'static>>`. `bar::render` walks
//! [`BarRegion::ALL`](super::BarRegion::ALL) and concatenates the
//! resulting spans into the matching field on
//! [`StatusBar`](super::StatusBar). The free fns here own the slot
//! formatting so every region renders slots identically: `" {key} {label}"`
//! per slot, with `"  "` between consecutive slots in the same region.
//!
//! Per-span styling: every emitted span is styled via the supplied
//! [`BarPalette`]. Enabled vs disabled styles are selected from
//! [`RenderedSlot::state`](crate::keymap::RenderedSlot). The
//! framework's [`BarPalette::default`] is theme-neutral
//! ([`Style::default`](ratatui::style::Style::default) for every
//! field) so the binary supplies its own palette to get any color.

use ratatui::text::Span;

use super::BarPalette;
use crate::KeyBind;
use crate::ShortcutState;
use crate::keymap::RenderedSlot;

/// Inter-slot separator inside one region.
const SEPARATOR: &str = "  ";

/// Render `slot` into the running `spans` vector. Pushes the
/// separator first when `spans` already contains slots for this
/// region.
pub(super) fn push_slot(spans: &mut Vec<Span<'static>>, slot: &RenderedSlot, palette: &BarPalette) {
    if !spans.is_empty() {
        spans.push(Span::styled(SEPARATOR, palette.separator_style));
    }
    let key = slot.key.display_short();
    let label = slot.label;
    let (key_style, label_style) = match slot.state {
        ShortcutState::Enabled => (palette.enabled_key_style, palette.enabled_label_style),
        ShortcutState::Disabled => (palette.disabled_key_style, palette.disabled_label_style),
    };
    spans.push(Span::styled(format!(" {key}"), key_style));
    spans.push(Span::styled(format!(" {label}"), label_style));
}

/// Render a paired-key row. Used for the framework's pane-cycle row
/// (`NextPane` + `PrevPane` → `"Tab/Shift+Tab pane"`) and for the nav
/// row (Up + Down → `"↑/↓ nav"`). Paired rows are always rendered as
/// `Enabled` — the palette's enabled styles apply.
pub(super) fn push_paired(
    spans: &mut Vec<Span<'static>>,
    primary: KeyBind,
    secondary: KeyBind,
    label: &'static str,
    palette: &BarPalette,
) {
    if !spans.is_empty() {
        spans.push(Span::styled(SEPARATOR, palette.separator_style));
    }
    let primary_disp = primary.display_short();
    let secondary_disp = secondary.display_short();
    spans.push(Span::styled(
        format!(" {primary_disp}/{secondary_disp}"),
        palette.enabled_key_style,
    ));
    spans.push(Span::styled(
        format!(" {label}"),
        palette.enabled_label_style,
    ));
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crossterm::event::KeyCode;

    use super::BarPalette;
    use super::push_paired;
    use super::push_slot;
    use crate::BarRegion;
    use crate::KeyBind;
    use crate::ShortcutState;
    use crate::Visibility;
    use crate::keymap::RenderedSlot;

    fn rendered(label: &'static str, key: KeyBind) -> RenderedSlot {
        RenderedSlot {
            region: BarRegion::PaneAction,
            label,
            key,
            state: ShortcutState::Enabled,
            visibility: Visibility::Visible,
        }
    }

    #[test]
    fn push_slot_emits_two_spans_per_call() {
        let palette = BarPalette::default();
        let mut spans = Vec::new();
        push_slot(&mut spans, &rendered("save", KeyBind::from('s')), &palette);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), " s");
        assert_eq!(spans[1].content.as_ref(), " save");
    }

    #[test]
    fn push_slot_inserts_separator_between_slots() {
        let palette = BarPalette::default();
        let mut spans = Vec::new();
        push_slot(&mut spans, &rendered("save", KeyBind::from('s')), &palette);
        push_slot(
            &mut spans,
            &rendered("cancel", KeyBind::from(KeyCode::Esc)),
            &palette,
        );
        // Two spans for the first slot, one separator, two for the second.
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content.as_ref(), "  ");
    }

    #[test]
    fn push_paired_emits_two_spans_with_slash() {
        let palette = BarPalette::default();
        let mut spans = Vec::new();
        push_paired(
            &mut spans,
            KeyBind::from(KeyCode::Up),
            KeyBind::from(KeyCode::Down),
            "nav",
            &palette,
        );
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), " ↑/↓");
        assert_eq!(spans[1].content.as_ref(), " nav");
    }
}
