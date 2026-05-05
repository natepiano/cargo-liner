use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use unicode_width::UnicodeWidthStr;

use super::CiData;
use super::pane_impls::CiPane;
use crate::ci;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::tui::columns::ColumnSpec;
use crate::tui::columns::ColumnWidths;
use crate::tui::constants::CI_TIMESTAMP_WIDTH;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::pane;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::pane::PaneSelectionState;
use crate::tui::pane::PaneTitleCount;
use crate::tui::render;
use crate::tui::render::CiColumn;

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
            Cell::from(ratatui::text::Line::from(col.label()).alignment(Alignment::Right))
                .style(right_aligned),
        );
        header_cells.push(Cell::from(""));
    }
    header_cells.push(
        Cell::from(ratatui::text::Line::from("Total").alignment(Alignment::Right))
            .style(right_aligned),
    );
    header_cells.push(Cell::from(""));
    Row::new(header_cells).bottom_margin(0)
}

pub const CI_COMPACT_DURATION_WIDTH: usize = 2;

fn build_ci_data_row(
    ci_run: &CiRun,
    cols: &[CiColumn],
    show_durations: bool,
    selection: PaneSelectionState,
) -> Row<'static> {
    let timestamp = super::format_timestamp(&ci_run.created_at);
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
                    .alignment(Alignment::Right),
                )
                .style(style),
            );
            cells.push(Cell::from(job.conclusion.icon().to_string()).style(style));
        } else {
            cells.push(
                Cell::from(ratatui::text::Line::from("—").alignment(Alignment::Right))
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
            .alignment(Alignment::Right),
        )
        .style(total_style),
    );
    cells.push(Cell::from(ci_run.conclusion.icon().to_string()).style(total_style));

    Row::new(cells).style(selection.overlay_style())
}

fn build_ci_widths(ci_runs: &[CiRun], cols: &[CiColumn], show_durations: bool) -> Vec<Constraint> {
    let glyph_width = u16::try_from(
        Conclusion::Success
            .icon()
            .width()
            .max(Conclusion::Failure.icon().width()),
    )
    .unwrap_or(u16::MAX);

    // Column layout: Commit, Branch, Timestamp, then per-job (duration,
    // glyph) pairs, then (Total, glyph). Header-label minimums seed the
    // `Fit` columns; `Fixed` columns ignore observed content.
    let mut specs = vec![
        ColumnSpec::fit(label_width(" Commit")),
        ColumnSpec::fit(label_width("Branch")),
        ColumnSpec::fixed(CI_TIMESTAMP_WIDTH),
    ];
    for col in cols {
        let label_min = label_width(col.label());
        if show_durations {
            specs.push(ColumnSpec::fit(label_min));
        } else {
            specs.push(ColumnSpec::fixed(
                label_min.max(u16::try_from(CI_COMPACT_DURATION_WIDTH).unwrap_or(u16::MAX)),
            ));
        }
        specs.push(ColumnSpec::fixed(glyph_width));
    }
    let total_label = label_width("Total");
    if show_durations {
        specs.push(ColumnSpec::fit(total_label));
    } else {
        let compact = u16::try_from(CI_COMPACT_DURATION_WIDTH).unwrap_or(u16::MAX);
        specs.push(ColumnSpec::fixed(total_label.max(compact)));
    }
    specs.push(ColumnSpec::fixed(glyph_width));

    let mut widths = ColumnWidths::new(specs);

    for run in ci_runs {
        widths.observe_cell_usize(0, run.commit_title.as_deref().unwrap_or("").len());
        widths.observe_cell_usize(1, run.branch.len());
    }
    if show_durations {
        for (i, col) in cols.iter().enumerate() {
            let col_idx = 3 + i * 2;
            for run in ci_runs {
                if let Some(job) = run.jobs.iter().find(|job| col.matches(&job.name)) {
                    widths.observe_cell_usize(col_idx, job.duration.trim().len());
                }
            }
        }
        let total_idx = 3 + cols.len() * 2;
        for run in ci_runs {
            if let Some(seconds) = run.wall_clock_secs {
                widths.observe_cell_usize(total_idx, ci::format_secs(seconds).trim().len());
            }
        }
    }

    widths.to_constraints()
}

/// Display width of a header label, clamped to `u16`.
fn label_width(label: &str) -> u16 { u16::try_from(label.len()).unwrap_or(u16::MAX) }

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

pub fn ci_total_width(ci_runs: &[CiRun], show_durations: bool) -> usize {
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

pub fn ci_table_shows_durations(ci_runs: &[CiRun], cols: &[CiColumn], inner_width: u16) -> bool {
    ci_table_fixed_width(ci_runs, cols, true) <= usize::from(inner_width)
}

fn ci_panel_title(data: &CiData, focused_pos: Option<usize>) -> String {
    let title = pane::pane_title(
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

pub(super) fn render_ci_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut CiPane,
    ctx: &PaneRenderCtx<'_>,
) {
    let Some(ci_data) = pane.content().cloned() else {
        render_empty_ci_block(frame, " No CI Runs ", area);
        return;
    };

    if !ci_data.has_runs() {
        let viewport = pane.viewport_mut();
        viewport.set_len(0);
        viewport.set_content_area(Rect::ZERO);
        render_empty_ci_block(frame, &empty_ci_title(&ci_data), area);
        return;
    }

    let ci_focused = ctx.is_focused;
    let ci_focus = ctx.focus_state;
    let focused_pos = ci_focused.then(|| pane.viewport().pos());
    let title = ci_panel_title(&ci_data, focused_pos);

    let ci_block = pane::default_pane_chrome().block(title, ci_focused);

    let inner = ci_block.inner(area);
    {
        let viewport = pane.viewport_mut();
        viewport.set_len(ci_data.runs.len());
        viewport.set_content_area(inner);
        viewport.set_viewport_rows(usize::from(inner.height.saturating_sub(1)));
    }

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

    let viewport_ref = pane.viewport();
    let rows: Vec<Row> = ci_data
        .runs
        .iter()
        .enumerate()
        .map(|(row_index, ci_run)| {
            build_ci_data_row(
                ci_run,
                &cols,
                show_durations,
                viewport_ref.selection_state(row_index, ci_focus),
            )
        })
        .collect();

    let widths = build_ci_widths(&ci_data.runs, &cols, show_durations);

    let table = Table::new(rows, widths)
        .header(header)
        .block(ci_block)
        .column_spacing(1)
        .row_highlight_style(Style::default());

    let mut table_state = TableState::default().with_selected(Some(pane.viewport().pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    pane.viewport_mut().set_scroll_offset(table_state.offset());
    pane::render_overflow_affordance(frame, area, pane.viewport());

    let _ = ctx;
}

fn render_empty_ci_block(frame: &mut Frame, title: &str, area: Rect) {
    let block = pane::empty_pane_block(title);
    frame.render_widget(block, area);
}

#[cfg(test)]
mod tests {
    use super::ci_panel_title;
    use crate::ci::CiRun;
    use crate::ci::Conclusion;
    use crate::ci::FetchStatus;
    use crate::tui::panes::CiData;
    use crate::tui::panes::CiEmptyState;

    fn ci_run(branch: &str) -> CiRun {
        CiRun {
            run_id:          1,
            created_at:      "2026-04-01T21:00:00-04:00".to_string(),
            branch:          branch.to_string(),
            url:             "https://example.com/run/1".to_string(),
            conclusion:      Conclusion::Success,
            jobs:            Vec::new(),
            wall_clock_secs: Some(17),
            commit_title:    Some("feat: add box select".to_string()),
            updated_at:      None,
            fetched:         FetchStatus::Fetched,
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
}
