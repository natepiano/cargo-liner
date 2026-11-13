use crossterm::event::KeyCode;

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
        for (key, action) in Self::key_bindings() {
            bindings.bind(key, action);
        }
        bindings
    }

    /// Return the default tree bindings with plain row arrows.
    #[must_use]
    pub fn key_bindings() -> [(KeyBind, Self); 4] {
        [
            (KeyBind::from_char('='), Self::ExpandAll),
            (KeyBind::from_char('-'), Self::CollapseAll),
            (KeyBind::from(KeyCode::Right), Self::ExpandRow),
            (KeyBind::from(KeyCode::Left), Self::CollapseRow),
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
