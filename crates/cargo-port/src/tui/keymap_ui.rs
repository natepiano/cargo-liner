use std::fmt::Write as _;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Frame;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::Action;
use tui_pane::FrameworkOverlayId;
use tui_pane::GlobalAction as FrameworkGlobalAction;
use tui_pane::KeyBind as FrameworkKeyBind;
use tui_pane::KeymapCaptureCommand;
use tui_pane::KeymapPaneAction;
use tui_pane::Pane;
use tui_pane::ScopeMap as FrameworkScopeMap;
use tui_pane::SettingsPaneAction;

use super::app::App;
use super::constants::ACTIVE_BORDER_COLOR;
use super::constants::ERROR_COLOR;
use super::constants::LABEL_COLOR;
use super::constants::SECTION_HEADER_INDENT;
use super::constants::SECTION_ITEM_INDENT;
use super::constants::TITLE_COLOR;
use super::framework_keymap::AppGlobalAction;
use super::framework_keymap::AppNavigation;
use super::framework_keymap::AppPaneId;
use super::framework_keymap::CiRunsPane;
use super::framework_keymap::CpuAction;
use super::framework_keymap::CpuPane;
use super::framework_keymap::FinderPane;
use super::framework_keymap::GitPane;
use super::framework_keymap::LangAction;
use super::framework_keymap::LangPane;
use super::framework_keymap::LintsPane;
use super::framework_keymap::NavigationAction;
use super::framework_keymap::OutputPane;
use super::framework_keymap::PackagePane;
use super::framework_keymap::ProjectListPane;
use super::framework_keymap::TargetsPane;
use super::pane::PaneFocusState;
use super::pane::PaneSelectionState;
use super::panes::PaneId;
use super::popup::PopupFrame;
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
    KeymapRow {
        section,
        scope,
        action: toml_key,
        description: action.description(),
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
    section.sort_by_key(|r| r.description);
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
    section_rows.sort_by_key(|r| r.description);
    rows.extend(section_rows);
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
    FrameworkGlobalAction::Quit,
    FrameworkGlobalAction::Restart,
    FrameworkGlobalAction::OpenKeymap,
    FrameworkGlobalAction::OpenSettings,
    FrameworkGlobalAction::Dismiss,
];

fn build_rows(app: &App) -> Vec<KeymapRow> {
    let mut rows = Vec::new();
    push_global_rows(&mut rows, app);
    push_navigation_rows(&mut rows, app);
    push_app_pane_rows(&mut rows, app);
    push_overlay_rows(&mut rows, app);
    rows
}

fn push_global_rows(rows: &mut Vec<KeymapRow>, app: &App) {
    push_scope(
        rows,
        "Global Navigation",
        "global",
        GLOBAL_NAV,
        app.framework_keymap.framework_globals(),
        framework_global_toml_key,
    );
    push_scope(
        rows,
        "Global Shortcuts",
        "global",
        GLOBAL_SHORTCUTS,
        app.framework_keymap.framework_globals(),
        framework_global_toml_key,
    );
    if let Some(scope) = app.framework_keymap.globals::<AppGlobalAction>() {
        push_scope(
            rows,
            "App Global Shortcuts",
            "global",
            AppGlobalAction::ALL,
            scope,
            action_toml_key,
        );
    }
}

fn push_navigation_rows(rows: &mut Vec<KeymapRow>, app: &App) {
    if let Some(scope) = app.framework_keymap.navigation::<AppNavigation>() {
        push_scope(
            rows,
            "Navigation",
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

fn framework_selection_state(
    app: &App,
    selection_index: usize,
    focus: PaneFocusState,
) -> PaneSelectionState {
    let viewport = app.framework.keymap_pane.viewport();
    if selection_index == viewport.pos() && matches!(focus, PaneFocusState::Active) {
        PaneSelectionState::Active
    } else if viewport.hovered() == Some(selection_index) {
        PaneSelectionState::Hovered
    } else if selection_index == viewport.pos() && matches!(focus, PaneFocusState::Remembered) {
        PaneSelectionState::Remembered
    } else {
        PaneSelectionState::Unselected
    }
}

fn build_lines<'a>(rows: &[KeymapRow], app: &App, is_capturing: bool) -> Vec<Line<'a>> {
    let mut selectable_index = 0usize;
    let mut lines = vec![Line::from("")];

    for row in rows {
        if row.is_header {
            lines.push(Line::from(vec![
                Span::raw(SECTION_HEADER_INDENT),
                Span::styled(
                    format!("{}:", row.scope),
                    Style::default()
                        .fg(TITLE_COLOR)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            continue;
        }

        let focus = if app.framework.overlay() == Some(FrameworkOverlayId::Keymap) {
            PaneFocusState::Active
        } else {
            app.pane_focus_state(PaneId::Keymap)
        };
        let selection = framework_selection_state(app, selectable_index, focus);
        let key_text = if selection != PaneSelectionState::Unselected && is_capturing {
            app.overlays
                .inline_error()
                .cloned()
                .unwrap_or_else(|| "Press key...".to_string())
        } else {
            row.key_display.clone()
        };

        let desc_width = 25usize;
        let padded_desc = format!("{:<width$}", row.description, width = desc_width);

        let line = if selection != PaneSelectionState::Unselected
            && is_capturing
            && app.overlays.inline_error().is_some()
        {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}  {padded_desc}"),
                    selection.patch(Style::default().fg(Color::White)),
                ),
                Span::styled(key_text, selection.patch(Style::default().fg(ERROR_COLOR))),
            ])
        } else if selection != PaneSelectionState::Unselected {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}▸ {padded_desc}"),
                    selection.patch(Style::default().fg(Color::White)),
                ),
                Span::styled(
                    key_text,
                    selection.patch(if is_capturing {
                        Style::default()
                            .fg(TITLE_COLOR)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(LABEL_COLOR)
                    }),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}  {padded_desc}"),
                    Style::default().fg(Color::White),
                ),
                Span::styled(key_text, Style::default().fg(LABEL_COLOR)),
            ])
        };

        lines.push(line);
        selectable_index += 1;
    }

    lines.push(Line::from(""));
    lines
}

pub(super) fn render_keymap_popup(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let rows = build_rows(app);

    // Dynamic width: base fits all normal keys, expands for conflict messages.
    let content_width = app.overlays.inline_error().map_or(BASE_POPUP_WIDTH, |msg| {
        // 2 indent + 25 desc + msg len + 2 pad
        let needed = u16::try_from(2 + 25 + msg.len() + 2).unwrap_or(u16::MAX);
        BASE_POPUP_WIDTH.max(needed)
    });
    // +2 for left/right border
    let width = (content_width + 2).min(area.width.saturating_sub(4));

    // Dynamic height: rows + 2 for top/bottom border.
    let content_height = u16::try_from(rows.len()).unwrap_or(u16::MAX);
    let height = (content_height + 2).min(area.height.saturating_sub(2));

    let inner = PopupFrame {
        title: Some(" Keymap ".to_string()),
        border_color: ACTIVE_BORDER_COLOR,
        width,
        height,
    }
    .render(frame);

    let selectable_len = selectable_row_count(app);
    app.framework
        .keymap_pane
        .viewport_mut()
        .set_len(selectable_len);
    app.framework
        .keymap_pane
        .viewport_mut()
        .set_content_area(inner);

    let selected_pos = app.framework.keymap_pane.viewport().pos();
    let is_capturing = app.framework.keymap_pane.is_capturing();
    let lines = build_lines(&rows, app, is_capturing);

    // Scroll to keep selection visible.
    let visible_height = usize::from(inner.height);
    let scroll_offset = if selected_pos >= visible_height {
        selected_pos - visible_height + 1
    } else {
        0
    };
    app.framework
        .keymap_pane
        .viewport_mut()
        .set_viewport_rows(visible_height);
    app.framework
        .keymap_pane
        .viewport_mut()
        .set_scroll_offset(scroll_offset);

    let para = Paragraph::new(lines).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
    frame.render_widget(para, inner);
}
