//! Rejected theme files stay visible in the real application settings overlay.

#![expect(
    clippy::expect_used,
    reason = "isolated theme files and rendered settings must exist"
)]

use std::path::PathBuf;
use std::sync::OnceLock;

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

use crate::app::App;
use crate::config::Config;
use crate::config::LoadedConfig;
use crate::render;
use crate::theme;

const REJECTED_FILE: &str = "broken-default-dark.toml";

// This identity isolates file loading without changing HOME or the real config.
enum ThemeNoteFixture {}

fn theme_directory() -> &'static TempDir {
    static DIRECTORY: OnceLock<TempDir> = OnceLock::new();
    DIRECTORY.get_or_init(|| {
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
    })
}

impl AppIdentity for ThemeNoteFixture {
    const BINARY_NAME: &'static str = "cargo-handler-theme-note-fixture";
    const CONFIG_DIRNAME: &'static str = "cargo-handler-theme-note-fixture";
    const DEFAULT_LIGHT_THEME: &'static str = "Default Light";
    const DEFAULT_DARK_THEME: &'static str = "Default Dark";

    fn themes_dir() -> Option<PathBuf> { Some(theme_directory().path().to_path_buf()) }
}

fn installed_app(first_answer: Option<Appearance>) -> App {
    let loaded_config = LoadedConfig {
        config: Config::default(),
        error:  None,
    };
    let installation = theme::install_theme(
        &AppearanceConfig::<ThemeNoteFixture>::default(),
        theme::builtins(),
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
    assert_rejection_is_drawn(&mut installed_app(None));
}

#[test]
fn rejected_builtin_theme_note_survives_the_initial_appearance_answer() {
    let mut app = installed_app(Some(Appearance::Light));
    assert_rejection_is_drawn(&mut app);
}

#[test]
fn rejected_builtin_theme_note_survives_a_later_appearance_answer() {
    let mut app = installed_app(None);
    app.apply_system_appearance(Appearance::Light);
    assert_rejection_is_drawn(&mut app);
}
