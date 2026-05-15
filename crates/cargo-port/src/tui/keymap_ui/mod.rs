mod view;

use std::fmt::Write as _;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::text::Line;
use tui_pane::Action;
use tui_pane::GlobalAction as FrameworkGlobalAction;
use tui_pane::KeyBind as FrameworkKeyBind;
use tui_pane::KeymapCaptureCommand;
use tui_pane::KeymapPaneAction;
use tui_pane::Pane;
#[cfg(test)]
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::ScopeMap as FrameworkScopeMap;
use tui_pane::SettingsPaneAction;
use view::KeymapLines;
pub(super) use view::render_keymap_pane_body;

use super::app::App;
use super::integration::AppGlobalAction;
use super::integration::AppNavigation;
use super::integration::AppPaneId;
use super::integration::CiRunsPane;
use super::integration::CpuAction;
use super::integration::CpuPane;
use super::integration::FinderPane;
use super::integration::GitPane;
use super::integration::LangAction;
use super::integration::LangPane;
use super::integration::LintsPane;
use super::integration::NavigationAction;
use super::integration::OutputPane;
use super::integration::PackagePane;
use super::integration::ProjectListPane;
use super::integration::TargetsPane;
use crate::keymap;
use crate::keymap::CiRunsAction;
use crate::keymap::FinderAction;
use crate::keymap::GitAction;
use crate::keymap::LintsAction;
use crate::keymap::OutputAction;
use crate::keymap::PackageAction;
use crate::keymap::ProjectListAction;
use crate::keymap::TargetsAction;

// ── Row model ────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct PendingRebind {
    scope:  &'static str,
    action: &'static str,
    bind:   FrameworkKeyBind,
}

#[derive(Clone)]
struct KeymapRow {
    section:     &'static str,
    scope:       &'static str,
    action:      &'static str,
    description: &'static str,
    key_display: String,
    bind:        Option<FrameworkKeyBind>,
    is_header:   bool,
}

const fn header(section: &'static str) -> KeymapRow {
    KeymapRow {
        section,
        scope: "",
        action: "",
        description: "",
        key_display: String::new(),
        bind: None,
        is_header: true,
    }
}

fn bind_display(bind: Option<FrameworkKeyBind>) -> String {
    bind.map_or_else(String::new, |key| key.display())
}

fn action_toml_key<A: Action>(action: A) -> &'static str { action.toml_key() }

fn action_row<A: Action>(
    section: &'static str,
    scope: &'static str,
    action: A,
    toml_key: &'static str,
    bind: Option<FrameworkKeyBind>,
) -> KeymapRow {
    action_row_with_description(section, scope, action.description(), toml_key, bind)
}

fn action_row_with_description(
    section: &'static str,
    scope: &'static str,
    description: &'static str,
    toml_key: &'static str,
    bind: Option<FrameworkKeyBind>,
) -> KeymapRow {
    KeymapRow {
        section,
        scope,
        action: toml_key,
        description,
        key_display: bind_display(bind),
        bind,
        is_header: false,
    }
}

fn push_scope<A: Action>(
    rows: &mut Vec<KeymapRow>,
    section: &'static str,
    scope: &'static str,
    actions: &[A],
    scope_map: &FrameworkScopeMap<A>,
    toml_key: fn(A) -> &'static str,
) {
    rows.push(header(section));
    let mut section: Vec<KeymapRow> = actions
        .iter()
        .map(|&action| {
            let bind = scope_map.key_for(action).copied();
            action_row(section, scope, action, toml_key(action), bind)
        })
        .collect();
    section.sort_by_key(|row| row.description);
    rows.extend(section);
}

fn push_app_pane_scope<A: Action>(
    rows: &mut Vec<KeymapRow>,
    section: &'static str,
    scope: &'static str,
    app_pane_id: AppPaneId,
    actions: &[A],
    app: &App,
) {
    rows.push(header(section));
    let mut section_rows: Vec<KeymapRow> = actions
        .iter()
        .map(|&action| {
            let toml_key = action.toml_key();
            let bind = app.framework_keymap.key_for_toml_key(app_pane_id, toml_key);
            action_row(section, scope, action, toml_key, bind)
        })
        .collect();
    sort_app_pane_rows(section, &mut section_rows);
    rows.extend(section_rows);
}

fn sort_app_pane_rows(section: &'static str, rows: &mut [KeymapRow]) {
    if section == "Project List" {
        rows.sort_by_key(project_list_keymap_sort_key);
    } else {
        rows.sort_by_key(|row| row.description);
    }
}

fn project_list_keymap_sort_key(row: &KeymapRow) -> (u8, &'static str) {
    match row.action {
        "clean" => (0, row.description),
        "collapse_all" => (1, row.description),
        "expand_all" => (2, row.description),
        "collapse_row" => (3, row.description),
        "expand_row" => (4, row.description),
        _ => (5, row.description),
    }
}

fn framework_global_toml_key(action: FrameworkGlobalAction) -> &'static str {
    match action {
        FrameworkGlobalAction::OpenSettings => "settings",
        _ => action.toml_key(),
    }
}

const GLOBAL_NAV: &[FrameworkGlobalAction] = &[
    FrameworkGlobalAction::NextPane,
    FrameworkGlobalAction::PrevPane,
];
const GLOBAL_SHORTCUTS: &[FrameworkGlobalAction] = &[
    FrameworkGlobalAction::Dismiss,
    FrameworkGlobalAction::OpenKeymap,
    FrameworkGlobalAction::OpenSettings,
    FrameworkGlobalAction::Quit,
    FrameworkGlobalAction::Restart,
];

/// Precomputed render inputs for the Keymap overlay. Built in
/// `prepare_keymap_render_inputs` while we still hold `&App`, then
/// stashed on [`crate::tui::pane::PaneRenderCtx::keymap_render_inputs`]
/// for `KeymapPane`'s `Renderable::render` impl to consume from
/// `&mut self` + `&PaneRenderCtx` without further `&App` access.
pub(crate) struct KeymapRenderInputs {
    pub lines:          Vec<Line<'static>>,
    pub line_targets:   Vec<Option<usize>>,
    pub selectable_len: usize,
    pub content_width:  u16,
}

/// Build the [`KeymapRenderInputs`] the overlay's body fn reads.
/// Called from `render::ui` before `App::split_for_render` runs,
/// so the still-current `&App` borrow can walk
/// `app.framework_keymap` to enumerate rows.
pub(super) fn prepare_keymap_render_inputs(app: &App) -> KeymapRenderInputs {
    let rows = build_rows(app);
    let is_capturing = app.framework.keymap_pane.is_capturing();
    let KeymapLines {
        lines,
        line_targets,
    } = view::build_lines(&rows, app, is_capturing);
    let selectable_len = selectable_row_count(app);
    let content_width = app.overlays.inline_error().map_or(BASE_POPUP_WIDTH, |msg| {
        // 2 indent + 25 desc + msg len + 2 pad
        let needed = u16::try_from(2 + 25 + msg.len() + 2).unwrap_or(u16::MAX);
        BASE_POPUP_WIDTH.max(needed)
    });
    KeymapRenderInputs {
        lines,
        line_targets,
        selectable_len,
        content_width,
    }
}

fn build_rows(app: &App) -> Vec<KeymapRow> {
    let mut rows = Vec::new();
    push_global_rows(&mut rows, app);
    push_navigation_rows(&mut rows, app);
    push_app_pane_rows(&mut rows, app);
    push_overlay_rows(&mut rows, app);
    rows
}

fn push_global_rows(rows: &mut Vec<KeymapRow>, app: &App) {
    let framework_globals = app.framework_keymap.framework_globals();
    rows.push(header("Global Navigation"));
    let mut nav_rows: Vec<KeymapRow> = GLOBAL_NAV
        .iter()
        .copied()
        .map(|action| framework_global_row(action, framework_globals))
        .collect();
    nav_rows.sort_by_key(|row| row.description);
    rows.extend(nav_rows);

    rows.push(header("Global Shortcuts"));
    let mut shortcut_rows: Vec<KeymapRow> = GLOBAL_SHORTCUTS
        .iter()
        .copied()
        .map(|action| framework_global_row(action, framework_globals))
        .collect();
    if let Some(scope) = app.framework_keymap.globals::<AppGlobalAction>() {
        shortcut_rows.extend(AppGlobalAction::ALL.iter().copied().map(|action| {
            action_row(
                "Global Shortcuts",
                "global",
                action,
                action.toml_key(),
                scope.key_for(action).copied(),
            )
        }));
    }
    shortcut_rows.sort_by_key(|row| row.description);
    rows.extend(shortcut_rows);
}

fn framework_global_row(
    action: FrameworkGlobalAction,
    scope: &FrameworkScopeMap<FrameworkGlobalAction>,
) -> KeymapRow {
    action_row_with_description(
        match action {
            FrameworkGlobalAction::NextPane | FrameworkGlobalAction::PrevPane => {
                "Global Navigation"
            },
            _ => "Global Shortcuts",
        },
        "global",
        framework_global_description(action),
        framework_global_toml_key(action),
        scope.key_for(action).copied(),
    )
}

const fn framework_global_description(action: FrameworkGlobalAction) -> &'static str {
    match action {
        FrameworkGlobalAction::Quit => "Quit application",
        FrameworkGlobalAction::Restart => "Restart application",
        FrameworkGlobalAction::NextPane => "Focus next pane",
        FrameworkGlobalAction::PrevPane => "Focus previous pane",
        FrameworkGlobalAction::OpenKeymap => "Open keymap",
        FrameworkGlobalAction::OpenSettings => "Open settings",
        FrameworkGlobalAction::Dismiss => "Dismiss focused item",
    }
}

fn push_navigation_rows(rows: &mut Vec<KeymapRow>, app: &App) {
    if let Some(scope) = app.framework_keymap.navigation::<AppNavigation>() {
        push_scope(
            rows,
            "List Navigation",
            "navigation",
            NavigationAction::ALL,
            scope,
            action_toml_key,
        );
    }
}

fn push_app_pane_rows(rows: &mut Vec<KeymapRow>, app: &App) {
    push_app_pane_scope(
        rows,
        "Project List",
        "project_list",
        ProjectListPane::APP_PANE_ID,
        <ProjectListAction as Action>::ALL,
        app,
    );
    push_app_pane_scope(
        rows,
        "Package",
        "package",
        PackagePane::APP_PANE_ID,
        <PackageAction as Action>::ALL,
        app,
    );
    push_app_pane_scope(
        rows,
        "Lang",
        "lang",
        LangPane::APP_PANE_ID,
        LangAction::ALL,
        app,
    );
    push_app_pane_scope(
        rows,
        "CPU",
        "cpu",
        CpuPane::APP_PANE_ID,
        CpuAction::ALL,
        app,
    );
    push_app_pane_scope(
        rows,
        "Git",
        "git",
        GitPane::APP_PANE_ID,
        <GitAction as Action>::ALL,
        app,
    );
    push_app_pane_scope(
        rows,
        "Targets",
        "targets",
        TargetsPane::APP_PANE_ID,
        <TargetsAction as Action>::ALL,
        app,
    );
    push_app_pane_scope(
        rows,
        "CI Runs",
        "ci_runs",
        CiRunsPane::APP_PANE_ID,
        <CiRunsAction as Action>::ALL,
        app,
    );
    push_app_pane_scope(
        rows,
        "Lints",
        "lints",
        LintsPane::APP_PANE_ID,
        <LintsAction as Action>::ALL,
        app,
    );
    push_app_pane_scope(
        rows,
        "Output",
        "output",
        OutputPane::APP_PANE_ID,
        OutputAction::ALL,
        app,
    );
    push_app_pane_scope(
        rows,
        "Finder",
        "finder",
        FinderPane::APP_PANE_ID,
        FinderAction::ALL,
        app,
    );
}

fn push_overlay_rows(rows: &mut Vec<KeymapRow>, app: &App) {
    push_scope(
        rows,
        "Settings",
        "settings",
        SettingsPaneAction::ALL,
        app.framework_keymap.settings_overlay(),
        action_toml_key,
    );
    push_scope(
        rows,
        "Keymap",
        "keymap",
        KeymapPaneAction::ALL,
        app.framework_keymap.keymap_overlay(),
        action_toml_key,
    );
}

/// Total number of selectable (non-header) rows.
pub(super) fn selectable_row_count(app: &App) -> usize {
    build_rows(app).iter().filter(|row| !row.is_header).count()
}

// ── Key handling ─────────────────────────────────────────────────────

pub(super) fn dispatch_keymap_action(action: KeymapPaneAction, app: &mut App) {
    match action {
        KeymapPaneAction::StartEdit => {
            app.overlays.clear_inline_error();
            app.framework.keymap_pane.enter_awaiting();
        },
        KeymapPaneAction::Save | KeymapPaneAction::Cancel => {
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

fn handle_captured_bind(app: &mut App, bind: FrameworkKeyBind) {
    let rows = build_rows(app);
    let selectable: Vec<&KeymapRow> = rows.iter().filter(|r| !r.is_header).collect();
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
    let conflict = check_scope_conflict(&rows, &row, bind);
    if let Some(msg) = conflict {
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
    rows: &[KeymapRow],
    current: &KeymapRow,
    bind: FrameworkKeyBind,
) -> Option<String> {
    find_conflict(rows, current, bind, |row| row.scope == "global")
}

fn check_non_global_conflict(
    rows: &[KeymapRow],
    current: &KeymapRow,
    bind: FrameworkKeyBind,
) -> Option<String> {
    find_conflict(rows, current, bind, |row| row.scope != "global")
}

fn check_scope_conflict(
    rows: &[KeymapRow],
    current: &KeymapRow,
    bind: FrameworkKeyBind,
) -> Option<String> {
    find_conflict(rows, current, bind, |row| row.scope == current.scope)
}

fn find_conflict(
    rows: &[KeymapRow],
    current: &KeymapRow,
    bind: FrameworkKeyBind,
    predicate: impl Fn(&KeymapRow) -> bool,
) -> Option<String> {
    rows.iter()
        .filter(|row| !row.is_header)
        .filter(|row| predicate(row))
        .filter(|row| row.bind == Some(bind))
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
    save_keymap_to_disk(
        app,
        Some(PendingRebind {
            scope,
            action,
            bind,
        }),
    );
}

pub(super) fn save_current_keymap_to_disk(app: &mut App) { save_keymap_to_disk(app, None); }

fn save_keymap_to_disk(app: &mut App, pending: Option<PendingRebind>) {
    let Some(path) = app.keymap.path() else {
        return;
    };
    let content = current_keymap_toml_with_pending(app, pending.as_ref());
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
    let mut out = String::from(
        "# cargo-port keymap configuration\n\
         # Edit bindings below. Format: action = \"Key\" or \"Modifier+Key\"\n\
         # Modifiers: Ctrl, Alt, Shift.  Examples: \"Ctrl+r\", \"Shift+Tab\", \"q\"\n\
         # Note: when vim navigation is enabled, h/j/k/l are reserved\n\
         #       for navigation and cannot be used as action keys.\n\n",
    );

    write_section(&mut out, "global", global_entries(app), pending);
    write_navigation_section(&mut out, app, pending);
    write_app_pane_sections(&mut out, app, pending);
    write_overlay_sections(&mut out, app, pending);
    if out.ends_with("\n\n") {
        out.pop();
    }

    out
}

fn write_navigation_section(out: &mut String, app: &App, pending: Option<&PendingRebind>) {
    if let Some(scope) = app.framework_keymap.navigation::<AppNavigation>() {
        write_section(
            out,
            "navigation",
            entries_from_scope(NavigationAction::ALL, scope, action_toml_key),
            pending,
        );
    }
}

fn write_app_pane_sections(out: &mut String, app: &App, pending: Option<&PendingRebind>) {
    write_section(
        out,
        "project_list",
        entries_from_app_pane(
            app,
            ProjectListPane::APP_PANE_ID,
            <ProjectListAction as Action>::ALL,
            action_toml_key,
        ),
        pending,
    );
    write_section(
        out,
        "package",
        entries_from_app_pane(
            app,
            PackagePane::APP_PANE_ID,
            <PackageAction as Action>::ALL,
            action_toml_key,
        ),
        pending,
    );
    write_section(
        out,
        "lang",
        entries_from_app_pane(app, LangPane::APP_PANE_ID, LangAction::ALL, action_toml_key),
        pending,
    );
    write_section(
        out,
        "cpu",
        entries_from_app_pane(app, CpuPane::APP_PANE_ID, CpuAction::ALL, action_toml_key),
        pending,
    );
    write_section(
        out,
        "git",
        entries_from_app_pane(
            app,
            GitPane::APP_PANE_ID,
            <GitAction as Action>::ALL,
            action_toml_key,
        ),
        pending,
    );
    write_section(
        out,
        "targets",
        entries_from_app_pane(
            app,
            TargetsPane::APP_PANE_ID,
            <TargetsAction as Action>::ALL,
            action_toml_key,
        ),
        pending,
    );
    write_section(
        out,
        "lints",
        entries_from_app_pane(
            app,
            LintsPane::APP_PANE_ID,
            <LintsAction as Action>::ALL,
            action_toml_key,
        ),
        pending,
    );
    write_section(
        out,
        "ci_runs",
        entries_from_app_pane(
            app,
            CiRunsPane::APP_PANE_ID,
            <CiRunsAction as Action>::ALL,
            action_toml_key,
        ),
        pending,
    );
    write_section(
        out,
        "output",
        entries_from_app_pane(
            app,
            OutputPane::APP_PANE_ID,
            OutputAction::ALL,
            action_toml_key,
        ),
        pending,
    );
    write_section(
        out,
        "finder",
        entries_from_app_pane(
            app,
            FinderPane::APP_PANE_ID,
            FinderAction::ALL,
            action_toml_key,
        ),
        pending,
    );
}

fn write_overlay_sections(out: &mut String, app: &App, pending: Option<&PendingRebind>) {
    write_section(
        out,
        "settings",
        entries_from_scope(
            SettingsPaneAction::ALL,
            app.framework_keymap.settings_overlay(),
            action_toml_key,
        ),
        pending,
    );
    write_section(
        out,
        "keymap",
        entries_from_scope(
            KeymapPaneAction::ALL,
            app.framework_keymap.keymap_overlay(),
            action_toml_key,
        ),
        pending,
    );
}

#[derive(Clone)]
struct TomlEntry {
    action: &'static str,
    binds:  Vec<FrameworkKeyBind>,
}

fn global_entries(app: &App) -> Vec<TomlEntry> {
    let mut entries = entries_from_scope(
        FrameworkGlobalAction::ALL,
        app.framework_keymap.framework_globals(),
        framework_global_toml_key,
    );
    if let Some(scope) = app.framework_keymap.globals::<AppGlobalAction>() {
        entries.extend(entries_from_scope(
            AppGlobalAction::ALL,
            scope,
            action_toml_key,
        ));
    }
    entries
}

fn entries_from_scope<A: Action>(
    actions: &[A],
    scope_map: &FrameworkScopeMap<A>,
    toml_key: fn(A) -> &'static str,
) -> Vec<TomlEntry> {
    actions
        .iter()
        .map(|&action| TomlEntry {
            action: toml_key(action),
            binds:  scope_map.display_keys_for(action).to_vec(),
        })
        .collect()
}

fn entries_from_app_pane<A: Action>(
    app: &App,
    app_pane_id: AppPaneId,
    actions: &[A],
    toml_key: fn(A) -> &'static str,
) -> Vec<TomlEntry> {
    actions
        .iter()
        .map(|&action| {
            let action_key = toml_key(action);
            TomlEntry {
                action: action_key,
                binds:  app
                    .framework_keymap
                    .keys_for_toml_key(app_pane_id, action_key),
            }
        })
        .collect()
}

fn write_section(
    out: &mut String,
    scope: &'static str,
    mut entries: Vec<TomlEntry>,
    pending: Option<&PendingRebind>,
) {
    let _ = writeln!(out, "[{scope}]");
    entries.sort_by_key(|entry| entry.action);
    let max_len = entries
        .iter()
        .map(|entry| entry.action.len())
        .max()
        .unwrap_or(0);
    for entry in entries {
        let value = pending
            .filter(|pending| pending.scope == scope && pending.action == entry.action)
            .map_or_else(
                || keybind_toml_value(&entry.binds),
                |pending| keybind_toml_value(&[pending.bind]),
            );
        let _ = writeln!(out, "{:<max_len$} = {}", entry.action, value);
    }
    out.push('\n');
}

fn keybind_toml_value(binds: &[FrameworkKeyBind]) -> String {
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
    build_rows(app)
        .into_iter()
        .filter(|row| !row.is_header)
        .filter_map(|row| {
            let bind = row.bind?;
            (bind.mods == KeyModifiers::NONE
                && matches!(bind.code, KeyCode::Char('h' | 'j' | 'k' | 'l')))
            .then(|| format!("{}.{}", row.scope, row.action))
        })
        .collect()
}

// ── Rendering ────────────────────────────────────────────────────────

const BASE_POPUP_WIDTH: u16 = 52;
const KEYMAP_POPUP_MAX_HEIGHT: u16 = 43;

#[cfg(test)]
mod tests {
    use super::view::build_lines;
    use super::view::keymap_header_line;
    use super::view::keymap_popup_height;
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn keymap_header_line_uses_section_name() {
        let line = keymap_header_line(&header("Global Navigation"));

        assert_eq!(
            line_text(&line),
            format!("{SECTION_HEADER_INDENT}Global Navigation:")
        );
    }

    #[test]
    fn keymap_header_line_can_label_list_navigation() {
        let line = keymap_header_line(&header("List Navigation"));

        assert_eq!(
            line_text(&line),
            format!("{SECTION_HEADER_INDENT}List Navigation:")
        );
    }

    #[test]
    fn project_list_rows_keep_expand_collapse_pairs_adjacent() {
        let mut rows = vec![
            action_row_with_description(
                "Project List",
                "project_list",
                "Expand row",
                "expand_row",
                None,
            ),
            action_row_with_description(
                "Project List",
                "project_list",
                "Collapse all",
                "collapse_all",
                None,
            ),
            action_row_with_description(
                "Project List",
                "project_list",
                "Clean project",
                "clean",
                None,
            ),
            action_row_with_description(
                "Project List",
                "project_list",
                "Expand all",
                "expand_all",
                None,
            ),
            action_row_with_description(
                "Project List",
                "project_list",
                "Collapse row",
                "collapse_row",
                None,
            ),
        ];

        sort_app_pane_rows("Project List", &mut rows);

        assert_eq!(
            rows.iter().map(|row| row.action).collect::<Vec<_>>(),
            vec![
                "clean",
                "collapse_all",
                "expand_all",
                "collapse_row",
                "expand_row",
            ],
        );
    }

    #[test]
    fn keymap_lines_track_selectable_rows_only() {
        let app = crate::tui::test_support::make_app(&[]);
        let rows = vec![
            header("One"),
            action_row_with_description("One", "one", "First", "first", None),
            header("Two"),
            action_row_with_description("Two", "two", "Second", "second", None),
        ];

        let rendered = build_lines(&rows, &app, false);

        assert_eq!(
            rendered.line_targets,
            vec![None, None, Some(0), None, Some(1), None],
        );
    }

    #[test]
    fn keymap_popup_height_is_bounded_on_tall_terminals() {
        assert_eq!(keymap_popup_height(10, 80), 12);
        assert_eq!(keymap_popup_height(100, 80), KEYMAP_POPUP_MAX_HEIGHT);
        assert_eq!(keymap_popup_height(100, 20), 18);
    }
}
