//! `Bindings<A>`: ordered key→action declarations for a single scope.
//!
//! Authored by pane impls in `Shortcuts::defaults()` (typically via the
//! [`bindings!`](crate::bindings) macro), then folded into the scope's
//! [`ScopeMap`] during `KeymapBuilder::build()`.
//!
//! Insertion order is significant: the **first** key bound to an action
//! is the action's primary key (what the bar shows when only one key
//! fits).

use std::hash::Hash;

use super::key_bind::KeyBind;
use super::scope_map::ScopeMap;

/// Default-key declaration table for a single scope.
///
/// `Bindings` is the *write* side of a scope; [`ScopeMap`] is the *read*
/// side. Authoring code builds a `Bindings`, then the framework consumes
/// it via [`Self::into_scope_map`] during keymap construction.
#[derive(Debug)]
pub struct Bindings<A> {
    entries: Vec<(KeyBind, A)>,
}

impl<A> Bindings<A> {
    /// Empty builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Bind one key to one action. Multiple `bind` calls with the same
    /// action append additional keys; the first call's key is primary.
    pub fn bind(&mut self, key: impl Into<KeyBind>, action: A) -> &mut Self {
        self.entries.push((key.into(), action));
        self
    }

    /// Bind every key in the iterator to the same action, in order. The
    /// first key in the iterator becomes the primary if the action has
    /// no prior binding.
    pub fn bind_many(&mut self, keys: impl IntoIterator<Item = KeyBind>, action: A) -> &mut Self
    where
        A: Clone,
    {
        for key in keys {
            self.entries.push((key, action.clone()));
        }
        self
    }

    /// Consume the builder and return the fully indexed [`ScopeMap`].
    ///
    /// `debug_assert!`s on cross-action collision (defaults are author-
    /// controlled; collisions are bugs, not user input). User-supplied
    /// TOML goes through [`load`](super::load), which returns
    /// [`KeymapError::CrossActionCollision`](super::load::KeymapError::CrossActionCollision)
    /// for the same condition.
    #[must_use]
    pub fn into_scope_map(self) -> ScopeMap<A>
    where
        A: Copy + Eq + Hash,
    {
        let mut map = ScopeMap::new();
        for (key, action) in self.entries {
            map.insert(key, action);
        }
        map
    }
}

impl<A> Default for Bindings<A> {
    fn default() -> Self { Self::new() }
}

impl<A: Clone> Clone for Bindings<A> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
}

/// Declares a [`Bindings`] table inline.
///
/// Grammar:
///
/// ```text
/// bindings! {
///     KEY               => ACTION,
///     [KEY, KEY, ...]   => ACTION,
///     ...
/// }
/// ```
///
/// Each `KEY` is any `impl Into<KeyBind>` expression — a [`KeyCode`],
/// a `char`, [`KeyBind::shift`], [`KeyBind::ctrl`], or compositions of
/// those. The macro expands to a fresh [`Bindings`] populated via
/// [`Bindings::bind`] / [`Bindings::bind_many`].
///
/// Trailing commas are permitted on every line. The action expression
/// must be `Copy + Clone` (already required by [`ActionEnum`] super-
/// traits) so multi-key list arms can clone per element.
///
/// [`KeyCode`]: crossterm::event::KeyCode
/// [`KeyBind::shift`]: crate::KeyBind::shift
/// [`KeyBind::ctrl`]: crate::KeyBind::ctrl
/// [`ActionEnum`]: crate::ActionEnum
///
/// Example:
///
/// ```ignore
/// use crossterm::event::KeyCode;
/// use tui_pane::bindings;
/// use tui_pane::KeyBind;
///
/// tui_pane::action_enum! {
///     #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
///     pub enum NavAction {
///         Up   => "up",   "Move up";
///         Down => "down", "Move down";
///     }
/// }
///
/// let _table = bindings! {
///     KeyCode::Up   => NavAction::Up,
///     [KeyBind::from('j'), KeyBind::from(KeyCode::Down)] => NavAction::Down,
/// };
/// ```
#[macro_export]
macro_rules! bindings {
    ( $( $tt:tt )* ) => {{
        let mut __table: $crate::Bindings<_> = $crate::Bindings::new();
        $crate::__bindings_arms!(__table; $($tt)*);
        __table
    }};
}

/// Internal arm-walker for [`bindings!`]. Not part of the public API.
///
/// The arms are written in `incremental TT muncher` style so that a
/// single macro can accept both `KEY => ACTION` and `[KEYS] => ACTION`
/// arms in any order, with optional trailing commas.
#[doc(hidden)]
#[macro_export]
macro_rules! __bindings_arms {
    // Base case: empty input.
    ( $table:ident ; ) => {};

    // Multi-key list arm with trailing comma.
    ( $table:ident ; [ $( $key:expr ),+ $(,)? ] => $action:expr , $( $rest:tt )* ) => {
        $table.bind_many(
            [ $( $crate::KeyBind::from($key) ),+ ],
            $action,
        );
        $crate::__bindings_arms!($table; $( $rest )*);
    };

    // Multi-key list arm without trailing comma (last arm).
    ( $table:ident ; [ $( $key:expr ),+ $(,)? ] => $action:expr ) => {
        $table.bind_many(
            [ $( $crate::KeyBind::from($key) ),+ ],
            $action,
        );
    };

    // Single-key arm with trailing comma.
    ( $table:ident ; $key:expr => $action:expr , $( $rest:tt )* ) => {
        $table.bind($key, $action);
        $crate::__bindings_arms!($table; $( $rest )*);
    };

    // Single-key arm without trailing comma (last arm).
    ( $table:ident ; $key:expr => $action:expr ) => {
        $table.bind($key, $action);
    };
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
    use crossterm::event::KeyModifiers;

    use super::*;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestAction {
        Up,
        Down,
        ExpandAll,
    }

    #[test]
    fn bind_records_insertion_order() {
        let mut table: Bindings<TestAction> = Bindings::new();
        table
            .bind(KeyCode::Up, TestAction::Up)
            .bind('k', TestAction::Up);
        let map = table.into_scope_map();
        assert_eq!(
            map.key_for(TestAction::Up),
            Some(&KeyBind::from(KeyCode::Up))
        );
        assert_eq!(
            map.display_keys_for(TestAction::Up),
            &[KeyBind::from(KeyCode::Up), KeyBind::from('k')],
        );
    }

    #[test]
    fn bind_many_uses_iterator_order() {
        let mut table: Bindings<TestAction> = Bindings::new();
        table.bind_many(
            [KeyBind::from('='), KeyBind::from('+')],
            TestAction::ExpandAll,
        );
        let map = table.into_scope_map();
        assert_eq!(
            map.key_for(TestAction::ExpandAll),
            Some(&KeyBind::from('=')),
        );
        assert_eq!(
            map.display_keys_for(TestAction::ExpandAll),
            &[KeyBind::from('='), KeyBind::from('+')],
        );
    }

    #[test]
    fn macro_single_key_arm() {
        let table = bindings! {
            KeyCode::Up => TestAction::Up
        };
        let map = table.into_scope_map();
        assert_eq!(
            map.action_for(&KeyBind::from(KeyCode::Up)),
            Some(TestAction::Up),
        );
    }

    #[test]
    fn macro_multi_key_list_arm() {
        let table = bindings! {
            [KeyBind::from('='), KeyBind::from('+')] => TestAction::ExpandAll
        };
        let map = table.into_scope_map();
        assert_eq!(
            map.action_for(&KeyBind::from('=')),
            Some(TestAction::ExpandAll),
        );
        assert_eq!(
            map.action_for(&KeyBind::from('+')),
            Some(TestAction::ExpandAll),
        );
    }

    #[test]
    fn macro_mixed_arms_with_trailing_comma() {
        let table = bindings! {
            KeyCode::Up => TestAction::Up,
            [KeyBind::from('j'), KeyBind::from(KeyCode::Down)] => TestAction::Down,
            'k' => TestAction::Up,
        };
        let map = table.into_scope_map();
        assert_eq!(
            map.action_for(&KeyBind::from(KeyCode::Up)),
            Some(TestAction::Up),
        );
        assert_eq!(map.action_for(&KeyBind::from('j')), Some(TestAction::Down),);
        assert_eq!(
            map.action_for(&KeyBind::from(KeyCode::Down)),
            Some(TestAction::Down),
        );
        assert_eq!(map.action_for(&KeyBind::from('k')), Some(TestAction::Up));
    }

    #[test]
    fn macro_accepts_composed_modifier_keybind() {
        let table = bindings! {
            KeyBind::ctrl(KeyBind::shift('g')) => TestAction::Up
        };
        let map = table.into_scope_map();
        let key = KeyBind {
            code: KeyCode::Char('g'),
            mods: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        };
        assert_eq!(map.action_for(&key), Some(TestAction::Up));
    }

    #[test]
    fn into_scope_map_invariant_no_orphans() {
        let table = bindings! {
            KeyCode::Up => TestAction::Up,
            'k' => TestAction::Up,
            KeyCode::Down => TestAction::Down,
            'j' => TestAction::Down,
            KeyCode::Left => TestAction::ExpandAll,
        };
        let map = table.into_scope_map();
        let total_action_keys: usize = [TestAction::Up, TestAction::Down, TestAction::ExpandAll]
            .iter()
            .map(|a| map.display_keys_for(*a).len())
            .sum();
        assert_eq!(total_action_keys, 5);
    }

    #[test]
    fn default_is_empty() {
        let table: Bindings<TestAction> = Bindings::default();
        let map = table.into_scope_map();
        assert_eq!(map.action_for(&KeyBind::from(KeyCode::Up)), None);
    }
}
