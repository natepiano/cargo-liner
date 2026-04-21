use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use super::LintsData;
use super::PaneId;
use crate::lint::LintRun;
use crate::lint::LintRunStatus;
use crate::tui::LINT_SPINNER;
use crate::tui::app::App;
use crate::tui::constants::ACCENT_COLOR;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::ERROR_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::SUCCESS_COLOR;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::interaction;
use crate::tui::interaction::UiSurface::Content;
use crate::tui::pane;
use crate::tui::pane::Pane;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneTitleCount;
use crate::tui::render;

fn lints_panel_title(data: &LintsData, focused: bool, cursor: usize) -> String {
    if data.runs.is_empty() {
        let msg = if data.is_rust {
            crate::constants::NO_LINT_RUNS
        } else {
            crate::constants::NO_LINT_RUNS_NOT_RUST
        };
        return format!(" {msg} ");
    }
    pane::pane_title(
        "Lint Runs",
        &PaneTitleCount::Single {
            len:    data.runs.len(),
            cursor: focused.then_some(cursor),
        },
    )
}

fn lints_panel_block(title: String, focused: bool, has_runs: bool) -> Block<'static> {
    if has_runs {
        pane::default_pane_chrome().block(title, focused)
    } else {
        pane::empty_pane_block(title)
    }
}

fn build_lint_rows(
    runs: &[LintRun],
    sizes: &[u64],
    animation_elapsed: std::time::Duration,
    pane: &Pane,
    focus: PaneFocusState,
) -> Vec<Row<'static>> {
    let date_style = Style::default()
        .fg(TITLE_COLOR)
        .add_modifier(Modifier::BOLD);

    let mut rows = Vec::new();
    let mut current_date = String::new();

    for (row_index, run) in runs.iter().enumerate() {
        let date = super::format_date(&run.started_at);
        let date_cell = if date == current_date {
            Cell::from("")
        } else {
            current_date.clone_from(&date);
            Cell::from(Span::styled(date, date_style))
        };

        let start_time = super::format_time(&run.started_at);
        let end_time = run
            .finished_at
            .as_deref()
            .map_or_else(|| "—".to_string(), super::format_time);
        let duration = super::format_duration(run.duration_ms);
        let size = sizes
            .get(row_index)
            .copied()
            .map_or_else(String::new, render::format_bytes);

        let (result_cell, row_style) = match run.status {
            LintRunStatus::Running => {
                let spinner = LINT_SPINNER.frame_at(animation_elapsed);
                (Cell::from(spinner), Style::default().fg(ACCENT_COLOR))
            },
            LintRunStatus::Passed => (Cell::from("passed"), Style::default().fg(SUCCESS_COLOR)),
            LintRunStatus::Failed => (Cell::from("failed"), Style::default().fg(ERROR_COLOR)),
        };

        let selection = pane.selection_state(row_index, focus);
        rows.push(
            Row::new(vec![
                date_cell,
                Cell::from(
                    Line::from(Span::styled(start_time, Style::default().fg(LABEL_COLOR)))
                        .alignment(Alignment::Right),
                ),
                Cell::from(
                    Line::from(Span::styled(end_time, Style::default().fg(LABEL_COLOR)))
                        .alignment(Alignment::Right),
                ),
                Cell::from(
                    Line::from(Span::styled(duration, Style::default().fg(LABEL_COLOR)))
                        .alignment(Alignment::Right),
                ),
                Cell::from(
                    Line::from(Span::styled(size, Style::default().fg(LABEL_COLOR)))
                        .alignment(Alignment::Right),
                ),
                result_cell,
            ])
            .style(selection.patch(row_style)),
        );
    }

    rows
}

pub fn render_lints_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(lints_data) = app.pane_data().lints.clone() else {
        let block = lints_panel_block(" No Lint Runs ".to_string(), false, false);
        frame.render_widget(block, area);
        return;
    };

    let focused = app.is_focused(PaneId::Lints);
    let title = lints_panel_title(
        &lints_data,
        focused,
        app.pane_manager().pane(PaneId::Lints).pos(),
    );
    let block = lints_panel_block(title, focused, !lints_data.runs.is_empty());

    let inner = block.inner(area);
    app.pane_manager_mut()
        .pane_mut(PaneId::Lints)
        .set_content_area(inner);
    app.pane_manager_mut()
        .pane_mut(PaneId::Lints)
        .set_viewport_rows(usize::from(inner.height.saturating_sub(1)));

    if lints_data.runs.is_empty() {
        frame.render_widget(block, area);
        app.pane_manager_mut().pane_mut(PaneId::Lints).set_len(0);
        return;
    }

    let pane = app.pane_manager().pane(PaneId::Lints).clone();
    let focus = app.pane_focus_state(PaneId::Lints);
    let rows = build_lint_rows(
        &lints_data.runs,
        &lints_data.sizes,
        app.animation_elapsed(),
        &pane,
        focus,
    );
    app.pane_manager_mut()
        .pane_mut(PaneId::Lints)
        .set_len(rows.len());

    let col_header_style = Style::default()
        .fg(COLUMN_HEADER_COLOR)
        .add_modifier(Modifier::BOLD);

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(9),
            Constraint::Length(8),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from(""),
            Cell::from(Line::from("Start").alignment(Alignment::Right)),
            Cell::from(Line::from("End").alignment(Alignment::Right)),
            Cell::from(Line::from("Duration").alignment(Alignment::Right)),
            Cell::from(Line::from("Size").alignment(Alignment::Right)),
            Cell::from("Result"),
        ])
        .style(col_header_style),
    )
    .block(block)
    .column_spacing(2)
    .row_highlight_style(Style::default());

    let mut table_state =
        TableState::default().with_selected(Some(app.pane_manager().pane(PaneId::Lints).pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.pane_manager_mut()
        .pane_mut(PaneId::Lints)
        .set_scroll_offset(table_state.offset());
    pane::render_overflow_affordance(frame, area, app.pane_manager().pane(PaneId::Lints));

    let visible_height = usize::from(inner.height.saturating_sub(1));
    let visible_start = table_state.offset();
    let visible_end = app
        .pane_manager()
        .pane(PaneId::Lints)
        .len()
        .min(visible_start.saturating_add(visible_height));

    for (screen_row, row_index) in (visible_start..visible_end).enumerate() {
        let row_y = inner
            .y
            .saturating_add(1)
            .saturating_add(u16::try_from(screen_row).unwrap_or(u16::MAX));
        interaction::register_pane_row_hitbox(
            app,
            Rect::new(inner.x, row_y, inner.width, 1),
            PaneId::Lints,
            row_index,
            Content,
        );
    }
}
