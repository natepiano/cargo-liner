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
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use unicode_width::UnicodeWidthStr;

use super::model;
use super::model::DetailField;
use super::model::DetailInfo;
use crate::constants::IN_SYNC;
use crate::tui::app::App;
use crate::tui::render;
use crate::tui::types::Pane;
use crate::tui::types::PaneFocusState;
use crate::tui::types::PaneId;

/// Compute the fixed stats column width from the stat rows.
/// Returns `(total_width, digit_width)`.
///
/// The column is sized to always fit 3-digit counts alongside "proc-macro"
/// (the longest possible label) with a trailing space. It only widens when a
/// count reaches 4+ digits.
pub(super) fn stats_column_width(stats_rows: &[(&str, usize)]) -> (u16, u16) {
    let max_count = stats_rows
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);
    let digit_width: u16 = if max_count >= 1000 { 4 } else { 3 };
    let total = 1 + 1 + digit_width + 1 + 10 + 1;
    (total, digit_width)
}

/// Shared style constants for detail panel rendering.
struct RenderStyles {
    readonly_label:  Style,
    active_border:   Style,
    inactive_border: Style,
    title:           Style,
}

#[derive(Clone, Copy)]
enum GitPresence {
    Available,
    Missing,
}

#[derive(Clone, Copy)]
enum TargetPresence {
    Available,
    Missing,
}

struct DetailLayoutSpec {
    constraints: Vec<Constraint>,
    git_col:     Option<usize>,
    targets_col: Option<usize>,
    max_col:     usize,
}

fn detail_layout_spec(git: GitPresence, targets: TargetPresence) -> DetailLayoutSpec {
    let has_git = matches!(git, GitPresence::Available);
    let has_targets = matches!(targets, TargetPresence::Available);
    let max_col = match (has_git, has_targets) {
        (false, false) => 0,
        (true, false) | (false, true) => 1,
        (true, true) => 2,
    };
    DetailLayoutSpec {
        constraints: vec![
            Constraint::Percentage(40),
            Constraint::Percentage(40),
            Constraint::Percentage(20),
        ],
        git_col: Some(1),
        targets_col: Some(2),
        max_col,
    }
}

const fn has_targets(info: &DetailInfo) -> bool {
    info.is_binary || !info.examples.is_empty() || !info.benches.is_empty()
}

fn render_column_inner(
    frame: &mut Frame,
    info: &DetailInfo,
    fields: &[DetailField],
    pane: &Pane,
    focus: PaneFocusState,
    styles: &RenderStyles,
    area: Rect,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_output_line: usize = 0;
    let label_width = package_label_width(fields);
    for (i, field) in fields.iter().enumerate() {
        if matches!(focus, PaneFocusState::Active) && i == pane.pos() {
            focused_output_line = lines.len();
        }
        let label = field.label();
        let selection = pane.selection_state(i, focus);
        let value = field.value(info);
        let base_label_style = styles.readonly_label;
        let base_value_style = if *field == DetailField::Ci {
            render::conclusion_style(info.ci)
        } else {
            Style::default()
        };
        let ls = selection.patch(base_label_style);
        let vs = selection.patch(base_value_style);

        if matches!(*field, DetailField::Description | DetailField::RepoDesc) && !value.is_empty() {
            let prefix = format!("  {label:<label_width$} ");
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
                    Span::styled(format!("  {label:<label_width$} "), ls),
                    Span::styled(value, vs),
                ]));
            }
        } else if matches!(*field, DetailField::Repo | DetailField::Branch) && !value.is_empty() {
            let prefix = format!("  {label:<label_width$} ");
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
                    Span::styled(format!("  {label:<label_width$} "), ls),
                    Span::styled(value, vs),
                ]));
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("  {label:<label_width$} "), ls),
                Span::styled(value, vs),
            ]));
        }
    }

    let scroll_y = if matches!(focus, PaneFocusState::Active) {
        let offset = focused_output_line.saturating_sub(area.height as usize / 2);
        u16::try_from(offset).unwrap_or(u16::MAX)
    } else {
        0
    };
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), area);
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

fn render_git_column_inner(
    frame: &mut Frame,
    info: &DetailInfo,
    fields: &[DetailField],
    pane: &Pane,
    focus: PaneFocusState,
    styles: &RenderStyles,
    area: Rect,
) {
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
        let value = field.value(info);
        let selection = pane.selection_state(i, focus);
        let base_value_style = if *field == DetailField::Origin && value.starts_with('⑂') {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if matches!(
            *field,
            DetailField::Sync | DetailField::VsOrigin | DetailField::VsLocal
        ) && value == IN_SYNC
        {
            Style::default().fg(Color::Green)
        } else if *field == DetailField::Sync && value == crate::constants::NO_REMOTE_SYNC {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };
        let ls = selection.patch(styles.readonly_label);
        let vs = selection.patch(base_value_style);
        if matches!(
            *field,
            DetailField::Repo | DetailField::Branch | DetailField::RepoDesc | DetailField::VsOrigin
        ) && !value.is_empty()
        {
            let prefix = format!("  {label:<label_width$} ");
            let prefix_len = prefix.width();
            let col_width = area.width as usize;
            let avail = col_width.saturating_sub(prefix_len + 1);
            if avail > 0 && value.width() > avail {
                let wrapped = if *field == DetailField::RepoDesc {
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
                Span::styled(format!("  {label:<label_width$} "), ls),
                Span::styled(value, vs),
            ]));
        }
    }

    append_worktree_lines(&mut lines, info);

    let scroll_y = if matches!(focus, PaneFocusState::Active) {
        let offset = focused_output_line.saturating_sub(area.height as usize / 2);
        u16::try_from(offset).unwrap_or(u16::MAX)
    } else {
        0
    };
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), area);
}

fn append_worktree_lines(lines: &mut Vec<Line<'static>>, info: &DetailInfo) {
    if info.worktree_names.is_empty() {
        return;
    }
    lines.push(Line::from(""));
    let wt_title_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    lines.push(Line::from(Span::styled("  Worktrees", wt_title_style)));
    let wt_style = Style::default().fg(Color::DarkGray);
    for name in &info.worktree_names {
        lines.push(Line::from(Span::styled(format!("    {name}"), wt_style)));
    }
}

pub fn render_detail_panel(
    frame: &mut Frame,
    app: &mut App,
    detail_info: Option<&DetailInfo>,
    area: Rect,
) {
    let title_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    if let Some(info) = detail_info {
        let git = model::git_fields(info);
        let git_presence = if git.is_empty() {
            GitPresence::Missing
        } else {
            GitPresence::Available
        };
        let target_presence = if has_targets(info) {
            TargetPresence::Available
        } else {
            TargetPresence::Missing
        };
        let spec = detail_layout_spec(git_presence, target_presence);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(spec.constraints)
            .split(area);

        app.layout_cache.detail_columns = columns.to_vec();
        app.layout_cache.detail_targets_col = spec.targets_col;

        let styles = RenderStyles {
            readonly_label:  Style::default().fg(Color::DarkGray),
            active_border:   Style::default().fg(Color::Cyan),
            inactive_border: Style::default(),
            title:           title_style,
        };

        render_project_panel(frame, app, info, &styles, columns[0]);

        if let Some(col) = spec.git_col {
            if git.is_empty() {
                let empty_git = Block::default()
                    .borders(Borders::ALL)
                    .title(" Not a git repo ")
                    .title_style(Style::default().fg(Color::DarkGray))
                    .border_style(Style::default().fg(Color::DarkGray));
                frame.render_widget(empty_git, columns[col]);
            } else {
                app.git_pane.set_len(git.len());
                let focus = app.pane_focus_state(PaneId::Git);
                let git_block = Block::default()
                    .borders(Borders::ALL)
                    .title("  Git ")
                    .title_style(styles.title)
                    .border_style(if matches!(focus, PaneFocusState::Active) {
                        styles.active_border
                    } else {
                        styles.inactive_border
                    });
                let git_inner = git_block.inner(columns[col]);
                app.git_pane.set_content_area(git_inner);
                frame.render_widget(git_block, columns[col]);
                render_git_column_inner(
                    frame,
                    info,
                    &git,
                    &app.git_pane,
                    focus,
                    &styles,
                    git_inner,
                );
            }
        }

        if let Some(col) = spec.targets_col {
            if has_targets(info) {
                render_targets_panel(frame, app, info, &styles, columns[col]);
            } else {
                let empty_targets = Block::default()
                    .borders(Borders::ALL)
                    .title(" No Targets ")
                    .title_style(Style::default().fg(Color::DarkGray))
                    .border_style(Style::default().fg(Color::DarkGray));
                frame.render_widget(empty_targets, columns[col]);
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
    app.package_pane.set_len(fields.len());
    let focus = app.pane_focus_state(PaneId::Package);
    let project_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", info.package_title))
        .title_style(styles.title)
        .border_style(if matches!(focus, PaneFocusState::Active) {
            styles.active_border
        } else {
            styles.inactive_border
        });
    let project_inner = project_block.inner(area);
    app.package_pane.set_content_area(project_inner);
    frame.render_widget(project_block, area);

    if info.stats_rows.is_empty() {
        render_column_inner(
            frame,
            info,
            &fields,
            &app.package_pane,
            focus,
            styles,
            project_inner,
        );
    } else {
        let (stats_width, digit_width) = stats_column_width(&info.stats_rows);

        let sub_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(stats_width)])
            .split(project_inner);

        render_column_inner(
            frame,
            info,
            &fields,
            &app.package_pane,
            focus,
            styles,
            sub_cols[0],
        );

        let stats_block = Block::default().borders(Borders::LEFT);
        let stats_inner = stats_block.inner(sub_cols[1]);
        frame.render_widget(stats_block, sub_cols[1]);

        let stat_label_style = Style::default().fg(Color::DarkGray);
        let stat_num_style = Style::default().fg(Color::Yellow);
        let dw = digit_width as usize;
        let mut stat_lines: Vec<Line<'static>> = Vec::new();
        for &(label, count) in &info.stats_rows {
            stat_lines.push(Line::from(vec![
                Span::styled(format!(" {count:>dw$} "), stat_num_style),
                Span::styled(label, stat_label_style),
            ]));
        }
        frame.render_widget(Paragraph::new(stat_lines), stats_inner);
    }
}

fn render_targets_panel(
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
    let cursor = app.targets_pane.pos();

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
    app.targets_pane.set_len(entries.len());
    let content_inner = targets_block.inner(area);
    app.targets_pane.set_content_area(content_inner);

    let kind_col_width: usize = 7;
    let col_spacing: usize = 1;
    let name_max_width =
        (content_inner.width as usize).saturating_sub(kind_col_width + col_spacing);

    let rows: Vec<Row> = entries
        .iter()
        .map(|entry| {
            let display = crate::tui::render::truncate_with_ellipsis(
                &entry.display_name,
                name_max_width,
                "\u{2026}",
            );
            Row::new(vec![
                Cell::from(display),
                Cell::from(Line::from(entry.kind.label()).alignment(Alignment::Right))
                    .style(Style::default().fg(entry.kind.color())),
            ])
        })
        .collect();

    let widths = [Constraint::Fill(1), Constraint::Length(7)];
    let highlight_style = Pane::selection_style(focus);

    let table = Table::new(rows, widths)
        .block(targets_block)
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let selected = match focus {
        PaneFocusState::Inactive => None,
        PaneFocusState::Active | PaneFocusState::Remembered => Some(cursor),
    };
    let mut table_state = TableState::default().with_selected(selected);
    frame.render_stateful_widget(table, area, &mut table_state);
    app.targets_pane.set_scroll_offset(table_state.offset());
}

/// Returns (`max_column_index`, `targets_column_index` or `None`).
pub fn detail_layout_pub(app: &App) -> (usize, Option<usize>) {
    let spec = detail_layout(app);
    (spec.max_col, spec.targets_col)
}

fn detail_layout(app: &App) -> DetailLayoutSpec {
    let Some(info) = app.cached_detail.as_ref().map(|c| &c.info) else {
        return detail_layout_spec(GitPresence::Missing, TargetPresence::Missing);
    };
    let git = if model::git_fields(info).is_empty() {
        GitPresence::Missing
    } else {
        GitPresence::Available
    };
    let targets = if has_targets(info) {
        TargetPresence::Available
    } else {
        TargetPresence::Missing
    };
    detail_layout_spec(git, targets)
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
