//! Output pane render body.
//!
//! Entry: `OutputPane::render` in `pane_impls.rs` calls
//! `render_output_pane_body`. The body reads in-flight example
//! state from `PaneRenderCtx::inflight` and the pane's own cursor /
//! selection / follow state from `OutputPane`.

use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::active_focus_color;
use tui_pane::finder_match_bg;
use tui_pane::label_color;

use super::pane_impls::OutputPane;
use super::pane_impls::OutputSelection;
use crate::tui::pane::PaneRenderCtx;

pub fn render_output_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut OutputPane,
    ctx: &PaneRenderCtx<'_>,
) {
    // While a selection is active, render and yank read the frozen
    // snapshot so streaming output can't drift the highlighted range;
    // otherwise render the live buffer. Cloning the `Rc` only bumps the
    // refcount and releases the `&pane` borrow before `sync_viewport`.
    let live = ctx.inflight.example_output();
    let snapshot: Option<Rc<[String]>> = match pane.selection() {
        OutputSelection::Active { snapshot, .. } => Some(Rc::clone(snapshot)),
        OutputSelection::Inactive => None,
    };
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
    let cursor = pane.viewport.pos();
    let selected_range = pane.selected_range();
    let focused = pane.focus.is_focused;
    let inner_width = usize::from(inner.width);

    let block = tui_pane::default_pane_chrome()
        .with_inactive_border(Style::default().fg(label_color()))
        .block(output_title(pane, ctx), focused);

    // The selected range takes the finder match background; the bare
    // cursor row (focused, not yet selecting) takes the same active-row
    // background every other pane uses, so navigating before pressing
    // `V` has a visible affordance and the two states read distinctly.
    let lines: Vec<Line> = source
        .iter()
        .enumerate()
        .map(|(row, raw)| {
            let parsed = parse_output_line(raw);
            if selected_range.is_some_and(|(lo, hi)| row >= lo && row <= hi) {
                fill_row(parsed, inner_width, finder_match_bg())
            } else if focused && selected_range.is_none() && row == cursor {
                fill_row(parsed, inner_width, active_focus_color())
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
/// padded by a leading space. Falls back to the raw text when the ANSI
/// parser rejects the input.
fn parse_output_line(raw: &str) -> Line<'static> {
    let padded = format!(" {raw}");
    ansi_to_tui::IntoText::into_text(&padded).map_or_else(
        |_| Line::from(Span::raw(padded.clone())),
        |text| {
            text.lines
                .into_iter()
                .next()
                .unwrap_or_else(|| Line::from(""))
        },
    )
}

/// Title with a follow / frozen / selecting indicator so the user can
/// tell whether the view is pinned to the streaming tail.
fn output_title(pane: &OutputPane, ctx: &PaneRenderCtx<'_>) -> String {
    if let Some(name) = ctx.inflight.example_running() {
        return format!(" Running: {name} ");
    }
    let focused = pane.focus.is_focused;
    match pane.selection() {
        OutputSelection::Active { .. } => {
            let count = pane.selection_line_count();
            let lines = if count == 1 { "line" } else { "lines" };
            format!(" Output — {count} {lines} selected (y copy · Esc cancel) ")
        },
        OutputSelection::Inactive if pane.is_following() && focused => {
            " Output (V select · Esc close) ".to_string()
        },
        OutputSelection::Inactive if pane.is_following() => " Output (Esc to close) ".to_string(),
        OutputSelection::Inactive if focused => {
            " Output — scrolled (V select · End follow) ".to_string()
        },
        OutputSelection::Inactive => " Output — scrolled (End to follow) ".to_string(),
    }
}
