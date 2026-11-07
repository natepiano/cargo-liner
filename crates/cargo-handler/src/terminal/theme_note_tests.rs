//! Rejected theme files stay visible in the real application settings overlay.

#![expect(
    clippy::expect_used,
    reason = "isolated theme files and rendered settings must exist"
)]
#![expect(
    clippy::panic,
    reason = "the unwind test forces a failure after creating its fixture"
)]

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;
use toml::Table;
use tui_pane::AppIdentity;
use tui_pane::Appearance;
use tui_pane::AppearanceConfig;
use tui_pane::TerminalApp;
use tui_pane::ThemeSource;

use crate::app::App;
use crate::config::Config;
use crate::config::LoadedConfig;
use crate::render;
use crate::theme;

const REJECTED_FILE: &str = "broken-default-dark.toml";

// This identity supplies fixture appearance defaults without reading the real configuration.
enum ThemeNoteFixture {}

fn theme_directory() -> TempDir {
    let dir = tempfile::tempdir().expect("theme directory");
    let mut document: Table = toml::from_str(include_str!("../../themes/default_dark.toml"))
        .expect("built-in dark template");
    document["variants"][0]
        .as_table_mut()
        .expect("variant")
        .remove("session_hues")
        .expect("required handler palette");
    std::fs::write(
        dir.path().join(REJECTED_FILE),
        toml::to_string(&document).expect("malformed handler theme"),
    )
    .expect("write rejected built-in override");
    dir
}

impl AppIdentity for ThemeNoteFixture {
    const BINARY_NAME: &'static str = "cargo-handler-theme-note-fixture";
    const CONFIG_DIRNAME: &'static str = "cargo-handler-theme-note-fixture";
    const DEFAULT_LIGHT_THEME: &'static str = "Default Light";
    const DEFAULT_DARK_THEME: &'static str = "Default Dark";

    fn themes_dir() -> Option<PathBuf> { None }
}

fn installed_app(theme_directory: &TempDir, first_answer: Option<Appearance>) -> App {
    let loaded_config = LoadedConfig {
        config: Config::default(),
        error:  None,
    };
    let installation = theme::install_theme(
        &AppearanceConfig::<ThemeNoteFixture>::default(),
        theme::builtins(),
        ThemeSource::Directory(theme_directory.path()),
    );
    let notes = super::startup_theme_notes(&loaded_config, installation, first_answer);
    // This constructor shares production field construction without live I/O.
    App::new_for_test_with_startup(loaded_config, notes)
        .expect("app from installed theme directory")
}

fn assert_rejection_is_drawn(app: &mut App) {
    let keymap = app.keymap();
    tui_pane::dispatch_key(app, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    let mut terminal = Terminal::new(TestBackend::new(240, 90)).expect("settings terminal");
    terminal
        .draw(|frame| render::draw(frame, app, &keymap))
        .expect("settings frame");
    let buffer = terminal.backend().buffer();
    let text = buffer
        .content
        .chunks_exact(usize::from(buffer.area.width))
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .filter(|row| !row.trim().is_empty())
        .map(|row| row.trim().to_owned())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains(REJECTED_FILE),
        "rejected filename must draw in settings: {text}"
    );
    assert!(
        text.contains("missing field"),
        "rejection reason must draw in settings: {text}"
    );
    assert!(
        text.contains("session_hues"),
        "rejected field must draw in settings: {text}"
    );
}

#[test]
fn rejected_builtin_theme_note_reaches_the_rendered_settings_overlay() {
    let directory = theme_directory();
    assert_rejection_is_drawn(&mut installed_app(&directory, None));
}

#[test]
fn rejected_builtin_theme_note_survives_the_initial_appearance_answer() {
    let directory = theme_directory();
    let mut app = installed_app(&directory, Some(Appearance::Light));
    assert_rejection_is_drawn(&mut app);
}

#[test]
fn rejected_builtin_theme_note_survives_a_later_appearance_answer() {
    let directory = theme_directory();
    let mut app = installed_app(&directory, None);
    app.apply_system_appearance(Appearance::Light);
    assert_rejection_is_drawn(&mut app);
}

#[test]
fn theme_note_fixture_directory_is_removed_after_normal_completion() {
    let directory_path = {
        let directory = theme_directory();
        let path = directory.path().to_path_buf();
        let mut app = installed_app(&directory, None);
        assert_rejection_is_drawn(&mut app);
        path
    };

    assert!(
        !directory_path.exists(),
        "fixture directory must be removed"
    );
}

#[test]
fn theme_note_fixture_directory_is_removed_after_unwind() {
    let directory_path = std::cell::RefCell::new(None);
    let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let directory = theme_directory();
        *directory_path.borrow_mut() = Some(directory.path().to_path_buf());
        let mut app = installed_app(&directory, None);
        assert_rejection_is_drawn(&mut app);
        panic!("forced fixture failure");
    }));

    assert!(panic.is_err(), "fixture must reach the forced failure");
    let payload = panic.expect_err("fixture must reach the forced failure");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"forced fixture failure"),
        "fixture must finish before failure"
    );
    let directory_path = directory_path.into_inner().expect("fixture path recorded");
    assert!(
        !directory_path.exists(),
        "fixture directory must be removed on unwind"
    );
}
