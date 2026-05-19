use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;

use super::parse;

/// A bindable key: a `KeyCode` plus modifier flags from crossterm.
///
/// `=` and `+` are normalised to a single canonical form (`+`) so they
/// are treated as the same physical key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct KeyBind {
    pub code:      KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyBind {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        // BackTab implies Shift — normalise to Tab + SHIFT.
        // Uppercase Char implies Shift — strip SHIFT since it's
        // encoded in the character itself (`Char('R')` already means
        // Shift+r).  This ensures the binding `"R"` matches the
        // crossterm event `Char('R') + SHIFT`.
        // Normalise Shift + lowercase letter → uppercase letter with
        // SHIFT stripped, so `Shift+r` and `R` produce the same KeyBind.
        let (code, modifiers) = match code {
            KeyCode::BackTab => (code, modifiers | KeyModifiers::SHIFT),
            KeyCode::Char(c)
                if c.is_ascii_lowercase() && modifiers.contains(KeyModifiers::SHIFT) =>
            {
                (
                    KeyCode::Char(c.to_ascii_uppercase()),
                    modifiers - KeyModifiers::SHIFT,
                )
            },
            KeyCode::Char(c) if c.is_ascii_uppercase() => (code, modifiers - KeyModifiers::SHIFT),
            _ => (code, modifiers),
        };
        Self {
            code: parse::normalize_code(code),
            modifiers,
        }
    }

    pub(super) fn plain(code: KeyCode) -> Self { Self::new(code, KeyModifiers::NONE) }

    /// Human-readable glyph string for display in status bar / keymap UI.
    pub(super) fn display(&self) -> String { self.to_toml_string() }

    /// TOML-serialisable string (e.g. `"ctrl-r"`, `"shift-tab"`, `"q"`).
    pub(super) fn to_toml_string(&self) -> String {
        let mut parts = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push_str("ctrl-");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push_str("alt-");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push_str("shift-");
        }
        parts.push_str(&parse::code_label(self.code).to_ascii_lowercase());
        parts
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_char() {
        let kb: KeyBind = "q".parse().unwrap();
        assert_eq!(kb.code, KeyCode::Char('q'));
        assert_eq!(kb.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_named_keys() {
        assert_eq!("Enter".parse::<KeyBind>().unwrap().code, KeyCode::Enter);
        assert_eq!("Esc".parse::<KeyBind>().unwrap().code, KeyCode::Esc);
        assert_eq!("Tab".parse::<KeyBind>().unwrap().code, KeyCode::Tab);
        assert_eq!("Space".parse::<KeyBind>().unwrap().code, KeyCode::Char(' '));
        assert_eq!("F1".parse::<KeyBind>().unwrap().code, KeyCode::F(1));
        assert_eq!("F12".parse::<KeyBind>().unwrap().code, KeyCode::F(12));
    }

    #[test]
    fn parse_ctrl_modifier() {
        let kb: KeyBind = "Ctrl+r".parse().unwrap();
        assert_eq!(kb.code, KeyCode::Char('r'));
        assert!(kb.modifiers.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn parse_shift_modifier() {
        let kb: KeyBind = "Shift+Tab".parse().unwrap();
        assert_eq!(kb.code, KeyCode::Tab);
        assert!(kb.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn parse_alt_modifier() {
        let kb: KeyBind = "Alt+d".parse().unwrap();
        assert_eq!(kb.code, KeyCode::Char('d'));
        assert!(kb.modifiers.contains(KeyModifiers::ALT));
    }

    #[test]
    fn parse_multiple_modifiers() {
        // Shift+x normalizes to Char('X') with SHIFT stripped.
        let kb: KeyBind = "Ctrl+Shift+x".parse().unwrap();
        assert_eq!(kb.code, KeyCode::Char('X'));
        assert!(kb.modifiers.contains(KeyModifiers::CONTROL));
        assert!(!kb.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn serde_round_trip() {
        let cases = [
            "q",
            "Ctrl+r",
            "Alt+d",
            "Shift+Tab",
            "Enter",
            "Esc",
            "/",
            "-",
        ];
        for input in cases {
            let kb: KeyBind = input.parse().unwrap();
            let serialized = kb.to_toml_string();
            let reparsed: KeyBind = serialized.parse().unwrap();
            assert_eq!(kb, reparsed, "round-trip failed for \"{input}\"");
        }
    }

    #[test]
    fn equals_plus_normalization() {
        let plus: KeyBind = "+".parse().unwrap();
        let equals: KeyBind = "=".parse().unwrap();
        assert_eq!(plus, equals);
    }

    #[test]
    fn uppercase_char_strips_shift() {
        // Crossterm delivers Shift+R as Char('R') + SHIFT.
        // Our normalization strips SHIFT since uppercase encodes it.
        let from_event = KeyBind::new(KeyCode::Char('R'), KeyModifiers::SHIFT);
        let from_toml = KeyBind::plain(KeyCode::Char('R'));
        assert_eq!(from_event, from_toml);
        assert_eq!(from_event.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn shift_plus_lowercase_becomes_uppercase() {
        // TOML "Shift+r" should match bare "R".
        let shift_r: KeyBind = "Shift+r".parse().unwrap();
        let bare_r: KeyBind = "R".parse().unwrap();
        assert_eq!(shift_r, bare_r);
        assert_eq!(shift_r.code, KeyCode::Char('R'));
        assert_eq!(shift_r.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn ctrl_shift_letter_keeps_ctrl() {
        // Ctrl+Shift+r → Char('R') + CONTROL (SHIFT stripped).
        let kb = KeyBind::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(kb.code, KeyCode::Char('R'));
        assert!(kb.modifiers.contains(KeyModifiers::CONTROL));
        assert!(!kb.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn lowercase_without_shift_unchanged() {
        let kb = KeyBind::plain(KeyCode::Char('r'));
        assert_eq!(kb.code, KeyCode::Char('r'));
        assert_eq!(kb.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn restart_default_matches_crossterm_event() {
        // Shifted-letter normalization remains on the legacy KeyBind bridge.
        let crossterm_event = KeyBind::new(KeyCode::Char('R'), KeyModifiers::SHIFT);
        assert_eq!(KeyBind::plain(KeyCode::Char('R')), crossterm_event);
    }

    #[test]
    fn display_uses_zed_style() {
        assert_eq!(
            KeyBind::new(KeyCode::Char('r'), KeyModifiers::CONTROL).display(),
            "ctrl-r"
        );
        assert_eq!(
            KeyBind::new(KeyCode::Char('d'), KeyModifiers::ALT).display(),
            "alt-d"
        );
        assert_eq!(
            KeyBind::new(KeyCode::Tab, KeyModifiers::SHIFT).display(),
            "shift-tab"
        );
        assert_eq!(KeyBind::plain(KeyCode::Char('q')).display(), "q");
    }

    #[test]
    fn plus_displays_as_plus() {
        let kb = KeyBind::plain(KeyCode::Char('='));
        assert_eq!(kb.display(), "+");
        assert_eq!(kb.to_toml_string(), "+");
    }

    #[test]
    fn parse_errors() {
        assert!("".parse::<KeyBind>().is_err(), "empty string");
        assert!("Ctrl+".parse::<KeyBind>().is_err(), "modifier with no key");
        assert!("Ctrl+Ctrl".parse::<KeyBind>().is_err(), "modifier as key");
    }

    #[test]
    fn valid_edge_cases() {
        assert!("+".parse::<KeyBind>().is_ok(), "plus key");
        assert!("/".parse::<KeyBind>().is_ok(), "slash key");
        assert!("Space".parse::<KeyBind>().is_ok(), "space key");
    }
}
