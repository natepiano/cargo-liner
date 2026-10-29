use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use tui_pane::KeyBind as FrameworkKeyBind;
use tui_pane::KeySequence;
use tui_pane::KeymapHelpRowKind;
use tui_pane::KeymapPane;

use crate::tui::app::App;

/// Every scope/action pair currently bound to a vim motion key, named
/// `scope.action`. The settings overlay warns with this before turning
/// vim navigation on.
pub fn vim_mode_conflicts(app: &App) -> Vec<String> {
    KeymapPane::ordered_help_rows(app, &app.framework_keymap)
        .into_iter()
        .filter(|keymap_help_row| keymap_help_row.row_kind != KeymapHelpRowKind::Header)
        .filter_map(|keymap_help_row| {
            let framework_key_bind = keymap_help_row.bind?.single_key()?;
            (framework_key_bind.mods == KeyModifiers::NONE
                && matches!(
                    framework_key_bind.code,
                    KeyCode::Char('h' | 'j' | 'k' | 'l')
                ))
            .then(|| format!("{}.{}", keymap_help_row.scope, keymap_help_row.action))
        })
        .collect()
}

/// Whether `bind` is a vim alias the keymap generated rather than one
/// the user configured. Generated aliases exist only while vim mode is
/// on, so the keymap writer strips them rather than freezing them into
/// the TOML.
pub fn is_generated_vim_extra(scope: &str, action_key: &str, key_sequence: &KeySequence) -> bool {
    if scope == "navigation" {
        return is_navigation_generated_vim_extra(action_key, key_sequence);
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

/// Every vim navigation alias is generated and stripped from the
/// written TOML — the letters (h/j/k/l/G) and the Ctrl page /
/// half-page motions (Ctrl-b/f/u/d). None of these are keymappable;
/// they exist only while vim mode is on (see
/// `vim_letter_extras`).
fn is_navigation_generated_vim_extra(action_key: &str, key_sequence: &KeySequence) -> bool {
    let Some(framework_key_bind) = key_sequence.single_key() else {
        return action_key == "home"
            && key_sequence.keys() == [FrameworkKeyBind::from('g'), FrameworkKeyBind::from('g')];
    };
    matches!(
        (action_key, framework_key_bind.code, framework_key_bind.mods),
        ("left", KeyCode::Char('h'), KeyModifiers::NONE)
            | ("down", KeyCode::Char('j'), KeyModifiers::NONE)
            | ("up", KeyCode::Char('k'), KeyModifiers::NONE)
            | ("right", KeyCode::Char('l'), KeyModifiers::NONE)
            | ("end", KeyCode::Char('G'), KeyModifiers::NONE)
            | ("page_up", KeyCode::Char('b'), KeyModifiers::CONTROL)
            | ("page_down", KeyCode::Char('f'), KeyModifiers::CONTROL)
            | ("half_page_up", KeyCode::Char('u'), KeyModifiers::CONTROL)
            | ("half_page_down", KeyCode::Char('d'), KeyModifiers::CONTROL)
    )
}
