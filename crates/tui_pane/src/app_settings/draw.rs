//! [`draw_settings`]: the settings popup, sized to its widest row.

use ratatui::Frame;
use ratatui::style::Style;

use super::constants::SETTINGS_POPUP_WIDTH;
use super::constants::SETTINGS_TITLE;
use super::rows::SettingsRows;
use crate::PaneFocusState;
use crate::PopupFrame;
use crate::SECTION_HEADER_INDENT;
use crate::SECTION_ITEM_INDENT;
use crate::SettingsPane;
use crate::SettingsRenderOptions;
use crate::error_color;
use crate::hover_focus_color;
use crate::inline_error_color;
use crate::label_color;
use crate::overlays::POPUP_BORDER_HEIGHT;
use crate::overlays::POPUP_BORDER_WIDTH;
use crate::selection_style;
use crate::success_color;
use crate::title_color;

/// Draw the settings overlay centred over the whole frame.
///
/// The popup is as wide as the widest row plus its border, never
/// narrower than 64 cells, and clamped to the frame; it is as tall as
/// the rendered lines plus its border, clamped the same way. Colours
/// come from the active theme. The pane's viewport is fitted to the
/// popup's inside and scrolled to keep the selection visible before the
/// lines are painted.
pub fn draw_settings<S: Copy>(frame: &mut Frame, pane: &mut SettingsPane, rows: &SettingsRows<S>) {
    let area = frame.area();
    // The popup centers itself and clamps to the frame, so the rows have
    // to be laid out for the width that survives that clamp.
    let width = fitted_width(rows.widest_row()).min(area.width);
    let content_width = usize::from(width.saturating_sub(POPUP_BORDER_WIDTH));
    let options = SettingsRenderOptions {
        focus: PaneFocusState::Active,
        inline_error: None,
        content_width,
        section_header_indent: SECTION_HEADER_INDENT,
        section_item_indent: SECTION_ITEM_INDENT,
        title_style: Style::default().fg(title_color()),
        label_style: Style::default().fg(label_color()),
        muted_style: Style::default().fg(label_color()),
        success_style: Style::default().fg(success_color()),
        error_style: Style::default().fg(error_color()),
        inline_error_style: Style::default().fg(inline_error_color()),
        active_style: selection_style(PaneFocusState::Active),
        remembered_style: selection_style(PaneFocusState::Remembered),
        hovered_style: Style::default().bg(hover_focus_color()),
    };
    let rendered = pane.render_rows(rows.rows(), options);
    let line_count = u16::try_from(rendered.lines.len()).unwrap_or(u16::MAX);
    let height = line_count
        .saturating_add(POPUP_BORDER_HEIGHT)
        .min(area.height);
    let popup = PopupFrame {
        title: Some(SETTINGS_TITLE.to_string()),
        border_color: title_color(),
        width,
        height,
    }
    .render_with_areas(frame);

    let viewport = pane.viewport_mut();
    viewport.set_content_area(popup.inner);
    viewport.set_viewport_rows(usize::from(popup.inner.height));
    pane.update_scroll();
    pane.render_lines(frame, rendered.lines);
}

/// Popup width that fits the widest row plus borders, never narrower
/// than [`SETTINGS_POPUP_WIDTH`]. The caller clamps it to the terminal.
fn fitted_width(widest_row: usize) -> u16 {
    let width = u16::try_from(widest_row.saturating_add(usize::from(POPUP_BORDER_WIDTH)))
        .unwrap_or(u16::MAX);
    width.max(SETTINGS_POPUP_WIDTH)
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;
    use ratatui::text::Line;

    use super::SettingsRows;
    use super::fitted_width;
    use crate::PaneFocusState;
    use crate::SECTION_HEADER_INDENT;
    use crate::SECTION_ITEM_INDENT;
    use crate::SettingsPane;
    use crate::SettingsRenderOptions;

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
        let options = SettingsRenderOptions {
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
        let rendered = SettingsPane::new().render_rows(rows.rows(), options);
        let widest = rendered.lines.iter().map(Line::width).max();
        assert_eq!(widest, Some(rows.widest_row()));
    }

    #[test]
    fn a_narrow_overlay_keeps_the_minimum_width() {
        assert_eq!(fitted_width(0), 64);
        assert_eq!(fitted_width(100), 102);
    }
}
