//! Actual application status-bar foregrounds and grounds under every built-in theme.
#![expect(clippy::expect_used, reason = "render fixture setup must succeed")]
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tui_pane::ShortcutState;
use tui_pane::TerminalApp;

use super::*;
use crate::theme;
#[path = "../../../tui_pane/src/bar/status_bar_contrast_support.rs"]
mod contrast;
fn check(theme_name: &str) {
    let previous = tui_pane::theme();
    let app = App::new_for_test().expect("bar app");
    let keymap = app.keymap();
    let variant = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == theme_name)
        .expect("built-in theme");
    tui_pane::set_active_theme(Arc::new(variant.theme));
    let mut terminal = Terminal::new(TestBackend::new(300, 1)).expect("bar terminal");
    terminal
        .draw(|frame| draw_status_line(frame, &app, &keymap, frame.area()))
        .expect("native app bar");
    let mut failures = contrast::failures(
        terminal.backend().buffer(),
        &["Uptime:", "?"],
        &format!("native {theme_name}"),
    );
    let expected_ground = if theme_name == "High Contrast Light" {
        [210, 210, 210]
    } else if theme_name == "Default Light" {
        [220, 220, 220]
    } else {
        [60, 60, 60]
    };
    assert_eq!(
        contrast::rgb(terminal.backend().buffer()[(0, 0)].bg),
        expected_ground,
        "handler bar uses its specified band ground"
    );

    // Drive both reachable shortcut states and activity through the same app keymap,
    // framework and production palette; all spans must survive actual bar composition.
    for state in [ShortcutState::Enabled, ShortcutState::Disabled] {
        let globals = [StatusLineGlobal::global_shortcuts_help().with_state(state)];
        let status = StatusLine::new(65, ScanIndicator::Shown, &[], &globals);
        terminal
            .draw(|frame| {
                tui_pane::render_status_line::<App, AppGlobalAction>(
                    frame,
                    frame.area(),
                    &app,
                    &keymap,
                    &app.framework,
                    &BarPalette::themed(),
                    &status,
                );
            })
            .expect("app status span states");
        failures.extend(contrast::failures(
            terminal.backend().buffer(),
            &["scanning", "Uptime:", "1m", "?", "shortcuts"],
            &format!("{theme_name} {state:?}"),
        ));
    }
    tui_pane::set_active_theme(previous);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn handler_default_dark_bar_keeps_all_span_states_readable() { check("Default Dark"); }
#[test]
fn handler_default_light_bar_keeps_all_span_states_readable() { check("Default Light"); }
#[test]
fn handler_high_contrast_dark_bar_keeps_all_span_states_readable() { check("High Contrast Dark"); }
#[test]
fn handler_high_contrast_light_bar_keeps_all_span_states_readable() {
    check("High Contrast Light");
}
