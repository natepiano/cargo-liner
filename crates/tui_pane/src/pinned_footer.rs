use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

/// Whether a footer follows content or stays at the viewport's bottom.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FooterPlacement {
    /// All content and footer rows share the scrolling list.
    Inline,
    /// Reserve the viewport's bottom rows for the footer.
    Pinned,
}

/// Pin a nonempty footer only when content overflows and some content fits.
#[must_use]
pub fn footer_placement(
    content_rows: usize,
    footer_rows: usize,
    inner_height: u16,
) -> FooterPlacement {
    if footer_rows > 0
        && usize::from(inner_height) > footer_rows
        && content_rows.saturating_add(footer_rows) > usize::from(inner_height)
    {
        FooterPlacement::Pinned
    } else {
        FooterPlacement::Inline
    }
}

/// Paint styled footer lines at the bottom of the content rectangle.
pub fn render_pinned_footer_lines(frame: &mut Frame, content_area: Rect, lines: &[Line<'static>]) {
    let footer_height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let footer_area = Rect::new(
        content_area.x,
        content_area
            .y
            .saturating_add(content_area.height.saturating_sub(footer_height)),
        content_area.width,
        footer_height,
    );
    for (index, line) in lines.iter().cloned().enumerate() {
        let y = footer_area
            .y
            .saturating_add(u16::try_from(index).unwrap_or(u16::MAX));
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(footer_area.x, y, footer_area.width, 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::FooterPlacement;
    use super::footer_placement;

    #[test]
    fn footer_stays_inline_when_everything_fits() {
        assert_eq!(footer_placement(5, 1, 6), FooterPlacement::Inline);
    }

    #[test]
    fn footer_pins_when_content_overflows() {
        assert_eq!(footer_placement(6, 1, 6), FooterPlacement::Pinned);
    }

    #[test]
    fn footer_does_not_pin_without_footer_content() {
        assert_eq!(footer_placement(100, 0, 6), FooterPlacement::Inline);
    }

    #[test]
    fn footer_stays_inline_without_room_for_content_rows() {
        assert_eq!(footer_placement(1, 2, 2), FooterPlacement::Inline);
    }

    #[test]
    fn overflowing_content_count_saturates_when_deciding_placement() {
        assert_eq!(footer_placement(usize::MAX, 2, 8), FooterPlacement::Pinned);
    }

    #[test]
    fn painter_places_each_line_at_the_content_bottom() -> Result<(), Infallible> {
        let backend = ratatui::backend::TestBackend::new(12, 6);
        let mut terminal = ratatui::Terminal::new(backend)?;
        terminal.draw(|frame| {
            super::render_pinned_footer_lines(
                frame,
                ratatui::layout::Rect::new(2, 1, 8, 4),
                &["first".into(), "second".into()],
            );
        })?;
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(2, 2)].symbol(), " ");
        assert_eq!(buffer[(2, 3)].symbol(), "f");
        assert_eq!(buffer[(2, 4)].symbol(), "s");
        assert_eq!(buffer[(2, 5)].symbol(), " ");
        Ok(())
    }
}
