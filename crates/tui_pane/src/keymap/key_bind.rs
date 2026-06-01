//! `KeyBind` + `KeyParseError`: the framework's key abstraction.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use thiserror::Error;

/// A single keystroke: a [`KeyCode`] plus its [`KeyModifiers`] flags.
///
/// `KeyBind` is the dispatch-time type — what the keymap stores and looks up.
///
/// Construct via `From<KeyCode>` / `From<char>` for the modifier-free case,
/// [`KeyBind::shift`] / [`KeyBind::ctrl`] when modifiers matter, or
/// [`KeyBind::from_key_event`] to canonicalize a crossterm event for dispatch.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct KeyBind {
    /// The key being pressed (letter, named key, function key, etc.).
    pub code: KeyCode,
    /// Modifier flags pressed alongside `code`.
    pub mods: KeyModifiers,
}

impl From<KeyCode> for KeyBind {
    fn from(code: KeyCode) -> Self {
        Self {
            code,
            mods: KeyModifiers::NONE,
        }
    }
}

impl From<char> for KeyBind {
    fn from(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            mods: KeyModifiers::NONE,
        }
    }
}

impl KeyBind {
    fn normalized(code: KeyCode, mods: KeyModifiers) -> Self {
        let (code, mods) = match code {
            KeyCode::BackTab => (KeyCode::Tab, mods | KeyModifiers::SHIFT),
            KeyCode::Char(c) if c.is_ascii_lowercase() && mods.contains(KeyModifiers::SHIFT) => (
                KeyCode::Char(c.to_ascii_uppercase()),
                mods - KeyModifiers::SHIFT,
            ),
            KeyCode::Char(c) if c.is_ascii_uppercase() => {
                (KeyCode::Char(c), mods - KeyModifiers::SHIFT)
            },
            _ => (code, mods),
        };
        Self { code, mods }
    }

    /// Canonicalize a crossterm key event into the keymap dispatch form.
    ///
    /// Crossterm can report `BackTab` separately from `Tab + Shift`, and shifted
    /// ASCII letters as both an uppercase character and a `SHIFT` modifier. The
    /// keymap stores those as `Tab + Shift` and uppercase `Char` without
    /// `SHIFT`, respectively. `+` and `=` are kept distinct (no collapse) —
    /// apps that want them merged use [`Self::canonicalize_code`].
    #[must_use]
    pub fn from_key_event(event: KeyEvent) -> Self { Self::normalized(event.code, event.modifiers) }

    /// Build a bind from an arbitrary `(code, mods)` pair, applying the
    /// same normalization rules as [`Self::from_key_event`].
    ///
    /// Use this when the app already has the `KeyCode` + `KeyModifiers`
    /// in hand (e.g. defaults table construction) and wants the
    /// canonical dispatch form.
    #[must_use]
    pub fn from_parts(code: KeyCode, mods: KeyModifiers) -> Self { Self::normalized(code, mods) }

    /// Apply a canonicalizer to this bind's [`KeyCode`], leaving
    /// modifiers untouched.
    ///
    /// The framework treats `+` and `=` (and any other key pair) as
    /// distinct by default. An app that wants two keys to dispatch
    /// through the same binding installs a canonicalizer that maps
    /// one to the other, and calls this both after [`Self::parse`]
    /// (when loading a keymap from TOML) and after
    /// [`Self::from_key_event`] (when dispatching crossterm input) so
    /// the storage and lookup paths agree.
    #[must_use]
    pub fn canonicalize_code(self, canonicalize: fn(KeyCode) -> KeyCode) -> Self {
        Self {
            code: canonicalize(self.code),
            mods: self.mods,
        }
    }

    /// `const`-friendly constructor for `Char(c)` with no modifiers.
    /// `From<char>` is not `const`, so `static` arrays of
    /// `(Action, KeyBind)` pairs (e.g. vim-extras tables) reach for
    /// this instead of the trait impl.
    #[must_use]
    pub const fn from_char(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            mods: KeyModifiers::NONE,
        }
    }

    /// Build a Shift-modified bind. `KeyBind::shift('g')` is `Char('g') + SHIFT`.
    /// Accepts anything convertible to `KeyBind` ([`KeyCode`] or `char`);
    /// composes with [`Self::ctrl`] (`KeyBind::ctrl(KeyBind::shift('g'))` →
    /// `Char('g') + CONTROL | SHIFT`).
    #[must_use]
    pub fn shift(into: impl Into<Self>) -> Self {
        let kb = into.into();
        Self {
            code: kb.code,
            mods: kb.mods | KeyModifiers::SHIFT,
        }
    }

    /// Build a Control-modified bind. `KeyBind::ctrl('k')` is `Char('k') + CONTROL`.
    /// Accepts anything convertible to `KeyBind` ([`KeyCode`] or `char`);
    /// composes with [`Self::shift`].
    #[must_use]
    pub fn ctrl(into: impl Into<Self>) -> Self {
        let kb = into.into();
        Self {
            code: kb.code,
            mods: kb.mods | KeyModifiers::CONTROL,
        }
    }

    /// Full display name, e.g. `"up"`, `"enter"`, `"escape"`, `"ctrl-k"`,
    /// `"shift-tab"`. Used by the keymap-overlay help screen and TOML output.
    #[must_use]
    pub fn display(&self) -> String { with_modifier_prefix(self.mods, &key_name(self.code)) }

    /// Compact display: arrow keys render as glyphs (`↑`, `↓`, `←`, `→`),
    /// every other key delegates to [`Self::display`]. Used by the status bar.
    ///
    /// Must not produce a string containing `,` or `/` for any key the app
    /// uses in a `BarRow::Paired` slot — the `bar/` `Paired` row will
    /// `debug_assert!` this once it lands.
    #[must_use]
    pub fn display_short(&self) -> String {
        let key = match self.code {
            KeyCode::Up => "↑",
            KeyCode::Down => "↓",
            KeyCode::Left => "←",
            KeyCode::Right => "→",
            // Compact "Esc" in the bar; the TOML form stays "escape"
            // (via `display`), which the parser also round-trips.
            KeyCode::Esc => "Esc",
            _ => return with_short_modifier_prefix(self.mods, &key_name(self.code)),
        };
        with_short_modifier_prefix(self.mods, key)
    }

    /// Parse a TOML-style key string (e.g. `"enter"`, `"ctrl-k"`,
    /// `"Shift+Tab"`, `"+"`, `"="`). `"+"` parses to
    /// `KeyCode::Char('+')` and `"="` to `KeyCode::Char('=')` — the two
    /// are kept distinct (no collapse).
    ///
    /// # Errors
    ///
    /// Returns [`KeyParseError`] if the string is empty, names an unknown
    /// modifier, or names an unknown key.
    pub fn parse(s: &str) -> Result<Self, KeyParseError> {
        if s.is_empty() {
            return Err(KeyParseError::Empty);
        }
        if s == "+" || s == "=" {
            let c = s.chars().next().ok_or(KeyParseError::Empty)?;
            return Ok(Self::from(c));
        }

        if let Ok(bind) = parse_zed_keybind(s) {
            return Ok(bind);
        }

        let mut mods = KeyModifiers::NONE;
        let mut rest = s;
        while let Some((head, tail)) = rest.split_once('+') {
            let Some(m) = parse_modifier(head) else {
                return Err(KeyParseError::UnknownModifier(head.to_string()));
            };
            mods |= m;
            rest = tail;
            if rest == "+" || rest == "=" {
                let c = rest.chars().next().ok_or(KeyParseError::Empty)?;
                return Ok(Self {
                    code: KeyCode::Char(c),
                    mods,
                });
            }
        }

        let code = parse_keycode(rest)?;
        Ok(Self::normalized(code, mods))
    }
}

fn parse_zed_keybind(s: &str) -> Result<KeyBind, KeyParseError> {
    if s == "-" || s == "+" || s == "=" {
        return Err(KeyParseError::UnknownKey(s.to_string()));
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() <= 1 {
        return Err(KeyParseError::UnknownKey(s.to_string()));
    }
    let Some((key_part, modifier_parts)) = parts.split_last() else {
        return Err(KeyParseError::Empty);
    };
    if key_part.is_empty() {
        return Err(KeyParseError::Empty);
    }
    let mut mods = KeyModifiers::NONE;
    for modifier in modifier_parts {
        let Some(m) = parse_zed_modifier(modifier) else {
            return Err(KeyParseError::UnknownModifier((*modifier).to_string()));
        };
        mods |= m;
    }
    let code = parse_keycode(key_part)?;
    Ok(KeyBind::normalized(code, mods))
}

fn parse_zed_modifier(s: &str) -> Option<KeyModifiers> {
    match s {
        "ctrl" | "control" => Some(KeyModifiers::CONTROL),
        "shift" => Some(KeyModifiers::SHIFT),
        "alt" | "option" => Some(KeyModifiers::ALT),
        _ => None,
    }
}

fn parse_modifier(s: &str) -> Option<KeyModifiers> {
    match s {
        "Ctrl" | "Control" => Some(KeyModifiers::CONTROL),
        "Shift" => Some(KeyModifiers::SHIFT),
        "Alt" => Some(KeyModifiers::ALT),
        _ => None,
    }
}

fn parse_keycode(s: &str) -> Result<KeyCode, KeyParseError> {
    if s.is_empty() {
        return Err(KeyParseError::Empty);
    }
    let code = match s {
        "Enter" | "enter" | "return" => KeyCode::Enter,
        "Tab" | "tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Esc" | "escape" => KeyCode::Esc,
        "Backspace" | "backspace" => KeyCode::Backspace,
        "Delete" | "delete" => KeyCode::Delete,
        "Insert" | "insert" => KeyCode::Insert,
        "Home" | "home" => KeyCode::Home,
        "End" | "end" => KeyCode::End,
        "PageUp" | "pageup" => KeyCode::PageUp,
        "PageDown" | "pagedown" => KeyCode::PageDown,
        "Up" | "up" => KeyCode::Up,
        "Down" | "down" => KeyCode::Down,
        "Left" | "left" => KeyCode::Left,
        "Right" | "right" => KeyCode::Right,
        "Space" | "space" => KeyCode::Char(' '),
        _ => {
            if let Some(rest) = s.strip_prefix('F').or_else(|| s.strip_prefix('f'))
                && let Ok(n) = rest.parse::<u8>()
            {
                if (1..=12).contains(&n) {
                    return Ok(KeyCode::F(n));
                }
                return Err(KeyParseError::UnknownKey(s.to_string()));
            }
            let mut chars = s.chars();
            let c = chars.next().ok_or(KeyParseError::Empty)?;
            if chars.next().is_some() {
                return Err(KeyParseError::UnknownKey(s.to_string()));
            }
            KeyCode::Char(c)
        },
    };
    Ok(code)
}

fn key_name(code: KeyCode) -> String {
    match code {
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::BackTab => "shift-tab".to_string(),
        KeyCode::Esc => "escape".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Insert => "insert".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::F(n) => format!("f{n}"),
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        other => format!("{other:?}"),
    }
}

fn with_modifier_prefix(mods: KeyModifiers, key: &str) -> String {
    if mods.is_empty() {
        return key.to_string();
    }
    let mut s = String::new();
    if mods.contains(KeyModifiers::CONTROL) {
        s.push_str("ctrl-");
    }
    if mods.contains(KeyModifiers::ALT) {
        s.push_str("alt-");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        s.push_str("shift-");
    }
    s.push_str(key);
    s
}

fn with_short_modifier_prefix(mods: KeyModifiers, key: &str) -> String {
    if mods.is_empty() {
        return key.to_string();
    }
    let mut s = String::new();
    if mods.contains(KeyModifiers::CONTROL) {
        s.push('⌃');
    }
    if mods.contains(KeyModifiers::ALT) {
        s.push('⌥');
    }
    if mods.contains(KeyModifiers::SHIFT) {
        s.push('⇧');
    }
    s.push_str(key);
    s
}

/// Error returned by [`KeyBind::parse`].
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum KeyParseError {
    /// Input was the empty string.
    #[error("empty key string")]
    Empty,
    /// Input contained a key token that was neither a recognized name
    /// nor a single character.
    #[error("unknown key: {0:?}")]
    UnknownKey(String),
    /// Input contained a modifier token that was not `Ctrl` / `Control` /
    /// `Shift` / `Alt`.
    #[error("unknown modifier: {0:?}")]
    UnknownModifier(String),
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crossterm::event::KeyEvent;

    use super::*;

    #[test]
    fn parse_named_keys() {
        assert_eq!(
            KeyBind::parse("Enter").unwrap(),
            KeyBind::from(KeyCode::Enter)
        );
        assert_eq!(KeyBind::parse("Tab").unwrap(), KeyBind::from(KeyCode::Tab));
        assert_eq!(KeyBind::parse("Esc").unwrap(), KeyBind::from(KeyCode::Esc));
        assert_eq!(KeyBind::parse("Up").unwrap(), KeyBind::from(KeyCode::Up));
        assert_eq!(
            KeyBind::parse("Down").unwrap(),
            KeyBind::from(KeyCode::Down)
        );
        assert_eq!(
            KeyBind::parse("Left").unwrap(),
            KeyBind::from(KeyCode::Left)
        );
        assert_eq!(
            KeyBind::parse("Right").unwrap(),
            KeyBind::from(KeyCode::Right)
        );
        assert_eq!(KeyBind::parse("F1").unwrap(), KeyBind::from(KeyCode::F(1)));
        assert_eq!(
            KeyBind::parse("F12").unwrap(),
            KeyBind::from(KeyCode::F(12))
        );
        assert_eq!(
            KeyBind::parse("Space").unwrap(),
            KeyBind::from(KeyCode::Char(' '))
        );
    }

    #[test]
    fn parse_modifiers() {
        let kb = KeyBind::parse("Ctrl+k").unwrap();
        assert_eq!(kb.code, KeyCode::Char('k'));
        assert_eq!(kb.mods, KeyModifiers::CONTROL);

        let kb = KeyBind::parse("Shift+Tab").unwrap();
        assert_eq!(kb.code, KeyCode::Tab);
        assert_eq!(kb.mods, KeyModifiers::SHIFT);

        let kb = KeyBind::parse("Ctrl+Shift+g").unwrap();
        assert_eq!(kb.code, KeyCode::Char('G'));
        assert_eq!(kb.mods, KeyModifiers::CONTROL);

        let kb = KeyBind::parse("Control+K").unwrap();
        assert_eq!(kb.code, KeyCode::Char('K'));
        assert_eq!(kb.mods, KeyModifiers::CONTROL);
    }

    #[test]
    fn from_key_event_normalizes_crossterm_backtab() {
        let event = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);

        assert_eq!(KeyBind::from_key_event(event), KeyBind::shift(KeyCode::Tab));
    }

    #[test]
    fn from_key_event_normalizes_shifted_ascii_letters() {
        let upper = KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT);
        let lower_with_shift = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::SHIFT);

        assert_eq!(KeyBind::from_key_event(upper), KeyBind::from('R'));
        assert_eq!(
            KeyBind::from_key_event(lower_with_shift),
            KeyBind::from('R')
        );
    }

    #[test]
    fn from_key_event_does_not_collapse_plus_and_equals() {
        let plus = KeyBind::from_key_event(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::SHIFT));
        let equals =
            KeyBind::from_key_event(KeyEvent::new(KeyCode::Char('='), KeyModifiers::SHIFT));

        assert_eq!(plus.code, KeyCode::Char('+'));
        assert_eq!(equals.code, KeyCode::Char('='));
        assert_ne!(plus, equals);
    }

    #[test]
    fn parse_plus_and_equals_distinct() {
        let plus = KeyBind::parse("+").unwrap();
        let eq = KeyBind::parse("=").unwrap();
        assert_eq!(plus.code, KeyCode::Char('+'));
        assert_eq!(eq.code, KeyCode::Char('='));
        assert_ne!(plus, eq);

        let ctrl_plus = KeyBind::parse("Ctrl++").unwrap();
        assert_eq!(ctrl_plus.code, KeyCode::Char('+'));
        assert_eq!(ctrl_plus.mods, KeyModifiers::CONTROL);

        let shift_eq = KeyBind::parse("Shift+=").unwrap();
        assert_eq!(shift_eq.code, KeyCode::Char('='));
        assert_eq!(shift_eq.mods, KeyModifiers::SHIFT);
    }

    #[test]
    fn parse_errors() {
        assert_eq!(KeyBind::parse(""), Err(KeyParseError::Empty));
        assert!(matches!(
            KeyBind::parse("Bogus"),
            Err(KeyParseError::UnknownKey(_))
        ));
        assert!(matches!(
            KeyBind::parse("Hyper+K"),
            Err(KeyParseError::UnknownModifier(_))
        ));
        assert!(matches!(
            KeyBind::parse("Command+C"),
            Err(KeyParseError::UnknownModifier(_))
        ));
        assert!(matches!(
            KeyBind::parse("F99"),
            Err(KeyParseError::UnknownKey(_))
        ));
    }

    #[test]
    fn display_short_no_separators() {
        let names = [
            "Enter",
            "Tab",
            "BackTab",
            "Esc",
            "Backspace",
            "Delete",
            "Insert",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "Up",
            "Down",
            "Left",
            "Right",
            "Space",
        ];
        for name in names {
            let key = KeyBind::parse(name).unwrap();
            check_no_paired_separators(key.code);
        }
        for c in (b' '..=b'~')
            .map(char::from)
            .filter(|c| *c != ',' && *c != '/')
        {
            check_no_paired_separators(KeyCode::Char(c));
        }
        for n in 1u8..=12 {
            check_no_paired_separators(KeyCode::F(n));
        }
    }

    fn check_no_paired_separators(code: KeyCode) {
        for mods in all_modifier_combos() {
            let bind = KeyBind { code, mods };
            let s = bind.display_short();
            assert!(!s.contains(','), "display_short of {bind:?} contained ','");
            assert!(!s.contains('/'), "display_short of {bind:?} contained '/'");
        }
    }

    fn all_modifier_combos() -> Vec<KeyModifiers> {
        let mut out = Vec::with_capacity(8);
        for ctrl in [false, true] {
            for shift in [false, true] {
                for alt in [false, true] {
                    let mut m = KeyModifiers::NONE;
                    if ctrl {
                        m |= KeyModifiers::CONTROL;
                    }
                    if shift {
                        m |= KeyModifiers::SHIFT;
                    }
                    if alt {
                        m |= KeyModifiers::ALT;
                    }
                    out.push(m);
                }
            }
        }
        out
    }

    #[test]
    fn from_keycode_and_char() {
        assert_eq!(KeyBind::from(KeyCode::Enter).mods, KeyModifiers::NONE);
        assert_eq!(KeyBind::from(KeyCode::Enter).code, KeyCode::Enter);
        assert_eq!(KeyBind::from('c').code, KeyCode::Char('c'));
        assert_eq!(KeyBind::from('c').mods, KeyModifiers::NONE);
    }

    #[test]
    fn shift_and_ctrl_constructors() {
        assert_eq!(
            KeyBind::shift('g'),
            KeyBind {
                code: KeyCode::Char('g'),
                mods: KeyModifiers::SHIFT,
            }
        );
        assert_eq!(
            KeyBind::ctrl('k'),
            KeyBind {
                code: KeyCode::Char('k'),
                mods: KeyModifiers::CONTROL,
            }
        );
        assert_eq!(
            KeyBind::shift(KeyCode::Tab),
            KeyBind {
                code: KeyCode::Tab,
                mods: KeyModifiers::SHIFT,
            }
        );
    }

    #[test]
    fn display_round_trip() {
        let cases = [
            (KeyBind::from(KeyCode::Enter), "enter"),
            (KeyBind::from(KeyCode::Up), "up"),
            (KeyBind::ctrl('k'), "ctrl-k"),
            (KeyBind::shift(KeyCode::Tab), "shift-tab"),
        ];
        for (bind, expected) in cases {
            assert_eq!(bind.display(), expected);
        }
    }

    #[test]
    fn display_short_arrow_glyphs() {
        assert_eq!(KeyBind::from(KeyCode::Up).display_short(), "↑");
        assert_eq!(KeyBind::from(KeyCode::Down).display_short(), "↓");
        assert_eq!(KeyBind::from(KeyCode::Left).display_short(), "←");
        assert_eq!(KeyBind::from(KeyCode::Right).display_short(), "→");
        assert_eq!(KeyBind::ctrl(KeyCode::Up).display_short(), "⌃↑");
        assert_eq!(KeyBind::ctrl('k').display_short(), "⌃k");
    }

    #[test]
    fn display_short_abbreviates_escape_but_display_keeps_full_form() {
        // The bar shows the compact "Esc"; the keymap TOML keeps the
        // full "escape" form, which the parser also round-trips.
        assert_eq!(KeyBind::from(KeyCode::Esc).display_short(), "Esc");
        assert_eq!(KeyBind::from(KeyCode::Esc).display(), "escape");
        assert_eq!(KeyBind::parse("Esc").unwrap(), KeyBind::from(KeyCode::Esc));
    }
}
