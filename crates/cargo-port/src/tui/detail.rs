use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

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

use super::app::App;
use super::app::CiState;
use super::render::CiColumn;
use super::types::FocusTarget;
use crate::ci::CiRun;
use crate::project::ExampleGroup;
use crate::project::ProjectType;
use crate::project::RustProject;

#[derive(Default)]
pub struct ProjectCounts {
    pub workspaces:  usize,
    pub libs:        usize,
    pub bins:        usize,
    pub proc_macros: usize,
    pub examples:    usize,
    pub benches:     usize,
    pub tests:       usize,
}

impl ProjectCounts {
    pub fn add_project(&mut self, project: &RustProject) {
        if project.is_workspace() {
            self.workspaces += 1;
        }
        for t in &project.types {
            match t {
                ProjectType::Library => self.libs += 1,
                ProjectType::Binary => self.bins += 1,
                ProjectType::ProcMacro => self.proc_macros += 1,
                ProjectType::BuildScript => {},
            }
        }
        self.examples += project.example_count();
        self.benches += project.benches.len();
        self.tests += project.test_count;
    }

    /// Returns non-zero stats as (label, count) pairs for column display.
    pub fn to_rows(&self) -> Vec<(&'static str, usize)> {
        let mut rows = Vec::new();
        if self.workspaces > 0 {
            rows.push(("ws", self.workspaces));
        }
        if self.libs > 0 {
            rows.push(("lib", self.libs));
        }
        if self.bins > 0 {
            rows.push(("bin", self.bins));
        }
        if self.proc_macros > 0 {
            rows.push(("proc-macro", self.proc_macros));
        }
        if self.examples > 0 {
            rows.push(("example", self.examples));
        }
        if self.benches > 0 {
            rows.push(("bench", self.benches));
        }
        if self.tests > 0 {
            rows.push(("test", self.tests));
        }
        rows
    }
}

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
    // "proc-macro" is the longest possible label at 10 chars
    let total = 1 + 1 + digit_width + 1 + 10 + 1; // border + lpad + digits + space + label + rpad
    (total, digit_width)
}

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
    pub name:         String,
    pub display_name: String,
    pub kind:         RunTargetKind,
}

/// Shared style constants for detail panel rendering.
struct RenderStyles {
    highlight:       Style,
    readonly_label:  Style,
    editable_label:  Style,
    active_border:   Style,
    inactive_border: Style,
    title:           Style,
}

/// Build a flat list of all runnable targets: binaries first, then examples alphabetically,
/// then benches alphabetically.
pub fn build_target_list(info: &DetailInfo) -> Vec<TargetEntry> {
    let mut entries = Vec::new();

    if info.is_binary
        && let Some(name) = &info.binary_name
    {
        entries.push(TargetEntry {
            display_name: name.clone(),
            name:         name.clone(),
            kind:         RunTargetKind::Binary,
        });
    }

    // Collect examples with category prefix for display, sorted with
    // categorized (containing '/') before uncategorized, then alphabetically.
    let mut examples: Vec<(String, String)> = info
        .examples
        .iter()
        .flat_map(|g| {
            g.names.iter().map(|n| {
                let display = if g.category.is_empty() {
                    n.clone()
                } else {
                    format!("{}/{n}", g.category)
                };
                (n.clone(), display)
            })
        })
        .collect();
    examples.sort_by(|a, b| {
        let a_has_cat = a.1.contains('/');
        let b_has_cat = b.1.contains('/');
        match (a_has_cat, b_has_cat) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.1.cmp(&b.1),
        }
    });
    for (name, display_name) in examples {
        entries.push(TargetEntry {
            name,
            display_name,
            kind: RunTargetKind::Example,
        });
    }

    let mut bench_names = info.benches.clone();
    bench_names.sort();
    for name in bench_names {
        entries.push(TargetEntry {
            display_name: name.clone(),
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
    Branch,
    Origin,
    Owner,
    Repo,
    Stars,
    Inception,
    LastCommit,
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
            Self::Branch => "Branch",
            Self::Origin => "Origin",
            Self::Owner => "Owner",
            Self::Repo => "Repo",
            Self::Stars => "Stars",
            Self::Inception => "Incept",
            Self::LastCommit => "Latest",
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
            Self::Branch => info.git_branch.as_deref().unwrap_or("").to_string(),
            Self::Origin => info.git_origin.as_deref().unwrap_or("").to_string(),
            Self::Owner => info.git_owner.as_deref().unwrap_or("").to_string(),
            Self::Repo => info.git_url.as_deref().unwrap_or("").to_string(),
            Self::Stars => info
                .git_stars
                .map_or_else(String::new, |c| format!("⭐ {c}")),
            Self::Inception => info.git_inception.as_deref().unwrap_or("").to_string(),
            Self::LastCommit => info.git_last_commit.as_deref().unwrap_or("").to_string(),
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
    if !info.vendored_names.is_empty() {
        fields.push(DetailField::Vendored);
    }
    if info.crates_version.is_some() {
        fields.push(DetailField::CratesIo);
    }
    if info.has_package {
        fields.push(DetailField::Version);
        fields.push(DetailField::Description);
    }
    fields
}

/// Git fields (right column). Only includes fields that have data.
pub(super) fn git_fields(info: &DetailInfo) -> Vec<DetailField> {
    let mut fields = Vec::new();
    if info.git_branch.is_some() {
        fields.push(DetailField::Branch);
    }
    if info.worktree_label.is_some() {
        fields.push(DetailField::Worktree);
    }
    if info.git_origin.is_some() {
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
    if info.git_inception.is_some() {
        fields.push(DetailField::Inception);
    }
    if info.git_last_commit.is_some() {
        fields.push(DetailField::LastCommit);
    }
    fields
}

#[derive(Clone)]
pub struct DetailInfo {
    pub package_title:   String,
    pub name:            String,
    pub path:            String,
    pub version:         String,
    pub description:     Option<String>,
    pub crates_version:  Option<String>,
    pub types:           String,
    pub disk:            String,
    pub ci:              String,
    pub stats_rows:      Vec<(&'static str, usize)>,
    pub git_branch:      Option<String>,
    pub git_origin:      Option<String>,
    pub git_owner:       Option<String>,
    pub git_url:         Option<String>,
    pub git_stars:       Option<u64>,
    pub git_inception:   Option<String>,
    pub git_last_commit: Option<String>,
    pub worktree_label:  Option<String>,
    pub worktree_names:  Vec<String>,
    pub vendored_names:  String,
    pub is_binary:       bool,
    pub binary_name:     Option<String>,
    pub examples:        Vec<ExampleGroup>,
    pub benches:         Vec<String>,
    /// Whether this is a Rust project (has `Cargo.toml`).
    pub is_rust:         bool,
    /// Whether this project declares `[package]` (has version/description fields).
    pub has_package:     bool,
    /// Whether the current user owns this project (git owner matches `owned_owners`).
    pub owned:           bool,
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
    let mut counts = app.workspace_counts(project).unwrap_or_else(|| {
        let mut c = ProjectCounts::default();
        c.add_project(project);
        c
    });
    // For standalone crates, add_project doesn't count the root project's
    // examples/benches/tests — only workspace aggregation does. Fill them in.
    if !project.is_workspace() {
        counts.examples = project.example_count();
        counts.benches = project.benches.len();
        counts.tests = project.test_count;
    }
    let stats_rows = counts.to_rows();

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
        stats_rows,
        crates_version,
        git_branch,
        git_origin,
        git_owner,
        git_url,
        git_stars: app.stars.get(&project.path).copied(),
        git_inception: git
            .and_then(|g| g.first_commit.as_deref())
            .map(format_timestamp),
        git_last_commit: git
            .and_then(|g| g.last_commit.as_deref())
            .map(format_timestamp),
        worktree_label,
        worktree_names,
        vendored_names,
        is_binary,
        binary_name,
        examples: project.examples.clone(),
        benches: project.benches.clone(),
        is_rust: project.is_rust,
        has_package: project.name.is_some(),
        owned,
    }
}

fn render_column_inner(
    frame: &mut Frame,
    app: &App,
    info: &DetailInfo,
    fields: &[DetailField],
    detail_focused: bool,
    is_active_column: bool,
    cursor: usize,
    styles: &RenderStyles,
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
            styles.editable_label
        } else {
            styles.readonly_label
        };
        let ls = if is_focused {
            styles.highlight
        } else {
            base_label_style
        };
        let vs = if is_focused {
            styles.highlight
        } else if *field == DetailField::Ci {
            super::render::conclusion_style(&info.ci)
        } else {
            Style::default()
        };

        // Word-wrap long fields across multiple lines
        if matches!(*field, DetailField::Description | DetailField::Vendored) && !value.is_empty() {
            let prefix = format!("  {label:<8} ");
            let prefix_len = prefix.len();
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
                    Span::styled(format!("  {label:<8} "), ls),
                    Span::styled(value, vs),
                ]));
            }
        } else if matches!(*field, DetailField::Repo | DetailField::Branch) && !value.is_empty() {
            // Hard-wrap fields that have no spaces (URLs, branch names)
            let prefix = format!("  {label:<8} ");
            let prefix_len = prefix.len();
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

fn render_git_column_inner(
    frame: &mut Frame,
    app: &App,
    info: &DetailInfo,
    fields: &[DetailField],
    detail_focused: bool,
    is_active_column: bool,
    cursor: usize,
    styles: &RenderStyles,
    area: Rect,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for (i, field) in fields.iter().enumerate() {
        let label = field.label();
        let value = field.value(info);
        let is_focused = detail_focused && is_active_column && i == cursor;
        let ls = if is_focused {
            styles.highlight
        } else {
            styles.readonly_label
        };
        let vs = if is_focused {
            styles.highlight
        } else {
            Style::default()
        };
        if matches!(*field, DetailField::Repo | DetailField::Branch) && !value.is_empty() {
            let prefix = format!("  {label:<8} ");
            let prefix_len = prefix.len();
            let col_width = area.width as usize;
            let avail = col_width.saturating_sub(prefix_len + 1);
            if avail > 0 && value.len() > avail {
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

        let styles = RenderStyles {
            highlight:       Style::default().fg(Color::Black).bg(Color::Cyan),
            editable_label:  Style::default().fg(Color::Cyan),
            readonly_label:  Style::default().fg(Color::DarkGray),
            active_border:   Style::default().fg(Color::Cyan),
            inactive_border: Style::default(),
            title:           title_style,
        };

        render_project_panel(frame, app, info, detail_focused, &styles, columns[0]);

        let mut next_col = 1;
        if has_git {
            render_git_panel(
                frame,
                app,
                info,
                &git,
                detail_focused,
                next_col,
                &styles,
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
                &styles,
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

fn render_project_panel(
    frame: &mut Frame,
    app: &App,
    info: &DetailInfo,
    detail_focused: bool,
    styles: &RenderStyles,
    area: Rect,
) {
    let project_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", info.package_title))
        .title_style(styles.title)
        .border_style(if detail_focused && app.detail_column.pos() == 0 {
            styles.active_border
        } else {
            styles.inactive_border
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
            app.detail_column.pos() == 0,
            app.detail_cursor.pos(),
            styles,
            project_inner,
        );
    } else {
        let (stats_width, digit_width) = stats_column_width(&info.stats_rows);

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
            app.detail_column.pos() == 0,
            app.detail_cursor.pos(),
            styles,
            sub_cols[0],
        );

        // Stats column with left border
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

fn render_git_panel(
    frame: &mut Frame,
    app: &App,
    info: &DetailInfo,
    git: &[DetailField],
    detail_focused: bool,
    col: usize,
    styles: &RenderStyles,
    area: Rect,
) {
    let git_block = Block::default()
        .borders(Borders::ALL)
        .title(" Git ")
        .title_style(styles.title)
        .border_style(if detail_focused && app.detail_column.pos() == col {
            styles.active_border
        } else {
            styles.inactive_border
        });
    let git_inner = git_block.inner(area);
    frame.render_widget(git_block, area);
    render_git_column_inner(
        frame,
        app,
        info,
        git,
        detail_focused,
        app.detail_column.pos() == col,
        app.detail_cursor.pos(),
        styles,
        git_inner,
    );
}

fn render_targets_panel(
    frame: &mut Frame,
    app: &App,
    info: &DetailInfo,
    detail_focused: bool,
    col: usize,
    styles: &RenderStyles,
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

    let is_active = detail_focused && app.detail_column.pos() == col;
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

    let type_style = Style::default().fg(Color::DarkGray);
    let num_style = Style::default().fg(Color::DarkGray);
    let num_width = entries.len().to_string().len();
    let rows: Vec<Row> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let kind_label = match entry.kind {
                RunTargetKind::Binary => "bin",
                RunTargetKind::Example => "example",
                RunTargetKind::Bench => "bench",
            };
            Row::new(vec![
                Cell::from(
                    Line::from(format!("{:>num_width$}", i + 1))
                        .alignment(ratatui::layout::Alignment::Right),
                )
                .style(num_style),
                Cell::from(entry.display_name.clone()),
                Cell::from(Line::from(kind_label).alignment(ratatui::layout::Alignment::Right))
                    .style(type_style),
            ])
        })
        .collect();

    #[allow(clippy::cast_possible_truncation)]
    let num_col_width = (num_width as u16).max(2);
    let widths = [
        Constraint::Length(num_col_width),
        Constraint::Fill(1),
        Constraint::Length(7),
    ];

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
        Some(app.examples_scroll.pos())
    } else {
        None
    };
    let mut table_state = TableState::default().with_selected(selected);
    frame.render_stateful_widget(table, area, &mut table_state);
}

/// Format ISO 8601 timestamp as `yyyy-mm-dd hh:mm`.
/// Get the local UTC offset in seconds (e.g., -28800 for PST).
fn local_utc_offset_secs() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
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
            let style = super::render::conclusion_style(&j.conclusion);
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
    let total_style = super::render::conclusion_style(&ci_run.conclusion);
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
    let ci_state = app.selected_project().and_then(|p| app.ci_state_for(p));

    let title = if ci_state.is_some_and(CiState::is_fetching) {
        let spinner = spinner_frame(app.spinner_tick);
        let count = ci_state.map_or(0, CiState::fetch_count);
        format!(" CI Runs {spinner} fetching {count} more… ")
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
    let is_fetching = ci_state.is_some_and(CiState::is_fetching);
    let is_exhausted = ci_state.is_some_and(CiState::is_exhausted);
    let fetch_label = if is_fetching {
        let spinner = spinner_frame(app.spinner_tick);
        let count = ci_state.map_or(0, CiState::fetch_count);
        format!("{spinner} fetching {count} more…")
    } else if is_exhausted {
        "— no older runs".to_string()
    } else {
        "↓ fetch more runs".to_string()
    };
    let fetch_style = if is_exhausted {
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

    let mut table_state = TableState::default().with_selected(Some(app.ci_runs_cursor.pos()));
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
    let count = detail_column_field_count(app, app.detail_column.pos());
    app.detail_cursor.clamp(count);
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
    if let Some(entry) = entries.get(app.examples_scroll.pos())
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
    let on_examples = Some(app.detail_column.pos()) == examples_col;
    let field_count = detail_column_field_count(app, app.detail_column.pos());

    match key {
        KeyCode::Up => {
            if on_examples {
                app.examples_scroll.up();
            } else {
                app.detail_cursor.up();
            }
        },
        KeyCode::Down => {
            if on_examples {
                let total = target_list_len(app);
                app.examples_scroll.down(total);
            } else {
                app.detail_cursor.down(field_count);
            }
        },
        KeyCode::Home => {
            if on_examples {
                app.examples_scroll.to_top();
            } else {
                app.detail_cursor.to_top();
            }
        },
        KeyCode::End => {
            if on_examples {
                let total = target_list_len(app);
                app.examples_scroll.to_bottom(total);
            } else {
                app.detail_cursor.to_bottom(field_count);
            }
        },
        KeyCode::Left => {
            if app.detail_column.pos() > 0 {
                app.detail_column.up();
                clamp_detail_cursor(app);
            }
        },
        KeyCode::Right => {
            if on_examples {
                // No expand — do nothing
            } else if app.detail_column.pos() < max_col {
                app.detail_column.down(max_col + 1);
                // If entering the targets column, jump to the first item
                if Some(app.detail_column.pos()) == examples_col {
                    app.examples_scroll.to_top();
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
        KeyCode::Char('c') => {
            if let Some(project) = app.selected_project()
                && project.is_rust
            {
                app.confirm = Some(super::app::ConfirmAction::Clean(project.abs_path.clone()));
            }
        },
        _ => {},
    }
}

/// Handle the Enter key in the detail panel.
fn handle_detail_enter(app: &mut App, on_examples: bool) {
    if on_examples {
        handle_target_action(app, false);
    } else if app.detail_column.pos() == 0 {
        let (fields, owned) = app
            .selected_project()
            .map(|p| {
                let info = build_detail_info(app, p);
                (package_fields(&info), info.owned)
            })
            .unwrap_or_default();
        if let Some(field) = fields.get(app.detail_cursor.pos())
            && let Some(project) = app.selected_project()
        {
            match *field {
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
    } else {
        // Git column — open repo URL in browser
        if let Some(info) = app.selected_project().map(|p| build_detail_info(app, p))
            && matches!(
                git_fields(&info).get(app.detail_cursor.pos()),
                Some(DetailField::Repo)
            )
            && let Some(ref url) = info.git_url
        {
            open_url(url);
        }
    }
}

fn open_url(url: &str) {
    let _ = std::process::Command::new(if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "start"
    } else {
        "xdg-open"
    })
    .arg(url)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn();
}

pub(super) fn handle_ci_runs_key(app: &mut App, key: KeyCode) {
    let ci_state = app.selected_project().and_then(|p| app.ci_state_for(p));
    let run_count = ci_state.map_or(0, |s: &CiState| s.runs().len());
    // Total rows = run data rows + the "fetch more" action row
    let total_rows = run_count + CI_EXTRA_ROWS;
    let is_fetching = ci_state.is_some_and(CiState::is_fetching);
    let is_exhausted = ci_state.is_some_and(CiState::is_exhausted);

    match key {
        KeyCode::Up => {
            app.ci_runs_cursor.up();
        },
        KeyCode::Down => {
            app.ci_runs_cursor.down(total_rows);
        },
        KeyCode::Home => {
            app.ci_runs_cursor.to_top();
        },
        KeyCode::End => {
            app.ci_runs_cursor.to_bottom(total_rows);
        },
        KeyCode::Enter => {
            let cursor_pos = app.ci_runs_cursor.pos();
            if cursor_pos < run_count {
                // Open the CI run in the browser
                if let Some(runs) = ci_state.map(CiState::runs)
                    && let Some(run) = runs.get(cursor_pos)
                {
                    let _ = std::process::Command::new("open").arg(&run.url).spawn();
                }
            } else if cursor_pos == run_count
                && !is_fetching
                && !is_exhausted
                && let Some(project) = app.selected_project()
            {
                // Fetch more runs
                #[allow(clippy::cast_possible_truncation)]
                let current_count = run_count as u32;
                app.pending_ci_fetch = Some(PendingCiFetch {
                    abs_path: project.abs_path.clone(),
                    project_path: project.path.clone(),
                    current_count,
                });
            }
        },
        KeyCode::Char('c') => {
            if let Some(project) = app.selected_project() {
                let path = project.path.clone();
                clear_ci_cache(app, &path);
            }
        },
        _ => {},
    }
}

/// Clear CI cache for a project and remove its runs from the app.
fn clear_ci_cache(app: &mut App, project_path: &str) {
    // Derive (owner, repo) from local git info — no network needed
    if let Some(git) = app.git_info.get(project_path)
        && let Some(url) = &git.url
        && let Some((owner, repo)) = crate::ci::parse_owner_repo(url)
        && let Some(dir) = crate::scan::repo_cache_dir_pub(&owner, &repo)
    {
        let _ = std::fs::remove_dir_all(dir);
    }

    // Insert empty Loaded so the CI panel stays visible with the "fetch more" row
    app.ci_state.insert(
        project_path.to_string(),
        CiState::Loaded {
            runs:      Vec::new(),
            exhausted: false,
        },
    );
    app.ci_runs_cursor.to_top();
    app.data_generation += 1;
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

    app.data_generation += 1;
}

#[cfg(test)]
mod tests {
    use super::stats_column_width;

    #[test]
    fn stats_width_fixed_for_three_digit_counts() {
        // 3-digit counts: border + lpad + 3 digits + space + 10 label + rpad = 17
        let rows = vec![("example", 999), ("lib", 1)];
        let (total, digits) = stats_column_width(&rows);
        assert_eq!(digits, 3);
        assert_eq!(total, 17);
    }

    #[test]
    fn stats_width_expands_at_four_digits() {
        // 4-digit counts: border + lpad + 4 digits + space + 10 label + rpad = 18
        let rows = vec![("example", 1000), ("lib", 1)];
        let (total, digits) = stats_column_width(&rows);
        assert_eq!(digits, 4);
        assert_eq!(total, 18);
    }

    #[test]
    fn stats_width_stable_for_short_labels() {
        // Even with only short labels present, width stays fixed to fit "proc-macro"
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
}
