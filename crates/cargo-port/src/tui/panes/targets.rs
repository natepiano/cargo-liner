//! Targets pane render body.
//!
//! Entry: `TargetsPane::render` in `pane_impls.rs` calls
//! `render_targets_pane_body`, which delegates to the data /
//! empty branches below.

use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use tui_pane::accent_color;
use tui_pane::column_header_color;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::success_color;

use super::TargetEntry;
use super::TargetSource;
use super::TargetsData;
use super::package::RenderStyles;
use super::pane_impls::TargetsPane;
use crate::tui::pane;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::pane::PaneTitleCount;
use crate::tui::pane::PaneTitleGroup;
use crate::tui::panes;
use crate::tui::render;
use crate::tui::running_targets::RunningKey;

/// Cap on the Source column width so a single long member name can't
/// crowd out the target name. Overflow truncates with an ellipsis.
const SOURCE_COL_MAX: usize = 24;
/// Header text for the Source column — also defines the column's
/// minimum width so the header never gets truncated.
const SOURCE_HEADER: &str = "Source";

pub fn render_targets_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut TargetsPane,
    styles: &RenderStyles,
    ctx: &PaneRenderCtx<'_>,
) {
    if let Some(data) = pane.content().cloned()
        && data.has_targets()
    {
        render_targets_with_data(frame, area, pane, &data, styles, ctx);
    } else {
        render_empty_targets(frame, area, pane);
    }
}

fn render_empty_targets(frame: &mut Frame, area: Rect, pane: &mut TargetsPane) {
    pane.viewport.clear_surface();
    let empty_targets = pane::empty_pane_block(" No Targets ");
    frame.render_widget(empty_targets, area);
}

/// Per-row geometry derived from the entry list and content width.
/// Computed once per render and shared between the row builder and
/// the table-widths declaration. All fields are character widths.
struct Layout {
    kind:     usize,
    source:   usize,
    name_max: usize,
}

fn render_targets_with_data(
    frame: &mut Frame,
    area: Rect,
    pane: &mut TargetsPane,
    data: &TargetsData,
    styles: &RenderStyles,
    ctx: &PaneRenderCtx<'_>,
) {
    let focus = pane.focus.state;
    let cursor = pane.viewport.pos();

    let targets_title = build_targets_title(focus, cursor, data);
    let targets_block = styles
        .chrome
        .block(targets_title, matches!(focus, PaneFocusState::Active));

    let running_for = |entry: &TargetEntry| {
        ctx.running_targets_dir.is_some_and(|dir| {
            ctx.running_targets.is_running(&RunningKey {
                target_dir: dir.clone(),
                kind:       entry.kind,
                name:       entry.name.clone(),
            })
        })
    };
    let entries = panes::build_target_list_from_data(data, &running_for);
    pane.viewport.set_len(entries.len());
    let content_inner = targets_block.inner(area);

    // The header row is always rendered so the pane layout stays
    // identical across selections. Data rows sit one row below the
    // block's inner area; hand the viewport only the data sub-rect so
    // `pos_to_local_row` maps screen y to the correct logical row
    // (clicking the header returns None, clicking data row N returns N).
    let data_area = Rect {
        x:      content_inner.x,
        y:      content_inner.y.saturating_add(1),
        width:  content_inner.width,
        height: content_inner.height.saturating_sub(1),
    };
    pane.viewport.set_content_area(data_area);
    pane.viewport
        .set_viewport_rows(usize::from(data_area.height));

    let layout = compute_layout(&entries, content_inner.width);
    let rows = build_rows(&entries, pane, focus, &layout, &running_for);
    let widths = build_widths(&layout);

    let table = Table::new(rows, widths)
        .block(targets_block)
        .column_spacing(1)
        .row_highlight_style(Style::default())
        .header(build_header_row());

    let mut table_state = TableState::default().with_selected(Some(cursor));
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

fn build_targets_title(focus: PaneFocusState, cursor: usize, data: &TargetsData) -> String {
    let bin_count = data.binaries.len();
    let ex_count = data.examples.len();
    let bench_count = data.benches.len();

    let focused_cursor = matches!(focus, PaneFocusState::Active).then_some(cursor);
    let section_cursor = |section_start: usize, section_len: usize| {
        focused_cursor
            .filter(|cursor| *cursor >= section_start && *cursor < section_start + section_len)
            .map(|cursor| cursor - section_start)
    };
    let mut groups = Vec::new();
    if bin_count > 0 {
        groups.push(PaneTitleGroup {
            label:  "Binary".into(),
            len:    bin_count,
            cursor: section_cursor(0, bin_count),
        });
    }
    if ex_count > 0 {
        groups.push(PaneTitleGroup {
            label:  "Examples".into(),
            len:    ex_count,
            cursor: section_cursor(bin_count, ex_count),
        });
    }
    if bench_count > 0 {
        groups.push(PaneTitleGroup {
            label:  "Benches".into(),
            len:    bench_count,
            cursor: section_cursor(bin_count + ex_count, bench_count),
        });
    }
    pane::prefixed_pane_title("Targets", &PaneTitleCount::Grouped(groups))
}

fn compute_layout(entries: &[TargetEntry], content_width: u16) -> Layout {
    let kind = panes::RunTargetKind::padded_label_width();
    let source = source_col_width_from(entries);
    let col_spacing: usize = 1;
    let leading_pad: usize = 1;
    let reserved = kind + source + (col_spacing * 2) + leading_pad;
    let name_max = (content_width as usize).saturating_sub(reserved);
    Layout {
        kind,
        source,
        name_max,
    }
}

/// Trailing marker appended to the target name when the target's process
/// is currently running. Includes a leading space so it sits one column
/// off from the name (or the ellipsis when truncated).
const RUNNING_SUFFIX: &str = " (r)";

fn build_rows<'a>(
    entries: &'a [TargetEntry],
    pane: &TargetsPane,
    focus: PaneFocusState,
    layout: &Layout,
    running_for: &dyn Fn(&TargetEntry) -> bool,
) -> Vec<Row<'a>> {
    entries
        .iter()
        .enumerate()
        .map(|(row_index, entry)| {
            let selection = pane::selection_state(&pane.viewport, row_index, focus);
            let name_cell = if running_for(entry) {
                let (visible, suffix) = render::truncate_with_suffix(
                    &entry.display_name,
                    RUNNING_SUFFIX,
                    layout.name_max,
                    "\u{2026}",
                );
                Cell::from(Line::from(vec![
                    Span::raw(format!(" {visible}")),
                    Span::styled(suffix, Style::default().fg(success_color())),
                ]))
            } else {
                let display = render::truncate_with_ellipsis(
                    &entry.display_name,
                    layout.name_max,
                    "\u{2026}",
                );
                Cell::from(format!(" {display}"))
            };
            let source_label =
                render::truncate_with_ellipsis(entry.source.label(), layout.source, "\u{2026}");
            Row::new(vec![
                name_cell,
                Cell::from(source_label).style(Style::default().fg(source_color(&entry.source))),
                Cell::from(
                    Line::from(format!("{} ", entry.kind.label())).alignment(Alignment::Right),
                )
                .style(Style::default().fg(entry.kind.color())),
            ])
            .style(selection.overlay_style())
        })
        .collect()
}

fn build_widths(layout: &Layout) -> Vec<Constraint> {
    vec![
        Constraint::Fill(1),
        Constraint::Length(u16::try_from(layout.source).unwrap_or(u16::MAX)),
        Constraint::Length(u16::try_from(layout.kind).unwrap_or(u16::MAX)),
    ]
}

fn build_header_row() -> Row<'static> {
    let header_style = Style::default().fg(column_header_color());
    Row::new(vec![
        Cell::from(Span::styled(" Target", header_style)),
        Cell::from(Span::styled(SOURCE_HEADER, header_style)),
        Cell::from(Line::from(Span::styled("Kind ", header_style)).alignment(Alignment::Right)),
    ])
    .height(1)
}

/// Width of the Source column: the longest label among the entries
/// (or the header text, whichever is wider), plus 1 for trailing pad,
/// clamped to [`SOURCE_COL_MAX`] so a runaway member name can't
/// dominate the table.
fn source_col_width_from(entries: &[TargetEntry]) -> usize {
    let max_entry_width = entries
        .iter()
        .map(|entry| entry.source.label().chars().count())
        .max()
        .unwrap_or(0);
    let header_width = SOURCE_HEADER.chars().count();
    (max_entry_width.max(header_width) + 1).min(SOURCE_COL_MAX)
}

fn source_color(source: &TargetSource) -> Color {
    match source {
        TargetSource::Workspace => accent_color(),
        TargetSource::Member(_) => label_color(),
    }
}
