//! [`draw_settings`]: the settings popup, sized to its widest row.

use ratatui::Frame;
#[cfg(any(test, feature = "contrast-test"))]
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;

use super::constants::MIN_SELECTED_ROW_CONTRAST;
use super::constants::SETTINGS_POPUP_WIDTH;
use super::constants::SETTINGS_TITLE;
use super::constants::VALUE_RIGHT_GAP;
use super::rows::SettingsRows;
use crate::ColorMode;
use crate::PaneFocusState;
use crate::PopupFrame;
use crate::SECTION_HEADER_INDENT;
use crate::SECTION_ITEM_INDENT;
use crate::SettingsPane;
use crate::SettingsRenderOptions;
use crate::SettingsRow;
use crate::Theme;
use crate::contrast_ratio;
use crate::overlays::POPUP_BORDER_HEIGHT;
use crate::overlays::POPUP_BORDER_WIDTH;
use crate::selection_style;
#[cfg(any(test, feature = "contrast-test"))]
use crate::support::contrast;
use crate::theme;

/// Draw the settings overlay centred over the whole frame.
///
/// The popup fits the widest row, the right gap, and the borders.
/// It uses at least 64 columns when the frame has room.
/// Its height fits the lines and the borders within the frame.
/// The viewport scrolls to keep the selection visible.
pub fn draw_settings<S: Copy>(
    frame: &mut Frame,
    settings_pane: &mut SettingsPane,
    rows: &SettingsRows<S>,
    mode: ColorMode,
) {
    settings_pane.draw_popup(frame, rows, mode);
}

impl SettingsPane {
    /// Find the selected row by its initial words and check its drawn contrast.
    ///
    /// # Panics
    ///
    /// Panics if the row is missing or any text fails the contrast threshold.
    #[cfg(any(test, feature = "contrast-test"))]
    pub fn assert_selected_row_contrast(
        buffer: &Buffer,
        area: Rect,
        expected_row: &str,
        case: &str,
    ) {
        contrast::assert_drawn_row_contrast(
            buffer,
            area,
            expected_row,
            MIN_SELECTED_ROW_CONTRAST,
            case,
        );
    }

    pub(crate) fn draw_popup<S: Copy>(
        &mut self,
        frame: &mut Frame,
        rows: &SettingsRows<S>,
        mode: ColorMode,
    ) -> Rect {
        self.draw_popup_rows(
            frame,
            rows.rows(),
            fitted_width(rows.widest_row()),
            mode,
            &theme(),
        )
    }

    fn draw_popup_rows(
        &mut self,
        frame: &mut Frame,
        rows: &[SettingsRow],
        requested_width: u16,
        mode: ColorMode,
        palette: &Theme,
    ) -> Rect {
        let area = frame.area();
        // Lay out the rows at the width that fits in the frame.
        let width = requested_width.min(area.width);
        let content_width = width.saturating_sub(POPUP_BORDER_WIDTH + VALUE_RIGHT_GAP);
        let settings_render_options = SettingsRenderOptions {
            focus:                 PaneFocusState::Active,
            inline_error:          None,
            content_width:         usize::from(content_width),
            section_header_indent: SECTION_HEADER_INDENT,
            section_item_indent:   SECTION_ITEM_INDENT,
            title_style:           Style::default().fg(palette.pane_chrome.active_title.color),
            label_style:           Style::default().fg(palette.semantic.label.color),
            muted_style:           Style::default().fg(palette.semantic.label.color),
            success_style:         Style::default().fg(palette.semantic.success.color),
            error_style:           Style::default().fg(palette.semantic.error.color),
            inline_error_style:    Style::default().fg(palette.semantic.inline_error.color),
            active_style:          settings_selection_style(palette, mode),
            remembered_style:      selection_style(PaneFocusState::Remembered, mode),
            hovered_style:         Style::default().bg(palette.focus.hover.color),
        };
        let settings_render = self.render_rows(rows, settings_render_options);
        let line_count = u16::try_from(settings_render.lines.len()).unwrap_or(u16::MAX);
        let height = line_count
            .saturating_add(POPUP_BORDER_HEIGHT)
            .min(area.height);
        let popup_areas = PopupFrame {
            color_mode: mode,
            title: Some(SETTINGS_TITLE.to_string()),
            border_color: palette.pane_chrome.active_title.color,
            width,
            height,
        }
        .render_with_areas(frame);

        self.set_content_area(popup_areas.inner);
        self.update_scroll();
        self.render_lines_at_width(frame, settings_render.lines, content_width);
        popup_areas.outer
    }
}

/// Use one readable foreground for the selected label and value.
fn settings_selection_style(palette: &Theme, mode: ColorMode) -> Style {
    if mode == ColorMode::NoColor {
        return selection_style(PaneFocusState::Active, mode);
    }

    let ground = palette.focus.active.color;
    let style = Style::new().bg(ground);
    let ink = palette.text.default.color;
    if ground == Color::Reset
        || contrast_ratio(ink, ground).is_some_and(|ratio| ratio >= MIN_SELECTED_ROW_CONTRAST)
    {
        return style.fg(ink);
    }

    let black = Color::Rgb(0, 0, 0);
    let white = Color::Rgb(255, 255, 255);
    if contrast_ratio(black, ground) >= contrast_ratio(white, ground) {
        style.fg(black)
    } else {
        style.fg(white)
    }
}

/// Width for the widest row, the right gap, and the borders.
/// The caller clamps the width to the terminal.
fn fitted_width(widest_row: usize) -> u16 {
    let extra = usize::from(POPUP_BORDER_WIDTH + VALUE_RIGHT_GAP);
    let width = u16::try_from(widest_row.saturating_add(extra)).unwrap_or(u16::MAX);
    width.max(SETTINGS_POPUP_WIDTH)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Position;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui::style::Style;
    use ratatui::text::Line;

    use super::SettingsRows;
    use super::fitted_width;
    use crate::Appearance;
    use crate::ColorMode;
    use crate::PaneFocusState;
    use crate::SECTION_HEADER_INDENT;
    use crate::SECTION_ITEM_INDENT;
    use crate::SettingsPane;
    use crate::SettingsRenderOptions;
    use crate::SettingsRow;
    use crate::SettingsRowHit;
    use crate::Theme;
    use crate::fallback_theme;

    #[derive(Clone, Copy)]
    struct Speed;

    /// The measured width is exactly what the pane draws: a longer
    /// label, a longer plain value and a longer stepper value each set
    /// the widest line.
    #[test]
    fn measured_width_matches_the_rendered_rows() {
        let mut rows = SettingsRows::new();
        rows.section("Section");
        rows.stepper(Speed, "speed", "a stepped value");
        rows.value("a much longer label", "short".to_string());
        rows.value(
            "where",
            "a plain value wider than the stepped one".to_string(),
        );
        let settings_render_options = SettingsRenderOptions {
            focus:                 PaneFocusState::Active,
            inline_error:          None,
            content_width:         usize::from(u16::MAX),
            section_header_indent: SECTION_HEADER_INDENT,
            section_item_indent:   SECTION_ITEM_INDENT,
            title_style:           Style::default(),
            label_style:           Style::default(),
            muted_style:           Style::default(),
            success_style:         Style::default(),
            error_style:           Style::default(),
            inline_error_style:    Style::default(),
            active_style:          Style::default(),
            remembered_style:      Style::default(),
            hovered_style:         Style::default(),
        };
        let settings_render = SettingsPane::new().render_rows(rows.rows(), settings_render_options);
        let widest = settings_render.lines.iter().map(Line::width).max();
        assert_eq!(widest, Some(rows.widest_row()));
    }

    #[test]
    fn a_narrow_overlay_keeps_the_minimum_width() {
        assert_eq!(fitted_width(0), 64);
        assert_eq!(fitted_width(100), 103);
    }

    #[test]
    fn the_overlay_selected_row_meets_contrast() -> Result<(), Box<dyn Error>> {
        let mut theme = fallback_theme(Appearance::Dark);
        theme.focus.active.color = Color::Rgb(125, 125, 125);
        let rows = [SettingsRow::stepper(0, "view", "summary")];
        let mut pane = SettingsPane::new();
        let (buffer, _) = draw_popup(&mut pane, &rows, 64, &theme)?;
        SettingsPane::assert_selected_row_contrast(
            &buffer,
            pane.viewport().content_area(),
            "▶ view < summary >",
            "gray selection",
        );
        Ok(())
    }

    #[test]
    fn a_long_settings_value_wraps_short_of_the_border() -> Result<(), Box<dyn Error>> {
        let words = "alpha ".repeat(18);
        let directories = format!("/{}", "directory/".repeat(10));
        let long_word = "w".repeat(109);
        let mut rows = SettingsRows::<Speed>::new();
        rows.value(
            "list",
            format!("{words}x next\n{directories}component/leaf\n{long_word} last"),
        );
        rows.stepper(Speed, "mode", &format!("{words}x next"));

        let theme = fallback_theme(Appearance::Dark);
        let mut pane = SettingsPane::new();
        let (buffer, popup) = draw_popup(&mut pane, rows.rows(), 120, &theme)?;
        let expected = [
            words.trim_end().to_owned(),
            "x next".to_owned(),
            directories,
            "component/leaf".to_owned(),
            format!("{}…", "w".repeat(107)),
            "last".to_owned(),
            format!("< {}", "alpha ".repeat(17).trim_end()),
            "  alpha x next >".to_owned(),
        ];
        assert_eq!(usize::from(popup.height), expected.len() + 2);
        let value_left = popup.left() + 1 + u16::try_from(SECTION_ITEM_INDENT.len() + 8)?;
        let border = popup.right() - 1;
        for (y, text) in (popup.top() + 1..popup.bottom() - 1).zip(expected) {
            assert_eq!(buffer[(border, y)].symbol(), "│");
            assert_eq!(buffer[(border - 1, y)].symbol(), " ");
            assert_eq!(drawn_text(&buffer, value_left, border, y), text);
        }

        let toggle = SettingsRow::toggle(0, "Vim nav keys", true)
            .with_suffix("  maps h/j/k/l to arrow navigation");
        let (buffer, popup) = draw_popup(&mut SettingsPane::new(), &[toggle], 34, &theme)?;
        let expected = [
            "▶ Vim nav keys  < ON >",
            "maps h/j/k/l",
            "to arrow",
            "navigation",
        ];
        assert_eq!(usize::from(popup.height), expected.len() + 2);
        let border = popup.right() - 1;
        for (y, text) in (popup.top() + 1..popup.bottom() - 1).zip(expected) {
            assert_eq!(buffer[(border, y)].symbol(), "│");
            assert_eq!(buffer[(border - 1, y)].symbol(), " ");
            assert_eq!(
                drawn_text(&buffer, popup.left() + 1, border, y).trim_start(),
                text
            );
        }

        let narrow_rows = [
            SettingsRow::toggle(0, "Vim nav keys", false),
            SettingsRow::stepper(1, "Dark theme", "High Contrast Dark"),
            SettingsRow::stepper(2, "Finished task visible secs", "9"),
        ];
        let (buffer, popup) = draw_popup(&mut SettingsPane::new(), &narrow_rows, 37, &theme)?;
        let border = popup.right() - 1;
        let y = popup.top() + 1;
        assert_eq!(
            drawn_text(&buffer, popup.left() + 1, border, y).trim_start(),
            format!("▶ Vim nav keys{}< OFF >", " ".repeat(12))
        );
        let theme_lines = [y + 1, y + 2, y + 3].map(|y| {
            let text = drawn_text(&buffer, popup.left() + 1, border, y);
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        });
        assert_eq!(theme_lines, ["Dark theme < High", "Contrast", "Dark >"]);
        for y in popup.top() + 1..popup.bottom() - 1 {
            assert_eq!(buffer[(border, y)].symbol(), "│");
            assert_eq!(buffer[(border - 1, y)].symbol(), " ");
        }
        Ok(())
    }

    #[test]
    fn a_settings_row_accepts_a_click_in_the_right_gap() -> Result<(), Box<dyn Error>> {
        let mut pane = SettingsPane::new();
        let rows = [SettingsRow::value(0, "where", "config")];
        let (buffer, popup) = draw_popup(&mut pane, &rows, 64, &fallback_theme(Appearance::Dark))?;
        let gap = Position::new(popup.right() - 2, popup.top() + 1);
        assert_eq!(buffer[gap].symbol(), " ");
        assert_eq!(pane.row_at(gap), SettingsRowHit::Row(0));
        assert_eq!(
            pane.row_at(Position::new(popup.right() - 1, gap.y)),
            SettingsRowHit::Missed
        );
        Ok(())
    }

    fn draw_popup(
        pane: &mut SettingsPane,
        rows: &[SettingsRow],
        width: u16,
        theme: &Theme,
    ) -> Result<(Buffer, Rect), Box<dyn Error>> {
        let mut terminal = Terminal::new(TestBackend::new(width, 40))?;
        let mut popup = Rect::default();
        terminal.draw(|frame| {
            popup = pane.draw_popup_rows(frame, rows, width, ColorMode::Colored, theme);
        })?;
        Ok((terminal.backend().buffer().clone(), popup))
    }

    fn drawn_text(buffer: &Buffer, left: u16, right: u16, y: u16) -> String {
        let mut text: String = (left..right).map(|x| buffer[(x, y)].symbol()).collect();
        text.truncate(text.trim_end().len());
        text
    }
}
