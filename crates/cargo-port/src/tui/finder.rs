use std::collections::HashMap;

use crossterm::event::KeyCode;
use nucleo_matcher::Matcher;
use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::Atom;
use nucleo_matcher::pattern::AtomKind;
use nucleo_matcher::pattern::CaseMatching;
use nucleo_matcher::pattern::Normalization;
use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Cell;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use super::app::App;
use super::constants::FINDER_POPUP_HEIGHT;
use super::constants::MAX_FINDER_RESULTS;
use super::detail::RunTargetKind;
use super::types::PaneId;
use crate::project::ExampleGroup;
use crate::project::GitInfo;
use crate::project::Package;
use crate::project::Project;
use crate::project::ProjectListItem;
use crate::project::ProjectType;
use crate::project::Workspace;

/// A searchable item in the universal finder.
#[derive(Clone)]
pub(super) struct FinderItem {
    /// Display name shown in the results list.
    pub display_name: String,
    /// The haystack string used for fuzzy matching (includes parent context).
    pub search_text:  String,
    /// What kind of item this is.
    pub kind:         FinderKind,
    /// Path of the project this item belongs to (for navigation).
    pub project_path: String,
    /// For targets: the cargo target name (used with --example/--bench).
    pub target_name:  Option<String>,
    /// Parent project display name (shown dimmed for non-project items).
    pub parent_label: String,
    /// Git branch, if known. Distinguishes worktrees.
    pub branch:       String,
    /// Directory name (last path component).
    pub dir:          String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FinderKind {
    Project,
    Binary,
    Example,
    Bench,
}

impl FinderKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Binary => "bin",
            Self::Example => "example",
            Self::Bench => "bench",
        }
    }

    pub const fn color(self) -> Color {
        match self {
            Self::Project => Color::Yellow,
            Self::Binary => RunTargetKind::BINARY_COLOR,
            Self::Example => RunTargetKind::EXAMPLE_COLOR,
            Self::Bench => RunTargetKind::BENCH_COLOR,
        }
    }
}

/// Column width metrics cached at index build time so the popup renders at a
/// stable size regardless of the current query results.
pub(super) const FINDER_COLUMN_COUNT: usize = 5;
pub(super) const FINDER_HEADERS: [&str; FINDER_COLUMN_COUNT] =
    ["Name", "Project", "Branch", "Dir", "Type"];

/// Build a flat index of all searchable items from the project list.
/// Uses the tree structure so workspace members inherit the branch
/// from their workspace root (members don't have their own `.git`).
/// Returns `(items, col_widths)` where `col_widths` is the max display
/// width of each column across the entire index.
pub(super) fn build_finder_index(
    list_items: &[ProjectListItem],
    git_info: &HashMap<std::path::PathBuf, GitInfo>,
) -> (Vec<FinderItem>, [usize; FINDER_COLUMN_COUNT]) {
    let mut items = Vec::new();

    for list_item in list_items {
        match list_item {
            ProjectListItem::Workspace(ws) => {
                add_workspace_items(&mut items, ws, git_info);
            },
            ProjectListItem::Package(pkg) => {
                add_package_items(&mut items, pkg, git_info);
            },
            ProjectListItem::NonRust(nr) => {
                let dp = nr.display_path();
                let branch = branch_for(nr.path(), git_info);
                add_project_items_from_typed(
                    &mut items,
                    &nr.display_name(),
                    &dp,
                    &[],
                    &[],
                    &[],
                    &branch,
                );
            },
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                add_workspace_items(&mut items, wtg.primary(), git_info);
                for linked in wtg.linked() {
                    let dp = linked.display_path();
                    if dp == wtg.primary().display_path() {
                        continue;
                    }
                    add_workspace_items(&mut items, linked, git_info);
                }
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                add_package_items(&mut items, wtg.primary(), git_info);
                for linked in wtg.linked() {
                    let dp = linked.display_path();
                    if dp == wtg.primary().display_path() {
                        continue;
                    }
                    add_package_items(&mut items, linked, git_info);
                }
            },
        }
    }

    // Pre-compute column widths from the full index
    let mut col_widths: [usize; FINDER_COLUMN_COUNT] = FINDER_HEADERS.map(str::len);
    for item in &items {
        col_widths[0] = col_widths[0].max(item.display_name.len());
        col_widths[1] = col_widths[1].max(if item.kind == FinderKind::Project {
            0
        } else {
            item.parent_label.len()
        });
        col_widths[2] = col_widths[2].max(item.branch.len());
        col_widths[3] = col_widths[3].max(item.dir.len());
        col_widths[4] = col_widths[4].max(item.kind.label().len());
    }

    (items, col_widths)
}

fn branch_for(
    abs_path: &std::path::Path,
    git_info: &HashMap<std::path::PathBuf, GitInfo>,
) -> String {
    git_info
        .get(abs_path)
        .and_then(|g| g.branch.as_deref())
        .unwrap_or("")
        .to_string()
}

fn add_workspace_items(
    items: &mut Vec<FinderItem>,
    ws: &Project<Workspace>,
    git_info: &HashMap<std::path::PathBuf, GitInfo>,
) {
    let root_path = ws.display_path();
    let root_branch = branch_for(ws.path(), git_info);
    let cargo = ws.cargo();

    add_project_items_from_typed(
        items,
        &ws.display_name(),
        &root_path,
        cargo.types(),
        cargo.examples(),
        cargo.benches(),
        &root_branch,
    );

    for group in ws.groups() {
        for member in group.members() {
            let member_cargo = member.cargo();
            add_project_items_from_typed(
                items,
                &member.display_name(),
                &member.display_path(),
                member_cargo.types(),
                member_cargo.examples(),
                member_cargo.benches(),
                &root_branch,
            );
        }
    }

    for vendored in ws.vendored() {
        add_vendored_items_typed(items, vendored, &ws.display_name());
    }
}

fn add_package_items(
    items: &mut Vec<FinderItem>,
    pkg: &Project<Package>,
    git_info: &HashMap<std::path::PathBuf, GitInfo>,
) {
    let root_path = pkg.display_path();
    let root_branch = branch_for(pkg.path(), git_info);
    let cargo = pkg.cargo();

    add_project_items_from_typed(
        items,
        &pkg.display_name(),
        &root_path,
        cargo.types(),
        cargo.examples(),
        cargo.benches(),
        &root_branch,
    );

    for vendored in pkg.vendored() {
        add_vendored_items_typed(items, vendored, &pkg.display_name());
    }
}

fn add_vendored_items_typed(
    items: &mut Vec<FinderItem>,
    project: &Project<Package>,
    parent_name: &str,
) {
    let project_name = project.display_name();
    let dir = project.display_path();
    let branch = String::new();
    let display_name = format!("{project_name} (vendored)");

    items.push(FinderItem {
        search_text: format!(
            "{display_name} {project_name} {parent_name} {dir} vendored {}",
            FinderKind::Project.label()
        ),
        display_name,
        kind: FinderKind::Project,
        project_path: dir.clone(),
        target_name: None,
        parent_label: parent_name.to_string(),
        branch: branch.clone(),
        dir: dir.clone(),
    });

    let cargo = project.cargo();

    if cargo.types().contains(&ProjectType::Binary) {
        let kind = FinderKind::Binary;
        items.push(FinderItem {
            search_text: format!(
                "{project_name} {project_name} {parent_name} {dir} vendored {}",
                kind.label()
            ),
            display_name: project_name.clone(),
            kind,
            project_path: dir.clone(),
            target_name: Some(project_name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
        });
    }

    for group in cargo.examples() {
        for name in &group.names {
            let display = if group.category.is_empty() {
                name.clone()
            } else {
                format!("{}/{name}", group.category)
            };
            let kind = FinderKind::Example;
            items.push(FinderItem {
                search_text: format!(
                    "{display} {project_name} {parent_name} {dir} vendored {}",
                    kind.label()
                ),
                display_name: display,
                kind,
                project_path: dir.clone(),
                target_name: Some(name.clone()),
                parent_label: project_name.clone(),
                branch: branch.clone(),
                dir: dir.clone(),
            });
        }
    }

    for name in cargo.benches() {
        let kind = FinderKind::Bench;
        items.push(FinderItem {
            search_text: format!(
                "{name} {project_name} {parent_name} {dir} vendored {}",
                kind.label()
            ),
            display_name: name.clone(),
            kind,
            project_path: dir.clone(),
            target_name: Some(name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
        });
    }
}

fn add_project_items_from_typed(
    items: &mut Vec<FinderItem>,
    project_name: &str,
    display_path: &str,
    types: &[ProjectType],
    examples: &[ExampleGroup],
    benches: &[String],
    branch: &str,
) {
    let project_name = project_name.to_string();
    let branch = branch.to_string();
    let dir = display_path.to_string();

    // The project itself
    let kind = FinderKind::Project;
    items.push(FinderItem {
        search_text: format!("{project_name} {dir} {branch} {}", kind.label()),
        display_name: project_name.clone(),
        kind,
        project_path: dir.clone(),
        target_name: None,
        parent_label: String::new(),
        branch: branch.clone(),
        dir: dir.clone(),
    });

    // Binary
    if types.contains(&ProjectType::Binary) {
        let kind = FinderKind::Binary;
        items.push(FinderItem {
            search_text: format!(
                "{project_name} {project_name} {dir} {branch} {}",
                kind.label()
            ),
            display_name: project_name.clone(),
            kind,
            project_path: dir.clone(),
            target_name: Some(project_name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
        });
    }

    // Examples (with category prefix)
    for group in examples {
        for name in &group.names {
            let display = if group.category.is_empty() {
                name.clone()
            } else {
                format!("{}/{name}", group.category)
            };
            let kind = FinderKind::Example;
            items.push(FinderItem {
                search_text: format!("{display} {project_name} {dir} {branch} {}", kind.label()),
                display_name: display,
                kind,
                project_path: dir.clone(),
                target_name: Some(name.clone()),
                parent_label: project_name.clone(),
                branch: branch.clone(),
                dir: dir.clone(),
            });
        }
    }

    // Benches
    for name in benches {
        let kind = FinderKind::Bench;
        items.push(FinderItem {
            search_text: format!("{name} {project_name} {dir} {branch} {}", kind.label()),
            display_name: name.clone(),
            kind,
            project_path: dir.clone(),
            target_name: Some(name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
        });
    }
}

/// Fuzzy-match the query against the finder index. Returns `(indices, total_matches)`.
/// Indices are sorted by score descending and capped at `max_results`.
///
/// Each whitespace-separated word is matched independently so that
/// "bench diegetic" and "diegetic bench" produce the same results.
pub(super) fn search_finder(
    index: &[FinderItem],
    query: &str,
    max_results: usize,
) -> (Vec<usize>, usize) {
    if query.is_empty() {
        return (Vec::new(), 0);
    }

    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return (Vec::new(), 0);
    }

    // Single word: fuzzy for forgiving typo-tolerant search.
    // Multiple words: each must be a substring so extra terms narrow, not widen.
    let kind = if words.len() == 1 {
        AtomKind::Fuzzy
    } else {
        AtomKind::Substring
    };

    let atoms: Vec<Atom> = words
        .iter()
        .map(|w| Atom::new(w, CaseMatching::Smart, Normalization::Smart, kind, false))
        .collect();

    let mut matcher = Matcher::default();
    let mut scored: Vec<(usize, u16)> = index
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let mut buf = Vec::new();
            let haystack = Utf32Str::new(&item.search_text, &mut buf);
            let mut total_score: u16 = 0;
            for atom in &atoms {
                let score = atom.score(haystack, &mut matcher)?;
                total_score = total_score.saturating_add(score);
            }
            Some((i, total_score))
        })
        .collect();

    let total = scored.len();
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    let indices = scored
        .into_iter()
        .take(max_results)
        .map(|(i, _)| i)
        .collect();
    (indices, total)
}

// ── Input handling ──────────────────────────────────────────────────────

pub(super) fn handle_finder_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => {
            app.close_finder();
            app.finder.query.clear();
            app.finder.results.clear();
            app.finder.pane.home();
            app.close_overlay();
        },
        KeyCode::Enter => {
            confirm_finder(app);
        },
        KeyCode::Up => {
            app.finder.pane.up();
        },
        KeyCode::Down => {
            app.finder.pane.down();
        },
        KeyCode::Home => {
            app.finder.pane.home();
        },
        KeyCode::End => {
            app.finder.pane.end();
        },
        KeyCode::Backspace => {
            if app.finder.query.is_empty() {
                app.close_finder();
                app.finder.results.clear();
                app.finder.pane.home();
                app.close_overlay();
            } else {
                app.finder.query.pop();
                refresh_finder_results(app);
            }
        },
        KeyCode::Char(c) => {
            app.finder.query.push(c);
            refresh_finder_results(app);
        },
        _ => {},
    }
}

fn refresh_finder_results(app: &mut App) {
    let (results, total) = search_finder(&app.finder.index, &app.finder.query, MAX_FINDER_RESULTS);
    app.finder.results = results;
    app.finder.total = total;
    app.finder.pane.home();
}

fn confirm_finder(app: &mut App) {
    let Some(&idx) = app.finder.results.get(app.finder.pane.pos()) else {
        return;
    };
    let item = app.finder.index[idx].clone();

    // Close finder
    app.close_finder();
    app.finder.query.clear();
    app.finder.results.clear();
    app.finder.pane.home();
    app.close_overlay();

    // Navigate to the project
    app.select_project_in_tree(&item.project_path);

    match item.kind {
        FinderKind::Project => {
            // Already navigated
        },
        FinderKind::Binary | FinderKind::Example | FinderKind::Bench => {
            navigate_to_target(app, &item);
        },
    }
}

/// After selecting the parent project, focus the targets column and scroll
/// to the matching target entry.
fn navigate_to_target(app: &mut App, item: &FinderItem) {
    // Focus the detail panel targets column
    let (_, targets_col) = super::detail::detail_layout_pub(app);
    if targets_col.is_some() {
        app.focus_pane(PaneId::Targets);

        // Build target list and find the matching entry index
        if let Some(info) = app.cached_detail.as_ref().map(|c| c.info.clone()) {
            let entries = super::detail::build_target_list(&info);
            let target_kind = match item.kind {
                FinderKind::Binary => RunTargetKind::Binary,
                FinderKind::Example => RunTargetKind::Example,
                FinderKind::Bench => RunTargetKind::Bench,
                FinderKind::Project => return,
            };
            let target_name = item.target_name.as_deref().unwrap_or("");
            for (i, entry) in entries.iter().enumerate() {
                if entry.name == target_name
                    && std::mem::discriminant(&entry.kind) == std::mem::discriminant(&target_kind)
                {
                    app.targets_pane.set_pos(i);
                    return;
                }
            }
        }
    }
}

// ── Rendering ───────────────────────────────────────────────────────────

pub(super) fn render_finder_popup(frame: &mut Frame, app: &mut App) {
    // Use cached column widths (computed at index build time) for stable popup sizing
    let col_widths = app.finder.col_widths;

    // Size popup to fit all columns + spacing (4 gaps) + borders (2), capped at terminal width
    let natural_width: usize = col_widths.iter().sum::<usize>() + 4 + 2;
    let min_popup_width: u16 = 60;
    let max_popup_width = frame.area().width;
    let popup_width = u16::try_from(natural_width)
        .unwrap_or(u16::MAX)
        .clamp(min_popup_width, max_popup_width);

    let title = if app.finder.query.is_empty() {
        " Find Anything ".to_string()
    } else if app.finder.total <= app.finder.results.len() {
        format!(" Find Anything ({}) ", app.finder.total)
    } else {
        format!(
            " Find Anything ({} of {}) ",
            app.finder.results.len(),
            app.finder.total
        )
    };

    let inner = super::popup::PopupFrame {
        title:        Some(title),
        border_color: Color::Cyan,
        width:        popup_width,
        height:       FINDER_POPUP_HEIGHT,
    }
    .render(frame);

    if inner.height < 3 {
        return;
    }

    // Search input line
    let input_area = Rect {
        x:      inner.x,
        y:      inner.y,
        width:  inner.width,
        height: 1,
    };
    let prompt_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let input_line = Line::from(vec![
        Span::styled("  / ", prompt_style),
        Span::styled(
            format!("{}_", app.finder.query),
            Style::default().fg(Color::Yellow),
        ),
    ]);
    frame.render_widget(ratatui::widgets::Paragraph::new(input_line), input_area);

    // Separator
    if inner.height < 4 {
        return;
    }
    let sep_area = Rect {
        x:      inner.x,
        y:      inner.y + 1,
        width:  inner.width,
        height: 1,
    };
    let sep = Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(ratatui::widgets::Paragraph::new(sep), sep_area);

    // Results table
    let results_area = Rect {
        x:      inner.x,
        y:      inner.y + 2,
        width:  inner.width,
        height: inner.height.saturating_sub(2),
    };

    app.finder.pane.set_len(app.finder.results.len());
    app.finder.pane.set_content_area(results_area);
    render_finder_results(frame, app, col_widths, results_area);
}

fn render_finder_results(
    frame: &mut Frame,
    app: &mut App,
    col_widths: [usize; FINDER_COLUMN_COUNT],
    area: Rect,
) {
    if app.finder.results.is_empty() {
        let msg = if app.finder.query.is_empty() {
            "Type to search projects, examples, benches..."
        } else {
            "No matches"
        };
        let hint = ratatui::widgets::Paragraph::new(Line::from(Span::styled(
            format!("  {msg}"),
            Style::default().fg(Color::DarkGray),
        )));
        frame.render_widget(hint, area);
        return;
    }

    let branch_style = Style::default().fg(Color::Blue);
    let parent_style = Style::default().fg(Color::DarkGray);
    let dir_style = Style::default().fg(Color::DarkGray);
    let rows: Vec<Row> = app
        .finder
        .results
        .iter()
        .map(|&idx| {
            let item = &app.finder.index[idx];
            let kind_style = Style::default()
                .fg(item.kind.color())
                .add_modifier(Modifier::BOLD);
            let parent = if item.kind == FinderKind::Project {
                String::new()
            } else {
                item.parent_label.clone()
            };
            Row::new(vec![
                Cell::from(item.display_name.clone()),
                Cell::from(Span::styled(parent, parent_style)),
                Cell::from(Span::styled(item.branch.clone(), branch_style)),
                Cell::from(Span::styled(item.dir.clone(), dir_style)),
                Cell::from(Span::styled(item.kind.label(), kind_style)),
            ])
        })
        .collect();

    let widths = col_widths.map(|w| Constraint::Length(u16::try_from(w).unwrap_or(u16::MAX)));

    let header_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);
    let header = Row::new(
        FINDER_HEADERS
            .iter()
            .map(|h| Cell::from(Span::styled(*h, header_style))),
    );

    let highlight_style = Style::default().fg(Color::Black).bg(Color::Cyan);
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let mut table_state = TableState::default().with_selected(Some(app.finder.pane.pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.finder.pane.set_scroll_offset(table_state.offset());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::LegacyProject;
    use crate::project::ProjectLanguage;
    use crate::project::WorkspaceStatus;
    use crate::scan::ProjectEntry;

    fn make_project(name: Option<&str>, path: &str) -> LegacyProject {
        crate::project::LegacyProject {
            path:                      path.to_string(),
            abs_path:                  path.to_string(),
            name:                      name.map(str::to_string),
            version:                   None,
            description:               None,
            worktree_name:             None,
            worktree_primary_abs_path: None,
            is_workspace:              WorkspaceStatus::Standalone,
            types:                     Vec::new(),
            examples:                  Vec::new(),
            benches:                   Vec::new(),
            test_count:                0,
            is_rust:                   ProjectLanguage::Rust,
        }
    }

    #[test]
    fn build_finder_index_includes_vendored_projects() {
        let mut node = ProjectEntry {
            project:   make_project(Some("hana"), "~/rust/hana"),
            groups:    Vec::new(),
            worktrees: Vec::new(),
            vendored:  vec![make_project(
                Some("clay-layout"),
                "~/rust/hana/crates/clay-layout",
            )],
        };
        node.project.is_workspace = WorkspaceStatus::Workspace;

        let list_items = crate::scan::build_project_list(&[node]);
        let (items, _widths) = build_finder_index(&list_items, &HashMap::new());
        assert!(items.iter().any(|item| {
            item.project_path == "~/rust/hana/crates/clay-layout"
                && item.display_name == "clay-layout (vendored)"
                && item.branch.is_empty()
        }));
    }
}
