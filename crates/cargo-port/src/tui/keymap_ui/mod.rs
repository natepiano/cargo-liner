//! Cargo-port app-side keymap-overlay orchestration: capture flow
//! command routing + TOML save.
//!
//! Rendering and row-building live in the framework's
//! [`tui_pane::KeymapPane::render_overlay`]; this module retains only
//! the cargo-port-specific orchestration: dispatching overlay
//! actions, navigation keys inside the popup, capture-command
//! routing, conflict detection against currently-bound rows, and the
//! TOML save / reload path.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use tui_pane::Action;
use tui_pane::KeyBind as FrameworkKeyBind;
use tui_pane::KeySequence;
use tui_pane::KeymapCaptureCommand;
use tui_pane::KeymapHelpRow;
use tui_pane::KeymapUiContext as _;
use tui_pane::OverlayAction;
use tui_pane::Shortcuts;

use super::app::App;
use super::integration::AppGlobalAction;
use super::integration::AppNavigation;
use super::integration::AppPaneId;
use super::integration::CiRunsPane;
use super::integration::FinderPane;
use super::integration::GitPane;
use super::integration::LintsPane;
use super::integration::NavigationAction;
use super::integration::OutputPane;
use super::integration::PackagePane;
use super::integration::ProjectListPane;
use super::integration::TargetsPane;
use super::keymap;

#[derive(Clone)]
struct PendingRebind {
    scope:  &'static str,
    action: &'static str,
    bind:   KeySequence,
}

pub(super) fn dispatch_keymap_action(action: OverlayAction, app: &mut App) {
    match action {
        OverlayAction::StartEdit => {
            app.overlays.clear_inline_error();
            app.framework.keymap_pane.enter_awaiting();
        },
        OverlayAction::Cancel => {
            app.overlays.clear_inline_error();
            app.framework.keymap_pane.enter_browse();
            app.close_framework_overlay_if_open();
        },
    }
}

pub(super) fn handle_keymap_navigation_key(app: &mut App, normalized: &KeyEvent) {
    match normalized.code {
        KeyCode::Up => app.framework.keymap_pane.viewport_mut().up(),
        KeyCode::Down => app.framework.keymap_pane.viewport_mut().down(),
        KeyCode::Home => app.framework.keymap_pane.viewport_mut().home(),
        KeyCode::End => {
            let last = selectable_row_count(app).saturating_sub(1);
            app.framework.keymap_pane.viewport_mut().set_pos(last);
        },
        KeyCode::PageUp => app.framework.keymap_pane.viewport_mut().page_up(),
        KeyCode::PageDown => app.framework.keymap_pane.viewport_mut().page_down(),
        KeyCode::Enter => {
            app.overlays.clear_inline_error();
            app.framework.keymap_pane.enter_awaiting();
        },
        _ => {},
    }
}

pub(super) fn handle_keymap_capture_command(app: &mut App, command: KeymapCaptureCommand) {
    match command {
        KeymapCaptureCommand::None => {},
        KeymapCaptureCommand::Cancel | KeymapCaptureCommand::ClearConflict => {
            app.overlays.clear_inline_error();
        },
        KeymapCaptureCommand::Captured(bind) => handle_captured_bind(app, bind),
    }
}

fn help_rows(app: &App) -> Vec<KeymapHelpRow> {
    let order = app.keymap_pane_display_order();
    app.framework_keymap.keymap_help_rows(order)
}

fn selectable_row_count(app: &App) -> usize {
    help_rows(app).iter().filter(|row| !row.is_header).count()
}

fn handle_captured_bind(app: &mut App, bind: FrameworkKeyBind) {
    let rows = help_rows(app);
    let selectable: Vec<&KeymapHelpRow> = rows.iter().filter(|r| !r.is_header).collect();
    let Some(row) = selectable
        .get(app.framework.keymap_pane.viewport().pos())
        .map(|row| (*row).clone())
    else {
        return;
    };

    // Check navigation reservation.
    if bind.mods == KeyModifiers::NONE
        && matches!(
            bind.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
        )
    {
        reject_capture(
            app,
            format!("\"{}\" reserved for navigation", bind.display()),
        );
        return;
    }

    // Check vim reservation.
    if app.config.navigation_keys().uses_vim()
        && bind.mods == KeyModifiers::NONE
        && matches!(bind.code, KeyCode::Char('h' | 'j' | 'k' | 'l'))
    {
        reject_capture(
            app,
            format!("\"{}\" reserved for vim navigation", bind.display()),
        );
        return;
    }

    // Check global conflict (if editing a non-global scope).
    if row.scope != "global"
        && let Some(msg) = check_global_conflict(&rows, &row, bind)
    {
        reject_capture(app, msg);
        return;
    }

    // Check non-global conflicts (if editing a global scope) — a
    // global key that shadows another scope would silently steal it.
    if row.scope == "global"
        && let Some(msg) = check_non_global_conflict(&rows, &row, bind)
    {
        reject_capture(app, msg);
        return;
    }

    // Check intra-scope conflict.
    if let Some(msg) = check_scope_conflict(&rows, &row, bind) {
        reject_capture(app, msg);
        return;
    }

    // Valid — apply the rebind.
    apply_rebind(app, row.scope, row.action, bind);
    app.overlays.clear_inline_error();
    app.framework.keymap_pane.enter_browse();
}

fn reject_capture(app: &mut App, message: String) {
    app.overlays.set_inline_error(message);
    app.framework.keymap_pane.enter_conflict();
}

fn check_global_conflict(
    rows: &[KeymapHelpRow],
    current: &KeymapHelpRow,
    bind: FrameworkKeyBind,
) -> Option<String> {
    find_conflict(rows, current, bind, |row| row.scope == "global")
}

fn check_non_global_conflict(
    rows: &[KeymapHelpRow],
    current: &KeymapHelpRow,
    bind: FrameworkKeyBind,
) -> Option<String> {
    find_conflict(rows, current, bind, |row| row.scope != "global")
}

fn check_scope_conflict(
    rows: &[KeymapHelpRow],
    current: &KeymapHelpRow,
    bind: FrameworkKeyBind,
) -> Option<String> {
    find_conflict(rows, current, bind, |row| row.scope == current.scope)
}

fn find_conflict(
    rows: &[KeymapHelpRow],
    current: &KeymapHelpRow,
    bind: FrameworkKeyBind,
    predicate: impl Fn(&KeymapHelpRow) -> bool,
) -> Option<String> {
    rows.iter()
        .filter(|row| !row.is_header)
        .filter(|row| predicate(row))
        .filter(|row| row.bind.as_ref().and_then(KeySequence::single_key) == Some(bind))
        .find(|row| row.scope != current.scope || row.action != current.action)
        .map(|row| {
            format!(
                "\"{}\" used by {} → {}",
                bind.display(),
                row.section,
                row.action,
            )
        })
}

fn apply_rebind(app: &mut App, scope: &'static str, action: &'static str, bind: FrameworkKeyBind) {
    let pending = PendingRebind {
        scope,
        action,
        bind: bind.into(),
    };
    save_keymap_to_disk(app, Some(&pending));
}

pub(super) fn save_current_keymap_to_disk(app: &mut App) { save_keymap_to_disk(app, None); }

fn save_keymap_to_disk(app: &mut App, pending: Option<&PendingRebind>) {
    let Some(path) = app.keymap.path() else {
        return;
    };
    let content = current_keymap_toml_with_pending(app, pending);
    // TODO(toml_edit): use toml_edit for targeted updates preserving comments.
    let _ = std::fs::write(path, &content);
    let legacy = keymap::load_keymap_from_str(&content, app.config.current().tui.navigation_keys);
    app.keymap.replace_current(legacy.keymap);
    app.keymap.sync_stamp();
    if let Err(err) = app.rebuild_framework_keymap_from_disk() {
        app.show_timed_toast("Keymap reload failed", err);
    }
}

#[cfg(test)]
pub(super) fn current_keymap_toml(app: &App) -> String {
    current_keymap_toml_with_pending(app, None)
}

fn current_keymap_toml_with_pending(app: &App, pending: Option<&PendingRebind>) -> String {
    use std::fmt::Write as _;
    let mut out = String::from(
        "# cargo-port keymap configuration\n\
         # Edit bindings below. Format: action = \"key\" or \"modifier-key\"\n\
         # Modifiers: ctrl, alt, shift.  Examples: \"ctrl-r\", \"shift-tab\", \"q\"\n\
         # Chord steps are space-separated, e.g. \"g g\".\n\
         # Note: when vim navigation is enabled, vim navigation keys are reserved\n\
         #       for navigation and cannot be used as action keys.\n\n",
    );

    let order = app.keymap_pane_display_order();
    let sections = app.framework_keymap.keymap_toml_scope_keys(order);
    for (scope, action_keys) in sections {
        let _ = writeln!(out, "[{scope}]");
        let mut entries: Vec<(&'static str, Vec<KeySequence>)> = action_keys
            .into_iter()
            .map(|action_key| {
                let binds = binds_for_scope_action(app, scope, action_key);
                (action_key, binds)
            })
            .collect();
        entries.sort_by_key(|(name, _)| *name);
        let max_len = entries
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0);
        for (action_key, binds) in &entries {
            let value = pending
                .filter(|pending| pending.scope == scope && pending.action == *action_key)
                .map_or_else(
                    || keybind_toml_value(binds),
                    |pending| keybind_toml_value(std::slice::from_ref(&pending.bind)),
                );
            let _ = writeln!(out, "{action_key:<max_len$} = {value}");
        }
        out.push('\n');
    }
    if out.ends_with("\n\n") {
        out.pop();
    }

    out
}

fn binds_for_scope_action(app: &App, scope: &str, action_key: &str) -> Vec<KeySequence> {
    let keymap = &*app.framework_keymap;
    if scope == "global" {
        // Framework + app globals share the [global] table. Try
        // framework globals first; if that has the action, return its
        // bind. Otherwise look for the action in app globals via the
        // help-rows walk (which lists app globals under "Global
        // Shortcuts").
        if let Some(action) = tui_pane::GlobalAction::from_toml_key(action_key)
            && let Some(bind) = keymap.framework_globals().key_for(action)
        {
            return vec![bind.clone()];
        }
        return app_global_binds_for_action(app, action_key);
    }
    if scope == "navigation" {
        return navigation_binds_for_action(app, action_key);
    }
    if scope == "overlay" {
        if let Some(action) = tui_pane::OverlayAction::from_toml_key(action_key) {
            return keymap.overlay().display_keys_for(action).to_vec();
        }
        return Vec::new();
    }
    // App-pane scope. Resolve via the runtime lookup keyed by toml
    // action key.
    let order = app.keymap_pane_display_order();
    for id in order {
        if let Some(name) = keymap_scope_name(app, *id)
            && name == scope
        {
            return keymap.keys_for_toml_key(*id, action_key);
        }
    }
    Vec::new()
}

const fn keymap_scope_name(_app: &App, app_pane_id: AppPaneId) -> Option<&'static str> {
    Some(match app_pane_id {
        AppPaneId::ProjectList => <ProjectListPane as Shortcuts<App>>::SCOPE_NAME,
        AppPaneId::Package => <PackagePane as Shortcuts<App>>::SCOPE_NAME,
        AppPaneId::Git => <GitPane as Shortcuts<App>>::SCOPE_NAME,
        AppPaneId::Targets => <TargetsPane as Shortcuts<App>>::SCOPE_NAME,
        AppPaneId::CiRuns => <CiRunsPane as Shortcuts<App>>::SCOPE_NAME,
        AppPaneId::Lints => <LintsPane as Shortcuts<App>>::SCOPE_NAME,
        AppPaneId::Output => <OutputPane as Shortcuts<App>>::SCOPE_NAME,
        AppPaneId::Finder => <FinderPane as Shortcuts<App>>::SCOPE_NAME,
        AppPaneId::Lang | AppPaneId::Cpu => return None,
    })
}

fn app_global_binds_for_action(app: &App, action_key: &str) -> Vec<KeySequence> {
    if let Some(action) = AppGlobalAction::from_toml_key(action_key)
        && let Some(scope) = app.framework_keymap.globals::<AppGlobalAction>()
    {
        return scope.display_keys_for(action).to_vec();
    }
    Vec::new()
}

fn navigation_binds_for_action(app: &App, action_key: &str) -> Vec<KeySequence> {
    if let Some(action) = NavigationAction::from_toml_key(action_key)
        && let Some(scope) = app.framework_keymap.navigation::<AppNavigation>()
    {
        return scope.display_keys_for(action).to_vec();
    }
    Vec::new()
}

fn keybind_toml_value(binds: &[KeySequence]) -> String {
    match binds {
        [] => "\"\"".to_string(),
        [bind] => format!("\"{}\"", bind.display()),
        _ => {
            let values = binds
                .iter()
                .map(|bind| format!("\"{}\"", bind.display()))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{values}]")
        },
    }
}

pub(super) fn vim_mode_conflicts(app: &App) -> Vec<String> {
    help_rows(app)
        .into_iter()
        .filter(|row| !row.is_header)
        .filter_map(|row| {
            let bind = row.bind?.single_key()?;
            (bind.mods == KeyModifiers::NONE
                && matches!(bind.code, KeyCode::Char('h' | 'j' | 'k' | 'l')))
            .then(|| format!("{}.{}", row.scope, row.action))
        })
        .collect()
}
