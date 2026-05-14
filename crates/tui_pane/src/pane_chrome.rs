use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;

use crate::ACTIVE_BORDER_COLOR;
use crate::INACTIVE_BORDER_COLOR;
use crate::INACTIVE_TITLE_COLOR;
use crate::TITLE_COLOR;

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
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(self.title_style(focused))
            .border_style(if focused {
                self.active_border
            } else {
                self.inactive_border
            })
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

/// Default pane chrome: yellow accent border + bold title when focused,
/// dim border + dim title when unfocused.
#[must_use]
pub fn default_pane_chrome() -> PaneChrome {
    let title_style = Style::default().add_modifier(Modifier::BOLD);
    PaneChrome {
        active_border:   Style::default().fg(ACTIVE_BORDER_COLOR),
        inactive_border: Style::default(),
        active_title:    title_style.fg(TITLE_COLOR),
        inactive_title:  title_style.fg(INACTIVE_TITLE_COLOR),
    }
}

/// Bordered empty-state block — used for panes that have no content
/// to render (no data yet, no git repo, etc.).
#[must_use]
pub fn empty_pane_block(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.into())
        .title_style(Style::default().fg(INACTIVE_BORDER_COLOR))
        .border_style(Style::default().fg(INACTIVE_BORDER_COLOR))
}
