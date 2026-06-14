use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use tui_pane::PaneSelectionState;
use tui_pane::PaneTitleCount;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use unicode_width::UnicodeWidthStr;

use super::CiData;
use crate::ci;
use crate::ci::CiJob;
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::tui::columns::ColumnSpec;
use crate::tui::columns::ColumnWidths;
use crate::tui::constants::CI_TIMESTAMP_WIDTH;
use crate::tui::panes::constants::CI_BRANCH_LONG_MIN_WIDTH;
use crate::tui::panes::constants::CI_BRANCH_MIN_WIDTH;
use crate::tui::panes::constants::CI_COMMIT_LONG_MIN_WIDTH;
use crate::tui::panes::constants::CI_COMMIT_MIN_WIDTH;
use crate::tui::panes::constants::CI_JOB_LABEL_MAX_WIDTH;
use crate::tui::panes::constants::CI_JOB_LABEL_MIN_WIDTH;
use crate::tui::panes::constants::CI_STATUS_GAP_WIDTH;
use crate::tui::panes::constants::OTHER_JOBS_HEADER;
use crate::tui::panes::pane_data;
use crate::tui::render;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::state::Ci;
use crate::tui::theme_roles;

#[derive(Clone, Debug, PartialEq, Eq)]
enum CiDisplayColumn {
    Job(String),
    Other { jobs: Vec<String> },
}

impl CiDisplayColumn {
    fn header_min_label(&self) -> String {
        match self {
            Self::Job(name) => ci_header_label_for_width(name, CI_JOB_LABEL_MIN_WIDTH),
            Self::Other { .. } => OTHER_JOBS_HEADER.to_string(),
        }
    }

    fn header_label_for_width(&self, width: usize) -> String {
        match self {
            Self::Job(name) => ci_header_label_for_width(name, width),
            Self::Other { .. } => ci_header_label_for_width(OTHER_JOBS_HEADER, width),
        }
    }

    fn header_target_width(&self) -> usize {
        match self {
            Self::Job(name) => name.width().min(CI_JOB_LABEL_MAX_WIDTH),
            Self::Other { .. } => OTHER_JOBS_HEADER.width(),
        }
    }
}

#[derive(Clone, Debug)]
struct CiJobColumn {
    name:          String,
    first_seen:    usize,
    max_runtime_s: u64,
}

/// Job columns in first-seen order across `runs` (newest first), deduped by
/// exact job name and annotated with observed runtime for display planning.
fn infer_ci_columns(runs: &[CiRun]) -> Vec<CiJobColumn> {
    let mut cols: Vec<CiJobColumn> = Vec::new();
    for run in runs {
        for job in &run.jobs {
            let runtime = job.duration_secs.unwrap_or(0);
            if let Some(existing) = cols.iter_mut().find(|col| col.name == job.name) {
                existing.max_runtime_s = existing.max_runtime_s.max(runtime);
            } else {
                cols.push(CiJobColumn {
                    name:          job.name.clone(),
                    first_seen:    cols.len(),
                    max_runtime_s: runtime,
                });
            }
        }
    }
    cols
}

fn ci_job_display_columns(cols: &[CiJobColumn]) -> Vec<CiDisplayColumn> {
    cols.iter()
        .map(|col| CiDisplayColumn::Job(col.name.clone()))
        .collect()
}

fn ci_display_columns_for_width(runs: &[CiRun], inner_width: u16) -> Vec<CiDisplayColumn> {
    let cols = infer_ci_columns(runs);
    let all_columns = ci_job_display_columns(&cols);
    if ci_table_fixed_width(runs, &all_columns, true) <= usize::from(inner_width) {
        return all_columns;
    }

    let mut ranked = cols.clone();
    ranked.sort_by(|left, right| {
        right
            .max_runtime_s
            .cmp(&left.max_runtime_s)
            .then_with(|| left.first_seen.cmp(&right.first_seen))
    });

    let mut selected = HashSet::new();
    for candidate in ranked {
        selected.insert(candidate.name.clone());
        let display_columns = ci_grouped_display_columns(&cols, &selected);
        if ci_table_fixed_width(runs, &display_columns, true) > usize::from(inner_width) {
            selected.remove(&candidate.name);
        }
    }

    ci_grouped_display_columns(&cols, &selected)
}

fn ci_grouped_display_columns(
    cols: &[CiJobColumn],
    selected: &HashSet<String>,
) -> Vec<CiDisplayColumn> {
    let mut display = Vec::new();
    let mut other = Vec::new();
    for col in cols {
        if selected.contains(&col.name) {
            display.push(CiDisplayColumn::Job(col.name.clone()));
        } else {
            other.push(col.name.clone());
        }
    }
    if !other.is_empty() {
        display.push(CiDisplayColumn::Other { jobs: other });
    }
    display
}

fn build_ci_header_row(cols: &[CiDisplayColumn], widths: &[Constraint]) -> Row<'static> {
    let right_aligned = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(theme_roles::column_header_color());
    let mut header_cells = vec![
        Cell::from(" Commit").style(right_aligned),
        Cell::from("Branch").style(right_aligned),
        Cell::from("Timestamp").style(right_aligned),
    ];
    for (i, col) in cols.iter().enumerate() {
        let width = constraint_width(widths.get(3 + i)).unwrap_or(CI_JOB_LABEL_MIN_WIDTH);
        header_cells.push(
            Cell::from(Line::from(col.header_label_for_width(width)).alignment(Alignment::Right))
                .style(right_aligned),
        );
    }
    header_cells
        .push(Cell::from(Line::from("Total").alignment(Alignment::Right)).style(right_aligned));
    Row::new(header_cells).bottom_margin(0)
}

fn build_ci_data_row(
    ci_run: &CiRun,
    cols: &[CiDisplayColumn],
    show_durations: bool,
    commit_width: usize,
    branch_width: usize,
    selection: PaneSelectionState,
) -> Row<'static> {
    let timestamp = pane_data::format_timestamp(&ci_run.created_at);
    let total_dur = ci_run
        .wall_clock_secs
        .map_or_else(|| "—".to_string(), ci::format_secs);

    let commit = ci_run.commit_title.as_deref().unwrap_or("");
    let commit_style = Style::default();
    let mut cells = vec![
        Cell::from(ci_header_label_for_width(
            &format!(" {commit}"),
            commit_width,
        ))
        .style(commit_style),
        Cell::from(ci_header_label_for_width(&ci_run.branch, branch_width)),
        Cell::from(timestamp),
    ];

    for col in cols {
        let data = ci_column_data(ci_run, col);
        if let Some(data) = data {
            let style = render::conclusion_style(Some(data.status));
            cells.push(ci_metric_cell(
                data.duration.trim(),
                data.status,
                show_durations,
                style,
            ));
        } else {
            cells.push(ci_missing_metric_cell(show_durations));
        }
    }

    let total_style = render::conclusion_style(Some(ci_run.ci_status));
    cells.push(ci_metric_cell(
        total_dur.trim(),
        ci_run.ci_status,
        show_durations,
        total_style,
    ));

    Row::new(cells).style(selection.overlay_style())
}

fn ci_metric_cell(
    duration: &str,
    status: CiStatus,
    show_durations: bool,
    style: Style,
) -> Cell<'static> {
    let mut spans = Vec::new();
    if show_durations {
        spans.push(Span::styled(duration.to_string(), style));
        spans.push(Span::styled(" ".repeat(CI_STATUS_GAP_WIDTH), style));
    }
    spans.push(Span::styled(status.icon().to_string(), style));
    Cell::from(Line::from(spans).alignment(Alignment::Right))
}

fn ci_missing_metric_cell(show_durations: bool) -> Cell<'static> {
    let style = Style::default().fg(label_color());
    let label = if show_durations {
        format!(
            "—{}{}",
            " ".repeat(CI_STATUS_GAP_WIDTH),
            " ".repeat(ci_status_glyph_width())
        )
    } else {
        "—".to_string()
    };
    Cell::from(Line::from(Span::styled(label, style)).alignment(Alignment::Right))
}

struct CiColumnData {
    duration: String,
    status:   CiStatus,
}

fn ci_column_data(run: &CiRun, col: &CiDisplayColumn) -> Option<CiColumnData> {
    match col {
        CiDisplayColumn::Job(name) => {
            let job = run.jobs.iter().find(|job| &job.name == name)?;
            Some(CiColumnData {
                duration: job.duration.clone(),
                status:   job.ci_status,
            })
        },
        CiDisplayColumn::Other { jobs } => ci_other_column_data(run, jobs),
    }
}

fn ci_other_column_data(run: &CiRun, jobs: &[String]) -> Option<CiColumnData> {
    let other_jobs: Vec<&CiJob> = run
        .jobs
        .iter()
        .filter(|job| jobs.iter().any(|name| name == &job.name))
        .collect();
    if other_jobs.is_empty() {
        return None;
    }

    let duration_secs = other_jobs
        .iter()
        .try_fold(0_u64, |total, job| Some(total + job.duration_secs?));
    let duration = duration_secs.map_or_else(|| "—".to_string(), ci::format_secs);

    Some(CiColumnData {
        duration,
        status: ci_jobs_status(&other_jobs),
    })
}

fn ci_jobs_status(jobs: &[&CiJob]) -> CiStatus {
    if jobs.iter().any(|job| job.ci_status.is_failure()) {
        return CiStatus::Failed;
    }
    if jobs.iter().any(|job| job.ci_status == CiStatus::Cancelled) {
        return CiStatus::Cancelled;
    }
    if jobs.iter().any(|job| job.ci_status.is_success()) {
        return CiStatus::Passed;
    }
    CiStatus::Skipped
}

fn build_ci_widths(
    ci_runs: &[CiRun],
    cols: &[CiDisplayColumn],
    show_durations: bool,
    inner_width: u16,
) -> Vec<Constraint> {
    // Column layout: Commit, Branch, Timestamp, then one metric column per
    // job, then Total. A metric cell contains the duration plus its status
    // glyph, so headers can use the whole duration and status area.
    let (commit_width, branch_width) =
        ci_description_widths(ci_runs, cols, show_durations, inner_width);
    let mut specs = vec![
        ColumnSpec {
            min: label_width(" Commit"),
            max: Some(commit_width),
        },
        ColumnSpec {
            min: label_width("Branch"),
            max: Some(branch_width),
        },
        ColumnSpec::fixed(CI_TIMESTAMP_WIDTH),
    ];
    for col in cols {
        let label_min = label_width(&col.header_min_label());
        if show_durations {
            specs.push(ColumnSpec::fit(label_min.max(ci_status_glyph_width_u16())));
        } else {
            specs.push(ColumnSpec::fixed(
                label_min.max(ci_status_glyph_width_u16()),
            ));
        }
    }
    let total_label = label_width("Total");
    if show_durations {
        specs.push(ColumnSpec::fit(
            total_label.max(ci_status_glyph_width_u16()),
        ));
    } else {
        specs.push(ColumnSpec::fixed(
            total_label.max(ci_status_glyph_width_u16()),
        ));
    }

    let spec_len = specs.len();
    let mut widths = ColumnWidths::new(specs);

    for run in ci_runs {
        widths.observe_cell_usize(0, run.commit_title.as_deref().unwrap_or("").len() + 1);
        widths.observe_cell_usize(1, run.branch.len());
    }
    if show_durations {
        for (i, col) in cols.iter().enumerate() {
            let col_idx = 3 + i;
            for run in ci_runs {
                if let Some(data) = ci_column_data(run, col) {
                    widths.observe_cell_usize(
                        col_idx,
                        ci_metric_content_width(data.duration.trim().width()),
                    );
                }
            }
        }
        let total_idx = 3 + cols.len();
        for run in ci_runs {
            if let Some(seconds) = run.wall_clock_secs {
                widths.observe_cell_usize(
                    total_idx,
                    ci_metric_content_width(ci::format_secs(seconds).trim().width()),
                );
            }
        }
    }

    let mut resolved: Vec<u16> = (0..spec_len).map(|i| widths.get(i)).collect();
    widen_ci_header_columns(&mut resolved, cols, inner_width);
    resolved.into_iter().map(Constraint::Length).collect()
}

/// Display width of a header label, clamped to `u16`.
fn label_width(label: &str) -> u16 { u16::try_from(label.width()).unwrap_or(u16::MAX) }

fn ci_header_label_for_width(label: &str, width: usize) -> String {
    render::truncate_with_ellipsis(label, width, "\u{2026}")
}

fn widen_ci_header_columns(widths: &mut [u16], cols: &[CiDisplayColumn], inner_width: u16) {
    let Some(mut slack) = ci_width_slack(widths, inner_width) else {
        return;
    };

    for (i, col) in cols.iter().enumerate() {
        if slack == 0 {
            break;
        }
        let idx = 3 + i;
        let target = col.header_target_width();
        let current = usize::from(widths[idx]);
        let grow_by = target.saturating_sub(current).min(slack);
        widths[idx] = widths[idx].saturating_add(u16::try_from(grow_by).unwrap_or(u16::MAX));
        slack -= grow_by;
    }
}

fn ci_width_slack(widths: &[u16], inner_width: u16) -> Option<usize> {
    let used = widths
        .iter()
        .map(|width| usize::from(*width))
        .sum::<usize>()
        + widths.len().saturating_sub(1);
    usize::from(inner_width).checked_sub(used)
}

fn constraint_width(constraint: Option<&Constraint>) -> Option<usize> {
    match constraint {
        Some(Constraint::Length(width)) => Some(usize::from(*width)),
        _ => None,
    }
}

fn ci_duration_width(ci_runs: &[CiRun], col: &CiDisplayColumn, show_durations: bool) -> usize {
    if show_durations {
        ci_metric_width(
            ci_duration_data_width(ci_runs, col),
            col.header_min_label().width(),
        )
    } else {
        col.header_min_label().width().max(ci_status_glyph_width())
    }
}

fn ci_duration_data_width(ci_runs: &[CiRun], col: &CiDisplayColumn) -> usize {
    ci_runs
        .iter()
        .filter_map(|run| ci_column_data(run, col))
        .map(|data| data.duration.trim().width())
        .max()
        .unwrap_or(0)
}

fn ci_metric_width(duration_width: usize, header_width: usize) -> usize {
    duration_width.max(header_width) + CI_STATUS_GAP_WIDTH + ci_status_glyph_width()
}

fn ci_metric_content_width(duration_width: usize) -> usize {
    duration_width + CI_STATUS_GAP_WIDTH + ci_status_glyph_width()
}

fn ci_status_glyph_width_u16() -> u16 { u16::try_from(ci_status_glyph_width()).unwrap_or(u16::MAX) }

fn ci_status_glyph_width() -> usize {
    [
        CiStatus::Passed,
        CiStatus::Failed,
        CiStatus::Cancelled,
        CiStatus::Skipped,
    ]
    .into_iter()
    .map(|status| status.icon().width())
    .max()
    .unwrap_or(0)
}

fn ci_description_widths(
    ci_runs: &[CiRun],
    cols: &[CiDisplayColumn],
    show_durations: bool,
    inner_width: u16,
) -> (u16, u16) {
    let commit_min = ci_commit_min_width(ci_runs);
    let branch_min = ci_branch_min_width(ci_runs);
    let min_total = commit_min + branch_min;
    let available = usize::from(inner_width)
        .saturating_sub(ci_non_description_width(ci_runs, cols, show_durations))
        .max(min_total);

    let commit_observed = ci_runs
        .iter()
        .map(|run| run.commit_title.as_deref().unwrap_or("").len() + 1)
        .max()
        .unwrap_or(CI_COMMIT_MIN_WIDTH)
        .max(CI_COMMIT_MIN_WIDTH);
    let branch_observed = ci_runs
        .iter()
        .map(|run| run.branch.len())
        .max()
        .unwrap_or(CI_BRANCH_MIN_WIDTH)
        .max(CI_BRANCH_MIN_WIDTH);

    let branch_ceiling = (available / 3).max(branch_min);
    let branch = branch_observed
        .min(branch_ceiling)
        .clamp(branch_min, available.saturating_sub(commit_min));
    let commit = commit_observed
        .min(available.saturating_sub(branch))
        .max(commit_min);

    (
        u16::try_from(commit).unwrap_or(u16::MAX),
        u16::try_from(branch).unwrap_or(u16::MAX),
    )
}

fn ci_total_width(ci_runs: &[CiRun], show_durations: bool) -> usize {
    if show_durations {
        ci_metric_width(ci_total_duration_width(ci_runs), "Total".width())
    } else {
        "Total".width().max(ci_status_glyph_width())
    }
}

fn ci_total_duration_width(ci_runs: &[CiRun]) -> usize {
    ci_runs
        .iter()
        .filter_map(|run| run.wall_clock_secs)
        .map(|seconds| ci::format_secs(seconds).trim().width())
        .max()
        .unwrap_or(0)
}

fn ci_table_fixed_width(
    ci_runs: &[CiRun],
    cols: &[CiDisplayColumn],
    show_durations: bool,
) -> usize {
    ci_description_min_width(ci_runs) + ci_non_description_width(ci_runs, cols, show_durations)
}

fn ci_description_min_width(ci_runs: &[CiRun]) -> usize {
    ci_commit_min_width(ci_runs) + ci_branch_min_width(ci_runs)
}

fn ci_commit_min_width(ci_runs: &[CiRun]) -> usize {
    let observed = ci_runs
        .iter()
        .map(|run| run.commit_title.as_deref().unwrap_or("").len() + 1)
        .max()
        .unwrap_or(CI_COMMIT_MIN_WIDTH);
    ci_content_aware_min_width(observed, CI_COMMIT_MIN_WIDTH, CI_COMMIT_LONG_MIN_WIDTH)
}

fn ci_branch_min_width(ci_runs: &[CiRun]) -> usize {
    let observed = ci_runs
        .iter()
        .map(|run| run.branch.len())
        .max()
        .unwrap_or(CI_BRANCH_MIN_WIDTH);
    ci_content_aware_min_width(observed, CI_BRANCH_MIN_WIDTH, CI_BRANCH_LONG_MIN_WIDTH)
}

const fn ci_content_aware_min_width(observed: usize, compact_min: usize, long_min: usize) -> usize {
    if observed > compact_min {
        if observed < long_min {
            observed
        } else {
            long_min
        }
    } else {
        compact_min
    }
}

fn ci_non_description_width(
    ci_runs: &[CiRun],
    cols: &[CiDisplayColumn],
    show_durations: bool,
) -> usize {
    let column_count = 3 + cols.len() + 1;
    let base = usize::from(CI_TIMESTAMP_WIDTH);
    let job_columns: usize = cols
        .iter()
        .map(|col| ci_duration_width(ci_runs, col, show_durations))
        .sum();
    let total = ci_total_width(ci_runs, show_durations);
    base + job_columns + total + column_count.saturating_sub(1)
}

fn ci_display_table_shows_durations(
    ci_runs: &[CiRun],
    cols: &[CiDisplayColumn],
    inner_width: u16,
) -> bool {
    ci_table_fixed_width(ci_runs, cols, true) <= usize::from(inner_width)
}

fn ci_panel_title(data: &CiData, focused_pos: Option<usize>) -> String {
    let title = tui_pane::pane_title(
        "CI Runs",
        &PaneTitleCount::Single {
            len:    data.runs.len(),
            cursor: focused_pos,
        },
    );

    match (data.mode_label.as_deref(), data.current_branch.as_deref()) {
        (Some("branch"), Some(branch)) if !branch.is_empty() => {
            let base = title.trim();
            format!(" {base} branch: {branch} ")
        },
        _ => title,
    }
}

fn empty_ci_title(data: &CiData) -> String { data.empty_state.title() }

pub fn render_ci_pane_body(frame: &mut Frame, area: Rect, pane: &mut Ci, ctx: &PaneRenderCtx<'_>) {
    let Some(ci_data) = pane.content().cloned() else {
        render_empty_ci_block(frame, " No CI Runs ", area);
        return;
    };

    if !ci_data.has_runs() {
        let viewport = &mut pane.viewport;
        viewport.set_len(0);
        viewport.set_content_area(Rect::ZERO);
        render_empty_ci_block(frame, &empty_ci_title(&ci_data), area);
        return;
    }

    let ci_focused = pane.focus.is_focused;
    let ci_pane_focus_state = pane.focus.pane_focus_state;
    let focused_pos = ci_focused.then(|| pane.viewport.pos());
    let title = ci_panel_title(&ci_data, focused_pos);

    let ci_block = tui_pane::default_pane_chrome().block(title, ci_focused);

    let inner = ci_block.inner(area);
    {
        let viewport = &mut pane.viewport;
        viewport.set_len(ci_data.runs.len());
        viewport.set_content_area(inner);
        viewport.set_viewport_rows(usize::from(inner.height.saturating_sub(1)));
    }

    let cols = ci_display_columns_for_width(&ci_data.runs, inner.width);
    let show_durations = ci_display_table_shows_durations(&ci_data.runs, &cols, inner.width);
    let widths = build_ci_widths(&ci_data.runs, &cols, show_durations, inner.width);
    let commit_width = constraint_width(widths.first()).unwrap_or(CI_COMMIT_MIN_WIDTH);
    let branch_width = constraint_width(widths.get(1)).unwrap_or(CI_BRANCH_MIN_WIDTH);

    let viewport_ref = &pane.viewport;
    let rows: Vec<Row> = ci_data
        .runs
        .iter()
        .enumerate()
        .map(|(row_index, ci_run)| {
            build_ci_data_row(
                ci_run,
                &cols,
                show_durations,
                commit_width,
                branch_width,
                tui_pane::selection_state(viewport_ref, row_index, ci_pane_focus_state),
            )
        })
        .collect();

    let header = build_ci_header_row(&cols, &widths);

    let table = Table::new(rows, widths)
        .header(header)
        .block(ci_block)
        .column_spacing(1)
        .row_highlight_style(Style::default());

    let mut table_state = TableState::default().with_selected(Some(pane.viewport.pos()));
    *table_state.offset_mut() = pane.viewport.scroll_offset();
    frame.render_stateful_widget(table, area, &mut table_state);
    pane.viewport.set_scroll_offset(table_state.offset());
    render_overflow_affordance(
        frame,
        area,
        pane.viewport.overflow(),
        Style::default().fg(label_color()),
    );

    let _ = ctx;
}

fn render_empty_ci_block(frame: &mut Frame, title: &str, area: Rect) {
    let block = tui_pane::empty_pane_block(title);
    frame.render_widget(block, area);
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Constraint;
    use unicode_width::UnicodeWidthStr;

    use super::CI_BRANCH_LONG_MIN_WIDTH;
    use super::CI_COMMIT_LONG_MIN_WIDTH;
    use super::CI_JOB_LABEL_MIN_WIDTH;
    use super::CiDisplayColumn;
    use super::OTHER_JOBS_HEADER;
    use super::build_ci_widths;
    use super::ci_branch_min_width;
    use super::ci_column_data;
    use super::ci_commit_min_width;
    use super::ci_display_columns_for_width;
    use super::ci_display_table_shows_durations;
    use super::ci_grouped_display_columns;
    use super::ci_panel_title;
    use super::ci_status_glyph_width;
    use super::ci_table_fixed_width;
    use super::constraint_width;
    use super::infer_ci_columns;
    use crate::ci::CiJob;
    use crate::ci::CiRun;
    use crate::ci::CiStatus;
    use crate::ci::FetchStatus;
    use crate::tui::panes::CiData;
    use crate::tui::panes::CiEmptyState;

    fn ci_run(branch: &str) -> CiRun {
        CiRun {
            run_id:          1,
            created_at:      "2026-04-01T21:00:00-04:00".to_string(),
            branch:          branch.to_string(),
            url:             "https://example.com/run/1".to_string(),
            ci_status:       CiStatus::Passed,
            jobs:            Vec::new(),
            wall_clock_secs: Some(17),
            commit_title:    Some("feat: add box select".to_string()),
            updated_at:      None,
            fetched:         FetchStatus::Fetched,
        }
    }

    fn ci_run_with_jobs(jobs: Vec<CiJob>) -> CiRun {
        CiRun {
            jobs,
            ..ci_run("main")
        }
    }

    fn ci_run_with_title_branch(title: &str, branch: &str, jobs: Vec<CiJob>) -> CiRun {
        CiRun {
            branch: branch.to_string(),
            commit_title: Some(title.to_string()),
            jobs,
            ..ci_run("main")
        }
    }

    fn ci_job(name: &str, seconds: u64, status: CiStatus) -> CiJob {
        CiJob {
            name:          name.to_string(),
            ci_status:     status,
            duration:      crate::ci::format_secs(seconds),
            duration_secs: Some(seconds),
        }
    }

    #[test]
    fn ci_panel_title_omits_all_mode_suffix() {
        let data = CiData {
            runs:           vec![ci_run("main")],
            mode_label:     Some("all".to_string()),
            current_branch: Some("main".to_string()),
            empty_state:    CiEmptyState::NoRuns,
        };

        assert_eq!(ci_panel_title(&data, Some(0)), " CI Runs (1 of 1) ");
    }

    #[test]
    fn ci_panel_title_appends_branch_name_for_branch_mode() {
        let data = CiData {
            runs:           vec![ci_run("main"), ci_run("main")],
            mode_label:     Some("branch".to_string()),
            current_branch: Some("main".to_string()),
            empty_state:    CiEmptyState::NoRuns,
        };

        assert_eq!(
            ci_panel_title(&data, Some(0)),
            " CI Runs (1 of 2) branch: main "
        );
        assert_eq!(ci_panel_title(&data, None), " CI Runs (2) branch: main ");
    }

    #[test]
    fn ci_columns_group_shorter_jobs_before_dropping_timings() {
        let runs = vec![ci_run_with_jobs(vec![
            ci_job("fmt", 10, CiStatus::Passed),
            ci_job("clippy", 120, CiStatus::Passed),
            ci_job("benchmark", 900, CiStatus::Passed),
            ci_job("docs", 20, CiStatus::Passed),
            ci_job("mend", 80, CiStatus::Passed),
        ])];
        let cols = infer_ci_columns(&runs);
        let selected =
            std::collections::HashSet::from(["clippy".to_string(), "benchmark".to_string()]);
        let expected = ci_grouped_display_columns(&cols, &selected);
        let width = ci_table_fixed_width(&runs, &expected, true);

        assert!(
            ci_table_fixed_width(
                &runs,
                &cols
                    .iter()
                    .map(|col| CiDisplayColumn::Job(col.name.clone()))
                    .collect::<Vec<_>>(),
                true,
            ) > width
        );
        assert_eq!(
            ci_display_columns_for_width(
                &runs,
                u16::try_from(width).unwrap_or_else(|_| std::process::abort()),
            ),
            expected
        );
        assert!(ci_display_table_shows_durations(
            &runs,
            &expected,
            u16::try_from(width).unwrap_or_else(|_| std::process::abort()),
        ));
    }

    #[test]
    fn ci_table_duration_visibility_tracks_fixed_column_width() {
        let runs = vec![ci_run_with_jobs(vec![
            ci_job("fmt", 17, CiStatus::Passed),
            ci_job("clippy", 21, CiStatus::Passed),
        ])];
        let cols = vec![
            CiDisplayColumn::Job("fmt".to_string()),
            CiDisplayColumn::Job("clippy".to_string()),
        ];

        assert!(!ci_display_table_shows_durations(&runs, &cols, 20));
        assert!(ci_display_table_shows_durations(&runs, &cols[..1], 80));
        assert_eq!(
            super::ci_total_width(&runs, false),
            "Total".len().max(ci_status_glyph_width())
        );
    }

    #[test]
    fn ci_job_headers_use_combined_duration_status_width() {
        let runs = vec![ci_run_with_jobs(vec![ci_job(
            "Format Check",
            18,
            CiStatus::Passed,
        )])];
        let cols = vec![CiDisplayColumn::Job("Format Check".to_string())];
        let baseline_width = super::ci_duration_width(&runs, &cols[0], true);
        let target_width = "Format Check".len();
        let inner_width = ci_table_fixed_width(&runs, &cols, true) + target_width - baseline_width;

        let widths = build_ci_widths(
            &runs,
            &cols,
            true,
            u16::try_from(inner_width).unwrap_or_else(|_| std::process::abort()),
        );
        let job_width = constraint_width(widths.get(3)).unwrap_or(0);

        assert_eq!(job_width, target_width);
        assert_eq!(cols[0].header_label_for_width(job_width), "Format Check");
    }

    #[test]
    fn ci_job_header_minimum_includes_status_area() {
        let runs = vec![ci_run_with_jobs(vec![ci_job(
            "Format Check",
            18,
            CiStatus::Passed,
        )])];
        let cols = vec![CiDisplayColumn::Job("Format Check".to_string())];
        let expected_min = cols[0].header_min_label().width()
            + super::CI_STATUS_GAP_WIDTH
            + ci_status_glyph_width();

        assert_eq!(
            super::ci_duration_width(&runs, &cols[0], true),
            expected_min
        );

        let widths = build_ci_widths(
            &runs,
            &cols,
            true,
            u16::try_from(ci_table_fixed_width(&runs, &cols, true))
                .unwrap_or_else(|_| std::process::abort()),
        );
        let job_width = constraint_width(widths.get(3)).unwrap_or(0);

        assert!(job_width >= expected_min);
        assert!(cols[0].header_label_for_width(job_width).width() > CI_JOB_LABEL_MIN_WIDTH);
    }

    #[test]
    fn ci_other_column_sums_duration_and_combines_status() {
        let run = ci_run_with_jobs(vec![
            ci_job("fmt", 10, CiStatus::Passed),
            ci_job("docs", 20, CiStatus::Failed),
            ci_job("benchmark", 900, CiStatus::Passed),
        ]);
        let other = CiDisplayColumn::Other {
            jobs: vec!["fmt".to_string(), "docs".to_string()],
        };

        let data = ci_column_data(&run, &other).unwrap_or_else(|| std::process::abort());

        assert_eq!(data.duration, "30s");
        assert_eq!(data.status, CiStatus::Failed);
    }

    #[test]
    fn ci_description_min_widths_grow_for_long_commit_and_branch_values() {
        let runs = vec![ci_run_with_title_branch(
            "Allow for querying supported sample counts in Msaa",
            "special-case-activities-now",
            vec![ci_job("fmt", 10, CiStatus::Passed)],
        )];
        let cols = ci_display_columns_for_width(&runs, 120);
        let widths = build_ci_widths(&runs, &cols, true, 120);

        assert_eq!(ci_commit_min_width(&runs), CI_COMMIT_LONG_MIN_WIDTH);
        assert_eq!(ci_branch_min_width(&runs), CI_BRANCH_LONG_MIN_WIDTH);
        assert!(
            constraint_width(widths.first()).unwrap_or(0) >= CI_COMMIT_LONG_MIN_WIDTH,
            "long commit titles should reserve a useful prefix"
        );
        assert!(
            constraint_width(widths.get(1)).unwrap_or(0) >= CI_BRANCH_LONG_MIN_WIDTH,
            "long branch names should reserve a useful prefix"
        );
    }

    #[test]
    fn ci_description_min_widths_stay_compact_for_short_values() {
        let runs = vec![ci_run_with_title_branch(
            "CI",
            "main",
            vec![ci_job("fmt", 10, CiStatus::Passed)],
        )];

        assert_eq!(ci_commit_min_width(&runs), " Commit".len());
        assert_eq!(ci_branch_min_width(&runs), "Branch".len());
    }

    #[test]
    fn ci_bevy_cached_shape_keeps_runtime_headers_and_timings_in_width() {
        let runs = bevy_cached_ci_runs();
        let width = 180;
        let cols = ci_display_columns_for_width(&runs, width);

        assert!(
            cols.iter()
                .any(|col| matches!(col, CiDisplayColumn::Other { .. })),
            "real Bevy cache shape should group short-runtime jobs into Other"
        );
        assert!(ci_display_table_shows_durations(&runs, &cols, width));

        let widths = build_ci_widths(&runs, &cols, true, width);
        let rendered_width = widths
            .iter()
            .map(|constraint| match constraint {
                Constraint::Length(width) => usize::from(*width),
                _ => 0,
            })
            .sum::<usize>()
            + widths.len().saturating_sub(1);

        assert!(
            rendered_width <= usize::from(width),
            "rendered width {rendered_width} should fit in {width}"
        );
    }

    #[test]
    fn ci_bevy_cached_shape_uses_slack_for_longer_headers() {
        let runs = bevy_cached_ci_runs();
        let width = 220;
        let cols = ci_display_columns_for_width(&runs, width);
        let widths = build_ci_widths(&runs, &cols, true, width);

        let labels: Vec<String> = cols
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let width = constraint_width(widths.get(3 + i)).unwrap_or(CI_JOB_LABEL_MIN_WIDTH);
                col.header_label_for_width(width)
            })
            .collect();

        assert!(
            labels
                .iter()
                .any(|label| label.chars().count() > CI_JOB_LABEL_MIN_WIDTH),
            "at least one Bevy CI job header should expand beyond the minimum: {labels:?}"
        );
        assert!(labels.iter().any(|label| label == OTHER_JOBS_HEADER));
    }

    #[allow(
        clippy::too_many_lines,
        reason = "fixture mirrors the cached Bevy CI shape that exposed the layout bug"
    )]
    fn bevy_cached_ci_runs() -> Vec<CiRun> {
        vec![
            ci_run_with_title_branch(
                "CI - PR Comments",
                "main",
                vec![
                    ci_job("msrv", 0, CiStatus::Skipped),
                    ci_job("missing-features", 0, CiStatus::Skipped),
                    ci_job("missing-examples", 0, CiStatus::Skipped),
                ],
            ),
            ci_run_with_title_branch(
                "Resolve `FontSource`s on changes",
                "reresolve-font-sources-on-font-asset-changes",
                vec![
                    ci_job("check-unused-dependencies", 0, CiStatus::Skipped),
                    ci_job(
                        "build-without-default-features-status",
                        0,
                        CiStatus::Skipped,
                    ),
                    ci_job("build-without-default-features", 0, CiStatus::Skipped),
                    ci_job("build-and-install-on-iOS", 0, CiStatus::Skipped),
                    ci_job(
                        "check-example-showcase-patches-still-work",
                        0,
                        CiStatus::Skipped,
                    ),
                    ci_job("run-examples-on-wasm", 0, CiStatus::Skipped),
                    ci_job("build-android", 0, CiStatus::Skipped),
                ],
            ),
            ci_run_with_title_branch(
                "Specular aa",
                "specular-aa",
                vec![
                    ci_job("run-examples-macos-metal", 568, CiStatus::Passed),
                    ci_job("run-examples-linux-vulkan", 0, CiStatus::Skipped),
                    ci_job("Compare Windows screenshots", 0, CiStatus::Skipped),
                    ci_job("Compare Linux screenshots", 0, CiStatus::Skipped),
                    ci_job("run-examples-on-windows-dx12", 0, CiStatus::Skipped),
                    ci_job("Compare Macos screenshots", 0, CiStatus::Skipped),
                ],
            ),
            ci_run_with_title_branch(
                "Specular aa",
                "specular-aa",
                vec![
                    ci_job("check-bans", 142, CiStatus::Passed),
                    ci_job("check-advisories", 142, CiStatus::Passed),
                    ci_job("check-licenses", 136, CiStatus::Passed),
                    ci_job("check-sources", 144, CiStatus::Passed),
                ],
            ),
            ci_run_with_title_branch(
                "Specular aa",
                "specular-aa",
                vec![
                    ci_job("CodeQL Analyze (rust)", 244, CiStatus::Passed),
                    ci_job("zizmor", 18, CiStatus::Passed),
                    ci_job("CodeQL Analyze (actions)", 49, CiStatus::Passed),
                ],
            ),
            ci_run_with_title_branch(
                "Allow for querying supported sample counts in Msaa",
                "msaa-sample-count-validation",
                vec![
                    ci_job("CodeQL Analyze (rust)", 236, CiStatus::Passed),
                    ci_job("zizmor", 22, CiStatus::Passed),
                    ci_job("CodeQL Analyze (actions)", 50, CiStatus::Passed),
                ],
            ),
            ci_run_with_title_branch(
                "Specular aa",
                "specular-aa",
                vec![
                    ci_job("check-doc", 743, CiStatus::Passed),
                    ci_job("check-missing-examples-in-docs", 27, CiStatus::Passed),
                    ci_job("toml", 8, CiStatus::Passed),
                    ci_job("typos", 11, CiStatus::Passed),
                    ci_job("build (macos-latest)", 1162, CiStatus::Passed),
                    ci_job("check-release-content", 30, CiStatus::Passed),
                    ci_job("build (windows-latest)", 1520, CiStatus::Passed),
                    ci_job("build (ubuntu-latest)", 860, CiStatus::Passed),
                    ci_job("ci", 397, CiStatus::Passed),
                    ci_job("miri", 1140, CiStatus::Passed),
                    ci_job("check-bevy-internal-imports", 7, CiStatus::Passed),
                    ci_job("markdownlint", 91, CiStatus::Passed),
                    ci_job("check-missing-features-in-docs", 0, CiStatus::Skipped),
                    ci_job("check-compiles-no-std-examples", 58, CiStatus::Passed),
                    ci_job(
                        "check-compiles-no-std-portable-atomic",
                        56,
                        CiStatus::Passed,
                    ),
                    ci_job("check-compiles-no-std", 77, CiStatus::Passed),
                    ci_job("check-compiles", 668, CiStatus::Passed),
                    ci_job("build-wasm-atomics", 129, CiStatus::Passed),
                    ci_job("build-wasm", 148, CiStatus::Passed),
                    ci_job("msrv", 146, CiStatus::Passed),
                ],
            ),
        ]
    }
}
