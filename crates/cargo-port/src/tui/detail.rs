use std::path::PathBuf;

use crossterm::event::KeyCode;
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
use toml_edit::DocumentMut;

use super::App;
use super::FocusTarget;
use super::ProjectCounts;
use super::advance_focus;
use super::render::CiColumn;
use super::render::conclusion_style;
use super::reverse_focus;
use crate::ci::CiRun;
use crate::project::ExampleGroup;
use crate::project::RustProject;

pub struct EditingState {
    pub field: DetailField,
    pub buf:   String,
}

pub enum RunTargetKind {
    Binary,
    Example,
    Bench,
}

pub struct PendingExampleRun {
    pub abs_path:     String,
    pub target_name:  String,
    pub package_name: Option<String>,
    pub kind:         RunTargetKind,
    pub release:      bool,
}

/// A pending request to fetch more CI runs for a project.
pub struct PendingCiFetch {
    pub abs_path:      String,
    pub project_path:  String,
    pub current_count: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DetailField {
    Name,
    Path,
    Types,
    Disk,
    Ci,
    Stats,
    Branch,
    Origin,
    Owner,
    Repo,
    Worktree,
    Vendored,
    Version,
    Description,
}

impl DetailField {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Path => "Path",
            Self::Types => "Types",
            Self::Disk => "Disk",
            Self::Ci => "CI",
            Self::Stats => "Stats",
            Self::Branch => "Branch",
            Self::Origin => "Origin",
            Self::Owner => "Owner",
            Self::Repo => "Repo",
            Self::Worktree => "Worktree",
            Self::Vendored => "Vendored",
            Self::Version => "Version",
            Self::Description => "Desc",
        }
    }

    pub(super) const fn is_editable(self) -> bool {
        matches!(self, Self::Version | Self::Description)
    }

    pub(super) const fn toml_key(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Description => "description",
            _ => "",
        }
    }

    pub(super) fn value(self, info: &DetailInfo) -> String {
        match self {
            Self::Name => info.name.clone(),
            Self::Path => info.path.clone(),
            Self::Types => info.types.clone(),
            Self::Disk => info.disk.clone(),
            Self::Ci => info.ci.clone(),
            Self::Stats => info.stats.clone(),
            Self::Branch => info.git_branch.as_deref().unwrap_or("").to_string(),
            Self::Origin => info.git_origin.as_deref().unwrap_or("").to_string(),
            Self::Owner => info.git_owner.as_deref().unwrap_or("").to_string(),
            Self::Repo => info.git_url.as_deref().unwrap_or("").to_string(),
            Self::Worktree => info.worktree_label.as_deref().unwrap_or("").to_string(),
            Self::Vendored => info.vendored_names.clone(),
            Self::Version => info.crates_version.as_ref().map_or_else(
                || info.version.clone(),
                |cv| format!("{} (crates.io: {cv})", info.version),
            ),
            Self::Description => info.description.as_deref().unwrap_or("—").to_string(),
        }
    }
}

/// All fields for the Project column: read-only info then editable at the bottom.
pub(super) fn project_fields(info: &DetailInfo) -> Vec<DetailField> {
    let mut fields = vec![
        DetailField::Name,
        DetailField::Path,
        DetailField::Types,
        DetailField::Disk,
        DetailField::Ci,
        DetailField::Stats,
    ];
    if !info.vendored_names.is_empty() {
        fields.push(DetailField::Vendored);
    }
    fields.push(DetailField::Version);
    fields.push(DetailField::Description);
    fields
}

/// Git fields (right column). Only includes fields that have data.
/// For worktree parents, skip Branch and Origin (those vary per worktree).
pub(super) fn git_fields(info: &DetailInfo) -> Vec<DetailField> {
    let is_worktree_parent = !info.worktree_names.is_empty();
    let mut fields = Vec::new();
    if !is_worktree_parent && info.git_branch.is_some() {
        fields.push(DetailField::Branch);
    }
    if info.worktree_label.is_some() {
        fields.push(DetailField::Worktree);
    }
    if !is_worktree_parent && info.git_origin.is_some() {
        fields.push(DetailField::Origin);
    }
    if info.git_url.is_some() {
        fields.push(DetailField::Repo);
    }
    if info.git_owner.is_some() {
        fields.push(DetailField::Owner);
    }
    fields
}

pub(super) struct DetailInfo {
    pub project_title:  String,
    pub name:           String,
    pub path:           String,
    pub version:        String,
    pub description:    Option<String>,
    pub crates_version: Option<String>,
    pub types:          String,
    pub disk:           String,
    pub ci:             String,
    pub stats:          String,
    pub git_branch:     Option<String>,
    pub git_origin:     Option<String>,
    pub git_owner:      Option<String>,
    pub git_url:        Option<String>,
    pub worktree_label: Option<String>,
    pub worktree_names: Vec<String>,
    pub vendored_names: String,
    pub is_binary:      bool,
    pub binary_name:    Option<String>,
    pub examples:       Vec<ExampleGroup>,
    pub benches:        Vec<String>,
}

/// Collect vendored crate names for a project from the node tree.
fn collect_vendored_names(app: &App, project: &RustProject) -> String {
    for node in &app.nodes {
        // Check the node itself
        if node.project.path == project.path && !node.vendored.is_empty() {
            return node
                .vendored
                .iter()
                .filter_map(|v| v.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
        }
        // Check worktrees
        for wt in &node.worktrees {
            if wt.project.path == project.path && !wt.vendored.is_empty() {
                return wt
                    .vendored
                    .iter()
                    .filter_map(|v| v.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
            }
        }
    }
    String::new()
}

#[allow(clippy::too_many_lines)]
pub(super) fn build_detail_info(app: &App, project: &RustProject) -> DetailInfo {
    let ws_counts = app.workspace_counts(project);
    let stats = ws_counts.as_ref().map_or_else(
        || {
            let mut parts: Vec<String> = Vec::new();
            if !project.example_count() == 0 {
                parts.push(format!("{} examples", project.example_count()));
            }
            if !project.benches.is_empty() {
                parts.push(format!("{} benches", project.benches.len()));
            }
            if project.test_count > 0 {
                parts.push(format!("{} tests", project.test_count));
            }
            if parts.is_empty() {
                "—".to_string()
            } else {
                parts.join("  ")
            }
        },
        ProjectCounts::summary,
    );

    let git = app.git_info.get(&project.path);
    let git_branch = git.and_then(|g| g.branch.clone());
    let git_origin = git.map(|g| format!("{} {}", g.origin.icon(), g.origin.label()));
    let git_owner = git.and_then(|g| g.owner.clone());
    let git_url = git.and_then(|g| g.url.clone());
    let crates_version = app.crates_versions.get(&project.path).cloned();
    let worktree_label = project.worktree_name.clone();

    // Aggregate disk and CI across worktrees when the selected node has them
    let worktree_node = app
        .selected_node()
        .filter(|n| n.project.path == project.path && !n.worktrees.is_empty());

    let (disk, ci) = worktree_node.map_or_else(
        || (app.formatted_disk(project), app.ci_for(project)),
        |node| (app.formatted_disk_for_node(node), app.ci_for_node(node)),
    );

    let project_title = if project.is_workspace() {
        "Workspace".to_string()
    } else {
        // Check if this project is under a workspace node
        let is_member = app.nodes.iter().any(|n| {
            n.project.is_workspace()
                && n.project.path != project.path
                && (n
                    .groups
                    .iter()
                    .any(|g| g.members.iter().any(|m| m.path == project.path))
                    || n.worktrees.iter().any(|wt| wt.project.path == project.path))
        });
        if is_member {
            "Workspace Member".to_string()
        } else {
            "Project".to_string()
        }
    };

    let worktree_names: Vec<String> = worktree_node.map_or_else(Vec::new, |node| {
        node.worktrees
            .iter()
            .map(|wt| {
                wt.project
                    .worktree_name
                    .as_deref()
                    .unwrap_or(&wt.project.path)
                    .to_string()
            })
            .collect()
    });

    // Collect vendored crate names for this project
    let vendored_names = collect_vendored_names(app, project);

    // Check if this project is a binary
    let is_binary = project
        .types
        .iter()
        .any(|t| matches!(t, crate::project::ProjectType::Binary));
    let binary_name = if is_binary {
        project.name.clone()
    } else {
        None
    };

    DetailInfo {
        project_title,
        name: project.name.clone().unwrap_or_else(|| "-".to_string()),
        path: project.path.clone(),
        version: project.version.clone().unwrap_or_else(|| "-".to_string()),
        description: project.description.clone(),
        types: project
            .types
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        disk,
        ci,
        stats,
        crates_version,
        git_branch,
        git_origin,
        git_owner,
        git_url,
        worktree_label,
        worktree_names,
        vendored_names,
        is_binary,
        binary_name,
        examples: project.examples.clone(),
        benches: project.benches.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_column(
    frame: &mut Frame,
    title: &str,
    app: &App,
    info: &DetailInfo,
    fields: &[DetailField],
    detail_focused: bool,
    is_active_column: bool,
    cursor: usize,
    highlight_style: Style,
    readonly_label_style: Style,
    editable_label_style: Style,
    area: Rect,
) {
    let title_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'static>> =
        vec![Line::from(Span::styled(format!("  {title}"), title_style))];
    for (i, field) in fields.iter().enumerate() {
        let label = field.label();
        let is_focused = detail_focused && is_active_column && i == cursor;

        // Editable field that is actively being edited
        if field.is_editable()
            && is_focused
            && let Some(editing) = app.editing.as_ref().filter(|e| e.field == *field)
        {
            let text = format!("{}_", editing.buf);
            lines.push(Line::from(vec![
                Span::styled(format!("  {label:<8} "), Style::default().fg(Color::Yellow)),
                Span::styled(text, Style::default().fg(Color::Yellow)),
            ]));
            continue;
        }

        let value = field.value(info);
        let base_label_style = if field.is_editable() {
            editable_label_style
        } else {
            readonly_label_style
        };
        let ls = if is_focused {
            highlight_style
        } else {
            base_label_style
        };
        let vs = if is_focused {
            highlight_style
        } else if *field == DetailField::Ci {
            conclusion_style(&info.ci)
        } else {
            Style::default()
        };

        // Word-wrap Description and Vendored across multiple lines
        if (*field == DetailField::Description || *field == DetailField::Vendored)
            && !value.is_empty()
        {
            let prefix = format!("  {label:<8} ");
            let prefix_len = prefix.len();
            let col_width = area.width as usize;
            let avail = col_width.saturating_sub(prefix_len);
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
                    Span::styled(format!("  {label:<8} "), ls),
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
    frame.render_widget(Paragraph::new(lines), area);
}

#[allow(clippy::too_many_arguments)]
fn render_git_column(
    frame: &mut Frame,
    app: &App,
    info: &DetailInfo,
    fields: &[DetailField],
    detail_focused: bool,
    is_active_column: bool,
    cursor: usize,
    highlight_style: Style,
    label_style: Style,
    area: Rect,
) {
    let title_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled("  Git", title_style))];

    for (i, field) in fields.iter().enumerate() {
        let label = field.label();
        let value = field.value(info);
        let is_focused = detail_focused && is_active_column && i == cursor;
        let ls = if is_focused {
            highlight_style
        } else {
            label_style
        };
        let vs = if is_focused {
            highlight_style
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {label:<8} "), ls),
            Span::styled(value, vs),
        ]));
    }

    // Worktree list for worktree parents
    if !info.worktree_names.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  Worktrees", title_style)));
        let wt_style = Style::default().fg(Color::DarkGray);
        for name in &info.worktree_names {
            lines.push(Line::from(Span::styled(format!("    {name}"), wt_style)));
        }
    }

    // Ignore the editing params — git column is read-only
    let _ = app;
    frame.render_widget(Paragraph::new(lines), area);
}

fn build_target_lines(app: &App, info: &DetailInfo, active: bool) -> Vec<Line<'static>> {
    let title_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let group_style = Style::default().fg(Color::Yellow);
    let name_style = Style::default();
    let highlight_style = Style::default().fg(Color::Black).bg(Color::Cyan);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut line_idx: usize = 0;

    // Binary section
    if info.is_binary
        && let Some(name) = &info.binary_name
    {
        lines.push(Line::from(Span::styled("  Binary", title_style)));
        line_idx += 1;
        let style = if active && line_idx == app.examples_scroll {
            highlight_style
        } else {
            name_style
        };
        lines.push(Line::from(Span::styled(format!("    {name}"), style)));
        line_idx += 1;
    }

    // Examples section
    if !info.examples.is_empty() {
        let total: usize = info.examples.iter().map(|g| g.names.len()).sum();
        lines.push(Line::from(Span::styled(
            format!("  Examples ({total})"),
            title_style,
        )));
        line_idx += 1;

        for group in &info.examples {
            if group.category.is_empty() {
                for name in &group.names {
                    let style = if active && line_idx == app.examples_scroll {
                        highlight_style
                    } else {
                        name_style
                    };
                    lines.push(Line::from(Span::styled(format!("    {name}"), style)));
                    line_idx += 1;
                }
            } else {
                let expanded = app.expanded_example_groups.contains(&group.category);
                let arrow = if expanded { "▼" } else { "▶" };
                let style = if active && line_idx == app.examples_scroll {
                    highlight_style
                } else {
                    group_style
                };
                lines.push(Line::from(Span::styled(
                    format!("  {arrow} {} ({})", group.category, group.names.len()),
                    style,
                )));
                line_idx += 1;
                if expanded {
                    for name in &group.names {
                        let style = if active && line_idx == app.examples_scroll {
                            highlight_style
                        } else {
                            name_style
                        };
                        lines.push(Line::from(Span::styled(format!("      {name}"), style)));
                        line_idx += 1;
                    }
                }
            }
        }
    }

    // Benches section
    if !info.benches.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  Benches ({})", info.benches.len()),
            title_style,
        )));
        line_idx += 1;

        for name in &info.benches {
            let style = if active && line_idx == app.examples_scroll {
                highlight_style
            } else {
                name_style
            };
            lines.push(Line::from(Span::styled(format!("    {name}"), style)));
            line_idx += 1;
        }
    }

    lines
}

fn render_targets_column(
    frame: &mut Frame,
    app: &App,
    info: &DetailInfo,
    detail_focused: bool,
    is_active_column: bool,
    area: Rect,
) {
    let active = detail_focused && is_active_column;
    let visible_lines = build_target_lines(app, info, active);

    // Scrollable view — keep cursor visible
    let visible_height = area.height as usize;
    let total_lines = visible_lines.len();
    let needs_indicator = total_lines > visible_height;
    let content_height = if needs_indicator {
        visible_height.saturating_sub(1)
    } else {
        visible_height
    };

    // Ensure the cursor is always visible within the content area
    let max_scroll = total_lines.saturating_sub(content_height);
    let viewport_start = if app.examples_scroll < content_height / 2 {
        0
    } else {
        (app.examples_scroll - content_height / 2).min(max_scroll)
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    for line in visible_lines
        .into_iter()
        .skip(viewport_start)
        .take(content_height)
    {
        lines.push(line);
    }

    if needs_indicator {
        let indicator = if viewport_start > 0 && viewport_start < max_scroll {
            format!("  ↑↓ {}/{total_lines}", app.examples_scroll + 1)
        } else if viewport_start > 0 {
            format!("  ↑ {}/{total_lines}", app.examples_scroll + 1)
        } else {
            format!("  ↓ {}/{total_lines}", app.examples_scroll + 1)
        };
        lines.push(Line::from(Span::styled(
            indicator,
            Style::default().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

pub(super) fn render_detail_panel(
    frame: &mut Frame,
    app: &App,
    detail_info: Option<&DetailInfo>,
    area: Rect,
) {
    let detail_focused = app.focus == FocusTarget::DetailFields;

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .title(" Details ")
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(if detail_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });

    if let Some(info) = detail_info {
        let detail_inner = detail_block.inner(area);
        frame.render_widget(detail_block, area);

        let git = git_fields(info);
        let has_git = !git.is_empty();
        let has_targets = info.is_binary || !info.examples.is_empty() || !info.benches.is_empty();

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(match (has_git, has_targets) {
                (true, true) => vec![
                    Constraint::Percentage(35),
                    Constraint::Percentage(30),
                    Constraint::Percentage(35),
                ],
                (true, false) | (false, true) => {
                    vec![Constraint::Percentage(50), Constraint::Percentage(50)]
                },
                (false, false) => vec![Constraint::Percentage(100)],
            })
            .split(detail_inner);

        let highlight_style = Style::default().fg(Color::Black).bg(Color::Cyan);
        let editable_label_style = Style::default().fg(Color::Cyan);
        let readonly_label_style = Style::default().fg(Color::DarkGray);

        // Left column: project info + editable fields
        render_column(
            frame,
            &info.project_title,
            app,
            info,
            &project_fields(info),
            detail_focused,
            app.detail_column == 0,
            app.detail_cursor,
            highlight_style,
            readonly_label_style,
            editable_label_style,
            columns[0],
        );

        // Git column
        let mut next_col = 1;
        if has_git {
            render_git_column(
                frame,
                app,
                info,
                &git,
                detail_focused,
                app.detail_column == next_col,
                app.detail_cursor,
                highlight_style,
                readonly_label_style,
                columns[next_col],
            );
            next_col += 1;
        }

        // Examples & benches column (scrollable)
        if has_targets {
            render_targets_column(
                frame,
                app,
                info,
                detail_focused,
                app.detail_column == next_col,
                columns[next_col],
            );
        }
    } else {
        let content = vec![Line::from("  No project selected")];
        let detail = Paragraph::new(content).block(detail_block);
        frame.render_widget(detail, area);
    }
}

/// Format ISO 8601 timestamp as `yyyy-mm-dd hh:mm`.
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_frame(tick: usize) -> &'static str {
    // Divide tick to slow down the spinner (renders at ~60fps, we want ~10fps spin)
    SPINNER_FRAMES[(tick / 6) % SPINNER_FRAMES.len()]
}

fn format_timestamp(iso: &str) -> String {
    let stripped = iso.trim_end_matches('Z');
    match stripped.split_once('T') {
        Some((date, time)) => {
            let short_time = if time.len() >= 5 { &time[..5] } else { time };
            format!("{date} {short_time}")
        },
        None => stripped.to_string(),
    }
}

/// The number of extra rows beyond the CI run data (the "fetch more" action row).
pub(super) const CI_EXTRA_ROWS: usize = 1;

#[allow(clippy::too_many_lines)]
pub(super) fn render_ci_panel(frame: &mut Frame, app: &App, ci_runs: &[CiRun], area: Rect) {
    let ci_focused = app.focus == FocusTarget::CiRuns;

    let title = if app.ci_fetching {
        let spinner = spinner_frame(app.spinner_tick);
        format!(" CI Runs {spinner} fetching {} more… ", app.ci_fetch_count)
    } else {
        " CI Runs ".to_string()
    };

    let ci_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(if ci_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });

    let has_bench = ci_runs
        .iter()
        .any(|r| r.jobs.iter().any(|j| CiColumn::Bench.matches(&j.name)));

    let mut cols: Vec<CiColumn> = vec![
        CiColumn::Fmt,
        CiColumn::Taplo,
        CiColumn::Clippy,
        CiColumn::Mend,
        CiColumn::Build,
        CiColumn::Test,
    ];
    if has_bench {
        cols.push(CiColumn::Bench);
    }

    // Header
    let right_aligned = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(Color::DarkGray);
    let mut header_cells = vec![
        Cell::from("#").style(right_aligned),
        Cell::from("Branch").style(right_aligned),
        Cell::from("Timestamp").style(right_aligned),
    ];
    for col in &cols {
        header_cells.push(
            Cell::from(Line::from(col.label()).alignment(ratatui::layout::Alignment::Right))
                .style(right_aligned),
        );
    }
    header_cells.push(
        Cell::from(Line::from("Total").alignment(ratatui::layout::Alignment::Right))
            .style(right_aligned),
    );
    let header = Row::new(header_cells).bottom_margin(0);

    // Data rows
    let mut rows: Vec<Row> = ci_runs
        .iter()
        .enumerate()
        .map(|(i, ci_run)| {
            let timestamp = format_timestamp(&ci_run.created_at);
            let branch = &ci_run.branch;

            let total_dur = ci_run
                .wall_clock_secs
                .map_or_else(|| "—".to_string(), crate::ci::format_secs);

            let row_num = format!("{}", i + 1);
            let mut cells = vec![
                Cell::from(row_num).style(Style::default().fg(Color::DarkGray)),
                Cell::from(branch.clone()),
                Cell::from(timestamp),
            ];

            for col in &cols {
                let job = ci_run.jobs.iter().find(|j| col.matches(&j.name));
                if let Some(j) = job {
                    let text = format!("{} {}", j.duration, j.conclusion);
                    cells.push(
                        Cell::from(Line::from(text).alignment(ratatui::layout::Alignment::Right))
                            .style(conclusion_style(&j.conclusion)),
                    );
                } else {
                    cells.push(
                        Cell::from(Line::from("—").alignment(ratatui::layout::Alignment::Right))
                            .style(Style::default().fg(Color::DarkGray)),
                    );
                }
            }

            // Total column
            let total_text = format!("{total_dur} {}", ci_run.conclusion);
            cells.push(
                Cell::from(Line::from(total_text).alignment(ratatui::layout::Alignment::Right))
                    .style(conclusion_style(&ci_run.conclusion)),
            );

            Row::new(cells)
        })
        .collect();

    // Column widths
    let mut widths = vec![
        Constraint::Length(3),  // # row number
        Constraint::Fill(1),    // Branch — takes leftover space
        Constraint::Length(16), // Timestamp (yyyy-mm-dd hh:mm)
    ];
    for _ in &cols {
        widths.push(Constraint::Min(8));
    }
    widths.push(Constraint::Min(8)); // Total

    // "Fetch more" / "no older runs" as a table row
    let no_more = app
        .selected_project()
        .is_some_and(|p| app.ci_no_more_runs.contains(&p.path));
    let fetch_label = if app.ci_fetching {
        let spinner = spinner_frame(app.spinner_tick);
        format!("{spinner} fetching {} more…", app.ci_fetch_count)
    } else if no_more {
        "— no older runs".to_string()
    } else {
        "↓ fetch more runs".to_string()
    };
    let fetch_style = if no_more {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let num_cols = widths.len();
    let mut fetch_cells: Vec<Cell> = vec![
        Cell::from("").style(fetch_style),
        Cell::from(fetch_label).style(fetch_style),
    ];
    for _ in 2..num_cols {
        fetch_cells.push(Cell::from(""));
    }
    rows.push(Row::new(fetch_cells));

    let highlight_style = if ci_focused {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default()
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(ci_block)
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let mut table_state = TableState::default().with_selected(Some(app.ci_runs_cursor));
    frame.render_stateful_widget(table, area, &mut table_state);
}

/// Returns the maximum column index (0 if no git info, 1 if git info present).
/// Column layout: 0=Project, then optionally Git, then optionally targets (examples/benches).
/// Returns (`max_column_index`, `targets_column_index` or `None`).
fn detail_layout(app: &App) -> (usize, Option<usize>) {
    let project = app.selected_project();
    let has_git = project.and_then(|p| app.git_info.get(&p.path)).is_some();
    let has_targets = project.is_some_and(|p| {
        p.types
            .iter()
            .any(|t| matches!(t, crate::project::ProjectType::Binary))
            || !p.examples.is_empty()
            || !p.benches.is_empty()
    });

    let mut col = 0; // Project is always column 0
    if has_git {
        col += 1;
    }
    let examples_col = if has_targets {
        col += 1;
        Some(col)
    } else {
        None
    };
    (col, examples_col)
}

/// Returns the field count for a given column index.
/// Returns 0 for the examples column (it uses scroll, not cursor).
fn detail_column_field_count(app: &App, column: usize) -> usize {
    let (_, examples_col) = detail_layout(app);
    if Some(column) == examples_col {
        return 0; // Examples column uses scroll, not cursor
    }
    if column == 0 {
        app.selected_project().map_or(0, |p| {
            let info = build_detail_info(app, p);
            project_fields(&info).len()
        })
    } else {
        // Git column
        app.selected_project().map_or(0, |p| {
            let info = build_detail_info(app, p);
            git_fields(&info).len()
        })
    }
}

/// Clamp the detail cursor to the current column's field count.
fn clamp_detail_cursor(app: &mut App) {
    let count = detail_column_field_count(app, app.detail_column);
    if count > 0 && app.detail_cursor >= count {
        app.detail_cursor = count - 1;
    }
}

/// What the cursor is pointing at in the targets column.
enum TargetItem {
    /// The binary itself.
    Binary(String),
    /// A root-level example (no category).
    RootExample(String),
    /// A category group header.
    GroupHeader(String),
    /// An example inside a category.
    GroupExample { category: String, name: String },
    /// A bench target.
    Bench(String),
}

/// Identify what's at the given scroll position in the targets column.
fn target_item_at(app: &App, scroll: usize) -> Option<TargetItem> {
    let project = app.selected_project()?;
    let info = build_detail_info(app, project);
    let mut line = 0;

    // Binary section
    if info.is_binary
        && let Some(name) = &info.binary_name
    {
        line += 1; // "Binary" header
        if line == scroll {
            return Some(TargetItem::Binary(name.clone()));
        }
        line += 1;
    }

    // Examples section
    if !project.examples.is_empty() {
        line += 1; // "Examples (N)" header
        for group in &project.examples {
            if group.category.is_empty() {
                for name in &group.names {
                    if line == scroll {
                        return Some(TargetItem::RootExample(name.clone()));
                    }
                    line += 1;
                }
            } else {
                if line == scroll {
                    return Some(TargetItem::GroupHeader(group.category.clone()));
                }
                line += 1;
                if app.expanded_example_groups.contains(&group.category) {
                    for name in &group.names {
                        if line == scroll {
                            return Some(TargetItem::GroupExample {
                                category: group.category.clone(),
                                name:     name.clone(),
                            });
                        }
                        line += 1;
                    }
                }
            }
        }
    }

    // Benches section
    if !project.benches.is_empty() {
        line += 1; // "Benches (N)" header
        for name in &project.benches {
            if line == scroll {
                return Some(TargetItem::Bench(name.clone()));
            }
            line += 1;
        }
    }

    None
}

/// Count total visible lines in the targets column.
fn targets_visible_line_count(app: &App) -> usize {
    let Some(project) = app.selected_project() else {
        return 0;
    };
    let info = build_detail_info(app, project);
    let mut count = 0;

    // Binary
    if info.is_binary && info.binary_name.is_some() {
        count += 2; // header + name
    }

    // Examples
    if !project.examples.is_empty() {
        count += 1; // header
        for group in &project.examples {
            if group.category.is_empty() {
                count += group.names.len();
            } else {
                count += 1; // Group header
                if app.expanded_example_groups.contains(&group.category) {
                    count += group.names.len();
                }
            }
        }
    }

    // Benches
    if !project.benches.is_empty() {
        count += 1; // header
        count += project.benches.len();
    }

    count
}

/// Check if a line is a selectable target item (not a section header).
fn is_selectable_target_line(app: &App, line: usize) -> bool { target_item_at(app, line).is_some() }

/// Find the first selectable line in the targets column.
fn first_selectable_target(app: &App) -> usize {
    let total = targets_visible_line_count(app);
    for i in 0..total {
        if is_selectable_target_line(app, i) {
            return i;
        }
    }
    0
}

/// Find the next selectable line after `from` (going down).
fn next_selectable_target(app: &App, from: usize) -> usize {
    let total = targets_visible_line_count(app);
    for i in (from + 1)..total {
        if is_selectable_target_line(app, i) {
            return i;
        }
    }
    from
}

/// Find the previous selectable line before `from` (going up).
fn prev_selectable_target(app: &App, from: usize) -> usize {
    for i in (0..from).rev() {
        if is_selectable_target_line(app, i) {
            return i;
        }
    }
    from
}

/// Expand the group at the current scroll position.
fn expand_example_group(app: &mut App) {
    if let Some(TargetItem::GroupHeader(cat)) = target_item_at(app, app.examples_scroll) {
        app.expanded_example_groups.insert(cat);
    }
}

/// Collapse the group at the current scroll position (or parent group if on a child).
/// Returns `true` if something was collapsed.
fn collapse_example_group(app: &mut App) -> bool {
    match target_item_at(app, app.examples_scroll) {
        Some(TargetItem::GroupHeader(ref cat)) if app.expanded_example_groups.contains(cat) => {
            app.expanded_example_groups.remove(cat);
            true
        },
        Some(TargetItem::GroupExample { category, .. }) => {
            app.expanded_example_groups.remove(&category);
            // Move scroll back to the group header
            let Some(project) = app.selected_project() else {
                return true;
            };
            let mut line = 0;
            for group in &project.examples {
                if group.category.is_empty() {
                    line += group.names.len();
                } else {
                    if group.category == category {
                        app.examples_scroll = line;
                        return true;
                    }
                    line += 1;
                    if app.expanded_example_groups.contains(&group.category) {
                        line += group.names.len();
                    }
                }
            }
            true
        },
        _ => false,
    }
}

/// Launch the selected target, or toggle a group header.
fn run_selected_target(app: &mut App, kind: RunTargetKind, name: String, release: bool) {
    if let Some(project) = app.selected_project() {
        app.pending_example_run = Some(PendingExampleRun {
            abs_path: project.abs_path.clone(),
            target_name: name,
            package_name: project.name.clone(),
            kind,
            release,
        });
    }
}

fn handle_target_action(app: &mut App, release: bool) {
    match target_item_at(app, app.examples_scroll) {
        Some(TargetItem::GroupHeader(cat)) if !release => {
            if app.expanded_example_groups.contains(&cat) {
                app.expanded_example_groups.remove(&cat);
            } else {
                app.expanded_example_groups.insert(cat);
            }
        },
        Some(TargetItem::RootExample(name) | TargetItem::GroupExample { name, .. }) => {
            run_selected_target(app, RunTargetKind::Example, name, release);
        },
        Some(TargetItem::Bench(name)) => {
            run_selected_target(app, RunTargetKind::Bench, name, release);
        },
        Some(TargetItem::Binary(name)) => {
            run_selected_target(app, RunTargetKind::Binary, name, release);
        },
        _ => {},
    }
}

pub(super) fn handle_detail_key(app: &mut App, key: KeyCode) {
    let (max_col, examples_col) = detail_layout(app);
    let on_examples = Some(app.detail_column) == examples_col;
    let field_count = detail_column_field_count(app, app.detail_column);

    match key {
        KeyCode::Up => {
            if on_examples {
                app.examples_scroll = prev_selectable_target(app, app.examples_scroll);
            } else if app.detail_cursor > 0 {
                app.detail_cursor -= 1;
            }
        },
        KeyCode::Down => {
            if on_examples {
                app.examples_scroll = next_selectable_target(app, app.examples_scroll);
            } else if field_count > 0 && app.detail_cursor < field_count - 1 {
                app.detail_cursor += 1;
            }
        },
        KeyCode::Left => {
            if on_examples && collapse_example_group(app) {
                // Collapsed an example group — stay in examples column
            } else if app.detail_column > 0 {
                app.detail_column -= 1;
                clamp_detail_cursor(app);
            }
        },
        KeyCode::Right => {
            if on_examples {
                expand_example_group(app);
            } else if app.detail_column < max_col {
                app.detail_column += 1;
                // If entering the targets column, jump to the first selectable item
                if Some(app.detail_column) == examples_col {
                    app.examples_scroll = first_selectable_target(app);
                } else {
                    clamp_detail_cursor(app);
                }
            }
        },
        KeyCode::Enter => {
            if on_examples {
                handle_target_action(app, false);
            } else if app.detail_column == 0 {
                let fields = app
                    .selected_project()
                    .map(|p| {
                        let info = build_detail_info(app, p);
                        project_fields(&info)
                    })
                    .unwrap_or_default();
                if let Some(field) = fields.get(app.detail_cursor)
                    && field.is_editable()
                    && let Some(project) = app.selected_project()
                {
                    match *field {
                        DetailField::Version => {
                            let version = project.version.clone().unwrap_or_default();
                            if version != "(workspace)" {
                                app.editing = Some(EditingState {
                                    field: DetailField::Version,
                                    buf:   version,
                                });
                            }
                        },
                        DetailField::Description => {
                            let desc = project.description.clone().unwrap_or_default();
                            app.editing = Some(EditingState {
                                field: DetailField::Description,
                                buf:   desc,
                            });
                        },
                        _ => {},
                    }
                }
            }
        },
        KeyCode::Char('r') => {
            if on_examples {
                handle_target_action(app, true);
            }
        },
        KeyCode::Tab => advance_focus(app),
        KeyCode::BackTab => reverse_focus(app),
        KeyCode::Esc => {
            app.focus = FocusTarget::ProjectList;
        },
        KeyCode::Char('q') => app.should_quit = true,
        _ => {},
    }
}

pub(super) fn handle_ci_runs_key(app: &mut App, key: KeyCode) {
    let run_count = app
        .selected_project()
        .and_then(|p| app.ci_runs_for(p))
        .map_or(0, Vec::len);
    // Total rows = run data rows + the "fetch more" action row
    let total_rows = run_count + CI_EXTRA_ROWS;

    match key {
        KeyCode::Up => {
            if app.ci_runs_cursor > 0 {
                app.ci_runs_cursor -= 1;
            }
        },
        KeyCode::Down => {
            if total_rows > 0 && app.ci_runs_cursor < total_rows - 1 {
                app.ci_runs_cursor += 1;
            }
        },
        KeyCode::Home => {
            app.ci_runs_cursor = 0;
        },
        KeyCode::End => {
            if total_rows > 0 {
                app.ci_runs_cursor = total_rows - 1;
            }
        },
        KeyCode::Enter => {
            // If cursor is on the "fetch more" row, trigger a background fetch
            if app.ci_runs_cursor == run_count
                && !app.ci_fetching
                && let Some(project) = app.selected_project()
                && !app.ci_no_more_runs.contains(&project.path)
            {
                #[allow(clippy::cast_possible_truncation)]
                let current_count = run_count as u32;
                app.pending_ci_fetch = Some(PendingCiFetch {
                    abs_path: project.abs_path.clone(),
                    project_path: project.path.clone(),
                    current_count,
                });
            }
        },
        KeyCode::Tab => advance_focus(app),
        KeyCode::BackTab => reverse_focus(app),
        KeyCode::Esc => {
            app.focus = FocusTarget::ProjectList;
        },
        KeyCode::Char('c') => {
            if let Some(project) = app.selected_project() {
                let path = project.path.clone();
                clear_ci_cache(app, &path);
            }
        },
        KeyCode::Char('q') => app.should_quit = true,
        _ => {},
    }
}

/// Clear CI cache for a project and remove its runs from the app.
fn clear_ci_cache(app: &mut App, project_path: &str) {
    use crate::ci::parse_owner_repo;

    // Find abs_path to derive repo URL
    let abs_path = app
        .nodes
        .iter()
        .find(|n| n.project.path == project_path)
        .map(|n| n.project.abs_path.clone());

    if let Some(abs_path) = abs_path
        && let Some(repo_url) = crate::ci::get_repo_url(std::path::Path::new(&abs_path))
        && let Some((owner, repo)) = parse_owner_repo(&repo_url)
        && let Some(dir) = super::scan::repo_cache_dir_pub(&owner, &repo)
    {
        let _ = std::fs::remove_dir_all(dir);
    }

    // Insert empty vec so the CI panel stays visible with the "fetch more" row
    app.ci_runs.insert(project_path.to_string(), Vec::new());
    app.ci_no_more_runs.remove(project_path);
    app.ci_runs_cursor = 0;
}

pub(super) fn handle_field_edit_key(app: &mut App, key: KeyCode) {
    let Some(editing) = app.editing.as_mut() else {
        return;
    };

    match key {
        KeyCode::Enter => {
            let field = editing.field;
            let new_value = editing.buf.clone();
            app.editing = None;
            if let Some(result) = write_toml_field(app, field, &new_value)
                && result.is_ok()
            {
                update_project_field(app, field, &new_value);
            }
        },
        KeyCode::Esc => {
            app.editing = None;
        },
        KeyCode::Backspace => {
            editing.buf.pop();
        },
        KeyCode::Char(c) => {
            editing.buf.push(c);
        },
        _ => {},
    }
}

/// Word-wrap text to fit within `max_width` characters, breaking at word boundaries.
fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            if word.len() > max_width {
                // Single word longer than the line — just push it
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

fn apply_field_update(project: &mut RustProject, field: DetailField, value: &str) {
    match field {
        DetailField::Version => project.version = Some(value.to_string()),
        DetailField::Description => project.description = Some(value.to_string()),
        _ => {},
    }
}

pub(super) fn write_toml_field(
    app: &App,
    field: DetailField,
    value: &str,
) -> Option<Result<(), String>> {
    let project = app.selected_project()?;
    let abs_path = PathBuf::from(&project.abs_path).join("Cargo.toml");
    let contents = match std::fs::read_to_string(&abs_path) {
        Ok(c) => c,
        Err(e) => return Some(Err(format!("Failed to read {}: {e}", abs_path.display()))),
    };
    let mut doc: DocumentMut = match contents.parse() {
        Ok(d) => d,
        Err(e) => return Some(Err(format!("Failed to parse TOML: {e}"))),
    };
    doc["package"][field.toml_key()] = toml_edit::value(value);
    if let Err(e) = std::fs::write(&abs_path, doc.to_string()) {
        return Some(Err(format!("Failed to write {}: {e}", abs_path.display())));
    }

    // Run taplo fmt on the edited file
    let _ = std::process::Command::new("taplo")
        .args(["fmt", &abs_path.to_string_lossy()])
        .output();

    Some(Ok(()))
}

pub(super) fn update_project_field(app: &mut App, field: DetailField, new_value: &str) {
    let project_path = match app.selected_project() {
        Some(p) => p.path.clone(),
        None => return,
    };

    for p in &mut app.all_projects {
        if p.path == project_path {
            apply_field_update(p, field, new_value);
        }
    }

    for node in &mut app.nodes {
        if node.project.path == project_path {
            apply_field_update(&mut node.project, field, new_value);
        }
        for group in &mut node.groups {
            for member in &mut group.members {
                if member.path == project_path {
                    apply_field_update(member, field, new_value);
                }
            }
        }
    }
}
