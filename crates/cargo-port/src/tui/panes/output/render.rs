use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::finder_match_bg;
use tui_pane::label_color;

use super::pane::OutputPane;
use crate::tui::panes::pane_data;
use crate::tui::render_context::PaneRenderCtx;

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
    let live = ctx.inflight.example_output();
    let snapshot: Option<Rc<[String]>> = pane.selection().snapshot().map(Rc::clone);
    let source: &[String] = snapshot.as_deref().unwrap_or(live);

    let visible_rows = usize::from(area.height.saturating_sub(2));
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    pane.sync_viewport(source.len(), visible_rows, inner);

    let scroll_offset = u16::try_from(pane.viewport.scroll_offset()).unwrap_or(u16::MAX);
    let selected_range = pane.selected_range(source);
    let focused = pane.focus.is_focused();
    let inner_width = usize::from(inner.width);

    let block = tui_pane::default_pane_chrome()
        .with_inactive_border(Style::default().fg(label_color()))
        .block(output_title(pane, ctx), focused);

    // There is always a selection — at minimum the single cursor row — so
    // it is drawn in one color (the selection background). A single
    // highlighted row is just a one-line selection; an extended range is
    // the same color, wider.
    let lines: Vec<Line> = source
        .iter()
        .enumerate()
        .map(|(row, raw)| {
            let parsed = parse_output_line(raw);
            if selected_range.is_some_and(|(lo, hi)| row >= lo && row <= hi) {
                fill_row(parsed, inner_width, finder_match_bg())
            } else {
                parsed
            }
        })
        .collect();

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));

    frame.render_widget(paragraph, area);
}

/// Force `bg` onto every span (overriding the per-span backgrounds the
/// ANSI parser sets, while keeping each span's foreground) and pad the
/// line with trailing spaces to `width`, so the highlight covers the
/// full pane row including the colored log text rather than stopping at
/// the timestamp.
fn fill_row(parsed: Line<'static>, width: usize, bg: Color) -> Line<'static> {
    let highlight = Style::default().bg(bg);
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
fn parse_output_line(raw: &str) -> Line<'static> {
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
    let live = ctx.inflight.example_output();
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
    if let Some(name) = ctx.inflight.example_running() {
        return format!(" Running: {name} (Esc to stop) ");
    }
    if let Some(name) = ctx.inflight.example_title() {
        return if focused {
            format!(" Output: {name} (y copy · Esc close) ")
        } else {
            format!(" Output: {name} (Esc close) ")
        };
    }
    if focused {
        " Output (y copy · Esc close) ".to_string()
    } else {
        " Output (Esc to close) ".to_string()
    }
}
