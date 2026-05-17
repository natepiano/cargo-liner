use std::collections::HashMap;

use super::KeyBind;

/// Bidirectional map for a single scope: key→action for dispatch,
/// action→key for display.
#[derive(Clone, Debug)]
pub(crate) struct ScopeMap<A: Copy + Eq + std::hash::Hash> {
    pub(crate) by_key:    HashMap<KeyBind, A>,
    pub(crate) by_action: HashMap<A, KeyBind>,
}

impl<A: Copy + Eq + std::hash::Hash> ScopeMap<A> {
    pub(super) fn new() -> Self {
        Self {
            by_key:    HashMap::new(),
            by_action: HashMap::new(),
        }
    }

    pub(super) fn insert(&mut self, key: KeyBind, action: A) {
        self.by_key.insert(key.clone(), action);
        self.by_action.insert(action, key);
    }

    #[cfg(test)]
    pub fn action_for(&self, key: &KeyBind) -> Option<A> { self.by_key.get(key).copied() }

    pub(super) fn key_for(&self, action: A) -> Option<&KeyBind> { self.by_action.get(&action) }

    /// Display string for an action's bound key, or `"—"` if unbound.
    #[cfg(test)]
    pub(super) fn display_key_for(&self, action: A) -> String {
        self.key_for(action)
            .map_or_else(|| "—".to_string(), KeyBind::display)
    }
}

impl<A: Copy + Eq + std::hash::Hash> Default for ScopeMap<A> {
    fn default() -> Self { Self::new() }
}
