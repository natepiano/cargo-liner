//! Per-role color accessors that read from the active theme.
//!
//! These replace the pre-theme `pub const FOO_COLOR: Color = ...`
//! items in `tui_pane/src/constants.rs`. Each call clones one `Arc`
//! and reads the matching field; the cost is sub-µs and unmeasurable
//! against ratatui's per-cell work.
//!
//! Modifiers carried on the underlying `StyleSpec` are NOT applied
//! here. Call sites that need the full themed `Style` should reach
//! `tui_pane::theme().<group>.<field>.style()` directly.

use ratatui::style::Color;
use ratatui::style::Style;

use super::StyleSpec;
use super::theme;

/// Spinners, shortcut hints, finder cursor.
#[must_use]
pub fn accent_color() -> Color { theme().semantic.accent.color }

/// Border color for the currently focused pane.
#[must_use]
pub fn active_border_color() -> Color { theme().pane_chrome.active_border.color }

/// Background highlight for the currently focused pane row.
#[must_use]
pub fn active_focus_color() -> Color { theme().focus.active.color }

/// Background highlight for the row currently under the mouse.
#[must_use]
pub fn hover_focus_color() -> Color { theme().focus.hover.color }

/// Error text, failure icons, broken worktree backgrounds.
#[must_use]
pub fn error_color() -> Color { theme().semantic.error.color }

/// Inline errors shown on selected settings rows.
#[must_use]
pub fn inline_error_color() -> Color { theme().semantic.inline_error.color }

/// Unfocused pane borders.
#[must_use]
pub fn inactive_border_color() -> Color { theme().pane_chrome.inactive_border.color }

/// Unfocused pane titles for populated panes.
#[must_use]
pub fn inactive_title_color() -> Color { theme().pane_chrome.inactive_title.color }

/// Detail panel field labels, stat labels, settings labels, toast
/// countdowns, finder hints, chevron arrows.
#[must_use]
pub fn label_color() -> Color { theme().semantic.label.color }

/// Background highlight showing the previously focused row when a
/// pane loses focus.
#[must_use]
pub fn remembered_focus_color() -> Color { theme().focus.remembered.color }

/// Dimmed secondary text.
#[must_use]
pub fn secondary_text_color() -> Color { theme().text.secondary.color }

/// Bottom status bar background.
#[must_use]
pub fn status_bar_color() -> Color { theme().status.bar.color }

/// Clean/passed/synced states.
#[must_use]
pub fn success_color() -> Color { theme().semantic.success.color }

/// Active pane titles, section headers, group header labels, stat
/// numbers, confirm dialog prompts, popup titles, summary row.
#[must_use]
pub fn title_color() -> Color { theme().pane_chrome.active_title.color }

/// Cautionary text. Service unavailability placeholders, pending
/// data that depends on an unreachable service. Distinct from
/// `error_color` — warning means degraded-but-recoverable.
#[must_use]
pub fn warning_color() -> Color { theme().semantic.warning.color }

/// Background tint on fuzzy-matched characters in finder results.
#[must_use]
pub fn finder_match_bg() -> Color { theme().finder.match_bg.color }

/// Universal "regular foreground" text — the previous code used inline
/// `Color::White` for this; Phase 1 routes those sites here.
#[must_use]
pub fn text_default() -> Color { theme().text.default.color }

/// Client-defined role spec by app-owned key.
#[must_use]
pub fn role_spec(role: &str, fallback: StyleSpec) -> StyleSpec {
    theme().roles.get(role).copied().unwrap_or(fallback)
}

/// Client-defined role style by app-owned key.
#[must_use]
pub fn role_style(role: &str, fallback: StyleSpec) -> Style { role_spec(role, fallback).style() }

/// Client-defined role foreground color by app-owned key.
#[must_use]
pub fn role_color(role: &str, fallback: StyleSpec) -> Color { role_spec(role, fallback).color }
