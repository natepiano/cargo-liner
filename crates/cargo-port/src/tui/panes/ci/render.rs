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
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use tui_pane::ColorMode;
use tui_pane::PaneFrameChrome;
use tui_pane::PaneSelectionState;
use tui_pane::PaneTitleCount;
use tui_pane::label_color;
use unicode_width::UnicodeWidthStr;

use super::CiData;
use super::data::CiEmptyState;
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
use crate::tui::panes::support;
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
            if let Some(existing_ci_job_column) = cols.iter_mut().find(|col| col.name == job.name) {
                existing_ci_job_column.max_runtime_s =
                    existing_ci_job_column.max_runtime_s.max(runtime);
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
    let all_ci_display_columns = ci_job_display_columns(&cols);
    if ci_table_fixed_width(runs, &all_ci_display_columns, true) <= usize::from(inner_width) {
        return all_ci_display_columns;
    }

    let mut ranked_ci_job_columns = cols.clone();
    ranked_ci_job_columns.sort_by(|left, right| {
        right
            .max_runtime_s
            .cmp(&left.max_runtime_s)
            .then_with(|| left.first_seen.cmp(&right.first_seen))
    });

    let mut selected = HashSet::new();
    for ci_job_column in ranked_ci_job_columns {
        selected.insert(ci_job_column.name.clone());
        let ci_display_columns = ci_grouped_display_columns(&cols, &selected);
        if ci_table_fixed_width(runs, &ci_display_columns, true) > usize::from(inner_width) {
            selected.remove(&ci_job_column.name);
        }
    }

    ci_grouped_display_columns(&cols, &selected)
}

fn ci_grouped_display_columns(
    cols: &[CiJobColumn],
    selected: &HashSet<String>,
) -> Vec<CiDisplayColumn> {
    let mut ci_display_columns = Vec::new();
    let mut other = Vec::new();
    for ci_job_column in cols {
        if selected.contains(&ci_job_column.name) {
            ci_display_columns.push(CiDisplayColumn::Job(ci_job_column.name.clone()));
        } else {
            other.push(ci_job_column.name.clone());
        }
    }
    if !other.is_empty() {
        ci_display_columns.push(CiDisplayColumn::Other { jobs: other });
    }
    ci_display_columns
}

fn build_ci_header_row(cols: &[CiDisplayColumn], widths: &[Constraint]) -> Row<'static> {
    let header_style = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(theme_roles::column_header_color());
    let mut header_cells = vec![
        Cell::from(" Commit").style(header_style),
        Cell::from("Branch").style(header_style),
        Cell::from("Timestamp").style(header_style),
    ];
    for (i, ci_display_column) in cols.iter().enumerate() {
        let width = constraint_width(widths.get(3 + i)).unwrap_or(CI_JOB_LABEL_MIN_WIDTH);
        header_cells.push(
            Cell::from(
                Line::from(ci_display_column.header_label_for_width(width))
                    .alignment(Alignment::Right),
            )
            .style(header_style),
        );
    }
    header_cells.push(ci_total_header_cell().style(header_style));
    Row::new(header_cells).bottom_margin(0)
}

fn build_ci_data_row(
    ci_run: &CiRun,
    cols: &[CiDisplayColumn],
    show_durations: bool,
    commit_width: usize,
    branch_width: usize,
    pane_selection_state: PaneSelectionState,
    mode: ColorMode,
) -> Row<'static> {
    let timestamp = pane_data::format_timestamp(&ci_run.created_at);
    let total_dur = ci_total_seconds(ci_run).map_or_else(|| "—".to_string(), ci::format_secs);

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

    for ci_display_column in cols {
        let data = ci_column_data(ci_run, ci_display_column);
        if let Some(data) = data {
            let style = render::ci_status_style(Some(data.status));
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

    let total_style = render::ci_status_style(Some(ci_run.ci_status));
    cells.push(ci_metric_cell(
        total_dur.trim(),
        ci_run.ci_status,
        show_durations,
        total_style,
    ));

    Row::new(cells).style(pane_selection_state.overlay_style(mode))
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
    // `build_ci_widths` builds `ColumnSpec` entries for Commit, Branch, Timestamp, each
    // `CiDisplayColumn` and Total; `ci_metric_cell` combines duration and status glyph, so headers
    // can use both areas.
    let (commit_width, branch_width) =
        ci_description_widths(ci_runs, cols, show_durations, inner_width);
    let mut column_specs = vec![
        ColumnSpec {
            min: label_width(" Commit"),
            max: Some(commit_width),
        },
        ColumnSpec {
            min: branch_width,
            max: Some(branch_width),
        },
        ColumnSpec::fixed(CI_TIMESTAMP_WIDTH),
    ];
    for ci_display_column in cols {
        let label_min = label_width(&ci_display_column.header_min_label());
        if show_durations {
            column_specs.push(ColumnSpec::fit(label_min.max(ci_status_glyph_width_u16())));
        } else {
            column_specs.push(ColumnSpec::fixed(
                label_min.max(ci_status_glyph_width_u16()),
            ));
        }
    }
    let total_label = label_width("Total");
    if show_durations {
        column_specs.push(ColumnSpec::fit(
            total_label.max(ci_status_glyph_width_u16()),
        ));
    } else {
        column_specs.push(ColumnSpec::fixed(
            total_label.max(ci_status_glyph_width_u16()),
        ));
    }

    let spec_len = column_specs.len();
    let mut column_widths = ColumnWidths::new(column_specs);

    for run in ci_runs {
        column_widths.observe_cell_usize(0, run.commit_title.as_deref().unwrap_or("").width() + 1);
        column_widths.observe_cell_usize(1, run.branch.width());
    }
    if show_durations {
        for (i, ci_display_column) in cols.iter().enumerate() {
            let column_index = 3 + i;
            for run in ci_runs {
                if let Some(data) = ci_column_data(run, ci_display_column) {
                    column_widths.observe_cell_usize(
                        column_index,
                        ci_metric_content_width(data.duration.trim().width()),
                    );
                }
            }
        }
        let total_idx = 3 + cols.len();
        for run in ci_runs {
            if let Some(seconds) = ci_total_seconds(run) {
                column_widths.observe_cell_usize(
                    total_idx,
                    ci_metric_content_width(ci::format_secs(seconds).trim().width()),
                );
            }
        }
    }

    let mut resolved: Vec<u16> = (0..spec_len).map(|i| column_widths.get(i)).collect();
    widen_ci_header_columns(&mut resolved, cols, inner_width);
    if let Some(slack) = ci_width_slack(&resolved, inner_width) {
        let title_width = ci_runs
            .iter()
            .map(|run| run.commit_title.as_deref().unwrap_or("").width() + 1)
            .max()
            .unwrap_or(0);
        let grow_by = title_width
            .saturating_sub(usize::from(resolved[0]))
            .min(slack);
        resolved[0] = resolved[0].saturating_add(u16::try_from(grow_by).unwrap_or(u16::MAX));
    }
    let mut used = 0;
    resolved
        .into_iter()
        .enumerate()
        .take_while(|(index, width)| {
            used += usize::from(*width) + usize::from(*index > 0);
            used <= usize::from(inner_width)
        })
        .map(|(_, width)| Constraint::Length(width))
        .collect()
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

    for (i, ci_display_column) in cols.iter().enumerate() {
        if slack == 0 {
            break;
        }
        let idx = 3 + i;
        let target = ci_display_column.header_target_width();
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
    if min_total.saturating_add(1) > usize::from(inner_width) {
        let branch = ci_runs
            .iter()
            .map(|run| run.branch.width())
            .max()
            .unwrap_or(0)
            .max("Branch".width());
        if branch + CI_COMMIT_MIN_WIDTH < usize::from(inner_width) {
            let branch = u16::try_from(branch).unwrap_or(u16::MAX);
            return (inner_width.saturating_sub(branch + 1), branch);
        }
        return (inner_width, 0);
    }
    let available = usize::from(inner_width)
        .saturating_sub(ci_non_description_width(ci_runs, cols, show_durations))
        .max(min_total);

    let commit_observed = ci_runs
        .iter()
        .map(|run| run.commit_title.as_deref().unwrap_or("").width() + 1)
        .max()
        .unwrap_or(CI_COMMIT_MIN_WIDTH)
        .max(CI_COMMIT_MIN_WIDTH);
    let branch_observed = ci_runs
        .iter()
        .map(|run| run.branch.width())
        .max()
        .unwrap_or(CI_BRANCH_MIN_WIDTH)
        .max(CI_BRANCH_MIN_WIDTH);

    let branch_ceiling = (available / 3).max(branch_min);
    let branch = if branch_observed <= branch_ceiling
        && branch_observed <= available.saturating_sub(commit_min)
    {
        branch_observed
    } else {
        0
    };
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

fn ci_total_seconds(run: &CiRun) -> Option<u64> {
    if run.jobs.is_empty() {
        run.wall_clock_secs
    } else {
        run.jobs.iter().try_fold(0_u64, |total, job| {
            Some(total.saturating_add(job.duration_secs?))
        })
    }
}

fn ci_total_duration_width(ci_runs: &[CiRun]) -> usize {
    ci_runs
        .iter()
        .filter_map(ci_total_seconds)
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
        .map(|run| run.commit_title.as_deref().unwrap_or("").width() + 1)
        .max()
        .unwrap_or(CI_COMMIT_MIN_WIDTH);
    ci_content_aware_min_width(observed, CI_COMMIT_MIN_WIDTH, CI_COMMIT_LONG_MIN_WIDTH)
}

fn ci_branch_min_width(ci_runs: &[CiRun]) -> usize {
    let observed = ci_runs
        .iter()
        .map(|run| run.branch.width())
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
        .map(|ci_display_column| ci_duration_width(ci_runs, ci_display_column, show_durations))
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
        "CI runs",
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

fn ci_total_header_cell() -> Cell<'static> {
    Cell::from(Line::from("Total").alignment(Alignment::Right))
}

fn compact_ci_header_row() -> Row<'static> {
    let style = Style::default()
        .fg(theme_roles::column_header_color())
        .add_modifier(Modifier::BOLD);
    Row::new(vec![
        Cell::from(" Commit").style(style),
        ci_total_header_cell().style(style),
    ])
}

fn paint_ci_row_selections(
    frame: &mut Frame,
    inner: Rect,
    pane: &Ci,
    run_count: usize,
    mode: ColorMode,
) {
    let offset = pane.viewport.scroll_offset();
    for slot in 0..usize::from(inner.height.saturating_sub(1)).min(run_count.saturating_sub(offset))
    {
        theme_roles::paint_selection(
            frame,
            Rect::new(
                inner.x,
                inner.y + 1 + u16::try_from(slot).unwrap_or(u16::MAX),
                inner.width,
                1,
            ),
            tui_pane::selection_state(&pane.viewport, offset + slot, pane.focus.pane_focus_state),
            mode,
        );
    }
}

fn render_empty_ci(frame: &mut Frame, area: Rect, data: &CiData) -> PaneFrameChrome {
    let title = empty_ci_title(data);
    let text = title.trim();
    let (message, required_width) = text.strip_prefix("CI runs — ").map_or_else(
        || {
            let fixed_words = match data.empty_state {
                CiEmptyState::NoRunsForBranch(_) => "No CI runs for branch",
                _ => text,
            };
            (text, fixed_words.width())
        },
        |message| (message, "CI runs — ".width() + 2),
    );
    if required_width > usize::from(area.width.saturating_sub(4)) {
        let inner = tui_pane::frame_inner(area);
        let lines = support::word_wrap(message, usize::from(inner.width.saturating_sub(2)));
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .map(|line| Line::from(format!(" {line}")))
                    .collect::<Vec<_>>(),
            )
            .style(Style::default().fg(label_color())),
            inner,
        );
        return empty_ci_chrome(" CI runs ");
    }
    empty_ci_chrome(&title)
}

pub fn render_ci_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut Ci,
    pane_render_ctx: &PaneRenderCtx<'_>,
) -> PaneFrameChrome {
    let mode = pane_render_ctx.color_mode;
    let ci_data = pane.content().cloned().unwrap_or_else(|| CiData {
        runs:           Vec::new(),
        mode_label:     None,
        current_branch: None,
        empty_state:    CiEmptyState::NoRuns,
    });

    if !ci_data.has_runs() {
        let viewport = &mut pane.viewport;
        viewport.set_len(0);
        viewport.set_content_area(Rect::ZERO);
        return render_empty_ci(frame, area, &ci_data);
    }

    let ci_focused = pane.focus.is_focused();
    let ci_pane_focus_state = pane.focus.pane_focus_state;
    let focused_pos = ci_focused.then(|| pane.viewport.pos());
    let title = ci_panel_title(&ci_data, focused_pos);

    let mut pane_frame_chrome = PaneFrameChrome {
        title,
        focused: ci_focused,
        ..PaneFrameChrome::default()
    };

    let mut inner = tui_pane::frame_inner(area);
    inner.width = inner.width.saturating_sub(1);
    {
        let viewport = &mut pane.viewport;
        viewport.set_len(ci_data.runs.len());
        viewport.set_content_area(inner);
        viewport.set_viewport_rows(usize::from(inner.height.saturating_sub(1)));
    }

    let cols = ci_display_columns_for_width(&ci_data.runs, inner.width);
    let show_durations = ci_display_table_shows_durations(&ci_data.runs, &cols, inner.width);
    let layout = ci_widths_for_mode(&ci_data, &cols, show_durations, inner.width);
    let widths = layout.widths();
    let commit_width = constraint_width(widths.first()).unwrap_or(CI_COMMIT_MIN_WIDTH);
    let branch_width = constraint_width(widths.get(1)).unwrap_or(CI_BRANCH_MIN_WIDTH);

    let viewport_ref = &pane.viewport;
    let rows: Vec<Row> = ci_data
        .runs
        .iter()
        .enumerate()
        .map(|(row_index, ci_run)| {
            if matches!(layout, CiTableLayout::Standard(_)) {
                build_ci_data_row(
                    ci_run,
                    &cols,
                    show_durations,
                    commit_width,
                    branch_width,
                    tui_pane::selection_state(viewport_ref, row_index, ci_pane_focus_state),
                    mode,
                )
            } else {
                layout.summary_row(ci_run)
            }
        })
        .collect();

    let header = layout.header(&cols);

    let spacing = u16::from(
        !(matches!(layout, CiTableLayout::BranchSummary(_))
            && widths
                .iter()
                .filter_map(|width| constraint_width(Some(width)))
                .sum::<usize>()
                == usize::from(inner.width)),
    );
    let table = Table::new(rows, widths.iter().copied())
        .header(header)
        .column_spacing(spacing)
        .row_highlight_style(Style::default());

    let mut table_state = TableState::default().with_selected(Some(pane.viewport.pos()));
    *table_state.offset_mut() = pane.viewport.scroll_offset();
    frame.render_stateful_widget(table, inner, &mut table_state);
    pane.viewport.set_scroll_offset(table_state.offset());
    paint_ci_row_selections(frame, inner, pane, ci_data.runs.len(), mode);
    pane_frame_chrome
        .labels
        .extend(tui_pane::overflow_affordance_label(
            area,
            pane.viewport.overflow(),
            Style::default().fg(label_color()),
        ));

    let _ = pane_render_ctx;

    pane_frame_chrome
}

/// The frame for a pane with nothing to list.
///
/// Reported unfocused whatever the pane's own focus is, which is the
/// dim shade `empty_pane_block` drew before the frame was shared.
fn empty_ci_chrome(title: &str) -> PaneFrameChrome {
    PaneFrameChrome {
        title: title.to_string(),
        focused: false,
        ..PaneFrameChrome::default()
    }
}

enum CiTableLayout {
    Standard(Vec<Constraint>),
    BranchSummary(Vec<Constraint>),
    AllSummary(Vec<Constraint>),
    StatusSummary(Vec<Constraint>),
}

impl CiTableLayout {
    fn summary_row(&self, run: &CiRun) -> Row<'static> {
        let widths = self.widths();
        let commit = constraint_width(widths.first()).unwrap_or(0);
        let mut cells = vec![Cell::from(ci_header_label_for_width(
            &format!(" {}", run.commit_title.as_deref().unwrap_or("")),
            commit,
        ))];
        if matches!(self, Self::StatusSummary(_)) {
            cells.clear();
            if widths.is_empty() {
                return Row::new(cells);
            }
        }
        if matches!(self, Self::AllSummary(_)) {
            let branch = constraint_width(widths.get(1)).unwrap_or(0);
            cells.push(Cell::from(ci_header_label_for_width(&run.branch, branch)));
        }
        cells.push(ci_metric_cell(
            &ci_total_seconds(run).map_or_else(|| "—".to_string(), ci::format_secs),
            run.ci_status,
            true,
            render::ci_status_style(Some(run.ci_status)),
        ));
        Row::new(cells)
    }

    fn header(&self, cols: &[CiDisplayColumn]) -> Row<'static> {
        match self {
            Self::Standard(widths) => build_ci_header_row(cols, widths),
            Self::BranchSummary(widths) => {
                if constraint_width(widths.first()).unwrap_or(0) < " Commit".width() {
                    Row::new(vec![Cell::from("Commit"), ci_total_header_cell()]).style(
                        Style::default()
                            .fg(theme_roles::column_header_color())
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    compact_ci_header_row()
                }
            },
            Self::AllSummary(widths) => Row::new(vec![
                Cell::from(if constraint_width(widths.first()).unwrap_or(0) == 0 {
                    ""
                } else {
                    " Commit"
                }),
                Cell::from("Branch"),
                ci_total_header_cell(),
            ])
            .style(
                Style::default()
                    .fg(theme_roles::column_header_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Self::StatusSummary(widths) => Row::new(if widths.is_empty() {
                Vec::new()
            } else {
                vec![ci_total_header_cell()]
            })
            .style(
                Style::default()
                    .fg(theme_roles::column_header_color())
                    .add_modifier(Modifier::BOLD),
            ),
        }
    }

    fn widths(&self) -> &[Constraint] {
        match self {
            Self::Standard(widths)
            | Self::BranchSummary(widths)
            | Self::AllSummary(widths)
            | Self::StatusSummary(widths) => widths,
        }
    }
}

fn ci_widths_for_mode(
    data: &CiData,
    cols: &[CiDisplayColumn],
    durations: bool,
    width: u16,
) -> CiTableLayout {
    let widths = build_ci_widths(&data.runs, cols, durations, width);
    let fits = ci_table_fixed_width(&data.runs, cols, durations) <= usize::from(width);
    let all = data.mode_label.as_deref() == Some("all");
    let branch_visible = constraint_width(widths.get(1)).unwrap_or(0) >= "Branch".width();
    if fits && (!all || branch_visible) {
        return CiTableLayout::Standard(widths);
    }
    let status = u16::try_from(ci_total_width(&data.runs, true)).unwrap_or(u16::MAX);
    if width < status {
        return CiTableLayout::StatusSummary(Vec::new());
    }
    if all
        && width
            >= label_width("Branch")
                .saturating_add(status)
                .saturating_add(2)
    {
        let branch = label_width("Branch").max(width.saturating_sub(status + 2) / 3);
        let remaining = width.saturating_sub(branch + status + 2);
        let commit = if remaining >= label_width(" Commit") {
            remaining
        } else {
            0
        };
        let branch = branch + remaining.saturating_sub(commit);
        return CiTableLayout::AllSummary(vec![
            Constraint::Length(commit),
            Constraint::Length(branch),
            Constraint::Length(status),
        ]);
    }
    if all || data.mode_label.as_deref() == Some("branch") || widths.len() < cols.len() + 4 {
        if width < label_width("Commit").saturating_add(status) {
            return CiTableLayout::StatusSummary(vec![Constraint::Length(status)]);
        }
        let spacing = u16::from(width > label_width("Commit").saturating_add(status));
        let commit = width.saturating_sub(status + spacing);
        return CiTableLayout::BranchSummary(vec![
            Constraint::Length(commit),
            Constraint::Length(status),
        ]);
    }
    CiTableLayout::Standard(widths)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests should fail on invalid fixtures")]
mod tests {
    use ratatui::layout::Constraint;
    use unicode_width::UnicodeWidthStr;

    use super::CI_BRANCH_LONG_MIN_WIDTH;
    use super::CI_COMMIT_LONG_MIN_WIDTH;
    use super::CI_JOB_LABEL_MIN_WIDTH;
    use super::CI_STATUS_GAP_WIDTH;
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

        assert_eq!(ci_panel_title(&data, Some(0)), " CI runs (1 of 1) ");
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
            " CI runs (1 of 2) branch: main "
        );
        assert_eq!(ci_panel_title(&data, None), " CI runs (2) branch: main ");
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
        let expected_ci_display_columns = ci_grouped_display_columns(&cols, &selected);
        let width = ci_table_fixed_width(&runs, &expected_ci_display_columns, true);

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
                u16::try_from(width).expect("grouped CI table width should fit u16"),
            ),
            expected_ci_display_columns
        );
        assert!(ci_display_table_shows_durations(
            &runs,
            &expected_ci_display_columns,
            u16::try_from(width).expect("grouped CI table width should fit u16"),
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
            u16::try_from(inner_width).expect("CI header width fixture should fit u16"),
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
        let expected_min =
            cols[0].header_min_label().width() + CI_STATUS_GAP_WIDTH + ci_status_glyph_width();

        assert_eq!(
            super::ci_duration_width(&runs, &cols[0], true),
            expected_min
        );

        let widths = build_ci_widths(
            &runs,
            &cols,
            true,
            u16::try_from(ci_table_fixed_width(&runs, &cols, true))
                .expect("CI minimum header fixture width should fit u16"),
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

        let data = ci_column_data(&run, &other).expect("other CI column should summarize jobs");

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
            .map(|(i, ci_display_column)| {
                let width = constraint_width(widths.get(3 + i)).unwrap_or(CI_JOB_LABEL_MIN_WIDTH);
                ci_display_column.header_label_for_width(width)
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

#[cfg(test)]
mod margin_title_tests {
    use anyhow::Context;
    use tui_pane::ColorMode;

    use crate::tui::panes::CiData;
    use crate::tui::panes::CiEmptyState;
    use crate::tui::panes::PaneId;
    use crate::tui::test_support;

    #[test]
    fn empty_ci_titles_keep_the_fixed_words_at_narrow_window_widths() -> anyhow::Result<()> {
        for (window, pane_width) in [(80, 34), (95, 41)] {
            let mut app = test_support::project_app();
            app.framework.color_mode = ColorMode::Colored;
            for (state, expected) in [
                (CiEmptyState::Fetching, "CI runs — loading…"),
                (CiEmptyState::Loading, "CI runs — loading…"),
                (CiEmptyState::NoRuns, "No CI runs"),
                (
                    CiEmptyState::NoWorkflowConfigured,
                    "No CI workflow configured",
                ),
                (CiEmptyState::NotGitRepo, "CI runs — not a git repo"),
                (
                    CiEmptyState::RequiresGithubRemote,
                    "CI runs — no GitHub remote",
                ),
                (CiEmptyState::Unreadable, "CI runs — unreadable"),
                (
                    CiEmptyState::NoRunsForBranch("verify_a_very_long_branch_name".to_string()),
                    "No CI runs for branch",
                ),
            ] {
                let branch = matches!(state, CiEmptyState::NoRunsForBranch(_));
                app.ci.set_content(CiData {
                    runs:           Vec::new(),
                    mode_label:     None,
                    current_branch: None,
                    empty_state:    state,
                });
                let buffer = test_support::draw_pane_in_window(
                    &mut app,
                    PaneId::CiRuns,
                    window,
                    pane_width,
                )?;
                let title = test_support::rows(&buffer)
                    .into_iter()
                    .next()
                    .context("CI title")?;
                assert!(title.contains(expected), "{window}: {title}");
                if branch {
                    assert!(title.contains('…'), "{title}");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn narrow_empty_ci_titles_move_the_message_into_the_body() -> anyhow::Result<()> {
        let mut app = test_support::project_app();
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            app.framework.color_mode = mode;
            for (width, state) in [
                (15, CiEmptyState::Fetching),
                (15, CiEmptyState::Loading),
                (15, CiEmptyState::NotGitRepo),
                (15, CiEmptyState::RequiresGithubRemote),
                (15, CiEmptyState::Unreadable),
                (13, CiEmptyState::Fetching),
                (13, CiEmptyState::Loading),
                (13, CiEmptyState::NotGitRepo),
                (13, CiEmptyState::RequiresGithubRemote),
                (13, CiEmptyState::Unreadable),
                (13, CiEmptyState::NoRuns),
                (13, CiEmptyState::NoWorkflowConfigured),
                (13, CiEmptyState::NoRunsForBranch("fleet".to_string())),
            ] {
                let title = state.title();
                let message = title
                    .trim()
                    .strip_prefix("CI runs — ")
                    .unwrap_or_else(|| title.trim())
                    .to_string();
                app.ci.set_content(CiData {
                    runs:           Vec::new(),
                    mode_label:     None,
                    current_branch: None,
                    empty_state:    state,
                });
                let buffer =
                    test_support::draw_pane_in_window(&mut app, PaneId::CiRuns, 60, width)?;
                let rows = test_support::rows(&buffer);
                assert!(rows[0].contains(" CI runs "), "{rows:?}");
                assert!(!rows[0].contains('…'), "{rows:?}");
                let body = rows[1..rows.len() - 1]
                    .iter()
                    .map(|row| row.trim_matches(['│', '┃', ' ']))
                    .filter(|row| !row.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                assert_eq!(
                    body.split_whitespace().collect::<String>(),
                    message.split_whitespace().collect::<String>()
                );
            }
        }
        Ok(())
    }
    #[test]
    fn ci_run_totals_keep_a_blank_cell_before_the_border() -> anyhow::Result<()> {
        for (window, pane_width) in [(80, 34), (95, 41)] {
            let mut app = test_support::project_app();
            app.framework.color_mode = ColorMode::Colored;
            app.ci.set_content(CiData {
                runs:           vec![crate::ci::CiRun {
                    run_id:          1,
                    created_at:      "2026-04-01T21:00:00-04:00".to_string(),
                    branch:          "main".to_string(),
                    url:             "https://example.com/run/1".to_string(),
                    ci_status:       crate::ci::CiStatus::Passed,
                    jobs:            Vec::new(),
                    wall_clock_secs: Some(1155),
                    commit_title:    Some("A long commit title that needs to fit".to_string()),
                    updated_at:      None,
                    fetched:         crate::ci::FetchStatus::Fetched,
                }],
                mode_label:     Some("branch".to_string()),
                current_branch: None,
                empty_state:    CiEmptyState::NoRuns,
            });
            let buffer =
                test_support::draw_pane_in_window(&mut app, PaneId::CiRuns, window, pane_width)?;
            let total = test_support::position(&buffer, "Total")?;
            test_support::position(&buffer, "19m 15s")?;
            for y in total.1..=total.1 + 1 {
                assert_eq!(
                    buffer[(pane_width - 2, y)].symbol(),
                    " ",
                    "{window}: {:?}",
                    test_support::rows(&buffer)
                );
                assert_eq!(buffer[(pane_width - 1, y)].symbol(), "│");
            }
        }
        Ok(())
    }
    #[test]
    fn wide_ci_commit_uses_the_space_before_the_single_margin_cell() -> anyhow::Result<()> {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        app.ci.set_content(CiData {
            runs:           vec![crate::ci::CiRun {
                run_id:          1,
                created_at:      "2026-04-01T21:00:00-04:00".to_string(),
                branch:          "main".to_string(),
                url:             "https://example.com/run/1".to_string(),
                ci_status:       crate::ci::CiStatus::Passed,
                jobs:            Vec::new(),
                wall_clock_secs: Some(5),
                commit_title:    Some("A long commit title that needs more space ".repeat(4)),
                updated_at:      None,
                fetched:         crate::ci::FetchStatus::Fetched,
            }],
            mode_label:     Some("branch".to_string()),
            current_branch: None,
            empty_state:    CiEmptyState::NoRuns,
        });
        let buffer = test_support::draw_pane_in_window(&mut app, PaneId::CiRuns, 160, 95)?;
        let total = test_support::position(&buffer, "Total")?;
        assert_eq!(total.0 + 5, 93, "{:?}", test_support::rows(&buffer));
        assert_eq!(buffer[(93, total.1)].symbol(), " ");
        let rows = test_support::rows(&buffer);
        assert!(rows[usize::from(total.1 + 1)].contains('…'), "{rows:?}");
        Ok(())
    }
}
