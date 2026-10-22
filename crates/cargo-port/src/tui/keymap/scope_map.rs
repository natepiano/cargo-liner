use std::collections::HashMap;
use std::collections::hash_map::Entry;

use super::KeyBind;

/// Bidirectional map for a single scope: key→action for dispatch,
/// action→primary key for display. The first key bound to an action stays primary.
#[derive(Clone, Debug)]
pub(crate) struct ScopeMap<A: Copy + Eq + std::hash::Hash> {
    by_key:    HashMap<KeyBind, A>,
    by_action: HashMap<A, KeyBind>,
}

/// The action that already owns the requested key in this scope.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct KeyAlreadyBound<A> {
    pub(super) existing_action: A,
}

impl<A: Copy + Eq + std::hash::Hash> ScopeMap<A> {
    pub(super) fn new() -> Self {
        Self {
            by_key:    HashMap::new(),
            by_action: HashMap::new(),
        }
    }

    /// Bind a free key, preserving the action's first key as its primary hint.
    /// An occupied key is rejected without changing either lookup.
    pub(super) fn bind(&mut self, key: KeyBind, action: A) -> Result<(), KeyAlreadyBound<A>> {
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

    #[cfg(test)]
    pub(super) fn bindings(&self) -> impl Iterator<Item = (&KeyBind, &A)> { self.by_key.iter() }

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

impl<A: Copy + Eq + std::hash::Hash, const N: usize> From<[(KeyBind, A); N]> for ScopeMap<A> {
    fn from(bindings: [(KeyBind, A); N]) -> Self {
        let mut scope = Self::new();
        for (key, action) in bindings {
            // Built-in defaults are unique per scope; defaults_scope_map_consistency pins this.
            let _ = scope.bind(key, action);
        }
        scope
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;

    use super::*;
    use crate::tui::keymap::ProjectListAction;

    #[test]
    fn conflicting_secondary_binding_preserves_both_primary_keys() {
        let mut scope = ScopeMap::new();
        let expand = KeyBind::from(KeyCode::Char('x'));
        let collapse = KeyBind::from(KeyCode::Char('y'));
        assert_eq!(scope.bind(expand, ProjectListAction::ExpandAll), Ok(()));
        assert_eq!(scope.bind(collapse, ProjectListAction::CollapseAll), Ok(()));

        assert_eq!(
            scope.bind(expand, ProjectListAction::CollapseAll),
            Err(KeyAlreadyBound {
                existing_action: ProjectListAction::ExpandAll,
            })
        );
        assert_eq!(scope.key_for(ProjectListAction::ExpandAll), Some(&expand));
        assert_eq!(
            scope.key_for(ProjectListAction::CollapseAll),
            Some(&collapse)
        );
        assert_eq!(
            scope.action_for(&expand),
            Some(ProjectListAction::ExpandAll)
        );
        assert_eq!(
            scope.action_for(&collapse),
            Some(ProjectListAction::CollapseAll)
        );
        assert_eq!(scope.bindings().count(), 2);
    }

    #[test]
    fn binding_an_occupied_key_preserves_the_existing_action() {
        let mut scope = ScopeMap::new();
        let key = KeyBind::from(KeyCode::Char('x'));
        assert_eq!(scope.bind(key, ProjectListAction::ExpandAll), Ok(()));
        assert_eq!(
            scope.bind(key, ProjectListAction::CollapseAll),
            Err(KeyAlreadyBound {
                existing_action: ProjectListAction::ExpandAll,
            })
        );
        assert_eq!(
            scope.bind(key, ProjectListAction::ExpandAll),
            Err(KeyAlreadyBound {
                existing_action: ProjectListAction::ExpandAll,
            })
        );

        assert_eq!(scope.action_for(&key), Some(ProjectListAction::ExpandAll));
        assert_eq!(scope.key_for(ProjectListAction::ExpandAll), Some(&key));
        assert_eq!(scope.key_for(ProjectListAction::CollapseAll), None);
    }
}
