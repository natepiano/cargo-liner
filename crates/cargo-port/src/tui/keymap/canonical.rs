//! Converts terminal [`KeyEvent`](crossterm::event::KeyEvent) values to the
//! representation stored in cargo-port's keymap.
//!
//! `REPORT_ALL_KEYS_AS_ESCAPE_CODES` makes terminals report shifted text keys
//! as CSI-u events. Depending on the terminal, Shift+`/` can arrive as `/` plus
//! Shift or `?` plus Shift. Keymap TOML stores the text result as bare `?`, so
//! [`canonical_event_code_and_mods`] converts both CSI-u forms to that value.
//! Shift+Space is the exception: Shift does not change the character, and the
//! modifier distinguishes `pause_all_lints` from `pause_selected_lint`.

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

pub(crate) fn canonical_event_code_and_mods(
    code: KeyCode,
    mods: KeyModifiers,
) -> (KeyCode, KeyModifiers) {
    let (code, mods) = canonical_shifted_ascii(code, mods);
    let code = canonical_code(code);
    let mods = if matches!(code, KeyCode::Char('=' | '+')) {
        mods - KeyModifiers::SHIFT
    } else {
        mods
    };
    (code, mods)
}

/// Convert Shift-modified ASCII punctuation to the character produced by the
/// keyboard layout and remove the now-redundant Shift modifier.
///
/// Each match accepts both CSI-u representations used by terminals: the base
/// key plus Shift (`/` + Shift) and the resulting character plus Shift (`?` +
/// Shift). Space is intentionally absent so Shift+Space remains distinct from
/// Space during keymap lookup.
fn canonical_shifted_ascii(code: KeyCode, mods: KeyModifiers) -> (KeyCode, KeyModifiers) {
    if !mods.contains(KeyModifiers::SHIFT) {
        return (code, mods);
    }
    let KeyCode::Char(character) = code else {
        return (code, mods);
    };
    let shifted = match character {
        '`' | '~' => '~',
        '1' | '!' => '!',
        '2' | '@' => '@',
        '3' | '#' => '#',
        '4' | '$' => '$',
        '5' | '%' => '%',
        '6' | '^' => '^',
        '7' | '&' => '&',
        '8' | '*' => '*',
        '9' | '(' => '(',
        '0' | ')' => ')',
        '-' | '_' => '_',
        '=' | '+' => '+',
        '[' | '{' => '{',
        ']' | '}' => '}',
        '\\' | '|' => '|',
        ';' | ':' => ':',
        '\'' | '"' => '"',
        ',' | '<' => '<',
        '.' | '>' => '>',
        '/' | '?' => '?',
        _ => return (code, mods),
    };
    (KeyCode::Char(shifted), mods - KeyModifiers::SHIFT)
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use crossterm::event::KeyModifiers;

    use super::canonical_event_code_and_mods;

    #[test]
    fn shifted_question_mark_matches_the_text_binding() {
        for character in ['/', '?'] {
            assert_eq!(
                canonical_event_code_and_mods(KeyCode::Char(character), KeyModifiers::SHIFT),
                (KeyCode::Char('?'), KeyModifiers::NONE)
            );
        }
    }

    #[test]
    fn shifted_space_keeps_its_modifier() {
        assert_eq!(
            canonical_event_code_and_mods(KeyCode::Char(' '), KeyModifiers::SHIFT),
            (KeyCode::Char(' '), KeyModifiers::SHIFT)
        );
    }
}
