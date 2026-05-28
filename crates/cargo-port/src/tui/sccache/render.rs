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
use tui_pane::active_border_color;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::selection_state;
use tui_pane::text_default;
use tui_pane::title_color;
use unicode_width::UnicodeWidthStr;

use super::pane::SccachePane;
use super::pane::SccacheStatus;
use super::pane::SccacheTarget;
use super::stats;
use super::stats::ParsedStatLine;
use super::stats::ValueAlignment;
use crate::tui::app::App;
use crate::tui::overlays::PopupFrame;

const POPUP_MIN_WIDTH: u16 = 56;
const POPUP_HORIZONTAL_MARGIN: u16 = 4;
const POPUP_VERTICAL_MARGIN: u16 = 4;
const POPUP_BORDER_HEIGHT: u16 = 2;
const CONTENT_WIDTH_PADDING: usize = 2;

struct SccacheLines {
    lines:             Vec<Line<'static>>,
    line_targets:      Vec<Option<usize>>,
    selectable_values: Vec<SccacheTarget>,
}

pub fn render_sccache_popup(frame: &mut Frame<'_>, app: &mut App) {
    let SccacheLines {
        lines,
        line_targets,
        selectable_values,
    } = build_lines(&app.overlays.sccache_pane);
    let width =
        content_width(&lines).min(frame.area().width.saturating_sub(POPUP_HORIZONTAL_MARGIN));
    let height = popup_height(lines.len(), frame.area().height);
    let popup = PopupFrame {
        title: Some(" Sccache Stats ".to_string()),
        border_color: active_border_color(),
        width,
        height,
    }
    .render_with_areas(frame);
    let inner = popup.inner;

    let line_count = lines.len();
    let selectable_count = selectable_values.len();
    app.overlays
        .sccache_pane
        .set_line_targets(line_targets, selectable_values);
    let visible_height = usize::from(inner.height);
    let selected_line = app
        .overlays
        .sccache_pane
        .line_for_selection(app.overlays.sccache_pane.viewport.pos())
        .unwrap_or_else(|| app.overlays.sccache_pane.viewport.pos());
    let scroll_offset = keep_visible_scroll_offset(selected_line, visible_height, line_count);
    let viewport = app.overlays.sccache_pane.viewport_mut();
    viewport.set_len(selectable_count);
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
    let mut line_targets = Vec::new();
    let mut selectable_values = Vec::new();
    match pane.status() {
        SccacheStatus::Loading { source } => {
            push_header(&mut lines, &mut line_targets, "Status");
            push_item(&mut lines, &mut line_targets, "Loading sccache stats");
            push_source(
                &mut lines,
                &mut line_targets,
                &mut selectable_values,
                pane,
                source,
            );
        },
        SccacheStatus::NotConfigured => {
            push_header(&mut lines, &mut line_targets, "Status");
            push_item(
                &mut lines,
                &mut line_targets,
                "sccache is not configured for this process",
            );
            push_item(
                &mut lines,
                &mut line_targets,
                "Set RUSTC_WRAPPER=sccache to enable stats",
            );
        },
        SccacheStatus::Ready {
            source,
            lines: stat_rows,
        } => {
            push_header(&mut lines, &mut line_targets, "Configured");
            push_source(
                &mut lines,
                &mut line_targets,
                &mut selectable_values,
                pane,
                source,
            );
            lines.push(Line::from(""));
            line_targets.push(None);
            push_header(&mut lines, &mut line_targets, "Stats");
            push_stat_lines(
                &mut lines,
                &mut line_targets,
                &mut selectable_values,
                pane,
                stat_rows,
            );
        },
        SccacheStatus::Failed {
            source,
            lines: errors,
        } => {
            push_header(&mut lines, &mut line_targets, "Configured");
            push_source(
                &mut lines,
                &mut line_targets,
                &mut selectable_values,
                pane,
                source,
            );
            lines.push(Line::from(""));
            line_targets.push(None);
            push_header(&mut lines, &mut line_targets, "Error");
            push_stat_lines(
                &mut lines,
                &mut line_targets,
                &mut selectable_values,
                pane,
                errors,
            );
        },
    }
    SccacheLines {
        lines,
        line_targets,
        selectable_values,
    }
}

fn push_header(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    text: &'static str,
) {
    lines.push(Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            format!("{text}:"),
            Style::default()
                .fg(title_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    line_targets.push(None);
}

fn push_source(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    selectable_values: &mut Vec<SccacheTarget>,
    pane: &SccachePane,
    source: &str,
) {
    let target = push_target(
        line_targets,
        selectable_values,
        "Configured",
        source.to_string(),
    );
    let selection = selection_state(&pane.viewport, target, PaneFocusState::Active);
    lines.push(Line::from(vec![
        Span::styled(SECTION_ITEM_INDENT, selection.patch(Style::default())),
        Span::styled(
            source.to_string(),
            selection.patch(Style::default().fg(text_default())),
        ),
    ]));
}

fn push_item(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    text: impl Into<String>,
) {
    lines.push(Line::from(vec![
        Span::raw(SECTION_ITEM_INDENT),
        Span::styled(text.into(), Style::default().fg(text_default())),
    ]));
    line_targets.push(None);
}

fn push_subheader(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    text: String,
) {
    lines.push(Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            text,
            Style::default()
                .fg(title_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    line_targets.push(None);
}

fn push_stat_lines(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    selectable_values: &mut Vec<SccacheTarget>,
    pane: &SccachePane,
    raw_lines: &[String],
) {
    let parsed = stats::parse_stat_lines(raw_lines);
    let alignment = ValueAlignment::for_lines(&parsed);
    for line in parsed {
        push_parsed_stat_line(
            lines,
            line_targets,
            selectable_values,
            pane,
            line,
            alignment,
        );
    }
}

fn push_parsed_stat_line(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    selectable_values: &mut Vec<SccacheTarget>,
    pane: &SccachePane,
    parsed: ParsedStatLine,
    alignment: ValueAlignment,
) {
    match parsed {
        ParsedStatLine::Field { label, value } => {
            let copy_value = value.clone();
            let target = push_target(line_targets, selectable_values, &label, copy_value);
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
        ParsedStatLine::Subheader { text, context: _ } => push_subheader(lines, line_targets, text),
        ParsedStatLine::Text(text) => push_item(lines, line_targets, text),
    }
}

fn push_target(
    line_targets: &mut Vec<Option<usize>>,
    selectable_values: &mut Vec<SccacheTarget>,
    label: &str,
    value: String,
) -> usize {
    let target = selectable_values.len();
    line_targets.push(Some(target));
    selectable_values.push(SccacheTarget {
        label: label.to_string(),
        value,
    });
    target
}

const fn stat_label_width() -> usize { 36 }

fn keep_visible_scroll_offset(
    selected_line: usize,
    visible_height: usize,
    line_count: usize,
) -> usize {
    if visible_height == 0 || line_count <= visible_height {
        return 0;
    }
    let max_offset = line_count - visible_height;
    if selected_line >= visible_height {
        (selected_line + 1 - visible_height).min(max_offset)
    } else {
        0
    }
}

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
