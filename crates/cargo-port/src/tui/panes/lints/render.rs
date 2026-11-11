use std::ops::Range;
use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use tui_pane::ACTIVITY_SPINNER;
use tui_pane::ColorMode;
use tui_pane::PaneFocusState;
use tui_pane::PaneFrameChrome;
use tui_pane::PaneTitleCount;
use tui_pane::Viewport;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::success_color;
use tui_pane::title_color;
use tui_pane::warning_color;

use super::constants::TABLE_LEFT_MARGIN;
use super::data::LintsData;
use crate::lint::LintRun;
use crate::lint::LintRunPhase;
use crate::lint::LintRunStatus;
use crate::tui::render;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::state;
use crate::tui::state::Lint;
use crate::tui::theme_roles;

fn lints_panel_title(data: &LintsData, focused: bool, cursor: usize) -> String {
    if data.runs.is_empty() {
        let msg = if data.project_kind.is_rust() {
            "No lint runs"
        } else {
            "No lint runs — not a Rust project"
        };
        return format!(" {msg} ");
    }
    tui_pane::pane_title(
        "Lint Runs",
        &PaneTitleCount::Single {
            len:    data.runs.len(),
            cursor: focused.then_some(cursor),
        },
    )
}

/// The frame for a pane with no lint runs to list.
///
/// Reported unfocused whatever the pane's own focus is, which is the
/// dim shade `empty_pane_block` drew before the frame was shared.
fn empty_lints_chrome(title: &str) -> PaneFrameChrome {
    PaneFrameChrome {
        title: title.to_string(),
        focused: false,
        ..PaneFrameChrome::default()
    }
}

#[derive(Clone, Copy)]
enum LintRowLayout {
    Stacked,
    Compact,
    Wide,
}

impl LintRowLayout {
    const fn for_width(width: u16) -> Self {
        if width < super::constants::COMPACT_TABLE_MIN_WIDTH {
            Self::Stacked
        } else if width < super::constants::WIDE_TABLE_MIN_WIDTH {
            Self::Compact
        } else {
            Self::Wide
        }
    }

    fn row_height(self, height: u16) -> u16 {
        match self {
            Self::Stacked => height.min(super::constants::STACKED_RUN_HEIGHT),
            Self::Compact | Self::Wide => 1,
        }
    }
}

struct LintRowsCtx<'a> {
    animation_elapsed: Duration,
    viewport:          &'a Viewport,
    pane_focus_state:  PaneFocusState,
    color_mode:        ColorMode,
    layout:            LintRowLayout,
    row_height:        u16,
}

fn build_lint_rows(
    runs: &[LintRun],
    sizes: &[Option<u64>],
    phases: &[LintRunPhase],
    ctx: &LintRowsCtx<'_>,
) -> Vec<Row<'static>> {
    let date_style = Style::default()
        .fg(title_color())
        .add_modifier(Modifier::BOLD);

    let mut rows = Vec::new();
    let mut current_date = String::new();

    for (row_index, lint_run) in runs.iter().enumerate() {
        let date = super::super::format_date(&lint_run.started_at);
        let date_cell = if date == current_date {
            Cell::from("")
        } else {
            current_date.clone_from(&date);
            Cell::from(Span::styled(date.clone(), date_style))
        };

        let start_time = super::super::format_time(&lint_run.started_at);
        let end_time = lint_run
            .finished_at
            .as_deref()
            .map_or_else(|| "—".to_string(), super::super::format_time);
        let duration = super::super::format_duration(lint_run.duration_ms);
        let size = sizes
            .get(row_index)
            .copied()
            .flatten()
            .map_or_else(|| "—".to_string(), render::format_bytes);

        let (result, row_style) = lint_result(
            lint_run,
            phases
                .get(row_index)
                .copied()
                .unwrap_or(LintRunPhase::Executing),
            ctx.animation_elapsed,
        );
        let pane_selection_state =
            tui_pane::selection_state(ctx.viewport, row_index, ctx.pane_focus_state);
        if matches!(ctx.layout, LintRowLayout::Stacked) {
            rows.push(
                Row::new(vec![
                    Cell::from(Text::from(vec![
                        Line::from("Date"),
                        Line::styled(date, date_style),
                        Line::from("Duration"),
                        Line::from(duration),
                    ])),
                    Cell::from(format!("Start\n{start_time}\nSize\n{size}")),
                    Cell::from(format!("End\n{end_time}\nResult\n{result}")),
                ])
                .height(ctx.row_height)
                .style(pane_selection_state.patch(row_style, ctx.color_mode)),
            );
            continue;
        }
        rows.push(
            Row::new(vec![
                date_cell,
                Cell::from(
                    Line::from(Span::styled(start_time, Style::default().fg(label_color())))
                        .alignment(Alignment::Right),
                ),
                Cell::from(
                    Line::from(Span::styled(end_time, Style::default().fg(label_color())))
                        .alignment(Alignment::Right),
                ),
                Cell::from(
                    Line::from(Span::styled(duration, Style::default().fg(label_color())))
                        .alignment(Alignment::Right),
                ),
                Cell::from(
                    Line::from(Span::styled(size, Style::default().fg(label_color())))
                        .alignment(Alignment::Right),
                ),
                Cell::from(result),
            ])
            .style(pane_selection_state.patch(row_style, ctx.color_mode)),
        );
    }

    rows
}

fn lint_result(
    run: &LintRun,
    phase: LintRunPhase,
    animation_elapsed: Duration,
) -> (&'static str, Style) {
    match run.status {
        LintRunStatus::Running => {
            let spinner = running_lint_spinner(animation_elapsed);

            (
                spinner,
                Style::default().fg(state::lint_running_spinner_color(phase)),
            )
        },
        LintRunStatus::Passed => ("passed", Style::default().fg(success_color())),
        LintRunStatus::Failed => ("failed", Style::default().fg(error_color())),
        // `LintRunStatus::EnvUnavailable` renders "no env" instead of "failed": direnv could
        // not build the environment, so no lint command examined the code.
        LintRunStatus::EnvUnavailable => ("no env", Style::default().fg(warning_color())),
    }
}

fn running_lint_spinner(animation_elapsed: Duration) -> &'static str {
    ACTIVITY_SPINNER.frame_at(animation_elapsed)
}

/// Body of `LintsPane::render`. Same as
/// `cpu::render_cpu_pane_body`: typed parameters instead of
/// `&mut App`. Helpers above already operate on `&Viewport`.
pub fn render_lints_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut Lint,
    pane_render_ctx: &PaneRenderCtx<'_>,
) -> PaneFrameChrome {
    let mode = pane_render_ctx.color_mode;
    let Some(lints_data) = pane.content().cloned() else {
        return empty_lints_chrome(" No Lint Runs ");
    };

    let focused = pane.focus.is_focused();
    let title = lints_panel_title(&lints_data, focused, pane.viewport.pos());
    let mut pane_frame_chrome = PaneFrameChrome {
        title,
        focused: focused && !lints_data.runs.is_empty(),
        ..PaneFrameChrome::default()
    };

    let inner = tui_pane::frame_inner(area);
    let table_inner = Rect {
        x: inner.x.saturating_add(TABLE_LEFT_MARGIN),
        width: inner.width.saturating_sub(TABLE_LEFT_MARGIN),
        ..inner
    };
    let layout = LintRowLayout::for_width(table_inner.width);
    let row_height = layout.row_height(inner.height);
    {
        let viewport = &mut pane.viewport;
        viewport.set_content_area(inner);
        viewport.set_viewport_rows(usize::from(if matches!(layout, LintRowLayout::Stacked) {
            inner.height.checked_div(row_height).unwrap_or(0)
        } else {
            inner.height.saturating_sub(1)
        }));
    }

    if lints_data.runs.is_empty() {
        pane.viewport.set_len(0);
        pane.set_row_rects(Vec::new());
        return pane_frame_chrome;
    }

    let viewport_clone = pane.viewport.clone();
    let pane_focus_state = pane.focus.pane_focus_state;
    let rows = build_lint_rows(
        &lints_data.runs,
        &lints_data.sizes,
        &lints_data.phases,
        &LintRowsCtx {
            animation_elapsed: pane_render_ctx.animation_elapsed,
            viewport: &viewport_clone,
            pane_focus_state,
            color_mode: mode,
            layout,
            row_height,
        },
    );
    pane.viewport.set_len(rows.len());

    let table = lint_table(rows, layout);

    let mut table_state = TableState::default().with_selected(Some(pane.viewport.pos()));
    *table_state.offset_mut() = pane.viewport.scroll_offset();
    let header_height = u16::from(!matches!(layout, LintRowLayout::Stacked));
    let table_area = Rect {
        height: (header_height
            + u16::try_from(pane.viewport.visible_rows())
                .unwrap_or(u16::MAX)
                .saturating_mul(row_height))
        .min(inner.height),
        ..table_inner
    };
    frame.render_stateful_widget(table, table_area, &mut table_state);
    pane.viewport.set_scroll_offset(table_state.offset());
    let row_rects = lint_row_rects(
        inner,
        header_height,
        row_height,
        table_state.offset()..lints_data.runs.len(),
        pane.viewport.visible_rows(),
    );
    // The selection bar reaches the left border across the margin cell.
    for (rect, row) in &row_rects {
        theme_roles::paint_selection(
            frame,
            Rect {
                width: TABLE_LEFT_MARGIN.min(rect.width),
                ..*rect
            },
            tui_pane::selection_state(&pane.viewport, *row, pane_focus_state),
            mode,
        );
    }
    pane.set_row_rects(row_rects);
    pane_frame_chrome
        .labels
        .extend(tui_pane::overflow_affordance_label(
            area,
            pane.viewport.overflow(),
            Style::default().fg(label_color()),
        ));

    pane_frame_chrome
}

/// Full-width hit rects for the visible runs, one per run.
fn lint_row_rects(
    inner: Rect,
    header_height: u16,
    row_height: u16,
    runs: Range<usize>,
    visible_rows: usize,
) -> Vec<(Rect, usize)> {
    runs.take(visible_rows)
        .enumerate()
        .map(|(slot, row)| {
            (
                Rect::new(
                    inner.x,
                    inner.y + header_height + u16::try_from(slot).unwrap_or(u16::MAX) * row_height,
                    inner.width,
                    row_height,
                ),
                row,
            )
        })
        .collect()
}

fn lint_table(rows: Vec<Row<'static>>, layout: LintRowLayout) -> Table<'static> {
    let col_header_style = Style::default()
        .fg(theme_roles::column_header_color())
        .add_modifier(Modifier::BOLD);

    let widths = if matches!(layout, LintRowLayout::Stacked) {
        vec![
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Min(8),
        ]
    } else if matches!(layout, LintRowLayout::Compact) {
        [10, 8, 8, 8, 9, 6]
            .into_iter()
            .map(Constraint::Length)
            .collect()
    } else {
        vec![
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(9),
            Constraint::Length(8),
        ]
    };
    let table = Table::new(rows, widths);
    let table = if matches!(layout, LintRowLayout::Stacked) {
        table
    } else {
        table.header(
            Row::new(vec![
                Cell::from("Date"),
                Cell::from(Line::from("Start").alignment(Alignment::Right)),
                Cell::from(Line::from("End").alignment(Alignment::Right)),
                Cell::from(Line::from("Duration").alignment(Alignment::Right)),
                Cell::from(Line::from("Size").alignment(Alignment::Right)),
                Cell::from("Result"),
            ])
            .style(col_header_style),
        )
    };
    table
        .column_spacing(if matches!(layout, LintRowLayout::Wide) {
            2
        } else {
            1
        })
        .row_highlight_style(Style::default())
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use ratatui::layout::Position;
    use tui_pane::ColorMode;
    use tui_pane::Hittable;

    use crate::lint::LintRun;
    use crate::lint::LintRunStatus;
    use crate::tui::hit_test::HoverTarget;
    use crate::tui::panes::LintsData;
    use crate::tui::panes::PaneId;
    use crate::tui::test_support;

    #[test]
    fn lint_runs_keep_whole_headers_dates_and_values_at_each_window_width() -> anyhow::Result<()> {
        let mut app = test_support::project_app();
        app.lint.set_content(LintsData {
            runs: vec![LintRun {
                run_id:        "recorded-run".to_string(),
                started_at:    "2026-10-31T19:00:55-04:00".to_string(),
                finished_at:   Some("2026-10-31T19:02:55-04:00".to_string()),
                duration_ms:   Some(120_000),
                status:        LintRunStatus::Passed,
                commands:      Vec::new(),
                archive_bytes: 1024,
            }],
            sizes: vec![Some(1024)],
            ..LintsData::default()
        });
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            app.framework.color_mode = mode;
            for width in [60, 80, 160] {
                let buffer = test_support::draw_ui(&mut app, width)?;
                let area = app.panes.tiled_layout.area(PaneId::Lints);
                for value in [
                    "Start",
                    "End",
                    "Duration",
                    "Size",
                    "Result",
                    "2026-10-31",
                    "19:00:55",
                    "19:02:55",
                    "passed",
                ] {
                    test_support::find_text_cell(&buffer, area, value).with_context(|| {
                        format!("width {width}, {value}: {:?}", test_support::rows(&buffer))
                    })?;
                }
            }
        }
        Ok(())
    }

    #[test]
    fn lint_runs_keep_one_cell_after_the_left_border_under_a_date_header() -> anyhow::Result<()> {
        let mut app = test_support::project_app();
        app.lint.set_content(LintsData {
            runs: vec![LintRun {
                run_id:        "recorded-run".to_string(),
                started_at:    "2026-10-31T19:00:55-04:00".to_string(),
                finished_at:   Some("2026-10-31T19:02:55-04:00".to_string()),
                duration_ms:   Some(120_000),
                status:        LintRunStatus::Passed,
                commands:      Vec::new(),
                archive_bytes: 1024,
            }],
            sizes: vec![Some(1024)],
            ..LintsData::default()
        });
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            app.framework.color_mode = mode;
            for width in [60, 80, 160] {
                let buffer = test_support::draw_ui(&mut app, width)?;
                let area = app.panes.tiled_layout.area(PaneId::Lints);
                let rows = test_support::rows(&buffer);
                for text in ["Date", "2026-10-31"] {
                    let (x, _) = test_support::find_text_cell(&buffer, area, text)
                        .with_context(|| format!("width {width}, {text}: {rows:?}"))?;
                    assert_eq!(x, area.x + 2, "width {width}, {text}: {rows:?}");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn stacked_lint_runs_keep_whole_rows_and_matching_hits() -> anyhow::Result<()> {
        let mut app = test_support::project_app();
        app.lint.set_content(LintsData {
            runs: (0..6)
                .map(|index| LintRun {
                    run_id:        format!("run-{index}"),
                    started_at:    format!("2026-10-31T19:{index:02}:55-04:00"),
                    finished_at:   None,
                    duration_ms:   Some(120_000),
                    status:        LintRunStatus::Passed,
                    commands:      Vec::new(),
                    archive_bytes: 1024,
                })
                .collect(),
            sizes: vec![Some(1024); 6],
            ..LintsData::default()
        });
        app.set_focus_to_pane(PaneId::Lints);
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            app.framework.color_mode = mode;
            for (height, pages) in [(5, 6), (12, 3), (15, 2)] {
                for cursor in [0, 5] {
                    app.lint.viewport.set_len(6);
                    app.lint.viewport.set_pos(cursor);
                    let buffer =
                        test_support::draw_pane_at_size(&mut app, PaneId::Lints, 46, height)?;
                    let rows = test_support::rows(&buffer);
                    assert!(
                        rows.last()
                            .context("footer")?
                            .contains(&format!("of {pages}")),
                        "{rows:?}"
                    );
                    let selected = format!("19:{cursor:02}:55");
                    test_support::position(&buffer, &selected).context("selected run draws")?;
                    for (y, text) in rows
                        .iter()
                        .enumerate()
                        .take(usize::from(height - 1))
                        .skip(1)
                    {
                        let y = u16::try_from(y)?;
                        if text.contains("Date") {
                            let value = &rows[usize::from(y + 1)];
                            let index = (0..6)
                                .find(|index| value.contains(&format!("19:{index:02}:55")))
                                .context("run below its headers")?;
                            assert!(value.contains("2026-10-31"), "{rows:?}");
                            let visible = if height == 5 { 3 } else { 4 };
                            if visible == 4 {
                                assert!(rows[usize::from(y + 3)].contains("passed"), "{rows:?}");
                            }
                            for line in y..y + visible {
                                assert!(
                                    matches!(app.lint.hit_test_at(Position::new(2, line)), Some(HoverTarget::PaneRow { pane: PaneId::Lints, row }) if row == index)
                                );
                            }
                        } else if text.trim_matches(['│', '┃', ' ']).is_empty() {
                            assert!(app.lint.hit_test_at(Position::new(2, y)).is_none());
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
