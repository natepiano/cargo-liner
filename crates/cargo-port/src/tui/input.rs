use std::path::Path;
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
use super::app::ConfirmAction;
use super::app::PendingClean;
use super::detail;
use super::finder;
use super::settings;
use super::shortcuts::InputContext;
use super::types::PaneId;
use crate::keymap::GlobalAction;
use crate::keymap::KeyBind;
use crate::keymap::ProjectListAction;
use crate::project;

pub(super) fn handle_event(app: &mut App, event: &Event) {
    let started = Instant::now();
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key_event(app, key),
        Event::Mouse(mouse) => handle_mouse_event(app, mouse.kind, mouse.column, mouse.row),
        _ => {},
    }

    app.sync_selected_project();

    let elapsed = started.elapsed();
    if elapsed.as_millis() >= crate::perf_log::SLOW_INPUT_EVENT_MS {
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(elapsed.as_millis()),
            kind = %event_label(event),
            focus = pane_label(app.focused_pane()),
            scan_complete = app.is_scan_complete(),
            selected = %app.selected_project_path()
                .map_or_else(|| "-".to_string(), |path| path.display().to_string()),
            "input_event"
        );
    }
}

fn handle_key_event(app: &mut App, raw: &KeyEvent) {
    let normalized = normalize_nav(app, raw);
    let code = normalized.code;

    // Structural keys checked by code only (modifiers irrelevant).
    if code == KeyCode::Esc && app.example_running().is_some() {
        let pid_holder = app.example_child();
        let pid = *pid_holder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(pid) = pid {
            let _ = std::process::Command::new("kill")
                .arg(pid.to_string())
                .output();
        }
        app.set_example_running(None);
        app.example_output_mut().push("── killed ──".to_string());
        app.mark_terminal_dirty();
        return;
    }
    if code == KeyCode::Esc && !app.example_output().is_empty() {
        app.example_output_mut().clear();
        return;
    }
    if handle_confirm_key(app, code) {
        return;
    }
    if handle_overlay_editor_key(app, &normalized) {
        return;
    }
    if app.is_keymap_open() {
        super::keymap_ui::handle_keymap_key(app, raw, &normalized);
        return;
    }
    if app.is_finder_open() {
        finder::handle_finder_key(app, code);
        return;
    }
    if app.is_settings_open() {
        settings::handle_settings_key(app, code);
        return;
    }
    if app.is_searching() {
        handle_search_key(app, code);
        return;
    }
    if handle_global_key(app, &normalized) {
        return;
    }

    match app.focused_pane() {
        PaneId::Package | PaneId::Git | PaneId::Targets => {
            detail::handle_detail_key(app, &normalized);
        },
        PaneId::Lints => detail::handle_lints_key(app, &normalized),
        PaneId::CiRuns => detail::handle_ci_runs_key(app, &normalized),
        PaneId::Toasts => handle_toast_key(app, &normalized),
        _ => handle_normal_key(app, &normalized),
    }
}

/// Build a `KeyBind` from a `KeyEvent`, applying `=`/`+` and `BackTab`
/// normalization.
fn bind_from(event: &KeyEvent) -> KeyBind { KeyBind::new(event.code, event.modifiers) }

/// Normalize navigation keys only. Vim hjkl conversion applies only when
/// no modifiers are held (so `Ctrl+k` is never eaten by vim mode).
/// Arrow remapping in list panes also only applies to bare arrows.
fn normalize_nav(app: &App, raw: &KeyEvent) -> KeyEvent {
    if app.is_searching() || app.is_finder_open() || app.is_settings_editing() {
        return *raw;
    }

    let code = if raw.modifiers == KeyModifiers::NONE && app.navigation_keys().uses_vim() {
        match app.focused_pane() {
            PaneId::Package | PaneId::Git | PaneId::Targets | PaneId::CiRuns | PaneId::Toasts => {
                match raw.code {
                    KeyCode::Char('h' | 'k') => KeyCode::Up,
                    KeyCode::Char('j' | 'l') => KeyCode::Down,
                    _ => raw.code,
                }
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
        match app.focused_pane() {
            PaneId::Package | PaneId::Git | PaneId::Targets | PaneId::CiRuns | PaneId::Toasts => {
                match code {
                    KeyCode::Left => KeyCode::Up,
                    KeyCode::Right => KeyCode::Down,
                    _ => code,
                }
            },
            _ => code,
        }
    } else {
        code
    };

    KeyEvent::new(code, raw.modifiers)
}

fn handle_confirm_key(app: &mut App, key: KeyCode) -> bool {
    let Some(action) = app.take_confirm() else {
        return false;
    };
    if key == KeyCode::Char('y') {
        match action {
            ConfirmAction::Clean(abs_path) => {
                let project_path = project::home_relative_path(Path::new(&abs_path));
                app.start_clean(Path::new(&abs_path));
                app.pending_cleans_mut().push_back(PendingClean {
                    abs_path,
                    project_path,
                });
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
        MouseEventKind::ScrollUp => scroll_main(app, true),
        MouseEventKind::ScrollDown => scroll_main(app, false),
        MouseEventKind::Down(MouseButton::Left) => handle_mouse_click(app, column, row),
        _ => {},
    }
}

fn scroll_main(app: &mut App, scroll_up: bool) {
    let up = scroll_up ^ app.invert_scroll().is_inverted();
    if up {
        app.move_up();
    } else {
        app.move_down();
    }
}

const fn pane_label(pane: PaneId) -> &'static str {
    match pane {
        PaneId::ProjectList => "project_list",
        PaneId::Package => "package",
        PaneId::Git => "git",
        PaneId::Targets => "targets",
        PaneId::Lints => "lints",
        PaneId::CiRuns => "ci_runs",
        PaneId::Toasts => "toasts",
        PaneId::Search => "search",
        PaneId::Settings => "settings",
        PaneId::Finder => "finder",
        PaneId::Keymap => "keymap",
    }
}

fn event_label(event: &Event) -> String {
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

    if super::interaction::handle_click(app, pos) {
        return;
    }

    if app.input_context().is_overlay() {
        return;
    }

    let project_list = app.layout_cache().project_list;
    let detail_columns = app.layout_cache().detail_columns.clone();
    let detail_targets_col = app.layout_cache().detail_targets_col;

    if project_list.contains(pos) {
        app.focus_pane(PaneId::ProjectList);
        return;
    }

    for (col_idx, col_rect) in detail_columns.iter().enumerate() {
        if !col_rect.contains(pos) {
            continue;
        }
        let pane_id = if Some(col_idx) == detail_targets_col {
            PaneId::Targets
        } else if col_idx == 0 {
            PaneId::Package
        } else {
            PaneId::Git
        };
        app.focus_pane(pane_id);
        return;
    }

    if app.lint_pane().content_area().contains(pos) {
        app.focus_pane(PaneId::Lints);
    } else if app.ci_pane().content_area().contains(pos) {
        app.focus_pane(PaneId::CiRuns);
    }
}

fn open_in_editor(app: &App) {
    let Some(selected_path) = app
        .selected_project_path()
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };
    let abs_path = app
        .projects()
        .iter()
        .find_map(|item| match item {
            crate::project::RootItem::Workspace(ws)
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

    let _ = std::process::Command::new(app.editor())
        .arg(&abs_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn overlay_editor_target_path(
    context: InputContext,
    config_path: Option<&Path>,
    keymap_path: Option<&Path>,
) -> Option<std::path::PathBuf> {
    match context {
        InputContext::Settings => config_path.map(Path::to_path_buf),
        InputContext::Keymap => keymap_path.map(Path::to_path_buf),
        _ => None,
    }
}

fn open_path_in_zed(path: &Path) -> std::io::Result<()> {
    let mut command = std::process::Command::new("zed");
    if let Some(parent) = path.parent() {
        command.current_dir(parent);
    }
    command
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

fn handle_overlay_editor_key(app: &mut App, event: &KeyEvent) -> bool {
    let bind = bind_from(event);
    let Some(GlobalAction::OpenEditor) = app.current_keymap().global.action_for(&bind) else {
        return false;
    };

    let context = app.input_context();
    let Some(path) = overlay_editor_target_path(
        context,
        app.config_path().map(std::path::PathBuf::as_path),
        app.keymap_path().map(std::path::PathBuf::as_path),
    ) else {
        return false;
    };

    if let Err(err) = open_path_in_zed(&path) {
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
    if app.dirty().finder.is_dirty() {
        let (index, col_widths) = super::finder::build_finder_index(app.projects());
        let finder = app.finder_mut();
        finder.index = index;
        finder.col_widths = col_widths;
        app.dirty_mut().finder.mark_clean();
    }
    app.open_overlay(PaneId::Finder);
    app.open_finder();
    let finder = app.finder_mut();
    finder.query.clear();
    finder.results.clear();
    finder.total = 0;
    finder.pane.home();
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
    app.open_overlay(PaneId::Settings);
    app.open_settings();
    settings::focus_terminal_command(app);
}

fn spawn_terminal_command(command: &str, cwd: &Path) -> std::io::Result<()> {
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
    let command = app.terminal_command().trim();
    if command.is_empty() {
        open_settings_to_terminal_command(app);
        return;
    }

    let Some(selected_path) = app
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
    let Some(action) = app.current_keymap().global.action_for(&bind) else {
        return false;
    };
    match action {
        GlobalAction::Quit => app.request_quit(),
        GlobalAction::Restart => app.request_restart(),
        GlobalAction::Find => open_finder(app),
        GlobalAction::OpenEditor => open_in_editor(app),
        GlobalAction::OpenTerminal => open_terminal(app),
        GlobalAction::Settings => {
            app.open_overlay(PaneId::Settings);
            app.open_settings();
        },
        GlobalAction::NextPane => app.focus_next_pane(),
        GlobalAction::PrevPane => app.focus_previous_pane(),
        GlobalAction::OpenKeymap => {
            app.open_overlay(PaneId::Keymap);
            app.open_keymap();
            app.keymap_pane_mut()
                .set_len(super::keymap_ui::selectable_row_count());
        },
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
        KeyCode::Up => return app.move_up(),
        KeyCode::Down => return app.move_down(),
        KeyCode::Home => return app.move_to_top(),
        KeyCode::End => return app.move_to_bottom(),
        KeyCode::Right => {
            if !app.expand() {
                app.move_down();
            }
            return;
        },
        KeyCode::Left => {
            if !app.collapse() {
                app.move_up();
            }
            return;
        },
        _ => {},
    }

    // Action keys through keymap.
    let bind = bind_from(event);
    let Some(action) = app.current_keymap().project_list.action_for(&bind) else {
        return;
    };
    match action {
        ProjectListAction::ExpandAll => app.expand_all(),
        ProjectListAction::CollapseAll => app.collapse_all(),
        ProjectListAction::Rescan => app.rescan(),
        ProjectListAction::Clean => {
            if let Some(path) = app.selected_project_path()
                && app
                    .selected_item()
                    .is_some_and(crate::project::RootItem::is_rust)
            {
                app.set_confirm(ConfirmAction::Clean(path.display().to_string()));
            }
        },
    }
}

fn handle_toast_key(app: &mut App, event: &KeyEvent) {
    match event.code {
        KeyCode::Up => app.toast_pane_mut().up(),
        KeyCode::Down => app.toast_pane_mut().down(),
        KeyCode::Home => app.toast_pane_mut().home(),
        KeyCode::End => {
            let last_index = app.active_toasts().len().saturating_sub(1);
            app.toast_pane_mut().set_pos(last_index);
        },
        KeyCode::Enter => {
            // Open action_path if the focused toast has one.
            if let Some(toast) = app.active_toasts().into_iter().nth(app.toast_pane().pos())
                && let Some(path) = toast.action_path()
            {
                let editor = app.editor().to_string();
                let path = path.to_path_buf();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new(&editor).arg(&path).spawn();
                });
            }
        },
        _ => {},
    }
}

fn handle_search_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.confirm_search(),
        KeyCode::Up => app.move_up(),
        KeyCode::Down => app.move_down(),
        KeyCode::Backspace => {
            let mut query = app.search_query().to_string();
            query.pop();
            app.update_search(&query);
        },
        KeyCode::Char(c) => {
            let query = format!("{}{c}", app.search_query());
            app.update_search(&query);
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
            Some(config_path.to_path_buf())
        );
    }

    #[test]
    fn overlay_editor_target_path_uses_keymap_path() {
        let keymap_path = Path::new("/tmp/keymap.toml");

        assert_eq!(
            overlay_editor_target_path(InputContext::Keymap, None, Some(keymap_path)),
            Some(keymap_path.to_path_buf())
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
