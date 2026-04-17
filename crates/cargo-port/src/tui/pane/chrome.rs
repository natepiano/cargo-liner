use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;

use crate::tui::constants::ACTIVE_BORDER_COLOR;
use crate::tui::constants::INACTIVE_BORDER_COLOR;
use crate::tui::constants::INACTIVE_TITLE_COLOR;
use crate::tui::constants::TITLE_COLOR;

#[derive(Clone, Copy)]
pub(in super::super) struct PaneChrome {
    pub(in super::super) active_border:   Style,
    pub(in super::super) inactive_border: Style,
    pub(in super::super) active_title:    Style,
    pub(in super::super) inactive_title:  Style,
}

impl PaneChrome {
    pub(in super::super) fn block(self, title: String, focused: bool) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(if focused {
                self.active_title
            } else {
                self.inactive_title
            })
            .border_style(if focused {
                self.active_border
            } else {
                self.inactive_border
            })
    }

    pub(in super::super) const fn with_inactive_border(self, inactive_border: Style) -> Self {
        Self {
            inactive_border,
            ..self
        }
    }
}

pub(in super::super) fn default_pane_chrome() -> PaneChrome {
    let title_style = Style::default().add_modifier(Modifier::BOLD);
    PaneChrome {
        active_border:   Style::default().fg(ACTIVE_BORDER_COLOR),
        inactive_border: Style::default(),
        active_title:    title_style.fg(TITLE_COLOR),
        inactive_title:  title_style.fg(INACTIVE_TITLE_COLOR),
    }
}

pub(in super::super) fn empty_pane_block(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.into())
        .title_style(Style::default().fg(INACTIVE_BORDER_COLOR))
        .border_style(Style::default().fg(INACTIVE_BORDER_COLOR))
}
