//! `ScopeMap<A>`: resolved binding table for a single scope.
//!
//! Two indexes:
//!
//! - `by_key`:    1-to-1 within a scope. The dispatcher's lookup path.
//! - `by_action`: 1-to-many. Insertion order preserved per action; the first entry in each
//!   `Vec<KeySequence>` is the action's primary binding (what the bar renders when only one fits).
//!
//! Constructed by [`Bindings::into_scope_map`](super::bindings::Bindings::into_scope_map)
//! and the TOML loader. App code never builds one directly — it
//! always receives `&ScopeMap<A>` from the keymap.

use std::collections::HashMap;
use std::hash::Hash;

use super::key_bind::KeyBind;
use super::key_sequence::KeySequence;

/// Resolved binding table for a single scope.
///
/// Invariant locked by tests:
///
/// ```text
/// by_key.len() == by_action.values().map(Vec::len).sum::<usize>()
/// ```
///
/// Every key in `by_key` appears exactly once across all `by_action`
/// vectors. No orphans, no double-counts.
#[derive(Debug)]
pub struct ScopeMap<A: Copy + Eq + Hash> {
    by_key:    HashMap<KeySequence, A>,
    by_action: HashMap<A, Vec<KeySequence>>,
}

impl<A: Copy + Eq + Hash> ScopeMap<A> {
    /// Empty map. `pub(super)` because only [`Bindings::into_scope_map`]
    /// (sibling) and the TOML loader build one.
    pub(super) fn new() -> Self {
        Self {
            by_key:    HashMap::new(),
            by_action: HashMap::new(),
        }
    }

    /// Insert one `(key, action)` pair.
    ///
    /// `pub(super)` for the same reason as [`Self::new`].
    ///
    /// `debug_assert!`s that `key` is unbound or already bound to the
    /// same `action`. Cross-action collisions inside one scope are bugs
    /// in `defaults()`; the TOML loader catches the same condition for
    /// user input and returns `Err` instead of panicking.
    pub(super) fn insert(&mut self, key: impl Into<KeySequence>, action: A) {
        let key = key.into();
        debug_assert!(
            self.by_key
                .get(&key)
                .is_none_or(|&existing| existing == action),
            "ScopeMap::insert: key {key:?} already maps to a different action",
        );
        if self.by_key.insert(key.clone(), action).is_none() {
            self.by_action.entry(action).or_default().push(key);
        }
    }

    /// Dispatcher lookup: which action does `key` fire?
    #[must_use]
    pub fn action_for(&self, key: &KeyBind) -> Option<A> {
        self.by_key.get(&KeySequence::from(*key)).copied()
    }

    /// Dispatcher lookup for a full sequence.
    #[must_use]
    pub fn action_for_sequence(&self, sequence: &KeySequence) -> Option<A> {
        self.by_key.get(sequence).copied()
    }

    /// Whether `prefix` is the start of any longer sequence.
    #[must_use]
    pub fn has_prefix(&self, prefix: &[KeyBind]) -> bool {
        self.by_key
            .keys()
            .any(|sequence| sequence.starts_with_strict(prefix))
    }

    /// Primary key for `action` — the first key bound to it. The bar
    /// reads this when rendering a `BarRow::Paired` slot.
    #[must_use]
    pub fn key_for(&self, action: A) -> Option<&KeySequence> {
        self.by_action.get(&action).and_then(|v| v.first())
    }

    /// Display the primary key, full name (`"Up"`, `"Ctrl+K"`). The
    /// keymap-overlay help screen renders these.
    #[must_use]
    pub fn display_key_for(&self, action: A) -> String {
        self.key_for(action)
            .map(KeySequence::display)
            .unwrap_or_default()
    }

    /// All keys bound to `action`, insertion order. The bar's `Single`
    /// row joins these with `,` after `display_short`. Returns an empty
    /// slice for unbound actions.
    #[must_use]
    pub fn display_keys_for(&self, action: A) -> &[KeySequence] {
        self.by_action.get(&action).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;

    use super::*;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestAction {
        Up,
        Down,
        Left,
    }

    #[test]
    fn empty_map_returns_none_and_empty_slice() {
        let map: ScopeMap<TestAction> = ScopeMap::new();
        assert_eq!(map.action_for(&KeyBind::from(KeyCode::Up)), None);
        assert_eq!(map.key_for(TestAction::Up), None);
        assert_eq!(map.display_key_for(TestAction::Up), "");
        assert!(map.display_keys_for(TestAction::Up).is_empty());
    }

    #[test]
    fn insert_then_lookup_round_trip() {
        let mut map = ScopeMap::new();
        map.insert(KeyBind::from(KeyCode::Up), TestAction::Up);
        assert_eq!(
            map.action_for(&KeyBind::from(KeyCode::Up)),
            Some(TestAction::Up),
        );
        assert_eq!(
            map.key_for(TestAction::Up)
                .and_then(KeySequence::single_key),
            Some(KeyBind::from(KeyCode::Up))
        );
    }

    #[test]
    fn first_key_is_primary_under_insertion_order() {
        let mut map = ScopeMap::new();
        map.insert(KeyBind::from(KeyCode::Up), TestAction::Up);
        map.insert(KeyBind::from('k'), TestAction::Up);
        assert_eq!(
            map.key_for(TestAction::Up)
                .and_then(KeySequence::single_key),
            Some(KeyBind::from(KeyCode::Up))
        );
        assert_eq!(
            map.display_keys_for(TestAction::Up),
            &[KeyBind::from(KeyCode::Up), KeyBind::from('k')],
        );
    }

    #[test]
    fn invariant_by_key_count_matches_by_action_total() {
        let mut map = ScopeMap::new();
        map.insert(KeyBind::from(KeyCode::Up), TestAction::Up);
        map.insert(KeyBind::from('k'), TestAction::Up);
        map.insert(KeyBind::from(KeyCode::Down), TestAction::Down);
        map.insert(KeyBind::from('j'), TestAction::Down);
        map.insert(KeyBind::from(KeyCode::Left), TestAction::Left);

        let by_action_total: usize = map.by_action.values().map(Vec::len).sum();
        assert_eq!(map.by_key.len(), by_action_total);
        assert_eq!(map.by_key.len(), 5);
    }

    #[test]
    fn insert_idempotent_same_action() {
        let mut map = ScopeMap::new();
        map.insert(KeyBind::from(KeyCode::Up), TestAction::Up);
        map.insert(KeyBind::from(KeyCode::Up), TestAction::Up);
        assert_eq!(map.display_keys_for(TestAction::Up).len(), 1);
        assert_eq!(map.by_key.len(), 1);
    }

    #[test]
    fn display_key_for_uses_primary() {
        let mut map = ScopeMap::new();
        map.insert(KeyBind::ctrl('k'), TestAction::Up);
        map.insert(KeyBind::from('k'), TestAction::Up);
        assert_eq!(map.display_key_for(TestAction::Up), "ctrl-k");
    }
}
