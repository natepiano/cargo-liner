//! Word presses use the painted grid and preserve row actions.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use tui_pane::TerminalApp;
use tui_pane::TileView;

use super::*;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::interaction::TileSelection;
use crate::theme::Role;
use crate::word_highlight::WordHighlight;
use crate::word_highlight::WordHit;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const WIDTH: u16 = 200;
const HEIGHT: u16 = 70;
const ACCOUNT: &str = "claude1";

pub(super) fn session(pid: u32, name: &str, work: &str) -> TestResult<AgentRow> {
    Ok(serde_json::from_value(serde_json::json!({
        "agent":"claude", "pid":pid, "name":name, "started":100,
        "session_id":format!("word-session-{pid}"), "directory":"/fixture",
        "account":{"home":"~/.claude1","label":ACCOUNT},
        "children":if work.is_empty() {vec![]} else {vec![serde_json::json!({
            "kind":"shell","pid":pid+1,"depth":0,"name":work,"started":101
        })]}
    }))?)
}

pub(super) fn apply_rows(app: &mut App, rows: Vec<AgentRow>) {
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
}

pub(super) fn application() -> TestResult<App> {
    let mut app = App::new_for_test()?;
    app.loaded_config.config.tiles.initial_rows = serde_json::from_value(serde_json::json!(8))?;
    app.loaded_config.config.tiles.widen_summary = false;
    let mut other_account = session(50, "other-account", "")?;
    other_account.account =
        serde_json::from_value(serde_json::json!({"home":"~/.claude2","label":"claude2"}))?;
    apply_rows(
        &mut app,
        vec![
            session(10, "first-owner", "first-work")?,
            session(20, "second-owner", "second-work")?,
            session(30, "pool-session", "")?,
            other_account,
        ],
    );
    app.accounts.table = AccountsTable::Ready(
        AccountsReading::parse(
            br#"{
        "accounts":[{"label":"claude1","tool":"claude","roles":["showrunners","units"],
        "weekly_left_percent":50,"use_credits":false,"read_ok":true,
        "role_counts":[{"busy":1,"idle":3,"paused":2,"exited":0,"total":6},
        {"busy":4,"idle":6,"paused":5,"exited":0,"total":15}]}],"problems":[]
    }"#,
        )
        .map_err(|error| error.to_string())?,
    );
    Ok(app)
}

pub(super) struct Renderer {
    terminal: Terminal<TestBackend>,
}

impl Renderer {
    pub(super) fn new(app: &mut App, width: u16, height: u16) -> TestResult<Self> {
        let mut renderer = Self {
            terminal: Terminal::new(TestBackend::new(width, height))?,
        };
        renderer.paint(app)?;
        if app.tiles.has_pending_motion_for_test() {
            app.tiles.settle_for_test();
            renderer.paint(app)?;
        }
        Ok(renderer)
    }

    pub(super) fn paint(&mut self, app: &mut App) -> TestResult<Buffer> {
        let keymap = app.keymap();
        self.terminal.draw(|frame| app.draw(frame, &keymap))?;
        Ok(self.terminal.backend().buffer().clone())
    }
}

pub(super) fn positions(buffer: &Buffer, text: &str) -> Vec<Position> {
    let mut result = Vec::new();
    for y in buffer.area.y..buffer.area.bottom() {
        let mut line = String::new();
        let mut offsets = Vec::new();
        let mut x = buffer.area.x;
        while x < buffer.area.right() {
            let symbol = buffer[(x, y)].symbol();
            offsets.push((line.len(), x));
            line.push_str(symbol);
            let width = u16::try_from(unicode_width::UnicodeWidthStr::width(symbol))
                .unwrap_or(u16::MAX)
                .max(1);
            x = x.saturating_add(width);
        }
        for (index, _) in line.match_indices(text) {
            if let Some((_, column)) = offsets.iter().find(|(byte, _)| *byte == index) {
                result.push(Position::new(*column, y));
            }
        }
    }
    result
}

pub(super) fn locate(buffer: &Buffer, text: &str) -> TestResult<Position> {
    positions(buffer, text)
        .into_iter()
        .next()
        .ok_or_else(|| format!("missing painted {text}").into())
}

fn header_word(buffer: &Buffer, text: &str) -> TestResult<Position> {
    let heading = locate(buffer, "accounts")?;
    positions(buffer, text)
        .into_iter()
        .find(|point| {
            let end = point.x + u16::try_from(text.len()).unwrap_or(u16::MAX);
            point.y > heading.y
                && (point.x == buffer.area.x || buffer[(point.x - 1, point.y)].symbol() == " ")
                && end < buffer.area.right()
                && buffer[(end, point.y)].symbol() == " "
        })
        .ok_or_else(|| format!("missing header {text}").into())
}

fn tree_word(buffer: &Buffer, text: &str) -> TestResult<Position> {
    positions(buffer, text)
        .into_iter()
        .find(|point| {
            (buffer.area.x..point.x).any(|x| matches!(buffer[(x, point.y)].symbol(), "├" | "└"))
        })
        .ok_or_else(|| format!("missing tree row {text}").into())
}

fn selections(app: &App) -> Vec<TileSelection> {
    app.tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .iter()
        .filter_map(|placement| match selection_surface(&placement.content) {
            SelectableSurface::Surface(surface) => app
                .selections
                .surface(&surface)
                .map_or(None, |state| Some(state.selection())),
            SelectableSurface::EmptyCell => None,
        })
        .collect()
}

fn key(app: &mut App, code: KeyCode) {
    tui_pane::dispatch_key(app, KeyEvent::new(code, KeyModifiers::NONE));
}

pub(super) fn match_background() -> TestResult<Color> {
    Role::WordMatch
        .style()
        .bg
        .ok_or_else(|| "word_match background".into())
}

pub(super) fn assert_matches(buffer: &Buffer, word: &str, background: Color) -> TestResult {
    let found: Vec<_> = positions(buffer, word)
        .into_iter()
        .filter(|point| {
            let end = point.x + u16::try_from(word.len()).unwrap_or(u16::MAX);
            let continues = |symbol: &str| {
                symbol.chars().any(|character| {
                    character.is_alphanumeric() || matches!(character, '-' | '_' | '\'')
                })
            };
            (point.x == buffer.area.x || !continues(buffer[(point.x - 1, point.y)].symbol()))
                && (end >= buffer.area.right() || !continues(buffer[(end, point.y)].symbol()))
        })
        .collect();
    assert!(!found.is_empty(), "painted {word}");
    let grounds = crate::theme::WordMatchGrounds::new(background);
    for point in found {
        let end = point.x + u16::try_from(word.len())?;
        let neighbor = [end, point.x.saturating_sub(1)]
            .into_iter()
            .filter(|&x| buffer.area.contains(Position::new(x, point.y)))
            .map(|x| &buffer[(x, point.y)])
            .find(|cell| cell.symbol() == " ")
            .ok_or("the matched word must have an unmatched neighboring space")?;
        let expected = grounds.background(grounds.surface(neighbor.bg));
        for x in point.x..end {
            assert_eq!(
                buffer[(x, point.y)].bg,
                expected,
                "{word} at {point:?} column {x}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_word_press_marks_two_agent_tiles_the_pool_the_summary_and_accounts() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    let points = positions(&before, ACCOUNT);
    assert!(
        points.len() >= 7,
        "two trees, pool, summary rows, and account table"
    );
    app.click(points[0]);
    let marked = renderer.paint(&mut app)?;
    assert_ne!(app.word_highlight, WordHighlight::Off);
    assert_matches(&marked, ACCOUNT, match_background()?)
}

#[test]
fn a_new_matching_row_receives_the_mark_on_the_next_frame() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    app.click(locate(&before, ACCOUNT)?);
    renderer.paint(&mut app)?;
    let mut other_account = session(50, "other-account", "")?;
    other_account.account =
        serde_json::from_value(serde_json::json!({"home":"~/.claude2","label":"claude2"}))?;
    apply_rows(
        &mut app,
        vec![
            session(10, "first-owner", "first-work")?,
            session(40, ACCOUNT, "")?,
            other_account,
        ],
    );
    renderer.paint(&mut app)?;
    app.tiles.settle_for_test();
    let after = renderer.paint(&mut app)?;
    let row = positions(&after, ACCOUNT);
    assert!(
        row.len() >= 4,
        "the new session and its account are painted"
    );
    assert_matches(&after, ACCOUNT, match_background()?)
}

#[test]
fn a_second_press_on_the_word_at_either_place_clears_the_mark() -> TestResult {
    for elsewhere in [false, true] {
        let mut app = application()?;
        let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
        let before = renderer.paint(&mut app)?;
        let points = positions(&before, ACCOUNT);
        assert!(points.len() > 1);
        app.click(points[0]);
        renderer.paint(&mut app)?;
        assert_ne!(app.word_highlight, WordHighlight::Off);
        app.click(if elsewhere { points[1] } else { points[0] });
        assert_eq!(app.word_highlight, WordHighlight::Off);
        let after = renderer.paint(&mut app)?;
        let background = match_background()?;
        assert!(
            positions(&after, ACCOUNT)
                .iter()
                .all(|point| after[(point.x, point.y)].bg != background)
        );
    }
    Ok(())
}

#[test]
fn another_word_moves_the_mark_and_escape_clears_it() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    app.click(locate(&before, ACCOUNT)?);
    renderer.paint(&mut app)?;
    app.click(tree_word(&before, "second-work")?);
    let after = renderer.paint(&mut app)?;
    assert_matches(&after, "second-work", match_background()?)?;
    let background = match_background()?;
    assert!(
        positions(&after, ACCOUNT)
            .iter()
            .all(|point| after[(point.x, point.y)].bg != background)
    );
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.word_highlight, WordHighlight::Off);
    Ok(())
}

#[test]
fn a_missed_content_press_clears_the_word_and_keeps_the_selected_row() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    app.click(tree_word(&before, "first-work")?);
    let marked = renderer.paint(&mut app)?;
    assert_ne!(app.word_highlight, WordHighlight::Off);
    let selected = selections(&app);
    let content = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .find(|placement| matches!(placement.content, TileContent::Group(_)))
        .ok_or("agent tile")?;
    let SelectableSurface::Surface(surface) = selection_surface(&content.content) else {
        return Err("agent surface".into());
    };
    let area = app
        .selections
        .surface(&surface)
        .map_or(None, |state| Some(state.viewport().content_area()))
        .ok_or("painted content area")?;
    let point = area
        .positions()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .find(|point| {
            marked[(point.x, point.y)].symbol() == " "
                && point.y > tree_word(&marked, "first-work").map_or(0, |p| p.y)
                && point.x > area.x + 1
        })
        .ok_or("empty tile content")?;
    app.click(point);
    assert_eq!(app.word_highlight, WordHighlight::Off);
    assert_eq!(selections(&app), selected);
    Ok(())
}

#[test]
fn title_words_are_targets_but_status_and_readout_words_are_chrome() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    let placements = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth());
    let title = positions(&before, "first-owner")
        .into_iter()
        .find(|point| {
            placements
                .iter()
                .any(|placement| placement.frame.rect().y == point.y)
        })
        .ok_or("painted tile title")?;
    app.click(title);
    let marked = renderer.paint(&mut app)?;
    assert_matches(&marked, "first-owner", match_background()?)?;
    assert_ne!(app.word_highlight, WordHighlight::Off);
    let highlight = app.word_highlight.clone();
    app.click(locate(&marked, "content rows:")?);
    assert_eq!(app.word_highlight, highlight);
    app.click(Position::new(5, HEIGHT - 1));
    assert_eq!(app.word_highlight, highlight);
    Ok(())
}

#[test]
fn an_overlay_receives_the_press_and_escape_before_the_word_state() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    let point = locate(&before, ACCOUNT)?;
    app.click(point);
    renderer.paint(&mut app)?;
    assert_ne!(app.word_highlight, WordHighlight::Off);
    let highlight = app.word_highlight.clone();
    key(&mut app, KeyCode::Char('s'));
    assert!(app.framework.overlay().is_some());
    renderer.paint(&mut app)?;
    app.click(point);
    assert_eq!(app.word_highlight, highlight);
    key(&mut app, KeyCode::Esc);
    assert!(app.framework.overlay().is_none());
    assert_eq!(app.word_highlight, highlight);
    Ok(())
}

#[test]
fn account_continuation_words_and_pinned_headers_use_the_painted_line() -> TestResult {
    let mut app = application()?;
    app.loaded_config.config.tiles.view = TileView::Summary;
    let mut renderer = Renderer::new(&mut app, 80, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    let point = locate(&before, "units")?;
    let account = locate(&before, ACCOUNT)?;
    assert!(
        point.y > account.y,
        "the second account role is a continuation"
    );
    app.click(point);
    let marked = renderer.paint(&mut app)?;
    assert_matches(&marked, "units", match_background()?)?;
    let old_sort = app.accounts_sort;
    app.click(header_word(&marked, "account")?);
    let header = renderer.paint(&mut app)?;
    assert_ne!(
        app.accounts_sort, old_sort,
        "the word press preserves header sorting"
    );
    assert_matches(&header, "account", match_background()?)?;
    app.click(locate(&header, "accounts")?);
    assert_matches(&renderer.paint(&mut app)?, "accounts", match_background()?)
}

fn branch_application() -> TestResult<App> {
    let mut app = application()?;
    let root = session(10, "branch-owner", "")?;
    let mut child = session(20, "branch-child", "")?;
    child.launched_by = Some(10);
    let mut leaf = session(30, "branch-leaf", "")?;
    leaf.launched_by = Some(20);
    apply_rows(&mut app, vec![root, child, leaf]);
    Ok(app)
}

#[test]
fn a_marker_press_toggles_the_branch_and_sets_no_word() -> TestResult {
    let mut app = branch_application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    let child = tree_word(&before, "branch-child")?;
    let column = (0..child.x)
        .rev()
        .find(|&x| before[(x, child.y)].symbol() == "▶")
        .ok_or("closed branch marker")?;
    assert_eq!(
        positions(&before, "branch-leaf"),
        [] as [ratatui::prelude::Position; 0]
    );
    app.click(locate(&before, ACCOUNT)?);
    renderer.paint(&mut app)?;
    assert_ne!(app.word_highlight, WordHighlight::Off);
    app.click(Position::new(column, child.y));
    let after = renderer.paint(&mut app)?;
    assert_ne!(
        positions(&after, "branch-leaf"),
        [] as [ratatui::prelude::Position; 0]
    );
    assert_eq!(app.word_highlight, WordHighlight::Off);
    assert!(selections(&app).iter().any(|selection| matches!(
        selection,
        TileSelection::Row(crate::interaction::RowIdentity::Session { pid: 20, .. })
    )));
    Ok(())
}

#[test]
fn guides_and_trailing_spaces_select_the_row_without_selecting_a_word() -> TestResult {
    for trailing in [false, true] {
        let mut app = application()?;
        let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
        let before = renderer.paint(&mut app)?;
        app.click(locate(&before, ACCOUNT)?);
        let marked = renderer.paint(&mut app)?;
        assert_ne!(app.word_highlight, WordHighlight::Off);
        let name = tree_word(&marked, "second-work")?;
        let x = if trailing {
            name.x + u16::try_from("second-work".len())?
        } else {
            (0..name.x)
                .rev()
                .find(|&x| matches!(marked[(x, name.y)].symbol(), "├" | "└"))
                .ok_or("row guide")?
        };
        if trailing {
            assert_eq!(marked[(x, name.y)].symbol(), " ");
        }
        app.click(Position::new(x, name.y));
        assert_eq!(app.word_highlight, WordHighlight::Off);
        assert!(selections(&app).iter().any(|selection| matches!(
            selection,
            TileSelection::Row(crate::interaction::RowIdentity::Shell { pid: 21, .. })
        )));
    }
    Ok(())
}

#[test]
fn a_name_press_selects_its_row_and_its_painted_word() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    app.click(tree_word(&before, "second-work")?);
    assert!(selections(&app).iter().any(|selection| matches!(
        selection,
        TileSelection::Row(crate::interaction::RowIdentity::Shell { pid: 21, .. })
    )));
    assert_matches(
        &renderer.paint(&mut app)?,
        "second-work",
        match_background()?,
    )
}

#[test]
fn word_lookup_uses_the_last_painted_name_before_a_census_change() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    let point = tree_word(&before, "first-work")?;
    apply_rows(&mut app, vec![session(10, "first-owner", "new-unpainted")?]);
    app.click(point);
    let WordHighlight::Word(word) = &app.word_highlight else {
        return Err("painted word was lost".into());
    };
    assert_eq!(word.text(), "first-work");
    Ok(())
}

#[test]
fn visible_unicode_tokens_resolve_from_every_terminal_cell() -> TestResult {
    for name in [
        "界面",
        "e\u{301}clair",
        "👩\u{200d}💻work",
        "✈\u{fe0f}flight",
        "resources…",
    ] {
        let mut app = application()?;
        apply_rows(&mut app, vec![session(10, "unicode-owner", name)?]);
        // Keep every token and cell press in a smaller frame.
        let mut renderer = Renderer::new(&mut app, 100, 30)?;
        let before = renderer.paint(&mut app)?;
        let point = tree_word(&before, name)?;
        let mut selected = Vec::new();
        for column in 0..u16::try_from(unicode_width::UnicodeWidthStr::width(name))? {
            app.click(Position::new(point.x + column, point.y));
            let WordHighlight::Word(word) = &app.word_highlight else {
                return Err(format!("{name} cell {column} gave no word").into());
            };
            selected.push(word.text().to_owned());
            key(&mut app, KeyCode::Esc);
            renderer.paint(&mut app)?;
        }
        assert!(
            selected.iter().all(|word| word == &selected[0]),
            "{name}: {selected:?}"
        );
    }
    Ok(())
}

#[test]
fn omitted_summary_sessions_supply_no_hidden_word_target() -> TestResult {
    let mut app = application()?;
    app.loaded_config.config.tiles.view = TileView::Summary;
    let mut hidden = session(10, "one", "first-work")?;
    hidden.account = serde_json::from_value(
        serde_json::json!({"home":"~/.claude-long","label":"long-account-label"}),
    )?;
    hidden.activity = serde_json::from_value(serde_json::json!({"active":"thinking"}))?;
    apply_rows(
        &mut app,
        vec![
            hidden,
            session(20, "two", "second-work")?,
            session(30, "three", "")?,
        ],
    );
    let mut renderer = Renderer::new(&mut app, 12, 50)?;
    app.word_highlight
        .press(crate::word_highlight::HighlightWord::from_token(
            "absent-word",
        ));
    let buffer = renderer.paint(&mut app)?;
    for name in ["one", "two", "three"] {
        assert!(
            positions(&buffer, name).is_empty(),
            "narrow summary omits {name}"
        );
        for point in buffer.area.positions() {
            if let WordHit::Word(word) = app.painted_words.hit(point) {
                assert_ne!(word.text(), name, "omitted session at {point:?}");
            }
        }
    }
    Ok(())
}

#[test]
fn a_retired_painted_tile_keeps_focus_selection_and_highlight() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    app.click(locate(&before, ACCOUNT)?);
    renderer.paint(&mut app)?;
    let departing = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .find(|placement| matches!(placement.content, TileContent::Group(_)))
        .ok_or("departing tile")?
        .content;
    apply_rows(
        &mut app,
        vec![
            session(20, "second-owner", "second-work")?,
            session(30, "pool-session", "")?,
        ],
    );
    renderer.paint(&mut app)?;
    let placements = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth());
    let departure = placements
        .iter()
        .find(|placement| placement.content == departing)
        .ok_or("painted departing tile")?;
    let point = departure
        .frame
        .inner()
        .intersection(departure.frame.clip())
        .positions()
        .find(|point| app.tiles.cell_at(*point).is_none())
        .ok_or("departure overlaps a replacement")?;
    let focus = app.tiles.focused_cell();
    let selected = selections(&app);
    assert_ne!(app.word_highlight, WordHighlight::Off);
    let highlight = app.word_highlight.clone();
    app.click(point);
    assert_eq!(app.tiles.focused_cell(), focus);
    assert_eq!(selections(&app), selected);
    assert_eq!(app.word_highlight, highlight);
    Ok(())
}

#[test]
fn empty_space_in_a_title_clears_the_word() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    app.click(locate(&before, ACCOUNT)?);
    let marked = renderer.paint(&mut app)?;
    assert_ne!(app.word_highlight, WordHighlight::Off);
    let placements = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth());
    let title = positions(&marked, "first-owner")
        .into_iter()
        .find(|point| {
            placements
                .iter()
                .any(|placement| placement.frame.rect().y == point.y)
        })
        .ok_or("painted title")?;
    let space = Position::new(title.x - 1, title.y);
    assert_eq!(marked[(space.x, space.y)].symbol(), " ");
    app.click(space);
    assert_eq!(app.word_highlight, WordHighlight::Off);
    Ok(())
}

#[test]
fn a_real_truncation_ellipsis_resolves_only_the_visible_name() -> TestResult {
    const FULL_NAME: &str = "resourcesabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";
    let mut app = application()?;
    apply_rows(&mut app, vec![session(10, "truncation-owner", FULL_NAME)?]);
    let mut renderer = Renderer::new(&mut app, 80, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    let start = tree_word(&before, "resources")?;
    let ellipsis = (start.x..before.area.right())
        .find(|&x| before[(x, start.y)].symbol() == "…")
        .ok_or("painted name ellipsis")?;
    assert_eq!(
        positions(&before, FULL_NAME),
        [] as [ratatui::prelude::Position; 0]
    );
    let visible: String = (start.x..ellipsis)
        .map(|x| before[(x, start.y)].symbol())
        .collect();
    app.click(Position::new(ellipsis, start.y));
    let WordHighlight::Word(word) = &app.word_highlight else {
        return Err("ellipsis must resolve the painted token".into());
    };
    assert_eq!(word.text(), visible);
    assert_ne!(word.text(), FULL_NAME);
    assert_matches(&renderer.paint(&mut app)?, &visible, match_background()?)
}

#[test]
fn pinned_account_headers_mark_words_without_breaking_the_sort_or_selection() -> TestResult {
    let mut app = application()?;
    app.loaded_config.config.tiles.view = TileView::Summary;
    let accounts: Vec<_> = (0..6)
        .map(|index| {
            serde_json::json!({
                "label":format!("pinned-account-{index}"),"roles":["showrunners","units"],
                "weekly_left_percent":50,"use_credits":false
            })
        })
        .collect();
    app.accounts.table = AccountsTable::Ready(
        AccountsReading::parse(&serde_json::to_vec(
            &serde_json::json!({"accounts":accounts,"problems":[]}),
        )?)
        .map_err(|error| error.to_string())?,
    );
    let mut renderer = Renderer::new(&mut app, 80, 16)?;
    let before = renderer.paint(&mut app)?;
    app.click(locate(&before, "pinned-account-0")?);
    key(&mut app, KeyCode::End);
    let pinned = renderer.paint(&mut app)?;
    let state = app
        .selections
        .surface(&SelectionSurface::Summary)
        .map_or(None, Some)
        .ok_or("summary selection")?;
    assert!(
        matches!(state.painted_mapping(), ViewportMapping::Pinned { .. }),
        "the header must be pinned"
    );
    let selected = state.selection();
    let old_sort = app.accounts_sort;
    app.click(header_word(&pinned, "account")?);
    assert_ne!(app.accounts_sort, old_sort);
    let marked = renderer.paint(&mut app)?;
    assert_matches(&marked, "account", match_background()?)?;
    app.click(locate(&marked, "accounts")?);
    let heading = renderer.paint(&mut app)?;
    assert_matches(&heading, "accounts", match_background()?)?;
    assert_eq!(
        app.selections.surface(&SelectionSurface::Summary).map_or(
            TileSelection::NoRows,
            super::super::interaction::SurfaceSelection::selection
        ),
        selected
    );
    Ok(())
}

#[test]
fn fullwidth_parentheses_mark_the_word_without_marking_the_parentheses() -> TestResult {
    assert_bracketed_account("（claude1）")
}

#[test]
fn corner_brackets_mark_the_word_without_marking_the_brackets() -> TestResult {
    assert_bracketed_account("「claude1」")
}

fn assert_bracketed_account(name: &str) -> TestResult {
    let mut app = application()?;
    apply_rows(&mut app, vec![session(10, "bracket-owner", name)?]);
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let before = renderer.paint(&mut app)?;
    let opening = tree_word(&before, name)?;
    let opening_width = u16::try_from(
        unicode_width::UnicodeWidthChar::width(name.chars().next().ok_or("opening bracket")?)
            .ok_or("bracket cell width")?,
    )?;
    app.click(Position::new(opening.x + opening_width, opening.y));
    renderer.paint(&mut app)?;
    key(&mut app, KeyCode::Esc);
    let selected_reference = renderer.paint(&mut app)?;
    app.click(Position::new(opening.x + opening_width, opening.y));
    let after = renderer.paint(&mut app)?;
    let WordHighlight::Word(word) = &app.word_highlight else {
        return Err("the bracketed word must resolve".into());
    };
    assert_eq!(word.text(), ACCOUNT);
    let plain = positions(&after, ACCOUNT)
        .into_iter()
        .find(|point| point.y != opening.y)
        .ok_or("plain account occurrence")?;
    for x in plain.x..plain.x + u16::try_from(ACCOUNT.len())? {
        assert_eq!(
            after[(x, plain.y)].bg,
            match_background()?,
            "the bracketed press also marks the plain account"
        );
    }
    let grounds = crate::theme::WordMatchGrounds::new(match_background()?);
    let selected_match = grounds.background(grounds.surface(after[(opening.x, opening.y)].bg));
    for x in opening.x + opening_width..opening.x + opening_width + u16::try_from(ACCOUNT.len())? {
        assert_eq!(
            after[(x, opening.y)].bg,
            selected_match,
            "the bracketed token receives the same word mark"
        );
    }
    let closing_x = opening.x + opening_width + u16::try_from(ACCOUNT.len())?;
    let closing_width = u16::try_from(
        unicode_width::UnicodeWidthChar::width(name.chars().last().ok_or("closing bracket")?)
            .ok_or("bracket cell width")?,
    )?;
    for x in (opening.x..opening.x + opening_width).chain(closing_x..closing_x + closing_width) {
        assert_eq!(
            after[(x, opening.y)].bg,
            selected_reference[(x, opening.y)].bg,
            "a bracket keeps its unmatched cell ground at column {x}"
        );
    }
    Ok(())
}

#[test]
fn idle_frames_keep_no_captured_words_until_a_word_press() -> TestResult {
    let mut app = application()?;
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let idle = renderer.paint(&mut app)?;
    assert_eq!(app.word_highlight, WordHighlight::Off);
    assert!(
        app.painted_words.is_uncaptured(),
        "an idle frame stores no captured words"
    );
    let point = locate(&idle, ACCOUNT)?;
    app.click(point);
    let WordHighlight::Word(word) = &app.word_highlight else {
        return Err("the first word press must resolve before the next draw".into());
    };
    assert_eq!(word.text(), ACCOUNT);
    let marked = renderer.paint(&mut app)?;
    assert_matches(&marked, ACCOUNT, match_background()?)?;
    key(&mut app, KeyCode::Esc);
    renderer.paint(&mut app)?;
    assert_eq!(app.word_highlight, WordHighlight::Off);
    assert!(
        app.painted_words.is_uncaptured(),
        "an Off frame releases the captured words"
    );
    Ok(())
}

#[test]
fn a_press_before_the_first_frame_keeps_the_existing_word() -> TestResult {
    let mut app = application()?;
    app.word_highlight
        .press(crate::word_highlight::HighlightWord::from_token(ACCOUNT));
    let highlight = app.word_highlight.clone();
    app.click(Position::new(10, 10));
    assert_eq!(app.word_highlight, highlight);
    Ok(())
}

#[test]
fn a_body_invalidation_keeps_the_painted_word_available_for_a_content_press() -> TestResult {
    let mut app = application()?;
    app.word_highlight
        .press(crate::word_highlight::HighlightWord::from_token(ACCOUNT));
    let mut renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    let painted = renderer.paint(&mut app)?;
    let point = tree_word(&painted, "first-work")?;
    app.invalidate_body();
    app.click(point);
    assert_eq!(app.word_highlight, {
        let mut expected = WordHighlight::Off;
        expected.press(crate::word_highlight::HighlightWord::from_token(
            "first-work",
        ));
        expected
    });
    assert!(selections(&app).iter().any(|selection| matches!(
        selection,
        TileSelection::Row(crate::interaction::RowIdentity::Shell { pid: 11, .. })
    )));
    Ok(())
}
