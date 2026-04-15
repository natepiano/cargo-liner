use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use crate::lint::LintCommandStatus;
use crate::lint::LintRun;
use crate::lint::LintRunStatus;
use crate::tui::app::App;
use crate::tui::constants::ACCENT_COLOR;
use crate::tui::constants::ACTIVE_BORDER_COLOR;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::ERROR_COLOR;
use crate::tui::constants::INACTIVE_BORDER_COLOR;
use crate::tui::constants::SUCCESS_COLOR;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::interaction;
use crate::tui::interaction::UiSurface::Content;
use crate::tui::types::Pane;
use crate::tui::types::PaneId;

fn format_lints_finished(run: &LintRun) -> String {
    run.finished_at
        .as_deref()
        .map_or_else(|| "—".to_string(), super::timestamp::format_timestamp)
}

pub(super) fn format_lints_pending(run: &LintRun) -> String {
    run.commands
        .iter()
        .filter(|command| matches!(command.status, LintCommandStatus::Pending))
        .count()
        .to_string()
}

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
    app.pane_manager_mut().lints.set_len(runs.len());
    app.pane_manager_mut().lints.set_content_area(inner);

    if runs.is_empty() {
        frame.render_widget(block, area);
        return;
    }

    let rows: Vec<Row> = runs
        .iter()
        .map(|run| {
            let style = match run.status {
                LintRunStatus::Running => Style::default().fg(ACCENT_COLOR),
                LintRunStatus::Passed => Style::default().fg(SUCCESS_COLOR),
                LintRunStatus::Failed => Style::default().fg(ERROR_COLOR),
            };
            Row::new(vec![
                Cell::from(format!(
                    " {}",
                    super::timestamp::format_timestamp(&run.started_at)
                )),
                Cell::from(format_lints_finished(run)),
                Cell::from(run.status.label()),
                Cell::from(format_lints_pending(run)),
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
            Constraint::Length(7),
        ],
    )
    .header(
        Row::new(vec![" Started", "Finished", "Result", "Pending"]).style(
            Style::default()
                .fg(COLUMN_HEADER_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
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
    let visible_end = runs.len().min(visible_start.saturating_add(visible_height));

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
