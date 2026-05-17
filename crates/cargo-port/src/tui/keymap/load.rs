#[cfg(test)]
use std::cell::RefCell;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use toml::Table;
use toml::Value;
use tui_pane::Action;

use super::KeyBind;
use super::ScopeMap;
use super::actions::CiRunsAction;
use super::actions::GitAction;
use super::actions::LintsAction;
use super::actions::PackageAction;
use super::actions::ProjectListAction;
use super::actions::TargetsAction;
use super::actions::action_from_toml_key;
use super::actions::action_toml_key;
use super::resolved::ResolvedKeymap;
use crate::config::NavigationKeys;
use crate::constants::APP_NAME;
use crate::constants::KEYMAP_FILE;
use crate::project::AbsolutePath;

const REMOVED_PROJECT_LIST_GLOBAL_ACTIONS: [(&str, &str); 2] =
    [("open_editor", "open_editor"), ("rescan", "rescan")];

pub(crate) struct KeymapLoadResult {
    pub(crate) keymap:          ResolvedKeymap,
    pub(crate) errors:          Vec<KeymapError>,
    pub(crate) missing_actions: Vec<String>,
}

pub(crate) struct KeymapError {
    pub(crate) scope:  String,
    pub(crate) action: String,
    pub(crate) key:    String,
    pub(crate) reason: KeymapErrorReason,
}

impl Display for KeymapError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}: \"{}\" — {}",
            self.scope, self.action, self.key, self.reason
        )
    }
}

pub(crate) enum KeymapErrorReason {
    Parse(String),
    ConflictWithinScope(String),
    ReservedForVimMode,
    UnknownAction,
}

impl Display for KeymapErrorReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::ConflictWithinScope(action) => write!(f, "conflicts with {action}"),
            Self::ReservedForVimMode => write!(f, "reserved for vim navigation"),
            Self::UnknownAction => write!(f, "unknown action (ignored)"),
        }
    }
}

/// Path to the keymap config file.
pub(crate) fn keymap_path() -> Option<AbsolutePath> {
    #[cfg(test)]
    if let Some(path) = KEYMAP_PATH_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return Some(path.into());
    }

    dirs::config_dir().map(|d| d.join(APP_NAME).join(KEYMAP_FILE).into())
}

#[cfg(test)]
thread_local! {
    static KEYMAP_PATH_OVERRIDE: RefCell<Option<PathBuf>> = const {
        RefCell::new(None)
    };
}

#[cfg(test)]
pub(crate) struct KeymapPathOverrideGuard {
    previous: Option<PathBuf>,
    active:   bool,
}

#[cfg(test)]
impl Drop for KeymapPathOverrideGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let previous = self.previous.take();
        KEYMAP_PATH_OVERRIDE.with(|slot| {
            *slot.borrow_mut() = previous;
        });
    }
}

#[cfg(test)]
pub(crate) fn override_keymap_path_for_test(path: PathBuf) -> KeymapPathOverrideGuard {
    let previous = KEYMAP_PATH_OVERRIDE.with(|slot| slot.replace(Some(path)));
    KeymapPathOverrideGuard {
        previous,
        active: true,
    }
}

/// Set the keymap path override only if no override is already
/// active. Returned guard is a no-op when an override existed; the
/// caller's outer override stays in effect. Lets `make_app` provide
/// a hermetic-default fallback that test helpers like
/// `make_app_with_keymap_toml` can layer on top of without being
/// clobbered.
#[cfg(test)]
pub(crate) fn override_keymap_path_for_test_if_absent(path: PathBuf) -> KeymapPathOverrideGuard {
    let already_set = KEYMAP_PATH_OVERRIDE.with(|slot| slot.borrow().is_some());
    if already_set {
        KeymapPathOverrideGuard {
            previous: None,
            active:   false,
        }
    } else {
        override_keymap_path_for_test(path)
    }
}

pub(crate) fn migrate_removed_action_keys_on_disk(path: &Path) -> std::io::Result<()> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let Ok(mut table) = contents.parse::<Table>() else {
        return Ok(());
    };
    if migrate_removed_action_keys(&mut table) {
        std::fs::write(path, table.to_string())?;
    }
    Ok(())
}

/// Load and validate keymap from disk. Creates the default file if missing.
pub(crate) fn load_keymap(vim_mode: NavigationKeys) -> KeymapLoadResult {
    let Some(path) = keymap_path() else {
        return KeymapLoadResult {
            keymap:          ResolvedKeymap::defaults(),
            errors:          Vec::new(),
            missing_actions: Vec::new(),
        };
    };

    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, ResolvedKeymap::default_toml());
        return KeymapLoadResult {
            keymap:          ResolvedKeymap::defaults(),
            errors:          Vec::new(),
            missing_actions: Vec::new(),
        };
    }

    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            return KeymapLoadResult {
                keymap:          ResolvedKeymap::defaults(),
                errors:          vec![KeymapError {
                    scope:  String::new(),
                    action: String::new(),
                    key:    String::new(),
                    reason: KeymapErrorReason::Parse(format!("read error: {e}")),
                }],
                missing_actions: Vec::new(),
            };
        },
    };

    let mut table: Table = match contents.parse() {
        Ok(t) => t,
        Err(e) => {
            return KeymapLoadResult {
                keymap:          ResolvedKeymap::defaults(),
                errors:          vec![KeymapError {
                    scope:  String::new(),
                    action: String::new(),
                    key:    String::new(),
                    reason: KeymapErrorReason::Parse(format!("TOML parse error: {e}")),
                }],
                missing_actions: Vec::new(),
            };
        },
    };

    let migrated = migrate_removed_action_keys(&mut table);
    let result = resolve_from_table(&table, vim_mode);

    // Backfill missing entries into the file.
    if !result.missing_actions.is_empty() {
        let content = ResolvedKeymap::default_toml_from(&result.keymap);
        let _ = std::fs::write(&path, content);
    } else if migrated {
        let _ = std::fs::write(&path, table.to_string());
    }

    result
}

/// Load keymap from a TOML string (for testing and hot-reload).
pub(crate) fn load_keymap_from_str(toml_str: &str, vim_mode: NavigationKeys) -> KeymapLoadResult {
    let mut table: Table = match toml_str.parse() {
        Ok(t) => t,
        Err(e) => {
            return KeymapLoadResult {
                keymap:          ResolvedKeymap::defaults(),
                errors:          vec![KeymapError {
                    scope:  String::new(),
                    action: String::new(),
                    key:    String::new(),
                    reason: KeymapErrorReason::Parse(format!("TOML parse error: {e}")),
                }],
                missing_actions: Vec::new(),
            };
        },
    };
    migrate_removed_action_keys(&mut table);
    resolve_from_table(&table, vim_mode)
}

/// Check whether enabling vim mode would conflict with current keymap bindings.
/// Returns the list of conflicting bindings (scope.action = key).
#[cfg(test)]
fn vim_mode_conflicts(keymap: &ResolvedKeymap) -> Vec<String> {
    fn check_scope<A: Copy + Eq + std::hash::Hash>(
        scope_name: &str,
        scope: &ScopeMap<A>,
        vim_keys: &[KeyCode; 4],
        toml_key: fn(A) -> &'static str,
        conflicts: &mut Vec<String>,
    ) {
        for (bind, &action) in &scope.by_key {
            if bind.modifiers == KeyModifiers::NONE && vim_keys.contains(&bind.code) {
                conflicts.push(format!("{scope_name}.{}", toml_key(action)));
            }
        }
    }

    let vim_keys: [KeyCode; 4] = [
        KeyCode::Char('h'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('l'),
    ];
    let mut conflicts = Vec::new();

    check_scope(
        "project_list",
        &keymap.project_list,
        &vim_keys,
        action_toml_key::<ProjectListAction>,
        &mut conflicts,
    );
    check_scope(
        "package",
        &keymap.package,
        &vim_keys,
        action_toml_key::<PackageAction>,
        &mut conflicts,
    );
    check_scope(
        "git",
        &keymap.git,
        &vim_keys,
        action_toml_key::<GitAction>,
        &mut conflicts,
    );
    check_scope(
        "targets",
        &keymap.targets,
        &vim_keys,
        action_toml_key::<TargetsAction>,
        &mut conflicts,
    );
    check_scope(
        "ci_runs",
        &keymap.ci_runs,
        &vim_keys,
        action_toml_key::<CiRunsAction>,
        &mut conflicts,
    );
    check_scope(
        "lints",
        &keymap.lints,
        &vim_keys,
        action_toml_key::<LintsAction>,
        &mut conflicts,
    );

    conflicts
}

// ── Internal resolution ──────────────────────────────────────────────

fn is_vim_reserved(bind: &KeyBind, vim_mode: NavigationKeys) -> bool {
    vim_mode.uses_vim()
        && bind.modifiers == KeyModifiers::NONE
        && matches!(bind.code, KeyCode::Char('h' | 'j' | 'k' | 'l'))
}

fn migrate_removed_action_keys(table: &mut Table) -> bool {
    if matches!(table.get("global"), Some(value) if !value.is_table()) {
        return false;
    }

    let Some(project_list) = table
        .get_mut("project_list")
        .and_then(toml::Value::as_table_mut)
    else {
        return false;
    };

    let mut removed = Vec::new();
    for (old_key, global_key) in REMOVED_PROJECT_LIST_GLOBAL_ACTIONS {
        if let Some(value) = project_list.remove(old_key) {
            removed.push((global_key, value));
        }
    }
    if removed.is_empty() {
        return false;
    }

    if !table.contains_key("global") {
        table.insert("global".to_string(), Value::Table(Table::new()));
    }
    let Some(global) = table.get_mut("global").and_then(toml::Value::as_table_mut) else {
        return false;
    };
    for (global_key, value) in removed {
        if !global.contains_key(global_key) {
            global.insert(global_key.to_string(), value);
        }
    }

    true
}

fn resolve_from_table(table: &Table, vim_mode: NavigationKeys) -> KeymapLoadResult {
    let defaults = ResolvedKeymap::defaults();
    let mut keymap = ResolvedKeymap::default();
    let mut errors = Vec::new();
    let mut missing_actions = Vec::new();

    let mut ctx = ScopeResolveContext {
        table,
        errors: &mut errors,
        missing_actions: &mut missing_actions,
        vim_mode,
    };
    resolve_pane_scopes(&mut ctx, &defaults, &mut keymap);

    KeymapLoadResult {
        keymap,
        errors,
        missing_actions,
    }
}

fn resolve_pane_scopes(
    ctx: &mut ScopeResolveContext<'_>,
    defaults: &ResolvedKeymap,
    keymap: &mut ResolvedKeymap,
) {
    resolve_scope(
        ctx,
        "project_list",
        <ProjectListAction as Action>::ALL,
        action_from_toml_key::<ProjectListAction>,
        action_toml_key::<ProjectListAction>,
        &defaults.project_list,
        &mut keymap.project_list,
    );
    resolve_scope(
        ctx,
        "package",
        <PackageAction as Action>::ALL,
        action_from_toml_key::<PackageAction>,
        action_toml_key::<PackageAction>,
        &defaults.package,
        &mut keymap.package,
    );
    resolve_scope(
        ctx,
        "git",
        <GitAction as Action>::ALL,
        action_from_toml_key::<GitAction>,
        action_toml_key::<GitAction>,
        &defaults.git,
        &mut keymap.git,
    );
    resolve_scope(
        ctx,
        "targets",
        <TargetsAction as Action>::ALL,
        action_from_toml_key::<TargetsAction>,
        action_toml_key::<TargetsAction>,
        &defaults.targets,
        &mut keymap.targets,
    );
    resolve_scope(
        ctx,
        "ci_runs",
        <CiRunsAction as Action>::ALL,
        action_from_toml_key::<CiRunsAction>,
        action_toml_key::<CiRunsAction>,
        &defaults.ci_runs,
        &mut keymap.ci_runs,
    );
    resolve_scope(
        ctx,
        "lints",
        <LintsAction as Action>::ALL,
        action_from_toml_key::<LintsAction>,
        action_toml_key::<LintsAction>,
        &defaults.lints,
        &mut keymap.lints,
    );
}

struct ScopeResolveContext<'a> {
    table:           &'a Table,
    errors:          &'a mut Vec<KeymapError>,
    missing_actions: &'a mut Vec<String>,
    vim_mode:        NavigationKeys,
}

fn resolve_scope<A: Copy + Eq + std::hash::Hash>(
    ctx: &mut ScopeResolveContext<'_>,
    scope_name: &str,
    all_actions: &[A],
    from_toml_key: fn(&str) -> Option<A>,
    to_toml_key: fn(A) -> &'static str,
    defaults: &ScopeMap<A>,
    target: &mut ScopeMap<A>,
) {
    let scope_table = ctx.table.get(scope_name).and_then(toml::Value::as_table);

    // Report unknown keys in this scope.
    if let Some(st) = scope_table {
        for key in st.keys() {
            if from_toml_key(key).is_none() {
                ctx.errors.push(KeymapError {
                    scope:  scope_name.to_string(),
                    action: key.clone(),
                    key:    st.get(key).map_or_else(String::new, keymap_value_string),
                    reason: KeymapErrorReason::UnknownAction,
                });
            }
        }
    }

    // Resolve each action.
    for &action in all_actions {
        let toml_key = to_toml_key(action);
        let raw_value = scope_table
            .and_then(|st| st.get(toml_key))
            .and_then(toml::Value::as_str);

        let bind_result = raw_value.map(str::parse::<KeyBind>);

        let (bind, error) = match bind_result {
            Some(Ok(bind)) => {
                // Validate the parsed binding.
                if is_vim_reserved(&bind, ctx.vim_mode) {
                    (None, Some(KeymapErrorReason::ReservedForVimMode))
                } else if let Some(&existing) = target.by_key.get(&bind) {
                    (
                        None,
                        Some(KeymapErrorReason::ConflictWithinScope(
                            to_toml_key(existing).to_string(),
                        )),
                    )
                } else {
                    (Some(bind), None)
                }
            },
            Some(Err(e)) => (None, Some(KeymapErrorReason::Parse(e))),
            None => {
                // Key missing from TOML — record and use default.
                ctx.missing_actions.push(format!("{scope_name}.{toml_key}"));
                (None, None)
            },
        };

        if let Some(reason) = error {
            ctx.errors.push(KeymapError {
                scope: scope_name.to_string(),
                action: toml_key.to_string(),
                key: raw_value.unwrap_or("").to_string(),
                reason,
            });
        }

        if let Some(bind) = bind {
            target.insert(bind, action);
        } else {
            // Fall back to default binding.
            if let Some(default_bind) = defaults.key_for(action) {
                target.insert(default_bind.clone(), action);
            }
        }
    }
}

fn keymap_value_string(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), ToOwned::to_owned)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn default_toml_loads_without_errors() {
        let toml_str = ResolvedKeymap::default_toml();
        let result = load_keymap_from_str(&toml_str, NavigationKeys::ArrowsOnly);
        assert!(
            result.errors.is_empty(),
            "errors: {:?}",
            result
                .errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn pane_scope_conflict_detected() {
        let toml = r#"
[project_list]
expand_all = "c"
clean = "c"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e.reason, KeymapErrorReason::ConflictWithinScope(_))),
            "expected intra-scope conflict for duplicate 'c'"
        );
    }

    #[test]
    fn cross_scope_same_key_is_ok() {
        let toml = r#"
[global]
quit = "q"
restart = "Shift+r"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
open_keymap = "Ctrl+k"

[project_list]
clean = "c"

[ci_runs]
clear_cache = "d"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        assert!(
            !result
                .errors
                .iter()
                .any(|e| !matches!(e.reason, KeymapErrorReason::UnknownAction)),
            "unexpected errors"
        );
    }

    #[test]
    fn vim_mode_reservation() {
        let toml = r#"
[global]
quit = "q"
restart = "Shift+r"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
open_keymap = "Ctrl+k"

[project_list]
clean = "h"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsAndVim);
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e.reason, KeymapErrorReason::ReservedForVimMode)),
            "expected vim reservation error for 'h'"
        );
    }

    #[test]
    fn vim_mode_allows_modified_hjkl() {
        let toml = r#"
[global]
quit = "q"
restart = "Shift+r"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
focus_list = "Esc"
open_keymap = "Ctrl+h"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsAndVim);
        assert!(
            !result
                .errors
                .iter()
                .any(|e| matches!(e.reason, KeymapErrorReason::ReservedForVimMode)),
            "Ctrl+h should be allowed even with vim mode"
        );
    }

    #[test]
    fn unknown_action_reported() {
        let toml = r#"
[project_list]
claen = "c"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        let unknown: Vec<_> = result
            .errors
            .iter()
            .filter(|e| matches!(e.reason, KeymapErrorReason::UnknownAction))
            .collect();
        assert!(
            !unknown.is_empty(),
            "expected unknown action for typo 'claen'"
        );
        assert_eq!(unknown[0].action, "claen");
        assert_eq!(unknown[0].key, "c");
        assert_eq!(
            unknown[0].to_string(),
            "project_list.claen: \"c\" — unknown action (ignored)",
        );
    }

    #[test]
    fn legacy_project_list_removed_actions_move_to_global_table_before_validation() {
        let mut table: Table = r#"
[global]
quit = "q"
restart = "R"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
open_keymap = "Ctrl+k"
dismiss = "x"

[project_list]
open_editor = "Enter"
rescan = "Ctrl+r"
expand_all = "="
collapse_all = "-"
clean = "c"
"#
        .parse()
        .unwrap();

        assert!(migrate_removed_action_keys(&mut table));

        let project_list = table.get("project_list").and_then(Value::as_table).unwrap();
        assert!(!project_list.contains_key("open_editor"));
        assert!(!project_list.contains_key("rescan"));
        let global = table.get("global").and_then(Value::as_table).unwrap();
        assert_eq!(
            global.get("open_editor").and_then(Value::as_str),
            Some("Enter"),
        );
        assert_eq!(global.get("rescan").and_then(Value::as_str), Some("Ctrl+r"),);

        let result = resolve_from_table(&table, NavigationKeys::ArrowsOnly);
        assert!(
            result
                .errors
                .iter()
                .all(|e| !matches!(e.reason, KeymapErrorReason::UnknownAction)),
            "migrated removed actions should not be reported as unknown: {:?}",
            result
                .errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn legacy_project_list_removed_action_does_not_override_global_value() {
        let mut table: Table = r#"
[global]
open_editor = "E"

[project_list]
open_editor = "Enter"
clean = "c"
"#
        .parse()
        .unwrap();

        assert!(migrate_removed_action_keys(&mut table));

        let project_list = table.get("project_list").and_then(Value::as_table).unwrap();
        assert!(!project_list.contains_key("open_editor"));
        let global = table.get("global").and_then(Value::as_table).unwrap();
        assert_eq!(global.get("open_editor").and_then(Value::as_str), Some("E"),);
    }

    #[test]
    fn partial_acceptance_valid_bindings_applied() {
        let toml = r#"
[project_list]
expand_all = "x"
collapse_all = "x"
clean = "c"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        // expand_all = "x" should be accepted.
        assert_eq!(
            result
                .keymap
                .project_list
                .key_for(ProjectListAction::ExpandAll),
            Some(&KeyBind::plain(KeyCode::Char('x')))
        );
        // collapse_all = "x" conflicts with expand_all, so it falls back.
        assert!(
            result
                .keymap
                .project_list
                .key_for(ProjectListAction::CollapseAll)
                .is_some(),
            "collapse_all should have a fallback binding"
        );
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn malformed_toml_returns_defaults() {
        let result = load_keymap_from_str("{{invalid toml", NavigationKeys::ArrowsOnly);
        assert!(!result.errors.is_empty());
        // Should have defaults for all actions.
        assert!(
            result
                .keymap
                .project_list
                .key_for(ProjectListAction::Clean)
                .is_some()
        );
    }

    #[test]
    fn vim_mode_conflicts_detected() {
        let defaults = ResolvedKeymap::defaults();
        let conflicts = vim_mode_conflicts(&defaults);
        // Default keymap doesn't use bare hjkl.
        assert!(conflicts.is_empty());

        // Build a keymap with 'h' bound.
        let toml = r#"
[package]
activate = "Enter"
clean = "h"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        let conflicts = vim_mode_conflicts(&result.keymap);
        assert!(!conflicts.is_empty(), "expected conflict for 'h' binding");
    }

    #[test]
    fn action_description_and_display_key() {
        let km = ResolvedKeymap::defaults();
        assert_eq!(
            <ProjectListAction as tui_pane::Action>::description(ProjectListAction::Clean),
            "Clean project"
        );
        assert_eq!(
            km.project_list.display_key_for(ProjectListAction::Clean),
            "c"
        );
        assert_eq!(km.ci_runs.display_key_for(CiRunsAction::ToggleView), "b");
    }

    #[test]
    fn legacy_loader_no_longer_checks_global_conflicts() {
        let toml = r#"
[global]
quit = "q"
restart = "R"
find = "/"
open_editor = "e"
open_terminal = "t"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
dismiss = "x"
open_keymap = "Ctrl+k"

[ci_runs]
activate = "Enter"
toggle_view = "t"
clear_cache = "d"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);

        assert!(
            result.errors.is_empty(),
            "legacy loader should ignore framework-owned globals: {:?}",
            result
                .errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            result.keymap.ci_runs.key_for(CiRunsAction::ToggleView),
            Some(&KeyBind::plain(KeyCode::Char('t')))
        );
    }

    #[test]
    fn missing_action_detected() {
        // Omit `clean` from package — should appear in missing_actions.
        let toml = r#"
[package]
activate = "Enter"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        assert!(
            result.missing_actions.iter().any(|m| m == "package.clean"),
            "expected package.clean in missing_actions: {:?}",
            result.missing_actions
        );
        // Default should still be applied.
        assert_eq!(
            result.keymap.package.key_for(PackageAction::Clean),
            Some(&KeyBind::plain(KeyCode::Char('c')))
        );
    }

    #[test]
    fn complete_keymap_has_no_missing() {
        let toml_str = ResolvedKeymap::default_toml();
        let result = load_keymap_from_str(&toml_str, NavigationKeys::ArrowsOnly);
        assert!(
            result.missing_actions.is_empty(),
            "default toml should have no missing actions: {:?}",
            result.missing_actions
        );
    }

    #[test]
    fn missing_entire_scope_detected() {
        // No [lints] section at all — its actions should appear in missing.
        let toml = r#"
[global]
quit = "q"
restart = "R"
find = "/"
settings = "s"
next_pane = "Tab"
prev_pane = "Shift+Tab"
dismiss = "x"
open_keymap = "Ctrl+k"
"#;
        let result = load_keymap_from_str(toml, NavigationKeys::ArrowsOnly);
        assert!(
            result
                .missing_actions
                .iter()
                .any(|m| m.starts_with("lints.")),
            "expected lints actions in missing: {:?}",
            result.missing_actions
        );
    }
}
