//! `NavAction`: the framework-owned navigation action set.
//!
//! The framework owns the navigation vocabulary, its default keymap,
//! and the vim aliases so that every embedding app inherits the same
//! guarantee: page / half-page motions are distinct variants that
//! cannot collapse onto a single-line move.
//!
//! [`default_keys`] is the rebindable, vim-independent keymap (arrows,
//! Home/End, PageUp/PageDown) — an exhaustive `match` so adding a
//! variant forces a decision about its default. Half-page has no
//! hardware key, so its arms return an empty slice: half-page is
//! reachable only through vim. [`vim_letter_extras`] supplies every
//! vim-only alias the builder layers on in
//! [`VimMode::Enabled`](crate::VimMode): the `h`/`j`/`k`/`l`/`gg`/`G`
//! letters plus the Ctrl page / half-page motions (`Ctrl-u`/`Ctrl-d`
//! and `Ctrl-b`/`Ctrl-f`). Those Ctrl motions are not keymappable and
//! turn off with vim mode.

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

/// Compiled, rebindable default key(s) for one navigation action. The
/// first key is the action's primary binding (what the bar shows); these
/// are the keys the TOML `[navigation]` overlay can rebind, and the only
/// navigation keys active when vim mode is off.
///
/// The exhaustive `match` (no wildcard) forces a decision when a variant
/// is added. Half-page has no hardware key, so its arms return an empty
/// slice — half-page is reachable only through the vim Ctrl motions in
/// [`vim_letter_extras`], never as a keymappable default.
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
        NavAction::PageUp => &[KeyBind {
            code: KeyCode::PageUp,
            mods: KeyModifiers::NONE,
        }],
        NavAction::PageDown => &[KeyBind {
            code: KeyCode::PageDown,
            mods: KeyModifiers::NONE,
        }],
        NavAction::HalfPageUp | NavAction::HalfPageDown => &[],
    }
}

/// The full default navigation keymap: every [`NavAction::ALL`] entry
/// bound to its [`default_keys`], primary key first. Actions whose
/// [`default_keys`] is empty (half-page) contribute no binding here —
/// they enter the table only via [`vim_letter_extras`].
pub(super) fn default_bindings() -> Bindings<NavAction> {
    let mut table = Bindings::new();
    for action in NavAction::ALL.iter().copied() {
        for key in default_keys(action) {
            table.bind(*key, action);
        }
    }
    table
}

/// Vim aliases layered on in [`VimMode::Enabled`](crate::VimMode):
/// `h`/`j`/`k`/`l` for the arrows, the `gg` chord for Home, `G` for End,
/// and the Ctrl page / half-page motions `Ctrl-b`/`Ctrl-f` (full page)
/// and `Ctrl-u`/`Ctrl-d` (half page). All of these are vim-only — they
/// are not keymappable and disappear when vim mode is off. Half-page has
/// no other binding, so it exists only while vim is enabled.
pub(super) fn vim_letter_extras() -> [(KeySequence, NavAction); 10] {
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
        (KeySequence::from(KeyBind::ctrl('b')), NavAction::PageUp),
        (KeySequence::from(KeyBind::ctrl('f')), NavAction::PageDown),
        (KeySequence::from(KeyBind::ctrl('u')), NavAction::HalfPageUp),
        (
            KeySequence::from(KeyBind::ctrl('d')),
            NavAction::HalfPageDown,
        ),
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

    /// Every non-vim action carries a compiled default; half-page is the
    /// sole exception — it has no hardware key and is vim-only.
    #[test]
    fn only_half_page_has_no_compiled_default() {
        for action in NavAction::ALL.iter().copied() {
            let empty = default_keys(action).is_empty();
            let is_half_page = matches!(action, NavAction::HalfPageUp | NavAction::HalfPageDown);
            assert_eq!(
                empty, is_half_page,
                "{action:?}: empty default ({empty}) must match half-page status ({is_half_page})",
            );
        }
    }

    /// Defaults + vim extras together reach every action — nothing is
    /// unreachable once vim mode is on.
    #[test]
    fn defaults_and_vim_extras_cover_every_action() {
        let mut table = default_bindings();
        for (key, action) in vim_letter_extras() {
            table.bind(key, action);
        }
        let map = table.into_scope_map();
        for action in NavAction::ALL.iter().copied() {
            assert!(
                map.key_for(action).is_some(),
                "{action:?} is unreachable even with vim extras applied",
            );
        }
    }

    #[test]
    fn half_page_is_vim_only() {
        assert!(default_keys(NavAction::HalfPageUp).is_empty());
        assert!(default_keys(NavAction::HalfPageDown).is_empty());

        let extras = vim_letter_extras();
        let half_up = extras.contains(&(KeySequence::from(ctrl('u')), NavAction::HalfPageUp));
        let half_down = extras.contains(&(KeySequence::from(ctrl('d')), NavAction::HalfPageDown));
        assert!(half_up, "Ctrl-u must be the vim binding for half-page up");
        assert!(
            half_down,
            "Ctrl-d must be the vim binding for half-page down"
        );
    }

    #[test]
    fn page_keys_keep_named_default_and_ctrl_is_vim_only() {
        let map = default_bindings().into_scope_map();
        // PageUp/PageDown keep their rebindable named-key default.
        assert_eq!(
            map.key_for(NavAction::PageUp)
                .and_then(KeySequence::single_key),
            Some(KeyBind::from(KeyCode::PageUp)),
        );
        assert_eq!(
            map.key_for(NavAction::PageDown)
                .and_then(KeySequence::single_key),
            Some(KeyBind::from(KeyCode::PageDown)),
        );
        // Ctrl-b/Ctrl-f are not compiled defaults — they arrive only via
        // the vim extras.
        assert_eq!(map.action_for(&ctrl('b')), None);
        assert_eq!(map.action_for(&ctrl('f')), None);

        let extras = vim_letter_extras();
        assert!(extras.contains(&(KeySequence::from(ctrl('b')), NavAction::PageUp)));
        assert!(extras.contains(&(KeySequence::from(ctrl('f')), NavAction::PageDown)));
    }
}
