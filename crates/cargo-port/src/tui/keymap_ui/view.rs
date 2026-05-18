use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::FrameworkOverlayId;
use tui_pane::KeymapPane;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::active_border_color;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::text_default;
use tui_pane::title_color;

use super::KEYMAP_POPUP_MAX_HEIGHT;
use super::KeymapRow;
use crate::tui::app::App;
use crate::tui::overlays::PopupFrame;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneRenderCtx;
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
                .fg(title_color())
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
                    selection.patch(Style::default().fg(text_default())),
                ),
                Span::styled(
                    key_text,
                    selection.patch(Style::default().fg(error_color())),
                ),
            ])
        } else if selection != PaneSelectionState::Unselected {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}▸ {padded_desc}"),
                    selection.patch(Style::default().fg(text_default())),
                ),
                Span::styled(
                    key_text,
                    selection.patch(if is_capturing {
                        Style::default()
                            .fg(title_color())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(label_color())
                    }),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}  {padded_desc}"),
                    Style::default().fg(text_default()),
                ),
                Span::styled(key_text, Style::default().fg(label_color())),
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

fn keep_visible_scroll_offset(
    current_offset: usize,
    selected_line: usize,
    visible_height: usize,
    line_count: usize,
) -> usize {
    if visible_height == 0 || line_count <= visible_height {
        return 0;
    }
    let max_offset = line_count - visible_height;
    let clamped = current_offset.min(max_offset);
    if selected_line < clamped {
        selected_line
    } else if selected_line >= clamped + visible_height {
        selected_line + 1 - visible_height
    } else {
        clamped
    }
}

pub(super) fn keymap_popup_height(row_count: usize, area_height: u16) -> u16 {
    let content_height = u16::try_from(row_count).unwrap_or(u16::MAX);
    content_height
        .saturating_add(2)
        .min(area_height.saturating_sub(2))
        .min(KEYMAP_POPUP_MAX_HEIGHT)
}

/// Render the Keymap overlay through the [`tui_pane::Renderable`]
/// trait. The expensive `&App` work (walking `framework_keymap`,
/// laying out rows, building [`ratatui::text::Line`]s) happens in
/// [`super::prepare_keymap_render_inputs`] before `App` is split for
/// render; this body fn just reads `self` (viewport), the
/// precomputed lines, and the inline-error string from
/// [`PaneRenderCtx`].
///
/// `area` is the full frame area — the popup centers itself within
/// it via [`PopupFrame::render_with_areas`].
pub fn render_keymap_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut KeymapPane,
    ctx: &PaneRenderCtx<'_>,
) {
    let Some(inputs) = ctx.keymap_render_inputs else {
        return;
    };

    // +2 for left/right border
    let width = (inputs.content_width + 2).min(area.width.saturating_sub(4));
    let row_count = inputs.lines.len();
    let height = keymap_popup_height(row_count, area.height);

    let popup = PopupFrame {
        title: Some(" Keymap ".to_string()),
        border_color: active_border_color(),
        width,
        height,
    }
    .render_with_areas(frame);
    let inner = popup.inner;

    pane.viewport_mut().set_len(inputs.selectable_len);
    pane.viewport_mut().set_content_area(inner);
    pane.replace_line_targets(inputs.line_targets.clone());

    let selected_pos = pane.viewport().pos();
    let line_count = inputs.lines.len();
    let visible_height = usize::from(inner.height);
    let selected_line = pane
        .line_for_selection(selected_pos)
        .unwrap_or(selected_pos);
    let scroll_offset = keep_visible_scroll_offset(
        pane.viewport().scroll_offset(),
        selected_line,
        visible_height,
        line_count,
    );
    pane.viewport_mut().set_viewport_rows(visible_height);
    pane.viewport_mut().set_scroll_offset(scroll_offset);

    let para =
        Paragraph::new(inputs.lines.clone()).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
    frame.render_widget(para, inner);
    render_overflow_affordance(
        frame,
        popup.outer,
        tui_pane::ViewportOverflow::new(line_count, scroll_offset, visible_height, selected_line),
        Style::default().fg(label_color()),
    );
}
