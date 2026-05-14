use ratatui::Frame;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::FrameworkOverlayId;
use tui_pane::render_overflow_affordance;

use super::BASE_POPUP_WIDTH;
use super::KEYMAP_POPUP_MAX_HEIGHT;
use super::KeymapRow;
use super::build_rows;
use super::selectable_row_count;
use crate::tui::app::App;
use crate::tui::constants::ACTIVE_BORDER_COLOR;
use crate::tui::constants::ERROR_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::SECTION_HEADER_INDENT;
use crate::tui::constants::SECTION_ITEM_INDENT;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::overlays::PopupFrame;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneSelectionState;
use crate::tui::panes::PaneId;

pub(super) struct KeymapLines<'a> {
    pub(super) lines:        Vec<Line<'a>>,
    pub(super) line_targets: Vec<Option<usize>>,
}

fn framework_selection_state(
    app: &App,
    selection_index: usize,
    focus: PaneFocusState,
) -> PaneSelectionState {
    let viewport = app.framework.keymap_pane.viewport();
    if selection_index == viewport.pos() && matches!(focus, PaneFocusState::Active) {
        PaneSelectionState::Active
    } else if viewport.hovered() == Some(selection_index) {
        PaneSelectionState::Hovered
    } else if selection_index == viewport.pos() && matches!(focus, PaneFocusState::Remembered) {
        PaneSelectionState::Remembered
    } else {
        PaneSelectionState::Unselected
    }
}

pub(super) fn keymap_header_line<'a>(row: &KeymapRow) -> Line<'a> {
    Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            format!("{}:", row.section),
            Style::default()
                .fg(TITLE_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

pub(super) fn build_lines<'a>(
    rows: &[KeymapRow],
    app: &App,
    is_capturing: bool,
) -> KeymapLines<'a> {
    let mut selectable_index = 0usize;
    let mut lines = vec![Line::from("")];
    let mut line_targets = vec![None];

    for row in rows {
        if row.is_header {
            lines.push(keymap_header_line(row));
            line_targets.push(None);
            continue;
        }

        let focus = if app.framework.overlay() == Some(FrameworkOverlayId::Keymap) {
            PaneFocusState::Active
        } else {
            app.pane_focus_state(PaneId::Keymap)
        };
        let selection = framework_selection_state(app, selectable_index, focus);
        let key_text = if selection != PaneSelectionState::Unselected && is_capturing {
            app.overlays
                .inline_error()
                .cloned()
                .unwrap_or_else(|| "Press key...".to_string())
        } else {
            row.key_display.clone()
        };

        let desc_width = 25usize;
        let padded_desc = format!("{:<width$}", row.description, width = desc_width);

        let line = if selection != PaneSelectionState::Unselected
            && is_capturing
            && app.overlays.inline_error().is_some()
        {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}  {padded_desc}"),
                    selection.patch(Style::default().fg(Color::White)),
                ),
                Span::styled(key_text, selection.patch(Style::default().fg(ERROR_COLOR))),
            ])
        } else if selection != PaneSelectionState::Unselected {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}▸ {padded_desc}"),
                    selection.patch(Style::default().fg(Color::White)),
                ),
                Span::styled(
                    key_text,
                    selection.patch(if is_capturing {
                        Style::default()
                            .fg(TITLE_COLOR)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(LABEL_COLOR)
                    }),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}  {padded_desc}"),
                    Style::default().fg(Color::White),
                ),
                Span::styled(key_text, Style::default().fg(LABEL_COLOR)),
            ])
        };

        lines.push(line);
        line_targets.push(Some(selectable_index));
        selectable_index += 1;
    }

    lines.push(Line::from(""));
    line_targets.push(None);
    KeymapLines {
        lines,
        line_targets,
    }
}

pub(super) fn keymap_popup_height(row_count: usize, area_height: u16) -> u16 {
    let content_height = u16::try_from(row_count).unwrap_or(u16::MAX);
    content_height
        .saturating_add(2)
        .min(area_height.saturating_sub(2))
        .min(KEYMAP_POPUP_MAX_HEIGHT)
}

pub fn render_keymap_popup(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let rows = build_rows(app);

    // Dynamic width: base fits all normal keys, expands for conflict messages.
    let content_width = app.overlays.inline_error().map_or(BASE_POPUP_WIDTH, |msg| {
        // 2 indent + 25 desc + msg len + 2 pad
        let needed = u16::try_from(2 + 25 + msg.len() + 2).unwrap_or(u16::MAX);
        BASE_POPUP_WIDTH.max(needed)
    });
    // +2 for left/right border
    let width = (content_width + 2).min(area.width.saturating_sub(4));

    let height = keymap_popup_height(rows.len(), area.height);

    let popup = PopupFrame {
        title: Some(" Keymap ".to_string()),
        border_color: ACTIVE_BORDER_COLOR,
        width,
        height,
    }
    .render_with_areas(frame);
    let inner = popup.inner;

    let selectable_len = selectable_row_count(app);
    app.framework
        .keymap_pane
        .viewport_mut()
        .set_len(selectable_len);
    app.framework
        .keymap_pane
        .viewport_mut()
        .set_content_area(inner);

    let selected_pos = app.framework.keymap_pane.viewport().pos();
    let is_capturing = app.framework.keymap_pane.is_capturing();
    let rendered = build_lines(&rows, app, is_capturing);

    // Scroll to keep selection visible.
    let visible_height = usize::from(inner.height);
    let scroll_offset = if selected_pos >= visible_height {
        selected_pos - visible_height + 1
    } else {
        0
    };
    app.framework
        .keymap_pane
        .viewport_mut()
        .set_viewport_rows(visible_height);
    app.framework
        .keymap_pane
        .viewport_mut()
        .set_scroll_offset(scroll_offset);
    app.framework
        .keymap_pane
        .replace_line_targets(rendered.line_targets);

    let para =
        Paragraph::new(rendered.lines).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
    frame.render_widget(para, inner);
    render_overflow_affordance(
        frame,
        popup.outer,
        app.framework.keymap_pane.viewport().overflow(),
        Style::default().fg(LABEL_COLOR),
    );
}
