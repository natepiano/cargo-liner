use std::process::Command;
use std::sync::OnceLock;

use ratatui::Frame;
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

use super::app::App;
use super::types::PaneId;

mod ci_panel;
mod interaction;
mod model;
mod port_report_panel;

pub(super) use ci_panel::render_ci_panel;
pub(super) use interaction::handle_ci_runs_key;
pub(super) use interaction::handle_detail_key;
pub(super) use model::CiFetchKind;
pub(super) use model::DetailField;
pub(super) use model::DetailInfo;
pub(super) use model::PendingCiFetch;
pub(super) use model::PendingExampleRun;
pub(super) use model::ProjectCounts;
pub(super) use model::RunTargetKind;
pub(super) use model::build_detail_info;
pub(super) use model::build_target_list;
pub(super) use model::git_fields;
pub(super) use model::package_fields;
pub(super) use port_report_panel::render_port_report_panel;

/// Compute the fixed stats column width from the stat rows.
/// Returns `(total_width, digit_width)`.
///
/// The column is sized to always fit 3-digit counts alongside "proc-macro"
/// (the longest possible label) with a trailing space. It only widens when a
/// count reaches 4+ digits.
fn stats_column_width(stats_rows: &[(&str, usize)]) -> (u16, u16) {
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
    highlight: Style,
    readonly_label: Style,
    active_border: Style,
    inactive_border: Style,
    title: Style,
}

struct ColumnFocus {
    active: bool,
    remembered: bool,
    cursor: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectionState {
    Focused,
    Remembered,
    Unselected,
}

impl ColumnFocus {
    const fn selection_state(&self, row: usize) -> SelectionState {
        if row != self.cursor {
            SelectionState::Unselected
        } else if self.active {
            SelectionState::Focused
        } else if self.remembered {
            SelectionState::Remembered
        } else {
            SelectionState::Unselected
        }
    }
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
    git_col: Option<usize>,
    targets_col: Option<usize>,
    max_col: usize,
}

fn detail_layout_spec(git: GitPresence, targets: TargetPresence) -> DetailLayoutSpec {
    let has_targets = matches!(targets, TargetPresence::Available);
    match git {
        GitPresence::Available => DetailLayoutSpec {
            constraints: vec![
                Constraint::Percentage(37),
                Constraint::Percentage(37),
                Constraint::Percentage(26),
            ],
            git_col: Some(1),
            targets_col: Some(2),
            max_col: 1 + usize::from(has_targets),
        },
        GitPresence::Missing => DetailLayoutSpec {
            constraints: vec![Constraint::Percentage(74), Constraint::Percentage(26)],
            git_col: None,
            targets_col: Some(1),
            max_col: usize::from(has_targets),
        },
    }
}

const fn has_targets(info: &DetailInfo) -> bool {
    info.is_binary || !info.examples.is_empty() || !info.benches.is_empty()
}

fn render_column_inner(
    frame: &mut Frame,
    info: &DetailInfo,
    fields: &[DetailField],
    focus: &ColumnFocus,
    styles: &RenderStyles,
    area: Rect,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_output_line: usize = 0;
    let label_width = package_label_width(fields);
    for (i, field) in fields.iter().enumerate() {
        if focus.active && i == focus.cursor {
            focused_output_line = lines.len();
        }
        let label = field.label();
        let selection = focus.selection_state(i);
        let value = field.value(info);
        let base_label_style = styles.readonly_label;
        let ls = match selection {
            SelectionState::Focused => styles.highlight,
            SelectionState::Remembered | SelectionState::Unselected => base_label_style,
        };
        let vs = match selection {
            SelectionState::Focused => styles.highlight,
            SelectionState::Remembered => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            SelectionState::Unselected => {
                if *field == DetailField::Ci {
                    super::render::conclusion_style(info.ci)
                } else {
                    Style::default()
                }
            },
        };

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

    let scroll_y = if focus.active {
        let offset = focused_output_line.saturating_sub(area.height as usize / 2);
        u16::try_from(offset).unwrap_or(u16::MAX)
    } else {
        0
    };
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), area);
}

fn package_label_width(fields: &[DetailField]) -> usize {
    fields
        .iter()
        .map(|field| field.label().width())
        .max()
        .unwrap_or(0)
        .max(8)
}

fn render_git_column_inner(
    frame: &mut Frame,
    info: &DetailInfo,
    fields: &[DetailField],
    focus: &ColumnFocus,
    styles: &RenderStyles,
    area: Rect,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_output_line: usize = 0;

    for (i, field) in fields.iter().enumerate() {
        if focus.active && i == focus.cursor {
            focused_output_line = lines.len();
        }
        let dynamic_label;
        let label = match *field {
            DetailField::VsOrigin => {
                let branch = info.default_branch.as_deref().unwrap_or("main");
                dynamic_label = format!("vs o/{branch}");
                &dynamic_label
            },
            DetailField::VsLocal => {
                let branch = info.default_branch.as_deref().unwrap_or("main");
                dynamic_label = format!("vs {branch}");
                &dynamic_label
            },
            _ => field.label(),
        };
        let value = field.value(info);
        let selection = focus.selection_state(i);
        let ls = match selection {
            SelectionState::Focused => styles.highlight,
            SelectionState::Remembered | SelectionState::Unselected => styles.readonly_label,
        };
        let vs = match selection {
            SelectionState::Focused => styles.highlight,
            SelectionState::Remembered => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            SelectionState::Unselected => {
                if *field == DetailField::Origin && value.starts_with('⑂') {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else if matches!(
                    *field,
                    DetailField::Sync | DetailField::VsOrigin | DetailField::VsLocal
                ) && value == crate::constants::IN_SYNC
                {
                    Style::default().fg(Color::Green)
                } else if *field == DetailField::Sync && value == "not published" {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                }
            },
        };
        if matches!(
            *field,
            DetailField::Repo | DetailField::Branch | DetailField::RepoDesc
        ) && !value.is_empty()
        {
            let prefix = format!("  {label:<8} ");
            let prefix_len = prefix.len();
            let col_width = area.width as usize;
            let avail = col_width.saturating_sub(prefix_len + 1);
            if avail > 0 && value.len() > avail {
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
                Span::styled(format!("  {label:<8} "), ls),
                Span::styled(value, vs),
            ]));
        }
    }

    append_worktree_lines(&mut lines, info);

    let scroll_y = if focus.active {
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

pub(super) fn render_detail_panel(
    frame: &mut Frame,
    app: &mut App,
    detail_info: Option<&DetailInfo>,
    area: Rect,
) {
    let title_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    if let Some(info) = detail_info {
        let git = git_fields(info);
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
            highlight: Style::default().fg(Color::Black).bg(Color::Cyan),
            readonly_label: Style::default().fg(Color::DarkGray),
            active_border: Style::default().fg(Color::Cyan),
            inactive_border: Style::default(),
            title: title_style,
        };

        render_project_panel(frame, app, info, &styles, columns[0]);

        if let Some(col) = spec.git_col {
            app.git_pane.set_len(git.len());
            let focus = ColumnFocus {
                active: app.is_focused(PaneId::Git),
                remembered: app.remembers_selection(PaneId::Git),
                cursor: app.git_pane.pos(),
            };
            let git_block = Block::default()
                .borders(Borders::ALL)
                .title(" Git ")
                .title_style(styles.title)
                .border_style(if focus.active {
                    styles.active_border
                } else {
                    styles.inactive_border
                });
            let git_inner = git_block.inner(columns[col]);
            app.git_pane.set_content_area(git_inner);
            frame.render_widget(git_block, columns[col]);
            render_git_column_inner(frame, info, &git, &focus, &styles, git_inner);
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
    let fields = package_fields(info);
    app.package_pane.set_len(fields.len());
    let focus = ColumnFocus {
        active: app.is_focused(PaneId::Package),
        remembered: app.remembers_selection(PaneId::Package),
        cursor: app.package_pane.pos(),
    };
    let project_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", info.package_title))
        .title_style(styles.title)
        .border_style(if focus.active {
            styles.active_border
        } else {
            styles.inactive_border
        });
    let project_inner = project_block.inner(area);
    app.package_pane.set_content_area(project_inner);
    frame.render_widget(project_block, area);

    if info.stats_rows.is_empty() {
        render_column_inner(frame, info, &fields, &focus, styles, project_inner);
    } else {
        let (stats_width, digit_width) = stats_column_width(&info.stats_rows);

        let sub_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(stats_width)])
            .split(project_inner);

        render_column_inner(frame, info, &fields, &focus, styles, sub_cols[0]);

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
    let mut title_parts = Vec::new();
    if bin_count > 0 {
        title_parts.push(format!("Binary ({bin_count})"));
    }
    if ex_count > 0 {
        title_parts.push(format!("Examples ({ex_count})"));
    }
    if bench_count > 0 {
        title_parts.push(format!("Benches ({bench_count})"));
    }
    let targets_title = format!(" {} ", title_parts.join(" / "));

    let is_active = app.is_focused(PaneId::Targets);
    let targets_block = Block::default()
        .borders(Borders::ALL)
        .title(targets_title)
        .title_style(styles.title)
        .border_style(if is_active {
            styles.active_border
        } else {
            styles.inactive_border
        });

    let entries = build_target_list(info);
    app.targets_pane.set_len(entries.len());
    app.targets_pane.set_content_area(targets_block.inner(area));

    let rows: Vec<Row> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let name_style = if i == app.targets_pane.pos()
                && !is_active
                && app.remembers_selection(PaneId::Targets)
            {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(entry.display_name.clone()).style(name_style),
                Cell::from(
                    Line::from(entry.kind.label()).alignment(ratatui::layout::Alignment::Right),
                )
                .style(Style::default().fg(entry.kind.color())),
            ])
        })
        .collect();

    let widths = [Constraint::Fill(1), Constraint::Length(7)];
    let highlight_style = if is_active {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default()
    };

    let table = Table::new(rows, widths)
        .block(targets_block)
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let selected = if is_active {
        Some(app.targets_pane.pos())
    } else {
        None
    };
    let mut table_state = TableState::default().with_selected(selected);
    frame.render_stateful_widget(table, area, &mut table_state);
    app.targets_pane.set_scroll_offset(table_state.offset());
}

/// Get the local UTC offset in seconds (e.g., -28800 for PST).
fn local_utc_offset_secs() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|output| {
                let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if value.len() >= 5 {
                    let sign: i64 = if value.starts_with('-') { -1 } else { 1 };
                    let hours: i64 = value[1..3].parse().ok()?;
                    let mins: i64 = value[3..5].parse().ok()?;
                    Some(sign * (hours * 3600 + mins * 60))
                } else {
                    None
                }
            })
            .unwrap_or(0)
    })
}

const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        },
        _ => 30,
    }
}

/// Convert a UTC ISO 8601 timestamp to local time, formatted as `yyyy-mm-dd hh:mm`.
fn format_timestamp(iso: &str) -> String {
    let utc_offset_secs = local_utc_offset_secs();
    let stripped = iso.trim_end_matches('Z');
    match stripped.split_once('T') {
        Some((date, time)) => {
            let date_parts: Vec<&str> = date.split('-').collect();
            let time_parts: Vec<&str> = time.split(':').collect();
            if date_parts.len() >= 3
                && time_parts.len() >= 2
                && let (Ok(y), Ok(month), Ok(day), Ok(hour), Ok(minute)) = (
                    date_parts[0].parse::<i64>(),
                    date_parts[1].parse::<i64>(),
                    date_parts[2].parse::<i64>(),
                    time_parts[0].parse::<i64>(),
                    time_parts[1].parse::<i64>(),
                )
            {
                let total_mins = hour * 60 + minute + utc_offset_secs / 60;
                let mut day = day;
                let mut month = month;
                let mut year = y;
                let mut adj_mins = total_mins % (24 * 60);
                if adj_mins < 0 {
                    adj_mins += 24 * 60;
                    day -= 1;
                    if day < 1 {
                        month -= 1;
                        if month < 1 {
                            month = 12;
                            year -= 1;
                        }
                        day = days_in_month(year, month);
                    }
                } else if adj_mins >= 24 * 60 {
                    adj_mins -= 24 * 60;
                    day += 1;
                    if day > days_in_month(year, month) {
                        day = 1;
                        month += 1;
                        if month > 12 {
                            month = 1;
                            year += 1;
                        }
                    }
                }
                let local_h = adj_mins / 60;
                let local_m = adj_mins % 60;
                return format!("{year:04}-{month:02}-{day:02} {local_h:02}:{local_m:02}");
            }
            let short_time = if time.len() >= 5 { &time[..5] } else { time };
            format!("{date} {short_time}")
        },
        None => stripped.to_string(),
    }
}

/// Returns (`max_column_index`, `targets_column_index` or `None`).
pub(super) fn detail_layout_pub(app: &App) -> (usize, Option<usize>) {
    let spec = detail_layout(app);
    (spec.max_col, spec.targets_col)
}

fn detail_layout(app: &App) -> DetailLayoutSpec {
    let Some(project) = app.selected_project() else {
        return detail_layout_spec(GitPresence::Missing, TargetPresence::Missing);
    };
    let info = build_detail_info(app, project);
    let git = if git_fields(&info).is_empty() {
        GitPresence::Missing
    } else {
        GitPresence::Available
    };
    let targets = if has_targets(&info) {
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

#[cfg(test)]
mod tests {
    use super::DetailField;
    use super::DetailInfo;
    use super::ci_panel::CI_COMPACT_DURATION_WIDTH;
    use super::ci_panel::ci_table_shows_durations;
    use super::ci_panel::ci_total_width;
    use super::model::package_fields;
    use super::package_label_width;
    use super::port_report_panel::format_port_report_commands;
    use super::port_report_panel::format_port_report_pending;
    use super::port_report_panel::format_port_report_slowest;
    use super::stats_column_width;
    use crate::ci::CiJob;
    use crate::ci::CiRun;
    use crate::ci::Conclusion;
    use crate::ci::FetchStatus::Fetched;
    use crate::port_report::PortReportCommand;
    use crate::port_report::PortReportCommandStatus;
    use crate::port_report::PortReportRun;
    use crate::port_report::PortReportRunStatus;
    use crate::project::ExampleGroup;
    use crate::project::ProjectLanguage;
    use crate::tui::render::CiColumn;

    fn detail_info(is_rust: ProjectLanguage, lint_label: &str) -> DetailInfo {
        DetailInfo {
            package_title: "Package".to_string(),
            name: "demo".to_string(),
            path: "~/demo".to_string(),
            version: "0.1.0".to_string(),
            description: None,
            crates_version: None,
            crates_downloads: None,
            types: "lib".to_string(),
            disk: "36.3 GiB".to_string(),
            lint_label: lint_label.to_string(),
            ci: None,
            stats_rows: Vec::new(),
            git_branch: None,
            git_sync: None,
            git_vs_origin: None,
            git_vs_local: None,
            default_branch: None,
            git_origin: None,
            git_owner: None,
            git_url: None,
            git_stars: None,
            repo_description: None,
            git_inception: None,
            git_last_commit: None,
            worktree_label: None,
            worktree_names: Vec::new(),
            is_binary: false,
            binary_name: None,
            examples: Vec::<ExampleGroup>::new(),
            benches: Vec::new(),
            is_rust,
            has_package: true,
            is_vendored: false,
        }
    }

    fn ci_run_with_jobs(jobs: Vec<CiJob>) -> CiRun {
        CiRun {
            run_id: 1,
            created_at: "2026-04-01T21:00:00-04:00".to_string(),
            branch: "feat/box-select".to_string(),
            url: "https://example.com/run/1".to_string(),
            conclusion: Conclusion::Success,
            jobs,
            wall_clock_secs: Some(17),
            commit_title: Some("feat: add box select".to_string()),
            fetched: Fetched,
        }
    }

    fn run_with_commands(
        status: PortReportRunStatus,
        commands: Vec<PortReportCommand>,
    ) -> PortReportRun {
        PortReportRun {
            run_id: "run-1".to_string(),
            started_at: "2026-04-01T21:00:00-04:00".to_string(),
            finished_at: Some("2026-04-01T21:00:10-04:00".to_string()),
            duration_ms: Some(10_000),
            status,
            commands,
        }
    }

    #[test]
    fn stats_width_fixed_for_three_digit_counts() {
        let rows = vec![("example", 999), ("lib", 1)];
        let (total, digits) = stats_column_width(&rows);
        assert_eq!(digits, 3);
        assert_eq!(total, 17);
    }

    #[test]
    fn stats_width_expands_at_four_digits() {
        let rows = vec![("example", 1000), ("lib", 1)];
        let (total, digits) = stats_column_width(&rows);
        assert_eq!(digits, 4);
        assert_eq!(total, 18);
    }

    #[test]
    fn stats_width_stable_for_short_labels() {
        let rows = vec![("lib", 5), ("bin", 2)];
        let (total, _) = stats_column_width(&rows);
        assert_eq!(total, 17);
    }

    #[test]
    fn stats_width_empty_rows() {
        let rows: Vec<(&str, usize)> = vec![];
        let (total, digits) = stats_column_width(&rows);
        assert_eq!(digits, 3);
        assert_eq!(total, 17);
    }

    #[test]
    fn package_fields_place_lint_and_ci_before_disk_for_rust_projects() {
        let info = detail_info(ProjectLanguage::Rust, "🟢");
        assert_eq!(
            package_fields(&info)
                .into_iter()
                .map(DetailField::label)
                .collect::<Vec<_>>(),
            vec![
                "Name", "Path", "Targets", "Lint", "CI", "Disk", "Version", "Desc",
            ]
        );
    }

    #[test]
    fn package_fields_place_lint_and_ci_before_disk_for_non_rust_projects() {
        let info = detail_info(ProjectLanguage::NonRust, "🔴");
        assert_eq!(
            package_fields(&info)
                .into_iter()
                .map(DetailField::label)
                .collect::<Vec<_>>(),
            vec!["Name", "Path", "Lint", "CI", "Disk"]
        );
    }

    #[test]
    fn package_label_width_expands_for_crates_io() {
        let info = DetailInfo {
            crates_version: Some("0.0.3".to_string()),
            crates_downloads: Some(74),
            ..detail_info(ProjectLanguage::Rust, "🟢")
        };
        let fields = package_fields(&info);
        assert_eq!(package_label_width(&fields), "crates.io".len());
    }

    #[test]
    fn ci_table_hides_durations_when_fixed_columns_overflow() {
        let runs = vec![ci_run_with_jobs(vec![
            CiJob {
                name: "fmt".to_string(),
                conclusion: Conclusion::Success,
                duration: "17s".to_string(),
                duration_secs: Some(17),
            },
            CiJob {
                name: "clippy".to_string(),
                conclusion: Conclusion::Success,
                duration: "21s".to_string(),
                duration_secs: Some(21),
            },
        ])];
        let cols = vec![CiColumn::Fmt, CiColumn::Clippy];

        assert!(!ci_table_shows_durations(&runs, &cols, 20));
        assert_eq!(ci_total_width(&runs, false), CI_COMPACT_DURATION_WIDTH);
    }

    #[test]
    fn ci_table_keeps_durations_when_fixed_columns_fit() {
        let runs = vec![ci_run_with_jobs(vec![CiJob {
            name: "fmt".to_string(),
            conclusion: Conclusion::Success,
            duration: "17s".to_string(),
            duration_secs: Some(17),
        }])];
        let cols = vec![CiColumn::Fmt];

        assert!(ci_table_shows_durations(&runs, &cols, 80));
    }

    #[test]
    fn port_report_commands_summary_for_passed_run() {
        let run = run_with_commands(
            PortReportRunStatus::Passed,
            vec![
                PortReportCommand {
                    name: "mend".to_string(),
                    command: "cargo mend".to_string(),
                    status: PortReportCommandStatus::Passed,
                    duration_ms: Some(1_000),
                    exit_code: Some(0),
                    log_file: "port-report/mend-latest.log".to_string(),
                },
                PortReportCommand {
                    name: "clippy".to_string(),
                    command: "cargo clippy".to_string(),
                    status: PortReportCommandStatus::Passed,
                    duration_ms: Some(2_000),
                    exit_code: Some(0),
                    log_file: "port-report/clippy-latest.log".to_string(),
                },
            ],
        );

        assert_eq!(format_port_report_commands(&run), "mend, clippy");
        assert_eq!(format_port_report_pending(&run), "0");
        assert_eq!(format_port_report_slowest(&run), "clippy 0:02");
    }

    #[test]
    fn port_report_commands_summary_for_failed_run() {
        let run = run_with_commands(
            PortReportRunStatus::Failed,
            vec![
                PortReportCommand {
                    name: "mend".to_string(),
                    command: "cargo mend".to_string(),
                    status: PortReportCommandStatus::Passed,
                    duration_ms: Some(1_000),
                    exit_code: Some(0),
                    log_file: "port-report/mend-latest.log".to_string(),
                },
                PortReportCommand {
                    name: "clippy".to_string(),
                    command: "cargo clippy".to_string(),
                    status: PortReportCommandStatus::Failed,
                    duration_ms: Some(2_000),
                    exit_code: Some(101),
                    log_file: "port-report/clippy-latest.log".to_string(),
                },
            ],
        );

        assert_eq!(format_port_report_commands(&run), "mend, clippy");
        assert_eq!(format_port_report_pending(&run), "0");
        assert_eq!(format_port_report_slowest(&run), "clippy 0:02");
    }

    #[test]
    fn port_report_commands_summary_for_running_run() {
        let run = run_with_commands(
            PortReportRunStatus::Running,
            vec![
                PortReportCommand {
                    name: "mend".to_string(),
                    command: "cargo mend".to_string(),
                    status: PortReportCommandStatus::Passed,
                    duration_ms: Some(1_000),
                    exit_code: Some(0),
                    log_file: "port-report/mend-latest.log".to_string(),
                },
                PortReportCommand {
                    name: "clippy".to_string(),
                    command: "cargo clippy".to_string(),
                    status: PortReportCommandStatus::Pending,
                    duration_ms: None,
                    exit_code: None,
                    log_file: "port-report/clippy-latest.log".to_string(),
                },
            ],
        );

        assert_eq!(format_port_report_commands(&run), "mend, clippy");
        assert_eq!(format_port_report_pending(&run), "1");
        assert_eq!(format_port_report_slowest(&run), "mend 0:01");
    }
}
