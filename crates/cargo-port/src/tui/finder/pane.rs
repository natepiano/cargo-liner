//! The finder popup pane: its render state, its hit test and its drawing.
//!
//! Finder is app-owned, so `FinderPane` keeps its own viewport and focus.
//! Settings and Keymap are framework-owned and keep that state on
//! `tui_pane::SettingsPane` and `tui_pane::KeymapPane`.

use nucleo_matcher::Matcher;
use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::Atom;
use nucleo_matcher::pattern::AtomKind;
use nucleo_matcher::pattern::CaseMatching;
use nucleo_matcher::pattern::Normalization;
use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Position;
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
use tui_pane::Hittable;
use tui_pane::PaneFrameChrome;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;
use tui_pane::accent_color;
use tui_pane::contrast_ratio;
use tui_pane::finder_match_bg;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::text_default;
use tui_pane::title_color;
use unicode_width::UnicodeWidthStr;

use super::constants::FINDER_COLUMN_COUNT;
use super::constants::FINDER_DIR_COLUMN;
use super::constants::FINDER_HEADERS;
use super::constants::MIN_POPUP_WIDTH;
use super::index::FinderKind;
use crate::tui::constants::FINDER_POPUP_HEIGHT;
use crate::tui::constants::FIT_ELLIPSIS;
use crate::tui::constants::READABLE_TEXT_CONTRAST_FLOOR;
use crate::tui::hit_test::HoverTarget;
use crate::tui::overlays::PopupFrame;
use crate::tui::panes;
use crate::tui::panes::PaneId;
use crate::tui::render;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::theme_roles;

pub(in crate::tui) struct FinderPane {
    pub viewport: Viewport,
    pub focus:    RenderFocus,
}

impl FinderPane {}

impl Default for FinderPane {
    fn default() -> Self {
        Self {
            viewport: Viewport::new(),
            focus:    RenderFocus::inactive(),
        }
    }
}

impl Renderable<PaneRenderCtx<'_>> for FinderPane {
    fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        pane_render_ctx: &PaneRenderCtx<'_>,
    ) -> Option<PaneFrameChrome> {
        render_finder_pane_body(frame, area, self, pane_render_ctx);
        None
    }
}

impl Hittable<HoverTarget> for FinderPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = panes::hit_test_table_row(&self.viewport, pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Finder,
            row,
        })
    }
}

// `render_finder_pane_body` lays out `FinderPane`; `render_finder_results` paints its table.

/// Draw the finder popup: the query line, a separator and the results table.
fn render_finder_pane_body(
    frame: &mut Frame,
    _: Rect,
    pane: &mut FinderPane,
    pane_render_ctx: &PaneRenderCtx<'_>,
) {
    // Use `ctx.project_list.finder.col_widths`, cached during index construction, to keep popup
    // sizing stable across query results.
    let col_widths = pane_render_ctx.project_list.finder.col_widths;

    // Compute `natural_width` from `col_widths` plus four gaps and two borders; clamp `popup_width`
    // to `frame.area().width`.
    let natural_width: usize = col_widths.iter().sum::<usize>() + 4 + 2;
    let max_popup_width = frame.area().width;
    let popup_width = u16::try_from(natural_width)
        .unwrap_or(u16::MAX)
        .clamp(MIN_POPUP_WIDTH, max_popup_width);

    let title = if pane_render_ctx.project_list.finder.query.is_empty() {
        " Find Anything ".to_string()
    } else if pane_render_ctx.project_list.finder.total
        <= pane_render_ctx.project_list.finder.results.len()
    {
        format!(
            " Find Anything ({}) ",
            pane_render_ctx.project_list.finder.total
        )
    } else {
        format!(
            " Find Anything ({} of {}) ",
            pane_render_ctx.project_list.finder.results.len(),
            pane_render_ctx.project_list.finder.total
        )
    };

    let popup = PopupFrame {
        color_mode:   pane_render_ctx.color_mode,
        title:        Some(title),
        border_color: title_color(),
        width:        popup_width,
        height:       FINDER_POPUP_HEIGHT,
    }
    .render_with_areas(frame);
    let inner = popup.inner;

    if inner.height < 3 {
        return;
    }

    // `input_line` shows the finder query in `input_area` beside its slash prompt.
    let input_area = Rect {
        x:      inner.x,
        y:      inner.y,
        width:  inner.width,
        height: 1,
    };
    let prompt_style = Style::default()
        .fg(accent_color())
        .add_modifier(Modifier::BOLD);
    let input_line = Line::from(vec![
        Span::styled("  / ", prompt_style),
        Span::styled(
            format!("{}_", pane_render_ctx.project_list.finder.query),
            Style::default().fg(title_color()),
        ),
    ]);
    frame.render_widget(ratatui::widgets::Paragraph::new(input_line), input_area);

    // Render the separator `Line` named `sep` into `sep_area` below the query.
    if inner.height < 4 {
        return;
    }
    let sep_area = Rect {
        x:      inner.x,
        y:      inner.y + 1,
        width:  inner.width,
        height: 1,
    };
    let separator_line = Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(label_color()),
    ));
    frame.render_widget(ratatui::widgets::Paragraph::new(separator_line), sep_area);

    // `results_area` reserves the remaining `inner` rectangle for `render_finder_results`.
    let results_area = Rect {
        x:      inner.x,
        y:      inner.y + 2,
        width:  inner.width,
        height: inner.height.saturating_sub(2),
    };

    let result_count = pane_render_ctx.project_list.finder.results.len();
    pane.viewport.set_len(result_count);
    pane.viewport.set_content_area(results_area);
    pane.viewport
        .set_viewport_rows(usize::from(results_area.height.saturating_sub(1)));
    render_finder_results(
        frame,
        pane,
        pane_render_ctx,
        col_widths,
        results_area,
        popup.outer,
    );
}

/// Select the original, black, or white ink for a finder match.
enum MatchInkSelection {
    Preserve,
    Black,
    White,
}

/// Select readable ink for the background from `finder_match_bg`.
fn match_ink(ink: Color, background: Color) -> Color {
    let selection = match (
        contrast_ratio(ink, background),
        contrast_ratio(Color::Black, background),
    ) {
        (_, None) => MatchInkSelection::Preserve,
        (Some(ratio), _) if ratio >= READABLE_TEXT_CONTRAST_FLOOR => MatchInkSelection::Preserve,
        (_, Some(ratio)) if ratio >= READABLE_TEXT_CONTRAST_FLOOR => MatchInkSelection::Black,
        _ => MatchInkSelection::White,
    };
    match selection {
        MatchInkSelection::Preserve => ink,
        MatchInkSelection::Black => Color::Black,
        MatchInkSelection::White => Color::White,
    }
}

/// Draw query matches with readable ink on the background from `finder_match_bg`.
fn highlighted_spans(text: &str, query: &str, fg: Color) -> Line<'static> {
    let base_style = Style::default().fg(fg);
    let background = finder_match_bg();
    let highlight_style = base_style.fg(match_ink(fg, background)).bg(background);

    if text.is_empty() || query.is_empty() {
        return Line::from(Span::styled(text.to_owned(), base_style));
    }

    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return Line::from(Span::styled(text.to_owned(), base_style));
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
    let utf32_str = Utf32Str::new(text, &mut buf);

    // Build `char_byte_ranges` to translate `Atom::indices` character offsets into byte slices of
    // `text`.
    let char_byte_ranges: Vec<(usize, usize)> = text
        .char_indices()
        .map(|(pos, ch)| (pos, pos + ch.len_utf8()))
        .collect();

    let mut highlight_mask: Vec<bool> = vec![false; text.len()];
    let mut indices = Vec::new();
    for atom in &atoms {
        indices.clear();
        if atom
            .indices(utf32_str, &mut matcher, &mut indices)
            .is_some()
        {
            for &char_idx in &indices {
                if let Some(&(start, end)) = char_byte_ranges.get(char_idx as usize) {
                    for flag in &mut highlight_mask[start..end] {
                        *flag = true;
                    }
                }
            }
        }
    }

    // Merge adjacent `text` characters with equal `highlight_mask` state into a single styled
    // `Span`.
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
        let style = if is_match {
            highlight_style
        } else {
            base_style
        };
        spans.push(Span::styled(text[start..end].to_owned(), style));
    }

    Line::from(spans)
}

fn render_finder_results(
    frame: &mut Frame,
    pane: &mut FinderPane,
    pane_render_ctx: &PaneRenderCtx<'_>,
    col_widths: [usize; FINDER_COLUMN_COUNT],
    area: Rect,
    popup_area: Rect,
) {
    let mode = pane_render_ctx.color_mode;
    if pane_render_ctx.project_list.finder.results.is_empty() {
        let msg = if pane_render_ctx.project_list.finder.query.is_empty() {
            "Type to search projects, PRs, examples, benches..."
        } else {
            "No matches"
        };
        let hint_paragraph = ratatui::widgets::Paragraph::new(Line::from(Span::styled(
            format!("  {msg}"),
            Style::default().fg(label_color()),
        )));
        frame.render_widget(hint_paragraph, area);
        return;
    }

    let col_widths = fit_dir_column(col_widths, usize::from(area.width));
    let dir_width = col_widths[FINDER_DIR_COLUMN];
    let query = pane_render_ctx.project_list.finder.query.clone();
    let rows: Vec<Row> = pane_render_ctx
        .project_list
        .finder
        .results
        .iter()
        .enumerate()
        .map(|(row_index, &idx)| {
            let finder_item = &pane_render_ctx.project_list.finder.index[idx];
            let parent = if finder_item.kind == FinderKind::Project {
                String::new()
            } else {
                finder_item.parent_label.clone()
            };
            let selection =
                tui_pane::selection_state(&pane.viewport, row_index, pane.focus.pane_focus_state);
            let row_style = theme_roles::selection_style(selection, mode);
            let kind_color = row_style.fg.unwrap_or_else(|| finder_item.kind.color());
            let name = render::truncate_with_ellipsis(
                &finder_item.display_name,
                col_widths[0],
                FIT_ELLIPSIS,
            );
            let parent = render::truncate_with_ellipsis(&parent, col_widths[1], FIT_ELLIPSIS);
            let branch =
                render::truncate_with_ellipsis(&finder_item.branch, col_widths[2], FIT_ELLIPSIS);
            let dir = render::cut_path_from_left(&finder_item.dir, dir_width);
            Row::new(vec![
                Cell::from(highlighted_spans(&name, &query, text_default())),
                Cell::from(highlighted_spans(&parent, &query, text_default())),
                Cell::from(highlighted_spans(&branch, &query, text_default())),
                Cell::from(highlighted_spans(&dir, &query, text_default())),
                Cell::from(highlighted_spans(
                    finder_item.kind.label(),
                    &query,
                    kind_color,
                )),
            ])
            .style(selection.overlay_style(mode))
        })
        .collect();

    let constraints = col_widths.map(|w| Constraint::Length(u16::try_from(w).unwrap_or(u16::MAX)));

    let header_style = Style::default()
        .fg(label_color())
        .add_modifier(Modifier::BOLD);
    let header_row = Row::new(
        FINDER_HEADERS
            .iter()
            .map(|h| Cell::from(Span::styled(*h, header_style))),
    );

    let table = Table::new(rows, constraints)
        .header(header_row)
        .column_spacing(1)
        .row_highlight_style(Style::default());

    let mut table_state = TableState::default().with_selected(Some(pane.viewport.pos()));
    *table_state.offset_mut() = pane.viewport.scroll_offset();
    frame.render_stateful_widget(table, area, &mut table_state);
    pane.viewport.set_scroll_offset(table_state.offset());
    render_overflow_affordance(
        frame,
        popup_area,
        pane.viewport.overflow(),
        Style::default().fg(label_color()),
    );
}

/// Cut the widest column first and keep every header label.
fn fit_dir_column(
    col_widths: [usize; FINDER_COLUMN_COUNT],
    area_width: usize,
) -> [usize; FINDER_COLUMN_COUNT] {
    let gaps = FINDER_COLUMN_COUNT - 1;
    let mut fitted = col_widths;
    let budget = area_width.saturating_sub(gaps);
    let mut excess = fitted.iter().sum::<usize>().saturating_sub(budget);
    while excess > 0 {
        let widest = fitted
            .iter()
            .enumerate()
            .filter(|(index, width)| **width > FINDER_HEADERS[*index].width())
            .max_by_key(|(_, width)| **width);
        let Some((index, _)) = widest else {
            break;
        };
        fitted[index] -= 1;
        excess -= 1;
    }
    fitted
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use tui_pane::ColorMode;
    use tui_pane::PaneSelectionState;
    use tui_pane::contrast_ratio;

    use super::FINDER_HEADERS;
    use super::fit_dir_column;
    use crate::project::AbsolutePath;
    use crate::project::Cargo;
    use crate::project::CheckoutInfo;
    use crate::project::CommitDate;
    use crate::project::Comparison;
    use crate::project::ExampleGroup;
    use crate::project::GitStatus;
    use crate::project::HeadState;
    use crate::project::LocalGitState;
    use crate::project::Package;
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::project::TrackedRef;
    use crate::tui::app::App;
    use crate::tui::constants::READABLE_TEXT_CONTRAST_FLOOR;
    use crate::tui::finder;
    use crate::tui::finder::index::FinderKind;
    use crate::tui::input;
    use crate::tui::render;
    use crate::tui::test_support;
    use crate::tui::theme_roles;

    #[test]
    fn the_widest_column_gives_up_cells_first() {
        let natural = [7, 8, 6, 60, 6];

        assert_eq!(fit_dir_column(natural, 78), [7, 8, 6, 47, 6]);
        assert_eq!(fit_dir_column(natural, 200), natural);
        {
            let (widths, expected) = ([80, 5, 6, 40, 7], [28, 5, 6, 28, 7]);
            assert_eq!(fit_dir_column(widths, 78), expected);
            assert_eq!(
                render::cut_path_from_left("/tmp/project", expected[3]),
                "/tmp/project"
            );
        }
    }

    #[test]
    fn narrow_finder_cuts_dir_from_the_left_and_selected_type_uses_selection_ink()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp
            .path()
            .join("a-long-folder-name-that-the-finder-cannot-fit")
            .join("another-long-folder-name")
            .join("binws");
        std::fs::create_dir_all(&dir)?;
        for (window, name) in [(80, "binws"), (60, "cargo-berth-test-support")] {
            let projects = vec![test_support::make_package(name, &dir)];
            let mut app = test_support::make_app(&projects);
            input::open_finder(&mut app);
            for item in &mut app.project_list.finder.index {
                if item.kind == FinderKind::Project {
                    item.display_name = name.to_string();
                }
            }
            app.project_list.finder.col_widths[0] = name.len();
            let result_count = app.project_list.finder.index.len();
            app.project_list.finder.results = (0..result_count).collect();
            app.project_list.finder.total = result_count;
            let backend = TestBackend::new(window, 30);
            let mut terminal = Terminal::new(backend)?;
            terminal.draw(|frame| render::ui(frame, &mut app))?;
            let buffer = terminal.backend().buffer();
            let rows: Vec<String> = (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect()
                })
                .collect();
            let (row_y, row) = rows
                .iter()
                .enumerate()
                .find(|(_, row)| row.contains("binws") && row.contains("…/"))
                .ok_or_else(|| anyhow::anyhow!("no cut Dir row:\n{}", rows.join("\n")))?;
            assert!(row.contains("/binws "), "the cut Dir keeps its end:\n{row}");
            if window == 60 {
                let header = rows
                    .iter()
                    .find(|row| row.contains("Name") && row.contains("Project"))
                    .ok_or_else(|| anyhow::anyhow!("missing finder headers"))?;
                let name_x = header
                    .find("Name")
                    .ok_or_else(|| anyhow::anyhow!("Name header"))?;
                let project_x = header
                    .find("Project")
                    .ok_or_else(|| anyhow::anyhow!("Project header"))?;
                let name_column = header[..name_x].chars().count();
                let name_width = header[name_x..project_x].chars().count();
                let drawn: String = row.chars().skip(name_column).take(name_width).collect();
                assert!(drawn.trim().starts_with("cargo-berth"), "{row}");
                assert!(
                    drawn.trim().ends_with('…'),
                    "a cut Name ends with an ellipsis: {row}"
                );
            }

            let label = FinderKind::Project.label();
            let label_x = row
                .char_indices()
                .filter_map(|(byte, _)| row[byte..].starts_with(label).then_some(byte))
                .next_back()
                .map(|byte| row[..byte].chars().count())
                .ok_or_else(|| anyhow::anyhow!("no Type cell:\n{row}"))?;
            let selection_ink =
                theme_roles::selection_style(PaneSelectionState::Active, ColorMode::Colored).fg;
            let type_cell = &buffer[(u16::try_from(label_x)?, u16::try_from(row_y)?)];
            assert_eq!(Some(type_cell.fg), selection_ink);
        }
        Ok(())
    }

    #[test]
    fn finder_matches_meet_contrast_in_each_registered_theme() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("match-project");
        let mut app = finder_app(AbsolutePath::from(path));
        let mut failures = Vec::new();
        for variant in crate::themes::builtins() {
            let _theme = test_support::ThemeOverride::install(variant.theme);
            app.framework.color_mode = ColorMode::Colored;
            input::open_finder(&mut app);
            finder::handle_finder_text_key(&mut app, KeyCode::Char('m'));
            finder::handle_finder_text_key(&mut app, KeyCode::End);
            let buffer = test_support::draw_ui(&mut app, 160)?;
            app.project_list.finder.query.clear();
            let baseline = test_support::draw_ui(&mut app, 160)?;
            let (_, header_y) = test_support::position(&buffer, "Name")?;
            let mut example_rows = 0;
            for (row, index) in app.project_list.finder.results.iter().enumerate() {
                let item = &app.project_list.finder.index[*index];
                if item.kind != FinderKind::Example {
                    continue;
                }
                example_rows += 1;
                for (header, value) in FINDER_HEADERS.into_iter().zip([
                    item.display_name.as_str(),
                    item.parent_label.as_str(),
                    item.branch.as_str(),
                    item.dir.as_str(),
                    item.kind.label(),
                ]) {
                    let (x, _) = test_support::find_text_cell(
                        &buffer,
                        ratatui::layout::Rect::new(0, header_y, 160, 1),
                        header,
                    )
                    .context("finder column header")?;
                    let y = header_y + 1 + u16::try_from(row)?;
                    failures.extend(check_column(
                        &buffer,
                        &baseline,
                        (x, y),
                        value,
                        variant.id.as_str(),
                        header,
                    )?);
                }
            }
            assert_eq!(
                example_rows, 2,
                "both selected and unselected example rows must draw"
            );
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        Ok(())
    }

    fn check_column(
        buffer: &Buffer,
        baseline: &Buffer,
        start: (u16, u16),
        value: &str,
        theme_name: &str,
        header: &str,
    ) -> anyhow::Result<Vec<String>> {
        let mut failures = Vec::new();
        let mut matches = 0;
        for (offset, letter) in value.chars().enumerate() {
            let position = (start.0 + u16::try_from(offset)?, start.1);
            let cell = &buffer[position];
            let original = &baseline[position];
            assert_eq!(cell.symbol(), letter.to_string(), "{theme_name} {header}");
            if letter == 'm' && cell.bg == tui_pane::finder_match_bg() {
                matches += 1;
                let background = cell.bg;
                let ratio = contrast_ratio(cell.fg, background).context("explicit match colors")?;
                if ratio < READABLE_TEXT_CONTRAST_FLOOR {
                    failures.push(format!(
                        "{theme_name} {header} {:?}: {:?} on {:?}: {ratio}:1",
                        cell.symbol(),
                        cell.fg,
                        cell.bg
                    ));
                }
                if contrast_ratio(original.fg, background).context("explicit original ink")?
                    >= READABLE_TEXT_CONTRAST_FLOOR
                {
                    assert_eq!(cell.fg, original.fg, "readable match ink stays unchanged");
                }
            } else {
                assert_eq!(cell.fg, original.fg, "unmatched letters stay unchanged");
                assert_eq!(cell.bg, original.bg, "unmatched backgrounds stay unchanged");
            }
        }
        assert!(matches > 0, "{theme_name} {header}: no highlighted letter");
        Ok(failures)
    }

    fn finder_app(path: AbsolutePath) -> App {
        let mut package = Package {
            path,
            name: Some("match-project".to_string()),
            ..Package::default()
        };
        package.cargo = Cargo {
            examples: vec![ExampleGroup {
                category: String::new(),
                names:    vec!["match-one".to_string(), "match-two".to_string()],
            }],
            ..Cargo::default()
        };
        package.local_git_state = LocalGitState::Detected(Box::new(CheckoutInfo {
            status:               GitStatus::Clean,
            head:                 HeadState::Branch("match-branch".to_string()),
            last_commit:          CommitDate::NoCommits,
            ahead_behind_local:   Comparison::NotCompared,
            ahead_behind_primary: Comparison::NotCompared,
            primary_tracked_ref:  TrackedRef::Untracked,
            bisect:               None,
        }));
        test_support::make_app(&[RootItem::Rust(RustProject::Package(package))])
    }
}
