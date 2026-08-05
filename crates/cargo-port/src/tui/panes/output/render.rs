use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::finder_match_bg;
use tui_pane::label_color;

use super::constants::PANE_BORDER_COLUMNS;
use super::constants::PANE_CHROME_ROWS;
use super::hit_map::MonitorHitMap;
use super::monitor_render::ColumnActivityExtent;
use super::monitor_render::OwnedBody;
use super::monitor_render::SelectedColumnScroll;
use super::monitor_render::owned_captured_output_height;
use super::monitor_render::render_monitor_half;
use super::monitor_render::selected_column_activity_extent;
use super::pane::OutputPane;
use super::presentation::MonitorVisibility;
use super::presentation::OwnedOutputVisibility;
use crate::tui::panes::pane_data;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::state::OwnedRunOutputTitleRef;
use crate::tui::state::OwnedRunRunningLabelRef;

pub fn render_output_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut OutputPane,
    ctx: &PaneRenderCtx<'_>,
) {
    // Render and yank read the frozen snapshot once the selection is
    // pinned off the tail, so streaming output can't drift the range;
    // while following the tail they read the live buffer. Cloning the
    // `Rc` only bumps the refcount and releases the `&pane` borrow before
    // `sync_viewport`.
    let live = ctx.output_presentation.captured_lines();
    let visual_selection_source = pane.selection().visual_selection_source().clone();
    let source: &[String] = visual_selection_source.lines(live);

    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(PANE_BORDER_COLUMNS),
        area.height.saturating_sub(PANE_CHROME_ROWS),
    );
    // The cursor settles first: the selected column's scroll offset belongs to
    // whichever column the cursor ended this frame in, and the owned body's
    // extent depends on how many of that column's activity rows are drawn.
    pane.reconcile_cursor(&ctx.output_presentation);
    let activity_extent = match ctx.output_presentation.monitor() {
        MonitorVisibility::Off => ColumnActivityExtent::none(),
        MonitorVisibility::On(monitor_presentation) => {
            selected_column_activity_extent(inner, monitor_presentation, pane.cursor())
        },
    };
    pane.sync_column_scroll(activity_extent.visible_rows, activity_extent.row_count);
    let selected_column_scroll =
        SelectedColumnScroll::new(pane.cursor().column_index(), pane.column_scroll_offset());

    // The viewport scrolls the captured output wherever it is drawn, so its
    // extent is the height that body gets: the whole inner pane with the
    // monitor off, and the rows left under the owned column's header, drawn
    // activity rows, and separator with it on.
    let visible_rows = match (
        ctx.output_presentation.monitor(),
        ctx.output_presentation.owned_output(),
    ) {
        (MonitorVisibility::On(monitor_presentation), OwnedOutputVisibility::OnScreen(owned)) => {
            owned_captured_output_height(
                inner,
                monitor_presentation,
                owned.producer(),
                selected_column_scroll,
            )
        },
        _ => usize::from(inner.height),
    };
    pane.sync_viewport(source.len(), visible_rows, inner);

    // The title reports follow state and selected-line count, both of which
    // read the viewport length this frame's buffer just set, so it is built
    // after the sync rather than from the previous frame's length.
    let focused = pane.focus.is_focused();
    let block = tui_pane::default_pane_chrome()
        .with_inactive_border(Style::default().fg(label_color()))
        .block(output_title(pane, ctx), focused);
    frame.render_widget(block, area);

    let selected_range = pane.selected_range(source);
    match ctx.output_presentation.monitor() {
        MonitorVisibility::Off => {
            render_captured_output(frame, inner, pane, source);
            pane.set_monitor_hit_map(MonitorHitMap::new());
        },
        MonitorVisibility::On(monitor_presentation) => {
            // The monitor's owned body scrolls on the same viewport offset the
            // monitor-off view uses, so switching the monitor on does not turn
            // captured output into a head-only view.
            let owned_body = match ctx.output_presentation.owned_output() {
                OwnedOutputVisibility::Absent => None,
                OwnedOutputVisibility::OnScreen(owned) => Some(OwnedBody::new(
                    owned,
                    source,
                    selected_range,
                    pane.viewport.scroll_offset(),
                )),
            };
            let mut monitor_hit_map = MonitorHitMap::new();
            render_monitor_half(
                frame,
                inner,
                monitor_presentation,
                owned_body,
                pane.cursor(),
                selected_column_scroll,
                &mut monitor_hit_map,
            );
            pane.set_monitor_hit_map(monitor_hit_map);
        },
    }
}

/// The monitor-off view: Cargo Port's own captured output filling the pane, as
/// it was drawn before build monitoring existed.
fn render_captured_output(frame: &mut Frame, inner: Rect, pane: &OutputPane, source: &[String]) {
    let scroll_offset = u16::try_from(pane.viewport.scroll_offset()).unwrap_or(u16::MAX);
    let selected_range = pane.selected_range(source);
    let inner_width = usize::from(inner.width);

    // There is always a selection — at minimum the single cursor row — so
    // it is drawn in one color (the selection background). A single
    // highlighted row is just a one-line selection; an extended range is
    // the same color, wider.
    let lines: Vec<Line> = source
        .iter()
        .enumerate()
        .map(|(row, raw)| {
            let parsed = parse_output_line(raw);
            if selected_range.contains(row) {
                fill_row(parsed, inner_width)
            } else {
                parsed
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).scroll((scroll_offset, 0)), inner);
}

/// Force the selection background onto every span (overriding the per-span
/// backgrounds the ANSI parser sets, while keeping each span's foreground) and
/// pad the line with trailing spaces to `width`, so the highlight covers the
/// full row including the colored log text rather than stopping at the
/// timestamp.
pub(super) fn fill_row(parsed: Line<'static>, width: usize) -> Line<'static> {
    let highlight = Style::default().bg(finder_match_bg());
    let mut line = Line::from(
        parsed
            .spans
            .into_iter()
            .map(|span| span.patch_style(highlight))
            .collect::<Vec<_>>(),
    );
    let used = line.width();
    if width > used {
        line.spans
            .push(Span::styled(" ".repeat(width - used), highlight));
    }
    line
}

/// Parse one raw output line (carrying ANSI) into a styled `Line`,
/// padded by a leading space. Falls back to sanitized plain text when the
/// ANSI parser rejects the input.
pub(super) fn parse_output_line(raw: &str) -> Line<'static> {
    let padded = format!(" {raw}");
    let safe = pane_data::sanitize_ansi_for_output(&padded);
    ansi_to_tui::IntoText::into_text(&safe).map_or_else(
        |_| Line::from(Span::raw(pane_data::strip_ansi(&safe))),
        |text| {
            text.lines
                .into_iter()
                .next()
                .unwrap_or_else(|| Line::from(""))
        },
    )
}

/// Title with a follow / selection indicator so the user can tell
/// whether the view is pinned to the streaming tail and how many lines
/// are selected. There is always a selection; the title only calls it
/// out once it is more than the single tail line being followed.
fn output_title(pane: &OutputPane, ctx: &PaneRenderCtx<'_>) -> String {
    let live = ctx.output_presentation.captured_lines();
    let count = pane.selection_line_count(live);
    let lines = if count == 1 { "line" } else { "lines" };
    let focused = pane.focus.is_focused();

    // Vim visual-line mode owns the title with the copy hint.
    if pane.selection().is_visual() {
        return format!(" Output — visual: {count} {lines} (y copy · Esc done) ");
    }
    // A multi-line selection (Shift+arrow / Ctrl-A) owns the title too.
    if count > 1 {
        return format!(" Output — {count} {lines} selected (y copy) ");
    }
    // A single-row selection: parked above the tail, or following it.
    if !pane.is_following() {
        return if focused {
            " Output — scrolled (End follow · y copy) ".to_string()
        } else {
            " Output — scrolled (End to follow) ".to_string()
        };
    }
    let owned_output_visibility = ctx.output_presentation.owned_output();
    let owned_run_running_label = match owned_output_visibility {
        OwnedOutputVisibility::Absent => OwnedRunRunningLabelRef::NotRunning,
        OwnedOutputVisibility::OnScreen(owned) => owned.running_label(),
    };
    match owned_run_running_label {
        OwnedRunRunningLabelRef::Running(name) => {
            return format!(" Running: {name} (Esc to stop) ");
        },
        OwnedRunRunningLabelRef::NotRunning => {},
    }
    let owned_run_output_title = match owned_output_visibility {
        OwnedOutputVisibility::Absent => OwnedRunOutputTitleRef::Unavailable,
        OwnedOutputVisibility::OnScreen(owned) => owned.title(),
    };
    match owned_run_output_title {
        OwnedRunOutputTitleRef::Named(name) => {
            return if focused {
                format!(" Output: {name} (y copy · Esc close) ")
            } else {
                format!(" Output: {name} (Esc close) ")
            };
        },
        OwnedRunOutputTitleRef::Unavailable => {},
    }
    if focused {
        " Output (y copy · Esc close) ".to_string()
    } else {
        " Output (Esc to close) ".to_string()
    }
}
