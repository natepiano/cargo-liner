use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use tui_pane::KeySequence;
use tui_pane::KeymapPane;

use crate::tui::app::App;

/// Every scope/action pair currently bound to a vim motion key, named
/// `scope.action`. The settings overlay warns with this before turning
/// vim navigation on.
pub fn vim_mode_conflicts(app: &App) -> Vec<String> {
    tui_pane::vim_motion_conflicts(KeymapPane::ordered_help_rows(app, &app.framework_keymap))
}

/// Whether `bind` is a vim alias the keymap generated rather than one
/// the user configured. Generated aliases exist only while vim mode is
/// on, so the keymap writer strips them rather than freezing them into
/// the TOML.
pub fn is_generated_vim_extra(scope: &str, action_key: &str, key_sequence: &KeySequence) -> bool {
    if scope == "navigation" {
        return tui_pane::is_navigation_generated_vim_extra(action_key, key_sequence);
    }
    if scope != "project_list" {
        return false;
    }
    let Some(framework_key_bind) = key_sequence.single_key() else {
        return false;
    };
    framework_key_bind.mods == KeyModifiers::NONE
        && matches!(
            (action_key, framework_key_bind.code),
            ("expand_row", KeyCode::Char('l')) | ("collapse_row", KeyCode::Char('h'))
        )
}
