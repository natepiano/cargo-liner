//! Favorites rows use the same mouse and keyboard selection.

use std::fmt::Write as _;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::layout::Position;
use tui_pane::FavoritesFileState;
use tui_pane::ScrollDirection;
use tui_pane::TerminalApp;

use super::word_click_tests;
use super::word_click_tests::TestResult;
use crate::app::App;

#[test]
fn favorites_click_and_wheel_reach_the_keyboard_row() -> TestResult {
    let mut app = App::new_for_test()?;
    let text = ["unknown_first", "unknown_second"].iter().enumerate().try_fold(
        String::new(),
        |mut text, (index, name)| {
            writeln!(
                text,
                "[[favorite]]\nid = \"01a03f62-9c14-7b41-8a02-1de4c7c9b33{index}\"\nsaved = \"2026-08-26T14:31:05-07:00\"\nmode = \"{name}\""
            )
            .map(|()| text)
        },
    )?;
    tui_pane::open_favorites_on_state_for_test(
        &mut app,
        FavoritesFileState::Loaded {
            path: "/fixture/favorites.toml".into(),
            rows: tui_pane::parse_favorite_rows_for_test(&text)?,
        },
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24))?;
    terminal.draw(|frame| {
        app.favorites_overlay
            .render(frame, app.framework.color_mode);
    })?;
    let target = word_click_tests::locate(terminal.backend().buffer(), "unknown_second")?;
    tui_pane::dispatch_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    terminal.draw(|frame| {
        app.favorites_overlay
            .render(frame, app.framework.color_mode);
    })?;
    let keyboard = terminal.backend().buffer()[(target.x, target.y)].clone();
    tui_pane::dispatch_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    app.click(target);
    terminal.draw(|frame| {
        app.favorites_overlay
            .render(frame, app.framework.color_mode);
    })?;
    assert_eq!(terminal.backend().buffer()[(target.x, target.y)], keyboard);
    app.scroll(target, ScrollDirection::Up);
    terminal.draw(|frame| {
        app.favorites_overlay
            .render(frame, app.framework.color_mode);
    })?;
    assert_ne!(terminal.backend().buffer()[(target.x, target.y)], keyboard);
    app.scroll(target, ScrollDirection::Down);
    terminal.draw(|frame| {
        app.favorites_overlay
            .render(frame, app.framework.color_mode);
    })?;
    assert_eq!(terminal.backend().buffer()[(target.x, target.y)], keyboard);
    for point in [Position::new(target.x, target.y - 2), Position::new(0, 0)] {
        app.click(point);
        app.scroll(Position::new(0, 0), ScrollDirection::Up);
        terminal.draw(|frame| {
            app.favorites_overlay
                .render(frame, app.framework.color_mode);
        })?;
        assert_eq!(terminal.backend().buffer()[(target.x, target.y)], keyboard);
    }
    Ok(())
}
