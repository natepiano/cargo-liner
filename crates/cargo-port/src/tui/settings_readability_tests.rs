use anyhow::Context;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use tui_pane::ColorMode;

use super::app::App;
use super::input;
use super::settings;
use super::settings::SettingOption;
use super::test_support;

fn select_setting(app: &mut App, setting: SettingOption) -> anyhow::Result<Buffer> {
    input::handle_event(
        app,
        &Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
    );
    test_support::draw_ui(app, 80)?;
    let target = settings::selection_index_for_setting_for_test(app, setting)
        .context("setting selection")?;
    for _ in 0..target {
        input::handle_event(
            app,
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
    }
    test_support::draw_ui(app, 80)
}

#[test]
fn phase4e_settings_wrap_paths_at_slashes_inside_the_popup_margin() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let path = "/tmp/claude/unit/b6e56837-8fc7-4cd5-ba71-0ff1c804cfd1/phases/global/cache";
    app.config.current_mut().cache.root = path.to_string();
    let buffer = select_setting(&mut app, SettingOption::CacheRoot)?;
    let area = app.framework.settings_pane.viewport().content_area();
    let rows: Vec<String> = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect();
    let start = rows
        .iter()
        .position(|row| row.contains("Cache root"))
        .context("Cache root row")?;
    let end = (start..rows.len())
        .find(|end| {
            let joined: String = rows[start..=*end]
                .iter()
                .flat_map(|row| row.chars())
                .filter(|character| !character.is_whitespace())
                .collect();
            joined.contains(path)
        })
        .context("the last path row")?;
    let path_rows: Vec<_> = rows[start..=end].iter().collect();
    let joined: String = path_rows
        .iter()
        .flat_map(|row| row.chars())
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(
        joined.contains(path),
        "the complete path must remain visible: {path_rows:?}"
    );
    for row in &path_rows {
        assert!(
            row.ends_with(' '),
            "the path needs a blank cell before the border: {row}"
        );
    }
    for row in path_rows.iter().take(path_rows.len().saturating_sub(1)) {
        assert!(
            row.trim_end().ends_with('/'),
            "break the path at a slash: {row}"
        );
    }
    Ok(())
}

#[test]
fn phase4e_selected_settings_use_one_readable_ink() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let previous = tui_pane::theme();
    tui_pane::set_active_theme(std::sync::Arc::new(test_support::shipped_dark_theme()?));
    let result = (|| -> anyhow::Result<()> {
        let buffer = select_setting(&mut app, SettingOption::InvertScroll)?;
        let label = test_support::position(&buffer, "Invert scroll")?;
        let area = app.framework.settings_pane.viewport().content_area();
        let mut selected =
            Buffer::empty(ratatui::layout::Rect::new(area.x, label.1, area.width, 1));
        for x in area.x..area.right() {
            selected[(x, label.1)] = buffer[(x, label.1)].clone();
            if !buffer[(x, label.1)].symbol().trim().is_empty() {
                assert_eq!(
                    buffer[(x, label.1)].fg,
                    buffer[label].fg,
                    "the selected value and label need one ink"
                );
            }
        }
        let toggle = if app.config.invert_scroll().is_inverted() {
            "ON"
        } else {
            "OFF"
        };
        let failures = test_support::contrast_failures(
            &selected,
            &["Invert scroll", toggle],
            "Settings selection",
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        Ok(())
    })();
    tui_pane::set_active_theme(previous);
    result
}

#[test]
fn phase4e_lint_cache_row_shows_the_cap_once() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.config.current_mut().lint.cache_size = "512 MiB".to_string();
    app.lint.cache_usage.bytes = 1;
    let buffer = select_setting(&mut app, SettingOption::LintCacheSize)?;
    let (_, y) = test_support::position(&buffer, "Cache size")?;
    let rows = test_support::rows(&buffer);
    let row = &rows[usize::from(y)];
    assert!(row.contains("1 B / 512 MiB"), "{row}");
    assert_eq!(
        row.matches("512 MiB").count(),
        1,
        "the cap must appear once: {row}"
    );
    Ok(())
}
