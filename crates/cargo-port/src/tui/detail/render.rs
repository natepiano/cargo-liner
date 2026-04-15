use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Cell;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use unicode_width::UnicodeWidthStr;

use super::model;
use super::model::DetailField;
use super::model::DetailInfo;
use crate::constants::IN_SYNC;
use crate::constants::NO_LINT_RUNS;
use crate::tui::app::App;
use crate::tui::constants::ACCENT_COLOR;
use crate::tui::constants::ACTIVE_BORDER_COLOR;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::ERROR_COLOR;
use crate::tui::constants::INACTIVE_BORDER_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::SUCCESS_COLOR;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::render;
use crate::tui::types::Pane;
use crate::tui::types::PaneFocusState;
use crate::tui::types::PaneId;

/// Compute the fixed stats column width from the stat rows and language stats.
/// Returns `(total_width, digit_width)`.
///
/// The column is sized to always fit 3-digit counts alongside "proc-macro"
/// (the longest possible label) with a trailing space. It only widens when a
/// count reaches 4+ digits.
pub(super) fn stats_column_width(info: &DetailInfo) -> (u16, u16) {
    let max_count = info
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
    // label width: max of "proc-macro" (10), icon+lang name (e.g. "🦀 Rust" ~6),
    // or "LOC" (3). "proc-macro" dominates at 10 chars.
    let label_width: u16 = 10;
    let total = 1 + 1 + digit_width + 1 + label_width + 1;
    (total, digit_width)
}

/// Shared style constants for detail panel rendering.
pub struct RenderStyles {
    pub readonly_label:  Style,
    pub active_border:   Style,
    pub inactive_border: Style,
    pub title:           Style,
}

#[derive(Clone, Copy)]
enum GitPresence {
    Available,
    Missing,
}

struct DetailLayoutSpec {
    constraints: Vec<Constraint>,
    lang_col:    usize,
    git_col:     Option<usize>,
}

fn detail_layout_spec(git: GitPresence) -> DetailLayoutSpec {
    let has_git = matches!(git, GitPresence::Available);
    if has_git {
        DetailLayoutSpec {
            constraints: vec![
                Constraint::Percentage(30),
                Constraint::Percentage(40),
                Constraint::Percentage(30),
            ],
            lang_col:    1,
            git_col:     Some(2),
        }
    } else {
        DetailLayoutSpec {
            constraints: vec![Constraint::Percentage(50), Constraint::Percentage(50)],
            lang_col:    1,
            git_col:     None,
        }
    }
}

struct ColumnRenderCtx<'a> {
    app:    &'a App,
    info:   &'a DetailInfo,
    fields: &'a [DetailField],
    pane:   &'a Pane,
    focus:  PaneFocusState,
    styles: &'a RenderStyles,
}

fn render_column_inner(frame: &mut Frame, ctx: &ColumnRenderCtx<'_>, area: Rect) -> usize {
    let app = ctx.app;
    let info = ctx.info;
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
        let value = field.value(info, app);
        let base_label_style = styles.readonly_label;
        let base_value_style = if *field == DetailField::Ci {
            if value == crate::constants::NO_CI_WORKFLOW || value == crate::constants::NO_CI_RUNS {
                Style::default().fg(INACTIVE_BORDER_COLOR)
            } else {
                render::conclusion_style(info.ci)
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

pub(super) fn detail_column_scroll_offset(
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

pub(super) fn package_label_width(fields: &[DetailField]) -> usize {
    fields
        .iter()
        .map(|field| field.label().width())
        .max()
        .unwrap_or(0)
        .max(8)
}

pub(super) fn git_label_width(info: &DetailInfo, fields: &[DetailField]) -> usize {
    fields
        .iter()
        .map(|field| match *field {
            DetailField::VsOrigin => "Remote branch".width(),
            DetailField::VsLocal => format!("vs local {}", info.main_branch_label).width(),
            _ => field.label().width(),
        })
        .max()
        .unwrap_or(0)
        .max(8)
}

fn render_git_column_inner(frame: &mut Frame, ctx: &ColumnRenderCtx<'_>, area: Rect) -> usize {
    let app = ctx.app;
    let info = ctx.info;
    let fields = ctx.fields;
    let pane = ctx.pane;
    let focus = ctx.focus;
    let styles = ctx.styles;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_output_line: usize = 0;
    let label_width = git_label_width(info, fields);

    for (i, field) in fields.iter().enumerate() {
        if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
            focused_output_line = lines.len();
        }
        let dynamic_label;
        let label = match *field {
            DetailField::VsOrigin => {
                dynamic_label = "Remote branch".to_string();
                &dynamic_label
            },
            DetailField::VsLocal => {
                let branch = info.main_branch_label.as_str();
                dynamic_label = format!("vs local {branch}");
                &dynamic_label
            },
            _ => field.label(),
        };
        let value = field.value(info, app);
        let selection = pane.selection_state(i, focus);
        let base_value_style = if *field == DetailField::Origin && value.starts_with('⑂') {
            Style::default()
                .fg(TITLE_COLOR)
                .add_modifier(Modifier::BOLD)
        } else if matches!(
            *field,
            DetailField::Sync | DetailField::VsOrigin | DetailField::VsLocal
        ) && value == IN_SYNC
        {
            Style::default().fg(SUCCESS_COLOR)
        } else if *field == DetailField::Sync && value == crate::constants::NO_REMOTE_SYNC {
            Style::default().fg(LABEL_COLOR)
        } else if *field == DetailField::WorktreeError {
            Style::default().fg(Color::White).bg(ERROR_COLOR)
        } else {
            Style::default()
        };
        let ls = selection.patch(styles.readonly_label);
        let vs = selection.patch(base_value_style);
        if matches!(
            *field,
            DetailField::Repo
                | DetailField::Branch
                | DetailField::RepoDesc
                | DetailField::VsOrigin
                | DetailField::WorktreeError
        ) && !value.is_empty()
        {
            let prefix = format!(" {label:<label_width$} ");
            let prefix_len = prefix.width();
            let col_width = area.width as usize;
            let avail = col_width.saturating_sub(prefix_len + 1);
            if avail > 0 && value.width() > avail {
                let wrapped =
                    if matches!(*field, DetailField::RepoDesc | DetailField::WorktreeError) {
                        word_wrap(&value, avail)
                    } else {
                        hard_wrap(&value, avail)
                    };
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
                    Span::styled(prefix, ls),
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

    append_worktree_lines(&mut lines, info);

    let scroll_y = detail_column_scroll_offset(focus, focused_output_line, area.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), area);
    usize::from(scroll_y)
}

fn append_worktree_lines(lines: &mut Vec<Line<'static>>, info: &DetailInfo) {
    if info.worktree_names.is_empty() {
        return;
    }
    let count = info.worktree_names.len();
    let label_style = Style::default().fg(LABEL_COLOR);
    let value_style = Style::default().fg(TITLE_COLOR);
    lines.push(Line::from(vec![
        Span::styled("  Worktrees  ", label_style),
        Span::styled(count.to_string(), value_style),
    ]));
}

const NO_DESCRIPTION_AVAILABLE: &str = "No description available";

pub(super) fn project_panel_title(info: &DetailInfo) -> String {
    format!(" {} - {} ", info.package_title, info.title_name)
}

struct ProjectPanelRender<'a> {
    info:         &'a DetailInfo,
    fields:       &'a [DetailField],
    focus:        PaneFocusState,
    styles:       &'a RenderStyles,
    border_style: Style,
}

#[derive(Clone, Copy)]
struct ProjectPanelAreas {
    lower: Rect,
}

pub fn render_detail_panel(
    frame: &mut Frame,
    app: &mut App,
    detail_info: Option<&DetailInfo>,
    area: Rect,
) {
    let title_style = Style::default()
        .fg(TITLE_COLOR)
        .add_modifier(Modifier::BOLD);

    if let Some(info) = detail_info {
        let git = model::git_fields(info);
        let git_presence = if git.is_empty() {
            GitPresence::Missing
        } else {
            GitPresence::Available
        };
        let spec = detail_layout_spec(git_presence);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(spec.constraints)
            .split(area);

        let mut detail_cols = vec![(PaneId::Package, columns[0])];
        detail_cols.push((PaneId::Lang, columns[spec.lang_col]));
        if let Some(git_col) = spec.git_col {
            detail_cols.push((PaneId::Git, columns[git_col]));
        }
        app.layout_cache_mut().detail_columns = detail_cols;

        let styles = RenderStyles {
            readonly_label:  Style::default().fg(LABEL_COLOR),
            active_border:   Style::default().fg(ACTIVE_BORDER_COLOR),
            inactive_border: Style::default(),
            title:           title_style,
        };

        render_project_panel(frame, app, info, &styles, columns[0]);
        render_lang_panel(frame, app, info, &styles, columns[spec.lang_col]);

        if let Some(col) = spec.git_col {
            if git.is_empty() {
                let empty_git = Block::default()
                    .borders(Borders::ALL)
                    .title(" Not a git repo ")
                    .title_style(Style::default().fg(INACTIVE_BORDER_COLOR))
                    .border_style(Style::default().fg(INACTIVE_BORDER_COLOR));
                frame.render_widget(empty_git, columns[col]);
            } else {
                app.pane_manager_mut().git.set_len(git.len());
                let focus = app.pane_focus_state(PaneId::Git);
                let git_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" Git ")
                    .title_style(styles.title)
                    .border_style(if matches!(focus, PaneFocusState::Active) {
                        styles.active_border
                    } else {
                        styles.inactive_border
                    });
                let git_inner = git_block.inner(columns[col]);
                app.pane_manager_mut().git.set_content_area(git_inner);
                frame.render_widget(git_block, columns[col]);
                let git_ctx = ColumnRenderCtx {
                    app,
                    info,
                    fields: &git,
                    pane: &app.pane_manager().git,
                    focus,
                    styles: &styles,
                };
                let scroll_offset = render_git_column_inner(frame, &git_ctx, git_inner);
                app.pane_manager_mut().git.set_scroll_offset(scroll_offset);
            }
        }
    } else {
        let empty_block = Block::default()
            .borders(Borders::ALL)
            .title(" Details ")
            .title_style(title_style);
        let content = vec![Line::from("  No project selected")];
        let detail = Paragraph::new(content).block(empty_block);
        frame.render_widget(detail, area);
    }
}

fn render_project_panel(
    frame: &mut Frame,
    app: &mut App,
    info: &DetailInfo,
    styles: &RenderStyles,
    area: Rect,
) {
    let fields = model::package_fields(info);
    app.pane_manager_mut().package.set_len(fields.len());
    let focus = app.pane_focus_state(PaneId::Package);
    let border_style = if matches!(focus, PaneFocusState::Active) {
        styles.active_border
    } else {
        styles.inactive_border
    };
    let project_block = Block::default()
        .borders(Borders::ALL)
        .title(project_panel_title(info))
        .title_style(styles.title)
        .border_style(border_style);
    let project_inner = project_block.inner(area);
    frame.render_widget(project_block, area);

    let context = ProjectPanelRender {
        info,
        fields: &fields,
        focus,
        styles,
        border_style,
    };
    let areas = render_project_description_section(frame, &context, area, project_inner);
    app.pane_manager_mut().package.set_content_area(areas.lower);

    let scroll_offset = render_project_metadata(
        frame,
        app,
        &app.pane_manager().package,
        &context,
        areas.lower,
    );
    app.pane_manager_mut()
        .package
        .set_scroll_offset(scroll_offset);
}

fn render_project_description_section(
    frame: &mut Frame,
    context: &ProjectPanelRender<'_>,
    area: Rect,
    project_inner: Rect,
) -> ProjectPanelAreas {
    let lower_metadata_height = context.fields.len().max(context.info.stats_rows.len());
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
        description_lines(context.info, description_width, description_max_height);
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
    let stats_connector_x = project_stats_connector_x(context.info, lower_area);
    if separator_height > 0 {
        render_separator(
            frame,
            Rect {
                x:      area.x,
                y:      description_area.y.saturating_add(description_height),
                width:  area.width,
                height: 1,
            },
            context.border_style,
            stats_connector_x,
        );
    }
    if let Some(connector_x) = stats_connector_x {
        render_bottom_connector(frame, area, connector_x, context.border_style);
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
    let col_ctx = ColumnRenderCtx {
        app,
        info: context.info,
        fields: context.fields,
        pane,
        focus: context.focus,
        styles: context.styles,
    };
    let has_stats = !context.info.stats_rows.is_empty();
    if has_stats {
        let (stats_width, digit_width) = stats_column_width(context.info);

        let sub_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(stats_width)])
            .split(lower_area);

        let scroll_offset = render_column_inner(frame, &col_ctx, sub_cols[0]);
        render_stats_column(
            frame,
            context.info,
            sub_cols[1],
            digit_width,
            context.border_style,
        );
        scroll_offset
    } else {
        render_column_inner(frame, &col_ctx, lower_area)
    }
}

fn project_stats_connector_x(info: &DetailInfo, lower_area: Rect) -> Option<u16> {
    if info.stats_rows.is_empty() {
        return None;
    }

    let (stats_width, _) = stats_column_width(info);
    let sub_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(stats_width)])
        .split(lower_area);
    sub_cols.get(1).map(|area| area.x)
}

fn render_stats_column(
    frame: &mut Frame,
    info: &DetailInfo,
    area: Rect,
    digit_width: u16,
    border_style: Style,
) {
    let stats_block = Block::default()
        .borders(Borders::LEFT)
        .border_style(border_style);
    let stats_inner = stats_block.inner(area);
    frame.render_widget(stats_block, area);

    let stat_label_style = Style::default().fg(LABEL_COLOR);
    let stat_num_style = Style::default().fg(TITLE_COLOR);
    let dw = digit_width as usize;
    let mut stat_lines: Vec<Line<'_>> = info
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

pub(super) fn description_lines(
    info: &DetailInfo,
    width: u16,
    max_height: u16,
) -> Vec<Line<'static>> {
    let max_width = usize::from(width);
    let max_height = usize::from(max_height);
    if max_width == 0 || max_height == 0 {
        return Vec::new();
    }

    let (description, style) = info
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

fn render_separator(frame: &mut Frame, area: Rect, style: Style, connector_x: Option<u16>) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let line = (0..area.width)
        .map(|offset| {
            let x = area.x.saturating_add(offset);
            if offset == 0 {
                '├'
            } else if offset == area.width.saturating_sub(1) {
                '┤'
            } else if connector_x == Some(x) {
                '┬'
            } else {
                '─'
            }
        })
        .collect::<String>();
    frame.render_widget(Paragraph::new(Line::from(Span::styled(line, style))), area);
}

fn render_bottom_connector(frame: &mut Frame, area: Rect, connector_x: u16, style: Style) {
    if area.width < 3 || area.height == 0 {
        return;
    }
    let first_inner_x = area.x.saturating_add(1);
    let last_inner_x = area.right().saturating_sub(2);
    if connector_x < first_inner_x || connector_x > last_inner_x {
        return;
    }

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled("┴", style))),
        Rect {
            x:      connector_x,
            y:      area.bottom().saturating_sub(1),
            width:  1,
            height: 1,
        },
    );
}

pub fn render_targets_panel(
    frame: &mut Frame,
    app: &mut App,
    info: &DetailInfo,
    styles: &RenderStyles,
    area: Rect,
) {
    let bin_count: usize = usize::from(info.is_binary);
    let ex_count: usize = info.examples.iter().map(|group| group.names.len()).sum();
    let bench_count = info.benches.len();

    let focus = app.pane_focus_state(PaneId::Targets);
    let cursor = app.pane_manager().targets.pos();

    let targets_title = {
        let mut parts = Vec::new();
        let section_indicator = |section_start: usize, section_len: usize| -> String {
            if matches!(focus, PaneFocusState::Active)
                && cursor >= section_start
                && cursor < section_start + section_len
            {
                crate::tui::types::scroll_indicator(cursor - section_start, section_len)
            } else {
                section_len.to_string()
            }
        };
        if bin_count > 0 {
            parts.push(format!("Binary ({})", section_indicator(0, bin_count)));
        }
        if ex_count > 0 {
            parts.push(format!(
                "Examples ({})",
                section_indicator(bin_count, ex_count)
            ));
        }
        if bench_count > 0 {
            parts.push(format!(
                "Benches ({})",
                section_indicator(bin_count + ex_count, bench_count)
            ));
        }
        format!(" {} ", parts.join(" / "))
    };

    let targets_block = Block::default()
        .borders(Borders::ALL)
        .title(targets_title)
        .title_style(styles.title)
        .border_style(if matches!(focus, PaneFocusState::Active) {
            styles.active_border
        } else {
            styles.inactive_border
        });

    let entries = model::build_target_list(info);
    app.pane_manager_mut().targets.set_len(entries.len());
    let content_inner = targets_block.inner(area);
    app.pane_manager_mut()
        .targets
        .set_content_area(content_inner);

    let kind_col_width = model::RunTargetKind::padded_label_width();
    let col_spacing: usize = 1;
    let leading_pad: usize = 1;
    let name_max_width =
        (content_inner.width as usize).saturating_sub(kind_col_width + col_spacing + leading_pad);

    let rows: Vec<Row> = entries
        .iter()
        .map(|entry| {
            let display = crate::tui::render::truncate_with_ellipsis(
                &entry.display_name,
                name_max_width,
                "\u{2026}",
            );
            Row::new(vec![
                Cell::from(format!(" {display}")),
                Cell::from(
                    Line::from(format!("{} ", entry.kind.label())).alignment(Alignment::Right),
                )
                .style(Style::default().fg(entry.kind.color())),
            ])
        })
        .collect();

    let widths = [
        Constraint::Fill(1),
        Constraint::Length(u16::try_from(kind_col_width).unwrap_or(u16::MAX)),
    ];
    let highlight_style = Pane::selection_style(focus);

    let table = Table::new(rows, widths)
        .block(targets_block)
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let mut table_state = TableState::default().with_selected(Some(cursor));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.pane_manager_mut()
        .targets
        .set_scroll_offset(table_state.offset());
}

/// Fixed numeric column width for language stats.
const LANG_NUM_COL: u16 = 8;

/// Column constraints for the language stats table.
/// Icon (3) + name (fill) + files + code + comments + blanks + total.
const fn lang_table_widths() -> [Constraint; 7] {
    [
        Constraint::Length(3),            // icon
        Constraint::Fill(1),              // name (expandable, truncated with ellipsis)
        Constraint::Length(LANG_NUM_COL), // files
        Constraint::Length(LANG_NUM_COL), // code
        Constraint::Length(LANG_NUM_COL), // comments
        Constraint::Length(LANG_NUM_COL), // blanks
        Constraint::Length(LANG_NUM_COL), // total
    ]
}

fn lang_header_row() -> Row<'static> {
    let style = Style::default()
        .fg(COLUMN_HEADER_COLOR)
        .add_modifier(Modifier::BOLD);
    Row::new(vec![
        Cell::from(""),
        Cell::from(""),
        Cell::from(Line::from("files").alignment(Alignment::Right)).style(style),
        Cell::from(Line::from("code").alignment(Alignment::Right)).style(style),
        Cell::from(Line::from("comments").alignment(Alignment::Right)).style(style),
        Cell::from(Line::from("blanks").alignment(Alignment::Right)).style(style),
        Cell::from(Line::from("total").alignment(Alignment::Right)).style(style),
    ])
}

fn lang_footer_row(stats: &crate::project::LanguageStats) -> Row<'static> {
    let num_bold = Style::default()
        .fg(TITLE_COLOR)
        .add_modifier(Modifier::BOLD);
    let dim_bold = Style::default()
        .fg(LABEL_COLOR)
        .add_modifier(Modifier::BOLD);
    let total_files: usize = stats.entries.iter().map(|e| e.files).sum();
    let total_code: usize = stats.entries.iter().map(|e| e.code).sum();
    let total_comments: usize = stats.entries.iter().map(|e| e.comments).sum();
    let total_blanks: usize = stats.entries.iter().map(|e| e.blanks).sum();
    let grand_total = total_code + total_comments + total_blanks;
    Row::new(vec![
        Cell::from(""),
        Cell::from(""),
        Cell::from(Line::from(total_files.to_string()).alignment(Alignment::Right)).style(num_bold),
        Cell::from(Line::from(total_code.to_string()).alignment(Alignment::Right)).style(num_bold),
        Cell::from(Line::from(total_comments.to_string()).alignment(Alignment::Right))
            .style(dim_bold),
        Cell::from(Line::from(total_blanks.to_string()).alignment(Alignment::Right))
            .style(dim_bold),
        Cell::from(Line::from(grand_total.to_string()).alignment(Alignment::Right)).style(num_bold),
    ])
}

fn lang_entry_row(entry: &crate::project::LangEntry, name_width: usize) -> Row<'static> {
    let icon = crate::project::language_icon(&entry.language);
    let name = render::truncate_with_ellipsis(&entry.language, name_width, "\u{2026}");
    let total = entry.code + entry.comments + entry.blanks;
    let num_style = Style::default().fg(TITLE_COLOR);
    let dim_style = Style::default().fg(LABEL_COLOR);
    Row::new(vec![
        Cell::from(format!(" {icon}")),
        Cell::from(name).style(dim_style),
        Cell::from(Line::from(entry.files.to_string()).alignment(Alignment::Right))
            .style(num_style),
        Cell::from(Line::from(entry.code.to_string()).alignment(Alignment::Right)).style(num_style),
        Cell::from(Line::from(entry.comments.to_string()).alignment(Alignment::Right))
            .style(dim_style),
        Cell::from(Line::from(entry.blanks.to_string()).alignment(Alignment::Right))
            .style(dim_style),
        Cell::from(Line::from(total.to_string()).alignment(Alignment::Right)).style(num_style),
    ])
}

fn render_lang_panel(
    frame: &mut Frame,
    app: &mut App,
    _info: &DetailInfo,
    styles: &RenderStyles,
    area: Rect,
) {
    let lang_stats = app
        .projects()
        .at_path(
            app.selected_project_path()
                .unwrap_or_else(|| std::path::Path::new("")),
        )
        .and_then(|p| p.language_stats.as_ref())
        .cloned();

    let lang_count = lang_stats.as_ref().map_or(0, |s| s.entries.len());
    let title = format!(" Languages ({lang_count}) ");
    let lang_focus = app.pane_focus_state(PaneId::Lang);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(styles.title)
        .border_style(if matches!(lang_focus, PaneFocusState::Active) {
            styles.active_border
        } else {
            styles.inactive_border
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(stats) = lang_stats else {
        frame.render_widget(Paragraph::new("  Scanning..."), inner);
        return;
    };

    if stats.entries.is_empty() {
        frame.render_widget(Paragraph::new("  No source files detected"), inner);
        return;
    }

    if inner.height < 2 {
        return;
    }

    let widths = lang_table_widths();
    let entry_count = stats.entries.len();

    // Compute available width for the name column: total inner width minus
    // icon (3) + 5 numeric columns (8 each) + 6 column spacings (1 each).
    let fixed_cols = 3 + 5 * usize::from(LANG_NUM_COL) + 6;
    let name_width = usize::from(inner.width).saturating_sub(fixed_cols);

    // Fixed header (1 row).
    let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Table::new([lang_header_row()], widths).column_spacing(1),
        header_area,
    );

    // Determine if footer needs pinning: entries + footer row (1) must
    // exceed the space below the header.
    let content_below_header = inner.height.saturating_sub(1);
    let rows_needed = u16::try_from(entry_count + 1).unwrap_or(u16::MAX);
    let pin_footer = rows_needed > content_below_header;

    let mut rows: Vec<Row> = stats
        .entries
        .iter()
        .map(|e| lang_entry_row(e, name_width))
        .collect();

    if pin_footer {
        // Footer pinned at bottom, body scrolls between header and footer.
        let footer_y = inner.y + inner.height.saturating_sub(1);
        let footer_area = Rect::new(inner.x, footer_y, inner.width, 1);
        frame.render_widget(
            Table::new([lang_footer_row(&stats)], widths).column_spacing(1),
            footer_area,
        );
        let body_height = inner.height.saturating_sub(2);
        let body_area = Rect::new(inner.x, inner.y + 1, inner.width, body_height);

        app.pane_manager_mut().lang.set_len(rows.len());
        app.pane_manager_mut().lang.set_content_area(body_area);
        let focus = app.pane_focus_state(PaneId::Lang);
        let table = Table::new(rows, widths)
            .column_spacing(1)
            .row_highlight_style(Pane::selection_style(focus));
        let cursor = app.pane_manager().lang.pos();
        let mut table_state = TableState::default().with_selected(Some(cursor));
        frame.render_stateful_widget(table, body_area, &mut table_state);
        app.pane_manager_mut()
            .lang
            .set_scroll_offset(table_state.offset());
    } else {
        // Footer inline — append as last row, no pinning needed.
        rows.push(lang_footer_row(&stats));
        let body_height = inner.height.saturating_sub(1);
        let body_area = Rect::new(inner.x, inner.y + 1, inner.width, body_height);

        app.pane_manager_mut().lang.set_len(entry_count);
        app.pane_manager_mut().lang.set_content_area(body_area);
        let focus = app.pane_focus_state(PaneId::Lang);
        let table = Table::new(rows, widths)
            .column_spacing(1)
            .row_highlight_style(Pane::selection_style(focus));
        let cursor = app.pane_manager().lang.pos();
        let mut table_state = TableState::default().with_selected(Some(cursor));
        frame.render_stateful_widget(table, body_area, &mut table_state);
        app.pane_manager_mut()
            .lang
            .set_scroll_offset(table_state.offset());
    }
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

/// Word-wrap text to fit within `max_width` characters, breaking at word boundaries.
fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
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

/// Hard-wrap text at exactly `max_width` characters, ignoring word boundaries.
fn hard_wrap(text: &str, max_width: usize) -> Vec<String> {
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
