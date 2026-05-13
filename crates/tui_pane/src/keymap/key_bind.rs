//! `KeyBind`, `KeyInput`, `KeyParseError`: the framework's key abstraction
//! plus a tagged event enum for press / release / repeat semantics.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use thiserror::Error;

/// A single keystroke: a [`KeyCode`] plus its [`KeyModifiers`] flags.
///
/// `KeyBind` is the dispatch-time type — what the keymap stores and looks up.
/// It carries no press/release information; that lives in [`KeyInput`].
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

/// Kind-tagged keyboard event.
///
/// The framework's event-loop entry converts each crossterm `KeyEvent`
/// into a `KeyInput`; downstream dispatch pattern-matches on the variant.
/// The keymap dispatcher only handles [`KeyInput::Press`]; future handlers
/// may opt in to `Release` / `Repeat` (vim-style modal sequences, key-up
/// cancellation, etc.).
///
/// `state` (`CapsLock` / `NumLock` flags) is not preserved — the framework
/// does not bind on those.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum KeyInput {
    /// Key down. The only variant the keymap dispatcher acts on.
    Press(KeyBind),
    /// Key up.
    Release(KeyBind),
    /// Auto-repeat (held key, OS-driven).
    Repeat(KeyBind),
}

impl KeyInput {
    /// Convert a crossterm `KeyEvent` to a [`KeyInput`].
    ///
    /// Canonicalizes the key/modifier pair the same way dispatch does through
    /// [`KeyBind::from_key_event`], preserves the event kind, and drops
    /// `state`.
    #[must_use]
    pub fn from_event(event: KeyEvent) -> Self {
        let bind = KeyBind::from_key_event(event);
        match event.kind {
            KeyEventKind::Press => Self::Press(bind),
            KeyEventKind::Release => Self::Release(bind),
            KeyEventKind::Repeat => Self::Repeat(bind),
        }
    }

    /// The underlying [`KeyBind`], regardless of kind.
    #[must_use]
    pub const fn bind(&self) -> &KeyBind {
        match self {
            Self::Press(b) | Self::Release(b) | Self::Repeat(b) => b,
        }
    }

    /// `Some(bind)` only when this is a [`KeyInput::Press`]. Idiomatic
    /// at keymap dispatch sites: `let Some(bind) = input.press() else { return; };`.
    #[must_use]
    pub const fn press(&self) -> Option<&KeyBind> {
        match self {
            Self::Press(b) => Some(b),
            Self::Release(_) | Self::Repeat(_) => None,
        }
    }
}

impl KeyBind {
    /// Canonicalize a crossterm key event into the keymap dispatch form.
    ///
    /// Crossterm can report `BackTab` separately from `Tab + Shift`, and shifted
    /// ASCII letters as both an uppercase character and a `SHIFT` modifier. The
    /// keymap stores those as `Tab + Shift` and uppercase `Char` without
    /// `SHIFT`, respectively. `+` and `=` are kept distinct (no collapse).
    #[must_use]
    pub fn from_key_event(event: KeyEvent) -> Self {
        let (code, mods) = match event.code {
            KeyCode::BackTab => (KeyCode::Tab, event.modifiers | KeyModifiers::SHIFT),
            KeyCode::Char(c)
                if c.is_ascii_lowercase() && event.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                (
                    KeyCode::Char(c.to_ascii_uppercase()),
                    event.modifiers - KeyModifiers::SHIFT,
                )
            },
            KeyCode::Char(c) if c.is_ascii_uppercase() => {
                (event.code, event.modifiers - KeyModifiers::SHIFT)
            },
            _ => (event.code, event.modifiers),
        };
        Self { code, mods }
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

    /// Full display name, e.g. `"Up"`, `"Enter"`, `"Esc"`, `"Ctrl+K"`,
    /// `"Shift+Tab"`. Used by the keymap-overlay help screen.
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
            _ => return with_short_modifier_prefix(self.mods, &key_name(self.code)),
        };
        with_short_modifier_prefix(self.mods, key)
    }

    /// Parse a TOML-style key string (e.g. `"Enter"`, `"Ctrl+K"`,
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
        Ok(Self { code, mods })
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
        "Enter" => KeyCode::Enter,
        "Tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Esc" => KeyCode::Esc,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Insert" => KeyCode::Insert,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Space" => KeyCode::Char(' '),
        _ => {
            if let Some(rest) = s.strip_prefix('F')
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
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Char(' ') => "Space".to_string(),
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
        s.push_str("Ctrl+");
    }
    if mods.contains(KeyModifiers::ALT) {
        s.push_str("Alt+");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        s.push_str("Shift+");
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
    use crossterm::event::KeyEventKind;
    use crossterm::event::KeyEventState;

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
        assert_eq!(kb.code, KeyCode::Char('g'));
        assert_eq!(kb.mods, KeyModifiers::CONTROL | KeyModifiers::SHIFT);

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
    fn key_input_from_event_tags_kind() {
        let make = |kind| KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::SHIFT,
            kind,
            state: KeyEventState::CAPS_LOCK,
        };
        let expected_bind = KeyBind::from('A');

        assert_eq!(
            KeyInput::from_event(make(KeyEventKind::Press)),
            KeyInput::Press(expected_bind)
        );
        assert_eq!(
            KeyInput::from_event(make(KeyEventKind::Release)),
            KeyInput::Release(expected_bind)
        );
        assert_eq!(
            KeyInput::from_event(make(KeyEventKind::Repeat)),
            KeyInput::Repeat(expected_bind)
        );
    }

    #[test]
    fn key_input_press_returns_bind_only_for_press() {
        let bind = KeyBind::from('a');
        assert_eq!(KeyInput::Press(bind).press(), Some(&bind));
        assert_eq!(KeyInput::Release(bind).press(), None);
        assert_eq!(KeyInput::Repeat(bind).press(), None);
    }

    #[test]
    fn key_input_bind_returns_underlying_for_all_kinds() {
        let bind = KeyBind::ctrl('k');
        assert_eq!(KeyInput::Press(bind).bind(), &bind);
        assert_eq!(KeyInput::Release(bind).bind(), &bind);
        assert_eq!(KeyInput::Repeat(bind).bind(), &bind);
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
            (KeyBind::from(KeyCode::Enter), "Enter"),
            (KeyBind::from(KeyCode::Up), "Up"),
            (KeyBind::ctrl('k'), "Ctrl+k"),
            (KeyBind::shift(KeyCode::Tab), "Shift+Tab"),
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
}
