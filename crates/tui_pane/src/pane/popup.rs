use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;

use crate::ColorMode;
use crate::title_color;

/// Shared chrome for popup overlays.
///
/// Handles centering, background clearing, and a bordered frame with
/// an optional left-aligned bold title.
pub struct PopupFrame {
    /// The colour mode selected by the app.
    pub color_mode:   ColorMode,
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

        let border_style = if self.color_mode == ColorMode::NoColor {
            Style::new()
                .fg(self.border_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(self.border_color)
        };
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(if self.color_mode == ColorMode::NoColor {
                BorderType::Thick
            } else {
                BorderType::Plain
            })
            .border_style(border_style);

        if let Some(title) = self.title {
            block = block.title(Span::styled(title, title_style()));
        }

        let inner_rect = block.inner(area);
        frame.render_widget(block, area);
        PopupAreas {
            outer: area,
            inner: inner_rect,
        }
    }
}

fn title_style() -> Style { Style::new().fg(title_color()).add_modifier(Modifier::BOLD) }

/// Center a `width × height` rect inside `area`.
#[must_use]
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;
    use ratatui::style::Modifier;

    use super::PopupFrame;
    use crate::ColorMode;

    fn draw_popup(color_mode: ColorMode) -> Result<Buffer, Infallible> {
        let mut terminal = Terminal::new(TestBackend::new(12, 7))?;
        terminal.draw(|frame| {
            PopupFrame {
                color_mode,
                title: None,
                border_color: Color::White,
                width: 8,
                height: 5,
            }
            .render(frame);
        })?;
        Ok(terminal.backend().buffer().clone())
    }

    #[test]
    fn colored_popup_draws_a_plain_border_without_bold() -> Result<(), Infallible> {
        let buffer = draw_popup(ColorMode::Colored)?;
        assert_eq!(buffer[(2, 1)].symbol(), "┌");
        assert_eq!(buffer[(3, 1)].symbol(), "─");
        assert!(!buffer[(2, 1)].modifier.contains(Modifier::BOLD));
        assert!(!buffer[(3, 1)].modifier.contains(Modifier::BOLD));
        Ok(())
    }

    #[test]
    fn no_color_popup_draws_a_thick_bold_border() -> Result<(), Infallible> {
        let buffer = draw_popup(ColorMode::NoColor)?;
        assert_eq!(buffer[(2, 1)].symbol(), "┏");
        assert_eq!(buffer[(3, 1)].symbol(), "━");
        assert!(buffer[(2, 1)].modifier.contains(Modifier::BOLD));
        assert!(buffer[(3, 1)].modifier.contains(Modifier::BOLD));
        Ok(())
    }
}
