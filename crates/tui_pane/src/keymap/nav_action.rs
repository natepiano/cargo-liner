//! `NavAction`: the framework-owned navigation action set.
//!
//! The framework owns the navigation vocabulary, its default keymap,
//! and the vim letter aliases so that every embedding app inherits the
//! same guarantees: every action has a compiled default key (an unbound
//! navigation action is unrepresentable), and page / half-page motions
//! are distinct variants that cannot collapse onto a single-line move.
//!
//! [`default_keys`] is an exhaustive `match` — adding a variant fails to
//! compile until it is given a key. [`default_bindings`] folds those
//! into a [`Bindings`] table, and [`vim_letter_extras`] supplies the
//! `h`/`j`/`k`/`l`/`gg`/`G` aliases the builder layers on in
//! [`VimMode::Enabled`](crate::VimMode).

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;

use super::Action;
use super::Bindings;
use super::key_bind::KeyBind;
use super::key_sequence::KeySequence;

crate::action_enum! {
    /// The framework-owned navigation action set.
    ///
    /// Closed enum: the framework owns the directional vocabulary, its
    /// default keymap ([`default_keys`]), and the vim aliases
    /// ([`vim_letter_extras`]). Apps route resolved actions through
    /// their [`Navigation`](crate::Navigation) impl's dispatcher; they
    /// neither define the set nor supply its default keys.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum NavAction {
        /// Move one step toward the start.
        Up           => ("up",            "up",        "Move up");
        /// Move one step toward the end.
        Down         => ("down",          "down",      "Move down");
        /// Move left (collapse / previous column).
        Left         => ("left",          "left",      "Move left");
        /// Move right (expand / next column).
        Right        => ("right",         "right",     "Move right");
        /// Jump to the first entry.
        Home         => ("home",          "home",      "Jump to start");
        /// Jump to the last entry.
        End          => ("end",           "end",       "Jump to end");
        /// Move up by one page.
        PageUp       => ("page_up",       "page up",   "Page up");
        /// Move down by one page.
        PageDown     => ("page_down",     "page down", "Page down");
        /// Move up by one half-page.
        HalfPageUp   => ("half_page_up",  "half up",   "Half-page up");
        /// Move down by one half-page.
        HalfPageDown => ("half_page_down", "half down", "Half-page down");
    }
}

/// Compiled default key(s) for one navigation action. The first key is
/// the action's primary binding (what the bar shows); later keys are
/// aliases. Every arm returns a non-empty slice — the exhaustive `match`
/// (no wildcard) forces a key when a variant is added, so an unbound
/// navigation action cannot compile.
///
/// All defaults are single keystrokes, so `&[KeyBind]` suffices; the
/// multi-key vim chord (`gg`) lives in [`vim_letter_extras`] as a
/// [`KeySequence`].
pub(super) const fn default_keys(action: NavAction) -> &'static [KeyBind] {
    match action {
        NavAction::Up => &[KeyBind {
            code: KeyCode::Up,
            mods: KeyModifiers::NONE,
        }],
        NavAction::Down => &[KeyBind {
            code: KeyCode::Down,
            mods: KeyModifiers::NONE,
        }],
        NavAction::Left => &[KeyBind {
            code: KeyCode::Left,
            mods: KeyModifiers::NONE,
        }],
        NavAction::Right => &[KeyBind {
            code: KeyCode::Right,
            mods: KeyModifiers::NONE,
        }],
        NavAction::Home => &[KeyBind {
            code: KeyCode::Home,
            mods: KeyModifiers::NONE,
        }],
        NavAction::End => &[KeyBind {
            code: KeyCode::End,
            mods: KeyModifiers::NONE,
        }],
        NavAction::PageUp => &[
            KeyBind {
                code: KeyCode::PageUp,
                mods: KeyModifiers::NONE,
            },
            KeyBind {
                code: KeyCode::Char('b'),
                mods: KeyModifiers::CONTROL,
            },
        ],
        NavAction::PageDown => &[
            KeyBind {
                code: KeyCode::PageDown,
                mods: KeyModifiers::NONE,
            },
            KeyBind {
                code: KeyCode::Char('f'),
                mods: KeyModifiers::CONTROL,
            },
        ],
        NavAction::HalfPageUp => &[KeyBind {
            code: KeyCode::Char('u'),
            mods: KeyModifiers::CONTROL,
        }],
        NavAction::HalfPageDown => &[KeyBind {
            code: KeyCode::Char('d'),
            mods: KeyModifiers::CONTROL,
        }],
    }
}

/// The full default navigation keymap: every [`NavAction::ALL`] entry
/// bound to its [`default_keys`], primary key first.
pub(super) fn default_bindings() -> Bindings<NavAction> {
    let mut table = Bindings::new();
    for action in NavAction::ALL.iter().copied() {
        for key in default_keys(action) {
            table.bind(*key, action);
        }
    }
    table
}

/// Vim letter aliases layered on in [`VimMode::Enabled`](crate::VimMode):
/// `h`/`j`/`k`/`l` for the arrows, the `gg` chord for Home, and `G` for
/// End. The Ctrl-based page / half-page aliases are unconditional
/// defaults (see [`default_keys`]), not vim extras, so half-page motions
/// work with vim off too.
pub(super) fn vim_letter_extras() -> [(KeySequence, NavAction); 6] {
    [
        (KeySequence::from('h'), NavAction::Left),
        (KeySequence::from('j'), NavAction::Down),
        (KeySequence::from('k'), NavAction::Up),
        (KeySequence::from('l'), NavAction::Right),
        (
            KeySequence::new(vec![KeyBind::from('g'), KeyBind::from('g')]),
            NavAction::Home,
        ),
        (KeySequence::from('G'), NavAction::End),
    ]
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    fn ctrl(c: char) -> KeyBind { KeyBind::ctrl(c) }

    #[test]
    fn every_action_has_a_default_key() {
        for action in NavAction::ALL.iter().copied() {
            assert!(
                !default_keys(action).is_empty(),
                "{action:?} has no compiled default key",
            );
        }
    }

    #[test]
    fn default_bindings_bind_half_page_to_ctrl_u_and_d() {
        let map = default_bindings().into_scope_map();
        assert_eq!(map.action_for(&ctrl('u')), Some(NavAction::HalfPageUp));
        assert_eq!(map.action_for(&ctrl('d')), Some(NavAction::HalfPageDown));
    }

    #[test]
    fn default_bindings_keep_arrow_keys_primary() {
        let map = default_bindings().into_scope_map();
        assert_eq!(
            map.key_for(NavAction::PageUp)
                .and_then(KeySequence::single_key),
            Some(KeyBind::from(KeyCode::PageUp)),
        );
        assert_eq!(map.action_for(&ctrl('b')), Some(NavAction::PageUp));
        assert_eq!(map.action_for(&ctrl('f')), Some(NavAction::PageDown));
    }
}
