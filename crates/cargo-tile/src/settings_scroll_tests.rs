//! The app dispatcher keeps off-screen settings selectable and editable.

#![expect(clippy::expect_used, reason = "tests panic on unexpected values")]

use std::fs;
use std::sync::Arc;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use tui_pane::FrameworkSetting;
use tui_pane::SettingTarget;
use tui_pane::SettingsPane;
use tui_pane::TerminalApp;
use tui_pane::TileView;
use tui_pane::dispatch_key;

use super::AppSetting;
use super::CommandList;
use super::rows;
use crate::app::App;
use crate::render;
use crate::theme::builtins;

fn key(app: &mut App, code: KeyCode) { dispatch_key(app, KeyEvent::new(code, KeyModifiers::NONE)); }

fn draw(app: &mut App, terminal: &mut Terminal<TestBackend>) {
    let keymap = app.keymap.clone();
    terminal
        .draw(|frame| render::draw(frame, app, &keymap))
        .expect("draw settings");
}

#[test]
fn settings_scroll_dispatcher_selects_and_edits_a_row_by_key_and_click() {
    let mut app = App::new_for_test().expect("settings app");
    let mut terminal = Terminal::new(TestBackend::new(100, 8)).expect("short terminal");
    key(&mut app, KeyCode::Char('s'));
    draw(&mut app, &mut terminal);
    let settings = rows(&app);
    let target = (0..settings.rows().len())
        .find(|&index| {
            settings.target(index)
                == Some(SettingTarget::AppText(AppSetting::Commands(
                    CommandList::DefaultArguments,
                )))
        })
        .expect("default arguments row");
    assert!(target > app.framework.settings_pane.viewport().visible_rows());
    key(&mut app, KeyCode::End);
    for _ in 0..settings.rows().len() {
        if app.framework.settings_pane.viewport().pos() == target {
            break;
        }
        key(&mut app, KeyCode::Up);
    }
    draw(&mut app, &mut terminal);
    assert_eq!(app.framework.settings_pane.viewport().pos(), target);
    key(&mut app, KeyCode::Enter);
    assert!(app.framework.settings_pane.is_editing());
    assert_eq!(
        app.framework.settings_pane.edit_buffer(),
        tui_pane::join_list(
            app.loaded_config
                .config
                .commands
                .default_arguments
                .entries()
        )
    );
    key(&mut app, KeyCode::Esc);
    draw(&mut app, &mut terminal);
    let area = app.framework.settings_pane.viewport().content_area();
    let buffer = terminal.backend().buffer();
    let y = (area.y..area.bottom())
        .find(|&y| {
            let line: String = (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            line.contains("default arguments")
        })
        .expect("target row is drawn after scrolling");
    key(&mut app, KeyCode::Home);
    app.click(Position::new(area.x + 3, y));
    assert_eq!(app.framework.settings_pane.viewport().pos(), target);
    key(&mut app, KeyCode::Enter);
    assert!(app.framework.settings_pane.is_editing());
    assert_eq!(
        app.framework.settings_pane.edit_buffer(),
        tui_pane::join_list(
            app.loaded_config
                .config
                .commands
                .default_arguments
                .entries()
        )
    );
}

#[test]
fn the_overlay_selected_row_meets_contrast() {
    let directory = tempfile::tempdir().expect("theme directory");
    fs::write(
        directory.path().join("starter.toml"),
        include_str!("../themes/starter.toml"),
    )
    .expect("write starter theme");
    let registry = tui_pane::ThemeRegistry::from_dir_with_builtins(
        Some(directory.path()),
        builtins::builtins(),
    );
    assert!(
        registry
            .all()
            .any(|variant| variant.id.as_str() == "Cargo Tile Dark")
    );

    let mut app = App::new_for_test().expect("settings app");
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("settings terminal");
    app.loaded_config.config.tiles.view = TileView::Summary;
    key(&mut app, KeyCode::Char('s'));
    draw(&mut app, &mut terminal);
    let settings = rows(&app);
    let target = (0..settings.selectable_count())
        .find(|&index| {
            settings.target(index) == Some(SettingTarget::Framework(FrameworkSetting::TileView))
        })
        .expect("view setting");
    app.framework.settings_pane.select_row(target);

    let previous_theme = tui_pane::theme();
    for variant in registry.all() {
        tui_pane::set_active_theme(Arc::new(variant.theme.clone()));
        draw(&mut app, &mut terminal);
        let area = app.framework.settings_pane.viewport().content_area();
        let buffer = terminal.backend().buffer();
        SettingsPane::assert_selected_row_contrast(
            buffer,
            area,
            "▶ view < summary >",
            variant.id.as_str(),
        );
    }
    tui_pane::set_active_theme(previous_theme);
}

#[test]
fn settings_scroll_dispatcher_keeps_end_before_the_first_draw() {
    let mut app = App::new_for_test().expect("settings app");
    let mut terminal = Terminal::new(TestBackend::new(100, 8)).expect("short terminal");
    key(&mut app, KeyCode::Char('s'));
    let settings = rows(&app);
    let row_count = (0..settings.rows().len())
        .filter(|&index| settings.target(index).is_some())
        .count();
    assert!(row_count > 8, "settings exceed the terminal height");
    key(&mut app, KeyCode::End);
    draw(&mut app, &mut terminal);
    assert_eq!(
        app.framework.settings_pane.viewport().pos(),
        row_count - 1,
        "End between opening settings and the first draw must reach the last row"
    );
    assert!(app.framework.settings_pane.viewport().scroll_offset() > 0);
}
