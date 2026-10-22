use std::fmt::Write as _;

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use tui_pane::Action;

use super::KeyBind;
use super::ScopeMap;
use super::actions;
use super::actions::CiRunsAction;
use super::actions::GitAction;
use super::actions::LintsAction;
use super::actions::PackageAction;
use super::actions::ProjectListAction;
use super::actions::TargetsAction;

/// Runtime lookup structure: one `ScopeMap` per scope, built from the
/// TOML config at load time.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedKeymap {
    pub(super) project_list:    ScopeMap<ProjectListAction>,
    pub(super) package:         ScopeMap<PackageAction>,
    pub(super) git:             ScopeMap<GitAction>,
    pub(super) targets:         ScopeMap<TargetsAction>,
    pub(in crate::tui) ci_runs: ScopeMap<CiRunsAction>,
    pub(super) lints:           ScopeMap<LintsAction>,
}

impl ResolvedKeymap {
    /// The built-in default keymap matching the current hardcoded bindings.
    pub(crate) fn defaults() -> Self {
        Self {
            project_list: ScopeMap::from([
                (
                    KeyBind::from(KeyCode::Char('=')),
                    ProjectListAction::ExpandAll,
                ),
                (
                    KeyBind::from(KeyCode::Char('-')),
                    ProjectListAction::CollapseAll,
                ),
                // ExpandRow / CollapseRow are pane-scope actions routed through
                // the pane-scope match in `handle_normal_key`. Bare Right / Left
                // are already mapped to NavAction::Right / ::Left in the
                // framework keymap, so the pane-scope defaults bind ExpandRow /
                // CollapseRow to Shift+Right / Shift+Left to avoid colliding
                // with the navigation defaults.
                (
                    KeyBind::from_parts(KeyCode::Right, KeyModifiers::SHIFT),
                    ProjectListAction::ExpandRow,
                ),
                (
                    KeyBind::from_parts(KeyCode::Left, KeyModifiers::SHIFT),
                    ProjectListAction::CollapseRow,
                ),
            ]),
            package:      ScopeMap::from([(
                KeyBind::from(KeyCode::Enter),
                PackageAction::Activate,
            )]),
            git:          ScopeMap::from([(KeyBind::from(KeyCode::Enter), GitAction::Activate)]),
            targets:      ScopeMap::from([
                (KeyBind::from(KeyCode::Enter), TargetsAction::Activate),
                (
                    KeyBind::from(KeyCode::Char('r')),
                    TargetsAction::ReleaseBuild,
                ),
            ]),
            ci_runs:      ScopeMap::from([
                (KeyBind::from(KeyCode::Enter), CiRunsAction::Activate),
                (KeyBind::from(KeyCode::Char('f')), CiRunsAction::FetchMore),
                (KeyBind::from(KeyCode::Char('b')), CiRunsAction::ShowBranch),
                (KeyBind::from(KeyCode::Char('a')), CiRunsAction::ShowAll),
                (KeyBind::from(KeyCode::Char('d')), CiRunsAction::ClearCache),
            ]),
            lints:        ScopeMap::from([
                (KeyBind::from(KeyCode::Enter), LintsAction::Activate),
                (KeyBind::from(KeyCode::Char('d')), LintsAction::ClearHistory),
            ]),
        }
    }

    fn write_scope<A: Copy + Eq + std::hash::Hash>(
        out: &mut String,
        header: &str,
        scope: &ScopeMap<A>,
        actions: &[A],
        toml_key: fn(A) -> &'static str,
    ) {
        let _ = writeln!(out, "[{header}]");
        let mut entries: Vec<(&str, String)> = actions
            .iter()
            .map(|&action| {
                let key_str = scope
                    .key_for(action)
                    .map_or_else(String::new, KeyBind::display);
                (toml_key(action), key_str)
            })
            .collect();
        entries.sort_by_key(|(name, _)| *name);
        let max_len = entries
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0);
        for (name, value) in &entries {
            let _ = writeln!(out, "{name:<max_len$} = \"{value}\"");
        }
        out.push('\n');
    }

    /// Generate the default TOML content for `keymap.toml`.
    pub(super) fn default_toml() -> String {
        let km = Self::defaults();
        let mut out = String::from(
            "# cargo-port keymap configuration\n\
             # Edit bindings below. Format: action = \"key\" or \"modifier-key\"\n\
             # Modifiers: ctrl, alt, shift.  Examples: \"ctrl-r\", \"shift-tab\", \"q\"\n\
             # Note: = and + are treated as the same physical key.\n\
             # Note: when vim navigation is enabled, vim navigation keys are reserved\n\
             #       for navigation and cannot be used as action keys.\n\n",
        );

        Self::write_all_scopes(&mut out, &km);

        out
    }

    /// Generate TOML content from the given keymap (for saving after UI edits).
    pub(super) fn default_toml_from(km: &Self) -> String {
        let mut out = String::new();
        Self::write_all_scopes(&mut out, km);
        out
    }

    fn write_all_scopes(out: &mut String, km: &Self) {
        Self::write_scope(
            out,
            "project_list",
            &km.project_list,
            <ProjectListAction as Action>::ALL,
            actions::action_toml_key::<ProjectListAction>,
        );
        Self::write_scope(
            out,
            "package",
            &km.package,
            <PackageAction as Action>::ALL,
            actions::action_toml_key::<PackageAction>,
        );
        Self::write_scope(
            out,
            "git",
            &km.git,
            <GitAction as Action>::ALL,
            actions::action_toml_key::<GitAction>,
        );
        Self::write_scope(
            out,
            "targets",
            &km.targets,
            <TargetsAction as Action>::ALL,
            actions::action_toml_key::<TargetsAction>,
        );
        Self::write_scope(
            out,
            "ci_runs",
            &km.ci_runs,
            <CiRunsAction as Action>::ALL,
            actions::action_toml_key::<CiRunsAction>,
        );
        Self::write_scope(
            out,
            "lints",
            &km.lints,
            <LintsAction as Action>::ALL,
            actions::action_toml_key::<LintsAction>,
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use toml::Table;

    use super::*;

    #[test]
    fn defaults_scope_map_consistency() {
        fn check<A: Copy + Eq + std::hash::Hash>(scope: &ScopeMap<A>, actions: &[A]) {
            for &action in actions {
                assert!(
                    scope.key_for(action).is_some(),
                    "action missing from by_action"
                );
            }
            for (key, &action) in scope.bindings() {
                assert_eq!(
                    scope.key_for(action),
                    Some(key),
                    "by_key/by_action mismatch"
                );
            }
            assert_eq!(scope.bindings().count(), actions.len());
        }

        let km = ResolvedKeymap::defaults();
        check(&km.project_list, <ProjectListAction as Action>::ALL);
        check(&km.package, <PackageAction as Action>::ALL);
        check(&km.git, <GitAction as Action>::ALL);
        check(&km.targets, <TargetsAction as Action>::ALL);
        check(&km.ci_runs, <CiRunsAction as Action>::ALL);
        check(&km.lints, <LintsAction as Action>::ALL);
    }

    #[test]
    fn default_toml_is_parseable() {
        let toml_str = ResolvedKeymap::default_toml();
        let table: Table = toml_str.parse().unwrap();
        assert!(table.contains_key("project_list"));
        assert!(table.contains_key("package"));
        assert!(table.contains_key("git"));
        assert!(table.contains_key("targets"));
        assert!(table.contains_key("ci_runs"));
        assert!(table.contains_key("lints"));
    }
}
