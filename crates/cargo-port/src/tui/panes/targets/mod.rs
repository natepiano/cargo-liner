//! Targets pane render body.
//!
//! Entry: `TargetsPane::render` in `pane_impls.rs` calls
//! `render_targets_pane_body`, which delegates to the data /
//! empty branches below. The pane is two boxes: the targets table
//! (one row per target, a `Fill` box) above the Running sub-pane
//! (every running instance across all tracked workspaces, a box
//! capped at [`RUNNING_CAP_PERCENT`] of the inner height that is
//! present only while anything runs).

mod running_subpane;

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
pub use running_subpane::CargoGroup;
pub use running_subpane::RunningListRow;
use running_subpane::RunningRow;
use running_subpane::RunningSubpaneRender;
pub use running_subpane::build_running_list;
pub use running_subpane::build_running_rows;
pub use running_subpane::format_start_age;
use running_subpane::render_running_subpane;
pub use running_subpane::resolve_kill_request;
use tui_pane::PaneFocusState;
use tui_pane::PaneTitleCount;
use tui_pane::PaneTitleGroup;
use tui_pane::Placed;
use tui_pane::Region;
use tui_pane::Size;
use tui_pane::ViewportOverflow;
use tui_pane::accent_color;
use tui_pane::column_header_color;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;

use super::TargetEntry;
use super::TargetSource;
use super::TargetsData;
use super::package::RenderStyles;
use super::pane_impls::TargetsPane;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::panes;
use crate::tui::render;

/// Cap on the Source column width so a single long member name can't
/// crowd out the target name. Overflow truncates with an ellipsis.
const SOURCE_COL_MAX: usize = 24;
/// Header text for the Source column — also defines the column's
/// minimum width so the header never gets truncated.
const SOURCE_HEADER: &str = "Source";

/// Leaf index of the targets table box in the pane's tree.
const TABLE_BOX: usize = 0;
/// Leaf index of the Running box; its `Placed` entry exists only while
/// anything runs.
const RUNNING_BOX: usize = 1;
/// Ceiling on the Running box: it grows upward to at most this percent of
/// the pane's inner height, so the table keeps the rest.
const RUNNING_CAP_PERCENT: u16 = 80;
/// Chrome rows the Running box reserves: the divider rule plus the column
/// header.
const RUNNING_CHROME: u16 = 2;
/// Chrome rows the table box reserves: the ratatui `Table` header row.
const TABLE_CHROME: u16 = 1;
/// Footer row the table box reserves while the Running box sits below:
/// blank when every table row is visible, the table's pager rule when it
/// scrolls.
const TABLE_FOOTER: u16 = 1;
/// Floor on the table's data rows in degenerate heights: the Running
/// box's rendered window shrinks before the table drops below this.
const MIN_TABLE_ROWS: u16 = 3;

pub fn render_targets_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut TargetsPane,
    styles: &RenderStyles,
    ctx: &PaneRenderCtx<'_>,
) {
    // The Running list is global across all tracked workspaces, so it
    // stays visible even when the selected project has no targets — the
    // empty block renders only when there is nothing to show at all.
    let running_rows = build_running_rows(ctx.running_targets);
    let has_targets = pane.content().is_some_and(TargetsData::has_targets);
    if has_targets || !running_rows.is_empty() {
        let data = pane.content().cloned().unwrap_or_default();
        render_targets_with_data(frame, area, pane, &data, &running_rows, styles);
    } else {
        render_empty_targets(frame, area, pane);
    }
}

fn render_empty_targets(frame: &mut Frame, area: Rect, pane: &mut TargetsPane) {
    pane.viewport.clear_surface();
    pane.clear_row_rects();
    pane.set_running_cursor_pid(None);
    let empty_targets = tui_pane::empty_pane_block(" No Targets ");
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

/// The Targets pane's box tree: the table (one selectable row per target,
/// plus its header chrome row, plus its footer boundary row while the
/// Running box sits below) takes the room the Running box leaves; the
/// Running box grows upward to [`RUNNING_CAP_PERCENT`] of the inner
/// height and is omitted entirely while nothing runs. In degenerate
/// heights the rendered-lines clamp reserves the table's
/// [`MIN_TABLE_ROWS`] floor before the cap takes its share, shrinking the
/// Running window first.
fn targets_region(table_rows: usize, running_rows: usize, inner_height: u16) -> Region {
    if running_rows == 0 {
        return Region::stack(vec![Region::rows(table_rows, Size::Fill).header()]);
    }
    // An empty table has no data rows to protect — the floor only guards
    // real targets from the Running window.
    let floor = if table_rows == 0 { 0 } else { MIN_TABLE_ROWS };
    let max_lines =
        inner_height.saturating_sub(TABLE_CHROME + TABLE_FOOTER + floor + RUNNING_CHROME);
    let lines = u16::try_from(running_rows)
        .unwrap_or(u16::MAX)
        .min(max_lines);
    Region::stack(vec![
        Region::rows(table_rows, Size::Fill).header().footer(),
        Region::rows(running_rows, Size::cap(RUNNING_CAP_PERCENT))
            .rule()
            .header()
            .lines(lines),
    ])
}

/// Reconcile the highlight with this frame's Running list (D2): while it
/// sits on an instance row it follows the anchored instance's PID as rows
/// reorder; an anchored instance that exited hands the highlight to the
/// adjacent row (next, else previous), and an emptied list drops it back
/// onto the last table row. The `cargo` header row anchors by its stable
/// list position instead of a PID. Navigation and clicks re-derive the
/// anchor when the user moves the highlight.
fn sync_running_cursor(
    pane: &mut TargetsPane,
    table_len: usize,
    rows: &[RunningRow],
    list: &[RunningListRow],
) {
    let Some(local) = pane.viewport.pos().checked_sub(table_len) else {
        pane.set_running_cursor_pid(None);
        return;
    };
    if let Some(pid) = pane.running_cursor_pid()
        && let Some(index) = list
            .iter()
            .position(|row| matches!(row, RunningListRow::Instance(i) if rows[*i].pid == pid))
    {
        pane.viewport.set_pos(table_len + index);
        return;
    }
    if list.is_empty() {
        pane.viewport.set_pos(table_len.saturating_sub(1));
        pane.set_running_cursor_pid(None);
        return;
    }
    let index = local.min(list.len() - 1);
    pane.viewport.set_pos(table_len + index);
    pane.set_running_cursor_pid(match list[index] {
        RunningListRow::Instance(i) => Some(rows[i].pid),
        RunningListRow::CargoHeader { .. } => None,
    });
}

fn render_targets_with_data(
    frame: &mut Frame,
    area: Rect,
    pane: &mut TargetsPane,
    data: &TargetsData,
    running_rows: &[RunningRow],
    styles: &RenderStyles,
) {
    let focus = pane.focus.state;
    let entries = panes::build_target_list_from_data(data);
    let running_list = build_running_list(running_rows, pane.cargo_group());
    let table_len = entries.len();

    pane.viewport.set_len(table_len + running_list.len());
    sync_running_cursor(pane, table_len, running_rows, &running_list);
    let cursor = pane.viewport.pos();

    // The title's section counter tracks the table's target rows; a
    // highlight in the Running box leaves it uncounted.
    let cursor_entry = (cursor < table_len).then_some(cursor);
    let targets_title = build_targets_title(focus, cursor_entry, data);
    let targets_block = styles
        .chrome
        .block(targets_title, matches!(focus, PaneFocusState::Active));
    let content_inner = targets_block.inner(area);
    frame.render_widget(targets_block, area);

    let region = targets_region(table_len, running_list.len(), content_inner.height);
    let prior_offsets = [pane.viewport.scroll_offset(), 0];
    let placed = region.place(content_inner, cursor, &prior_offsets);
    let table_box = placed[TABLE_BOX];
    let running_visible = placed
        .get(RUNNING_BOX)
        .map_or(0, |running| usize::from(running.content.height));

    pane.viewport.set_content_area(table_box.content);
    pane.viewport
        .set_viewport_rows(usize::from(table_box.content.height) + running_visible);

    let mut row_rects = render_targets_table(frame, pane, &entries, table_box, area, styles);

    if !running_list.is_empty() {
        render_running_subpane(
            frame,
            &RunningSubpaneRender {
                rows: running_rows,
                list: &running_list,
                cargo_group: pane.cargo_group(),
                viewport: &pane.viewport,
                focus,
                table_len,
                border_style: if matches!(focus, PaneFocusState::Active) {
                    styles.chrome.active_border
                } else {
                    styles.chrome.inactive_border
                },
                title_style: styles
                    .chrome
                    .title_style(matches!(focus, PaneFocusState::Active)),
            },
            placed[RUNNING_BOX],
            area,
            &mut row_rects,
        );
    }
    pane.set_row_rects(row_rects);
}

/// Render the targets table into its placed box: the ratatui `Table`
/// (header chrome row + data rows), the scroll-offset sync against the
/// pane's viewport, and the table's pager — on the pane's bottom border
/// while the table owns the full pane, on the table's footer row once the
/// Running box sits below. Returns the visible data rows' hit-test rects.
fn render_targets_table(
    frame: &mut Frame,
    pane: &mut TargetsPane,
    entries: &[TargetEntry],
    table_box: Placed,
    pane_area: Rect,
    styles: &RenderStyles,
) -> Vec<(Rect, usize)> {
    let focus = pane.focus.state;
    let cursor = pane.viewport.pos();
    let table_len = entries.len();
    let table_area = table_box.chrome.union(table_box.content);

    let layout = compute_layout(entries, table_area.width);
    let rows = build_rows(entries, pane, focus, &layout);
    let widths = build_widths(&layout);
    let table = Table::new(rows, widths)
        .column_spacing(1)
        .row_highlight_style(Style::default())
        .header(build_header_row());
    // Feed ratatui the prior offset while the highlight is in the table so
    // its sticky scrolling is preserved; otherwise the box's re-clamped
    // prior, since with no selection ratatui leaves the offset alone.
    let mut table_state =
        TableState::default().with_selected((cursor < table_len).then_some(cursor));
    *table_state.offset_mut() = if cursor < table_len {
        pane.viewport.scroll_offset()
    } else {
        table_box.scroll_offset
    };
    frame.render_stateful_widget(table, table_area, &mut table_state);
    pane.viewport.set_scroll_offset(table_state.offset());

    let table_offset = table_state.offset();
    let table_visible = usize::from(table_box.content.height);
    let mut row_rects: Vec<(Rect, usize)> = Vec::new();
    let visible_count = table_visible.min(table_len.saturating_sub(table_offset));
    for slot in 0..visible_count {
        row_rects.push((
            Rect {
                x:      table_box.content.x,
                y:      table_box
                    .content
                    .y
                    .saturating_add(u16::try_from(slot).unwrap_or(u16::MAX)),
                width:  table_box.content.width,
                height: 1,
            },
            table_offset + slot,
        ));
    }

    let table_cursor = if cursor < table_len {
        cursor
    } else {
        table_offset
    };
    let overflow = ViewportOverflow::new(table_len, table_offset, table_visible, table_cursor);
    if table_box.footer.height == 0 {
        render_overflow_affordance(
            frame,
            pane_area,
            overflow,
            Style::default().fg(label_color()),
        );
    } else {
        render_table_footer(frame, pane, table_box.footer, pane_area, overflow, styles);
    }
    row_rects
}

/// The table's lower boundary while the Running box sits below: nothing (a
/// blank gap row) when every table row is visible, or a rule across the
/// pane with the table's pager centered on it — the same affordance every
/// pane renders on its bottom border — once it scrolls.
fn render_table_footer(
    frame: &mut Frame,
    pane: &TargetsPane,
    footer: Rect,
    pane_area: Rect,
    overflow: ViewportOverflow,
    styles: &RenderStyles,
) {
    if overflow.label().is_none() {
        return;
    }
    let rule_area = Rect {
        x:      pane_area.x,
        y:      footer.y,
        width:  pane_area.width,
        height: 1,
    };
    let active = matches!(pane.focus.state, PaneFocusState::Active);
    tui_pane::render_horizontal_rule(
        frame,
        rule_area,
        if active {
            styles.chrome.active_border
        } else {
            styles.chrome.inactive_border
        },
        None,
        None,
    );
    render_overflow_affordance(
        frame,
        rule_area,
        overflow,
        Style::default().fg(label_color()),
    );
}

fn build_targets_title(
    focus: PaneFocusState,
    cursor_entry: Option<usize>,
    data: &TargetsData,
) -> String {
    let bin_count = data.binaries.len();
    let ex_count = data.examples.len();
    let bench_count = data.benches.len();

    let focused_cursor = matches!(focus, PaneFocusState::Active)
        .then_some(cursor_entry)
        .flatten();
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
    tui_pane::prefixed_pane_title("Targets", &PaneTitleCount::Grouped(groups))
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

/// Name cell for a target row: ` <name>`, truncated.
fn name_cell(display_name: &str, name_max: usize) -> Cell<'static> {
    let display = render::truncate_with_ellipsis(display_name, name_max, "\u{2026}");
    Cell::from(format!(" {display}"))
}

/// Three-column target row: name cell + Source + Kind.
fn target_row(entry: &TargetEntry, name_cell: Cell<'static>, layout: &Layout) -> Row<'static> {
    let source_label =
        render::truncate_with_ellipsis(entry.source.label(), layout.source, "\u{2026}");
    Row::new(vec![
        name_cell,
        Cell::from(source_label).style(Style::default().fg(source_color(&entry.source))),
        Cell::from(Line::from(format!("{} ", entry.kind.label())).alignment(Alignment::Right))
            .style(Style::default().fg(entry.kind.color())),
    ])
}

fn build_rows(
    entries: &[TargetEntry],
    pane: &TargetsPane,
    focus: PaneFocusState,
    layout: &Layout,
) -> Vec<Row<'static>> {
    entries
        .iter()
        .enumerate()
        .map(|(row_index, entry)| {
            let selection = tui_pane::selection_state(&pane.viewport, row_index, focus);
            target_row(
                entry,
                name_cell(&entry.display_name, layout.name_max),
                layout,
            )
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

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::MIN_TABLE_ROWS;
    use super::RUNNING_BOX;
    use super::TABLE_BOX;
    use super::TABLE_CHROME;
    use super::targets_region;

    fn inner(height: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 60,
            height,
        }
    }

    #[test]
    fn tree_addresses_table_then_running_rows() {
        let region = targets_region(5, 3, 20);
        assert_eq!(region.total_selectable(), 8);
        // The boundary row: down past the last table row enters the
        // Running box.
        assert_eq!(region.locate(4), Some((TABLE_BOX, 4)));
        assert_eq!(region.locate(5), Some((RUNNING_BOX, 0)));
        assert_eq!(region.locate(7), Some((RUNNING_BOX, 2)));
        assert_eq!(region.locate(8), None);
    }

    #[test]
    fn without_running_rows_the_table_owns_the_pane() {
        let region = targets_region(5, 0, 20);
        assert_eq!(region.total_selectable(), 5);
        let placed = region.place(inner(20), 0, &[0]);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[TABLE_BOX].chrome.height, 1);
        assert_eq!(placed[TABLE_BOX].content.height, 19);
        // No footer either — the pager sits on the pane's bottom border.
        assert_eq!(placed[TABLE_BOX].footer.height, 0);
    }

    #[test]
    fn running_box_grows_upward_to_the_cap() {
        // 30 running rows over a 20-row inner: the lines clamp caps the
        // Running window at 13 (inner minus the table's chrome + footer +
        // floor and the Running chrome), leaving the table its floor.
        let placed = targets_region(5, 30, 20).place(inner(20), 0, &[0, 0]);
        assert_eq!(placed[RUNNING_BOX].chrome.height, 2);
        assert_eq!(placed[RUNNING_BOX].content.height, 13);
        assert_eq!(placed[TABLE_BOX].content.height, 3);
        // The table's footer boundary row sits between the boxes.
        assert_eq!(placed[TABLE_BOX].footer.height, 1);
        assert_eq!(placed[TABLE_BOX].footer.y + 1, placed[RUNNING_BOX].chrome.y);
    }

    #[test]
    fn degenerate_height_keeps_the_table_floor() {
        // An 8-row inner: 80% would give Running 6 of the 8 rows and the
        // table only 2 (header + one row). The lines clamp shrinks the
        // Running window so the table keeps its MIN_TABLE_ROWS data rows.
        let placed = targets_region(5, 30, 8).place(inner(8), 0, &[0, 0]);
        assert_eq!(placed[TABLE_BOX].content.height, MIN_TABLE_ROWS);
        assert_eq!(placed[TABLE_BOX].chrome.height, TABLE_CHROME);
        assert_eq!(placed[RUNNING_BOX].content.height, 1);
    }

    #[test]
    fn without_targets_the_running_list_keeps_the_pane() {
        // No table rows: the floor drops, so the Running window pays only
        // the table's chrome + footer rows and its own chrome.
        let placed = targets_region(0, 30, 8).place(inner(8), 0, &[0, 0]);
        assert_eq!(placed[TABLE_BOX].content.height, 0);
        assert_eq!(placed[RUNNING_BOX].content.height, 4);
    }

    #[test]
    fn running_box_pins_to_the_newest_row_while_the_cursor_is_in_the_table() {
        // 30 rows in a 13-row window: scrolled to the bottom (offset 17)
        // whenever the highlight sits in the table.
        let placed = targets_region(5, 30, 20).place(inner(20), 0, &[0, 5]);
        assert_eq!(placed[RUNNING_BOX].scroll_offset, 17);
    }
}
