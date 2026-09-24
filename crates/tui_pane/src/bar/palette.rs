//! `BarPalette`: theme-neutral styling pass for the framework's bar
//! renderer.
//!
//! The framework owns the per-slot styling pass — it knows
//! `RenderedSlot::shortcut_state` at the moment
//! a span is emitted, and the binary does not — but it ships no
//! colors of its own. The default constructor is theme-neutral: every
//! field is [`Style::default()`]. [`BarPalette::themed`] draws every
//! field from the installed theme; a binary that wants other choices
//! supplies a populated [`BarPalette`] of its own.

use ratatui::style::Modifier;
use ratatui::style::Style;

use crate::accent_color;
use crate::secondary_text_color;
use crate::status_bar_color;
use crate::text_default;
use crate::title_color;

/// Style choices applied to bar spans by
/// [`render_status_bar`](super::render()).
///
/// Five fields select between enabled / disabled `key` and `label`
/// styling; the fifth styles the inter-slot separator.
/// [`render_status_bar`](super::render()) reads
/// `RenderedSlot::shortcut_state` and chooses
/// `enabled_*` vs `disabled_*` per slot at emit time.
#[derive(Clone, Copy, Debug, Default)]
pub struct BarPalette {
    /// Base style for the full status-line background fill.
    pub status_line_style:     Style,
    /// Style applied to status activity text such as `"scanning"`.
    pub status_activity_style: Style,
    /// Style applied to status labels such as `"Uptime:"`.
    pub status_label_style:    Style,
    /// Style applied to status values such as the uptime duration.
    pub status_value_style:    Style,
    /// Style applied to the key span (e.g. `" Enter"`) when the slot's
    /// shortcut state is [`ShortcutState::Enabled`](super::ShortcutState).
    pub enabled_key_style:     Style,
    /// Style applied to the label span (e.g. `" activate"`) when the
    /// slot's shortcut state is [`ShortcutState::Enabled`](super::ShortcutState).
    pub enabled_label_style:   Style,
    /// Style applied to the key span when the slot's shortcut state is
    /// [`ShortcutState::Disabled`](super::ShortcutState).
    pub disabled_key_style:    Style,
    /// Style applied to the label span when the slot's shortcut state is
    /// [`ShortcutState::Disabled`](super::ShortcutState).
    pub disabled_label_style:  Style,
    /// Style applied to the inter-slot separator (`"  "`).
    pub separator_style:       Style,
}

impl BarPalette {
    /// Status-line styling drawn from the installed theme: bold accent
    /// keys, default-text labels and values, greyed disabled slots, all
    /// on the theme's status-bar background. Read on each call, so a
    /// theme swapped under a running app shows on the next frame.
    #[must_use]
    pub fn themed() -> Self {
        let enabled_key_style = Style::default()
            .fg(accent_color())
            .add_modifier(Modifier::BOLD);
        let disabled_key_style = Style::default()
            .fg(secondary_text_color())
            .add_modifier(Modifier::BOLD);
        Self {
            status_line_style: Style::default().bg(status_bar_color()).fg(text_default()),
            status_activity_style: enabled_key_style,
            status_label_style: Style::default()
                .fg(title_color())
                .add_modifier(Modifier::BOLD),
            status_value_style: Style::default().fg(text_default()),
            enabled_key_style,
            enabled_label_style: Style::default().fg(text_default()),
            disabled_key_style,
            disabled_label_style: Style::default().fg(secondary_text_color()),
            separator_style: Style::default(),
        }
    }
}
