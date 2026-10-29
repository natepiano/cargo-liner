#[cfg(test)]
use tui_pane::KeyAlreadyBound;
pub(crate) use tui_pane::SingleKeyScope as ScopeMap;

#[cfg(test)]
use super::KeyBind;

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
