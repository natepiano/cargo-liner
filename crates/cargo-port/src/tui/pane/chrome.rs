use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::Pane;
use crate::tui::constants::ACTIVE_BORDER_COLOR;
use crate::tui::constants::INACTIVE_BORDER_COLOR;
use crate::tui::constants::INACTIVE_TITLE_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::TITLE_COLOR;

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

pub fn render_overflow_affordance(frame: &mut Frame, area: Rect, pane: &Pane) {
    let Some(label) = pane.overflow_affordance() else {
        return;
    };
    if area.width <= 2 || area.height == 0 {
        return;
    }

    let inner_width = area.width.saturating_sub(2);
    let label_width = u16::try_from(label.width()).unwrap_or(u16::MAX);
    if label_width == 0 || label_width > inner_width {
        return;
    }

    let x = area
        .x
        .saturating_add(1)
        .saturating_add(inner_width.saturating_sub(label_width) / 2);
    let affordance_area = Rect::new(x, area.bottom().saturating_sub(1), label_width, 1);
    let style = Style::default().fg(LABEL_COLOR);
    frame.render_widget(Paragraph::new(label).style(style), affordance_area);
}
