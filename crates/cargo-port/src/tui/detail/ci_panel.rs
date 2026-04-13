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
use unicode_width::UnicodeWidthStr;

use super::timestamp;
use crate::ci;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::tui::animation::BRAILLE_SPINNER;
use crate::tui::app::App;
use crate::tui::app::CiState;
use crate::tui::constants::ACCENT_COLOR;
use crate::tui::constants::ACTIVE_FOCUS_COLOR;
use crate::tui::constants::CI_EXTRA_ROWS;
use crate::tui::constants::CI_TIMESTAMP_WIDTH;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::interaction;
use crate::tui::interaction::UiSurface;
use crate::tui::render::CiColumn;
use crate::tui::types::Pane;
use crate::tui::types::PaneId;

/// Build the header `Row` for the CI table from the given columns.
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

pub(super) const CI_COMPACT_DURATION_WIDTH: usize = 2;

/// Build one data `Row` for a single `CiRun`.
fn build_ci_data_row(ci_run: &CiRun, cols: &[CiColumn], show_durations: bool) -> Row<'static> {
    let timestamp = timestamp::format_timestamp(&ci_run.created_at);
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
            let style = super::super::render::conclusion_style(Some(job.conclusion));
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

    let total_style = super::super::render::conclusion_style(Some(ci_run.conclusion));
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

    Row::new(cells)
}

/// Build column width constraints for the CI table based on content.
///
/// Duration, timestamp, branch, and glyph columns use `Length` (exact
/// fit-to-content). Commit uses `Fill` to absorb all remaining space.
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
            .unwrap_or("Commit".len())
            .max("Commit".len()),
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

/// Minimum width for a CI duration column: the wider of the header label
/// and the widest duration value across all runs.
fn ci_duration_min_width(ci_runs: &[CiRun], col: CiColumn) -> usize {
    let max_data = ci_runs
        .iter()
        .filter_map(|run| run.jobs.iter().find(|job| col.matches(&job.name)))
        .map(|job| job.duration.trim().len())
        .max()
        .unwrap_or(0);
    col.label().len().max(max_data)
}

pub(super) fn ci_total_width(ci_runs: &[CiRun], show_durations: bool) -> usize {
    if show_durations {
        ci_total_min_width(ci_runs)
    } else {
        CI_COMPACT_DURATION_WIDTH
    }
}

/// Minimum width for the Total duration column.
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

pub(super) fn ci_table_shows_durations(
    ci_runs: &[CiRun],
    cols: &[CiColumn],
    inner_width: u16,
) -> bool {
    ci_table_fixed_width(ci_runs, cols, true) <= usize::from(inner_width)
}

fn selected_ci_state(app: &App) -> Option<&CiState> { app.selected_ci_state() }

fn ci_panel_title(
    local: usize,
    is_fetching: bool,
    fetch_count: u32,
    elapsed: std::time::Duration,
    focused_pos: Option<usize>,
    mode_label: Option<&str>,
) -> String {
    let suffix = mode_label.map_or(String::new(), |label| format!(" [{label}]"));
    if is_fetching {
        let spinner = BRAILLE_SPINNER.frame_at(elapsed);
        format!(" CI Runs{suffix} {spinner} fetching {fetch_count} more… ")
    } else if let Some(pos) = focused_pos
        && pos < local
    {
        let indicator = crate::tui::types::scroll_indicator(pos, local);
        format!(" CI Runs{suffix} ({indicator}) ")
    } else {
        format!(" CI Runs{suffix} ")
    }
}

fn build_fetch_row(
    widths_len: usize,
    is_fetching: bool,
    is_exhausted: bool,
    fetch_count: u32,
    elapsed: std::time::Duration,
) -> Row<'static> {
    let fetch_label = if is_fetching {
        let spinner = BRAILLE_SPINNER.frame_at(elapsed);
        format!(" {spinner} fetching {fetch_count} more…")
    } else if is_exhausted {
        " ↓ fetch new runs".to_string()
    } else {
        " ↓ fetch more runs".to_string()
    };
    let fetch_style = Style::default().fg(ACCENT_COLOR);
    let mut fetch_cells: Vec<Cell> = vec![Cell::from(fetch_label).style(fetch_style)];
    for _ in 1..widths_len {
        fetch_cells.push(Cell::from(""));
    }
    Row::new(fetch_cells)
}

pub fn render_ci_panel(
    frame: &mut Frame,
    app: &mut App,
    ci_runs: &[CiRun],
    area: ratatui::layout::Rect,
) {
    let ci_focused = app.is_focused(PaneId::CiRuns);

    let local = ci_runs.len();
    let ci_state = selected_ci_state(app);
    let is_fetching = ci_state.is_some_and(CiState::is_fetching);
    let is_exhausted = ci_state.is_some_and(CiState::is_exhausted);
    let fetch_count = ci_state.map_or(0, CiState::fetch_count);
    let elapsed = app.animation_elapsed();
    let focused_pos = if ci_focused {
        Some(app.ci_pane().pos())
    } else {
        None
    };
    let mode_label = app.selected_project_path().and_then(|path| {
        app.ci_toggle_available_for(path)
            .then(|| app.ci_display_mode_label_for(path))
    });
    let title = ci_panel_title(
        local,
        is_fetching,
        fetch_count,
        elapsed,
        focused_pos,
        mode_label,
    );

    let ci_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(TITLE_COLOR)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(if ci_focused {
            Style::default().fg(ACTIVE_FOCUS_COLOR)
        } else {
            Style::default()
        });

    let inner = ci_block.inner(area);
    app.ci_pane_mut().set_len(ci_runs.len() + CI_EXTRA_ROWS);
    app.ci_pane_mut().set_content_area(inner);

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
            ci_runs
                .iter()
                .any(|run| run.jobs.iter().any(|job| col.matches(&job.name)))
        })
        .collect();
    let show_durations = ci_table_shows_durations(ci_runs, &cols, inner.width);

    let header = build_ci_header_row(&cols);

    let mut rows: Vec<Row> = ci_runs
        .iter()
        .map(|ci_run| build_ci_data_row(ci_run, &cols, show_durations))
        .collect();

    let widths = build_ci_widths(ci_runs, &cols, show_durations);
    rows.push(build_fetch_row(
        widths.len(),
        is_fetching,
        is_exhausted,
        fetch_count,
        elapsed,
    ));

    let highlight_style = Pane::selection_style(app.pane_focus_state(PaneId::CiRuns));

    let table = Table::new(rows, widths)
        .header(header)
        .block(ci_block)
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let mut table_state = TableState::default().with_selected(Some(app.ci_pane().pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.ci_pane_mut().set_scroll_offset(table_state.offset());
    register_ci_row_hitboxes(app, ci_runs.len(), inner, table_state.offset());
}

fn register_ci_row_hitboxes(
    app: &mut App,
    run_count: usize,
    inner: ratatui::layout::Rect,
    visible_start: usize,
) {
    let visible_height = usize::from(inner.height.saturating_sub(1));
    let visible_end = run_count.min(visible_start.saturating_add(visible_height));

    for (screen_row, row_index) in (visible_start..visible_end).enumerate() {
        let row_y = inner
            .y
            .saturating_add(1)
            .saturating_add(u16::try_from(screen_row).unwrap_or(u16::MAX));
        interaction::register_pane_row_hitbox(
            app,
            ratatui::layout::Rect::new(inner.x, row_y, inner.width, 1),
            PaneId::CiRuns,
            row_index,
            UiSurface::Content,
        );
    }
}
