//! Enter shows the focused tile on the full pane, and Esc returns it to the grid.

use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::layout::Position;
use tui_pane::FullScreenCell;
use tui_pane::Repaint;
use tui_pane::TerminalApp;

use super::word_click_tests::Renderer;
use super::word_click_tests::TestResult;
use super::word_click_tests::{self};
use super::*;
use crate::details::DetailsOverlay;
use crate::word_highlight::WordHighlight;

const WIDTH: u16 = 200;
const HEIGHT: u16 = 70;
const NOW: u64 = 1_000;
const OWNER: u32 = 20;

fn press(app: &mut App, code: KeyCode) {
    tui_pane::dispatch_key(app, KeyEvent::new(code, KeyModifiers::NONE));
}

/// Read the poll clock past the end of the motion, as the terminal poll does.
fn finish_motion(app: &mut App) {
    if app
        .full_screen
        .advance(Instant::now() + Duration::from_secs(60))
        == Repaint::Needed
    {
        app.invalidate_body();
    }
}

fn owner_key(app: &App) -> TestResult<TileKey> {
    app.tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .find_map(|placement| match placement.content {
            TileContent::Group(TileKey::Agent(cell)) if cell.pid == OWNER => {
                Some(TileKey::Agent(cell))
            },
            _ => None,
        })
        .ok_or_else(|| "owner tile".into())
}

fn row_selection(app: &App, key: &TileKey) -> Option<TileSelection> {
    app.selections
        .surface(&SelectionSurface::Tile(key.clone()))
        .map_or(None, |state| Some(state.selection()))
}

fn body_rows(buffer: &Buffer) -> Vec<String> {
    let body = buffer.area.bottom().saturating_sub(STATUS_LINE_HEIGHT);
    (buffer.area.top()..body)
        .map(|y| {
            (buffer.area.left()..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// A grid with the owner tile focused, its owner row selected and open.
fn focused_grid() -> TestResult<(App, Renderer, Buffer)> {
    let mut app = word_click_tests::application()?;
    let renderer = Renderer::with_fixed_time(&mut app, WIDTH, HEIGHT, NOW)?;
    focus_owner(app, renderer)
}

/// The focused grid painted through `App::draw`, which keeps the frame that word presses read.
fn retained_grid() -> TestResult<(App, Renderer, Buffer)> {
    let mut app = word_click_tests::application()?;
    let renderer = Renderer::new(&mut app, WIDTH, HEIGHT)?;
    focus_owner(app, renderer)
}

fn focus_owner(mut app: App, mut renderer: Renderer) -> TestResult<(App, Renderer, Buffer)> {
    let buffer = renderer.paint(&mut app)?;
    let owner = word_click_tests::positions(&buffer, "second-owner")
        .into_iter()
        .last()
        .ok_or("owner row")?;
    // A press on a row toggles it, so Right opens the owner row again.
    app.click(owner);
    press(&mut app, KeyCode::Right);
    let buffer = renderer.paint(&mut app)?;
    Ok((app, renderer, buffer))
}

fn expand(app: &mut App, renderer: &mut Renderer) -> TestResult<Buffer> {
    press(app, KeyCode::Enter);
    finish_motion(app);
    let buffer = renderer.paint(app)?;
    if !matches!(app.full_screen, FullScreenView::Expanded { .. }) {
        return Err("Enter must show the tile on the full pane".into());
    }
    Ok(buffer)
}

fn collapse(app: &mut App, renderer: &mut Renderer) -> TestResult<Buffer> {
    press(app, KeyCode::Esc);
    finish_motion(app);
    let buffer = renderer.paint(app)?;
    if app.full_screen != FullScreenView::Grid {
        return Err("Esc must return the tile to the grid".into());
    }
    Ok(buffer)
}

#[test]
fn enter_fills_the_pane_with_the_focused_tile_and_esc_restores_the_grid() -> TestResult {
    let (mut app, mut renderer, grid) = focused_grid()?;
    let key = owner_key(&app)?;
    let focus = app.tiles.focused_cell();
    let selection = row_selection(&app, &key);
    let full = expand(&mut app, &mut renderer)?;
    let cell = app.full_screen.cell().ok_or("full-screen cell")?;
    assert_eq!(cell.content(), &TileContent::Group(key.clone()));
    assert_eq!(cell.displayed(), app.selection_area);
    let text = body_rows(&full).join("\n");
    assert!(text.contains("second-owner"), "{text}");
    assert!(!text.contains("first-owner"), "{text}");
    let after = collapse(&mut app, &mut renderer)?;
    assert_eq!(app.tiles.focused_cell(), focus);
    assert_eq!(row_selection(&app, &key), selection);
    assert_eq!(body_rows(&after), body_rows(&grid));
    Ok(())
}

#[test]
fn right_and_left_open_and_close_rows_in_the_full_screen_tile() -> TestResult {
    let (mut app, mut renderer, grid) = focused_grid()?;
    assert!(body_rows(&grid).join("\n").contains("second-work"));
    expand(&mut app, &mut renderer)?;
    press(&mut app, KeyCode::Left);
    let closed = body_rows(&renderer.paint(&mut app)?).join("\n");
    assert!(!closed.contains("second-work"), "{closed}");
    press(&mut app, KeyCode::Right);
    let opened = body_rows(&renderer.paint(&mut app)?).join("\n");
    assert!(opened.contains("second-work"), "{opened}");
    assert!(matches!(app.full_screen, FullScreenView::Expanded { .. }));
    Ok(())
}

#[test]
fn up_and_down_move_the_row_selection_in_the_full_screen_tile() -> TestResult {
    let (mut app, mut renderer, _) = focused_grid()?;
    let key = owner_key(&app)?;
    expand(&mut app, &mut renderer)?;
    let owner = row_selection(&app, &key);
    press(&mut app, KeyCode::Down);
    renderer.paint(&mut app)?;
    let work = row_selection(&app, &key);
    assert_ne!(work, owner);
    press(&mut app, KeyCode::Up);
    renderer.paint(&mut app)?;
    assert_eq!(row_selection(&app, &key), owner);
    Ok(())
}

#[test]
fn focus_keys_keep_the_full_screen_tile_focused() -> TestResult {
    let (mut app, mut renderer, _) = focused_grid()?;
    let focus = app.tiles.focused_cell();
    expand(&mut app, &mut renderer)?;
    for code in [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down] {
        tui_pane::dispatch_key(&mut app, KeyEvent::new(code, KeyModifiers::SHIFT));
        renderer.paint(&mut app)?;
        assert_eq!(app.tiles.focused_cell(), focus, "{code:?}");
    }
    collapse(&mut app, &mut renderer)?;
    // In the grid, one of the four keys moves the focus off the owner tile.
    let moved = [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down]
        .into_iter()
        .any(|code| {
            tui_pane::dispatch_key(&mut app, KeyEvent::new(code, KeyModifiers::SHIFT));
            app.tiles.focused_cell() != focus
        });
    assert!(moved);
    Ok(())
}

#[test]
fn esc_closes_details_then_the_full_screen_tile_then_the_word_highlight() -> TestResult {
    let (mut app, mut renderer, _) = retained_grid()?;
    let full = expand(&mut app, &mut renderer)?;
    app.click(word_click_tests::locate(&full, "second-work")?);
    renderer.paint(&mut app)?;
    assert_ne!(app.word_highlight, WordHighlight::Off);
    press(&mut app, KeyCode::Char('d'));
    renderer.paint(&mut app)?;
    assert!(matches!(app.details_overlay, DetailsOverlay::Open { .. }));
    press(&mut app, KeyCode::Esc);
    renderer.paint(&mut app)?;
    assert!(matches!(app.details_overlay, DetailsOverlay::Closed));
    assert!(matches!(app.full_screen, FullScreenView::Expanded { .. }));
    collapse(&mut app, &mut renderer)?;
    assert_ne!(app.word_highlight, WordHighlight::Off);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.word_highlight, WordHighlight::Off);
    Ok(())
}

#[test]
fn the_full_screen_motion_moves_between_the_tile_and_the_pane() -> TestResult {
    let (mut app, mut renderer, _) = focused_grid()?;
    let key = owner_key(&app)?;
    let tile = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .find(|placement| placement.content == TileContent::Group(key.clone()))
        .ok_or("owner placement")?
        .frame
        .clip();
    press(&mut app, KeyCode::Enter);
    let FullScreenView::Opening { started, .. } = app.full_screen else {
        return Err("Enter must open the full-screen tile".into());
    };
    let midpoint = started + app.tiles.cell_travel_duration() / 2;
    assert_eq!(app.full_screen.advance(midpoint), Repaint::Needed);
    app.invalidate_body();
    renderer.paint(&mut app)?;
    let shown = app.full_screen.cell().ok_or("moving cell")?.displayed();
    assert!(matches!(app.full_screen, FullScreenView::Expanding { .. }));
    assert!(
        shown.area() > tile.area() && shown.area() < app.selection_area.area(),
        "{shown:?}"
    );
    assert_eq!(app.full_screen.advance(midpoint), Repaint::Needed);
    Ok(())
}

#[test]
fn a_word_click_in_the_full_screen_tile_repaints_its_highlight() -> TestResult {
    let (mut app, mut renderer, _) = retained_grid()?;
    let full = expand(&mut app, &mut renderer)?;
    let word = word_click_tests::locate(&full, "second-work")?;
    let before = full[word].bg;
    app.click(word);
    let highlighted = renderer.paint(&mut app)?;
    assert_ne!(highlighted[word].bg, before);
    word_click_tests::assert_matches(
        &highlighted,
        "second-work",
        word_click_tests::match_background()?,
    )
}

#[test]
fn resize_and_collapse_keep_the_selection_and_bring_back_the_grid() -> TestResult {
    let (mut app, mut renderer, grid) = focused_grid()?;
    let key = owner_key(&app)?;
    let selection = row_selection(&app, &key);
    expand(&mut app, &mut renderer)?;
    let mut smaller = Renderer::with_fixed_time(&mut app, WIDTH - 40, HEIGHT - 20, NOW)?;
    smaller.paint(&mut app)?;
    assert_eq!(
        app.full_screen.cell().map(FullScreenCell::displayed),
        Some(app.selection_area)
    );
    assert_eq!(row_selection(&app, &key), selection);
    let mut renderer = Renderer::with_fixed_time(&mut app, WIDTH, HEIGHT, NOW)?;
    let after = collapse(&mut app, &mut renderer)?;
    assert_eq!(row_selection(&app, &key), selection);
    assert_eq!(body_rows(&after), body_rows(&grid));
    Ok(())
}

#[test]
fn header_clicks_in_the_full_screen_tile_hit_the_drawn_headers() -> TestResult {
    let (mut app, mut renderer, _) = focused_grid()?;
    let key = owner_key(&app)?;
    let full = expand(&mut app, &mut renderer)?;
    let surface = app
        .painted_surfaces
        .iter()
        .rev()
        .find(|surface| surface.surface() == SelectionSurface::Tile(key.clone()))
        .ok_or("painted full-screen tile")?;
    assert_eq!(surface.area.width, app.selection_area.width - 2);
    let PaintedSurfaceContent::Tree { spans, .. } = &surface.content else {
        return Err("tree surface".into());
    };
    let status = spans
        .iter()
        .find(|span| span.column == TreeColumn::Status)
        .ok_or("status header")?;
    let point = Position::new(
        surface.area.x + status.columns.start,
        surface.area.y + status.row,
    );
    assert_eq!(full[point].symbol(), "s");
    app.click(point);
    assert_eq!(
        app.tree_sorts.get(&key).ok_or("clicked sort")?.column,
        TreeColumn::Status
    );
    Ok(())
}

#[test]
fn a_full_screen_tile_that_leaves_the_census_returns_to_the_grid() -> TestResult {
    let (mut app, mut renderer, _) = focused_grid()?;
    expand(&mut app, &mut renderer)?;
    word_click_tests::apply_rows(
        &mut app,
        vec![word_click_tests::session(10, "first-owner", "first-work")?],
    );
    app.invalidate_body();
    let buffer = renderer.paint(&mut app)?;
    assert_eq!(app.full_screen, FullScreenView::Grid);
    let text = body_rows(&buffer).join("\n");
    assert!(text.contains("first-owner"), "{text}");
    assert!(!text.contains("second-owner"), "{text}");
    Ok(())
}

fn status_row(buffer: &Buffer) -> String {
    let y = buffer.area.bottom().saturating_sub(STATUS_LINE_HEIGHT);
    (buffer.area.left()..buffer.area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

#[test]
fn the_status_line_names_enter_in_the_grid_and_esc_on_the_full_screen() -> TestResult {
    let (mut app, mut renderer, grid) = focused_grid()?;
    let row = status_row(&grid);
    assert!(row.contains("enter expand"), "{row:?}");
    assert!(!row.contains("esc collapse"), "{row:?}");
    let row = status_row(&expand(&mut app, &mut renderer)?);
    assert!(row.contains("esc collapse"), "{row:?}");
    assert!(!row.contains("enter expand"), "{row:?}");
    let row = status_row(&collapse(&mut app, &mut renderer)?);
    assert!(row.contains("enter expand"), "{row:?}");
    Ok(())
}

#[test]
fn the_expand_hint_yields_to_uptime_at_a_narrow_width() -> TestResult {
    const NARROW: u16 = 60;
    let mut app = word_click_tests::application()?;
    let mut renderer = Renderer::with_fixed_time(&mut app, NARROW, HEIGHT, NOW)?;
    let row = status_row(&renderer.paint(&mut app)?);
    assert!(row.contains("Uptime"), "{row:?}");
    assert!(row.contains("? shortcuts"), "{row:?}");
    assert!(!row.contains("enter expand"), "{row:?}");
    Ok(())
}
