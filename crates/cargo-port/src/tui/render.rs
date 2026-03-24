use std::collections::HashMap;

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
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::Paragraph;

use super::App;
use super::ExpandKey;
use super::FocusTarget;
use super::VisibleRow;
use super::detail::build_detail_info;
use super::detail::render_ci_panel;
use super::detail::render_detail_panel;
use super::settings::render_settings_popup;
use crate::ci::CiRun;

pub const BYTES_PER_MIB: u64 = 1024 * 1024;
pub const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

pub const DISK_COL_WIDTH: usize = 10;
pub const CI_COL_WIDTH: usize = 4;
pub const GIT_COL_WIDTH: usize = 2;
pub const BORDER_PADDING: usize = 3;

#[derive(Clone, Copy)]
pub enum CiColumn {
    Fmt,
    Taplo,
    Clippy,
    Mend,
    Build,
    Test,
    Bench,
}

impl CiColumn {
    pub fn matches(self, job_name: &str) -> bool {
        let lower = job_name.to_lowercase();
        match self {
            Self::Fmt => lower.contains("format") || lower.contains("fmt"),
            Self::Taplo => lower.contains("taplo"),
            Self::Clippy => lower.contains("clippy"),
            Self::Mend => lower.contains("mend"),
            Self::Build => lower.contains("build"),
            Self::Test => lower.contains("test"),
            Self::Bench => lower.contains("bench"),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fmt => "fmt",
            Self::Taplo => "taplo",
            Self::Clippy => "clippy",
            Self::Mend => "mend",
            Self::Build => "build",
            Self::Test => "test",
            Self::Bench => "bench",
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    if bytes >= BYTES_PER_GIB {
        format!("{:.1} GiB", bytes as f64 / BYTES_PER_GIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / BYTES_PER_MIB as f64)
    }
}

pub fn display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    UnicodeWidthStr::width(s)
}

pub fn conclusion_style(conclusion: &str) -> Style {
    if conclusion.contains('✓') {
        Style::default().fg(Color::Green)
    } else if conclusion.contains('✗') {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

/// Compute a color for a disk value: green (smallest) → white (middle) → red (largest).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn disk_color(bytes: Option<u64>, min_bytes: u64, max_bytes: u64) -> Style {
    let Some(bytes) = bytes else {
        return Style::default().fg(Color::DarkGray);
    };

    if min_bytes == max_bytes {
        return Style::default();
    }

    // Normalize to 0.0..1.0 using log scale for better distribution with outliers
    let log_min = (min_bytes.max(1) as f64).ln();
    let log_max = (max_bytes.max(1) as f64).ln();
    let log_val = (bytes.max(1) as f64).ln();
    let pos = (log_val - log_min) / (log_max - log_min);

    // Green (0.0) → White (0.5) → Red (1.0)
    let (r, g, b) = if pos < 0.5 {
        // Green to white: increase R and B
        let t = pos * 2.0;
        (
            155.0f64.mul_add(t, 100.0) as u8,
            35.0f64.mul_add(t, 220.0) as u8,
            155.0f64.mul_add(t, 100.0) as u8,
        )
    } else {
        // White to red: decrease G and B
        let t = (pos - 0.5) * 2.0;
        let gb = 155.0f64.mul_add(-t, 255.0) as u8;
        (255, gb, gb)
    };

    Style::default().fg(Color::Rgb(r, g, b))
}

pub fn project_row_spans(
    prefix: &str,
    name: &str,
    disk: &str,
    disk_style: Style,
    ci: &str,
    git_icon: &str,
    name_width: usize,
) -> Line<'static> {
    let prefix_width = display_width(prefix);
    let available = name_width.saturating_sub(prefix_width);
    let padded_name = format!("{prefix}{name:<available$}");
    let ci_style = conclusion_style(ci);
    let git_style = match git_icon {
        "⑂" => Style::default().fg(Color::Cyan),
        "⊙" => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };

    Line::from(vec![
        Span::raw(padded_name),
        Span::styled(format!(" {disk:>9}"), disk_style),
        Span::styled(format!("  {ci}"), ci_style),
        Span::styled(format!(" {git_icon}"), git_style),
    ])
}

pub fn group_header_spans(prefix: &str, name: &str, name_width: usize) -> Line<'static> {
    let prefix_width = display_width(prefix);
    let available = name_width.saturating_sub(prefix_width);
    let padded = format!("{prefix}{name:<available$}");
    Line::from(vec![Span::styled(
        padded,
        Style::default().fg(Color::Yellow),
    )])
}

pub fn ui(frame: &mut Frame, app: &mut App) {
    let outer_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    // Left panel width: name column + disk + ci + git + borders + padding
    #[allow(clippy::cast_possible_truncation)]
    let left_width =
        (app.max_name_width() + DISK_COL_WIDTH + CI_COL_WIDTH + GIT_COL_WIDTH + BORDER_PADDING)
            as u16;

    let main_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_width), Constraint::Min(20)])
        .split(outer_layout[0]);

    // Left panel: split into optional search bar + project list + optional scan log
    let search_height = if app.searching { 3 } else { 0 };
    let left_constraints = if app.scan_complete {
        vec![Constraint::Length(search_height), Constraint::Min(1)]
    } else {
        #[allow(clippy::cast_possible_truncation)]
        let project_rows = app.visible_rows().len() as u16;
        let project_height = (project_rows + 2).max(3);
        vec![
            Constraint::Length(search_height),
            Constraint::Length(project_height),
            Constraint::Min(3),
        ]
    };
    let left_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(left_constraints)
        .split(main_layout[0]);

    if app.searching {
        render_search_bar(frame, app, left_layout[0]);
    }

    #[allow(clippy::cast_possible_truncation)]
    let inner_width = left_layout[1].width.saturating_sub(2) as usize;
    let name_col_width = inner_width.saturating_sub(DISK_COL_WIDTH + CI_COL_WIDTH + GIT_COL_WIDTH);

    let selected_project_ref = app.selected_project();
    let detail_info = selected_project_ref.map(|p| build_detail_info(app, p));
    let has_ci_runs = selected_project_ref.is_some_and(|p| app.ci_runs.contains_key(&p.path));
    let detail_ci_runs: Vec<CiRun> = selected_project_ref
        .and_then(|p| app.ci_runs_for(p))
        .cloned()
        .unwrap_or_default();

    render_project_list(frame, app, name_col_width, left_layout[1]);

    if !app.scan_complete {
        render_scan_log(frame, app, left_layout[2]);
    }

    // Right panel
    let has_example_output = !app.example_output.is_empty();
    let right_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(match (has_ci_runs, has_example_output) {
            (true, true) => vec![
                Constraint::Length(14),
                Constraint::Length(6),
                Constraint::Min(5),
            ],
            (true, false) => {
                vec![Constraint::Length(14), Constraint::Min(3)]
            },
            (false, true) => {
                vec![Constraint::Min(1), Constraint::Min(5)]
            },
            (false, false) => vec![Constraint::Min(1)],
        })
        .split(main_layout[1]);

    render_detail_panel(frame, app, detail_info.as_ref(), right_layout[0]);

    if has_ci_runs {
        render_ci_panel(frame, app, &detail_ci_runs, right_layout[1]);
    }

    if has_example_output {
        let example_area = right_layout[right_layout.len() - 1];
        render_example_output(frame, app, example_area);
    }

    render_status_bar(frame, app, outer_layout[1]);

    if app.show_settings {
        render_settings_popup(frame, app);
    }
}

pub fn render_search_bar(frame: &mut Frame, app: &App, area: Rect) {
    let search_style = if app.searching {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let search_text = if app.searching {
        if app.search_query.is_empty() {
            "…".to_string()
        } else {
            app.search_query.clone()
        }
    } else {
        "/ to search".to_string()
    };

    let search_bar = Paragraph::new(Line::from(vec![
        Span::styled(" 🔍 ", Style::default().fg(Color::Yellow)),
        Span::styled(search_text, search_style),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if app.searching {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            }),
    );

    frame.render_widget(search_bar, area);
}

pub fn render_project_list(frame: &mut Frame, app: &mut App, name_col_width: usize, area: Rect) {
    let items: Vec<ListItem> = if app.searching && !app.search_query.is_empty() {
        render_filtered_items(app, name_col_width)
    } else {
        render_tree_items(app, name_col_width)
    };

    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let header_line = Line::from(vec![
        Span::styled(format!("{:<name_col_width$}", "Project"), header_style),
        Span::styled(format!(" {:>9}", "Disk"), header_style),
        Span::styled("  CI", header_style),
    ]);

    let project_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(header_line)
                .title_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .highlight_style(if app.focus == FocusTarget::ProjectList {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        });

    frame.render_stateful_widget(project_list, area, &mut app.list_state);
}

pub fn render_scan_log(frame: &mut Frame, app: &mut App, area: Rect) {
    let log_items: Vec<ListItem> = app
        .scan_log
        .iter()
        .map(|p| {
            ListItem::new(Span::styled(
                format!("  {p}"),
                Style::default().fg(Color::DarkGray),
            ))
        })
        .collect();

    let scan_focused = app.focus == FocusTarget::ScanLog;
    let scan_title = if scan_focused {
        " Scanning (focused) "
    } else {
        " Scanning "
    };
    let scan_log = List::new(log_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(scan_title)
                .title_style(if scan_focused {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                })
                .border_style(if scan_focused {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                }),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(scan_log, area, &mut app.scan_log_state);
}

fn render_example_output(frame: &mut Frame, app: &App, area: Rect) {
    let title = app.example_running.as_ref().map_or_else(
        || " Example Output ".to_string(),
        |n| format!(" Running: {n} "),
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(if app.example_running.is_some() {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let lines: Vec<Line> = app
        .example_output
        .iter()
        .map(|l| {
            Line::from(Span::styled(
                format!(" {l}"),
                Style::default().fg(Color::DarkGray),
            ))
        })
        .collect();

    let inner_height = area.height.saturating_sub(2);
    #[allow(clippy::cast_possible_truncation)]
    let total_lines = lines.len() as u16;
    let scroll_offset = total_lines.saturating_sub(inner_height);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));

    frame.render_widget(paragraph, area);
}

fn status_bar_spans(app: &App) -> Vec<Span<'static>> {
    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let global_counts = app.project_counts();
    let count_str = global_counts.summary();
    let count_style = Style::default().fg(Color::Yellow);

    let scan_indicator = if app.scan_complete {
        Span::raw("")
    } else {
        Span::styled(
            " ⟳ scanning… ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    };

    if app.editing.is_some() {
        vec![
            scan_indicator,
            Span::styled(" Enter", key_style),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style),
            Span::raw(" cancel"),
        ]
    } else if app.searching {
        vec![
            scan_indicator,
            Span::styled(" ↑/↓", key_style),
            Span::raw(" navigate  "),
            Span::styled("enter", key_style),
            Span::raw(" select  "),
            Span::styled("esc", key_style),
            Span::raw(" cancel"),
        ]
    } else if app.focus == FocusTarget::DetailFields {
        vec![
            scan_indicator,
            Span::styled(" ↑/↓", key_style),
            Span::raw(" nav  "),
            Span::styled("←/→", key_style),
            Span::raw(" column  "),
            Span::styled("Enter", key_style),
            Span::raw(" edit  "),
            Span::styled("Tab", key_style),
            Span::raw(" next  "),
            Span::styled("Esc", key_style),
            Span::raw(" back"),
        ]
    } else if app.focus == FocusTarget::CiRuns {
        vec![
            scan_indicator,
            Span::styled(" ↑/↓", key_style),
            Span::raw(" nav  "),
            Span::styled("Enter", key_style),
            Span::raw(" fetch  "),
            Span::styled("c", key_style),
            Span::raw(" clear cache  "),
            Span::styled("Tab", key_style),
            Span::raw(" next  "),
            Span::styled("Esc", key_style),
            Span::raw(" back"),
        ]
    } else {
        vec![
            scan_indicator,
            Span::styled(format!(" {count_str}"), count_style),
            Span::raw("  "),
            Span::styled("↑/↓", key_style),
            Span::raw(" nav  "),
            Span::styled("←/→", key_style),
            Span::raw(" expand  "),
            Span::styled("Tab", key_style),
            Span::raw(" details  "),
            Span::styled("Home/End", key_style),
            Span::raw(" top/btm  "),
            Span::styled("/", key_style),
            Span::raw(" search  "),
            Span::styled("r", key_style),
            Span::raw(" rescan  "),
            Span::styled("s", key_style),
            Span::raw(" settings  "),
            Span::styled("q", key_style),
            Span::raw(" quit"),
        ]
    }
}

pub fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let status_spans = status_bar_spans(app);

    // Total disk usage on the right
    let total_bytes: u64 = app.disk_usage.values().sum();
    let total_disk = if total_bytes > 0 {
        format!("  Σ {}", super::format_bytes(total_bytes))
    } else {
        String::new()
    };
    let disk_style = Style::default().fg(Color::Yellow);

    // Build a two-part line: left-aligned keys, right-aligned disk total
    let left = Line::from(status_spans);
    let right = Span::styled(total_disk, disk_style);

    // Render left-aligned status
    let status_bar =
        Paragraph::new(left).style(Style::default().bg(Color::DarkGray).fg(Color::White));
    frame.render_widget(status_bar, area);

    // Render right-aligned disk total
    #[allow(clippy::cast_possible_truncation)]
    let right_width = right.width() as u16;
    if area.width > right_width {
        let right_area = Rect::new(area.x + area.width - right_width, area.y, right_width, 1);
        let right_para =
            Paragraph::new(Line::from(right)).style(Style::default().bg(Color::DarkGray));
        frame.render_widget(right_para, right_area);
    }
}

/// Compute (min, max) disk bytes across all top-level nodes.
fn root_disk_range(app: &App) -> (u64, u64) {
    let mut min = u64::MAX;
    let mut max = 0u64;
    let mut any = false;
    for node in &app.nodes {
        if let Some(bytes) = app.disk_bytes_for_node(node) {
            min = min.min(bytes);
            max = max.max(bytes);
            any = true;
        }
    }
    if any { (min, max) } else { (0, 0) }
}

/// Compute (min, max) disk bytes for children within each node (members + worktrees).
/// Returns a map from `node_index` to (min, max).
fn child_disk_ranges(app: &App) -> HashMap<usize, (u64, u64)> {
    let mut ranges = HashMap::new();
    for (ni, node) in app.nodes.iter().enumerate() {
        let mut min = u64::MAX;
        let mut max = 0u64;
        let mut any = false;
        // Members
        for group in &node.groups {
            for member in &group.members {
                if let Some(&bytes) = app.disk_usage.get(&member.path) {
                    min = min.min(bytes);
                    max = max.max(bytes);
                    any = true;
                }
            }
        }
        // Worktrees
        for wt in &node.worktrees {
            if let Some(&bytes) = app.disk_usage.get(&wt.project.path) {
                min = min.min(bytes);
                max = max.max(bytes);
                any = true;
            }
        }
        if any {
            ranges.insert(ni, (min, max));
        }
    }
    ranges
}

#[allow(clippy::too_many_lines)]
pub fn render_tree_items(app: &App, name_width: usize) -> Vec<ListItem<'static>> {
    // Precompute min/max disk bytes per level for coloring
    let (root_min, root_max) = root_disk_range(app);
    let child_ranges = child_disk_ranges(app);

    let rows = app.visible_rows();
    rows.iter()
        .map(|row| match row {
            VisibleRow::Root { node_index } => {
                let node = &app.nodes[*node_index];
                let project = &node.project;
                let mut name = project.display_name();
                if !node.worktrees.is_empty() {
                    name = format!("{name} wt:{}", node.worktrees.len());
                }
                let disk = app.formatted_disk_for_node(node);
                let disk_bytes = app.disk_bytes_for_node(node);
                let ds = disk_color(disk_bytes, root_min, root_max);
                let ci = app.ci_for_node(node);
                let git = app.git_icon(project);
                if node.has_children() {
                    let arrow = if app.expanded.contains(&ExpandKey::Node(*node_index)) {
                        "▼ "
                    } else {
                        "▶ "
                    };
                    ListItem::new(project_row_spans(
                        arrow, &name, &disk, ds, &ci, git, name_width,
                    ))
                } else {
                    ListItem::new(project_row_spans(
                        "  ", &name, &disk, ds, &ci, git, name_width,
                    ))
                }
            },
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => {
                let group = &app.nodes[*node_index].groups[*group_index];
                let arrow = if app
                    .expanded
                    .contains(&ExpandKey::Group(*node_index, *group_index))
                {
                    "▼ "
                } else {
                    "▶ "
                };
                let prefix = format!("    {arrow}");
                let label = format!("{} ({})", group.name, group.members.len());
                ListItem::new(group_header_spans(&prefix, &label, name_width))
            },
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => {
                let group = &app.nodes[*node_index].groups[*group_index];
                let member = &group.members[*member_index];
                let name = member.display_name();
                let disk = app.formatted_disk(member);
                let disk_bytes = app.disk_usage.get(&member.path).copied();
                let (cmin, cmax) = child_ranges.get(node_index).copied().unwrap_or((0, 0));
                let ds = disk_color(disk_bytes, cmin, cmax);
                let ci = app.ci_for(member);
                let git = app.git_icon(member);
                let indent = if group.name.is_empty() {
                    "    "
                } else {
                    "        "
                };
                ListItem::new(project_row_spans(
                    indent, &name, &disk, ds, &ci, git, name_width,
                ))
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => {
                let wt = &app.nodes[*node_index].worktrees[*worktree_index];
                let name = wt
                    .project
                    .worktree_name
                    .as_deref()
                    .unwrap_or(&wt.project.path)
                    .to_string();
                let disk = app.formatted_disk(&wt.project);
                let disk_bytes = app.disk_usage.get(&wt.project.path).copied();
                let (cmin, cmax) = child_ranges.get(node_index).copied().unwrap_or((0, 0));
                let ds = disk_color(disk_bytes, cmin, cmax);
                let ci = app.ci_for(&wt.project);
                let git = app.git_icon(&wt.project);
                ListItem::new(project_row_spans(
                    "    ", &name, &disk, ds, &ci, git, name_width,
                ))
            },
        })
        .collect()
}

pub fn render_filtered_items(app: &App, name_width: usize) -> Vec<ListItem<'static>> {
    let (root_min, root_max) = root_disk_range(app);
    app.filtered
        .iter()
        .filter_map(|&flat_idx| {
            let entry = app.flat_entries.get(flat_idx)?;
            let node = app.nodes.get(entry.node_index)?;
            let project = node
                .groups
                .get(entry.group_index)
                .and_then(|g| g.members.get(entry.member_index))
                .unwrap_or(&node.project);
            let disk = app.formatted_disk(project);
            let disk_bytes = app.disk_usage.get(&project.path).copied();
            let ds = disk_color(disk_bytes, root_min, root_max);
            let ci = app.ci_for(project);
            let git = app.git_icon(project);
            Some(ListItem::new(project_row_spans(
                "  ",
                &entry.name,
                &disk,
                ds,
                &ci,
                git,
                name_width,
            )))
        })
        .collect()
}
