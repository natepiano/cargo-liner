use ratatui::Frame;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::PaneFocusState;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::ViewportOverflow;
use tui_pane::keep_visible_scroll_offset;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::selection_state;
use tui_pane::text_default;
use tui_pane::title_color;
use unicode_width::UnicodeWidthStr;

use super::constants::CONTENT_WIDTH_PADDING;
use super::constants::POPUP_BORDER_HEIGHT;
use super::constants::POPUP_HORIZONTAL_MARGIN;
use super::constants::POPUP_MIN_WIDTH;
use super::constants::POPUP_VERTICAL_MARGIN;
use super::pane::SccachePane;
use super::pane::SccacheSelectableLine;
use super::pane::SccacheStatus;
use super::pane::SccacheTarget;
use super::stats;
use super::stats::ParsedStatLine;
use super::stats::ValueAlignment;
use crate::tui::app::App;
use crate::tui::overlays::PopupFrame;

struct SccacheLines {
    lines:            Vec<Line<'static>>,
    selectable_lines: Vec<SccacheSelectableLine>,
}

pub(in crate::tui) fn render_sccache_popup(frame: &mut Frame<'_>, app: &mut App) {
    let SccacheLines {
        lines,
        selectable_lines,
    } = build_lines(&app.overlays.sccache_pane);
    let width =
        content_width(&lines).min(frame.area().width.saturating_sub(POPUP_HORIZONTAL_MARGIN));
    let height = popup_height(lines.len(), frame.area().height);
    let popup = PopupFrame {
        title: Some(" Sccache Stats ".to_string()),
        border_color: title_color(),
        width,
        height,
    }
    .render_with_areas(frame);
    let inner = popup.inner;

    let line_count = lines.len();
    app.overlays
        .sccache_pane
        .set_selectable_lines(selectable_lines);
    let visible_height = usize::from(inner.height);
    // A status-only popup starts at its first line.
    let scroll_offset = app.overlays.sccache_pane.selected_line().map_or(0, |line| {
        keep_visible_scroll_offset(line.line_index, visible_height, line_count)
    });
    let viewport = app.overlays.sccache_pane.viewport_mut();
    viewport.set_content_area(inner);
    viewport.set_viewport_rows(visible_height);
    viewport.set_scroll_offset(scroll_offset);

    let paragraph = Paragraph::new(lines).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
    frame.render_widget(paragraph, inner);
    render_overflow_affordance(
        frame,
        popup.outer,
        ViewportOverflow::new(line_count, scroll_offset, visible_height, scroll_offset),
        Style::default().fg(label_color()),
    );
}

fn build_lines(pane: &SccachePane) -> SccacheLines {
    let mut lines = Vec::new();
    let mut selectable_lines = Vec::new();
    match pane.status() {
        SccacheStatus::Loading => {
            push_header(&mut lines, "Status");
            push_item(&mut lines, "Loading sccache stats");
        },
        SccacheStatus::Ready { lines: stat_rows } => {
            push_header(&mut lines, "Stats");
            push_stat_lines(&mut lines, &mut selectable_lines, pane, stat_rows);
        },
        SccacheStatus::Failed { lines: errors } => {
            push_header(&mut lines, "Error");
            push_stat_lines(&mut lines, &mut selectable_lines, pane, errors);
        },
    }
    SccacheLines {
        lines,
        selectable_lines,
    }
}

fn push_header(lines: &mut Vec<Line<'static>>, text: &'static str) {
    lines.push(Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            format!("{text}:"),
            Style::default()
                .fg(title_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
}

fn push_item(lines: &mut Vec<Line<'static>>, text: impl Into<String>) {
    lines.push(Line::from(vec![
        Span::raw(SECTION_ITEM_INDENT),
        Span::styled(text.into(), Style::default().fg(text_default())),
    ]));
}

fn push_subheader(lines: &mut Vec<Line<'static>>, text: String) {
    lines.push(Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            text,
            Style::default()
                .fg(title_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
}

fn push_stat_lines(
    lines: &mut Vec<Line<'static>>,
    selectable_lines: &mut Vec<SccacheSelectableLine>,
    pane: &SccachePane,
    raw_lines: &[String],
) {
    let parsed = stats::parse_stat_lines(raw_lines);
    let alignment = ValueAlignment::for_lines(&parsed);
    for line in parsed {
        push_parsed_stat_line(lines, selectable_lines, pane, line, alignment);
    }
}

fn push_parsed_stat_line(
    lines: &mut Vec<Line<'static>>,
    selectable_lines: &mut Vec<SccacheSelectableLine>,
    pane: &SccachePane,
    parsed: ParsedStatLine,
    alignment: ValueAlignment,
) {
    match parsed {
        ParsedStatLine::Field { label, value } => {
            let copy_value = value.clone();
            let target = selectable_lines.len();
            selectable_lines.push(SccacheSelectableLine {
                line_index: lines.len(),
                target:     SccacheTarget {
                    label: label.clone(),
                    value: copy_value,
                },
            });
            let selection = selection_state(&pane.viewport, target, PaneFocusState::Active);
            let label_width = stat_label_width();
            lines.push(Line::from(vec![
                Span::styled(SECTION_ITEM_INDENT, selection.patch(Style::default())),
                Span::styled(
                    format!("{label:<label_width$} "),
                    selection.patch(Style::default().fg(label_color())),
                ),
                Span::styled(
                    alignment.format(&value),
                    selection.patch(Style::default().fg(text_default())),
                ),
            ]));
        },
        ParsedStatLine::Subheader { text, context: _ } => push_subheader(lines, text),
        ParsedStatLine::Text(text) => push_item(lines, text),
    }
}

const fn stat_label_width() -> usize { 36 }

fn content_width(lines: &[Line<'_>]) -> u16 {
    lines
        .iter()
        .map(line_width)
        .max()
        .and_then(|width| u16::try_from(width.saturating_add(CONTENT_WIDTH_PADDING)).ok())
        .unwrap_or(POPUP_MIN_WIDTH)
        .max(POPUP_MIN_WIDTH)
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

fn popup_height(row_count: usize, area_height: u16) -> u16 {
    let content_height = u16::try_from(row_count).unwrap_or(u16::MAX);
    let max_height = area_height.saturating_sub(POPUP_VERTICAL_MARGIN);
    content_height
        .saturating_add(POPUP_BORDER_HEIGHT)
        .min(max_height)
        .max(POPUP_BORDER_HEIGHT.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use ratatui::layout::Position;
    use ratatui::layout::Rect;

    use super::*;
    use crate::sccache::StatsResult;

    #[test]
    fn clicking_a_scrolled_stat_line_copies_its_own_value_across_headings() {
        let mut pane = SccachePane::new();
        let request = pane.start_loading();
        pane.apply_result(
            request,
            StatsResult::Ready(vec![
                "Cache hits  7".to_string(),
                String::new(),
                "Compiler:".to_string(),
                "Cache misses  3".to_string(),
                "Some explanatory text".to_string(),
            ]),
        );
        let SccacheLines {
            lines,
            selectable_lines,
        } = build_lines(&pane);
        assert!(lines[1].to_string().contains("Cache hits"));
        assert!(lines[4].to_string().contains("Cache misses"));
        pane.set_selectable_lines(selectable_lines);
        pane.viewport.set_content_area(Rect::new(10, 20, 60, 4));
        assert_eq!(pane.row_at(Position::new(10, 20)), None);
        assert_eq!(pane.row_at(Position::new(10, 21)), Some(0));
        pane.viewport.set_scroll_offset(2);
        assert_eq!(pane.row_at(Position::new(10, 20)), None);
        assert_eq!(pane.row_at(Position::new(10, 21)), None);
        let clicked_row = pane.row_at(Position::new(10, 22));
        assert_eq!(clicked_row, Some(1));
        if let Some(row) = clicked_row {
            pane.viewport.set_pos(row);
        }
        assert_eq!(pane.selected_line().map(|line| line.line_index), Some(4));
        assert_eq!(
            pane.selected_copy_value(),
            Some(&SccacheTarget {
                label: "Cache misses".to_string(),
                value: "3".to_string(),
            })
        );
        pane.handle_navigation_key(KeyCode::Up);
        assert_eq!(
            pane.selected_copy_value(),
            Some(&SccacheTarget {
                label: "Cache hits".to_string(),
                value: "7".to_string(),
            })
        );
        assert_eq!(pane.row_at(Position::new(10, 23)), None);
        assert_eq!(pane.row_at(Position::new(9, 22)), None);
    }

    #[test]
    fn status_only_lines_have_no_selection_or_copy_value() {
        let mut pane = SccachePane::new();
        let SccacheLines {
            lines,
            selectable_lines,
        } = build_lines(&pane);
        assert_eq!(lines.len(), 2);
        pane.set_selectable_lines(selectable_lines);
        pane.viewport.set_content_area(Rect::new(0, 0, 60, 4));
        assert_eq!(pane.viewport.len(), 0);
        assert_eq!(pane.selected_line(), None);
        assert_eq!(pane.selected_copy_value(), None);
        assert_eq!(pane.row_at(Position::new(0, 1)), None);
    }
}
