use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::ColorMode;
use tui_pane::PaneFocusState;
use tui_pane::PaneFrameChrome;
use tui_pane::PaneFrameLabel;
use tui_pane::PaneSelectionState;
use tui_pane::Placed;
use tui_pane::Region;
use tui_pane::RuleTitle;
use tui_pane::Size;
use tui_pane::Viewport;
use tui_pane::ViewportOverflow;
use tui_pane::VisualRowSpan;
use tui_pane::accent_color;
use tui_pane::error_color;
use tui_pane::keep_visible_scroll_offset;
use tui_pane::label_color;
use tui_pane::secondary_text_color;
use tui_pane::success_color;
use tui_pane::text_default;
use tui_pane::title_color;
use tui_pane::warning_color;
use unicode_width::UnicodeWidthStr;

use super::PackagePane;
use super::constants::PATH_VALUE_RESERVE;
use super::constants::STATS_VALUE_WIDTH;
use crate::constants::GIT_UNREADABLE_LABEL;
use crate::constants::LINT_NO_LOG;
use crate::lint::LintStatus;
use crate::project::GithubRunCount;
use crate::project::PinnedCommit;
use crate::tui::app::ProjectMetadataState;
use crate::tui::integration;
use crate::tui::panes;
use crate::tui::panes::DescriptionBlock;
use crate::tui::panes::DetailField;
use crate::tui::panes::EmptyDescriptionBehavior;
use crate::tui::panes::PackageData;
use crate::tui::panes::PackageRow;
use crate::tui::panes::PackageVersion;
use crate::tui::panes::RenderStyles;
use crate::tui::panes::SyncedDescriptionHeight;
use crate::tui::panes::constants::CRATES_IO_TITLE;
use crate::tui::panes::constants::DESCRIPTION_BOX;
use crate::tui::panes::constants::METADATA_BOX;
use crate::tui::panes::constants::MIN_STATS_LABEL_WIDTH;
use crate::tui::panes::constants::STATS_TITLE;
use crate::tui::panes::constants::TESTS_IGNORED_LABEL;
use crate::tui::panes::constants::TESTS_TITLE;
use crate::tui::panes::constants::TESTS_TOTAL_LABEL;
use crate::tui::panes::pane_data::CRATES_IO_LABELS;
use crate::tui::panes::pane_data::CRATES_IO_UNREACHABLE;
use crate::tui::panes::pane_data::PackageDeletion;
use crate::tui::panes::pane_data::PackageSection;
use crate::tui::panes::support;
use crate::tui::panes::support::DetailSectionHeaders;
use crate::tui::render;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::state;
use crate::tui::state::CiDisplay;
use crate::tui::state::LintDisplay;
use crate::tui::theme_roles;

const MIN_TWO_COLUMN_INNER_WIDTH: u16 = 37;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LintRenderMode {
    Enabled,
    Disabled,
}

impl LintRenderMode {
    const fn is_enabled(self) -> bool { matches!(self, Self::Enabled) }
}

impl From<bool> for LintRenderMode {
    fn from(lint_enabled: bool) -> Self {
        if lint_enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

#[derive(Clone, Copy)]
enum StatisticsLayout {
    Stacked,
    Columns,
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
    lint_mode:         LintRenderMode,
    statistics_layout: StatisticsLayout,
}

struct PackageRenderLayout {
    content_height: usize,
    scroll_offset:  usize,
    row_rects:      Vec<(Rect, usize)>,
}

struct PackageFieldRender {
    field:        DetailField,
    label:        &'static str,
    label_width:  usize,
    area_width:   usize,
    label_style:  Style,
    value_style:  Style,
    value:        String,
    continuation: String,
}

struct StatsColumnRender<'a> {
    color_mode:   ColorMode,
    data:         &'a PackageData,
    rows:         &'a [PackageRow],
    pane:         &'a Viewport,
    focus:        PaneFocusState,
    value_width:  u16,
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
    const fn statistics_layout(&self) -> StatisticsLayout {
        if self.structure.is_none() && self.tests.is_none() && self.crates_io.is_none() {
            StatisticsLayout::Stacked
        } else {
            StatisticsLayout::Columns
        }
    }

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
        .filter(|row| {
            stats_width == 0 || matches!(row, PackageRow::Field(_) | PackageRow::Section(_))
        })
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
    if !has_stats_column(data) || stats_width == 0 {
        return (Region::stack(vec![description, metadata]), boxes);
    }

    let structure_count = data.stats_rows.len();
    let tests_count = data.test_rows.len();
    // `crates_io_selectable` counts flat `PackageRow::CratesIo` entries for cursor mapping, while
    // `crates_io_lines` reserves rendered rows via `Region::lines`: worktree-summary data may
    // render crates.io rows with no selectable counterpart.
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
            (Constraint::Fill(1), metadata),
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
pub(super) fn stats_column_width(data: &PackageData) -> (u16, u16) {
    let max_count = data
        .stats_rows
        .iter()
        .chain(&data.test_rows)
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);
    let value_width = u16::try_from(max_count.to_string().len())
        .unwrap_or(u16::MAX)
        .max(STATS_VALUE_WIDTH);
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
    let crates_io_labels = CRATES_IO_LABELS
        .into_iter()
        .map(unicode_width::UnicodeWidthStr::width)
        .chain(data.crates_io_rows.iter().map(|(label, _)| label.width()));
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
pub(in crate::tui::panes) fn package_lower_metadata_height(
    data: &PackageData,
    rows: &[PackageRow],
) -> usize {
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
pub(in crate::tui::panes) fn package_content_height(
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

fn package_disk_value_width(ctx: &PackageRenderCtx<'_>) -> usize {
    ctx.rows
        .iter()
        .filter_map(|row| match row {
            PackageRow::Field(
                field @ (DetailField::Disk
                | DetailField::DiskTarget
                | DetailField::DiskNonTarget
                | DetailField::DiskOutOfTreeTarget),
            ) => Some(package_field_value(ctx, *field).width()),
            _ => None,
        })
        .max()
        .unwrap_or(0)
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
    let mut field_spans = Vec::new();
    let mut header_starts = Vec::new();
    let mut repeated_headers = Vec::new();
    let label_width = package_row_label_width(rows).min(usize::from(area.width).saturating_sub(3));
    for (i, row) in rows.iter().enumerate() {
        match row {
            PackageRow::Description => {
                if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
                    focused_output_line = 0;
                }
            },
            PackageRow::Field(field) => {
                let row_start = lines.len();
                if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
                    focused_output_line = lines.len();
                }
                let selection = tui_pane::selection_state(pane, i, focus);
                push_package_field_lines(
                    &mut lines,
                    package_field_render(ctx, *field, label_width, area.width, selection),
                );
                field_spans.push(VisualRowSpan {
                    start_y: row_start,
                    height:  lines.len() - row_start,
                });
                row_line_ys.extend((row_start..lines.len()).map(|y| (i, y)));
            },
            PackageRow::Structure(_) | PackageRow::Tests(_) | PackageRow::CratesIo(_) => {
                if matches!(ctx.statistics_layout, StatisticsLayout::Stacked) {
                    if matches!(
                        row,
                        PackageRow::Structure(0) | PackageRow::Tests(0) | PackageRow::CratesIo(0)
                    ) {
                        header_starts.push(lines.len());
                    }
                    append_inline_stat(&mut lines, ctx, *row, area.width);
                    let start = lines.len().saturating_sub(1);
                    if let Some(&start_y) = header_starts.last()
                        && start_y < start
                        && matches!(
                            row,
                            PackageRow::Structure(0)
                                | PackageRow::Tests(0)
                                | PackageRow::CratesIo(0)
                        )
                    {
                        repeated_headers.push(VisualRowSpan {
                            start_y,
                            height: start - start_y,
                        });
                    }
                    field_spans.push(VisualRowSpan {
                        start_y: start,
                        height:  1,
                    });
                    row_line_ys.push((i, start));
                    if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
                        focused_output_line = start;
                    }
                } else if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
                    focused_output_line = lines.len().saturating_sub(1);
                }
            },
            PackageRow::Section(section) => {
                // For `PackageSection::PrimaryPackage` and `PrimaryWorkspace`, push a blank `Line`
                // to separate that block from the Worktree Group Summary above.
                if matches!(
                    section,
                    PackageSection::PrimaryPackage | PackageSection::PrimaryWorkspace
                ) {
                    lines.push(Line::default());
                }
                header_starts.push(lines.len());
                append_package_heading(&mut lines, section.label(), area.width, ctx);
            },
        }
    }

    let headers = support::detail_header_groups(&field_spans, header_starts);
    let pages = support::paginate_detail_sections(
        &mut lines,
        &field_spans,
        &DetailSectionHeaders {
            groups:   &headers,
            repeated: &repeated_headers,
        },
        focused_output_line,
        area.height,
        focus,
    );
    for (_, line_y) in &mut row_line_ys {
        *line_y = pages.line_y(*line_y);
    }
    draw_package_column(frame, lines, row_line_ys, pages.scroll_offset, area)
}

fn draw_package_column(
    frame: &mut Frame,
    lines: Vec<Line<'static>>,
    row_line_ys: Vec<(usize, usize)>,
    scroll_y: u16,
    area: Rect,
) -> PackageRenderLayout {
    let content_height = lines.len();
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), area);
    let scroll_offset = usize::from(scroll_y);
    PackageRenderLayout {
        content_height,
        scroll_offset,
        row_rects: visible_row_rects(row_line_ys, scroll_offset, area),
    }
}

fn append_inline_stat(
    lines: &mut Vec<Line<'static>>,
    ctx: &PackageRenderCtx<'_>,
    row: PackageRow,
    width: u16,
) {
    let (heading, index, label, value) = match row {
        PackageRow::Structure(index) => {
            let (label, value) = ctx.data.stats_rows[index];
            ("Structure", index, label, value.to_string())
        },
        PackageRow::Tests(index) => {
            let (label, value) = ctx.data.test_rows[index];
            ("Tests", index, label, value.to_string())
        },
        PackageRow::CratesIo(index) => {
            let (label, value) = &ctx.data.crates_io_rows[index];
            ("crates.io", index, *label, value.clone())
        },
        _ => return,
    };
    if index == 0 {
        append_package_heading(lines, heading, width, ctx);
    }
    let available = usize::from(width.saturating_sub(3));
    let label = if label.is_empty() { "Total" } else { label };
    let label_width = available.saturating_sub(value.width());
    let label = render::truncate_with_ellipsis(label, label_width, "…");
    let padding = " ".repeat(label_width.saturating_sub(label.width()));
    let text = format!(" {label}{padding} {value} ");
    lines.push(Line::from(text));
}

fn append_package_heading(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    width: u16,
    ctx: &PackageRenderCtx<'_>,
) {
    let style = ctx
        .styles
        .chrome
        .title_style(matches!(ctx.focus, PaneFocusState::Active));
    for part in support::word_wrap(title, usize::from(width.saturating_sub(2))) {
        lines.push(Line::from(Span::styled(format!(" {part}"), style)));
    }
}

fn package_field_label(
    ctx: &PackageRenderCtx<'_>,
    field: DetailField,
    label_width: usize,
) -> &'static str {
    let label = field.label();
    if !matches!(
        field,
        DetailField::DiskTarget | DetailField::DiskNonTarget | DetailField::DiskOutOfTreeTarget
    ) {
        return label;
    }
    let original_indent = label.len() - label.trim_start().len();
    let indent = ctx
        .rows
        .iter()
        .filter_map(|row| match row {
            PackageRow::Field(
                child @ (DetailField::DiskTarget
                | DetailField::DiskNonTarget
                | DetailField::DiskOutOfTreeTarget),
            ) => {
                let child_label = child.label();
                let text = child_label.trim_start();
                let available_indent = label_width.checked_sub(text.width())?;
                Some((child_label.len() - text.len()).min(available_indent))
            },
            _ => None,
        })
        .min()
        .unwrap_or(original_indent);
    &label[original_indent.saturating_sub(indent)..]
}

fn package_field_render(
    ctx: &PackageRenderCtx<'_>,
    field: DetailField,
    label_width: usize,
    area_width: u16,
    selection: PaneSelectionState,
) -> PackageFieldRender {
    let mode = ctx.styles.chrome.color_mode;
    let label_width = if matches!(
        field,
        DetailField::Disk
            | DetailField::DiskTarget
            | DetailField::DiskNonTarget
            | DetailField::DiskOutOfTreeTarget
    ) {
        label_width.min(usize::from(area_width).saturating_sub(package_disk_value_width(ctx) + 3))
    } else {
        label_width
    };
    PackageFieldRender {
        field,
        label: package_field_label(ctx, field, label_width),
        label_width,
        area_width: usize::from(area_width),
        label_style: ctx
            .styles
            .readonly_label
            .patch(theme_roles::selection_style(selection, mode)),
        value_style: package_field_value_style(ctx, field)
            .patch(theme_roles::selection_style(selection, mode)),
        value: package_field_value(ctx, field),
        continuation: if field == DetailField::Ci {
            match ctx.data.ci_display {
                CiDisplay::Runs { github_total, .. } => match github_total {
                    GithubRunCount::Loading => "gh …".to_string(),
                    GithubRunCount::Known(count) => format!("gh {count}"),
                    GithubRunCount::Unavailable => "gh ?".to_string(),
                },
                _ => String::new(),
            }
        } else {
            String::new()
        },
    }
}

fn package_field_value(ctx: &PackageRenderCtx<'_>, field: DetailField) -> String {
    match field {
        DetailField::Lint => {
            lint_display_to_string(&ctx.data.lint_display, ctx.animation_elapsed, ctx.lint_mode)
        },
        DetailField::Ci => ci_display_to_string(&ctx.data.ci_display),
        _ => field.package_value(ctx.data),
    }
}

fn package_field_value_style(ctx: &PackageRenderCtx<'_>, field: DetailField) -> Style {
    if package_field_value(ctx, field) == "—" {
        return Style::default().fg(tui_pane::dim_value_color());
    }
    match field {
        DetailField::Ci => ci_display_style(&ctx.data.ci_display),
        DetailField::Lint => lint_display_style(&ctx.data.lint_display),
        DetailField::Version => match &ctx.data.version {
            PackageVersion::Submodule(PinnedCommit::Unreadable) => {
                Style::default().fg(error_color())
            },
            PackageVersion::AwaitingMetadata
            | PackageVersion::Manifest(_)
            | PackageVersion::Submodule(PinnedCommit::Sha(_) | PinnedCommit::Absent) => {
                Style::default()
            },
        },
        _ => Style::default(),
    }
}

fn package_field_wrap(field: DetailField) -> Option<FieldWrapFn> {
    match field {
        DetailField::Head => Some(support::hard_wrap),
        _ => None,
    }
}

fn push_package_field_lines(lines: &mut Vec<Line<'static>>, render: PackageFieldRender) {
    let start = lines.len();
    let mut render = render;
    let style = render.label_style;
    if render.label.trim_start().width() > package_field_label_width(&render)
        && render.label.width() < render.area_width
    {
        lines.push(Line::from(Span::styled(
            format!(" {}", render.label),
            render.label_style,
        )));
        render.label = "";
    }
    if render.field == DetailField::Ci
        && render.area_width <= support::NARROW_FIELD_INNER_WIDTH
        && package_field_prefix(&render).width() + render.value.width() + 1 > render.area_width
    {
        lines.push(Line::from(Span::styled(
            format!(" {}", render.field.label()),
            render.label_style,
        )));
        for part in support::word_wrap(&render.value, render.area_width.saturating_sub(2)) {
            lines.push(Line::from(Span::styled(
                format!(" {part}"),
                render.value_style,
            )));
        }
    } else if let Some(wrap) = package_field_wrap(render.field)
        && !render.value.is_empty()
    {
        push_wrapped_package_field_lines(lines, &render, wrap);
    } else {
        push_single_package_field_line(lines, &render);
    }
    if !render.continuation.is_empty() {
        // Align the continuation with the rendered local label after wrapping.
        let local_indent = lines[start..].iter().rev().find_map(|line| {
            let mut indent = 0;
            for span in &line.spans {
                if let Some((prefix, _)) = span.content.split_once("local") {
                    return Some(indent + prefix.width());
                }
                indent += span.width();
            }
            None
        });
        if let Some(indent) = local_indent
            && render.continuation.width() <= render.area_width.saturating_sub(2)
        {
            let indent = indent.min(
                render
                    .area_width
                    .saturating_sub(render.continuation.width() + 1),
            );
            lines.push(Line::from(Span::styled(
                format!("{}{text}", " ".repeat(indent), text = render.continuation),
                render.value_style,
            )));
        }
    }
    for line in &mut lines[start..] {
        line.style = line.style.patch(style);
    }
}

fn push_wrapped_package_field_lines(
    lines: &mut Vec<Line<'static>>,
    render: &PackageFieldRender,
    wrap: FieldWrapFn,
) {
    let prefix = package_field_prefix(render);
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

fn push_single_package_field_line(lines: &mut Vec<Line<'static>>, render: &PackageFieldRender) {
    let separate_value = render.area_width <= support::NARROW_FIELD_INNER_WIDTH
        || matches!(
            render.field,
            DetailField::Ci
                | DetailField::Disk
                | DetailField::DiskTarget
                | DetailField::DiskNonTarget
                | DetailField::DiskOutOfTreeTarget
        );
    lines.extend(support::field_value_lines(
        package_field_prefix(render),
        &render.value,
        render.area_width,
        render.label_style,
        render.value_style,
        separate_value,
    ));
}

fn package_field_prefix(render: &PackageFieldRender) -> String {
    let label_width = package_field_label_width(render);
    let text = render.label.trim_start();
    let indent = render.label.width().saturating_sub(text.width());
    let indent = indent.min(label_width.saturating_sub(text.width()));
    let label = format!("{}{text}", " ".repeat(indent));
    let label = render::truncate_with_ellipsis(&label, label_width, "…");
    let padding = " ".repeat(label_width.saturating_sub(label.width()));
    format!(" {label}{padding} ")
}

fn package_field_label_width(render: &PackageFieldRender) -> usize {
    let label_width = render.label_width.min(render.area_width.saturating_sub(3));
    match render.field {
        DetailField::Path => label_width.min(render.area_width.saturating_sub(PATH_VALUE_RESERVE)),
        DetailField::Ci => label_width
            .min(render.area_width.saturating_sub(render.value.width() + 3))
            .max(render.label.trim().width()),
        _ => label_width,
    }
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
    /// Inter-pane description sync floor; clamped per-pane by the
    /// available `description_max_height`. Read by
    /// [`DescriptionBlock::render_with_selection`] so the rendered content stays in
    /// step with what `sync_floor` saw at the top of the frame.
    synced_description_height: SyncedDescriptionHeight,
    empty_description:         EmptyDescriptionBehavior,
}

/// Shared wrapping for the selected diagnostic and the top-pane height fit.
pub(in crate::tui::panes) fn manifest_error_lines(
    state: &ProjectMetadataState,
    pane_width: u16,
) -> Vec<String> {
    match state {
        ProjectMetadataState::Failed { first_error_line } => {
            let width = tui_pane::frame_inner(Rect::new(0, 0, pane_width, u16::MAX)).width;
            support::word_wrap(first_error_line, usize::from(width))
        },
        ProjectMetadataState::NotLoaded | ProjectMetadataState::Available => Vec::new(),
    }
}

/// Draw the diagnostic above selectable Package fields and return their area.
fn render_manifest_error_header(
    frame: &mut Frame,
    area: Rect,
    state: &ProjectMetadataState,
) -> Rect {
    let pane_inner = tui_pane::frame_inner(area);
    let diagnostic = manifest_error_lines(state, area.width);
    let diagnostic_height = u16::try_from(diagnostic.len())
        .unwrap_or(u16::MAX)
        .min(pane_inner.height.div_ceil(2))
        .min(pane_inner.height.saturating_sub(1));
    let diagnostic = render::clip_rows_with_ellipsis(
        &diagnostic,
        usize::from(diagnostic_height),
        usize::from(pane_inner.width),
    );
    frame.render_widget(
        Paragraph::new(diagnostic.into_iter().map(Line::from).collect::<Vec<_>>())
            .style(Style::default().fg(error_color())),
        Rect {
            height: diagnostic_height,
            ..pane_inner
        },
    );
    Rect {
        y: pane_inner.y.saturating_add(diagnostic_height),
        height: pane_inner.height.saturating_sub(diagnostic_height),
        ..pane_inner
    }
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
) -> PaneFrameChrome {
    let mode = ctx.color_mode;
    let pane_focus_state = pane.focus.pane_focus_state;
    let PaneRenderCtx {
        animation_elapsed,
        config,
        synced_description_height,
        ..
    } = ctx;
    let lint_mode = LintRenderMode::from(config.current().lint.enabled.is_enabled());

    let Some(pkg_data) = pane.content().cloned() else {
        return render_no_project_selected(frame, area, pane);
    };

    let rows = panes::package_rows_from_data(&pkg_data);
    sync_package_viewport(pane, &pkg_data, &rows, *synced_description_height);
    let mut chrome = package_chrome(&pkg_data, pane_focus_state);
    chrome.title = fit_package_title(&pkg_data, usize::from(area.width.saturating_sub(2)));
    let pane_inner = tui_pane::frame_inner(area);
    let metadata_state = ctx
        .selected_project_path
        .map_or(&ProjectMetadataState::NotLoaded, |path| {
            ctx.scan.manifest_metadata.for_project(path)
        });
    let project_inner = render_manifest_error_header(frame, area, metadata_state);

    // The diagnostic is fixed above the normal, selectable Package content.
    pane.viewport.set_content_area(pane_inner);
    pane.viewport
        .set_viewport_rows(usize::from(project_inner.height));
    let context = ProjectPanelRender {
        pkg_data: &pkg_data,
        rows: &rows,
        pane: &pane.viewport,
        focus: pane_focus_state,
        styles,
        synced_description_height: *synced_description_height,
        empty_description: ctx.empty_package_description,
    };

    // `render_project_description` runs first because its returned `description_height` fixes the
    // description region's rendered lines for this frame's `package_region` tree.
    let description_height =
        render_project_description(frame, &context, area, project_inner, metadata_state, mode);
    let separator = u16::from(
        description_height > 0
            && project_inner.y.saturating_add(description_height) < project_inner.bottom(),
    );
    let columns = place_package_columns(
        &pkg_data,
        &rows,
        project_inner,
        description_height,
        separator,
        pane,
    );

    push_separator(
        &context,
        area,
        &columns.placed,
        &columns.boxes,
        separator,
        &mut chrome,
    );
    select_first_visible_package_field(pane, &rows, description_height);

    let col_ctx = PackageRenderCtx {
        data: &pkg_data,
        rows: &rows,
        pane: &pane.viewport,
        focus: pane_focus_state,
        styles,
        animation_elapsed: *animation_elapsed,
        lint_mode,
        statistics_layout: columns.boxes.statistics_layout(),
    };
    let layout = render_package_columns(
        frame,
        &col_ctx,
        &columns,
        project_inner,
        separator,
        &mut chrome,
    );
    pane.viewport.set_content_height(
        usize::from(description_height + separator)
            + layout
                .content_height
                .max(stats_column_line_count(&pkg_data)),
    );
    chrome
        .labels
        .extend(package_page_label(area, &layout, &columns, &pkg_data));
    let mut row_rects = layout.row_rects;
    pane.viewport.set_scroll_offset(layout.scroll_offset);
    pane.set_tests_scroll_offset(
        columns
            .boxes
            .tests
            .map_or(0, |tests| columns.placed[tests].scroll_offset),
    );
    if description_height > 0 {
        row_rects.push((columns.placed[DESCRIPTION_BOX].content, 0));
    }
    paint_package_selection(frame, pane_inner, pane, &row_rects, mode);
    pane.set_row_rects(row_rects);
    chrome
}

fn package_page_label(
    area: Rect,
    layout: &PackageRenderLayout,
    columns: &PackageColumnLayout,
    data: &PackageData,
) -> Option<PaneFrameLabel> {
    let label_area = if columns.boxes.structure.is_some()
        || columns.boxes.tests.is_some()
        || columns.boxes.crates_io.is_some()
    {
        Rect {
            width: columns.placed[METADATA_BOX].content.width.saturating_add(1),
            ..area
        }
    } else {
        area
    };
    tui_pane::overflow_affordance_label(
        label_area,
        ViewportOverflow::for_paged_content(
            layout.content_height.max(stats_column_line_count(data)),
            layout.scroll_offset,
            usize::from(columns.placed[METADATA_BOX].content.height),
        ),
        Style::default().fg(label_color()),
    )
}

fn select_first_visible_package_field(
    pane: &mut PackagePane,
    rows: &[PackageRow],
    description_height: u16,
) {
    if description_height == 0
        && pane.viewport.pos() == 0
        && let Some(row) = rows
            .iter()
            .position(|row| matches!(row, PackageRow::Field(_)))
    {
        pane.viewport.set_pos(row);
    }
}

struct PackageColumnLayout {
    region:      Region,
    boxes:       PackageBoxes,
    placed:      Vec<Placed>,
    value_width: u16,
}

fn render_package_columns(
    frame: &mut Frame,
    ctx: &PackageRenderCtx<'_>,
    columns: &PackageColumnLayout,
    inner: Rect,
    separator: u16,
    chrome: &mut PaneFrameChrome,
) -> PackageRenderLayout {
    let mut layout = render_column_inner(frame, ctx, columns.placed[METADATA_BOX].content);
    let tests_cursor = columns
        .region
        .locate(ctx.pane.pos())
        .and_then(|(box_index, row)| (Some(box_index) == columns.boxes.tests).then_some(row));
    layout.row_rects.extend(render_stats_column(
        frame,
        &StatsColumnRender {
            color_mode: ctx.styles.chrome.color_mode,
            data: ctx.data,
            rows: ctx.rows,
            pane: ctx.pane,
            focus: ctx.focus,
            value_width: columns.value_width,
            title_style: ctx
                .styles
                .chrome
                .title_style(matches!(ctx.focus, PaneFocusState::Active)),
            tests_cursor,
        },
        &columns.placed,
        &columns.boxes,
        inner,
        separator,
        chrome,
    ));
    layout
}

fn place_package_columns(
    data: &PackageData,
    rows: &[PackageRow],
    inner: Rect,
    description_height: u16,
    separator: u16,
    pane: &PackagePane,
) -> PackageColumnLayout {
    let (stats_width, value_width) = stats_column_width(data);
    let stats_width = if inner.width >= MIN_TWO_COLUMN_INNER_WIDTH {
        stats_width
    } else {
        0
    };
    let (region, boxes) = package_region(data, rows, description_height, separator, stats_width);
    let mut offsets = vec![0; boxes.leaf_count()];
    if let Some(tests) = boxes.tests {
        offsets[tests] = pane.tests_scroll_offset();
    }
    let placed = region.place(inner, pane.viewport.pos(), &offsets);
    PackageColumnLayout {
        region,
        boxes,
        placed,
        value_width,
    }
}

fn paint_package_selection(
    frame: &mut Frame,
    pane_inner: Rect,
    pane: &PackagePane,
    row_rects: &[(Rect, usize)],
    mode: ColorMode,
) {
    for (rect, row) in row_rects {
        theme_roles::paint_selection(
            frame,
            Rect {
                x: rect.x.max(pane_inner.x),
                width: rect.width.min(pane_inner.width),
                ..*rect
            },
            tui_pane::selection_state(&pane.viewport, *row, pane.focus.pane_focus_state),
            mode,
        );
    }
}

fn fit_package_title(data: &PackageData, width: usize) -> String {
    let title = package_chrome(data, PaneFocusState::Inactive).title;
    if title.trim_end().width() <= width {
        return title;
    }
    let label = if data.title.width() < width {
        format!(" {}", data.title)
    } else {
        data.title.clone()
    };
    let available = width.saturating_sub(label.width()).saturating_sub(1);
    if available > 4 {
        format!(
            "{label} - {} ",
            render::truncate_with_ellipsis(&data.name, available - 3, "…")
        )
    } else if label.width() < width {
        format!("{label} ")
    } else {
        label
    }
}

fn package_chrome(data: &PackageData, focus: PaneFocusState) -> PaneFrameChrome {
    let deletion = if data.deletion == PackageDeletion::Deleted {
        " (deleted)"
    } else {
        ""
    };
    PaneFrameChrome {
        title: format!(" {} - {}{deletion} ", data.title, data.name),
        focused: matches!(focus, PaneFocusState::Active),
        ..PaneFrameChrome::default()
    }
}

/// Render the "No project selected" placeholder, clear the pane's
/// rendered state, and report the chrome the grid draws around it. The
/// border always takes the inactive shade, as the placeholder's own
/// block did.
fn render_no_project_selected(
    frame: &mut Frame,
    area: Rect,
    pane: &mut PackagePane,
) -> PaneFrameChrome {
    pane.viewport.clear_surface();
    pane.clear_row_rects();
    let content = vec![Line::from("  No project selected")];
    frame.render_widget(Paragraph::new(content), tui_pane::frame_inner(area));
    PaneFrameChrome {
        title: " Details ".to_string(),
        focused: false,
        ..PaneFrameChrome::default()
    }
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
    // `pane.viewport.set_content_height(package_content_height(...))` uses the taller
    // metadata/stats column plus About because the two columns render in parallel; the pager must
    // not count their rows as vertically stacked.
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
    metadata_state: &ProjectMetadataState,
    mode: ColorMode,
) -> u16 {
    let lower_metadata_height = package_lower_metadata_height(context.pkg_data, context.rows);
    let reserved_lower_height = u16::from(lower_metadata_height > 0);
    let reserved_separator_height =
        u16::from(project_inner.height > reserved_lower_height.saturating_add(1));
    let baseline_max = project_inner
        .height
        .saturating_sub(reserved_lower_height.saturating_add(reserved_separator_height));
    let (baseline_max, synced_floor) = match metadata_state {
        ProjectMetadataState::Failed { .. } => {
            // Leave one field row and its separator; the description yields,
            // including its shared height floor, even in a short pane.
            let max_description = project_inner.height.saturating_sub(2);
            (
                max_description,
                context.synced_description_height.capped_at(max_description),
            )
        },
        ProjectMetadataState::NotLoaded | ProjectMetadataState::Available => {
            (baseline_max, context.synced_description_height)
        },
    };
    // `DescriptionBlock::for_pane` rebuilds the wrapped rows used for the frame's synchronized
    // description height; `block.render_with_selection` draws those owned rows so content cannot
    // drift from the height used for inter-pane synchronization.
    let block = DescriptionBlock::for_pane(
        &context.pkg_data.description,
        area,
        context.empty_description,
    );
    block.render_with_selection(
        frame,
        project_inner,
        synced_floor,
        baseline_max,
        tui_pane::selection_state(context.pane, 0, context.focus),
        mode,
    )
}

/// Draw the full-width separator rule between the description and the
/// columns below — titled `Structure` and teed into the stats column's
/// vertical rule when that column is present — plus the `┴` connector where
/// the vertical rule meets the pane's bottom border.
fn push_separator(
    context: &ProjectPanelRender<'_>,
    area: Rect,
    placed: &[Placed],
    boxes: &PackageBoxes,
    separator: u16,
    chrome: &mut PaneFrameChrome,
) {
    if separator == 0 {
        return;
    }
    let rule_area = Rect {
        x:      area.x,
        y:      placed[METADATA_BOX].chrome.y,
        width:  area.width,
        height: 1,
    };
    chrome.rules.push(rule_area);
    // The grid resolves the stats-column junction with `chrome.rules`; only the title needs
    // placement here through `chrome.labels`.
    let Some(connector_x) = boxes.first().map(|index| placed[index].content.x) else {
        return;
    };
    let stats_rule = Rect {
        x: connector_x,
        width: rule_area.right().saturating_sub(connector_x),
        ..rule_area
    };
    chrome.labels.extend(tui_pane::rule_title_label(
        stats_rule,
        RuleTitle {
            text:  STATS_TITLE,
            style: context
                .styles
                .chrome
                .title_style(matches!(context.focus, PaneFocusState::Active)),
        },
        None,
    ));
}

/// Shared geometry and pane state for rendering one stat section
/// (Structure or Tests) inside the stats column.
struct StatSectionCtx<'a> {
    color_mode:  ColorMode,
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
    chrome: &mut PaneFrameChrome,
) -> Vec<(Rect, usize)> {
    let Some(first) = boxes.first() else {
        return Vec::new();
    };
    let column = placed[first].chrome;
    // Start on the separator, or on the top border when no separator exists.
    // Reach the bottom border so the grid draws both junctions.
    let rule_top = if separator > 0 {
        column.y
    } else {
        column.y.saturating_sub(1)
    };
    chrome.rules.push(Rect {
        x:      column.x,
        y:      rule_top,
        width:  1,
        height: project_inner
            .bottom()
            .saturating_add(1)
            .saturating_sub(rule_top),
    });

    if separator == 0 && boxes.structure.is_some() {
        chrome.title = fit_package_title(
            context.data,
            usize::from(column.x.saturating_sub(project_inner.x).saturating_sub(1)),
        );
        chrome.labels.extend(tui_pane::rule_title_label(
            Rect::new(column.x, rule_top, column.width.saturating_add(1), 1),
            RuleTitle {
                text:  STATS_TITLE,
                style: context.title_style,
            },
            None,
        ));
    }

    let stat_section_context = StatSectionCtx {
        color_mode:  context.color_mode,
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
            &stat_section_context,
            &mut row_rects,
        );
    }
    if let Some(index) = boxes.tests {
        push_section_rule(context, placed[index].chrome, TESTS_TITLE, chrome);
        let test_rows = count_rows_as_strings(&context.data.test_rows);
        render_stat_section(
            frame,
            &test_rows,
            PackageRow::Tests,
            tests_value_style,
            section_placement(placed[index]),
            &stat_section_context,
            &mut row_rects,
        );
        render_tests_affordance(frame, context, placed[index]);
    }
    if let Some(index) = boxes.crates_io {
        push_section_rule(context, placed[index].chrome, CRATES_IO_TITLE, chrome);
        render_stat_section(
            frame,
            &context.data.crates_io_rows,
            PackageRow::CratesIo,
            crates_io_value_style,
            section_placement(placed[index]),
            &stat_section_context,
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

/// Report a section's titled rule on the last row of its chrome rect (the
/// rows above it are blank spacers or the shared separator row). Spans one
/// column past the section so it meets the column's left vertical rule and
/// the pane's right border — matching the full-width "Structure" rule
/// above — and the grid pass turns both meetings into `├` and `┤`.
fn push_section_rule(
    context: &StatsColumnRender<'_>,
    section: Rect,
    title: &'static str,
    chrome: &mut PaneFrameChrome,
) {
    if section.height == 0 {
        return;
    }
    let rule_area = Rect {
        x:      section.x,
        y:      section.bottom().saturating_sub(1),
        width:  section.width.saturating_add(1),
        height: 1,
    };
    chrome.rules.push(rule_area);
    chrome.labels.extend(tui_pane::rule_title_label(
        rule_area,
        RuleTitle {
            text:  title,
            style: context.title_style,
        },
        None,
    ));
}

/// Draw the `▲ n of m ▼` overflow label on the Tests box when more test
/// rows exist than fit. Skipped when the box has no visible rows.
fn render_tests_affordance(frame: &mut Frame, context: &StatsColumnRender<'_>, placed: Placed) {
    let visible = usize::from(placed.content.height);
    if visible == 0 {
        return;
    }
    let cursor = context.tests_cursor.unwrap_or(placed.scroll_offset);
    tui_pane::render_overflow_affordance(
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
        "Version" | "Downloads" => Style::default().fg(text_default()),
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
            let (label, full_value) = (section_rows[i].0, section_rows[i].1.as_str());
            let value = render::truncate_with_ellipsis(full_value, vw, "…");
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
                Span::styled(
                    format!(" {label:<lw$} "),
                    selection.patch(label_style, ctx.color_mode),
                ),
                Span::styled(
                    format!("{value:>vw$} "),
                    selection.patch(value_style(label, full_value), ctx.color_mode),
                ),
            ])
            .style(theme_roles::selection_style(selection, ctx.color_mode))
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
fn lint_display_style(display: &LintDisplay) -> Style {
    match display {
        LintDisplay::NotRust | LintDisplay::NoRuns => {
            Style::default().fg(tui_pane::dim_value_color())
        },
        LintDisplay::Runs { status, .. } => match status {
            LintStatus::Passed(_) => Style::default().fg(success_color()),
            LintStatus::Failed(_) => Style::default().fg(error_color()),
            LintStatus::Running(_, phase) => {
                Style::default().fg(state::lint_running_spinner_color(*phase))
            },
            LintStatus::EnvUnavailable(_) => Style::default().fg(warning_color()),
            LintStatus::Stale | LintStatus::Waiting => Style::default().fg(accent_color()),
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
    display: &LintDisplay,
    animation_elapsed: Duration,
    lint_mode: LintRenderMode,
) -> String {
    match display {
        LintDisplay::NotRust => "Not a Rust project".to_string(),
        LintDisplay::NoRuns => "—".to_string(),
        LintDisplay::Runs { count, status } => {
            let icon = if lint_mode.is_enabled() {
                integration::lint_icon_for(status.kind()).frame_at(animation_elapsed)
            } else {
                LINT_NO_LOG
            };
            // When `count` on `LintDisplay::Runs` is zero during the first in-progress run, show
            // `icon` alone because there is no completed count yet, rather than a bare "0".
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
fn ci_display_style(display: &CiDisplay) -> Style {
    match display {
        CiDisplay::Unreadable => Style::default().fg(tui_pane::error_color()),
        CiDisplay::NoWorkflow | CiDisplay::UnpublishedBranch | CiDisplay::NoRuns => {
            Style::default().fg(tui_pane::dim_value_color())
        },
        CiDisplay::Runs { ci_status, .. } => render::ci_status_style(*ci_status),
    }
}

/// Render a typed [`CiDisplay`] to the string shown in the
/// Package detail row. The `ci_status` icon is read from
/// `CiStatus::icon()` at render time, in parallel with
/// `lint_display_to_string`.
fn ci_display_to_string(display: &CiDisplay) -> String {
    match display {
        CiDisplay::Unreadable => GIT_UNREADABLE_LABEL.to_string(),
        CiDisplay::NoWorkflow => "No CI".to_string(),
        CiDisplay::NoRuns => "No CI runs".to_string(),
        CiDisplay::UnpublishedBranch => "unpublished branch".to_string(),
        CiDisplay::Runs {
            ci_status, local, ..
        } => {
            let count_label = format!("local {local}");
            ci_status.map_or(count_label.clone(), |status| {
                format!("{} {count_label}", status.icon())
            })
        },
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests should fail on invalid fixtures")]
mod tests {
    use std::time::Duration;

    use chrono::DateTime;
    use ratatui::layout::Rect;
    use tui_pane::ACTIVITY_SPINNER;
    use unicode_width::UnicodeWidthStr;

    use super::accent_color;
    use super::error_color;
    use super::lint_display_style;
    use super::lint_display_to_string;
    use super::package_region;
    use super::stats_column_width;
    use crate::lint::LintRunPhase;
    use crate::lint::LintStatus;
    use crate::tui::panes;
    use crate::tui::panes::PackageData;
    use crate::tui::panes::PackageRow;
    use crate::tui::state::LintDisplay;

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
        // `support::hard_wrap` must preserve the Branch value's middle dot (`·`, two bytes): byte
        // slicing previously panicked when a narrow column cut inside that character.
        let lines = super::support::hard_wrap("main · default", 4);
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
        // The cursor traverses flat `rows`, so `region.total_selectable()` must equal `rows.len()`.
        assert_eq!(region.total_selectable(), rows.len());
        let first_tests = first_tests_row(&rows);
        assert_eq!(
            region.locate(first_tests),
            boxes.tests.map(|tests| (tests, 0))
        );
        // `region.locate(first_crates)` must resolve to `boxes.crates_io` rather than the Tests box
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
        // `placed_tests_offset(22, last_tests, 0)` scrolls the three-row Tests box to its last page
        // for its last row (box-local index four).
        let rows = panes::package_rows_from_data(&band_data());
        let last_tests = first_tests_row(&rows) + 4;
        assert_eq!(placed_tests_offset(22, last_tests, 0), 2);
    }

    #[test]
    fn tests_box_offset_holds_prior_on_a_pinned_row() {
        // `placed_tests_offset(22, structure_row, 5)` retains the prior offset on a pinned
        // Structure row, clamped to the last page (five rows minus three visible).
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
            status: LintStatus::Running(timestamp, LintRunPhase::Executing),
        };

        assert_eq!(
            lint_display_to_string(&display, elapsed, super::LintRenderMode::Enabled),
            format!("{} 3", ACTIVITY_SPINNER.frame_at(elapsed))
        );
    }

    #[test]
    fn package_lint_row_reddens_while_blocked_on_a_file_lock() {
        let timestamp =
            DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("timestamp");
        let style_for = |phase| {
            lint_display_style(&LintDisplay::Runs {
                count:  3,
                status: LintStatus::Running(timestamp, phase),
            })
            .fg
        };

        assert_eq!(style_for(LintRunPhase::Executing), Some(accent_color()));
        assert_eq!(style_for(LintRunPhase::Blocked), Some(error_color()));
    }

    #[test]
    fn package_lint_row_omits_zero_count_during_first_run() {
        let timestamp =
            DateTime::parse_from_rfc3339("2026-03-30T14:22:18-05:00").expect("timestamp");
        let elapsed = Duration::from_millis(100);
        // `LintDisplay::Runs { count: 0, .. }` represents a first run without completed history, so
        // `lint_display_to_string` shows only the spinner rather than a bare "0".
        let display = LintDisplay::Runs {
            count:  0,
            status: LintStatus::Running(timestamp, LintRunPhase::Executing),
        };

        assert_eq!(
            lint_display_to_string(&display, elapsed, super::LintRenderMode::Enabled),
            ACTIVITY_SPINNER.frame_at(elapsed).to_string()
        );
    }
}

#[cfg(test)]
mod moved_facade_tests {
    use tui_pane::PaneFocusState;

    use super::detail_column_scroll_offset;
    use super::package_label_width;
    use super::stats_column_width;
    use crate::project::DiskMeasurement;
    use crate::project::ProjectType;
    use crate::tui::panes::DescriptionState;
    use crate::tui::panes::PackageVersion;
    use crate::tui::panes::pane_data;
    use crate::tui::panes::pane_data::PackageData;
    use crate::tui::panes::pane_data::PackageDeletion;
    use crate::tui::panes::pane_data::PackagePresence;
    use crate::tui::state::CiDisplay;
    use crate::tui::state::LintDisplay;

    fn package_data(is_rust_project: bool) -> PackageData {
        PackageData {
            deletion:                 PackageDeletion::Present,
            title:                    if is_rust_project {
                "Package".to_string()
            } else {
                "Project".to_string()
            },
            name:                     "demo".to_string(),
            worktree_group_summary:   None,
            primary_section:          None,
            path:                     "~/demo".to_string(),
            version:                  PackageVersion::Manifest("0.1.0".to_string()),
            description:              DescriptionState::KnownEmpty,
            crates_io_rows:           Vec::new(),
            types:                    Some(vec![ProjectType::Library]),
            disk:                     DiskMeasurement::TotalOnly(38_989_922_304),
            stats_rows:               Vec::new(),
            test_rows:                Vec::new(),
            package_presence:         PackagePresence::Present,
            edition:                  None,
            license:                  None,
            homepage:                 None,
            repository:               None,
            out_of_tree_target_bytes: None,
            lint_display:             LintDisplay::default(),
            ci_display:               CiDisplay::default(),
        }
    }

    #[test]
    fn stats_width_cases() {
        let cases = [
            (
                "three_digit_counts",
                vec![("example", 999), ("lib", 1)],
                22,
                8,
            ),
            (
                "four_digit_counts",
                vec![("example", 1000), ("lib", 1)],
                22,
                8,
            ),
            ("short_labels", vec![("lib", 5), ("bin", 2)], 22, 8),
            ("empty_rows", vec![], 22, 8),
        ];

        for (name, rows, expected_total, expected_digits) in cases {
            let mut data = package_data(true);
            data.stats_rows = rows;
            let (total, digits) = stats_column_width(&data);
            assert_eq!(total, expected_total, "{name}");
            assert_eq!(digits, expected_digits, "{name}");
        }
    }

    #[test]
    fn package_label_width_matches_widest_visible_field() {
        let data = package_data(true);
        let fields = pane_data::package_fields_from_data(&data);
        let expected = fields.iter().map(|f| f.label().len()).max().unwrap_or(0);
        assert_eq!(package_label_width(&fields), expected);
        assert!(
            expected >= "Repository".len(),
            "label column must be wide enough for Step 4 fields (Repository = 10 chars)"
        );
    }

    #[test]
    fn detail_column_scroll_waits_until_cursor_reaches_bottom() {
        let focus = PaneFocusState::Active;
        let line_count = 20;

        assert_eq!(detail_column_scroll_offset(focus, 0, 4, line_count), 0);
        assert_eq!(detail_column_scroll_offset(focus, 3, 4, line_count), 0);
        assert_eq!(detail_column_scroll_offset(focus, 4, 4, line_count), 1);
        assert_eq!(detail_column_scroll_offset(focus, 7, 4, line_count), 4);
    }

    #[test]
    fn detail_column_scroll_clamps_to_last_page() {
        let focus = PaneFocusState::Active;

        // `detail_column_scroll_offset(focus, 9, 4, 10)` clamps the final-line cursor to the last
        // full page so content does not scroll off the bottom.
        assert_eq!(detail_column_scroll_offset(focus, 9, 4, 10), 6);
    }

    #[test]
    fn detail_column_scroll_stays_at_top_when_not_active() {
        assert_eq!(
            detail_column_scroll_offset(PaneFocusState::Remembered, 7, 4, 20),
            0
        );
        assert_eq!(
            detail_column_scroll_offset(PaneFocusState::Inactive, 7, 4, 20),
            0
        );
    }
}
