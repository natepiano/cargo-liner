use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use tui_pane::PaneChrome;
use tui_pane::PaneFocusState;
use tui_pane::PaneRule;
use tui_pane::PaneSelectionState;
use tui_pane::Placed;
use tui_pane::Region;
use tui_pane::RuleTitle;
use tui_pane::Size;
use tui_pane::Viewport;
use tui_pane::ViewportOverflow;
use tui_pane::accent_color;
use tui_pane::error_color;
use tui_pane::inactive_border_color;
use tui_pane::keep_visible_scroll_offset;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::secondary_text_color;
use tui_pane::success_color;
use tui_pane::text_default;
use tui_pane::title_color;
use tui_pane::warning_color;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use super::CiDisplay;
use super::DescriptionBlock;
use super::DetailField;
use super::EmptyDescriptionBehavior;
use super::LintDisplay;
use super::PackageData;
use super::PackageRow;
use super::SyncedDescriptionHeight;
use super::constants::CRATES_IO_TITLE;
use super::constants::DESCRIPTION_BOX;
use super::constants::METADATA_BOX;
use super::constants::MIN_METADATA_WIDTH;
use super::constants::MIN_STATS_LABEL_WIDTH;
use super::constants::STATS_TITLE;
use super::constants::TESTS_IGNORED_LABEL;
use super::constants::TESTS_TITLE;
use super::constants::TESTS_TOTAL_LABEL;
use super::pane_data::CRATES_IO_UNREACHABLE;
use super::pane_data::PackageSection;
use super::pane_impls::PackagePane;
use crate::constants::LINT_NO_LOG;
use crate::lint::LintStatus;
use crate::tui::integration;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::panes;
use crate::tui::render;

/// Shared style constants for pane rendering.
pub struct RenderStyles {
    pub readonly_label: Style,
    pub chrome:         PaneChrome,
}

struct PackageRenderCtx<'a> {
    data:              &'a PackageData,
    rows:              &'a [PackageRow],
    pane:              &'a Viewport,
    focus:             PaneFocusState,
    styles:            &'a RenderStyles,
    /// Threaded through so the Lint row can frame its icon at
    /// render time (the typed `LintDisplay` carries an unframed
    /// `LintStatus`).
    animation_elapsed: Duration,
    lint_enabled:      bool,
}

struct PackageRenderLayout {
    scroll_offset: usize,
    row_rects:     Vec<(Rect, usize)>,
}

struct PackageFieldRender {
    field:       DetailField,
    label:       &'static str,
    label_width: usize,
    area_width:  usize,
    label_style: Style,
    value_style: Style,
    value:       String,
}

struct StatsColumnRender<'a> {
    data:         &'a PackageData,
    rows:         &'a [PackageRow],
    pane:         &'a Viewport,
    focus:        PaneFocusState,
    value_width:  u16,
    border_style: Style,
    /// Style for the `Tests` / `crates.io` sub-section title rules, matching
    /// the `Structure` title (chrome title style for the current focus).
    title_style:  Style,
    /// Tests-box row the cursor sits on, when it is on a Tests row — anchors
    /// the Tests pager.
    tests_cursor: Option<usize>,
}

type FieldWrapFn = fn(&str, usize) -> Vec<String>;

/// Leaf indices of the stats-column sections in the package tree's
/// flattened-leaf order. The description and metadata boxes are always
/// leaves [`DESCRIPTION_BOX`] and [`METADATA_BOX`]; the sections follow, top
/// to bottom, for the sections with rendered rows.
struct PackageBoxes {
    structure: Option<usize>,
    tests:     Option<usize>,
    crates_io: Option<usize>,
}

impl PackageBoxes {
    /// Leaf index of the stats column's top box; `None` when the pane has no
    /// stats column.
    const fn first(&self) -> Option<usize> {
        match (self.structure, self.tests, self.crates_io) {
            (Some(index), _, _) | (None, Some(index), _) | (None, None, Some(index)) => Some(index),
            (None, None, None) => None,
        }
    }

    /// Number of leaves in the tree (the two fixed boxes plus the present
    /// sections), sizing the `prior_offsets` slice.
    fn leaf_count(&self) -> usize {
        METADATA_BOX
            + 1
            + usize::from(self.structure.is_some())
            + usize::from(self.tests.is_some())
            + usize::from(self.crates_io.is_some())
    }
}

/// The package pane's layout tree and its section leaf indices: the
/// description block on top, then the metadata column beside the stats
/// column (Structure pinned, Tests scrolling, crates.io below). Sections
/// without rendered rows are omitted; the box that takes the column's
/// leftover room is Tests when present, otherwise the last present section
/// (whose rows render from the top of the leftover, so it draws the same as
/// a pinned box). Both columns' top boxes reserve one chrome row for the
/// shared separator rule when `separator` is `1`.
fn package_region(
    data: &PackageData,
    rows: &[PackageRow],
    description_lines: u16,
    separator: u16,
    stats_width: u16,
) -> (Region, PackageBoxes) {
    let metadata_count = rows
        .iter()
        .filter(|row| matches!(row, PackageRow::Field(_) | PackageRow::Section(_)))
        .count();
    let description = Region::rows(1, Size::Fixed).lines(description_lines);
    let mut metadata = Region::rows(metadata_count, Size::Fill);
    if separator > 0 {
        metadata = metadata.rule();
    }
    let mut boxes = PackageBoxes {
        structure: None,
        tests:     None,
        crates_io: None,
    };
    if !has_stats_column(data) {
        return (Region::stack(vec![description, metadata]), boxes);
    }

    let structure_count = data.stats_rows.len();
    let tests_count = data.test_rows.len();
    // The crates.io section can render rows that have no selectable
    // counterpart in the flat row list (worktree-summary data), so its box
    // keeps the flat count for the cursor mapping and reserves the rendered
    // rows through `lines`.
    let crates_io_selectable = rows
        .iter()
        .filter(|row| matches!(row, PackageRow::CratesIo(_)))
        .count();
    let crates_io_lines = data.crates_io_rows.len();

    let mut sections: Vec<Region> = Vec::new();
    if structure_count > 0 {
        let size = if tests_count > 0 || crates_io_lines > 0 {
            Size::Fixed
        } else {
            Size::Fill
        };
        let mut structure = Region::rows(structure_count, size);
        if separator > 0 {
            structure = structure.rule();
        }
        boxes.structure = Some(METADATA_BOX + 1 + sections.len());
        sections.push(structure);
    }
    if tests_count > 0 {
        let mut tests = Region::rows(tests_count, Size::Fill);
        if separator > 0 && sections.is_empty() {
            tests = tests.rule();
        }
        if structure_count > 0 {
            tests = tests.spacer();
        }
        boxes.tests = Some(METADATA_BOX + 1 + sections.len());
        sections.push(tests.rule());
    }
    if crates_io_lines > 0 {
        let size = if tests_count > 0 {
            Size::Fixed
        } else {
            Size::Fill
        };
        let mut crates_io = Region::rows(crates_io_selectable, size)
            .lines(u16::try_from(crates_io_lines).unwrap_or(u16::MAX));
        if separator > 0 && sections.is_empty() {
            crates_io = crates_io.rule();
        }
        if structure_count > 0 || tests_count > 0 {
            crates_io = crates_io.spacer();
        }
        boxes.crates_io = Some(METADATA_BOX + 1 + sections.len());
        sections.push(crates_io.rule());
    }

    let region = Region::stack(vec![
        description,
        Region::columns(vec![
            (Constraint::Min(MIN_METADATA_WIDTH), metadata),
            (Constraint::Length(stats_width), Region::stack(sections)),
        ]),
    ]);
    (region, boxes)
}

/// True when any stats-column section (Structure / Tests / crates.io)
/// has rows, so the right-hand column should render at all.
const fn has_stats_column(data: &PackageData) -> bool {
    !data.stats_rows.is_empty() || !data.test_rows.is_empty() || !data.crates_io_rows.is_empty()
}

/// Compute the fixed stats column width across all three stat sections
/// (Structure + Tests + crates.io). Returns `(total_width, value_width)`.
pub fn stats_column_width(data: &PackageData) -> (u16, u16) {
    let max_count = data
        .stats_rows
        .iter()
        .chain(&data.test_rows)
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);
    let digit_width: u16 = match max_count {
        0..1000 => 3,
        1000..10_000 => 4,
        10_000..100_000 => 5,
        _ => 6,
    };
    // The crates.io section stores string values (versions, formatted
    // download counts) that can be wider than any count; widen the value
    // field so all three sections share one right edge.
    let widest_crates_io = data
        .crates_io_rows
        .iter()
        .map(|(_, value)| value.as_str().width())
        .max()
        .unwrap_or(0);
    let value_width = digit_width.max(u16::try_from(widest_crates_io).unwrap_or(u16::MAX));
    let label_width = stats_label_width(data);
    let total = 1 + 1 + label_width + 1 + value_width + 1;
    (total, value_width)
}

/// Width of the label field shared by all stat sections, so the
/// right-aligned values line up on a single right edge across Structure,
/// Tests, and crates.io.
fn stats_label_width(data: &PackageData) -> u16 {
    let count_labels = data
        .stats_rows
        .iter()
        .chain(&data.test_rows)
        .map(|(label, _)| label.width());
    let crates_io_labels = data.crates_io_rows.iter().map(|(label, _)| label.width());
    let widest = count_labels.chain(crates_io_labels).max().unwrap_or(0);
    u16::try_from(widest)
        .unwrap_or(u16::MAX)
        .max(MIN_STATS_LABEL_WIDTH)
}

/// Rows the Package pane's lower (metadata) content occupies, below the About
/// section: the taller of the left metadata column (every row that is not the
/// Description / Structure / Tests band) and the right stats column. Shared by
/// the render path and the cross-project top-row height measurement so the
/// predicted height matches what renders.
pub(super) fn package_lower_metadata_height(data: &PackageData, rows: &[PackageRow]) -> usize {
    let metadata_line_count = rows
        .iter()
        .filter(|row| {
            !matches!(
                row,
                PackageRow::Description | PackageRow::Structure(_) | PackageRow::Tests(_)
            )
        })
        .count();
    metadata_line_count.max(stats_column_line_count(data))
}

/// Total vertical rows the Package pane's content occupies: the synced
/// About-section description height (at least one row — the section always
/// renders, showing a placeholder when the crate has no description), the
/// separator below it, and the lower metadata block. Shared by the render
/// path (the viewport's scroll-overflow extent) and the cross-project top-row
/// measurement so the pager and the row height agree on what renders.
pub(super) fn package_content_height(
    synced_description_height: usize,
    lower_metadata_height: usize,
) -> usize {
    synced_description_height.max(1) + 1 + lower_metadata_height
}

/// Number of rendered lines in the stats column: the Structure rows, then
/// (when present) the Tests section and the crates.io section, each
/// preceded by a blank spacer when a section above it has rows, plus its
/// own title rule.
const fn stats_column_line_count(data: &PackageData) -> usize {
    let tests = if data.test_rows.is_empty() {
        0
    } else {
        let spacer = if data.stats_rows.is_empty() { 0 } else { 1 };
        spacer + 1 + data.test_rows.len()
    };
    let crates_io = if data.crates_io_rows.is_empty() {
        0
    } else {
        let spacer = if data.stats_rows.is_empty() && data.test_rows.is_empty() {
            0
        } else {
            1
        };
        spacer + 1 + data.crates_io_rows.len()
    };
    data.stats_rows.len() + tests + crates_io
}

pub fn detail_column_scroll_offset(
    focus: PaneFocusState,
    focused_output_line: usize,
    visible_height: u16,
    line_count: usize,
) -> u16 {
    if !matches!(focus, PaneFocusState::Active) {
        return 0;
    }

    let offset =
        keep_visible_scroll_offset(focused_output_line, usize::from(visible_height), line_count);
    u16::try_from(offset).unwrap_or(u16::MAX)
}

#[cfg(test)]
pub fn package_label_width(fields: &[DetailField]) -> usize {
    fields
        .iter()
        .map(|field| field.label().width())
        .max()
        .unwrap_or(0)
        .max(8)
}

fn package_row_label_width(rows: &[PackageRow]) -> usize {
    rows.iter()
        .filter_map(|row| match row {
            PackageRow::Description
            | PackageRow::Section(_)
            | PackageRow::Structure(_)
            | PackageRow::Tests(_)
            | PackageRow::CratesIo(_) => None,
            PackageRow::Field(field) => Some(field.label().width()),
        })
        .max()
        .unwrap_or(0)
        .max(8)
}

fn render_column_inner(
    frame: &mut Frame,
    ctx: &PackageRenderCtx<'_>,
    area: Rect,
) -> PackageRenderLayout {
    let rows = ctx.rows;
    let pane = ctx.pane;
    let focus = ctx.focus;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_output_line: usize = 0;
    let mut row_line_ys: Vec<(usize, usize)> = Vec::new();
    let label_width = package_row_label_width(rows);
    for (i, row) in rows.iter().enumerate() {
        match row {
            PackageRow::Description => {
                if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
                    focused_output_line = 0;
                }
            },
            PackageRow::Field(field) => {
                row_line_ys.push((i, lines.len()));
                if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
                    focused_output_line = lines.len();
                }
                let selection = tui_pane::selection_state(pane, i, focus);
                push_package_field_lines(
                    &mut lines,
                    package_field_render(ctx, *field, label_width, area.width, selection),
                );
            },
            PackageRow::Structure(_) | PackageRow::Tests(_) | PackageRow::CratesIo(_) => {
                // Structure, Tests, and crates.io rows render in the separate
                // stats column, which doesn't scroll. Anchor the metadata
                // column to its own last line rather than a position past its
                // content, so focusing a stat keeps the metadata steady
                // instead of scrolling it up.
                if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
                    focused_output_line = lines.len().saturating_sub(1);
                }
            },
            PackageRow::Section(section) => {
                // Separate the Primary Package / Workspace block from the
                // Worktree Group Summary above it with a blank row.
                if matches!(
                    section,
                    PackageSection::PrimaryPackage | PackageSection::PrimaryWorkspace
                ) {
                    lines.push(Line::default());
                }
                let style = ctx
                    .styles
                    .chrome
                    .title_style(matches!(focus, PaneFocusState::Active));
                lines.push(Line::from(Span::styled(
                    format!(" {}", section.label()),
                    style,
                )));
            },
        }
    }

    let scroll_y =
        detail_column_scroll_offset(focus, focused_output_line, area.height, lines.len());
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), area);
    let scroll_offset = usize::from(scroll_y);
    PackageRenderLayout {
        scroll_offset,
        row_rects: visible_row_rects(row_line_ys, scroll_offset, area),
    }
}

fn package_field_render(
    ctx: &PackageRenderCtx<'_>,
    field: DetailField,
    label_width: usize,
    area_width: u16,
    selection: PaneSelectionState,
) -> PackageFieldRender {
    PackageFieldRender {
        field,
        label: field.label(),
        label_width,
        area_width: usize::from(area_width),
        label_style: selection.patch(ctx.styles.readonly_label),
        value_style: selection.patch(package_field_value_style(ctx, field)),
        value: package_field_value(ctx, field),
    }
}

fn package_field_value(ctx: &PackageRenderCtx<'_>, field: DetailField) -> String {
    match field {
        DetailField::Lint => lint_display_to_string(
            &ctx.data.lint_display,
            ctx.animation_elapsed,
            ctx.lint_enabled,
        ),
        DetailField::Ci => ci_display_to_string(&ctx.data.ci_display),
        _ => field.package_value(ctx.data),
    }
}

fn package_field_value_style(ctx: &PackageRenderCtx<'_>, field: DetailField) -> Style {
    match field {
        DetailField::Ci => ci_display_style(&ctx.data.ci_display),
        DetailField::Lint => lint_display_style(&ctx.data.lint_display),
        _ => Style::default(),
    }
}

fn package_field_wrap(field: DetailField) -> Option<FieldWrapFn> {
    match field {
        DetailField::Head => Some(hard_wrap),
        _ => None,
    }
}

fn push_package_field_lines(lines: &mut Vec<Line<'static>>, render: PackageFieldRender) {
    if let Some(wrap) = package_field_wrap(render.field)
        && !render.value.is_empty()
    {
        push_wrapped_package_field_lines(lines, render, wrap);
    } else {
        push_single_package_field_line(lines, render);
    }
}

fn push_wrapped_package_field_lines(
    lines: &mut Vec<Line<'static>>,
    render: PackageFieldRender,
    wrap: FieldWrapFn,
) {
    let prefix = package_field_prefix(&render);
    let prefix_len = prefix.width();
    let avail = render.area_width.saturating_sub(prefix_len + 1);
    if avail == 0 {
        push_single_package_field_line(lines, render);
        return;
    }

    for (wrapped_index, chunk) in wrap(&render.value, avail).iter().enumerate() {
        if wrapped_index == 0 {
            lines.push(Line::from(vec![
                Span::styled(prefix.clone(), render.label_style),
                Span::styled(chunk.clone(), render.value_style),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(prefix_len)),
                Span::styled(chunk.clone(), render.value_style),
            ]));
        }
    }
}

fn push_single_package_field_line(lines: &mut Vec<Line<'static>>, render: PackageFieldRender) {
    lines.push(Line::from(vec![
        Span::styled(package_field_prefix(&render), render.label_style),
        Span::styled(render.value, render.value_style),
    ]));
}

fn package_field_prefix(render: &PackageFieldRender) -> String {
    let label = render.label;
    let label_width = render.label_width;
    format!(" {label:<label_width$} ")
}

fn visible_row_rects(
    row_line_ys: Vec<(usize, usize)>,
    scroll_offset: usize,
    area: Rect,
) -> Vec<(Rect, usize)> {
    row_line_ys
        .into_iter()
        .filter_map(|(row_index, line_y)| {
            if line_y < scroll_offset {
                return None;
            }
            let offset = line_y - scroll_offset;
            if offset >= usize::from(area.height) {
                return None;
            }
            Some((
                Rect {
                    x:      area.x,
                    y:      area
                        .y
                        .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX)),
                    width:  area.width,
                    height: 1,
                },
                row_index,
            ))
        })
        .collect()
}

struct ProjectPanelRender<'a> {
    pkg_data:                  &'a PackageData,
    rows:                      &'a [PackageRow],
    pane:                      &'a Viewport,
    focus:                     PaneFocusState,
    styles:                    &'a RenderStyles,
    border_style:              Style,
    /// Inter-pane description sync floor; clamped per-pane by the
    /// available `description_max_height`. Read by
    /// [`DescriptionBlock::render`] so the rendered content stays in
    /// step with what `sync_floor` saw at the top of the frame.
    synced_description_height: SyncedDescriptionHeight,
}

/// Body of `PackagePane::render`. Reads pane state through
/// `pane: &mut PackagePane` and the typed `PaneRenderCtx` instead
/// of the whole `App`.
pub(super) fn render_package_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut PackagePane,
    styles: &RenderStyles,
    ctx: &PaneRenderCtx<'_>,
) {
    let pane_focus_state = pane.focus.pane_focus_state;
    let PaneRenderCtx {
        animation_elapsed,
        config,
        synced_description_height,
        ..
    } = ctx;
    let lint_enabled = config.current().lint.enabled;

    let Some(pkg_data) = pane.content().cloned() else {
        render_no_project_selected(frame, area, pane);
        return;
    };

    let rows = panes::package_rows_from_data(&pkg_data);
    sync_package_viewport(pane, &pkg_data, &rows, *synced_description_height);
    let border_style = if matches!(pane_focus_state, PaneFocusState::Active) {
        styles.chrome.active_border
    } else {
        styles.chrome.inactive_border
    };
    let title = format!(" {} - {} ", pkg_data.package_title, pkg_data.title_name);
    let project_block = styles
        .chrome
        .with_inactive_border(border_style)
        .block(title, matches!(pane_focus_state, PaneFocusState::Active));
    let project_inner = project_block.inner(area);
    frame.render_widget(project_block, area);

    {
        let viewport = &mut pane.viewport;
        viewport.set_content_area(project_inner);
        viewport.set_viewport_rows(usize::from(project_inner.height));
    }
    let context = ProjectPanelRender {
        pkg_data: &pkg_data,
        rows: &rows,
        pane: &pane.viewport,
        focus: pane_focus_state,
        styles,
        border_style,
        synced_description_height: *synced_description_height,
    };

    // The description renders first; its returned height fixes the
    // description box's rendered lines for this frame's tree.
    let description_height = render_project_description(frame, &context, area, project_inner);
    let separator = u16::from(
        description_height > 0
            && project_inner.y.saturating_add(description_height) < project_inner.bottom(),
    );
    let (stats_width, value_width) = stats_column_width(&pkg_data);
    let (region, boxes) =
        package_region(&pkg_data, &rows, description_height, separator, stats_width);
    let mut prior_offsets = vec![0; boxes.leaf_count()];
    if let Some(tests) = boxes.tests {
        prior_offsets[tests] = pane.tests_scroll_offset();
    }
    let placed = region.place(project_inner, pane.viewport.pos(), &prior_offsets);

    render_separator(frame, &context, area, &placed, &boxes, separator);

    let col_ctx = PackageRenderCtx {
        data: &pkg_data,
        rows: &rows,
        pane: &pane.viewport,
        focus: pane_focus_state,
        styles,
        animation_elapsed: *animation_elapsed,
        lint_enabled,
    };
    let layout = render_column_inner(frame, &col_ctx, placed[METADATA_BOX].content);
    let mut row_rects = layout.row_rects;

    let tests_cursor = region
        .locate(pane.viewport.pos())
        .and_then(|(box_index, row)| (Some(box_index) == boxes.tests).then_some(row));
    row_rects.extend(render_stats_column(
        frame,
        &StatsColumnRender {
            data: &pkg_data,
            rows: &rows,
            pane: &pane.viewport,
            focus: pane_focus_state,
            value_width,
            border_style,
            title_style: styles
                .chrome
                .title_style(matches!(pane_focus_state, PaneFocusState::Active)),
            tests_cursor,
        },
        &placed,
        &boxes,
        project_inner,
        separator,
    ));

    pane.viewport.set_scroll_offset(layout.scroll_offset);
    pane.set_tests_scroll_offset(boxes.tests.map_or(0, |tests| placed[tests].scroll_offset));
    if description_height > 0 {
        row_rects.push((placed[DESCRIPTION_BOX].content, 0));
    }
    pane.set_row_rects(row_rects);
    render_overflow_affordance(
        frame,
        area,
        pane.viewport.overflow(),
        Style::default().fg(label_color()),
    );
}

/// Render the bordered "No project selected" placeholder and clear the
/// pane's rendered state.
fn render_no_project_selected(frame: &mut Frame, area: Rect, pane: &mut PackagePane) {
    let title_style = Style::default()
        .fg(title_color())
        .add_modifier(Modifier::BOLD);
    pane.viewport.clear_surface();
    pane.clear_row_rects();
    let empty_block = Block::default()
        .borders(Borders::ALL)
        .title(" Details ")
        .title_style(title_style);
    let content = vec![Line::from("  No project selected")];
    let detail = Paragraph::new(content).block(empty_block);
    frame.render_widget(detail, area);
}

/// Sync the pane's viewport to this frame's row list: the addressable row
/// count, the rendered content height, and a cursor nudge off any
/// non-selectable row.
fn sync_package_viewport(
    pane: &mut PackagePane,
    pkg_data: &PackageData,
    rows: &[PackageRow],
    synced_description_height: SyncedDescriptionHeight,
) {
    pane.viewport.set_len(rows.len());
    // The lower metadata and stats render in two parallel columns, so the
    // content's rendered height is the taller column plus the About section —
    // not the flat row count. Tell the viewport the rendered height so the
    // scroll pager does not page on rows that never stack vertically.
    pane.viewport.set_content_height(package_content_height(
        usize::from(synced_description_height.rows()),
        package_lower_metadata_height(pkg_data, rows),
    ));
    if !rows
        .get(pane.viewport.pos())
        .is_some_and(panes::package_row_is_selectable)
        && let Some(pos) = panes::package_nearest_selectable_row(rows, pane.viewport.pos())
    {
        pane.viewport.set_pos(pos);
    }
}

/// Render the About/description block at the top of the pane and return its
/// rendered height — the description box's `lines` override for this frame.
fn render_project_description(
    frame: &mut Frame,
    context: &ProjectPanelRender<'_>,
    area: Rect,
    project_inner: Rect,
) -> u16 {
    let lower_metadata_height = package_lower_metadata_height(context.pkg_data, context.rows);
    let reserved_lower_height = u16::from(lower_metadata_height > 0);
    let reserved_separator_height =
        u16::from(project_inner.height > reserved_lower_height.saturating_add(1));
    let baseline_max = project_inner
        .height
        .saturating_sub(reserved_lower_height.saturating_add(reserved_separator_height));
    // Build the same DescriptionBlock that `sync_floor` consumed at
    // the top of the frame and let it render — the block owns the
    // wrapped rows so the rendered content can't drift from the
    // height that fed the inter-pane sync.
    let block = DescriptionBlock::for_pane(
        context.pkg_data.description.as_deref(),
        area,
        EmptyDescriptionBehavior::ShowPlaceholder,
    );
    block.render_with_selection(
        frame,
        project_inner,
        context.synced_description_height,
        baseline_max,
        tui_pane::selection_state(context.pane, 0, context.focus),
    )
}

/// Draw the full-width separator rule between the description and the
/// columns below — titled `Structure` and teed into the stats column's
/// vertical rule when that column is present — plus the `┴` connector where
/// the vertical rule meets the pane's bottom border.
fn render_separator(
    frame: &mut Frame,
    context: &ProjectPanelRender<'_>,
    area: Rect,
    placed: &[Placed],
    boxes: &PackageBoxes,
    separator: u16,
) {
    let stats_connector_x = boxes.first().map(|index| placed[index].content.x);
    if separator > 0 {
        let rule_area = Rect {
            x:      area.x,
            y:      placed[METADATA_BOX].chrome.y,
            width:  area.width,
            height: 1,
        };
        let title = stats_connector_x.map(|_| RuleTitle {
            text:  STATS_TITLE,
            style: context
                .styles
                .chrome
                .title_style(matches!(context.focus, PaneFocusState::Active)),
        });
        tui_pane::render_horizontal_rule(
            frame,
            rule_area,
            context.border_style,
            title,
            stats_connector_x,
        );
    }
    if let Some(connector_x) = stats_connector_x {
        let first_inner_x = area.x.saturating_add(1);
        let last_inner_x = area.right().saturating_sub(2);
        if connector_x >= first_inner_x
            && connector_x <= last_inner_x
            && area.width >= 3
            && area.height > 0
        {
            tui_pane::render_rules(
                frame,
                &[PaneRule::Symbol {
                    area:  Rect {
                        x:      connector_x,
                        y:      area.bottom().saturating_sub(1),
                        width:  1,
                        height: 1,
                    },
                    glyph: '┴',
                }],
                context.border_style,
            );
        }
    }
}

/// Shared geometry and pane state for rendering one stat section
/// (Structure or Tests) inside the stats column.
struct StatSectionCtx<'a> {
    rows:        &'a [PackageRow],
    pane:        &'a Viewport,
    focus:       PaneFocusState,
    inner_x:     u16,
    inner_width: u16,
    label_width: usize,
    value_width: usize,
    area_bottom: u16,
}

/// Render the stats column from its placed boxes: the vertical rule along
/// the column's left edge, then each present section — Structure pinned at
/// the top, Tests scrolling in the middle (with its overflow affordance),
/// crates.io below — into its box's content rect. Returns the sections'
/// hit-test rects; a no-stats-column pane returns none.
fn render_stats_column(
    frame: &mut Frame,
    context: &StatsColumnRender<'_>,
    placed: &[Placed],
    boxes: &PackageBoxes,
    project_inner: Rect,
    separator: u16,
) -> Vec<(Rect, usize)> {
    let Some(first) = boxes.first() else {
        return Vec::new();
    };
    let column = placed[first].chrome;
    // Vertical rule along the column's left edge, from below the separator
    // row to the pane's bottom border.
    let rule_top = column.y.saturating_add(separator);
    tui_pane::render_rules(
        frame,
        &[PaneRule::Vertical {
            area: Rect {
                x:      column.x,
                y:      rule_top,
                width:  1,
                height: project_inner.bottom().saturating_sub(rule_top),
            },
        }],
        context.border_style,
    );

    let ctx = StatSectionCtx {
        rows:        context.rows,
        pane:        context.pane,
        focus:       context.focus,
        inner_x:     column.x.saturating_add(1),
        inner_width: column.width.saturating_sub(1),
        label_width: stats_label_width(context.data) as usize,
        value_width: context.value_width as usize,
        area_bottom: project_inner.bottom(),
    };

    let mut row_rects = Vec::new();
    if let Some(index) = boxes.structure {
        let structure_rows = count_rows_as_strings(&context.data.stats_rows);
        render_stat_section(
            frame,
            &structure_rows,
            PackageRow::Structure,
            structure_value_style,
            section_placement(placed[index]),
            &ctx,
            &mut row_rects,
        );
    }
    if let Some(index) = boxes.tests {
        render_section_rule(frame, context, placed[index].chrome, TESTS_TITLE);
        let test_rows = count_rows_as_strings(&context.data.test_rows);
        render_stat_section(
            frame,
            &test_rows,
            PackageRow::Tests,
            tests_value_style,
            section_placement(placed[index]),
            &ctx,
            &mut row_rects,
        );
        render_tests_affordance(frame, context, placed[index]);
    }
    if let Some(index) = boxes.crates_io {
        render_section_rule(frame, context, placed[index].chrome, CRATES_IO_TITLE);
        render_stat_section(
            frame,
            &context.data.crates_io_rows,
            PackageRow::CratesIo,
            crates_io_value_style,
            section_placement(placed[index]),
            &ctx,
            &mut row_rects,
        );
    }
    row_rects
}

/// Where a stat section renders, read off its placed box: the content rows
/// from the top of the box's content rect, scrolled by the box's resolved
/// offset.
const fn section_placement(placed: Placed) -> SectionPlacement {
    SectionPlacement {
        start_y:      placed.content.y,
        row_offset:   placed.scroll_offset,
        visible_rows: placed.content.height as usize,
    }
}

/// Draw a section's titled rule on the last row of its chrome rect (the
/// rows above it are blank spacers or the shared separator row). Spans one
/// column past the section so the `├` endcap tees into the column's left
/// vertical rule and the `┤` endcap tees into the pane's right border —
/// matching the full-width "Structure" rule above. Rendered after the
/// vertical rule so the left join wins.
fn render_section_rule(
    frame: &mut Frame,
    context: &StatsColumnRender<'_>,
    chrome: Rect,
    title: &'static str,
) {
    if chrome.height == 0 {
        return;
    }
    tui_pane::render_horizontal_rule(
        frame,
        Rect {
            x:      chrome.x,
            y:      chrome.bottom().saturating_sub(1),
            width:  chrome.width.saturating_add(1),
            height: 1,
        },
        context.border_style,
        Some(RuleTitle {
            text:  title,
            style: context.title_style,
        }),
        None,
    );
}

/// Draw the `▲ n of m ▼` overflow label on the Tests box when more test
/// rows exist than fit. Skipped when the box has no visible rows.
fn render_tests_affordance(frame: &mut Frame, context: &StatsColumnRender<'_>, placed: Placed) {
    let visible = usize::from(placed.content.height);
    if visible == 0 {
        return;
    }
    let cursor = context.tests_cursor.unwrap_or(placed.scroll_offset);
    render_overflow_affordance(
        frame,
        placed.content,
        ViewportOverflow::new(
            context.data.test_rows.len(),
            placed.scroll_offset,
            visible,
            cursor,
        ),
        Style::default().fg(label_color()),
    );
}

/// Value-cell style for a Structure row — the accent title color, shared
/// across every `ws` / `lib` / `bin` / … count.
fn structure_value_style(_: &str, _: &str) -> Style { Style::default().fg(title_color()) }

/// Value-cell style for a Tests row: `unit` / `integration` / `doc`
/// counts render in primary white; the unlabelled `total` renders in bold
/// accent color (matching the Languages pane footer); the `(ignored)`
/// annotation renders dimmed, since rustdoc registers but never runs it.
fn tests_value_style(label: &str, _: &str) -> Style {
    match label {
        TESTS_IGNORED_LABEL => Style::default().fg(secondary_text_color()),
        TESTS_TOTAL_LABEL => Style::default()
            .fg(title_color())
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(text_default()),
    }
}

/// Value-cell style for a crates.io row: `version` and `downloads` render
/// in primary white, the prerelease (`rc` / `beta` / …) dimmed. The
/// `unreachable` outage placeholder renders in warning color so the user
/// can tell data is missing rather than zero.
fn crates_io_value_style(label: &str, value: &str) -> Style {
    if value == CRATES_IO_UNREACHABLE {
        return Style::default().fg(warning_color());
    }
    match label {
        "version" | "downloads" => Style::default().fg(text_default()),
        _ => Style::default().fg(secondary_text_color()),
    }
}

/// Where a stat section renders: the top row, the first section-row index to
/// draw, and how many rows are visible. Structure draws every row from `0`
/// (it never overflows); the Tests band draws the slice
/// `[row_offset, row_offset + visible_rows)`.
#[derive(Clone, Copy)]
struct SectionPlacement {
    start_y:      u16,
    row_offset:   usize,
    visible_rows: usize,
}

/// Format the count-valued Structure / Tests rows into the
/// `(label, value-string)` pairs `render_stat_section` renders, so all
/// sections share one rendering path.
fn count_rows_as_strings(rows: &[(&'static str, usize)]) -> Vec<(&'static str, String)> {
    rows.iter()
        .map(|(label, count)| (*label, count.to_string()))
        .collect()
}

/// Render one stat section's visible row slice, pushing a hit-test rect for
/// each visible selectable row. Labels are left-aligned in the shared label
/// field; values are right-aligned against the column edge so all sections
/// share one right edge. Values are pre-formatted strings — counts for
/// Structure / Tests, version / download strings for crates.io.
fn render_stat_section(
    frame: &mut Frame,
    section_rows: &[(&'static str, String)],
    row_for_index: fn(usize) -> PackageRow,
    value_style: fn(&str, &str) -> Style,
    placement: SectionPlacement,
    ctx: &StatSectionCtx<'_>,
    row_rects: &mut Vec<(Rect, usize)>,
) {
    let label_style = Style::default().fg(label_color());
    let lw = ctx.label_width;
    let vw = ctx.value_width;
    let end = placement
        .row_offset
        .saturating_add(placement.visible_rows)
        .min(section_rows.len());
    let lines: Vec<Line<'_>> = (placement.row_offset..end)
        .map(|i| {
            let (label, value) = (section_rows[i].0, section_rows[i].1.as_str());
            let slot = i - placement.row_offset;
            let y_abs = placement
                .start_y
                .saturating_add(u16::try_from(slot).unwrap_or(u16::MAX));
            let target = row_for_index(i);
            let pane_index = ctx.rows.iter().position(|row| *row == target);
            let selection = pane_index.map_or(PaneSelectionState::Unselected, |index| {
                if y_abs < ctx.area_bottom {
                    row_rects.push((
                        Rect {
                            x:      ctx.inner_x,
                            y:      y_abs,
                            width:  ctx.inner_width,
                            height: 1,
                        },
                        index,
                    ));
                }
                tui_pane::selection_state(ctx.pane, index, ctx.focus)
            });
            Line::from(vec![
                Span::styled(format!(" {label:<lw$} "), selection.patch(label_style)),
                Span::styled(
                    format!("{value:>vw$} "),
                    selection.patch(value_style(label, value)),
                ),
            ])
        })
        .collect();
    let section_area = Rect {
        x:      ctx.inner_x,
        y:      placement.start_y,
        width:  ctx.inner_width,
        height: u16::try_from(end - placement.row_offset).unwrap_or(u16::MAX),
    };
    frame.render_widget(Paragraph::new(lines), section_area);
}

/// Style for the Lint row in the Package detail pane, derived
/// from the typed [`LintDisplay`].
fn lint_display_style(display: &super::LintDisplay) -> Style {
    match display {
        LintDisplay::NotRust | LintDisplay::NoRuns => Style::default().fg(inactive_border_color()),
        LintDisplay::Runs { status, .. } => match status {
            LintStatus::Passed(_) => Style::default().fg(success_color()),
            LintStatus::Failed(_) => Style::default().fg(error_color()),
            LintStatus::Running(_) | LintStatus::Stale => Style::default().fg(accent_color()),
            LintStatus::NoLog => Style::default(),
        },
    }
}

/// Render a typed [`LintDisplay`] to the string shown in the
/// Package detail row. The icon is framed at render time using
/// the current animation tick (the typed `LintDisplay` carries
/// an unframed `LintStatus` so the icon stays in sync with the
/// spinner animation).
fn lint_display_to_string(
    display: &super::LintDisplay,
    animation_elapsed: Duration,
    lint_enabled: bool,
) -> String {
    match display {
        LintDisplay::NotRust => "No lint runs — not a Rust project".to_string(),
        LintDisplay::NoRuns => "No lint runs".to_string(),
        LintDisplay::Runs { count, status } => {
            let icon = if lint_enabled {
                integration::lint_icon_for(status.kind()).frame_at(animation_elapsed)
            } else {
                LINT_NO_LOG
            };
            // A first, in-progress run has no completed count yet; show the
            // spinner alone rather than a bare "0".
            if *count == 0 {
                icon.to_string()
            } else {
                format!("{icon} {count}")
            }
        },
    }
}

/// Style for the Ci row in the Package detail pane, derived
/// from the typed [`CiDisplay`].
fn ci_display_style(display: &super::CiDisplay) -> Style {
    match display {
        CiDisplay::NoWorkflow | CiDisplay::UnpublishedBranch | CiDisplay::NoRuns => {
            Style::default().fg(inactive_border_color())
        },
        CiDisplay::Runs {
            ci_status: conclusion,
            ..
        } => render::conclusion_style(*conclusion),
    }
}

/// Render a typed [`CiDisplay`] to the string shown in the
/// Package detail row. The conclusion icon is read from
/// `CiStatus::icon()` at render time, in parallel with
/// `lint_display_to_string`.
fn ci_display_to_string(display: &super::CiDisplay) -> String {
    match display {
        CiDisplay::NoWorkflow => "No CI workflow configured".to_string(),
        CiDisplay::UnpublishedBranch => "unpublished branch".to_string(),
        CiDisplay::NoRuns => "No CI runs".to_string(),
        CiDisplay::Runs {
            ci_status: conclusion,
            local,
            github_total,
        } => {
            let icon = conclusion.map_or_else(String::new, |c| c.icon().to_string());
            let count_label = if *github_total > 0 {
                format!("local {local} / github {github_total}")
            } else if *local > 0 {
                format!("{local}")
            } else {
                String::new()
            };
            match (icon.is_empty(), count_label.is_empty()) {
                (true, true) => "No CI runs".to_string(),
                (true, false) => count_label,
                (false, true) => icon,
                (false, false) => format!("{icon} {count_label}"),
            }
        },
    }
}

pub(super) fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            if word.len() > max_width {
                result.push(word.to_string());
            } else {
                current_line.push_str(word);
            }
        } else if current_line.len() + 1 + word.len() > max_width {
            result.push(current_line);
            current_line = word.to_string();
        } else {
            current_line.push(' ');
            current_line.push_str(word);
        }
    }
    if !current_line.is_empty() {
        result.push(current_line);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

pub(super) fn hard_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    let mut result = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        // Break before a char that would overflow the column budget, but never
        // on an empty line: a single char wider than `max_width` overflows
        // onto its own line rather than splitting inside the character (which
        // byte slicing would do, panicking on a multi-byte char like `·`).
        if current_width + ch_width > max_width && !current.is_empty() {
            result.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    if !current.is_empty() {
        result.push(current);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests should fail on invalid fixtures")]
mod tests {
    use std::time::Duration;

    use chrono::DateTime;
    use ratatui::layout::Rect;
    use tui_pane::ACTIVITY_SPINNER;
    use unicode_width::UnicodeWidthStr;

    use super::PackageData;
    use super::PackageRow;
    use super::lint_display_to_string;
    use super::package_region;
    use super::stats_column_width;
    use crate::lint::LintStatus;
    use crate::tui::panes;
    use crate::tui::panes::LintDisplay;

    /// 15 Structure rows and 5 Tests rows; the flat row list is Description,
    /// the metadata fields, then the section rows.
    fn band_data() -> PackageData {
        PackageData {
            stats_rows: vec![("lib", 1); 15],
            test_rows: vec![("unit", 1); 5],
            ..PackageData::default()
        }
    }

    /// Pane inner area wide enough for both columns (`Min(20)` metadata plus
    /// the fixture's 17-wide stats column).
    fn inner(height: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 40,
            height,
        }
    }

    fn first_tests_row(rows: &[PackageRow]) -> usize {
        rows.iter()
            .position(|row| matches!(row, PackageRow::Tests(0)))
            .expect("fixture has Tests rows")
    }

    /// The Tests box's resolved scroll offset for the band fixture: a 1-line
    /// description with a separator, so a 22-row inner leaves the 5-row Tests
    /// box 3 content rows (21 column rows - 16 Structure outer - 2 chrome).
    fn placed_tests_offset(inner_height: u16, cursor: usize, prior: usize) -> usize {
        let data = band_data();
        let rows = panes::package_rows_from_data(&data);
        let (stats_width, _) = stats_column_width(&data);
        let (region, boxes) = package_region(&data, &rows, 1, 1, stats_width);
        let tests = boxes.tests.expect("fixture has a Tests box");
        let mut prior_offsets = vec![0; boxes.leaf_count()];
        prior_offsets[tests] = prior;
        region.place(inner(inner_height), cursor, &prior_offsets)[tests].scroll_offset
    }

    #[test]
    fn hard_wrap_splits_multibyte_value_without_panicking() {
        // The Branch field value carries a middle dot (`·`, two bytes); byte
        // slicing panicked when a narrow column cut inside the character.
        let lines = super::hard_wrap("main · default", 4);
        assert!(lines.iter().all(|line| line.width() <= 4));
        assert_eq!(lines.concat(), "main · default");
    }

    #[test]
    fn package_tree_maps_section_rows_to_their_boxes() {
        let mut data = band_data();
        data.crates_io_rows = vec![
            ("version", "1.0.0".to_string()),
            ("downloads", "12".to_string()),
        ];
        let rows = panes::package_rows_from_data(&data);
        let (stats_width, _) = stats_column_width(&data);
        let (region, boxes) = package_region(&data, &rows, 1, 1, stats_width);
        // The cursor walks the flat row list, so the tree must address
        // exactly those rows.
        assert_eq!(region.total_selectable(), rows.len());
        let first_tests = first_tests_row(&rows);
        assert_eq!(
            region.locate(first_tests),
            boxes.tests.map(|tests| (tests, 0))
        );
        // A crates.io row resolves to the crates.io box, not the Tests box,
        // even though both sit past the Tests rows' flat-list start.
        let first_crates = rows
            .iter()
            .position(|row| matches!(row, PackageRow::CratesIo(0)))
            .expect("fixture has crates.io rows");
        assert_eq!(
            region.locate(first_crates),
            boxes.crates_io.map(|crates_io| (crates_io, 0))
        );
    }

    #[test]
    fn tests_box_offset_tracks_cursor_on_a_test_row() {
        // Cursor on the last Tests row (box-local 4) scrolls the 3-tall box
        // to its last page.
        let rows = panes::package_rows_from_data(&band_data());
        let last_tests = first_tests_row(&rows) + 4;
        assert_eq!(placed_tests_offset(22, last_tests, 0), 2);
    }

    #[test]
    fn tests_box_offset_holds_prior_on_a_pinned_row() {
        // Cursor on a Structure row holds the prior offset, clamped to the
        // box's last page (5 rows - 3 visible).
        let rows = panes::package_rows_from_data(&band_data());
        let structure_row = first_tests_row(&rows) - 1;
        assert_eq!(placed_tests_offset(22, structure_row, 5), 2);
    }

    #[test]
    fn package_lint_row_uses_framework_activity_spinner() {
        let timestamp =
            DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("timestamp");
        let elapsed = Duration::from_millis(100);
        let display = LintDisplay::Runs {
            count:  3,
            status: LintStatus::Running(timestamp),
        };

        assert_eq!(
            lint_display_to_string(&display, elapsed, true),
            format!("{} 3", ACTIVITY_SPINNER.frame_at(elapsed))
        );
    }

    #[test]
    fn package_lint_row_omits_zero_count_during_first_run() {
        let timestamp =
            DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("timestamp");
        let elapsed = Duration::from_millis(100);
        // First run, no completed history yet: spinner only, no bare "0".
        let display = LintDisplay::Runs {
            count:  0,
            status: LintStatus::Running(timestamp),
        };

        assert_eq!(
            lint_display_to_string(&display, elapsed, true),
            ACTIVITY_SPINNER.frame_at(elapsed).to_string()
        );
    }
}
