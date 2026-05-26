use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
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
use tui_pane::RuleTitle;
use tui_pane::Viewport;
use tui_pane::accent_color;
use tui_pane::error_color;
use tui_pane::inactive_border_color;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::success_color;
use tui_pane::title_color;
use tui_pane::warning_color;
use unicode_width::UnicodeWidthStr;

use super::CiDisplay;
use super::DescriptionBlock;
use super::DetailField;
use super::EmptyDescriptionBehavior;
use super::LintDisplay;
use super::PackageData;
use super::PackageRow;
use super::SyncedDescriptionHeight;
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
    area:         Rect,
    digit_width:  u16,
    border_style: Style,
}

type FieldWrapFn = fn(&str, usize) -> Vec<String>;

/// Compute the fixed stats column width from the stat rows and language stats.
/// Returns `(total_width, digit_width)`.
pub fn stats_column_width(data: &PackageData) -> (u16, u16) {
    let max_count = data
        .stats_rows
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);
    let digit_width: u16 = match max_count {
        0..1000 => 3,
        1000..10_000 => 4,
        10_000..100_000 => 5,
        _ => 6,
    };
    let label_width: u16 = 10;
    let total = 1 + 1 + digit_width + 1 + label_width + 1;
    (total, digit_width)
}

pub fn detail_column_scroll_offset(
    focus: PaneFocusState,
    focused_output_line: usize,
    visible_height: u16,
) -> u16 {
    if !matches!(focus, PaneFocusState::Active) || visible_height == 0 {
        return 0;
    }

    let visible_height = usize::from(visible_height);
    let offset = focused_output_line
        .saturating_add(1)
        .saturating_sub(visible_height);
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

pub fn package_row_label_width(rows: &[PackageRow]) -> usize {
    rows.iter()
        .filter_map(|row| match row {
            PackageRow::Description | PackageRow::Section(_) | PackageRow::Structure(_) => None,
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
            PackageRow::Structure(_) => {
                // Structure rows render in the separate stats column,
                // which doesn't scroll. Anchor the metadata column to its
                // own last line rather than a position past its content,
                // so focusing a stat keeps the metadata steady instead of
                // scrolling it up.
                if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
                    focused_output_line = lines.len().saturating_sub(1);
                }
            },
            PackageRow::Section(section) => {
                let style = Style::default()
                    .fg(title_color())
                    .add_modifier(Modifier::BOLD);
                lines.push(Line::from(Span::styled(
                    format!(" {}", section.label()),
                    style,
                )));
            },
        }
    }

    let scroll_y = detail_column_scroll_offset(focus, focused_output_line, area.height);
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
        DetailField::CratesIo | DetailField::Downloads
            if panes::crates_io_value_is_unreachable_placeholder(ctx.data) =>
        {
            Style::default().fg(warning_color())
        },
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

const STATS_TITLE: &str = "Structure";

struct ProjectPanelRender<'a> {
    pkg_data:                  &'a PackageData,
    rows:                      &'a [PackageRow],
    pane:                      &'a Viewport,
    focus:                     PaneFocusState,
    styles:                    &'a RenderStyles,
    border_style:              Style,
    animation_elapsed:         Duration,
    lint_enabled:              bool,
    /// Inter-pane description sync floor; clamped per-pane by the
    /// available `description_max_height`. Read by
    /// [`DescriptionBlock::render`] so the rendered content stays in
    /// step with what `sync_floor` saw at the top of the frame.
    synced_description_height: SyncedDescriptionHeight,
}

#[derive(Clone, Copy)]
struct ProjectPanelAreas {
    lower:            Rect,
    description_rect: Option<Rect>,
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
    let focus_state = pane.focus.state;
    let PaneRenderCtx {
        animation_elapsed,
        config,
        synced_description_height,
        ..
    } = ctx;
    let lint_enabled = config.current().lint.enabled;

    let Some(pkg_data) = pane.content().cloned() else {
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
        return;
    };

    let rows = panes::package_rows_from_data(&pkg_data);
    pane.viewport.set_len(rows.len());
    if !rows
        .get(pane.viewport.pos())
        .is_some_and(panes::package_row_is_selectable)
        && let Some(pos) = panes::package_nearest_selectable_row(&rows, pane.viewport.pos())
    {
        pane.viewport.set_pos(pos);
    }
    let border_style = if matches!(focus_state, PaneFocusState::Active) {
        styles.chrome.active_border
    } else {
        styles.chrome.inactive_border
    };
    let title = format!(" {} - {} ", pkg_data.package_title, pkg_data.title_name);
    let project_block = styles
        .chrome
        .with_inactive_border(border_style)
        .block(title, matches!(focus_state, PaneFocusState::Active));
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
        focus: focus_state,
        styles,
        border_style,
        animation_elapsed: *animation_elapsed,
        lint_enabled,
        synced_description_height: *synced_description_height,
    };
    let areas = render_project_description_section(frame, &context, area, project_inner);

    let layout = render_project_metadata(frame, &pane.viewport, &context, areas.lower);
    pane.viewport.set_scroll_offset(layout.scroll_offset);
    let mut row_rects = layout.row_rects;
    if let Some(rect) = areas.description_rect {
        row_rects.push((rect, 0));
    }
    pane.set_row_rects(row_rects);
    render_overflow_affordance(
        frame,
        area,
        pane.viewport.overflow(),
        Style::default().fg(label_color()),
    );
}

fn render_project_description_section(
    frame: &mut Frame,
    context: &ProjectPanelRender<'_>,
    area: Rect,
    project_inner: Rect,
) -> ProjectPanelAreas {
    let metadata_line_count = context
        .rows
        .iter()
        .filter(|row| !matches!(row, PackageRow::Description | PackageRow::Structure(_)))
        .count();
    let lower_metadata_height = metadata_line_count.max(context.pkg_data.stats_rows.len());
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
    let description_height = block.render_with_selection(
        frame,
        project_inner,
        context.synced_description_height,
        baseline_max,
        tui_pane::selection_state(context.pane, 0, context.focus),
    );
    let description_area = Rect {
        x:      project_inner.x,
        y:      project_inner.y,
        width:  project_inner.width,
        height: description_height,
    };

    let separator_height = u16::from(
        description_height > 0
            && description_area.y.saturating_add(description_height) < project_inner.bottom(),
    );
    let lower_y = description_area
        .y
        .saturating_add(description_height)
        .saturating_add(separator_height);
    let lower_area = Rect {
        x:      project_inner.x,
        y:      lower_y,
        width:  project_inner.width,
        height: project_inner.bottom().saturating_sub(lower_y),
    };
    let stats_connector_x = project_stats_connector_x(context.pkg_data, lower_area);
    if separator_height > 0 {
        let rule_area = Rect {
            x:      area.x,
            y:      description_area.y.saturating_add(description_height),
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

    let description_rect = (description_height > 0).then_some(description_area);
    ProjectPanelAreas {
        lower: lower_area,
        description_rect,
    }
}

fn render_project_metadata(
    frame: &mut Frame,
    pane: &Viewport,
    context: &ProjectPanelRender<'_>,
    lower_area: Rect,
) -> PackageRenderLayout {
    let col_ctx = PackageRenderCtx {
        data: context.pkg_data,
        rows: context.rows,
        pane,
        focus: context.focus,
        styles: context.styles,
        animation_elapsed: context.animation_elapsed,
        lint_enabled: context.lint_enabled,
    };
    let has_stats = !context.pkg_data.stats_rows.is_empty();
    if has_stats {
        let (stats_width, digit_width) = stats_column_width(context.pkg_data);

        let sub_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(stats_width)])
            .split(lower_area);

        let mut layout = render_column_inner(frame, &col_ctx, sub_cols[0]);
        layout.row_rects.extend(render_stats_column(
            frame,
            &StatsColumnRender {
                data: context.pkg_data,
                rows: context.rows,
                pane,
                focus: context.focus,
                area: sub_cols[1],
                digit_width,
                border_style: context.border_style,
            },
        ));
        layout
    } else {
        render_column_inner(frame, &col_ctx, lower_area)
    }
}

fn project_stats_connector_x(data: &PackageData, lower_area: Rect) -> Option<u16> {
    if data.stats_rows.is_empty() {
        return None;
    }

    let (stats_width, _) = stats_column_width(data);
    let sub_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(stats_width)])
        .split(lower_area);
    sub_cols.get(1).map(|area| area.x)
}

fn render_stats_column(frame: &mut Frame, context: &StatsColumnRender<'_>) -> Vec<(Rect, usize)> {
    let data = context.data;
    let rows = context.rows;
    let pane = context.pane;
    let focus = context.focus;
    let area = context.area;
    let digit_width = context.digit_width;
    let border_style = context.border_style;
    tui_pane::render_rules(
        frame,
        &[PaneRule::Vertical {
            area: Rect {
                x:      area.x,
                y:      area.y,
                width:  1,
                height: area.height,
            },
        }],
        border_style,
    );
    let stats_inner = Rect {
        x:      area.x.saturating_add(1),
        y:      area.y,
        width:  area.width.saturating_sub(1),
        height: area.height,
    };

    let stat_label_style = Style::default().fg(label_color());
    let stat_num_style = Style::default().fg(title_color());
    let dw = digit_width as usize;
    let mut row_rects = Vec::new();
    let stat_lines: Vec<Line<'_>> = data
        .stats_rows
        .iter()
        .enumerate()
        .map(|(stat_index, (label, count))| {
            let row_index = rows.iter().position(
                |row| matches!(row, PackageRow::Structure(index) if *index == stat_index),
            );
            let selection = row_index.map_or(PaneSelectionState::Unselected, |index| {
                if stat_index < usize::from(stats_inner.height) {
                    let rect_y = stats_inner
                        .y
                        .saturating_add(u16::try_from(stat_index).unwrap_or(u16::MAX));
                    row_rects.push((
                        Rect {
                            x:      stats_inner.x,
                            y:      rect_y,
                            width:  stats_inner.width,
                            height: 1,
                        },
                        index,
                    ));
                }
                tui_pane::selection_state(pane, index, focus)
            });
            Line::from(vec![
                Span::styled(format!(" {count:>dw$} "), selection.patch(stat_num_style)),
                Span::styled(*label, selection.patch(stat_label_style)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(stat_lines), stats_inner);
    row_rects
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
    let mut remaining = text;
    while remaining.len() > max_width {
        result.push(remaining[..max_width].to_string());
        remaining = &remaining[max_width..];
    }
    result.push(remaining.to_string());
    result
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests should fail on invalid fixtures")]
mod tests {
    use std::time::Duration;

    use chrono::DateTime;
    use tui_pane::ACTIVITY_SPINNER;

    use super::lint_display_to_string;
    use crate::lint::LintStatus;
    use crate::tui::panes::LintDisplay;

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
