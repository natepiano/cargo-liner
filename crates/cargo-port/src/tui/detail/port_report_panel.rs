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

fn format_port_report_when(timestamp: &str) -> String {
    DateTime::parse_from_rfc3339(timestamp).map_or_else(
        |_| timestamp.to_string(),
        |ts: DateTime<FixedOffset>| ts.with_timezone(&Local).format("%H:%M:%S").to_string(),
    )
}

fn format_port_report_duration(run: &PortReportRun) -> String {
    let duration_ms = run.duration_ms.or_else(|| {
        if matches!(run.status, PortReportRunStatus::Running) {
            DateTime::parse_from_rfc3339(&run.started_at)
                .ok()
                .and_then(|started| {
                    u64::try_from((Local::now() - started.with_timezone(&Local)).num_milliseconds())
                        .ok()
                })
        } else {
            None
        }
    });
    let Some(duration_ms) = duration_ms else {
        return "—".to_string();
    };
    let total_seconds = duration_ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}:{seconds:02}")
}

pub(super) fn format_port_report_commands(run: &PortReportRun) -> String {
    let total = run.commands.len();
    if total == 0 {
        return "-".to_string();
    }

    let passed = run
        .commands
        .iter()
        .filter(|command| matches!(command.status, PortReportCommandStatus::Passed))
        .count();
    let failed = run
        .commands
        .iter()
        .filter(|command| matches!(command.status, PortReportCommandStatus::Failed))
        .count();
    let pending = run
        .commands
        .iter()
        .filter(|command| matches!(command.status, PortReportCommandStatus::Pending))
        .count();

    match run.status {
        PortReportRunStatus::Running => format!("{passed}/{total} running"),
        PortReportRunStatus::Passed => format!("{total}/{total}"),
        PortReportRunStatus::Failed if failed > 0 => format!("{failed}/{total} failed"),
        PortReportRunStatus::Failed => format!("{}/{}", total.saturating_sub(pending), total),
    }
}

pub(super) fn first_failed_command(run: &PortReportRun) -> Option<&str> {
    run.commands
        .iter()
        .find(|command| matches!(command.status, PortReportCommandStatus::Failed))
        .map(|command| command.name.as_str())
}

fn format_port_report_failed(run: &PortReportRun) -> String {
    first_failed_command(run).unwrap_or("-").to_string()
}

pub(super) fn format_port_report_exit(run: &PortReportRun) -> String {
    run.commands
        .iter()
        .find(|command| matches!(command.status, PortReportCommandStatus::Failed))
        .and_then(|command| command.exit_code)
        .map_or_else(|| "-".to_string(), |code| code.to_string())
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
                Cell::from(format_port_report_when(&run.started_at)),
                Cell::from(run.status.label()),
                Cell::from(format_port_report_duration(run)),
                Cell::from(format_port_report_commands(run)),
                Cell::from(format_port_report_failed(run)),
                Cell::from(format_port_report_exit(run)),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(6),
        ],
    )
    .header(
        Row::new(vec!["When", "Result", "Dur", "Cmds", "Failed", "Exit"]).style(
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
