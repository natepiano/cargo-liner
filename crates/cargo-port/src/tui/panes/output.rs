//! Output pane render body.
//!
//! `render_output_panel` is a free function rather than a `Pane`
//! trait impl — Output's data dependencies (`example_running`,
//! `example_output`) live on App-shell rather than
//! `PaneRenderCtx`, so it doesn't fit the trait-dispatch path
//! used by the detail panes.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::spec::PaneId;
use crate::tui::app::App;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::pane;

pub fn render_output_panel(frame: &mut Frame, app: &App, area: Rect) {
    let title = app.example_running().map_or_else(
        || " Output (Esc to close) ".to_string(),
        |n| format!(" Running: {n} "),
    );

    let block = pane::default_pane_chrome()
        .with_inactive_border(Style::default().fg(LABEL_COLOR))
        .block(title, app.is_focused(PaneId::Output));

    let lines: Vec<Line> = app
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
