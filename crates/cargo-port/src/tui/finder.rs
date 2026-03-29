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
use super::constants::FINDER_POPUP_HEIGHT;
use super::constants::MAX_FINDER_RESULTS;
use super::detail::RunTargetKind;
use super::render;
use super::types::FocusTarget;
use crate::project::GitInfo;
use crate::project::ProjectType;
use crate::project::RustProject;
use crate::scan::ProjectNode;

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

/// Build a flat index of all searchable items from the node tree.
/// Uses the tree structure so workspace members inherit the branch
/// from their workspace root (members don't have their own `.git`).
/// Returns `(items, col_widths)` where `col_widths` is the max display
/// width of each column across the entire index.
pub(super) fn build_finder_index(
    nodes: &[ProjectNode],
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

    // The project itself
    let kind = FinderKind::Project;
    items.push(FinderItem {
        search_text: format!("{project_name} {dir} {branch} {}", kind.label()),
        display_name: project_name.clone(),
        kind,
        project_path: project.path.clone(),
        target_name: None,
        parent_label: String::new(),
        branch: branch.clone(),
        dir: dir.clone(),
    });

    // Binary
    if project.types.contains(&ProjectType::Binary) {
        let kind = FinderKind::Binary;
        items.push(FinderItem {
            search_text: format!(
                "{project_name} {project_name} {dir} {branch} {}",
                kind.label()
            ),
            display_name: project_name.clone(),
            kind,
            project_path: project.path.clone(),
            target_name: Some(project_name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
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
            let kind = FinderKind::Example;
            items.push(FinderItem {
                search_text: format!("{display} {project_name} {dir} {branch} {}", kind.label()),
                display_name: display,
                kind,
                project_path: project.path.clone(),
                target_name: Some(name.clone()),
                parent_label: project_name.clone(),
                branch: branch.clone(),
                dir: dir.clone(),
            });
        }
    }

    // Benches
    for name in &project.benches {
        let kind = FinderKind::Bench;
        items.push(FinderItem {
            search_text: format!("{name} {project_name} {dir} {branch} {}", kind.label()),
            display_name: name.clone(),
            kind,
            project_path: project.path.clone(),
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
            app.show_finder = false;
            app.finder_query.clear();
            app.finder_results.clear();
            app.finder_cursor.jump_home();
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
            app.finder_cursor.jump_home();
        },
        KeyCode::End => {
            app.finder_cursor.jump_end(app.finder_results.len());
        },
        KeyCode::Backspace => {
            if app.finder_query.is_empty() {
                app.show_finder = false;
                app.finder_results.clear();
                app.finder_cursor.jump_home();
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
    app.finder_cursor.jump_home();
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
    app.finder_cursor.jump_home();

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
    if let Some(col) = targets_col {
        app.focus = FocusTarget::DetailFields;
        app.detail_column.set(col);
        app.detail_cursor.jump_home();

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

pub(super) fn render_finder_popup(frame: &mut Frame, app: &mut App) {
    // Use cached column widths (computed at index build time) for stable popup sizing
    let col_widths = app.finder_col_widths;

    // Size popup to fit all columns + spacing (4 gaps) + borders (2), capped at terminal width
    let natural_width: usize = col_widths.iter().sum::<usize>() + 4 + 2;
    let min_popup_width: u16 = 60;
    let max_popup_width = frame.area().width;
    #[allow(clippy::cast_possible_truncation)]
    let popup_width = (natural_width as u16).clamp(min_popup_width, max_popup_width);

    let area = render::centered_rect(popup_width, FINDER_POPUP_HEIGHT, frame.area());
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

    app.layout_cache.finder_results_area = Some(results_area);
    render_finder_results(frame, app, col_widths, results_area);
}

fn render_finder_results(
    frame: &mut Frame,
    app: &mut App,
    col_widths: [usize; FINDER_COLUMN_COUNT],
    area: Rect,
) {
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
        frame.render_widget(hint, area);
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
            .map(|h| Cell::from(Span::styled(*h, header_style))),
    );

    let highlight_style = Style::default().fg(Color::Black).bg(Color::Cyan);
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let mut table_state = TableState::default().with_selected(Some(app.finder_cursor.pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.layout_cache.finder_table_offset = table_state.offset();
}
