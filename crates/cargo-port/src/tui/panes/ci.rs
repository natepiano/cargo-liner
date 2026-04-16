use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use unicode_width::UnicodeWidthStr;

use super::PaneTitleCount;
use super::pane_title;
use crate::ci;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::tui::app::App;
use crate::tui::constants::ACTIVE_BORDER_COLOR;
use crate::tui::constants::CI_TIMESTAMP_WIDTH;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::INACTIVE_BORDER_COLOR;
use crate::tui::constants::INACTIVE_TITLE_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::detail;
use crate::tui::detail::CiData;
use crate::tui::interaction;
use crate::tui::interaction::UiSurface;
use crate::tui::render;
use crate::tui::render::CiColumn;
use crate::tui::types::PaneId;
use crate::tui::types::PaneSelectionState;

fn build_ci_header_row(cols: &[CiColumn]) -> Row<'static> {
    let right_aligned = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(COLUMN_HEADER_COLOR);
    let mut header_cells = vec![
        Cell::from(" Commit").style(right_aligned),
        Cell::from("Branch").style(right_aligned),
        Cell::from("Timestamp").style(right_aligned),
    ];
    for col in cols {
        header_cells.push(
            Cell::from(
                ratatui::text::Line::from(col.label()).alignment(ratatui::layout::Alignment::Right),
            )
            .style(right_aligned),
        );
        header_cells.push(Cell::from(""));
    }
    header_cells.push(
        Cell::from(ratatui::text::Line::from("Total").alignment(ratatui::layout::Alignment::Right))
            .style(right_aligned),
    );
    header_cells.push(Cell::from(""));
    Row::new(header_cells).bottom_margin(0)
}

pub(in super::super) const CI_COMPACT_DURATION_WIDTH: usize = 2;

fn build_ci_data_row(
    ci_run: &CiRun,
    cols: &[CiColumn],
    show_durations: bool,
    selection: PaneSelectionState,
) -> Row<'static> {
    let timestamp = detail::format_timestamp(&ci_run.created_at);
    let total_dur = ci_run
        .wall_clock_secs
        .map_or_else(|| "—".to_string(), ci::format_secs);

    let commit = ci_run.commit_title.as_deref().unwrap_or("");
    let commit_style = Style::default();
    let mut cells = vec![
        Cell::from(format!(" {commit}")).style(commit_style),
        Cell::from(ci_run.branch.clone()),
        Cell::from(timestamp),
    ];

    for col in cols {
        let job = ci_run.jobs.iter().find(|job| col.matches(&job.name));
        if let Some(job) = job {
            let style = render::conclusion_style(Some(job.conclusion));
            cells.push(
                Cell::from(
                    ratatui::text::Line::from(if show_durations {
                        job.duration.trim().to_string()
                    } else {
                        String::new()
                    })
                    .alignment(ratatui::layout::Alignment::Right),
                )
                .style(style),
            );
            cells.push(Cell::from(job.conclusion.icon().to_string()).style(style));
        } else {
            cells.push(
                Cell::from(
                    ratatui::text::Line::from("—").alignment(ratatui::layout::Alignment::Right),
                )
                .style(Style::default().fg(LABEL_COLOR)),
            );
            cells.push(Cell::from(""));
        }
    }

    let total_style = render::conclusion_style(Some(ci_run.conclusion));
    cells.push(
        Cell::from(
            ratatui::text::Line::from(if show_durations {
                total_dur.trim().to_string()
            } else {
                String::new()
            })
            .alignment(ratatui::layout::Alignment::Right),
        )
        .style(total_style),
    );
    cells.push(Cell::from(ci_run.conclusion.icon().to_string()).style(total_style));

    Row::new(cells).style(selection.overlay_style())
}

fn build_ci_widths(ci_runs: &[CiRun], cols: &[CiColumn], show_durations: bool) -> Vec<Constraint> {
    let branch_width = u16::try_from(
        ci_runs
            .iter()
            .map(|run| run.branch.len())
            .max()
            .unwrap_or("Branch".len())
            .max("Branch".len()),
    )
    .unwrap_or(u16::MAX);
    let glyph_width = u16::try_from(
        Conclusion::Success
            .icon()
            .width()
            .max(Conclusion::Failure.icon().width()),
    )
    .unwrap_or(u16::MAX);
    let commit_width = u16::try_from(
        ci_runs
            .iter()
            .map(|run| run.commit_title.as_deref().unwrap_or("").len())
            .max()
            .unwrap_or(" Commit".len())
            .max(" Commit".len()),
    )
    .unwrap_or(u16::MAX);
    let mut widths = vec![
        Constraint::Length(commit_width),
        Constraint::Length(branch_width),
        Constraint::Length(CI_TIMESTAMP_WIDTH),
    ];
    for col in cols {
        let width =
            u16::try_from(ci_duration_width(ci_runs, *col, show_durations)).unwrap_or(u16::MAX);
        widths.push(Constraint::Length(width));
        widths.push(Constraint::Length(glyph_width));
    }
    let total_width = u16::try_from(ci_total_width(ci_runs, show_durations)).unwrap_or(u16::MAX);
    widths.push(Constraint::Length(total_width));
    widths.push(Constraint::Length(glyph_width));
    widths
}

fn ci_duration_width(ci_runs: &[CiRun], col: CiColumn, show_durations: bool) -> usize {
    if show_durations {
        ci_duration_min_width(ci_runs, col)
    } else {
        CI_COMPACT_DURATION_WIDTH
    }
}

fn ci_duration_min_width(ci_runs: &[CiRun], col: CiColumn) -> usize {
    let max_data = ci_runs
        .iter()
        .filter_map(|run| run.jobs.iter().find(|job| col.matches(&job.name)))
        .map(|job| job.duration.trim().len())
        .max()
        .unwrap_or(0);
    col.label().len().max(max_data)
}

pub(in super::super) fn ci_total_width(ci_runs: &[CiRun], show_durations: bool) -> usize {
    if show_durations {
        ci_total_min_width(ci_runs)
    } else {
        CI_COMPACT_DURATION_WIDTH
    }
}

fn ci_total_min_width(ci_runs: &[CiRun]) -> usize {
    let max_data = ci_runs
        .iter()
        .filter_map(|run| run.wall_clock_secs)
        .map(|seconds| ci::format_secs(seconds).trim().len())
        .max()
        .unwrap_or(0);
    "Total".len().max(max_data)
}

fn ci_table_fixed_width(ci_runs: &[CiRun], cols: &[CiColumn], show_durations: bool) -> usize {
    let glyph_width = Conclusion::Success
        .icon()
        .width()
        .max(Conclusion::Failure.icon().width());
    let branch_width = ci_runs
        .iter()
        .map(|run| run.branch.len())
        .max()
        .unwrap_or("Branch".len())
        .max("Branch".len());
    let column_count = 1 + 2 + (cols.len() * 2) + 2;
    let base = branch_width + usize::from(CI_TIMESTAMP_WIDTH);
    let job_columns: usize = cols
        .iter()
        .map(|col| ci_duration_width(ci_runs, *col, show_durations) + glyph_width)
        .sum();
    let total = ci_total_width(ci_runs, show_durations) + glyph_width;
    base + job_columns + total + column_count.saturating_sub(1)
}

pub(in super::super) fn ci_table_shows_durations(
    ci_runs: &[CiRun],
    cols: &[CiColumn],
    inner_width: u16,
) -> bool {
    ci_table_fixed_width(ci_runs, cols, true) <= usize::from(inner_width)
}

fn ci_panel_title(data: &CiData, focused_pos: Option<usize>) -> String {
    let suffix = data
        .mode_label
        .as_deref()
        .map_or(String::new(), |label| format!(" [{label}]"));
    pane_title(
        &format!("CI Runs{suffix}"),
        &PaneTitleCount::Single {
            len:    data.runs.len(),
            cursor: focused_pos,
        },
    )
}

fn empty_ci_title(data: &CiData) -> String { data.empty_state.title() }

pub fn render_ci_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(ci_data) = app.pane_manager().ci_data.clone() else {
        render_empty_ci_block(frame, " No CI Runs ", area);
        return;
    };

    if !ci_data.has_runs() {
        app.pane_manager_mut().pane_mut(PaneId::CiRuns).set_len(0);
        app.pane_manager_mut()
            .pane_mut(PaneId::CiRuns)
            .set_content_area(Rect::ZERO);
        render_empty_ci_block(frame, &empty_ci_title(&ci_data), area);
        return;
    }

    let ci_focused = app.is_focused(PaneId::CiRuns);
    let ci_focus = app.pane_focus_state(PaneId::CiRuns);
    let focused_pos = ci_focused.then(|| app.pane_manager().pane(PaneId::CiRuns).pos());
    let title = ci_panel_title(&ci_data, focused_pos);

    let ci_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(if ci_focused {
                    TITLE_COLOR
                } else {
                    INACTIVE_TITLE_COLOR
                })
                .add_modifier(Modifier::BOLD),
        )
        .border_style(if ci_focused {
            Style::default().fg(ACTIVE_BORDER_COLOR)
        } else {
            Style::default()
        });

    let inner = ci_block.inner(area);
    app.pane_manager_mut()
        .pane_mut(PaneId::CiRuns)
        .set_len(ci_data.runs.len());
    app.pane_manager_mut()
        .pane_mut(PaneId::CiRuns)
        .set_content_area(inner);

    let all_columns = [
        CiColumn::Fmt,
        CiColumn::Taplo,
        CiColumn::Clippy,
        CiColumn::Mend,
        CiColumn::Build,
        CiColumn::Test,
        CiColumn::Bench,
    ];
    let cols: Vec<CiColumn> = all_columns
        .into_iter()
        .filter(|col| {
            ci_data
                .runs
                .iter()
                .any(|run| run.jobs.iter().any(|job| col.matches(&job.name)))
        })
        .collect();
    let show_durations = ci_table_shows_durations(&ci_data.runs, &cols, inner.width);

    let header = build_ci_header_row(&cols);

    let rows: Vec<Row> = ci_data
        .runs
        .iter()
        .enumerate()
        .map(|(row_index, ci_run)| {
            build_ci_data_row(
                ci_run,
                &cols,
                show_durations,
                app.pane_manager()
                    .pane(PaneId::CiRuns)
                    .selection_state(row_index, ci_focus),
            )
        })
        .collect();

    let widths = build_ci_widths(&ci_data.runs, &cols, show_durations);

    let table = Table::new(rows, widths)
        .header(header)
        .block(ci_block)
        .column_spacing(1)
        .row_highlight_style(Style::default());

    let mut table_state =
        TableState::default().with_selected(Some(app.pane_manager().pane(PaneId::CiRuns).pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.pane_manager_mut()
        .pane_mut(PaneId::CiRuns)
        .set_scroll_offset(table_state.offset());
    register_ci_row_hitboxes(app, ci_data.runs.len(), inner, table_state.offset());
}

fn render_empty_ci_block(frame: &mut Frame, title: &str, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(Style::default().fg(INACTIVE_BORDER_COLOR))
        .border_style(Style::default().fg(INACTIVE_BORDER_COLOR));
    frame.render_widget(block, area);
}

fn register_ci_row_hitboxes(app: &mut App, run_count: usize, inner: Rect, visible_start: usize) {
    let visible_height = usize::from(inner.height.saturating_sub(1));
    let visible_end = run_count.min(visible_start.saturating_add(visible_height));

    for (screen_row, row_index) in (visible_start..visible_end).enumerate() {
        let row_y = inner
            .y
            .saturating_add(1)
            .saturating_add(u16::try_from(screen_row).unwrap_or(u16::MAX));
        interaction::register_pane_row_hitbox(
            app,
            Rect::new(inner.x, row_y, inner.width, 1),
            PaneId::CiRuns,
            row_index,
            UiSurface::Content,
        );
    }
}
