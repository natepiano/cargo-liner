use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use tui_pane::ACTIVE_BORDER_COLOR;
use tui_pane::INACTIVE_BORDER_COLOR;
use tui_pane::INACTIVE_TITLE_COLOR;
use tui_pane::TITLE_COLOR;

#[derive(Clone, Copy)]
pub struct PaneChrome {
    pub active_border:   Style,
    pub inactive_border: Style,
    pub active_title:    Style,
    pub inactive_title:  Style,
}

impl PaneChrome {
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

    pub const fn title_style(self, focused: bool) -> Style {
        if focused {
            self.active_title
        } else {
            self.inactive_title
        }
    }

    pub const fn with_inactive_border(self, inactive_border: Style) -> Self {
        Self {
            inactive_border,
            ..self
        }
    }
}

pub fn default_pane_chrome() -> PaneChrome {
    let title_style = Style::default().add_modifier(Modifier::BOLD);
    PaneChrome {
        active_border:   Style::default().fg(ACTIVE_BORDER_COLOR),
        inactive_border: Style::default(),
        active_title:    title_style.fg(TITLE_COLOR),
        inactive_title:  title_style.fg(INACTIVE_TITLE_COLOR),
    }
}

pub fn empty_pane_block(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.into())
        .title_style(Style::default().fg(INACTIVE_BORDER_COLOR))
        .border_style(Style::default().fg(INACTIVE_BORDER_COLOR))
}
