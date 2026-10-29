//! Single-key scopes, preserving one primary display key per action.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use super::KeyBind;

/// Bidirectional map for a single scope: key→action for dispatch,
/// action→primary key for display. The first key bound to an action stays primary.
#[derive(Clone, Debug)]
pub struct SingleKeyScope<A: Copy + Eq + std::hash::Hash> {
    by_key:    HashMap<KeyBind, A>,
    by_action: HashMap<A, KeyBind>,
}

/// The action that already owns the requested key in this scope.
#[derive(Debug, PartialEq, Eq)]
pub struct KeyAlreadyBound<A> {
    /// Action whose binding was preserved.
    pub existing_action: A,
}

impl<A: Copy + Eq + std::hash::Hash> SingleKeyScope<A> {
    /// Create an empty single-key scope.
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_key:    HashMap::new(),
            by_action: HashMap::new(),
        }
    }

    /// Bind a free key, preserving the action's first key as its primary hint.
    /// An occupied key is rejected without changing either lookup.
    ///
    /// # Errors
    /// Returns the existing action if this key is already occupied.
    pub fn bind(&mut self, key: KeyBind, action: A) -> Result<(), KeyAlreadyBound<A>> {
        match self.by_key.entry(key) {
            Entry::Occupied(entry) => Err(KeyAlreadyBound {
                existing_action: *entry.get(),
            }),
            Entry::Vacant(entry) => {
                entry.insert(action);
                self.by_action.entry(action).or_insert(key);
                Ok(())
            },
        }
    }

    /// Iterate the bound keys and their actions.
    pub fn bindings(&self) -> impl Iterator<Item = (&KeyBind, &A)> { self.by_key.iter() }

    /// Find the action bound to one key.
    #[must_use]
    pub fn action_for(&self, key: &KeyBind) -> Option<A> { self.by_key.get(key).copied() }

    /// The primary key bound to an action.
    #[must_use]
    pub fn key_for(&self, action: A) -> Option<&KeyBind> { self.by_action.get(&action) }

    /// Display string for an action's bound key, or `"—"` if unbound.
    #[must_use]
    pub fn display_key_for(&self, action: A) -> String {
        self.key_for(action)
            .map_or_else(|| "—".to_string(), KeyBind::display)
    }
}

impl<A: Copy + Eq + std::hash::Hash> Default for SingleKeyScope<A> {
    fn default() -> Self { Self::new() }
}

impl<A: Copy + Eq + std::hash::Hash, const N: usize> From<[(KeyBind, A); N]> for SingleKeyScope<A> {
    fn from(bindings: [(KeyBind, A); N]) -> Self {
        let mut scope = Self::new();
        for (key, action) in bindings {
            // Built-in defaults are unique per scope; defaults_scope_map_consistency pins this.
            let _ = scope.bind(key, action);
        }
        scope
    }
}
