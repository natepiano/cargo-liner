use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;

use crate::tui::constants::TITLE_COLOR;
use crate::tui::render;

const TITLE_STYLE: Style = Style::new().fg(TITLE_COLOR).add_modifier(Modifier::BOLD);

/// Shared chrome for popup overlays.
///
/// Handles centering, background clearing, and a bordered frame with
/// an optional left-aligned yellow-bold title.
pub struct PopupFrame {
    pub title:        Option<String>,
    pub border_color: Color,
    pub width:        u16,
    pub height:       u16,
}

#[derive(Clone, Copy)]
pub struct PopupAreas {
    pub outer: Rect,
    pub inner: Rect,
}

impl PopupFrame {
    /// Render the popup chrome and return the usable inner `Rect`.
    pub fn render(self, frame: &mut Frame) -> Rect { self.render_with_areas(frame).inner }

    /// Render the popup chrome and return both outer and inner `Rect`s.
    pub fn render_with_areas(self, frame: &mut Frame) -> PopupAreas {
        let area = render::centered_rect(self.width, self.height, frame.area());

        frame.render_widget(Clear, area);

        let border_style = Style::new().fg(self.border_color);
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);

        if let Some(title) = self.title {
            block = block.title(Span::styled(title, TITLE_STYLE));
        }

        let inner = block.inner(area);
        frame.render_widget(block, area);
        PopupAreas { outer: area, inner }
    }
}
