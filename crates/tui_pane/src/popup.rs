use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;

use crate::TITLE_COLOR;

const TITLE_STYLE: Style = Style::new().fg(TITLE_COLOR).add_modifier(Modifier::BOLD);

/// Shared chrome for popup overlays.
///
/// Handles centering, background clearing, and a bordered frame with
/// an optional left-aligned bold title.
pub struct PopupFrame {
    /// Optional title rendered inside the top border.
    pub title:        Option<String>,
    /// Border color for the popup chrome.
    pub border_color: Color,
    /// Popup width in cells.
    pub width:        u16,
    /// Popup height in cells.
    pub height:       u16,
}

/// Outer (popup) and inner (content) rects returned by
/// [`PopupFrame::render_with_areas`].
#[derive(Clone, Copy)]
pub struct PopupAreas {
    /// Outer popup rect including the border.
    pub outer: Rect,
    /// Inner content rect inside the border.
    pub inner: Rect,
}

impl PopupFrame {
    /// Render the popup chrome and return the usable inner `Rect`.
    pub fn render(self, frame: &mut Frame) -> Rect { self.render_with_areas(frame).inner }

    /// Render the popup chrome and return both outer and inner `Rect`s.
    pub fn render_with_areas(self, frame: &mut Frame) -> PopupAreas {
        let area = centered_rect(self.width, self.height, frame.area());

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

/// Center a `width × height` rect inside `area`.
#[must_use]
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
