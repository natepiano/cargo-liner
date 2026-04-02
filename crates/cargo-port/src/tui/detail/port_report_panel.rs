use chrono::DateTime;
use chrono::FixedOffset;
use chrono::Local;
use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Cell;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use super::super::app::App;
use super::super::types::PaneId;
use crate::port_report::PortReportCommandStatus;
use crate::port_report::PortReportRun;
use crate::port_report::PortReportRunStatus;

fn format_port_report_timestamp(timestamp: &str) -> String {
    DateTime::parse_from_rfc3339(timestamp).map_or_else(
        |_| timestamp.to_string(),
        |ts: DateTime<FixedOffset>| {
            ts.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        },
    )
}

fn format_port_report_finished(run: &PortReportRun) -> String {
    run.finished_at
        .as_deref()
        .map_or_else(|| "—".to_string(), format_port_report_timestamp)
}

fn format_duration_ms(duration_ms: Option<u64>) -> String {
    let Some(duration_ms) = duration_ms else {
        return "—".to_string();
    };
    let total_seconds = duration_ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}:{seconds:02}")
}

pub(super) fn format_port_report_commands(run: &PortReportRun) -> String {
    if run.commands.is_empty() {
        return "-".to_string();
    }

    let names = run
        .commands
        .iter()
        .map(|command| command.name.trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    if names.is_empty() {
        "-".to_string()
    } else {
        names
    }
}

pub(super) fn format_port_report_pending(run: &PortReportRun) -> String {
    run.commands
        .iter()
        .filter(|command| matches!(command.status, PortReportCommandStatus::Pending))
        .count()
        .to_string()
}

pub(super) fn format_port_report_slowest(run: &PortReportRun) -> String {
    run.commands
        .iter()
        .filter_map(|command| {
            command
                .duration_ms
                .map(|duration_ms| (command.name.trim(), duration_ms))
        })
        .max_by_key(|(_, duration_ms)| *duration_ms)
        .map_or_else(
            || "—".to_string(),
            |(name, duration_ms)| format!("{name} {}", format_duration_ms(Some(duration_ms))),
        )
}

pub fn render_port_report_panel(
    frame: &mut Frame,
    app: &mut App,
    runs: &[PortReportRun],
    area: ratatui::layout::Rect,
) {
    let focused = app.is_focused(PaneId::CiRuns);
    let (watching, worker_count) = app.selected_project().map_or((false, 0usize), |project| {
        let watching = app.port_report_is_watchable(project) && app.lint_runtime.is_some();
        (watching, usize::from(watching))
    });
    let title = format!(
        " Port Report (watching {}, workers {}, runs {}) ",
        if watching { "yes" } else { "no" },
        worker_count,
        runs.len()
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });

    let inner = block.inner(area);
    app.port_report_pane.set_len(runs.len());
    app.port_report_pane.set_content_area(inner);

    if runs.is_empty() {
        frame.render_widget(block, area);
        if area.height > 2 {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "No local Port Report runs yet",
                    Style::default().fg(Color::DarkGray),
                )))
                .alignment(ratatui::layout::Alignment::Center),
                inner,
            );
        }
        return;
    }

    let rows: Vec<Row> = runs
        .iter()
        .map(|run| {
            let style = match run.status {
                PortReportRunStatus::Running => Style::default().fg(Color::Cyan),
                PortReportRunStatus::Passed => Style::default().fg(Color::Green),
                PortReportRunStatus::Failed => Style::default().fg(Color::Red),
            };
            Row::new(vec![
                Cell::from(format_port_report_timestamp(&run.started_at)),
                Cell::from(format_port_report_finished(run)),
                Cell::from(run.status.label()),
                Cell::from(format_port_report_commands(run)),
                Cell::from(format_port_report_pending(run)),
                Cell::from(format_port_report_slowest(run)),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Length(8),
            Constraint::Fill(1),
            Constraint::Length(7),
            Constraint::Length(16),
        ],
    )
    .header(
        Row::new(vec![
            "Started", "Finished", "Result", "Cmds", "Pending", "Slowest",
        ])
        .style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(block)
    .column_spacing(1)
    .row_highlight_style(if focused {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default()
    });

    let mut table_state = TableState::default().with_selected(Some(app.port_report_pane.pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.port_report_pane.set_scroll_offset(table_state.offset());
}
