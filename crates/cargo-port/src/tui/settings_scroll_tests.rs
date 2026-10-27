//! Settings selection below the first screen through the real input dispatcher.

#![expect(clippy::expect_used, reason = "tests panic on unexpected values")]

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::SettingOption;
use super::selected_setting;
use super::selection_index_for_setting;
use crate::tui::app::App;
use crate::tui::input;
use crate::tui::render;
use crate::tui::test_support;

fn key(app: &mut App, code: KeyCode) {
    input::handle_event(app, &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn draw(app: &mut App, terminal: &mut Terminal<TestBackend>) {
    terminal
        .draw(|frame| render::ui(frame, app))
        .expect("draw settings");
}

fn at_end() -> (App, Terminal<TestBackend>, usize) {
    let mut app = test_support::make_app(&[]);
    let mut terminal = Terminal::new(TestBackend::new(100, 12)).expect("short terminal");
    key(&mut app, KeyCode::Char('s'));
    draw(&mut app, &mut terminal);
    let target = selection_index_for_setting(&app, SettingOption::LintIdleBeforeLintSecs)
        .expect("last editable setting");
    assert!(target > app.framework.settings_pane.viewport().visible_rows());
    for _ in 0..target {
        key(&mut app, KeyCode::Down);
        draw(&mut app, &mut terminal);
    }
    (app, terminal, target)
}

#[test]
fn settings_scroll_keys_select_and_activate_the_last_row() {
    let (mut app, _terminal, target) = at_end();
    assert_eq!(app.framework.settings_pane.viewport().pos(), target);
    assert!(app.framework.settings_pane.viewport().scroll_offset() > 0);
    assert_eq!(
        selected_setting(&app),
        Some(SettingOption::LintIdleBeforeLintSecs)
    );
    let expected = app.config.current().lint.idle_before_lint_secs.to_string();
    key(&mut app, KeyCode::Enter);
    assert!(app.framework.settings_pane.is_editing());
    assert_eq!(app.framework.settings_pane.edit_buffer(), expected);
}

#[test]
fn settings_scroll_click_selects_the_drawn_row_after_port_padding() {
    let (mut app, terminal, target) = at_end();
    let area = app.framework.settings_pane.viewport().content_area();
    let buffer = terminal.backend().buffer();
    let y = (area.y..area.bottom())
        .find(|&y| {
            let line: String = (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            line.contains("Idle before lint")
        })
        .expect("last setting is drawn below the first screen");
    // Keep the last painted offset: this key has not been painted yet.
    key(&mut app, KeyCode::Up);
    input::handle_event(
        &mut app,
        &Event::Mouse(MouseEvent {
            kind:      MouseEventKind::Down(MouseButton::Left),
            column:    area.x + 3,
            row:       y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(
        app.framework.settings_pane.viewport().pos(),
        target,
        "drawn row plus drawn offset must select the labelled setting"
    );
    let expected = app.config.current().lint.idle_before_lint_secs.to_string();
    key(&mut app, KeyCode::Enter);
    assert!(app.framework.settings_pane.is_editing());
    assert_eq!(app.framework.settings_pane.edit_buffer(), expected);
}

#[test]
fn settings_scroll_wheel_cancels_an_edit_before_enter_on_another_setting() {
    let mut app = test_support::make_app(&[]);
    let mut terminal = Terminal::new(TestBackend::new(100, 12)).expect("short terminal");
    key(&mut app, KeyCode::Char('s'));
    draw(&mut app, &mut terminal);
    let first =
        selection_index_for_setting(&app, SettingOption::MainBranch).expect("main branch setting");
    for _ in 0..first {
        key(&mut app, KeyCode::Down);
        draw(&mut app, &mut terminal);
    }
    assert_eq!(selected_setting(&app), Some(SettingOption::MainBranch));
    let main_branch = app.config.current().tui.main_branch.clone();
    let other_branches = app.config.current().tui.other_primary_branches.clone();
    key(&mut app, KeyCode::Enter);
    assert!(app.framework.settings_pane.is_editing());
    for character in "-phase33-unsaved".chars() {
        key(&mut app, KeyCode::Char(character));
    }
    let area = app.framework.settings_pane.viewport().content_area();
    let wheel_down = if app.config.invert_scroll().is_inverted() {
        MouseEventKind::ScrollUp
    } else {
        MouseEventKind::ScrollDown
    };
    input::handle_event(
        &mut app,
        &Event::Mouse(MouseEvent {
            kind:      wheel_down,
            column:    area.x + 3,
            row:       area.y,
            modifiers: KeyModifiers::NONE,
        }),
    );
    assert_eq!(
        selected_setting(&app),
        Some(SettingOption::OtherPrimaryBranches)
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.config.current().tui.main_branch,
        main_branch,
        "wheel must not commit the old edit"
    );
    assert_eq!(
        app.config.current().tui.other_primary_branches,
        other_branches,
        "wheel must not save the old buffer into the next setting"
    );
    assert!(
        app.framework.settings_pane.is_editing(),
        "Enter opens the newly selected row"
    );
    assert!(
        !app.framework
            .settings_pane
            .edit_buffer()
            .contains("phase33-unsaved")
    );
}

#[test]
fn settings_scroll_config_reload_discards_edit_before_forced_include_dirs_activation() {
    let mut app = test_support::make_app(&[]);
    let mut terminal = Terminal::new(TestBackend::new(100, 12)).expect("short terminal");
    key(&mut app, KeyCode::Char('s'));
    draw(&mut app, &mut terminal);
    let first =
        selection_index_for_setting(&app, SettingOption::MainBranch).expect("main branch setting");
    for _ in 0..first {
        key(&mut app, KeyCode::Down);
    }
    assert_eq!(selected_setting(&app), Some(SettingOption::MainBranch));
    key(&mut app, KeyCode::Enter);
    assert!(app.framework.settings_pane.is_editing());
    for character in "-phase33-reload-unsaved".chars() {
        key(&mut app, KeyCode::Char(character));
    }

    let mut reloaded = app.config.current().clone();
    reloaded.tui.include_dirs.clear();
    let directory = tempfile::tempdir().expect("config directory");
    let path = directory.path().join("config.toml");
    std::fs::write(
        &path,
        toml::to_string_pretty(&reloaded).expect("serialize config without include directories"),
    )
    .expect("write reloaded config");
    app.config.force_reload_from(path);
    app.maybe_reload_config_from_disk();
    assert_eq!(selected_setting(&app), Some(SettingOption::IncludeDirs));
    let edit_was_cancelled = !app.framework.settings_pane.is_editing();

    // The forced OpenSettings action can close an already open overlay.
    if app.framework.overlay().is_none() {
        key(&mut app, KeyCode::Char('s'));
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.config.current().tui.include_dirs,
        reloaded.tui.include_dirs,
        "forced selection must not save the old MainBranch buffer into IncludeDirs"
    );
    assert_eq!(
        app.config.current(),
        &reloaded,
        "activating the forced row must not save the old edit into any setting"
    );
    assert!(
        edit_was_cancelled,
        "forced selection must leave the old editor in browse mode before Enter"
    );
}
