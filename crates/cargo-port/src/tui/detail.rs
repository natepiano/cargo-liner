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

#[derive(Clone, Copy)]
pub enum RunTargetKind {
    Binary,
    Example,
    Bench,
}

pub struct TargetEntry {
    pub name: String,
    pub kind: RunTargetKind,
}

/// Build a flat list of all runnable targets: binaries first, then examples alphabetically,
/// then benches alphabetically.
pub fn build_target_list(info: &DetailInfo) -> Vec<TargetEntry> {
    let mut entries = Vec::new();

    if info.is_binary
        && let Some(name) = &info.binary_name
    {
        entries.push(TargetEntry {
            name: name.clone(),
            kind: RunTargetKind::Binary,
        });
    }

    let mut example_names: Vec<String> = info
        .examples
        .iter()
        .flat_map(|g| g.names.iter().cloned())
        .collect();
    example_names.sort();
    for name in example_names {
        entries.push(TargetEntry {
            name,
            kind: RunTargetKind::Example,
        });
    }

    let mut bench_names = info.benches.clone();
    bench_names.sort();
    for name in bench_names {
        entries.push(TargetEntry {
            name,
            kind: RunTargetKind::Bench,
        });
    }

    entries
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
    Targets,
    Disk,
    Ci,
    Stats,
    Branch,
    Origin,
    Owner,
    Repo,
    Stars,
    Worktree,
    Vendored,
    CratesIo,
    Version,
    Description,
}

impl DetailField {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Path => "Path",
            Self::Targets => "Targets",
            Self::Disk => "Disk",
            Self::Ci => "CI",
            Self::Stats => "Stats",
            Self::Branch => "Branch",
            Self::Origin => "Origin",
            Self::Owner => "Owner",
            Self::Repo => "Repo",
            Self::Stars => "Stars",
            Self::Worktree => "Worktree",
            Self::Vendored => "Vendored",
            Self::CratesIo => "Crate",
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
            Self::Targets => info.types.clone(),
            Self::Disk => info.disk.clone(),
            Self::Ci => info.ci.clone(),
            Self::Stats => info.stats.clone(),
            Self::Branch => info.git_branch.as_deref().unwrap_or("").to_string(),
            Self::Origin => info.git_origin.as_deref().unwrap_or("").to_string(),
            Self::Owner => info.git_owner.as_deref().unwrap_or("").to_string(),
            Self::Repo => info.git_url.as_deref().unwrap_or("").to_string(),
            Self::Stars => info
                .git_stars
                .map_or_else(String::new, |c| format!("⭐ {c}")),
            Self::Worktree => info.worktree_label.as_deref().unwrap_or("").to_string(),
            Self::Vendored => info.vendored_names.clone(),
            Self::CratesIo => info.crates_version.as_deref().unwrap_or("").to_string(),
            Self::Version => info.version.clone(),
            Self::Description => info.description.as_deref().unwrap_or("—").to_string(),
        }
    }
}

/// All fields for the `Package` column: read-only info then editable at the bottom.
/// Non-Rust projects show only name, path, disk, and CI.
pub(super) fn package_fields(info: &DetailInfo) -> Vec<DetailField> {
    if !info.is_rust {
        return vec![
            DetailField::Name,
            DetailField::Path,
            DetailField::Disk,
            DetailField::Ci,
        ];
    }
    let mut fields = vec![
        DetailField::Name,
        DetailField::Path,
        DetailField::Targets,
        DetailField::Disk,
        DetailField::Ci,
    ];
    if info.stats_rows.is_empty() {
        fields.push(DetailField::Stats);
    }
    if !info.vendored_names.is_empty() {
        fields.push(DetailField::Vendored);
    }
    if info.crates_version.is_some() {
        fields.push(DetailField::CratesIo);
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
    if info.git_stars.is_some() {
        fields.push(DetailField::Stars);
    }
    fields
}

#[derive(Clone)]
pub struct DetailInfo {
    pub package_title:  String,
    pub name:           String,
    pub path:           String,
    pub version:        String,
    pub description:    Option<String>,
    pub crates_version: Option<String>,
    pub types:          String,
    pub disk:           String,
    pub ci:             String,
    pub stats:          String,
    pub stats_rows:     Vec<(&'static str, usize)>,
    pub git_branch:     Option<String>,
    pub git_origin:     Option<String>,
    pub git_owner:      Option<String>,
    pub git_url:        Option<String>,
    pub git_stars:      Option<u64>,
    pub worktree_label: Option<String>,
    pub worktree_names: Vec<String>,
    pub vendored_names: String,
    pub is_binary:      bool,
    pub binary_name:    Option<String>,
    pub examples:       Vec<ExampleGroup>,
    pub benches:        Vec<String>,
    /// Whether this is a Rust project (has `Cargo.toml`).
    pub is_rust:        bool,
    /// Whether the current user owns this project (git owner matches `owned_owners`).
    pub owned:          bool,
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

/// Resolve the title shown in the `Package` column header.
fn resolve_package_title(app: &App, project: &RustProject) -> String {
    if !project.is_rust {
        return "Project".to_string();
    }
    if project.is_workspace() {
        return "Workspace".to_string();
    }
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
        "Package".to_string()
    }
}

pub(super) fn build_detail_info(app: &App, project: &RustProject) -> DetailInfo {
    let ws_counts = app.workspace_counts(project);
    let stats_rows = ws_counts
        .as_ref()
        .map(ProjectCounts::to_rows)
        .unwrap_or_default();
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
    let owned = git_owner
        .as_ref()
        .is_some_and(|owner| app.owned_owners.iter().any(|o| o == owner));
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

    let package_title = resolve_package_title(app, project);

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
        package_title,
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
        stats_rows,
        crates_version,
        git_branch,
        git_origin,
        git_owner,
        git_url,
        git_stars: app.stars.get(&project.path).copied(),
        worktree_label,
        worktree_names,
        vendored_names,
        is_binary,
        binary_name,
        examples: project.examples.clone(),
        benches: project.benches.clone(),
        is_rust: project.is_rust,
        owned,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_column_inner(
    frame: &mut Frame,
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
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        let label = field.label();
        let is_focused = detail_focused && is_active_column && i == cursor;

        // Editable field that is actively being edited
        if field.is_editable()
            && info.owned
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
        let base_label_style = if field.is_editable() && info.owned {
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
fn render_git_column_inner(
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
    let mut lines: Vec<Line<'static>> = Vec::new();

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
        let wt_title_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        lines.push(Line::from(Span::styled("  Worktrees", wt_title_style)));
        let wt_style = Style::default().fg(Color::DarkGray);
        for name in &info.worktree_names {
            lines.push(Line::from(Span::styled(format!("    {name}"), wt_style)));
        }
    }

    // Ignore the editing params — git column is read-only
    let _ = app;
    frame.render_widget(Paragraph::new(lines), area);
}

pub(super) fn render_detail_panel(
    frame: &mut Frame,
    app: &App,
    detail_info: Option<&DetailInfo>,
    area: Rect,
) {
    let detail_focused = app.focus == FocusTarget::DetailFields;
    let title_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    if let Some(info) = detail_info {
        let git = git_fields(info);
        let has_git = !git.is_empty();
        let has_targets = info.is_binary || !info.examples.is_empty() || !info.benches.is_empty();

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(if has_git {
                vec![
                    Constraint::Min(30),
                    Constraint::Min(25),
                    Constraint::Fill(1),
                ]
            } else {
                vec![Constraint::Min(35), Constraint::Fill(1)]
            })
            .split(area);

        let highlight_style = Style::default().fg(Color::Black).bg(Color::Cyan);
        let editable_label_style = Style::default().fg(Color::Cyan);
        let readonly_label_style = Style::default().fg(Color::DarkGray);

        let active_border = Style::default().fg(Color::Cyan);
        let inactive_border = Style::default();

        render_project_panel(
            frame,
            app,
            info,
            detail_focused,
            highlight_style,
            readonly_label_style,
            editable_label_style,
            active_border,
            inactive_border,
            title_style,
            columns[0],
        );

        let mut next_col = 1;
        if has_git {
            render_git_panel(
                frame,
                app,
                info,
                &git,
                detail_focused,
                next_col,
                highlight_style,
                readonly_label_style,
                active_border,
                inactive_border,
                title_style,
                columns[next_col],
            );
            next_col += 1;
        }

        if has_targets {
            render_targets_panel(
                frame,
                app,
                info,
                detail_focused,
                next_col,
                active_border,
                inactive_border,
                title_style,
                columns[next_col],
            );
        } else {
            // Empty targets pane — greyed out
            let empty_targets = Block::default()
                .borders(Borders::ALL)
                .title(" No Targets ")
                .title_style(Style::default().fg(Color::DarkGray))
                .border_style(Style::default().fg(Color::DarkGray));
            frame.render_widget(empty_targets, columns[next_col]);
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

#[allow(clippy::too_many_arguments)]
fn render_project_panel(
    frame: &mut Frame,
    app: &App,
    info: &DetailInfo,
    detail_focused: bool,
    highlight_style: Style,
    readonly_label_style: Style,
    editable_label_style: Style,
    active_border: Style,
    inactive_border: Style,
    title_style: Style,
    area: Rect,
) {
    let project_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", info.package_title))
        .title_style(title_style)
        .border_style(if detail_focused && app.detail_column == 0 {
            active_border
        } else {
            inactive_border
        });
    let project_inner = project_block.inner(area);
    frame.render_widget(project_block, area);

    if info.stats_rows.is_empty() {
        render_column_inner(
            frame,
            app,
            info,
            &package_fields(info),
            detail_focused,
            app.detail_column == 0,
            app.detail_cursor,
            highlight_style,
            readonly_label_style,
            editable_label_style,
            project_inner,
        );
    } else {
        // Compute stats column width: 4 (number) + 1 (space) + longest label + 1 (border)
        #[allow(clippy::cast_possible_truncation)]
        let max_label_len = info
            .stats_rows
            .iter()
            .map(|(label, _)| label.len())
            .max()
            .unwrap_or(2) as u16;
        let stats_width = 4 + 1 + max_label_len + 1; // num + space + label + border

        // Split into fields (left) and stats column (right) with border
        let sub_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(stats_width)])
            .split(project_inner);

        render_column_inner(
            frame,
            app,
            info,
            &package_fields(info),
            detail_focused,
            app.detail_column == 0,
            app.detail_cursor,
            highlight_style,
            readonly_label_style,
            editable_label_style,
            sub_cols[0],
        );

        // Stats column with left border
        let stats_block = Block::default().borders(Borders::LEFT);
        let stats_inner = stats_block.inner(sub_cols[1]);
        frame.render_widget(stats_block, sub_cols[1]);

        let stat_label_style = Style::default().fg(Color::DarkGray);
        let stat_num_style = Style::default().fg(Color::Yellow);
        let mut stat_lines: Vec<Line<'static>> = Vec::new();
        for &(label, count) in &info.stats_rows {
            stat_lines.push(Line::from(vec![
                Span::styled(format!("{count:>4} "), stat_num_style),
                Span::styled(label, stat_label_style),
            ]));
        }
        frame.render_widget(Paragraph::new(stat_lines), stats_inner);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_git_panel(
    frame: &mut Frame,
    app: &App,
    info: &DetailInfo,
    git: &[DetailField],
    detail_focused: bool,
    col: usize,
    highlight_style: Style,
    readonly_label_style: Style,
    active_border: Style,
    inactive_border: Style,
    title_style: Style,
    area: Rect,
) {
    let git_block = Block::default()
        .borders(Borders::ALL)
        .title(" Git ")
        .title_style(title_style)
        .border_style(if detail_focused && app.detail_column == col {
            active_border
        } else {
            inactive_border
        });
    let git_inner = git_block.inner(area);
    frame.render_widget(git_block, area);
    render_git_column_inner(
        frame,
        app,
        info,
        git,
        detail_focused,
        app.detail_column == col,
        app.detail_cursor,
        highlight_style,
        readonly_label_style,
        git_inner,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_targets_panel(
    frame: &mut Frame,
    app: &App,
    info: &DetailInfo,
    detail_focused: bool,
    col: usize,
    active_border: Style,
    inactive_border: Style,
    title_style: Style,
    area: Rect,
) {
    let bin_count: usize = usize::from(info.is_binary);
    let ex_count: usize = info.examples.iter().map(|g| g.names.len()).sum();
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

    let is_active = detail_focused && app.detail_column == col;
    let targets_block = Block::default()
        .borders(Borders::ALL)
        .title(targets_title)
        .title_style(title_style)
        .border_style(if is_active {
            active_border
        } else {
            inactive_border
        });

    let entries = build_target_list(info);

    let type_style = Style::default().fg(Color::DarkGray);
    let rows: Vec<Row> = entries
        .iter()
        .map(|entry| {
            let kind_label = match entry.kind {
                RunTargetKind::Binary => "bin",
                RunTargetKind::Example => "example",
                RunTargetKind::Bench => "bench",
            };
            Row::new(vec![
                Cell::from(entry.name.clone()),
                Cell::from(Line::from(kind_label).alignment(ratatui::layout::Alignment::Right))
                    .style(type_style),
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
        Some(app.examples_scroll)
    } else {
        None
    };
    let mut table_state = TableState::default().with_selected(selected);
    frame.render_stateful_widget(table, area, &mut table_state);
}

/// Format ISO 8601 timestamp as `yyyy-mm-dd hh:mm`.
/// Get the local UTC offset in seconds (e.g., -28800 for PST).
fn local_utc_offset_secs() -> i64 {
    use std::sync::OnceLock;
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        use std::process::Command;
        Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.len() >= 5 {
                    let sign: i64 = if s.starts_with('-') { -1 } else { 1 };
                    let hours: i64 = s[1..3].parse().ok()?;
                    let mins: i64 = s[3..5].parse().ok()?;
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

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_frame(tick: usize) -> &'static str {
    // Divide tick to slow down the spinner (renders at ~60fps, we want ~10fps spin)
    SPINNER_FRAMES[(tick / 6) % SPINNER_FRAMES.len()]
}

/// Convert a UTC ISO 8601 timestamp to local time, formatted as `yyyy-mm-dd hh:mm`.
fn format_timestamp(iso: &str) -> String {
    // Get local UTC offset using libc (macOS/Linux)
    let utc_offset_secs = local_utc_offset_secs();

    let stripped = iso.trim_end_matches('Z');
    match stripped.split_once('T') {
        Some((date, time)) => {
            // Parse date and time components
            let date_parts: Vec<&str> = date.split('-').collect();
            let time_parts: Vec<&str> = time.split(':').collect();
            if date_parts.len() >= 3
                && time_parts.len() >= 2
                && let (Ok(y), Ok(mo), Ok(d), Ok(h), Ok(mi)) = (
                    date_parts[0].parse::<i64>(),
                    date_parts[1].parse::<i64>(),
                    date_parts[2].parse::<i64>(),
                    time_parts[0].parse::<i64>(),
                    time_parts[1].parse::<i64>(),
                )
            {
                // Convert to total minutes, apply offset, reconstruct
                let total_mins = h * 60 + mi + utc_offset_secs / 60;
                let mut day = d;
                let mut month = mo;
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
            // Fallback: just strip Z and show as-is
            let short_time = if time.len() >= 5 { &time[..5] } else { time };
            format!("{date} {short_time}")
        },
        None => stripped.to_string(),
    }
}

/// The number of extra rows beyond the CI run data (the "fetch more" action row).
pub(super) const CI_EXTRA_ROWS: usize = 1;

/// Build the header `Row` for the CI table from the given columns.
fn build_ci_header_row(cols: &[CiColumn]) -> Row<'static> {
    let right_aligned = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(Color::DarkGray);
    let mut header_cells = vec![
        Cell::from("#").style(right_aligned),
        Cell::from("Commit").style(right_aligned),
        Cell::from("Branch").style(right_aligned),
        Cell::from("Timestamp").style(right_aligned),
    ];
    for col in cols {
        header_cells.push(
            Cell::from(Line::from(col.label()).alignment(ratatui::layout::Alignment::Right))
                .style(right_aligned),
        );
        header_cells.push(Cell::from("")); // glyph column
    }
    header_cells.push(
        Cell::from(Line::from("Total").alignment(ratatui::layout::Alignment::Right))
            .style(right_aligned),
    );
    header_cells.push(Cell::from("")); // glyph column
    Row::new(header_cells).bottom_margin(0)
}

/// Build one data `Row` for a single `CiRun`.
fn build_ci_data_row(index: usize, ci_run: &CiRun, cols: &[CiColumn]) -> Row<'static> {
    let timestamp = format_timestamp(&ci_run.created_at);
    let branch = &ci_run.branch;

    let total_dur = ci_run
        .wall_clock_secs
        .map_or_else(|| "—".to_string(), crate::ci::format_secs);

    let row_num = format!("{}", index + 1);
    let commit = ci_run.commit_title.as_deref().unwrap_or("");
    let mut cells = vec![
        Cell::from(row_num).style(Style::default().fg(Color::DarkGray)),
        Cell::from(commit.to_string()),
        Cell::from(branch.clone()),
        Cell::from(timestamp),
    ];

    for col in cols {
        let job = ci_run.jobs.iter().find(|j| col.matches(&j.name));
        if let Some(j) = job {
            let style = conclusion_style(&j.conclusion);
            cells.push(
                Cell::from(
                    Line::from(j.duration.trim().to_string())
                        .alignment(ratatui::layout::Alignment::Right),
                )
                .style(style),
            );
            cells.push(Cell::from(j.conclusion.clone()).style(style));
        } else {
            cells.push(
                Cell::from(Line::from("—").alignment(ratatui::layout::Alignment::Right))
                    .style(Style::default().fg(Color::DarkGray)),
            );
            cells.push(Cell::from(""));
        }
    }

    // Total column
    let total_style = conclusion_style(&ci_run.conclusion);
    cells.push(
        Cell::from(
            Line::from(total_dur.trim().to_string()).alignment(ratatui::layout::Alignment::Right),
        )
        .style(total_style),
    );
    cells.push(Cell::from(ci_run.conclusion.clone()).style(total_style));

    Row::new(cells)
}

/// Build column width constraints for the CI table based on content.
fn build_ci_widths(ci_runs: &[CiRun], cols: &[CiColumn]) -> Vec<Constraint> {
    #[allow(clippy::cast_possible_truncation)]
    let max_commit_width = ci_runs
        .iter()
        .filter_map(|r| r.commit_title.as_ref())
        .map(String::len)
        .max()
        .unwrap_or(18)
        .max(18) as u16;

    #[allow(clippy::cast_possible_truncation)]
    let max_branch_width = ci_runs
        .iter()
        .map(|r| r.branch.len())
        .max()
        .unwrap_or(6)
        .max(6) as u16;

    let mut widths = vec![
        Constraint::Length(3),                // # row number
        Constraint::Length(max_commit_width), // Commit
        Constraint::Length(max_branch_width), // Branch
        Constraint::Length(16),               // Timestamp
    ];
    for _ in cols {
        widths.push(Constraint::Length(8)); // duration
        widths.push(Constraint::Length(1)); // glyph
    }
    widths.push(Constraint::Length(8)); // Total duration
    widths.push(Constraint::Length(1)); // Total glyph
    widths
}

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

    let header = build_ci_header_row(&cols);

    let mut rows: Vec<Row> = ci_runs
        .iter()
        .enumerate()
        .map(|(i, ci_run)| build_ci_data_row(i, ci_run, &cols))
        .collect();

    let widths = build_ci_widths(ci_runs, &cols);

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
pub fn detail_layout_pub(app: &App) -> (usize, Option<usize>) { detail_layout(app) }

/// Returns the maximum detail column index for the selected project.
pub fn detail_max_column(app: &App) -> usize {
    let (max_col, _) = detail_layout(app);
    max_col
}

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
            package_fields(&info).len()
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

/// Get the total number of target entries for the selected project.
fn target_list_len(app: &App) -> usize {
    let Some(project) = app.selected_project() else {
        return 0;
    };
    let info = build_detail_info(app, project);
    build_target_list(&info).len()
}

fn handle_target_action(app: &mut App, release: bool) {
    let Some(project) = app.selected_project() else {
        return;
    };
    let info = build_detail_info(app, project);
    let entries = build_target_list(&info);
    if let Some(entry) = entries.get(app.examples_scroll)
        && let Some(project) = app.selected_project()
    {
        app.pending_example_run = Some(PendingExampleRun {
            abs_path: project.abs_path.clone(),
            target_name: entry.name.clone(),
            package_name: project.name.clone(),
            kind: entry.kind,
            release,
        });
    }
}

pub(super) fn handle_detail_key(app: &mut App, key: KeyCode) {
    let (max_col, examples_col) = detail_layout(app);
    let on_examples = Some(app.detail_column) == examples_col;
    let field_count = detail_column_field_count(app, app.detail_column);

    match key {
        KeyCode::Up => {
            if on_examples {
                if app.examples_scroll > 0 {
                    app.examples_scroll -= 1;
                }
            } else if app.detail_cursor > 0 {
                app.detail_cursor -= 1;
            }
        },
        KeyCode::Down => {
            if on_examples {
                let total = target_list_len(app);
                if total > 0 && app.examples_scroll < total - 1 {
                    app.examples_scroll += 1;
                }
            } else if field_count > 0 && app.detail_cursor < field_count - 1 {
                app.detail_cursor += 1;
            }
        },
        KeyCode::Home => {
            if on_examples {
                app.examples_scroll = 0;
            } else {
                app.detail_cursor = 0;
            }
        },
        KeyCode::End => {
            if on_examples {
                let total = target_list_len(app);
                if total > 0 {
                    app.examples_scroll = total - 1;
                }
            } else if field_count > 0 {
                app.detail_cursor = field_count - 1;
            }
        },
        KeyCode::Left => {
            if app.detail_column > 0 {
                app.detail_column -= 1;
                clamp_detail_cursor(app);
            }
        },
        KeyCode::Right => {
            if on_examples {
                // No expand — do nothing
            } else if app.detail_column < max_col {
                app.detail_column += 1;
                // If entering the targets column, jump to the first item
                if Some(app.detail_column) == examples_col {
                    app.examples_scroll = 0;
                } else {
                    clamp_detail_cursor(app);
                }
            }
        },
        KeyCode::Enter => handle_detail_enter(app, on_examples),
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

/// Handle the Enter key in the detail panel.
fn handle_detail_enter(app: &mut App, on_examples: bool) {
    if on_examples {
        handle_target_action(app, false);
    } else if app.detail_column == 0 {
        let (fields, owned) = app
            .selected_project()
            .map(|p| {
                let info = build_detail_info(app, p);
                (package_fields(&info), info.owned)
            })
            .unwrap_or_default();
        if let Some(field) = fields.get(app.detail_cursor)
            && let Some(project) = app.selected_project()
        {
            match *field {
                DetailField::Name => {
                    // Open project in zed
                    let abs_path = project.abs_path.clone();
                    let _ = std::process::Command::new("zed").arg(&abs_path).spawn();
                },
                DetailField::Version if field.is_editable() && owned => {
                    let version = project.version.clone().unwrap_or_default();
                    if version != "(workspace)" {
                        app.editing = Some(EditingState {
                            field: DetailField::Version,
                            buf:   version,
                        });
                    }
                },
                DetailField::Description if field.is_editable() && owned => {
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
