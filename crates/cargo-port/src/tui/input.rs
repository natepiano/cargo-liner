use std::io::Result;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEventKind;
use ratatui::layout::Position;

use super::app::App;
use super::app::CleanSelection;
use super::app::ConfirmAction;
use super::app::PendingClean;
use super::finder;
use super::interaction;
use super::keymap_ui;
use super::panes;
use super::panes::PaneBehavior;
use super::panes::PaneId;
use super::settings;
use super::shortcuts::InputContext;
use super::terminal;
use crate::keymap::GlobalAction;
use crate::keymap::KeyBind;
use crate::keymap::ProjectListAction;
use crate::project::AbsolutePath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;

/// Last known mouse position, updated from every mouse event. Used to
/// synthesize a click when `FocusGained` arrives because iTerm2 eats the
/// mouse-down event that caused the focus change.
static LAST_MOUSE_POS: Mutex<Option<(u16, u16)>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub(super) fn set_last_mouse_pos_for_test(pos: Option<(u16, u16)>) {
    if let Ok(mut last) = LAST_MOUSE_POS.lock() {
        *last = pos;
    }
}

pub(super) fn handle_event(app: &mut App, event: &Event) {
    let started = Instant::now();
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key_event(app, key),
        Event::Mouse(mouse) => {
            if let Ok(mut pos) = LAST_MOUSE_POS.lock() {
                *pos = Some((mouse.column, mouse.row));
            }
            app.set_mouse_pos(Some(Position::new(mouse.column, mouse.row)));
            handle_mouse_event(app, mouse.kind, mouse.column, mouse.row);
        },
        Event::FocusGained => {
            let _ = terminal::rearm_input_modes();
            if let Ok(pos) = LAST_MOUSE_POS.lock()
                && let Some((column, row)) = *pos
            {
                app.set_mouse_pos(Some(Position::new(column, row)));
                handle_mouse_click(app, column, row);
            }
        },
        _ => {},
    }

    app.sync_selected_project();

    let elapsed = started.elapsed();
    if elapsed.as_millis() >= crate::perf_log::SLOW_INPUT_EVENT_MS {
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(elapsed.as_millis()),
            kind = %event_label(event),
            focus = pane_label(app.focus.current()),
            scan_complete = app.scan.is_complete(),
            selected = %app.project_list.selected_project_path()
                .map_or_else(|| "-".to_string(), |path| path.display().to_string()),
            "input_event"
        );
    }
}

fn handle_key_event(app: &mut App, raw: &KeyEvent) {
    app.set_mouse_pos(None);

    let normalized = normalize_nav(app, raw);
    let code = normalized.code;

    // Structural keys checked by code only (modifiers irrelevant).
    if code == KeyCode::Esc && app.inflight.example_running().is_some() {
        let pid_holder = app.inflight.example_child();
        let pid = *pid_holder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(pid) = pid {
            let _ = std::process::Command::new("kill")
                .arg(pid.to_string())
                .output();
        }
        app.inflight.set_example_running(None);
        app.inflight
            .example_output_mut()
            .push("── killed ──".to_string());
        app.scan.mark_terminal_dirty();
        return;
    }
    if code == KeyCode::Esc && !app.inflight.example_output().is_empty() {
        let was_on_output = app.focus.is(PaneId::Output);
        app.inflight.example_output_mut().clear();
        if was_on_output {
            app.focus.set(PaneId::Targets);
        }
        return;
    }
    if handle_confirm_key(app, code) {
        return;
    }
    if handle_overlay_editor_key(app, &normalized) {
        return;
    }
    if app.overlays.is_keymap_open() {
        keymap_ui::handle_keymap_key(app, raw, &normalized);
        return;
    }
    if app.overlays.is_finder_open() {
        finder::handle_finder_key(app, code);
        return;
    }
    if app.overlays.is_settings_open() {
        settings::handle_settings_key(app, code);
        return;
    }
    if handle_global_key(app, &normalized) {
        return;
    }

    match panes::behavior(app.focus.current()) {
        PaneBehavior::DetailFields | PaneBehavior::DetailTargets | PaneBehavior::Cpu => {
            panes::handle_detail_key(app, &normalized);
        },
        PaneBehavior::Lints => panes::handle_lints_key(app, &normalized),
        PaneBehavior::CiRuns => panes::handle_ci_runs_key(app, &normalized),
        PaneBehavior::Toasts => handle_toast_key(app, &normalized),
        PaneBehavior::ProjectList | PaneBehavior::Output | PaneBehavior::Overlay => {
            handle_normal_key(app, &normalized);
        },
    }
}

/// Build a `KeyBind` from a `KeyEvent`, applying `=`/`+` and `BackTab`
/// normalization.
fn bind_from(event: &KeyEvent) -> KeyBind { KeyBind::new(event.code, event.modifiers) }

/// Normalize navigation keys only. Vim hjkl conversion applies only when
/// no modifiers are held (so `Ctrl+k` is never eaten by vim mode).
/// Arrow remapping in list panes also only applies to bare arrows.
fn normalize_nav(app: &App, raw: &KeyEvent) -> KeyEvent {
    if app.overlays.is_finder_open() || app.overlays.is_settings_editing() {
        return *raw;
    }

    let code = if raw.modifiers == KeyModifiers::NONE && app.config.navigation_keys().uses_vim() {
        match panes::behavior(app.focus.current()) {
            PaneBehavior::DetailFields
            | PaneBehavior::DetailTargets
            | PaneBehavior::Cpu
            | PaneBehavior::CiRuns
            | PaneBehavior::Toasts => match raw.code {
                KeyCode::Char('h' | 'k') => KeyCode::Up,
                KeyCode::Char('j' | 'l') => KeyCode::Down,
                _ => raw.code,
            },
            _ => match raw.code {
                KeyCode::Char('h') => KeyCode::Left,
                KeyCode::Char('j') => KeyCode::Down,
                KeyCode::Char('k') => KeyCode::Up,
                KeyCode::Char('l') => KeyCode::Right,
                _ => raw.code,
            },
        }
    } else {
        raw.code
    };

    // In list panes, bare left/right map to up/down.
    let code = if raw.modifiers == KeyModifiers::NONE {
        match panes::behavior(app.focus.current()) {
            PaneBehavior::DetailFields
            | PaneBehavior::DetailTargets
            | PaneBehavior::Cpu
            | PaneBehavior::CiRuns
            | PaneBehavior::Toasts => match code {
                KeyCode::Left => KeyCode::Up,
                KeyCode::Right => KeyCode::Down,
                _ => code,
            },
            _ => code,
        }
    } else {
        code
    };

    KeyEvent::new(code, raw.modifiers)
}

fn handle_confirm_key(app: &mut App, key: KeyCode) -> bool {
    // Step 6e: while the confirm is waiting for a `cargo metadata`
    // re-fetch, `y` is disabled — the plan isn't trustworthy yet.
    // `n` cancels regardless, so we let the Ignore path fall through
    // to take_confirm().
    if key == KeyCode::Char('y') && app.scan.confirm_verifying().is_some() {
        return true;
    }
    let Some(action) = app.take_confirm() else {
        return false;
    };
    if key == KeyCode::Char('y') {
        match action {
            ConfirmAction::Clean(abs_path) => {
                if app.start_clean(&abs_path) {
                    app.inflight
                        .pending_cleans_mut()
                        .push_back(PendingClean { abs_path });
                }
            },
            ConfirmAction::CleanGroup { primary, linked } => {
                // Fan out `start_clean` over every checkout in the
                // group. Paths whose resolved target dir is absent
                // short-circuit with the "Already clean" toast inside
                // `start_clean` and don't contribute a pending entry;
                // the remainder queue up for execution like individual
                // project cleans.
                for path in std::iter::once(primary).chain(linked) {
                    if app.start_clean(&path) {
                        app.inflight
                            .pending_cleans_mut()
                            .push_back(PendingClean { abs_path: path });
                    }
                }
            },
        }
    }
    true
}

fn handle_mouse_event(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    if app.confirm().is_some() {
        return;
    }
    match kind {
        MouseEventKind::ScrollUp => scroll_pane_at(app, column, row, true),
        MouseEventKind::ScrollDown => scroll_pane_at(app, column, row, false),
        MouseEventKind::Down(MouseButton::Left) => handle_mouse_click(app, column, row),
        _ => {},
    }
}

fn scroll_pane_at(app: &mut App, column: u16, row: u16, scroll_up: bool) {
    let up = scroll_up ^ app.config.invert_scroll().is_inverted();
    let pos = Position::new(column, row);

    if app.layout_cache.project_list_body.contains(pos) {
        if up {
            app.project_list.move_up();
        } else {
            app.project_list.move_down();
        }
        return;
    }

    let pane_regions = app
        .layout_cache
        .tiled
        .panes()
        .iter()
        .map(|resolved| (resolved.pane, resolved.area))
        .collect::<Vec<_>>();
    for (pane_id, pane_rect) in pane_regions {
        if pane_id == PaneId::ProjectList || !pane_rect.contains(pos) {
            continue;
        }
        let pane = interaction::viewport_mut_for(app, pane_id);
        if up {
            pane.up();
        } else {
            pane.down();
        }
        return;
    }
}

const fn pane_label(pane: PaneId) -> &'static str {
    match pane {
        PaneId::ProjectList => "project_list",
        PaneId::Package => "package",
        PaneId::Lang => "lang",
        PaneId::Cpu => "cpu",
        PaneId::Git => "git",
        PaneId::Targets => "targets",
        PaneId::Lints => "lints",
        PaneId::CiRuns => "ci_runs",
        PaneId::Output => "output",
        PaneId::Toasts => "toasts",
        PaneId::Settings => "settings",
        PaneId::Finder => "finder",
        PaneId::Keymap => "keymap",
    }
}

pub(super) fn event_label(event: &Event) -> String {
    match event {
        Event::Key(key) => format!("key:{:?}:{:?}", key.kind, key.code),
        Event::Mouse(mouse) => format!("mouse:{:?}", mouse.kind),
        Event::Resize(width, height) => format!("resize:{width}x{height}"),
        Event::FocusGained => "focus_gained".to_string(),
        Event::FocusLost => "focus_lost".to_string(),
        Event::Paste(text) => format!("paste:{}", text.len()),
    }
}

fn handle_mouse_click(app: &mut App, column: u16, row: u16) {
    let pos = Position::new(column, row);

    if app.confirm().is_some() {
        return;
    }

    if interaction::handle_click(app, pos) {
        return;
    }

    if app.input_context().is_overlay() {
        return;
    }

    let project_list = app.layout_cache.project_list_body;
    let pane_regions = app
        .layout_cache
        .tiled
        .panes()
        .iter()
        .map(|resolved| (resolved.pane, resolved.area))
        .collect::<Vec<_>>();

    if project_list.contains(pos) {
        app.focus.set(PaneId::ProjectList);
        return;
    }

    for (pane_id, pane_rect) in pane_regions {
        if pane_id != PaneId::ProjectList && pane_rect.contains(pos) {
            app.focus.set(pane_id);
            return;
        }
    }
}

fn selected_project_display_name(app: &App) -> String {
    if let Some(name) = app.selected_item().and_then(crate::project::RootItem::name) {
        return name.to_owned();
    }
    app.project_list
        .selected_project_path()
        .and_then(Path::file_name)
        .map_or_else(
            || "selected project".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        )
}

fn open_in_editor(app: &mut App) {
    if app.selected_project_is_deleted() {
        let name = selected_project_display_name(app);
        app.show_timed_warning_toast(
            "Editor unavailable",
            format!("Can't open editor, {name} is deleted"),
        );
        return;
    }
    let Some(selected_path) = app
        .project_list
        .selected_project_path()
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };
    let abs_path = app
        .project_list
        .iter()
        .find_map(|item| match &item.item {
            RootItem::Rust(RustProject::Workspace(ws))
                if ws.groups().iter().any(|g| {
                    g.members()
                        .iter()
                        .any(|m| m.path() == selected_path.as_path())
                }) =>
            {
                Some(ws.path().to_path_buf())
            },
            _ => None,
        })
        .unwrap_or(selected_path);

    let _ = open_paths_in_editor(app.config.editor(), [&abs_path]);
}

fn overlay_editor_target_path(
    context: InputContext,
    config_path: Option<&Path>,
    keymap_path: Option<&Path>,
) -> Option<AbsolutePath> {
    match context {
        InputContext::Settings => config_path.map(AbsolutePath::from),
        InputContext::Keymap => keymap_path.map(AbsolutePath::from),
        _ => None,
    }
}

fn open_path_in_editor(editor: &str, path: &Path) -> Result<()> {
    open_paths_in_editor(editor, [path])
}

pub(super) fn open_paths_in_editor<P>(
    editor: &str,
    paths: impl IntoIterator<Item = P>,
) -> Result<()>
where
    P: AsRef<Path>,
{
    let owned_paths: Vec<PathBuf> = paths
        .into_iter()
        .map(|path| path.as_ref().to_path_buf())
        .collect();
    let paths: Vec<&Path> = owned_paths
        .iter()
        .map(std::path::PathBuf::as_path)
        .collect();
    open_paths_via_editor_command(editor, &paths)
}

fn open_paths_via_editor_command(editor: &str, paths: &[&Path]) -> Result<()> {
    let mut command = std::process::Command::new(editor);
    if let Some(path) = paths.first()
        && let Some(parent) = path.parent()
    {
        command.current_dir(parent);
    }
    for path in paths {
        command.arg(path);
    }
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

fn handle_overlay_editor_key(app: &mut App, event: &KeyEvent) -> bool {
    let bind = bind_from(event);
    let Some(GlobalAction::OpenEditor) = app.keymap.current().global.action_for(&bind) else {
        return false;
    };

    let context = app.input_context();
    let Some(path) = overlay_editor_target_path(context, app.config.path(), app.keymap.path())
    else {
        return false;
    };

    if let Err(err) = open_path_in_editor(app.config.editor(), &path) {
        let title = match context {
            InputContext::Settings => "Settings editor failed",
            InputContext::Keymap => "Keymap editor failed",
            _ => "Editor failed",
        };
        app.show_timed_toast(title, err.to_string());
    }
    true
}

fn open_finder(app: &mut App) {
    let (index, col_widths) = finder::build_finder_index(&app.project_list);
    let finder = app.finder_mut();
    finder.index = index;
    finder.col_widths = col_widths;
    app.focus.open_overlay(PaneId::Finder);
    app.overlays.open_finder();
    let finder = app.finder_mut();
    finder.query.clear();
    finder.results.clear();
    finder.total = 0;
    app.overlays.finder_pane_mut().viewport_mut().home();
}

fn shell_escape_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if path.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", path.replace('\'', "'\\''"))
}

fn terminal_shell_command(command: &str, selected_path: &Path) -> String {
    command.replace("{path}", &shell_escape_path(selected_path))
}

fn open_settings_to_terminal_command(app: &mut App) {
    app.focus.open_overlay(PaneId::Settings);
    app.overlays.open_settings();
    settings::focus_terminal_command(app);
}

fn spawn_terminal_command(command: &str, cwd: &Path) -> Result<()> {
    let mut process = if cfg!(windows) {
        let mut process = std::process::Command::new("cmd");
        process.arg("/C").arg(command);
        process
    } else {
        let mut process = std::process::Command::new("sh");
        process.arg("-c").arg(command);
        process
    };
    process
        .current_dir(cwd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

fn open_terminal(app: &mut App) {
    if app.selected_project_is_deleted() {
        let name = selected_project_display_name(app);
        app.show_timed_warning_toast(
            "Terminal unavailable",
            format!("Can't open terminal, {name} is deleted"),
        );
        return;
    }
    let command = app.config.terminal_command().trim();
    if command.is_empty() {
        open_settings_to_terminal_command(app);
        return;
    }

    let Some(selected_path) = app
        .project_list
        .selected_project_path()
        .map(std::path::Path::to_path_buf)
    else {
        app.show_timed_toast("Terminal", "No selected project path");
        return;
    };

    let command = terminal_shell_command(command, &selected_path);
    if let Err(err) = spawn_terminal_command(&command, &selected_path) {
        app.show_timed_toast("Terminal failed", err.to_string());
    }
}

fn handle_global_key(app: &mut App, event: &KeyEvent) -> bool {
    let bind = bind_from(event);
    let Some(action) = app.keymap.current().global.action_for(&bind) else {
        return false;
    };
    match action {
        GlobalAction::Quit => app.overlays.request_quit(),
        GlobalAction::Restart => app.overlays.request_restart(),
        GlobalAction::Find => open_finder(app),
        GlobalAction::OpenEditor => open_in_editor(app),
        GlobalAction::OpenTerminal => open_terminal(app),
        GlobalAction::Settings => {
            app.focus.open_overlay(PaneId::Settings);
            app.overlays.open_settings();
        },
        GlobalAction::NextPane => app.focus_next_pane(),
        GlobalAction::PrevPane => app.focus_previous_pane(),
        GlobalAction::OpenKeymap => {
            app.focus.open_overlay(PaneId::Keymap);
            app.overlays.open_keymap();
            app.overlays
                .keymap_pane_mut()
                .viewport_mut()
                .set_len(keymap_ui::selectable_row_count());
        },
        GlobalAction::Rescan => app.rescan(),
        GlobalAction::Dismiss => {
            if let Some(target) = app.focused_dismiss_target() {
                app.dismiss(target);
            }
        },
    }
    true
}

fn handle_normal_key(app: &mut App, event: &KeyEvent) {
    // Navigation keys stay hardcoded.
    match event.code {
        KeyCode::Up => return app.project_list.move_up(),
        KeyCode::Down => return app.project_list.move_down(),
        KeyCode::Home => return app.project_list.move_to_top(),
        KeyCode::End => return app.project_list.move_to_bottom(),
        KeyCode::Right => {
            if !app.expand() {
                app.project_list.move_down();
            }
            return;
        },
        KeyCode::Left => {
            if !app.collapse() {
                app.project_list.move_up();
            }
            return;
        },
        _ => {},
    }

    // Action keys through keymap.
    let bind = bind_from(event);
    let Some(action) = app.keymap.current().project_list.action_for(&bind) else {
        return;
    };
    match action {
        ProjectListAction::ExpandAll => app.expand_all(),
        ProjectListAction::CollapseAll => app.collapse_all(),
        ProjectListAction::Clean => {
            // Gate through App::clean_selection — the single source of
            // truth for clean eligibility (design plan → gating fix).
            // Previously this asked for `selected_item().is_rust()`
            // which returns None for WorktreeEntry rows, dropping the
            // per-worktree Clean shortcut.
            if let Some(selection) = app.clean_selection() {
                match selection {
                    CleanSelection::Project { root } => {
                        // Step 6e: request_clean_confirm re-fingerprints
                        // the workspace. On drift it dispatches a
                        // metadata refresh and opens the confirm in
                        // Verifying state; on match it opens Ready.
                        app.request_clean_confirm(root);
                    },
                    CleanSelection::WorktreeGroup { primary, linked } => {
                        app.request_clean_group_confirm(primary, linked);
                    },
                }
            }
        },
    }
}

fn handle_toast_key(app: &mut App, event: &KeyEvent) {
    match event.code {
        KeyCode::Up => app.toasts.viewport_mut().up(),
        KeyCode::Down => app.toasts.viewport_mut().down(),
        KeyCode::Home => app.toasts.viewport_mut().home(),
        KeyCode::End => {
            let last_index = app.toasts.active_now().len().saturating_sub(1);
            app.toasts.viewport_mut().set_pos(last_index);
        },
        KeyCode::Enter => {
            // Open action_path if the focused toast has one.
            if let Some(toast) = app
                .toasts
                .active_now()
                .into_iter()
                .nth(app.toasts.viewport().pos())
                && let Some(path) = toast.action_path()
            {
                let editor = app.config.editor().to_string();
                let path = path.to_path_buf();
                std::thread::spawn(move || {
                    let _ = open_path_in_editor(&editor, &path);
                });
            }
        },
        _ => {},
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn terminal_shell_command_leaves_command_without_path_placeholder_unchanged() {
        assert_eq!(
            terminal_shell_command("open -a Terminal .", Path::new("/tmp/my project")),
            "open -a Terminal ."
        );
    }

    #[test]
    fn terminal_shell_command_substitutes_shell_escaped_path() {
        assert_eq!(
            terminal_shell_command("cd {path} && exec zsh", Path::new("/tmp/my project")),
            "cd '/tmp/my project' && exec zsh"
        );
    }

    #[test]
    fn terminal_shell_command_escapes_single_quotes() {
        assert_eq!(
            terminal_shell_command("cd {path}", Path::new("/tmp/bob's project")),
            "cd '/tmp/bob'\\''s project'"
        );
    }

    #[test]
    fn overlay_editor_target_path_uses_settings_config_path() {
        let config_path = Path::new("/tmp/config.toml");

        assert_eq!(
            overlay_editor_target_path(InputContext::Settings, Some(config_path), None),
            Some(AbsolutePath::from(config_path))
        );
    }

    #[test]
    fn overlay_editor_target_path_uses_keymap_path() {
        let keymap_path = Path::new("/tmp/keymap.toml");

        assert_eq!(
            overlay_editor_target_path(InputContext::Keymap, None, Some(keymap_path)),
            Some(AbsolutePath::from(keymap_path))
        );
    }

    #[test]
    fn overlay_editor_target_path_ignores_non_browsing_contexts() {
        let config_path = Path::new("/tmp/config.toml");
        let keymap_path = Path::new("/tmp/keymap.toml");

        assert_eq!(
            overlay_editor_target_path(
                InputContext::SettingsEditing,
                Some(config_path),
                Some(keymap_path)
            ),
            None
        );
        assert_eq!(
            overlay_editor_target_path(
                InputContext::KeymapAwaiting,
                Some(config_path),
                Some(keymap_path)
            ),
            None
        );
    }
}
