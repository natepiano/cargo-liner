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
//! [`RenderedSlot::shortcut_state`](crate::keymap::RenderedSlot). The
//! framework's [`BarPalette::default`] is theme-neutral
//! ([`Style::default`](ratatui::style::Style::default) for every
//! field) so the binary supplies its own palette to get any color.

use ratatui::text::Span;

use super::BarPalette;
use super::constants::SEPARATOR;
use crate::KeySequence;
use crate::ShortcutState;
use crate::keymap::RenderedSlot;

/// Render `slot` into the running `spans` vector. Pushes the
/// separator first when `spans` already contains slots for this
/// region.
pub(super) fn push_slot(spans: &mut Vec<Span<'static>>, slot: &RenderedSlot, palette: &BarPalette) {
    if let Some(secondary_key) = &slot.secondary_key {
        push_paired(spans, &slot.key, secondary_key, slot.label, palette);
        return;
    }
    push_hint(
        spans,
        &slot.key.display_short(),
        slot.label,
        slot.shortcut_state,
        palette,
    );
}

/// Style one hint and separate it from earlier hints.
pub(super) fn push_hint(
    spans: &mut Vec<Span<'static>>,
    key: &str,
    label: &'static str,
    shortcut_state: ShortcutState,
    palette: &BarPalette,
) {
    if !spans.is_empty() {
        spans.push(Span::styled(SEPARATOR, palette.separator_style));
    }
    let (key_style, label_style) = match shortcut_state {
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
    primary: &KeySequence,
    secondary: &KeySequence,
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
mod tests {
    use crossterm::event::KeyCode;

    use super::BarPalette;
    use super::push_paired;
    use super::push_slot;
    use crate::BarRegion;
    use crate::KeyBind;
    use crate::KeySequence;
    use crate::ShortcutState;
    use crate::Visibility;
    use crate::keymap::RenderedSlot;

    fn rendered(label: &'static str, key: KeyBind) -> RenderedSlot {
        RenderedSlot {
            region: BarRegion::PaneAction,
            label,
            key: key.into(),
            shortcut_state: ShortcutState::Enabled,
            visibility: Visibility::Visible,
            secondary_key: None,
        }
    }

    #[test]
    fn push_slot_emits_two_spans_per_call() {
        let bar_palette = BarPalette::default();
        let mut spans = Vec::new();
        push_slot(
            &mut spans,
            &rendered("save", KeyBind::from('s')),
            &bar_palette,
        );
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), " s");
        assert_eq!(spans[1].content.as_ref(), " save");
    }

    #[test]
    fn push_slot_inserts_separator_between_slots() {
        let bar_palette = BarPalette::default();
        let mut spans = Vec::new();
        push_slot(
            &mut spans,
            &rendered("save", KeyBind::from('s')),
            &bar_palette,
        );
        push_slot(
            &mut spans,
            &rendered("cancel", KeyBind::from(KeyCode::Esc)),
            &bar_palette,
        );
        // `spans` contains two spans for the first slot, one `super::SEPARATOR`, two for the
        // second.
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[2].content.as_ref(), "  ");
    }

    #[test]
    fn push_paired_emits_two_spans_with_slash() {
        let bar_palette = BarPalette::default();
        let mut spans = Vec::new();
        push_paired(
            &mut spans,
            &KeyBind::from(KeyCode::Up).into(),
            &KeyBind::from(KeyCode::Down).into(),
            "nav",
            &bar_palette,
        );
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), " ↑/↓");
        assert_eq!(spans[1].content.as_ref(), " nav");
    }

    #[test]
    fn push_slot_routes_secondary_key_through_paired_renderer() {
        let bar_palette = BarPalette::default();
        let mut spans = Vec::new();
        let mut rendered_slot = rendered("expand", KeyBind::from(KeyCode::Left));
        rendered_slot.secondary_key = Some(KeySequence::from(KeyBind::from(KeyCode::Right)));

        push_slot(&mut spans, &rendered_slot, &bar_palette);

        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), " ←/→");
        assert_eq!(spans[1].content.as_ref(), " expand");
    }
}
