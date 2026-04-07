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

use crate::lint::LintCommandStatus;
use crate::lint::LintRun;
use crate::lint::LintRunStatus;
use crate::tui::app::App;
use crate::tui::types::PaneId;

fn format_bytes(bytes: u64) -> String {
    const BYTES_PER_KIB: u64 = 1024;
    const BYTES_PER_MIB: u64 = BYTES_PER_KIB * 1024;
    const BYTES_PER_GIB: u64 = BYTES_PER_MIB * 1024;

    if bytes >= BYTES_PER_GIB {
        format_decimal_unit(bytes, BYTES_PER_GIB, "GiB")
    } else if bytes >= BYTES_PER_MIB {
        format_decimal_unit(bytes, BYTES_PER_MIB, "MiB")
    } else if bytes >= BYTES_PER_KIB {
        format_decimal_unit(bytes, BYTES_PER_KIB, "KiB")
    } else {
        format!("{bytes} B")
    }
}

fn format_decimal_unit(bytes: u64, unit_bytes: u64, unit_label: &str) -> String {
    let whole = bytes / unit_bytes;
    let remainder = bytes % unit_bytes;
    let mut tenths =
        (u128::from(remainder) * 10 + u128::from(unit_bytes / 2)) / u128::from(unit_bytes);
    let mut whole = whole;
    if tenths == 10 {
        whole += 1;
        tenths = 0;
    }

    format!("{whole}.{tenths} {unit_label}")
}

fn format_lints_finished(run: &LintRun) -> String {
    run.finished_at
        .as_deref()
        .map_or_else(|| "—".to_string(), super::timestamp::format_timestamp)
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

pub(super) fn format_lints_commands(run: &LintRun) -> String {
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

pub(super) fn format_lints_pending(run: &LintRun) -> String {
    run.commands
        .iter()
        .filter(|command| matches!(command.status, LintCommandStatus::Pending))
        .count()
        .to_string()
}

pub(super) fn format_lints_slowest(run: &LintRun) -> String {
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

fn lints_panel_title(app: &App, runs: &[LintRun], focused: bool) -> String {
    if focused && !runs.is_empty() {
        let indicator = crate::tui::types::scroll_indicator(app.lint_pane.pos(), runs.len());
        return format!(" Lints ({indicator}) ");
    }
    let (watching, worker_count) = app.selected_project_path().map_or((false, 0usize), |path| {
        let watching = app.lint_is_watchable(path) && app.lint_runtime.is_some();
        (watching, usize::from(watching))
    });
    let cache_size = app
        .lint_cache_usage
        .cache_size_bytes
        .map_or_else(|| "unlimited".to_string(), format_bytes);
    format!(
        " Lints (watching {}, workers {}, runs {}, cache {}/{}) ",
        if watching { "yes" } else { "no" },
        worker_count,
        runs.len(),
        format_bytes(app.lint_cache_usage.bytes),
        cache_size,
    )
}

pub fn render_lints_panel(
    frame: &mut Frame,
    app: &mut App,
    runs: &[LintRun],
    area: ratatui::layout::Rect,
) {
    let focused = app.is_focused(PaneId::CiRuns);
    let title = lints_panel_title(app, runs, focused);

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
    app.lint_pane.set_len(runs.len());
    app.lint_pane.set_content_area(inner);

    if runs.is_empty() {
        frame.render_widget(block, area);
        if area.height > 2 {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "No local lint runs yet",
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
                LintRunStatus::Running => Style::default().fg(Color::Cyan),
                LintRunStatus::Passed => Style::default().fg(Color::Green),
                LintRunStatus::Failed => Style::default().fg(Color::Red),
            };
            Row::new(vec![
                Cell::from(super::timestamp::format_timestamp(&run.started_at)),
                Cell::from(format_lints_finished(run)),
                Cell::from(run.status.label()),
                Cell::from(format_lints_commands(run)),
                Cell::from(format_lints_pending(run)),
                Cell::from(format_lints_slowest(run)),
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

    let mut table_state = TableState::default().with_selected(Some(app.lint_pane.pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.lint_pane.set_scroll_offset(table_state.offset());
}
