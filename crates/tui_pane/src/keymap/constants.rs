use super::ExpansionAction;
use super::KeyBind;

// `MAX_FUNCTION_KEY` limits the function-key number accepted by key parsing.
pub(super) const MAX_FUNCTION_KEY: u8 = 12;

pub(super) const EXPANSION_VIM_EXTRAS: &[(ExpansionAction, KeyBind)] = &[
    (ExpansionAction::ExpandRow, KeyBind::from_char('l')),
    (ExpansionAction::CollapseRow, KeyBind::from_char('h')),
];
