use std::path::Path;

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
use super::constants::ACCENT_COLOR;
use super::constants::ACTIVE_FOCUS_COLOR;
use super::constants::FINDER_POPUP_HEIGHT;
use super::constants::LABEL_COLOR;
use super::constants::MAX_FINDER_RESULTS;
use super::constants::TITLE_COLOR;
use super::detail::RunTargetKind;
use super::interaction::UiSurface::Overlay;
use super::types::Pane;
use super::types::PaneId;
use crate::project::ExampleGroup;
use crate::project::GitInfo;
use crate::project::PackageProject;
use crate::project::ProjectFields;
use crate::project::ProjectType;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorkspaceProject;
use crate::project::WorktreeGroup;

/// A searchable item in the universal finder.
#[derive(Clone)]
pub(super) struct FinderItem {
    /// Display name shown in the results list.
    pub display_name:  String,
    /// Search tokens derived from visible fields and path segments.
    pub search_tokens: Vec<String>,
    /// What kind of item this is.
    pub kind:          FinderKind,
    /// Path of the project this item belongs to (for navigation).
    pub project_path:  String,
    /// For targets: the cargo target name (used with --example/--bench).
    pub target_name:   Option<String>,
    /// Parent project display name (shown dimmed for non-project items).
    pub parent_label:  String,
    /// Git branch, if known. Distinguishes worktrees.
    pub branch:        String,
    /// Directory name (last path component).
    pub dir:           String,
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
            Self::Project => TITLE_COLOR,
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
    list_items: &[RootItem],
) -> (Vec<FinderItem>, [usize; FINDER_COLUMN_COUNT]) {
    let mut items = Vec::new();

    for list_item in list_items {
        match list_item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                add_workspace_items(&mut items, ws);
            },
            RootItem::Rust(RustProject::Package(pkg)) => {
                add_package_items(&mut items, pkg);
            },
            RootItem::NonRust(nr) => {
                let dp = nr.display_path().into_string();
                let abs = nr.path().display().to_string();
                let branch = branch_for(nr.git_info());
                let root_name = nr.root_directory_name().into_string();
                let context = TypedProjectContext {
                    project_name: &root_name,
                    cargo_name:   None,
                    abs_path:     &abs,
                    display_path: &dp,
                    branch:       &branch,
                };
                add_project_items_from_typed(&mut items, &context, &[], &[], &[]);
            },
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => {
                add_workspace_items(&mut items, primary);
                for l in linked {
                    if l.path() == primary.path() {
                        continue;
                    }
                    add_workspace_items(&mut items, l);
                }
            },
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => {
                add_package_items(&mut items, primary);
                for l in linked {
                    if l.path() == primary.path() {
                        continue;
                    }
                    add_package_items(&mut items, l);
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

fn branch_for(git_info: Option<&GitInfo>) -> String {
    git_info
        .and_then(|g| g.branch.as_deref())
        .unwrap_or("")
        .to_string()
}

fn add_workspace_items(items: &mut Vec<FinderItem>, ws: &WorkspaceProject) {
    let root_path = ws.display_path().into_string();
    let root_abs_path = ws.path().display().to_string();
    let root_branch = branch_for(ws.git_info());
    let cargo = ws.cargo();
    let root_name = ws.root_directory_name().into_string();
    let cargo_name = ws.package_name().into_string();
    let cargo_name = (cargo_name != root_name).then_some(cargo_name);
    let root_context = TypedProjectContext {
        project_name: &root_name,
        cargo_name:   cargo_name.as_deref(),
        abs_path:     &root_abs_path,
        display_path: &root_path,
        branch:       &root_branch,
    };

    add_project_items_from_typed(
        items,
        &root_context,
        cargo.types(),
        cargo.examples(),
        cargo.benches(),
    );

    for group in ws.groups() {
        for member in group.members() {
            let member_cargo = member.cargo();
            let member_display_path = member.display_path();
            let member_abs_path = member.path().display().to_string();
            let member_name = member.package_name().into_string();
            let member_context = TypedProjectContext {
                project_name: &member_name,
                cargo_name:   None,
                abs_path:     &member_abs_path,
                display_path: member_display_path.as_str(),
                branch:       &root_branch,
            };
            add_project_items_from_typed(
                items,
                &member_context,
                member_cargo.types(),
                member_cargo.examples(),
                member_cargo.benches(),
            );
        }
    }

    let ws_package_name = ws.package_name().into_string();
    for vendored in ws.vendored() {
        add_vendored_items_typed(items, vendored, &ws_package_name);
    }
}

fn add_package_items(items: &mut Vec<FinderItem>, pkg: &PackageProject) {
    let root_path = pkg.display_path().into_string();
    let root_abs_path = pkg.path().display().to_string();
    let root_branch = branch_for(pkg.git_info());
    let cargo = pkg.cargo();
    let root_name = pkg.root_directory_name().into_string();
    let pkg_name = pkg.package_name().into_string();
    let cargo_name = (pkg_name != root_name).then_some(pkg_name);
    let root_context = TypedProjectContext {
        project_name: &root_name,
        cargo_name:   cargo_name.as_deref(),
        abs_path:     &root_abs_path,
        display_path: &root_path,
        branch:       &root_branch,
    };

    add_project_items_from_typed(
        items,
        &root_context,
        cargo.types(),
        cargo.examples(),
        cargo.benches(),
    );

    let pkg_parent_name = pkg.package_name().into_string();
    for vendored in pkg.vendored() {
        add_vendored_items_typed(items, vendored, &pkg_parent_name);
    }
}

fn add_vendored_items_typed(
    items: &mut Vec<FinderItem>,
    project: &PackageProject,
    parent_name: &str,
) {
    let project_name = project.package_name().into_string();
    let dir = project.display_path().into_string();
    let project_path = project.path().display().to_string();
    let branch = String::new();
    let display_name = format!("{project_name} (vendored)");

    items.push(FinderItem {
        search_tokens: build_search_tokens(&[
            &display_name,
            &project_name,
            parent_name,
            &dir,
            "vendored",
            FinderKind::Project.label(),
        ]),
        display_name,
        kind: FinderKind::Project,
        project_path: project_path.clone(),
        target_name: None,
        parent_label: parent_name.to_string(),
        branch: branch.clone(),
        dir: dir.clone(),
    });

    let cargo = project.cargo();

    if cargo.types().contains(&ProjectType::Binary) {
        let kind = FinderKind::Binary;
        items.push(FinderItem {
            search_tokens: build_search_tokens(&[
                &project_name,
                &project_name,
                parent_name,
                &dir,
                "vendored",
                kind.label(),
            ]),
            display_name: project_name.clone(),
            kind,
            project_path: project_path.clone(),
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
                search_tokens: build_search_tokens(&[
                    &display,
                    &project_name,
                    parent_name,
                    &dir,
                    "vendored",
                    kind.label(),
                ]),
                display_name: display,
                kind,
                project_path: project_path.clone(),
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
            search_tokens: build_search_tokens(&[
                name,
                &project_name,
                parent_name,
                &dir,
                "vendored",
                kind.label(),
            ]),
            display_name: name.clone(),
            kind,
            project_path: project_path.clone(),
            target_name: Some(name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
        });
    }
}

fn add_project_items_from_typed(
    items: &mut Vec<FinderItem>,
    context: &TypedProjectContext<'_>,
    types: &[ProjectType],
    examples: &[ExampleGroup],
    benches: &[String],
) {
    let project_name = context.project_name.to_string();
    let cargo_name = context.cargo_name.map(str::to_string);
    let branch = context.branch.to_string();
    let dir = context.display_path.to_string();

    // Build base token fields shared by all rows. Cargo name is included so
    // all targets remain findable by Cargo name when the directory differs.
    let base_fields: Vec<&str> = [&project_name as &str, &dir, &branch]
        .into_iter()
        .chain(cargo_name.as_deref())
        .collect();

    // The project itself
    let kind = FinderKind::Project;
    let mut project_tokens = base_fields.clone();
    project_tokens.push(kind.label());
    items.push(FinderItem {
        search_tokens: build_search_tokens(&project_tokens),
        display_name: project_name.clone(),
        kind,
        project_path: context.abs_path.to_string(),
        target_name: None,
        parent_label: String::new(),
        branch: branch.clone(),
        dir: dir.clone(),
    });

    // Binary
    if types.contains(&ProjectType::Binary) {
        let kind = FinderKind::Binary;
        let mut tokens = base_fields.clone();
        tokens.push(kind.label());
        items.push(FinderItem {
            search_tokens: build_search_tokens(&tokens),
            display_name: project_name.clone(),
            kind,
            project_path: context.abs_path.to_string(),
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
            let mut tokens = vec![display.as_str()];
            tokens.extend_from_slice(&base_fields);
            tokens.push(kind.label());
            items.push(FinderItem {
                search_tokens: build_search_tokens(&tokens),
                display_name: display,
                kind,
                project_path: context.abs_path.to_string(),
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
        let mut tokens = vec![name.as_str()];
        tokens.extend_from_slice(&base_fields);
        tokens.push(kind.label());
        items.push(FinderItem {
            search_tokens: build_search_tokens(&tokens),
            display_name: name.clone(),
            kind,
            project_path: context.abs_path.to_string(),
            target_name: Some(name.clone()),
            parent_label: project_name.clone(),
            branch: branch.clone(),
            dir: dir.clone(),
        });
    }
}

struct TypedProjectContext<'a> {
    project_name: &'a str,
    /// Cargo package name when it differs from `project_name`. Included in
    /// search tokens so root-level Rust items remain findable by Cargo name.
    cargo_name:   Option<&'a str>,
    abs_path:     &'a str,
    display_path: &'a str,
    branch:       &'a str,
}

fn build_search_tokens(fields: &[&str]) -> Vec<String> {
    let mut tokens = Vec::new();
    for field in fields {
        for segment in field
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '/' | '\\'))
            .filter(|segment| !segment.is_empty())
        {
            push_search_token(&mut tokens, segment);
            for fragment in segment.split(|ch: char| !ch.is_alphanumeric()) {
                push_search_token(&mut tokens, fragment);
            }
        }
    }
    tokens
}

fn push_search_token(tokens: &mut Vec<String>, token: &str) {
    if token.is_empty() || !token.chars().any(char::is_alphanumeric) {
        return;
    }
    if tokens.iter().any(|existing| existing == token) {
        return;
    }
    tokens.push(token.to_string());
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

    let atoms: Vec<Atom> = words
        .iter()
        .map(|word| {
            Atom::new(
                word,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
                false,
            )
        })
        .collect();

    let mut matcher = Matcher::default();
    let mut scored: Vec<(usize, u16)> = index
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let mut total_score: u16 = 0;
            for atom in &atoms {
                let score = item
                    .search_tokens
                    .iter()
                    .filter_map(|token| {
                        let mut buf = Vec::new();
                        let haystack = Utf32Str::new(token, &mut buf);
                        atom.score(haystack, &mut matcher)
                    })
                    .max()?;
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
            app.finder_mut().query.clear();
            app.finder_mut().results.clear();
            app.finder_mut().pane.home();
            app.close_overlay();
        },
        KeyCode::Enter => {
            confirm_finder(app);
        },
        KeyCode::Up => {
            app.finder_mut().pane.up();
        },
        KeyCode::Down => {
            app.finder_mut().pane.down();
        },
        KeyCode::Home => {
            app.finder_mut().pane.home();
        },
        KeyCode::End => {
            app.finder_mut().pane.end();
        },
        KeyCode::Backspace => {
            if app.finder().query.is_empty() {
                app.close_finder();
                app.finder_mut().results.clear();
                app.finder_mut().pane.home();
                app.close_overlay();
            } else {
                app.finder_mut().query.pop();
                refresh_finder_results(app);
            }
        },
        KeyCode::Char(c) => {
            app.finder_mut().query.push(c);
            refresh_finder_results(app);
        },
        _ => {},
    }
}

fn refresh_finder_results(app: &mut App) {
    let (results, total) = {
        let finder = app.finder();
        search_finder(&finder.index, &finder.query, MAX_FINDER_RESULTS)
    };
    let finder = app.finder_mut();
    finder.results = results;
    finder.total = total;
    finder.pane.home();
}

fn confirm_finder(app: &mut App) {
    let Some(&idx) = app.finder().results.get(app.finder().pane.pos()) else {
        return;
    };
    let item = app.finder().index[idx].clone();

    // Close finder
    app.close_finder();
    app.finder_mut().query.clear();
    app.finder_mut().results.clear();
    app.finder_mut().pane.home();
    app.close_overlay();

    // Navigate to the project
    app.select_project_in_tree(Path::new(&item.project_path));

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
        if let Some(info) = app.cached_detail().map(|c| c.info.clone()) {
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
                    app.targets_pane_mut().set_pos(i);
                    return;
                }
            }
        }
    }
}

// ── Rendering ───────────────────────────────────────────────────────────

pub(super) fn render_finder_popup(frame: &mut Frame, app: &mut App) {
    // Use cached column widths (computed at index build time) for stable popup sizing
    let col_widths = app.finder().col_widths;

    // Size popup to fit all columns + spacing (4 gaps) + borders (2), capped at terminal width
    let natural_width: usize = col_widths.iter().sum::<usize>() + 4 + 2;
    let min_popup_width: u16 = 60;
    let max_popup_width = frame.area().width;
    let popup_width = u16::try_from(natural_width)
        .unwrap_or(u16::MAX)
        .clamp(min_popup_width, max_popup_width);

    let title = if app.finder().query.is_empty() {
        " Find Anything ".to_string()
    } else if app.finder().total <= app.finder().results.len() {
        format!(" Find Anything ({}) ", app.finder().total)
    } else {
        format!(
            " Find Anything ({} of {}) ",
            app.finder().results.len(),
            app.finder().total
        )
    };

    let inner = super::popup::PopupFrame {
        title:        Some(title),
        border_color: ACTIVE_FOCUS_COLOR,
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
        .fg(ACCENT_COLOR)
        .add_modifier(Modifier::BOLD);
    let input_line = Line::from(vec![
        Span::styled("  / ", prompt_style),
        Span::styled(
            format!("{}_", app.finder().query),
            Style::default().fg(TITLE_COLOR),
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
        Style::default().fg(LABEL_COLOR),
    ));
    frame.render_widget(ratatui::widgets::Paragraph::new(sep), sep_area);

    // Results table
    let results_area = Rect {
        x:      inner.x,
        y:      inner.y + 2,
        width:  inner.width,
        height: inner.height.saturating_sub(2),
    };

    let result_count = app.finder().results.len();
    app.finder_mut().pane.set_len(result_count);
    app.finder_mut().pane.set_content_area(results_area);
    render_finder_results(frame, app, col_widths, results_area);
}

fn render_finder_results(
    frame: &mut Frame,
    app: &mut App,
    col_widths: [usize; FINDER_COLUMN_COUNT],
    area: Rect,
) {
    if app.finder().results.is_empty() {
        let msg = if app.finder().query.is_empty() {
            "Type to search projects, examples, benches..."
        } else {
            "No matches"
        };
        let hint = ratatui::widgets::Paragraph::new(Line::from(Span::styled(
            format!("  {msg}"),
            Style::default().fg(LABEL_COLOR),
        )));
        frame.render_widget(hint, area);
        return;
    }

    let branch_style = Style::default().fg(Color::Blue);
    let parent_style = Style::default().fg(LABEL_COLOR);
    let dir_style = Style::default().fg(LABEL_COLOR);
    let rows: Vec<Row> = app
        .finder()
        .results
        .iter()
        .map(|&idx| {
            let item = &app.finder().index[idx];
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
        .fg(LABEL_COLOR)
        .add_modifier(Modifier::BOLD);
    let header = Row::new(
        FINDER_HEADERS
            .iter()
            .map(|h| Cell::from(Span::styled(*h, header_style))),
    );

    let highlight_style = Pane::selection_style(app.pane_focus_state(PaneId::Finder));
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let mut table_state = TableState::default().with_selected(Some(app.finder().pane.pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.finder_mut()
        .pane
        .set_scroll_offset(table_state.offset());

    let visible_height = usize::from(area.height.saturating_sub(1));
    let visible_start = table_state.offset();
    let visible_end = app
        .finder()
        .results
        .len()
        .min(visible_start.saturating_add(visible_height));

    for (screen_row, row_index) in (visible_start..visible_end).enumerate() {
        let row_y = area
            .y
            .saturating_add(1)
            .saturating_add(u16::try_from(screen_row).unwrap_or(u16::MAX));
        super::interaction::register_pane_row_hitbox(
            app,
            Rect::new(area.x, row_y, area.width, 1),
            PaneId::Finder,
            row_index,
            Overlay,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::project::Cargo;
    use crate::project::ExampleGroup;
    use crate::project::PackageProject;
    use crate::project::ProjectType;
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::project::WorkspaceProject;

    fn test_path(path: &str) -> PathBuf {
        if path == "~" {
            return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
        }
        if let Some(rest) = path.strip_prefix("~/") {
            return dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(rest);
        }
        PathBuf::from(path)
    }

    #[test]
    fn build_finder_index_includes_vendored_projects() {
        let ws = WorkspaceProject::new(
            test_path("~/rust/hana"),
            Some("hana".to_string()),
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
            Vec::new(),
            vec![PackageProject::new(
                test_path("~/rust/hana/crates/clay-layout"),
                Some("clay-layout".to_string()),
                Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
                Vec::new(),
                None,
                None,
            )],
            None,
            None,
        );
        let list_items = vec![RootItem::Rust(RustProject::Workspace(ws))];
        let (items, _widths) = build_finder_index(&list_items);
        assert!(items.iter().any(|item| {
            item.project_path
                == test_path("~/rust/hana/crates/clay-layout")
                    .display()
                    .to_string()
                && item.display_name == "clay-layout (vendored)"
                && item.branch.is_empty()
        }));
    }

    #[test]
    fn finder_single_word_does_not_match_across_unrelated_tokens() {
        let item = FinderItem {
            display_name:  "clay-layout (vendored)".to_string(),
            search_tokens: build_search_tokens(&[
                "clay-layout (vendored)",
                "clay-layout",
                "clay-layout",
                "~/rust/bevy_diegetic/clay-layout",
                "vendored",
                FinderKind::Project.label(),
            ]),
            kind:          FinderKind::Project,
            project_path:  "~/rust/bevy_diegetic/clay-layout".to_string(),
            target_name:   None,
            parent_label:  "clay-layout".to_string(),
            branch:        String::new(),
            dir:           "~/rust/bevy_diegetic/clay-layout".to_string(),
        };

        let (results, total) = search_finder(&[item], "android", 50);
        assert!(results.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn finder_single_word_matches_directory_token() {
        let item = FinderItem {
            display_name:  "raylib_renderer".to_string(),
            search_tokens: build_search_tokens(&[
                "raylib_renderer",
                "clay-layout",
                "~/rust/bevy_diegetic/clay-layout",
                "",
                FinderKind::Example.label(),
            ]),
            kind:          FinderKind::Example,
            project_path:  "~/rust/bevy_diegetic/clay-layout".to_string(),
            target_name:   Some("raylib_renderer".to_string()),
            parent_label:  "clay-layout".to_string(),
            branch:        String::new(),
            dir:           "~/rust/bevy_diegetic/clay-layout".to_string(),
        };

        let (results, total) = search_finder(&[item], "diegetic", 50);
        assert_eq!(results, vec![0]);
        assert_eq!(total, 1);
    }

    #[test]
    fn finder_multi_word_matches_across_tokens() {
        let item = FinderItem {
            display_name:  "build-easefunction-graphs".to_string(),
            search_tokens: build_search_tokens(&[
                "build-easefunction-graphs",
                "build-easefunction-graphs",
                "~/rust/bevy/tools/build-easefunction-graphs",
                "fix/position-before-size-v0.19",
                FinderKind::Binary.label(),
            ]),
            kind:          FinderKind::Binary,
            project_path:  "~/rust/bevy/tools/build-easefunction-graphs".to_string(),
            target_name:   Some("build-easefunction-graphs".to_string()),
            parent_label:  "build-easefunction-graphs".to_string(),
            branch:        "fix/position-before-size-v0.19".to_string(),
            dir:           "~/rust/bevy/tools/build-easefunction-graphs".to_string(),
        };

        let (results, total) = search_finder(&[item], "tools graphs", 50);
        assert_eq!(results, vec![0]);
        assert_eq!(total, 1);
    }

    #[test]
    fn build_finder_index_tokenizes_display_name_and_dir_segments() {
        let pkg = PackageProject::new(
            test_path("~/rust/bevy/tools/build-easefunction-graphs"),
            Some("build-easefunction-graphs".to_string()),
            Cargo::new(
                None,
                None,
                vec![ProjectType::Binary],
                vec![ExampleGroup {
                    category: String::new(),
                    names:    vec!["raylib_renderer".to_string()],
                }],
                Vec::new(),
                0,
            ),
            Vec::new(),
            None,
            None,
        );

        let (items, _widths) = build_finder_index(&[RootItem::Rust(RustProject::Package(pkg))]);
        assert!(items.iter().any(|item| {
            item.display_name == "build-easefunction-graphs"
                && item.search_tokens.iter().any(|token| token == "tools")
                && item.search_tokens.iter().any(|token| token == "graphs")
        }));
    }
}
