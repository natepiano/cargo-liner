use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use crate::lint::LintRun;
use crate::lint::LintRunStatus;
use crate::tui::LINT_SPINNER;
use crate::tui::app::App;
use crate::tui::constants::ACCENT_COLOR;
use crate::tui::constants::ACTIVE_BORDER_COLOR;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::ERROR_COLOR;
use crate::tui::constants::INACTIVE_BORDER_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::SUCCESS_COLOR;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::interaction;
use crate::tui::interaction::UiSurface::Content;
use crate::tui::types::Pane;
use crate::tui::types::PaneId;

fn lints_panel_title(app: &App, runs: &[LintRun], focused: bool) -> String {
    if runs.is_empty() {
        let is_rust = app
            .selected_project_path()
            .is_some_and(|path| app.is_cargo_active_path(path));
        let msg = if is_rust {
            crate::constants::NO_LINT_RUNS
        } else {
            crate::constants::NO_LINT_RUNS_NOT_RUST
        };
        return format!(" {msg} ");
    }
    if focused {
        let indicator =
            crate::tui::types::scroll_indicator(app.pane_manager().lints.pos(), runs.len());
        return format!(" Lint Runs ({indicator}) ");
    }
    " Lint Runs ".to_string()
}

fn lints_panel_block(title: String, focused: bool, has_runs: bool) -> Block<'static> {
    let title_style = if has_runs {
        Style::default()
            .fg(TITLE_COLOR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(INACTIVE_BORDER_COLOR)
    };
    let border_style = if focused {
        Style::default().fg(ACTIVE_BORDER_COLOR)
    } else if has_runs {
        Style::default()
    } else {
        Style::default().fg(INACTIVE_BORDER_COLOR)
    };
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(title_style)
        .border_style(border_style)
}

/// Build display rows for lint runs, grouped by date.
///
/// Returns `(rows, row_to_run_index)` where `row_to_run_index` maps each
/// table row to its lint run index (date headers map to `None`).
fn col_header_row() -> Row<'static> {
    let style = Style::default()
        .fg(COLUMN_HEADER_COLOR)
        .add_modifier(Modifier::BOLD);
    Row::new(vec![
        Cell::from(" Start"),
        Cell::from("End"),
        Cell::from(Line::from("Duration").alignment(Alignment::Right)),
        Cell::from("Result"),
    ])
    .style(style)
}

fn build_lint_rows(runs: &[LintRun], animation_elapsed: std::time::Duration) -> Vec<Row<'static>> {
    let date_style = Style::default()
        .fg(TITLE_COLOR)
        .add_modifier(Modifier::BOLD);

    let mut rows = Vec::new();
    let mut current_date = String::new();

    for run in runs {
        let date = super::timestamp::format_date(&run.started_at);
        if date != current_date {
            current_date.clone_from(&date);
            rows.push(Row::new(vec![Cell::from(Span::styled(
                format!(" {date}"),
                date_style,
            ))]));
            rows.push(col_header_row());
        }

        let start_time = super::timestamp::format_time(&run.started_at);
        let end_time = run
            .finished_at
            .as_deref()
            .map_or_else(|| "—".to_string(), super::timestamp::format_time);
        let duration = super::timestamp::format_duration(run.duration_ms);

        let (result_cell, row_style) = match run.status {
            LintRunStatus::Running => {
                let spinner = LINT_SPINNER.frame_at(animation_elapsed);
                (
                    Cell::from(Line::from(spinner).alignment(Alignment::Center)),
                    Style::default().fg(ACCENT_COLOR),
                )
            },
            LintRunStatus::Passed => (
                Cell::from(Line::from("passed").alignment(Alignment::Center)),
                Style::default().fg(SUCCESS_COLOR),
            ),
            LintRunStatus::Failed => (
                Cell::from(Line::from("failed").alignment(Alignment::Center)),
                Style::default().fg(ERROR_COLOR),
            ),
        };

        rows.push(
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("  {start_time}"),
                    Style::default().fg(LABEL_COLOR),
                )),
                Cell::from(Span::styled(end_time, Style::default().fg(LABEL_COLOR))),
                Cell::from(
                    Line::from(Span::styled(duration, Style::default().fg(LABEL_COLOR)))
                        .alignment(Alignment::Right),
                ),
                result_cell,
            ])
            .style(row_style),
        );
    }

    rows
}

pub fn render_lints_panel(
    frame: &mut Frame,
    app: &mut App,
    runs: &[LintRun],
    area: ratatui::layout::Rect,
) {
    let focused = app.is_focused(PaneId::Lints);
    let title = lints_panel_title(app, runs, focused);
    let block = lints_panel_block(title, focused, !runs.is_empty());

    let inner = block.inner(area);
    app.pane_manager_mut().lints.set_content_area(inner);

    if runs.is_empty() {
        frame.render_widget(block, area);
        app.pane_manager_mut().lints.set_len(0);
        return;
    }

    let rows = build_lint_rows(runs, app.animation_elapsed());
    app.pane_manager_mut().lints.set_len(rows.len());

    let table = Table::new(
        rows,
        [
            Constraint::Length(10), // start time
            Constraint::Length(8),  // end time
            Constraint::Length(8),  // duration
            Constraint::Length(8),  // result
        ],
    )
    .block(block)
    .column_spacing(1)
    .row_highlight_style(Pane::selection_style(app.pane_focus_state(PaneId::Lints)));

    let mut table_state = TableState::default().with_selected(Some(app.pane_manager().lints.pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.pane_manager_mut()
        .lints
        .set_scroll_offset(table_state.offset());

    let visible_height = usize::from(inner.height.saturating_sub(1));
    let visible_start = table_state.offset();
    let visible_end = app
        .pane_manager()
        .lints
        .len()
        .min(visible_start.saturating_add(visible_height));

    for (screen_row, row_index) in (visible_start..visible_end).enumerate() {
        let row_y = inner
            .y
            .saturating_add(1)
            .saturating_add(u16::try_from(screen_row).unwrap_or(u16::MAX));
        interaction::register_pane_row_hitbox(
            app,
            ratatui::layout::Rect::new(inner.x, row_y, inner.width, 1),
            PaneId::Lints,
            row_index,
            Content,
        );
    }
}
