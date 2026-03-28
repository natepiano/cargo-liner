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
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Cell;
use ratatui::widgets::Clear;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;

use super::app::App;

const MAX_FINDER_RESULTS: usize = 50;
use super::detail::RunTargetKind;
use super::render::centered_rect;
use crate::project::GitInfo;
use crate::project::ProjectType;
use crate::project::RustProject;

/// A searchable item in the universal finder.
#[derive(Clone)]
pub struct FinderItem {
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
pub enum FinderKind {
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
            Self::Project => Color::Cyan,
            Self::Binary => Color::Green,
            Self::Example => Color::Yellow,
            Self::Bench => Color::Magenta,
        }
    }
}

/// Column width metrics cached at index build time so the popup renders at a
/// stable size regardless of the current query results.
pub const FINDER_COLUMN_COUNT: usize = 5;
pub const FINDER_HEADERS: [&str; FINDER_COLUMN_COUNT] =
    ["Name", "Project", "Branch", "Dir", "Type"];

/// Build a flat index of all searchable items from the node tree.
/// Uses the tree structure so workspace members inherit the branch
/// from their workspace root (members don't have their own `.git`).
/// Returns `(items, col_widths)` where `col_widths` is the max display
/// width of each column across the entire index.
pub fn build_finder_index(
    nodes: &[crate::scan::ProjectNode],
    git_info: &HashMap<String, GitInfo>,
) -> (Vec<FinderItem>, [usize; FINDER_COLUMN_COUNT]) {
    let mut items = Vec::new();

    for node in nodes {
        let root_branch = git_info
            .get(&node.project.path)
            .and_then(|g| g.branch.as_deref())
            .unwrap_or("")
            .to_string();

        // Add the root project and its targets
        add_project_items(&mut items, &node.project, &root_branch);

        // Add workspace members (inherit root branch)
        for group in &node.groups {
            for member in &group.members {
                add_project_items(&mut items, member, &root_branch);
            }
        }

        // Add worktree entries — each has its own branch.
        // Skip the primary-as-worktree clone (same path as root) since
        // the root was already added above.
        for wt in &node.worktrees {
            if wt.project.path == node.project.path {
                continue;
            }
            let wt_branch = git_info
                .get(&wt.project.path)
                .and_then(|g| g.branch.as_deref())
                .unwrap_or("")
                .to_string();

            add_project_items(&mut items, &wt.project, &wt_branch);

            for group in &wt.groups {
                for member in &group.members {
                    add_project_items(&mut items, member, &wt_branch);
                }
            }
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

fn add_project_items(items: &mut Vec<FinderItem>, project: &RustProject, branch: &str) {
    let project_name = project
        .name
        .as_deref()
        .unwrap_or_else(|| project.path.rsplit('/').next().unwrap_or(&project.path))
        .to_string();

    let branch = branch.to_string();
    let dir = project.path.clone();

    let branch_suffix = if branch.is_empty() {
        String::new()
    } else {
        format!(" {branch}")
    };

    // The project itself
    items.push(FinderItem {
        display_name: project_name.clone(),
        search_text:  format!("{project_name}{branch_suffix}"),
        kind:         FinderKind::Project,
        project_path: project.path.clone(),
        target_name:  None,
        parent_label: String::new(),
        branch:       branch.clone(),
        dir:          dir.clone(),
    });

    // Binary
    if project.types.contains(&ProjectType::Binary) {
        items.push(FinderItem {
            display_name: project_name.clone(),
            search_text:  format!("{project_name} bin{branch_suffix}"),
            kind:         FinderKind::Binary,
            project_path: project.path.clone(),
            target_name:  Some(project_name.clone()),
            parent_label: project_name.clone(),
            branch:       branch.clone(),
            dir:          dir.clone(),
        });
    }

    // Examples (with category prefix)
    for group in &project.examples {
        for name in &group.names {
            let display = if group.category.is_empty() {
                name.clone()
            } else {
                format!("{}/{name}", group.category)
            };
            items.push(FinderItem {
                display_name: display.clone(),
                search_text:  format!("{display} {project_name}{branch_suffix}"),
                kind:         FinderKind::Example,
                project_path: project.path.clone(),
                target_name:  Some(name.clone()),
                parent_label: project_name.clone(),
                branch:       branch.clone(),
                dir:          dir.clone(),
            });
        }
    }

    // Benches
    for name in &project.benches {
        items.push(FinderItem {
            display_name: name.clone(),
            search_text:  format!("{name} {project_name}{branch_suffix}"),
            kind:         FinderKind::Bench,
            project_path: project.path.clone(),
            target_name:  Some(name.clone()),
            parent_label: project_name.clone(),
            branch:       branch.clone(),
            dir:          dir.clone(),
        });
    }
}

/// Fuzzy-match the query against the finder index. Returns `(indices, total_matches)`.
/// Indices are sorted by score descending and capped at `max_results`.
pub fn search_finder(index: &[FinderItem], query: &str, max_results: usize) -> (Vec<usize>, usize) {
    if query.is_empty() {
        return (Vec::new(), 0);
    }

    let mut matcher = Matcher::default();
    let atom = Atom::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
        false,
    );

    let mut scored: Vec<(usize, u16)> = index
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let mut buf = Vec::new();
            let haystack = Utf32Str::new(&item.search_text, &mut buf);
            atom.score(haystack, &mut matcher).map(|score| (i, score))
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
            app.show_finder = false;
            app.finder_query.clear();
            app.finder_results.clear();
            app.finder_cursor.to_top();
        },
        KeyCode::Enter => {
            confirm_finder(app);
        },
        KeyCode::Up => {
            app.finder_cursor.up();
        },
        KeyCode::Down => {
            app.finder_cursor.down(app.finder_results.len());
        },
        KeyCode::Home => {
            app.finder_cursor.to_top();
        },
        KeyCode::End => {
            app.finder_cursor.to_bottom(app.finder_results.len());
        },
        KeyCode::Backspace => {
            if app.finder_query.is_empty() {
                app.show_finder = false;
                app.finder_results.clear();
                app.finder_cursor.to_top();
            } else {
                app.finder_query.pop();
                refresh_finder_results(app);
            }
        },
        KeyCode::Char(c) => {
            app.finder_query.push(c);
            refresh_finder_results(app);
        },
        _ => {},
    }
}

fn refresh_finder_results(app: &mut App) {
    let (results, total) = search_finder(&app.finder_index, &app.finder_query, MAX_FINDER_RESULTS);
    app.finder_results = results;
    app.finder_total = total;
    app.finder_cursor.to_top();
}

fn confirm_finder(app: &mut App) {
    let Some(&idx) = app.finder_results.get(app.finder_cursor.pos()) else {
        return;
    };
    let item = app.finder_index[idx].clone();

    // Close finder
    app.show_finder = false;
    app.finder_query.clear();
    app.finder_results.clear();
    app.finder_cursor.to_top();

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
    use super::types::FocusTarget;

    // Focus the detail panel targets column
    let (_, targets_col) = super::detail::detail_layout_pub(app);
    if let Some(col) = targets_col {
        app.focus = FocusTarget::DetailFields;
        app.detail_column.set(col);
        app.detail_cursor.to_top();

        // Build target list and find the matching entry index
        if let Some(project) = app.selected_project() {
            let info = super::detail::build_detail_info(app, project);
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
                    app.examples_scroll.set(i);
                    return;
                }
            }
        }
    }
}

// ── Rendering ───────────────────────────────────────────────────────────

pub(super) fn render_finder_popup(frame: &mut Frame, app: &App) {
    // Use cached column widths (computed at index build time) for stable popup sizing
    let col_widths = app.finder_col_widths;

    // Size popup to fit all columns + spacing (4 gaps) + borders (2), capped at terminal width
    let natural_width: usize = col_widths.iter().sum::<usize>() + 4 + 2;
    let min_popup_width: u16 = 60;
    let max_popup_width = frame.area().width;
    #[allow(clippy::cast_possible_truncation)]
    let popup_width = (natural_width as u16).clamp(min_popup_width, max_popup_width);

    let area = centered_rect(popup_width, 28, frame.area());
    frame.render_widget(Clear, area);

    let title = if app.finder_query.is_empty() {
        " Find Anything ".to_string()
    } else if app.finder_total <= app.finder_results.len() {
        format!(" Find Anything ({}) ", app.finder_total)
    } else {
        format!(
            " Find Anything ({} of {}) ",
            app.finder_results.len(),
            app.finder_total
        )
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

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
            format!("{}_", app.finder_query),
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

    if app.finder_results.is_empty() {
        let msg = if app.finder_query.is_empty() {
            "Type to search projects, examples, benches..."
        } else {
            "No matches"
        };
        let hint = ratatui::widgets::Paragraph::new(Line::from(Span::styled(
            format!("  {msg}"),
            Style::default().fg(Color::DarkGray),
        )));
        frame.render_widget(hint, results_area);
        return;
    }

    let branch_style = Style::default().fg(Color::Blue);
    let parent_style = Style::default().fg(Color::DarkGray);
    let dir_style = Style::default().fg(Color::DarkGray);
    let rows: Vec<Row> = app
        .finder_results
        .iter()
        .map(|&idx| {
            let item = &app.finder_index[idx];
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

    #[allow(clippy::cast_possible_truncation)]
    let widths = col_widths.map(|w| Constraint::Length(w as u16));

    let header_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);
    let header = Row::new(
        FINDER_HEADERS
            .iter()
            .map(|h| Cell::from(Span::styled(*h, header_style)))
            .collect::<Vec<_>>(),
    );

    let highlight_style = Style::default().fg(Color::Black).bg(Color::Cyan);
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let mut table_state = TableState::default().with_selected(Some(app.finder_cursor.pos()));
    frame.render_stateful_widget(table, results_area, &mut table_state);
}
