use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;

use super::Bindings;
use super::KeyBind;
use super::constants::EXPANSION_VIM_EXTRAS;

crate::action_enum! {
    /// Shared actions for trees with expandable rows.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum ExpansionAction {
        /// Open every expandable row.
        ExpandAll   => ("expand_all",   "expand all", "Expand all");
        /// Close every expandable row.
        CollapseAll => ("collapse_all", "collapse all", "Collapse all");
        /// Open the selected row.
        ExpandRow   => ("expand_row",   "expand row", "Expand row");
        /// Close the selected row, or select and close its parent.
        CollapseRow => ("collapse_row", "collapse row", "Collapse row");
    }
}

impl ExpansionAction {
    /// Return the default tree bindings.
    #[must_use]
    pub fn bindings() -> Bindings<Self> {
        let mut bindings = Bindings::new();
        for (key, action) in Self::key_bindings(KeyModifiers::NONE) {
            bindings.bind(key, action);
        }
        bindings
    }

    /// Return the defaults with modifiers for row arrows in a separate navigation scope.
    #[must_use]
    pub fn key_bindings(row_modifiers: KeyModifiers) -> [(KeyBind, Self); 4] {
        [
            (KeyBind::from_char('='), Self::ExpandAll),
            (KeyBind::from_char('-'), Self::CollapseAll),
            (
                KeyBind::from_parts(KeyCode::Right, row_modifiers),
                Self::ExpandRow,
            ),
            (
                KeyBind::from_parts(KeyCode::Left, row_modifiers),
                Self::CollapseRow,
            ),
        ]
    }

    /// Return row arrows' optional Vim letter bindings.
    #[must_use]
    pub const fn vim_extras() -> &'static [(Self, KeyBind)] { EXPANSION_VIM_EXTRAS }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_resolve_each_tree_operation() {
        let bindings = ExpansionAction::bindings().into_scope_map();
        assert_eq!(
            bindings.action_for(&KeyBind::from(KeyCode::Right)),
            Some(ExpansionAction::ExpandRow)
        );
        assert_eq!(
            bindings.action_for(&KeyBind::from(KeyCode::Left)),
            Some(ExpansionAction::CollapseRow)
        );
        assert_eq!(
            bindings.action_for(&KeyBind::from_char('=')),
            Some(ExpansionAction::ExpandAll)
        );
        assert_eq!(
            bindings.action_for(&KeyBind::from_char('-')),
            Some(ExpansionAction::CollapseAll)
        );
    }
}
