use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use tui_pane::PaneFocusState;
use tui_pane::PaneFrameChrome;
use tui_pane::PaneTitleCount;
use tui_pane::PaneTitleGroup;
use tui_pane::Placed;
use tui_pane::Region;
use tui_pane::Size;
use tui_pane::ViewportOverflow;
use tui_pane::label_color;

use super::constants::SOURCE_HEADER;
use super::constants::TABLE_BOX;
use super::constants::TARGET_HEADER;
use super::constants::TARGET_LEADING_PAD;
use super::constants::TARGET_TABLE_COLUMN_SPACING;
use super::constants::TARGET_TABLE_GAP_COUNT;
use super::data::TargetEntry;
use super::data::TargetsData;
use super::pane::TargetsPane;
use crate::tui::columns;
use crate::tui::panes;
use crate::tui::render;
use crate::tui::theme_roles;

pub(super) fn render_targets_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut TargetsPane,
) -> PaneFrameChrome {
    if pane.content().is_some_and(TargetsData::has_targets) {
        let data = pane.content().cloned().unwrap_or_default();
        render_targets_with_data(frame, area, pane, &data)
    } else {
        render_empty_targets(frame, area, pane)
    }
}

fn render_empty_targets(frame: &mut Frame, area: Rect, pane: &mut TargetsPane) -> PaneFrameChrome {
    let _ = (frame, area);
    pane.viewport.clear_surface();
    pane.clear_row_rects();
    PaneFrameChrome {
        title: " No Targets ".to_string(),
        focused: false,
        ..PaneFrameChrome::default()
    }
}

/// Per-row geometry derived from the entry list and content width.
/// Computed once per render and shared between the row builder and
/// the table-widths declaration. All fields are terminal display widths.
struct Layout {
    target:   usize,
    kind:     usize,
    source:   usize,
    name_max: usize,
}

/// The targets table fills the pane body.
fn targets_region(table_rows: usize) -> Region {
    Region::stack(vec![Region::rows(table_rows, Size::Fill).header()])
}

fn render_targets_with_data(
    frame: &mut Frame,
    area: Rect,
    pane: &mut TargetsPane,
    data: &TargetsData,
) -> PaneFrameChrome {
    let pane_focus_state = pane.focus.pane_focus_state;
    let entries = panes::build_target_list_from_data(data);
    let table_len = entries.len();
    pane.viewport.set_len(table_len);
    let cursor = pane.viewport.pos();

    let targets_title = build_targets_title(pane_focus_state, cursor, data);
    let mut chrome = PaneFrameChrome {
        title: targets_title,
        focused: matches!(pane_focus_state, PaneFocusState::Active),
        ..PaneFrameChrome::default()
    };
    let content_inner = tui_pane::frame_inner(area);

    let region = targets_region(table_len);
    let placed = region.place(content_inner, cursor, &[pane.viewport.scroll_offset()]);
    let table_box = placed[TABLE_BOX];
    pane.viewport.set_content_area(table_box.content);
    pane.viewport
        .set_viewport_rows(usize::from(table_box.content.height));

    let row_rects = render_targets_table(frame, pane, &entries, table_box, area, &mut chrome);
    pane.set_row_rects(row_rects);
    chrome
}

/// Render the targets table into its placed box: the ratatui `Table`
/// (header chrome row + data rows), the scroll-offset sync against the
/// pane's viewport, and the pager on the pane's bottom border.
/// Returns the visible data rows' hit-test rects.
fn render_targets_table(
    frame: &mut Frame,
    pane: &mut TargetsPane,
    entries: &[TargetEntry],
    table_box: Placed,
    pane_area: Rect,
    chrome: &mut PaneFrameChrome,
) -> Vec<(Rect, usize)> {
    let pane_focus_state = pane.focus.pane_focus_state;
    let cursor = pane.viewport.pos();
    let table_len = entries.len();
    let table_area = table_box.chrome.union(table_box.content);

    let layout = compute_layout(entries, table_area.width);
    let rows = build_rows(entries, pane, pane_focus_state, &layout);
    let widths = build_widths(&layout);
    let table = Table::new(rows, widths)
        .column_spacing(TARGET_TABLE_COLUMN_SPACING)
        .row_highlight_style(Style::default())
        .header(build_header_row());
    let mut table_state = TableState::default().with_selected(Some(cursor));
    *table_state.offset_mut() = pane.viewport.scroll_offset();
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

    let overflow = ViewportOverflow::new(table_len, table_offset, table_visible, cursor);
    chrome.labels.extend(tui_pane::overflow_affordance_label(
        pane_area,
        overflow,
        Style::default().fg(label_color()),
    ));
    row_rects
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
    tui_pane::prefixed_pane_title("Targets", &PaneTitleCount::Grouped(groups))
}

fn compute_layout(entries: &[TargetEntry], content_width: u16) -> Layout {
    let kind = panes::RunTargetKind::padded_label_width();
    let source = source_col_width_from(entries);
    let gaps = usize::from(TARGET_TABLE_COLUMN_SPACING) * TARGET_TABLE_GAP_COUNT;
    let text_budget = usize::from(content_width).saturating_sub(kind + gaps);
    let source = source.min(text_budget);
    let target = text_budget.saturating_sub(source);
    Layout {
        target,
        kind,
        source,
        name_max: target.saturating_sub(TARGET_LEADING_PAD),
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
        Cell::from(source_label).style(Style::default().fg(label_color())),
        Cell::from(
            Line::from(format!("{} ", entry.run_target_kind.label())).alignment(Alignment::Right),
        )
        .style(Style::default().fg(entry.run_target_kind.color())),
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
        Constraint::Length(u16::try_from(layout.target).unwrap_or(u16::MAX)),
        Constraint::Length(u16::try_from(layout.source).unwrap_or(u16::MAX)),
        Constraint::Length(u16::try_from(layout.kind).unwrap_or(u16::MAX)),
    ]
}

fn build_header_row() -> Row<'static> {
    let header_style = Style::default().fg(theme_roles::column_header_color());
    Row::new(vec![
        Cell::from(Span::styled(format!(" {TARGET_HEADER}"), header_style)),
        Cell::from(Span::styled(SOURCE_HEADER, header_style)),
        Cell::from(Line::from(Span::styled("Kind ", header_style)).alignment(Alignment::Right)),
    ])
    .height(1)
}

/// Width of the Target column: the longest visible target label plus the
/// leading pad, or the header if it is wider.
#[cfg(test)]
fn target_col_width_from(entries: &[TargetEntry]) -> usize {
    let max_entry_width = entries
        .iter()
        .map(|entry| columns::display_width(&entry.display_name))
        .max()
        .unwrap_or(0);
    TARGET_LEADING_PAD + max_entry_width.max(columns::display_width(TARGET_HEADER))
}

/// Width of the Source column: the longest label among the entries, or
/// the header text if it is wider.
fn source_col_width_from(entries: &[TargetEntry]) -> usize {
    let max_entry_width = entries
        .iter()
        .map(|entry| columns::display_width(entry.source.label()))
        .max()
        .unwrap_or(0);
    max_entry_width.max(columns::display_width(SOURCE_HEADER))
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::TABLE_BOX;
    use super::TARGET_LEADING_PAD;
    use super::TARGET_TABLE_COLUMN_SPACING;
    use super::TARGET_TABLE_GAP_COUNT;
    use super::compute_layout;
    use super::source_col_width_from;
    use super::target_col_width_from;
    use super::targets_region;
    use crate::project::AbsolutePath;
    use crate::tui::panes::RunTargetKind;
    use crate::tui::panes::TargetEntry;
    use crate::tui::panes::TargetSource;

    fn entry(display_name: &str, source_label: &str) -> TargetEntry {
        TargetEntry {
            name:              display_name.to_string(),
            display_name:      display_name.to_string(),
            run_target_kind:   RunTargetKind::Example,
            source:            TargetSource::worktree(source_label.to_string()),
            project_path:      AbsolutePath::from("/tmp/demo"),
            package_name:      "demo".to_string(),
            src_path:          AbsolutePath::from(format!("/tmp/demo/examples/{display_name}.rs")),
            required_features: Vec::new(),
        }
    }

    /// Chrome rows the table box reserves: the ratatui `Table` header row.
    const TABLE_CHROME: u16 = 1;

    #[test]
    fn table_fills_pane_body() {
        let region = targets_region(5);
        assert_eq!(region.total_selectable(), 5);
        let area = Rect {
            x:      0,
            y:      0,
            width:  60,
            height: 20,
        };
        let placed = region.place(area, 0, &[0]);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[TABLE_BOX].chrome.height, TABLE_CHROME);
        assert_eq!(placed[TABLE_BOX].content.height, 20 - TABLE_CHROME);
        assert_eq!(placed[TABLE_BOX].footer.height, 0);
    }

    #[test]
    fn target_table_anchors_source_and_kind_to_right_edge_when_roomy() {
        let entries = vec![
            entry("cascade", "bevy_hana/fake_widgets"),
            entry("two_window_panels", "bevy_hana/bevy_lagrange"),
        ];
        let content_width = 80;
        let kind = RunTargetKind::padded_label_width();
        let gaps = usize::from(TARGET_TABLE_COLUMN_SPACING) * TARGET_TABLE_GAP_COUNT;
        let source = source_col_width_from(&entries);

        let layout = compute_layout(&entries, content_width);

        assert!(layout.target > target_col_width_from(&entries));
        assert_eq!(layout.source, source);
        assert_eq!(
            layout.target + layout.source + layout.kind + gaps,
            usize::from(content_width)
        );
        assert_eq!(layout.kind, kind);
        assert_eq!(
            layout.name_max,
            layout.target.saturating_sub(TARGET_LEADING_PAD)
        );
    }

    #[test]
    fn target_table_shrinks_target_before_source_when_narrow() {
        let entries = vec![
            entry(
                "long_target_name_that_will_not_fit",
                "bevy_hana/fake_widgets",
            ),
            entry("short", "bevy_hana/bevy_lagrange"),
        ];
        let kind = RunTargetKind::padded_label_width();
        let gaps = usize::from(TARGET_TABLE_COLUMN_SPACING) * TARGET_TABLE_GAP_COUNT;
        let source = source_col_width_from(&entries);
        let target_budget = target_col_width_from(&entries).saturating_sub(6);
        let content_width = u16::try_from(kind + gaps + source + target_budget).unwrap_or(u16::MAX);

        let layout = compute_layout(&entries, content_width);

        assert_eq!(layout.source, source);
        assert_eq!(layout.target, target_budget);
        assert_eq!(
            layout.name_max,
            target_budget.saturating_sub(TARGET_LEADING_PAD)
        );
    }
}
