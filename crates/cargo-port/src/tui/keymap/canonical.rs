use crossterm::event::KeyCode;

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
