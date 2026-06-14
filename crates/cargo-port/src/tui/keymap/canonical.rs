use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;

/// Cargo-port's [`tui_pane::KeyBind::canonicalize_code`] hook: collapses
/// `+` and `=` so a user binding to either key matches presses of
/// both. Apply at every load-time `KeyBind` construction and at the
/// crossterm-event dispatch boundary so storage and lookup agree.
pub(crate) const fn canonical_code(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char('+') => KeyCode::Char('='),
        other => other,
    }
}

pub(crate) fn canonical_event_code_and_mods(code: KeyCode, mods: KeyModifiers) -> (KeyCode, KeyModifiers) {
    let code = canonical_code(code);
    let mods = if matches!(code, KeyCode::Char('=' | '+')) {
        mods - KeyModifiers::SHIFT
    } else {
        mods
    };
    (code, mods)
}
