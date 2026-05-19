//! `KeySequence`: one bindable key chord or a single-key binding.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use crossterm::event::KeyCode;

use super::key_bind::KeyBind;
use super::key_bind::KeyParseError;

/// One binding alternative. Most bindings contain a single key; vim-style
/// chords such as `g g` contain multiple steps.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeySequence {
    keys: Vec<KeyBind>,
}

impl KeySequence {
    /// Build a sequence from explicit steps.
    ///
    /// Empty input is allowed only for internal construction paths that
    /// immediately replace it; parsing rejects empty strings.
    #[must_use]
    pub const fn new(keys: Vec<KeyBind>) -> Self { Self { keys } }

    /// Borrow the sequence steps.
    #[must_use]
    pub fn keys(&self) -> &[KeyBind] { &self.keys }

    /// True when this sequence contains exactly one key.
    #[must_use]
    pub const fn is_single(&self) -> bool { self.keys.len() == 1 }

    /// Return the single key when this is a single-key sequence.
    #[must_use]
    pub fn single_key(&self) -> Option<KeyBind> { (self.keys.len() == 1).then(|| self.keys[0]) }

    /// Whether `prefix` is a strict prefix of this sequence.
    #[must_use]
    pub fn starts_with_strict(&self, prefix: &[KeyBind]) -> bool {
        prefix.len() < self.keys.len() && self.keys.starts_with(prefix)
    }

    /// Full display string, with chord steps separated by spaces.
    #[must_use]
    pub fn display(&self) -> String {
        self.keys
            .iter()
            .map(KeyBind::display)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Compact display string, with chord steps separated by spaces.
    #[must_use]
    pub fn display_short(&self) -> String {
        self.keys
            .iter()
            .map(KeyBind::display_short)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Parse a key sequence. Whitespace separates chord steps; each step uses
    /// the single-key parser.
    ///
    /// # Errors
    ///
    /// Returns [`KeyParseError`] when the string is empty or any step is not a
    /// valid key binding.
    pub fn parse(s: &str) -> Result<Self, KeyParseError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(KeyParseError::Empty);
        }
        let keys = s
            .split_whitespace()
            .map(KeyBind::parse)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { keys })
    }
}

impl From<KeyBind> for KeySequence {
    fn from(key: KeyBind) -> Self { Self { keys: vec![key] } }
}

impl From<KeyCode> for KeySequence {
    fn from(code: KeyCode) -> Self { KeyBind::from(code).into() }
}

impl From<char> for KeySequence {
    fn from(c: char) -> Self { KeyBind::from(c).into() }
}

impl PartialEq<KeyBind> for KeySequence {
    fn eq(&self, other: &KeyBind) -> bool { self.single_key() == Some(*other) }
}

impl PartialEq<KeySequence> for KeyBind {
    fn eq(&self, other: &KeySequence) -> bool { other == self }
}

impl Display for KeySequence {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result { f.write_str(&self.display()) }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crossterm::event::KeyCode;

    use super::*;

    #[test]
    fn parses_single_key() {
        assert_eq!(
            KeySequence::parse("Enter").unwrap().keys(),
            &[KeyBind::from(KeyCode::Enter)]
        );
    }

    #[test]
    fn parses_chord_steps() {
        assert_eq!(
            KeySequence::parse("g g").unwrap().keys(),
            &[KeyBind::from('g'), KeyBind::from('g')]
        );
    }

    #[test]
    fn display_round_trips() {
        let seq = KeySequence::parse("Ctrl+k Ctrl+s").unwrap();
        let reparsed = KeySequence::parse(&seq.display()).unwrap();
        assert_eq!(seq, reparsed);
    }
}
