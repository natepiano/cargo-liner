use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;

use crate::active_border_color;
use crate::focused_pane_tint_enabled;
use crate::inactive_border_color;
use crate::inactive_title_color;
use crate::theme;
use crate::title_color;

/// Pane chrome styling bundle: border and title styles for the
/// focused / unfocused render paths of a bordered pane.
#[derive(Clone, Copy)]
pub struct PaneChrome {
    /// Border style when the pane is focused.
    pub active_border:   Style,
    /// Border style when the pane is unfocused.
    pub inactive_border: Style,
    /// Title style when the pane is focused.
    pub active_title:    Style,
    /// Title style when the pane is unfocused.
    pub inactive_title:  Style,
}

impl PaneChrome {
    /// Build a bordered ratatui [`Block`] using this chrome.
    #[must_use]
    pub fn block(self, title: String, focused: bool) -> Block<'static> {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(self.title_style(focused))
            .border_style(if focused {
                self.active_border
            } else {
                self.inactive_border
            });
        if focused && focused_pane_tint_enabled() {
            block.style(Style::default().bg(focused_pane_tint()))
        } else {
            block
        }
    }

    /// The title style this chrome applies given focus.
    #[must_use]
    pub const fn title_style(self, focused: bool) -> Style {
        if focused {
            self.active_title
        } else {
            self.inactive_title
        }
    }

    /// Replace the inactive border style.
    #[must_use]
    pub const fn with_inactive_border(self, inactive_border: Style) -> Self {
        Self {
            inactive_border,
            ..self
        }
    }
}

/// Default pane chrome.
///
/// Focused: accent border + bold accent title. Unfocused: the
/// theme's `pane_chrome.inactive_border` colour + dim title. Driving
/// the unfocused border from the theme (rather than
/// `Style::default()`) so every pane in cargo-port draws the same
/// shade, regardless of how a given terminal profile renders its
/// "default foreground" colour.
#[must_use]
pub fn default_pane_chrome() -> PaneChrome {
    PaneChrome {
        active_border:   Style::default().fg(active_border_color()),
        inactive_border: Style::default().fg(inactive_border_color()),
        active_title:    Style::default()
            .fg(title_color())
            .add_modifier(Modifier::BOLD),
        inactive_title:  Style::default().fg(inactive_title_color()),
    }
}

/// Subtle background tint for the focused pane.
///
/// Derived from `text.bg_focus` so it tracks the active appearance:
/// dark themes get a small lift away from black; light themes get a
/// small drop away from white. Terminals have no alpha channel, so
/// this is a solid RGB nudge — see `docs/themes.md`.
fn focused_pane_tint() -> Color {
    match theme().text.bg_focus.color {
        Color::Black => Color::Rgb(28, 28, 32),
        Color::White => Color::Rgb(240, 240, 245),
        Color::Rgb(r, g, b) => {
            let avg = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
            if avg < 128 {
                Color::Rgb(
                    r.saturating_add(18),
                    g.saturating_add(18),
                    b.saturating_add(22),
                )
            } else {
                Color::Rgb(
                    r.saturating_sub(14),
                    g.saturating_sub(14),
                    b.saturating_sub(10),
                )
            }
        },
        other => other,
    }
}

/// Bordered empty-state block.
///
/// Used for panes that have no content to render (no data yet, no
/// git repo, etc.). Matches the unfocused chrome of
/// [`default_pane_chrome`] so empty and populated panes draw the
/// same border shade.
#[must_use]
pub fn empty_pane_block(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.into())
        .title_style(Style::default().fg(inactive_border_color()))
        .border_style(Style::default().fg(inactive_border_color()))
}
