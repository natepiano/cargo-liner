//! `BarPalette`: theme-neutral styling pass for the framework's bar
//! renderer.
//!
//! The framework owns the per-slot styling pass — it knows
//! [`RenderedSlot::state`](crate::keymap::RenderedSlot) at the moment
//! a span is emitted, and the binary does not — but it ships no
//! colors of its own. The default constructor is theme-neutral: every
//! field is [`Style::default()`]. Binaries supply a populated
//! [`BarPalette`] to get any visible color in the bar.

use ratatui::style::Style;

/// Style choices applied to bar spans by
/// [`render_status_bar`](super::render).
///
/// Five fields select between enabled / disabled `key` and `label`
/// styling; the fifth styles the inter-slot separator.
/// [`render_status_bar`](super::render) reads
/// [`RenderedSlot::state`](crate::keymap::RenderedSlot) and chooses
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
    /// state is [`ShortcutState::Enabled`](super::ShortcutState).
    pub enabled_key_style:     Style,
    /// Style applied to the label span (e.g. `" activate"`) when the
    /// slot's state is [`ShortcutState::Enabled`](super::ShortcutState).
    pub enabled_label_style:   Style,
    /// Style applied to the key span when the slot's state is
    /// [`ShortcutState::Disabled`](super::ShortcutState).
    pub disabled_key_style:    Style,
    /// Style applied to the label span when the slot's state is
    /// [`ShortcutState::Disabled`](super::ShortcutState).
    pub disabled_label_style:  Style,
    /// Style applied to the inter-slot separator (`"  "`).
    pub separator_style:       Style,
}
