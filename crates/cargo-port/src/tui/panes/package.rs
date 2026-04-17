use ratatui::Frame;
use ratatui::layout::Alignment;
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
use ratatui::widgets::Cell;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use unicode_width::UnicodeWidthStr;

use super::PaneChrome;
use super::PaneRule;
use super::PaneTitleCount;
use super::PaneTitleGroup;
use super::default_pane_chrome;
use super::empty_pane_block;
use super::prefixed_pane_title;
use super::render_rules;
use crate::constants::NO_LINT_RUNS;
use crate::tui::app::App;
use crate::tui::constants::ACCENT_COLOR;
use crate::tui::constants::ERROR_COLOR;
use crate::tui::constants::INACTIVE_BORDER_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::SUCCESS_COLOR;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::detail;
use crate::tui::detail::DetailField;
use crate::tui::detail::PackageData;
use crate::tui::detail::TargetsData;
use crate::tui::render;
use crate::tui::types::Pane;
use crate::tui::types::PaneFocusState;
use crate::tui::types::PaneId;

/// Shared style constants for pane rendering.
pub struct RenderStyles {
    pub readonly_label: Style,
    pub chrome:         PaneChrome,
}

struct PackageRenderCtx<'a> {
    app:    &'a App,
    data:   &'a PackageData,
    fields: &'a [DetailField],
    pane:   &'a Pane,
    focus:  PaneFocusState,
    styles: &'a RenderStyles,
}

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

pub fn package_label_width(fields: &[DetailField]) -> usize {
    fields
        .iter()
        .map(|field| field.label().width())
        .max()
        .unwrap_or(0)
        .max(8)
}

fn render_column_inner(frame: &mut Frame, ctx: &PackageRenderCtx<'_>, area: Rect) -> usize {
    let app = ctx.app;
    let data = ctx.data;
    let fields = ctx.fields;
    let pane = ctx.pane;
    let focus = ctx.focus;
    let styles = ctx.styles;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_output_line: usize = 0;
    let label_width = package_label_width(fields);
    for (i, field) in fields.iter().enumerate() {
        if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
            focused_output_line = lines.len();
        }
        let label = field.label();
        let selection = pane.selection_state(i, focus);
        let value = field.package_value(data, app);
        let base_label_style = styles.readonly_label;
        let base_value_style = if *field == DetailField::Ci {
            if value == crate::constants::NO_CI_WORKFLOW
                || value == crate::constants::NO_CI_RUNS
                || value == crate::constants::NO_CI_UNPUBLISHED_BRANCH
            {
                Style::default().fg(INACTIVE_BORDER_COLOR)
            } else {
                render::conclusion_style(data.ci)
            }
        } else if *field == DetailField::Lint {
            lint_value_style(&value)
        } else {
            Style::default()
        };
        let ls = selection.patch(base_label_style);
        let vs = selection.patch(base_value_style);

        if *field == DetailField::RepoDesc && !value.is_empty() {
            let prefix = format!(" {label:<label_width$} ");
            let prefix_len = prefix.width();
            let col_width = area.width as usize;
            let avail = col_width.saturating_sub(prefix_len + 1);
            if avail > 0 {
                let wrapped = word_wrap(&value, avail);
                for (wi, chunk) in wrapped.iter().enumerate() {
                    if wi == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(prefix.clone(), ls),
                            Span::styled(chunk.clone(), vs),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::raw(" ".repeat(prefix_len)),
                            Span::styled(chunk.clone(), vs),
                        ]));
                    }
                }
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {label:<label_width$} "), ls),
                    Span::styled(value, vs),
                ]));
            }
        } else if matches!(*field, DetailField::Repo | DetailField::Branch) && !value.is_empty() {
            let prefix = format!(" {label:<label_width$} ");
            let prefix_len = prefix.width();
            let col_width = area.width as usize;
            let avail = col_width.saturating_sub(prefix_len + 1);
            if avail > 0 {
                let wrapped = hard_wrap(&value, avail);
                for (wi, chunk) in wrapped.iter().enumerate() {
                    if wi == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(prefix.clone(), ls),
                            Span::styled(chunk.clone(), vs),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::raw(" ".repeat(prefix_len)),
                            Span::styled(chunk.clone(), vs),
                        ]));
                    }
                }
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {label:<label_width$} "), ls),
                    Span::styled(value, vs),
                ]));
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!(" {label:<label_width$} "), ls),
                Span::styled(value, vs),
            ]));
        }
    }

    let scroll_y = detail_column_scroll_offset(focus, focused_output_line, area.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), area);
    usize::from(scroll_y)
}

const NO_DESCRIPTION_AVAILABLE: &str = "No description available";

struct ProjectPanelRender<'a> {
    pkg_data:     &'a PackageData,
    fields:       &'a [DetailField],
    focus:        PaneFocusState,
    styles:       &'a RenderStyles,
    border_style: Style,
}

#[derive(Clone, Copy)]
struct ProjectPanelAreas {
    lower: Rect,
}

pub fn render_package_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    if let Some(pkg_data) = app.pane_manager().package_data.clone() {
        let styles = RenderStyles {
            readonly_label: Style::default().fg(LABEL_COLOR),
            chrome:         default_pane_chrome(),
        };

        render_project_panel(frame, app, &pkg_data, &styles, area);
    } else {
        let title_style = Style::default()
            .fg(TITLE_COLOR)
            .add_modifier(Modifier::BOLD);
        app.pane_manager_mut()
            .pane_mut(PaneId::Package)
            .clear_surface();
        let empty_block = Block::default()
            .borders(Borders::ALL)
            .title(" Details ")
            .title_style(title_style);
        let content = vec![Line::from("  No project selected")];
        let detail = Paragraph::new(content).block(empty_block);
        frame.render_widget(detail, area);
    }
}

pub fn render_empty_targets_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    app.pane_manager_mut()
        .pane_mut(PaneId::Targets)
        .clear_surface();
    let empty_targets = empty_pane_block(" No Targets ");
    frame.render_widget(empty_targets, area);
}

fn render_project_panel(
    frame: &mut Frame,
    app: &mut App,
    pkg_data: &PackageData,
    styles: &RenderStyles,
    area: Rect,
) {
    let fields = detail::package_fields_from_data(pkg_data);
    app.pane_manager_mut()
        .pane_mut(PaneId::Package)
        .set_len(fields.len());
    let focus = app.pane_focus_state(PaneId::Package);
    let border_style = if matches!(focus, PaneFocusState::Active) {
        styles.chrome.active_border
    } else {
        styles.chrome.inactive_border
    };
    let title = format!(" {} - {} ", pkg_data.package_title, pkg_data.title_name);
    let project_block = styles
        .chrome
        .with_inactive_border(border_style)
        .block(title, matches!(focus, PaneFocusState::Active));
    let project_inner = project_block.inner(area);
    frame.render_widget(project_block, area);

    let context = ProjectPanelRender {
        pkg_data,
        fields: &fields,
        focus,
        styles,
        border_style,
    };
    let areas = render_project_description_section(frame, &context, area, project_inner);
    app.pane_manager_mut()
        .pane_mut(PaneId::Package)
        .set_content_area(areas.lower);

    let scroll_offset = render_project_metadata(
        frame,
        app,
        app.pane_manager().pane(PaneId::Package),
        &context,
        areas.lower,
    );
    app.pane_manager_mut()
        .pane_mut(PaneId::Package)
        .set_scroll_offset(scroll_offset);
}

fn render_project_description_section(
    frame: &mut Frame,
    context: &ProjectPanelRender<'_>,
    area: Rect,
    project_inner: Rect,
) -> ProjectPanelAreas {
    let lower_metadata_height = context.fields.len().max(context.pkg_data.stats_rows.len());
    let reserved_lower_height = u16::try_from(lower_metadata_height).unwrap_or(u16::MAX);
    let reserved_separator_height = u16::from(project_inner.height > reserved_lower_height);
    let description_max_height = project_inner
        .height
        .saturating_sub(reserved_lower_height.saturating_add(reserved_separator_height));
    let description_padding = u16::from(project_inner.width > 2);
    let description_width = project_inner
        .width
        .saturating_sub(description_padding.saturating_mul(2));
    let description_lines =
        description_lines(context.pkg_data, description_width, description_max_height);
    let description_height = u16::try_from(description_lines.len()).unwrap_or(u16::MAX);
    let description_area = Rect {
        x: project_inner.x.saturating_add(description_padding),
        width: description_width,
        height: description_height,
        ..project_inner
    };
    frame.render_widget(Paragraph::new(description_lines), description_area);

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
        render_rules(
            frame,
            &[PaneRule::Horizontal {
                area:        Rect {
                    x:      area.x,
                    y:      description_area.y.saturating_add(description_height),
                    width:  area.width,
                    height: 1,
                },
                connector_x: stats_connector_x,
            }],
            context.border_style,
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
            render_rules(
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

    ProjectPanelAreas { lower: lower_area }
}

fn render_project_metadata(
    frame: &mut Frame,
    app: &App,
    pane: &Pane,
    context: &ProjectPanelRender<'_>,
    lower_area: Rect,
) -> usize {
    let col_ctx = PackageRenderCtx {
        app,
        data: context.pkg_data,
        fields: context.fields,
        pane,
        focus: context.focus,
        styles: context.styles,
    };
    let has_stats = !context.pkg_data.stats_rows.is_empty();
    if has_stats {
        let (stats_width, digit_width) = stats_column_width(context.pkg_data);

        let sub_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(stats_width)])
            .split(lower_area);

        let scroll_offset = render_column_inner(frame, &col_ctx, sub_cols[0]);
        render_stats_column(
            frame,
            context.pkg_data,
            sub_cols[1],
            digit_width,
            context.border_style,
        );
        scroll_offset
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

fn render_stats_column(
    frame: &mut Frame,
    data: &PackageData,
    area: Rect,
    digit_width: u16,
    border_style: Style,
) {
    render_rules(
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

    let stat_label_style = Style::default().fg(LABEL_COLOR);
    let stat_num_style = Style::default().fg(TITLE_COLOR);
    let dw = digit_width as usize;
    let stat_lines: Vec<Line<'_>> = data
        .stats_rows
        .iter()
        .map(|(label, count)| {
            Line::from(vec![
                Span::styled(format!(" {count:>dw$} "), stat_num_style),
                Span::styled(*label, stat_label_style),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(stat_lines), stats_inner);
}

pub(in super::super) fn description_lines(
    data: &PackageData,
    width: u16,
    max_height: u16,
) -> Vec<Line<'static>> {
    let max_width = usize::from(width);
    let max_height = usize::from(max_height);
    if max_width == 0 || max_height == 0 {
        return Vec::new();
    }

    let (description, style) = data
        .description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map_or_else(
            || (NO_DESCRIPTION_AVAILABLE, Style::default().fg(LABEL_COLOR)),
            |description| (description, Style::default()),
        );

    let wrapped = word_wrap(description, max_width);
    let overflowed = wrapped.len() > max_height;
    let mut visible = wrapped.into_iter().take(max_height).collect::<Vec<_>>();
    if overflowed && let Some(last) = visible.last_mut() {
        let with_ellipsis = format!("{last}\u{2026}");
        *last = render::truncate_with_ellipsis(&with_ellipsis, max_width, "\u{2026}");
    }

    visible
        .into_iter()
        .map(|line| Line::from(Span::styled(line, style)))
        .collect()
}

pub fn render_targets_panel(
    frame: &mut Frame,
    app: &mut App,
    data: &TargetsData,
    styles: &RenderStyles,
    area: Rect,
) {
    let bin_count: usize = usize::from(data.is_binary);
    let ex_count: usize = data.examples.iter().map(|group| group.names.len()).sum();
    let bench_count = data.benches.len();

    let focus = app.pane_focus_state(PaneId::Targets);
    let cursor = app.pane_manager().pane(PaneId::Targets).pos();

    let targets_title = {
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
        prefixed_pane_title("Targets", &PaneTitleCount::Grouped(groups))
    };

    let targets_block = styles
        .chrome
        .block(targets_title, matches!(focus, PaneFocusState::Active));

    let entries = detail::build_target_list_from_data(data);
    app.pane_manager_mut()
        .pane_mut(PaneId::Targets)
        .set_len(entries.len());
    let content_inner = targets_block.inner(area);
    app.pane_manager_mut()
        .pane_mut(PaneId::Targets)
        .set_content_area(content_inner);

    let kind_col_width = detail::RunTargetKind::padded_label_width();
    let col_spacing: usize = 1;
    let leading_pad: usize = 1;
    let name_max_width =
        (content_inner.width as usize).saturating_sub(kind_col_width + col_spacing + leading_pad);

    let rows: Vec<Row> = entries
        .iter()
        .enumerate()
        .map(|(row_index, entry)| {
            let display =
                render::truncate_with_ellipsis(&entry.display_name, name_max_width, "\u{2026}");
            Row::new(vec![
                Cell::from(format!(" {display}")),
                Cell::from(
                    Line::from(format!("{} ", entry.kind.label())).alignment(Alignment::Right),
                )
                .style(Style::default().fg(entry.kind.color())),
            ])
            .style(
                app.pane_manager()
                    .pane(PaneId::Targets)
                    .selection_state(row_index, focus)
                    .overlay_style(),
            )
        })
        .collect();

    let widths = [
        Constraint::Fill(1),
        Constraint::Length(u16::try_from(kind_col_width).unwrap_or(u16::MAX)),
    ];
    let table = Table::new(rows, widths)
        .block(targets_block)
        .column_spacing(1)
        .row_highlight_style(Style::default());

    let mut table_state = TableState::default().with_selected(Some(cursor));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.pane_manager_mut()
        .pane_mut(PaneId::Targets)
        .set_scroll_offset(table_state.offset());
}

/// Returns the appropriate style for the lint detail field value based on
/// the icon: green for passed, red for failed, accent for running spinner,
/// inactive for "no lint runs".
fn lint_value_style(value: &str) -> Style {
    use crate::constants::LINT_FAILED;
    use crate::constants::LINT_PASSED;

    if value.contains(LINT_PASSED) {
        Style::default().fg(SUCCESS_COLOR)
    } else if value.contains(LINT_FAILED) {
        Style::default().fg(ERROR_COLOR)
    } else if value.starts_with(NO_LINT_RUNS) {
        Style::default().fg(INACTIVE_BORDER_COLOR)
    } else if !value.is_empty() && value.trim() != " " {
        Style::default().fg(ACCENT_COLOR)
    } else {
        Style::default()
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
