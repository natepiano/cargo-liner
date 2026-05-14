use std::cmp::Reverse;

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
use tui_pane::AppContext;
use tui_pane::FocusedPane;
use tui_pane::render_overflow_affordance;

use super::index::FINDER_COLUMN_COUNT;
use super::index::FINDER_HEADERS;
use super::index::FinderItem;
use super::index::FinderKind;
use crate::keymap::FinderAction;
use crate::tui::app::App;
use crate::tui::constants::ACCENT_COLOR;
use crate::tui::constants::ACTIVE_BORDER_COLOR;
use crate::tui::constants::FINDER_MATCH_BG;
use crate::tui::constants::FINDER_POPUP_HEIGHT;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::constants::MAX_FINDER_RESULTS;
use crate::tui::constants::TITLE_COLOR;
use crate::tui::integration::AppPaneId;
use crate::tui::overlays::PopupFrame;
use crate::tui::pane;
use crate::tui::panes;
use crate::tui::panes::PaneId;
use crate::tui::panes::RunTargetKind;

/// "bench diegetic" and "diegetic bench" produce the same results.
pub fn search_finder(index: &[FinderItem], query: &str, max_results: usize) -> (Vec<usize>, usize) {
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
    scored.sort_by_key(|entry| Reverse(entry.1));
    let indices = scored
        .into_iter()
        .take(max_results)
        .map(|(i, _)| i)
        .collect();
    (indices, total)
}

// ── Input handling ──────────────────────────────────────────────────────

pub fn dispatch_finder_action(action: FinderAction, app: &mut App) {
    match action {
        FinderAction::Activate => confirm_finder(app),
        FinderAction::Cancel => close_finder(app),
    }
}

pub fn handle_finder_text_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Up => {
            app.overlays.finder_pane.viewport.up();
        },
        KeyCode::Down => {
            app.overlays.finder_pane.viewport.down();
        },
        KeyCode::Home => {
            app.overlays.finder_pane.viewport.home();
        },
        KeyCode::End => {
            app.overlays.finder_pane.viewport.end();
        },
        KeyCode::Backspace => {
            if app.project_list.finder.query.is_empty() {
                close_finder(app);
            } else {
                app.project_list.finder.query.pop();
                refresh_finder_results(app);
            }
        },
        KeyCode::Char(c) => {
            app.project_list.finder.query.push(c);
            refresh_finder_results(app);
        },
        _ => {},
    }
}

fn close_finder(app: &mut App) {
    let return_target = app
        .overlays
        .take_finder_return()
        .unwrap_or(FocusedPane::App(AppPaneId::ProjectList));
    app.overlays.close_finder();
    app.project_list.finder.query.clear();
    app.project_list.finder.results.clear();
    app.overlays.finder_pane.viewport.home();
    app.set_focus(return_target);
}

fn refresh_finder_results(app: &mut App) {
    let (results, total) = {
        let finder = &app.project_list.finder;
        search_finder(&finder.index, &finder.query, MAX_FINDER_RESULTS)
    };
    let finder = &mut app.project_list.finder;
    finder.results = results;
    finder.total = total;
    app.overlays.finder_pane.viewport.home();
}

fn confirm_finder(app: &mut App) {
    let Some(&idx) = app
        .project_list
        .finder
        .results
        .get(app.overlays.finder_pane.viewport.pos())
    else {
        return;
    };
    let item = app.project_list.finder.index[idx].clone();

    let return_target = app
        .overlays
        .take_finder_return()
        .unwrap_or(FocusedPane::App(AppPaneId::ProjectList));
    app.overlays.close_finder();
    app.project_list.finder.query.clear();
    app.project_list.finder.results.clear();
    app.overlays.finder_pane.viewport.home();
    app.set_focus(return_target);

    // Navigate to the project
    let include_non_rust = app.config.include_non_rust().includes_non_rust();
    app.project_list
        .select_project_in_tree(item.project_path.as_path(), include_non_rust);

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
    // Focus the targets pane (now in the left panel below the project list).
    let Some(targets_data) = app.panes.targets.content().cloned() else {
        return;
    };
    if targets_data.has_targets() {
        app.set_focus(FocusedPane::App(AppPaneId::Targets));

        // Build target list and find the matching entry index
        {
            let entries = panes::build_target_list_from_data(&targets_data);
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
                    app.panes.targets.viewport.set_pos(i);
                    return;
                }
            }
        }
    }
}

// ── Rendering ───────────────────────────────────────────────────────────

pub fn render_finder_popup(frame: &mut Frame, app: &mut App) {
    // Use cached column widths (computed at index build time) for stable popup sizing
    let col_widths = app.project_list.finder.col_widths;

    // Size popup to fit all columns + spacing (4 gaps) + borders (2), capped at terminal width
    let natural_width: usize = col_widths.iter().sum::<usize>() + 4 + 2;
    let min_popup_width: u16 = 60;
    let max_popup_width = frame.area().width;
    let popup_width = u16::try_from(natural_width)
        .unwrap_or(u16::MAX)
        .clamp(min_popup_width, max_popup_width);

    let title = if app.project_list.finder.query.is_empty() {
        " Find Anything ".to_string()
    } else if app.project_list.finder.total <= app.project_list.finder.results.len() {
        format!(" Find Anything ({}) ", app.project_list.finder.total)
    } else {
        format!(
            " Find Anything ({} of {}) ",
            app.project_list.finder.results.len(),
            app.project_list.finder.total
        )
    };

    let popup = PopupFrame {
        title:        Some(title),
        border_color: ACTIVE_BORDER_COLOR,
        width:        popup_width,
        height:       FINDER_POPUP_HEIGHT,
    }
    .render_with_areas(frame);
    let inner = popup.inner;

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
            format!("{}_", app.project_list.finder.query),
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

    let result_count = app.project_list.finder.results.len();
    app.overlays.finder_pane.viewport.set_len(result_count);
    app.overlays
        .finder_pane
        .viewport
        .set_content_area(results_area);
    app.overlays
        .finder_pane
        .viewport
        .set_viewport_rows(usize::from(results_area.height.saturating_sub(1)));
    render_finder_results(frame, app, col_widths, results_area, popup.outer);
}

/// Build a `Line` where characters matching the fuzzy query get a tinted
/// background, similar to Zed's finder highlighting.
fn highlighted_spans(text: &str, query: &str, fg: Color) -> Line<'static> {
    let base = Style::default().fg(fg);
    let highlight = base.bg(FINDER_MATCH_BG);

    if text.is_empty() || query.is_empty() {
        return Line::from(Span::styled(text.to_owned(), base));
    }

    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return Line::from(Span::styled(text.to_owned(), base));
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
    let mut buf = Vec::new();
    let haystack = Utf32Str::new(text, &mut buf);

    // Pre-build char-index → byte-range lookup
    let char_byte_ranges: Vec<(usize, usize)> = text
        .char_indices()
        .map(|(pos, ch)| (pos, pos + ch.len_utf8()))
        .collect();

    let mut highlight_mask: Vec<bool> = vec![false; text.len()];
    let mut indices = Vec::new();
    for atom in &atoms {
        indices.clear();
        if atom.indices(haystack, &mut matcher, &mut indices).is_some() {
            for &char_idx in &indices {
                if let Some(&(start, end)) = char_byte_ranges.get(char_idx as usize) {
                    for flag in &mut highlight_mask[start..end] {
                        *flag = true;
                    }
                }
            }
        }
    }

    // Merge runs of same-highlight state into spans
    let mut spans = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(start, _)) = chars.peek() {
        let is_match = highlight_mask[start];
        let mut end = start;
        while let Some(&(pos, ch)) = chars.peek() {
            if highlight_mask[pos] != is_match {
                break;
            }
            end = pos + ch.len_utf8();
            chars.next();
        }
        let style = if is_match { highlight } else { base };
        spans.push(Span::styled(text[start..end].to_owned(), style));
    }

    Line::from(spans)
}

fn render_finder_results(
    frame: &mut Frame,
    app: &mut App,
    col_widths: [usize; FINDER_COLUMN_COUNT],
    area: Rect,
    popup_area: Rect,
) {
    if app.project_list.finder.results.is_empty() {
        let msg = if app.project_list.finder.query.is_empty() {
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

    let query = app.project_list.finder.query.clone();
    let rows: Vec<Row> = app
        .project_list
        .finder
        .results
        .iter()
        .enumerate()
        .map(|(row_index, &idx)| {
            let item = &app.project_list.finder.index[idx];
            let parent = if item.kind == FinderKind::Project {
                String::new()
            } else {
                item.parent_label.clone()
            };
            Row::new(vec![
                Cell::from(highlighted_spans(&item.display_name, &query, Color::White)),
                Cell::from(highlighted_spans(&parent, &query, Color::White)),
                Cell::from(highlighted_spans(&item.branch, &query, Color::White)),
                Cell::from(highlighted_spans(&item.dir, &query, Color::White)),
                Cell::from(highlighted_spans(
                    item.kind.label(),
                    &query,
                    item.kind.color(),
                )),
            ])
            .style(
                pane::selection_state(
                    &app.overlays.finder_pane.viewport,
                    row_index,
                    app.pane_focus_state(PaneId::Finder),
                )
                .overlay_style(),
            )
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

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(Style::default());

    let mut table_state =
        TableState::default().with_selected(Some(app.overlays.finder_pane.viewport.pos()));
    frame.render_stateful_widget(table, area, &mut table_state);
    app.overlays
        .finder_pane
        .viewport
        .set_scroll_offset(table_state.offset());
    render_overflow_affordance(
        frame,
        popup_area,
        app.overlays.finder_pane.viewport.overflow(),
        Style::default().fg(LABEL_COLOR),
    );

    // FinderPane participates in hit-test dispatch via its
    // viewport (content_area covers the rows-area starting at the
    // header line; `Hittable` skips that header row internally).
    let _ = app;
}
