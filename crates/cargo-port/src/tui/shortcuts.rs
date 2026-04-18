use std::borrow::Cow;

use crate::keymap::CiRunsAction;
use crate::keymap::GlobalAction;
use crate::keymap::LintsAction;
use crate::keymap::PackageAction;
use crate::keymap::ProjectListAction;
use crate::keymap::ResolvedKeymap;
use crate::keymap::TargetsAction;

/// The current input context, derived from app state. Determines which
/// shortcuts are shown in the status bar and how keys are dispatched.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum InputContext {
    ProjectList,
    DetailFields,
    DetailTargets,
    CiRuns,
    Toasts,
    Lints,
    Output,
    Finder,
    Settings,
    SettingsEditing,
    Keymap,
    KeymapAwaiting,
    KeymapConflict,
}

impl InputContext {
    /// Text-input contexts consume all `Char` keys, so global shortcuts
    /// (which are letter-based) must not be shown or dispatched.
    pub const fn is_text_input(self) -> bool {
        matches!(
            self,
            Self::Finder
                | Self::Settings
                | Self::SettingsEditing
                | Self::KeymapAwaiting
                | Self::KeymapConflict
        )
    }

    /// Overlay contexts own total focus — global shortcuts are hidden.
    pub const fn is_overlay(self) -> bool {
        matches!(
            self,
            Self::Finder
                | Self::Settings
                | Self::SettingsEditing
                | Self::Keymap
                | Self::KeymapAwaiting
                | Self::KeymapConflict
        )
    }
}

/// A keyboard shortcut for display in the status bar.
pub(super) struct Shortcut {
    pub key:         Cow<'static, str>,
    pub description: &'static str,
    pub state:       ShortcutState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShortcutState {
    Enabled,
    Disabled,
}

impl Shortcut {
    const fn fixed(key: &'static str, description: &'static str) -> Self {
        Self {
            key: Cow::Borrowed(key),
            description,
            state: ShortcutState::Enabled,
        }
    }

    const fn from_keymap(key: String, description: &'static str) -> Self {
        Self {
            key: Cow::Owned(key),
            description,
            state: ShortcutState::Enabled,
        }
    }

    const fn disabled_from_keymap(key: String, description: &'static str) -> Self {
        Self {
            key: Cow::Owned(key),
            description,
            state: ShortcutState::Disabled,
        }
    }
}

// ── Static navigation shortcuts ──────────────────────────────────────

const NAV: Shortcut = Shortcut::fixed("↑/↓", "nav");
const ARROWS_EXPAND: Shortcut = Shortcut::fixed("←/→", "expand");
const ARROWS_TOGGLE: Shortcut = Shortcut::fixed("←/→", "toggle");
const TAB_PANE: Shortcut = Shortcut::fixed("Tab", "pane");
const ESC_CANCEL: Shortcut = Shortcut::fixed("Esc", "cancel");
const ESC_CLOSE: Shortcut = Shortcut::fixed("Esc", "close");
const EXPAND_COLLAPSE_ALL: Shortcut = Shortcut::fixed("+/-", "all");

const fn enter(description: &'static str) -> Shortcut { Shortcut::fixed("Enter", description) }

// ── Public API ─────────────────────────────────────────────────────────

/// Status bar shortcut groups: left (navigation), center (pane actions),
/// right (globals).
pub(super) struct StatusBarGroups {
    pub navigation: Vec<Shortcut>,
    pub actions:    Vec<Shortcut>,
    pub global:     Vec<Shortcut>,
}

/// Build all three shortcut groups for the current context.
pub(super) fn for_status_bar(
    context: InputContext,
    enter_action: Option<&'static str>,
    is_rust: bool,
    clear_lint_action: Option<&'static str>,
    km: &ResolvedKeymap,
    terminal_command_configured: bool,
    selected_project_is_deleted: bool,
) -> StatusBarGroups {
    let (navigation, actions) = match context {
        InputContext::Finder => (vec![NAV], vec![enter("go to"), ESC_CLOSE]),
        InputContext::Settings => (
            vec![NAV, ARROWS_TOGGLE],
            vec![
                enter("edit"),
                Shortcut::from_keymap(
                    km.global.display_key_for(GlobalAction::OpenEditor),
                    "editor",
                ),
                ESC_CLOSE,
            ],
        ),
        InputContext::SettingsEditing => (vec![], vec![enter("confirm"), ESC_CANCEL]),
        InputContext::Keymap => (
            vec![NAV],
            vec![
                enter("edit"),
                Shortcut::from_keymap(
                    km.global.display_key_for(GlobalAction::OpenEditor),
                    "editor",
                ),
                ESC_CLOSE,
            ],
        ),
        InputContext::KeymapAwaiting => (vec![], vec![ESC_CANCEL]),
        InputContext::KeymapConflict => (vec![], vec![enter("clear"), ESC_CANCEL]),
        InputContext::DetailFields | InputContext::DetailTargets => {
            detail_groups(context, enter_action, is_rust, km)
        },
        InputContext::CiRuns => ci_groups(enter_action, Some("branch/all"), km),
        InputContext::Toasts => toast_groups(km),
        InputContext::Lints => lints_groups(enter_action, clear_lint_action, km),
        InputContext::Output => (vec![], vec![ESC_CLOSE]),
        InputContext::ProjectList => project_list_groups(enter_action, is_rust, km),
    };

    let global = if context.is_overlay() || context.is_text_input() {
        vec![]
    } else {
        let terminal_shortcut = if terminal_command_configured && !selected_project_is_deleted {
            Shortcut::from_keymap(
                km.global.display_key_for(GlobalAction::OpenTerminal),
                "terminal",
            )
        } else {
            Shortcut::disabled_from_keymap(
                km.global.display_key_for(GlobalAction::OpenTerminal),
                "terminal",
            )
        };
        let editor_shortcut = if selected_project_is_deleted {
            Shortcut::disabled_from_keymap(
                km.global.display_key_for(GlobalAction::OpenEditor),
                "editor",
            )
        } else {
            Shortcut::from_keymap(
                km.global.display_key_for(GlobalAction::OpenEditor),
                "editor",
            )
        };
        vec![
            Shortcut::from_keymap(km.global.display_key_for(GlobalAction::Find), "find"),
            editor_shortcut,
            terminal_shortcut,
            Shortcut::from_keymap(
                km.global.display_key_for(GlobalAction::Settings),
                "settings",
            ),
            Shortcut::from_keymap(
                km.global.display_key_for(GlobalAction::OpenKeymap),
                "keymap",
            ),
            Shortcut::from_keymap(km.global.display_key_for(GlobalAction::Rescan), "rescan"),
            Shortcut::from_keymap(km.global.display_key_for(GlobalAction::Quit), "quit"),
            Shortcut::from_keymap(km.global.display_key_for(GlobalAction::Restart), "restart"),
        ]
    };

    StatusBarGroups {
        navigation,
        actions,
        global,
    }
}

// ── Context builders ───────────────────────────────────────────────────

fn detail_groups(
    context: InputContext,
    enter_action: Option<&'static str>,
    is_rust: bool,
    km: &ResolvedKeymap,
) -> (Vec<Shortcut>, Vec<Shortcut>) {
    let navigation = vec![NAV, TAB_PANE];

    let mut actions = Vec::new();
    if let Some(action) = enter_action {
        actions.push(enter(action));
    }
    if context == InputContext::DetailTargets {
        actions.push(Shortcut::from_keymap(
            km.targets.display_key_for(TargetsAction::ReleaseBuild),
            "release",
        ));
    }
    if is_rust {
        // All detail panes share the same default key for clean.
        actions.push(Shortcut::from_keymap(
            km.package.display_key_for(PackageAction::Clean),
            "clean",
        ));
    }

    (navigation, actions)
}

fn ci_groups(
    enter_action: Option<&'static str>,
    toggle_action: Option<&'static str>,
    km: &ResolvedKeymap,
) -> (Vec<Shortcut>, Vec<Shortcut>) {
    let navigation = vec![NAV, TAB_PANE];

    let mut actions = Vec::new();
    if let Some(action) = enter_action {
        actions.push(enter(action));
    }
    if let Some(action) = toggle_action {
        actions.push(Shortcut::from_keymap(
            km.ci_runs.display_key_for(CiRunsAction::ToggleView),
            action,
        ));
    }
    actions.push(Shortcut::from_keymap(
        km.ci_runs.display_key_for(CiRunsAction::FetchMore),
        "fetch more",
    ));
    actions.push(Shortcut::from_keymap(
        km.ci_runs.display_key_for(CiRunsAction::ClearCache),
        "clear cache",
    ));

    (navigation, actions)
}

fn toast_groups(km: &ResolvedKeymap) -> (Vec<Shortcut>, Vec<Shortcut>) {
    (
        vec![NAV, TAB_PANE],
        vec![Shortcut::from_keymap(
            km.global.display_key_for(GlobalAction::Dismiss),
            "dismiss",
        )],
    )
}

fn lints_groups(
    enter_action: Option<&'static str>,
    clear_action: Option<&'static str>,
    km: &ResolvedKeymap,
) -> (Vec<Shortcut>, Vec<Shortcut>) {
    let navigation = vec![NAV, TAB_PANE];

    let mut actions = Vec::new();
    if let Some(action) = enter_action {
        actions.push(enter(action));
    }
    if let Some(action) = clear_action {
        actions.push(Shortcut::from_keymap(
            km.lints.display_key_for(LintsAction::ClearHistory),
            action,
        ));
    }

    (navigation, actions)
}

fn project_list_groups(
    enter_action: Option<&'static str>,
    is_rust: bool,
    km: &ResolvedKeymap,
) -> (Vec<Shortcut>, Vec<Shortcut>) {
    let navigation = vec![NAV, ARROWS_EXPAND, EXPAND_COLLAPSE_ALL, TAB_PANE];

    let mut actions = Vec::new();
    if let Some(action) = enter_action {
        actions.push(enter(action));
    }
    if is_rust {
        actions.push(Shortcut::from_keymap(
            km.project_list.display_key_for(ProjectListAction::Clean),
            "clean",
        ));
    }
    (navigation, actions)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn global_shortcuts_include_terminal_between_editor_and_settings() {
        let groups = for_status_bar(
            InputContext::ProjectList,
            None,
            true,
            None,
            &ResolvedKeymap::defaults(),
            true,
            false,
        );
        let global_labels = groups
            .global
            .iter()
            .map(|shortcut| shortcut.description)
            .collect::<Vec<_>>();

        assert_eq!(
            &global_labels[..4],
            &["find", "editor", "terminal", "settings"]
        );
    }

    #[test]
    fn terminal_shortcut_is_disabled_when_command_is_unset() {
        let groups = for_status_bar(
            InputContext::ProjectList,
            None,
            true,
            None,
            &ResolvedKeymap::defaults(),
            false,
            false,
        );
        let terminal = groups
            .global
            .iter()
            .find(|shortcut| shortcut.description == "terminal")
            .expect("terminal shortcut");

        assert_eq!(terminal.state, ShortcutState::Disabled);
    }

    #[test]
    fn editor_and_terminal_are_disabled_when_selected_project_is_deleted() {
        let groups = for_status_bar(
            InputContext::ProjectList,
            None,
            true,
            None,
            &ResolvedKeymap::defaults(),
            true,
            true,
        );
        let editor = groups
            .global
            .iter()
            .find(|shortcut| shortcut.description == "editor")
            .expect("editor shortcut");
        let terminal = groups
            .global
            .iter()
            .find(|shortcut| shortcut.description == "terminal")
            .expect("terminal shortcut");

        assert_eq!(editor.state, ShortcutState::Disabled);
        assert_eq!(terminal.state, ShortcutState::Disabled);
    }

    #[test]
    fn ci_runs_shortcut_uses_branch_all_label() {
        let groups = for_status_bar(
            InputContext::CiRuns,
            None,
            true,
            None,
            &ResolvedKeymap::defaults(),
            true,
            false,
        );

        assert!(
            groups
                .actions
                .iter()
                .any(|shortcut| shortcut.description == "branch/all")
        );
    }

    #[test]
    fn settings_actions_include_editor_shortcut() {
        let groups = for_status_bar(
            InputContext::Settings,
            None,
            true,
            None,
            &ResolvedKeymap::defaults(),
            true,
            false,
        );

        assert!(
            groups
                .actions
                .iter()
                .any(|shortcut| shortcut.description == "editor")
        );
    }

    #[test]
    fn keymap_actions_include_editor_shortcut() {
        let groups = for_status_bar(
            InputContext::Keymap,
            None,
            true,
            None,
            &ResolvedKeymap::defaults(),
            true,
            false,
        );

        assert!(
            groups
                .actions
                .iter()
                .any(|shortcut| shortcut.description == "editor")
        );
    }
}
