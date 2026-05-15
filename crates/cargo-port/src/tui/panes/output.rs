//! Output pane render body.
//!
//! Entry: `OutputPane::render` in `pane_impls.rs` calls
//! `render_output_pane_body`. The body reads in-flight example
//! state from `PaneRenderCtx::inflight`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::LABEL_COLOR;

use super::pane_impls::OutputPane;
use crate::tui::pane;
use crate::tui::pane::PaneRenderCtx;

pub fn render_output_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &OutputPane,
    ctx: &PaneRenderCtx<'_>,
) {
    let title = ctx.inflight.example_running().map_or_else(
        || " Output (Esc to close) ".to_string(),
        |n| format!(" Running: {n} "),
    );

    let block = pane::default_pane_chrome()
        .with_inactive_border(Style::default().fg(LABEL_COLOR))
        .block(title, pane.focus.is_focused);

    let lines: Vec<Line> = ctx
        .inflight
        .example_output()
        .iter()
        .map(|l| {
            let padded = format!(" {l}");
            ansi_to_tui::IntoText::into_text(&padded).map_or_else(
                |_| Line::from(Span::raw(padded.clone())),
                |text| {
                    text.lines
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| Line::from(""))
                },
            )
        })
        .collect();

    let inner_height = area.height.saturating_sub(2);
    let total_lines = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let scroll_offset = total_lines.saturating_sub(inner_height);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));

    frame.render_widget(paragraph, area);
}
