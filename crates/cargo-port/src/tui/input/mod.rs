mod editor_terminal;

use std::rc::Rc;
use std::sync::Mutex;
use std::time::Instant;

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEventKind;
pub(super) use editor_terminal::handle_framework_overlay_editor_key;
pub(super) use editor_terminal::open_finder;
pub(super) use editor_terminal::open_in_editor;
pub(super) use editor_terminal::open_paths_in_editor;
pub(super) use editor_terminal::open_terminal;
use ratatui::layout::Position;
use tui_pane::Action;
use tui_pane::AppContext;
use tui_pane::FocusedPane;
use tui_pane::FrameworkFocusId;
use tui_pane::FrameworkOverlayId;
use tui_pane::GlobalAction as FrameworkGlobalAction;
use tui_pane::Globals;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::Mode;
use tui_pane::Navigation;
use tui_pane::Pane;
use tui_pane::ToastCommand;
use tui_pane::Viewport;

use super::app::App;
use super::app::CleanSelection;
use super::app::ConfirmAction;
use super::app::PendingClean;
use super::finder;
use super::integration::AppGlobalAction;
use super::integration::AppNavigation;
use super::integration::AppPaneId;
use super::integration::FinderPane;
use super::integration::OutputPane;
use super::interaction;
use super::keymap_ui;
use super::panes;
use super::panes::PaneBehavior;
use super::panes::PaneId;
use super::settings;
use super::terminal;
use crate::keymap::OutputAction;
use crate::keymap::ProjectListAction;

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
            app.mouse_pos = Some(Position::new(mouse.column, mouse.row));
            handle_mouse_event(app, mouse.kind, mouse.column, mouse.row);
        },
        Event::FocusGained => {
            let _ = terminal::rearm_input_modes();
            if let Ok(pos) = LAST_MOUSE_POS.lock()
                && let Some((column, row)) = *pos
            {
                app.mouse_pos = Some(Position::new(column, row));
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
            focus = pane_label(app.focused_pane_id()),
            scan_complete = app.scan.is_complete(),
            selected = %app.project_list.selected_project_path()
                .map_or_else(|| "-".to_string(), |path| path.display().to_string()),
            "input_event"
        );
    }
}

fn handle_key_event(app: &mut App, raw: &KeyEvent) {
    app.mouse_pos = None;

    let normalized = normalize_nav(app, raw);
    let code = raw.code;

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
    let bind = key_bind_from_event(raw);
    if !app.inflight.example_output().is_empty()
        && !focused_text_input_mode(app)
        && app.framework_keymap.is_key_bound_to_toml_key(
            OutputPane::APP_PANE_ID,
            OutputAction::Cancel.toml_key(),
            &bind,
        )
    {
        let was_on_output = app.focus_is(PaneId::Output);
        app.inflight.example_output_mut().clear();
        if was_on_output {
            app.set_focus(FocusedPane::App(AppPaneId::Targets));
        }
        return;
    }
    if handle_confirm_key(app, code) {
        return;
    }
    if dispatch_framework_overlay(app, &bind, &normalized) {
        return;
    }
    if dispatch_finder_overlay(app, &bind) {
        return;
    }
    let focused = *app.framework.focused();
    let focused_on_toasts = matches!(focused, FocusedPane::Framework(FrameworkFocusId::Toasts));
    if focused_on_toasts && dispatch_focused_toasts(app, &bind) {
        return;
    }
    if dispatch_framework_global(app, &bind) {
        return;
    }
    if dispatch_app_global(app, &bind) {
        return;
    }
    if let FocusedPane::App(id) = focused
        && dispatch_focused_app_pane(app, id, &bind)
    {
        return;
    }
    let _ = dispatch_navigation(app, focused, &bind);
}

fn key_bind_from_event(event: &KeyEvent) -> KeyBind { KeyBind::from_key_event(*event) }

fn dispatch_framework_global(app: &mut App, bind: &KeyBind) -> bool {
    let keymap = Rc::clone(&app.framework_keymap);
    let Some(action) = keymap.framework_globals().action_for(bind) else {
        return false;
    };
    let overlay_before = app.framework.overlay();
    keymap.dispatch_framework_global(action, app);
    if matches!(action, FrameworkGlobalAction::Dismiss)
        && app.framework.overlay().is_none()
        && let Some(overlay) = overlay_before
    {
        clear_legacy_framework_overlay_state(app, overlay);
    }
    true
}

fn clear_legacy_framework_overlay_state(app: &mut App, overlay: FrameworkOverlayId) {
    match overlay {
        FrameworkOverlayId::Settings => {
            app.overlays.close_settings();
            app.framework.settings_pane.enter_browse();
        },
        FrameworkOverlayId::Keymap => {
            app.overlays.clear_inline_error();
            app.framework.keymap_pane.enter_browse();
        },
    }
}

fn dispatch_app_global(app: &mut App, bind: &KeyBind) -> bool {
    let keymap = Rc::clone(&app.framework_keymap);
    let Some(scope) = keymap.globals::<AppGlobalAction>() else {
        return false;
    };
    let Some(action) = scope.action_for(bind) else {
        return false;
    };
    (AppGlobalAction::dispatcher())(action, app);
    true
}

fn dispatch_focused_app_pane(app: &mut App, id: AppPaneId, bind: &KeyBind) -> bool {
    let keymap = Rc::clone(&app.framework_keymap);
    matches!(
        keymap.dispatch_app_pane(id, bind, app),
        KeyOutcome::Consumed
    )
}

fn dispatch_focused_toasts(app: &mut App, bind: &KeyBind) -> bool {
    let (outcome, command) = app.framework.toasts.handle_key_command(bind);
    if let ToastCommand::Activate(action) = command {
        app.handle_toast_action(action);
    }
    matches!(outcome, KeyOutcome::Consumed)
}

fn dispatch_framework_overlay(app: &mut App, bind: &KeyBind, normalized: &KeyEvent) -> bool {
    let Some(overlay) = app.framework.overlay() else {
        return false;
    };

    if overlay == FrameworkOverlayId::Settings && app.framework.settings_pane.is_editing() {
        let command = app.framework.settings_pane.handle_text_input(*bind);
        settings::handle_settings_text_command(app, command);
        return true;
    }

    if overlay == FrameworkOverlayId::Keymap && app.framework.keymap_pane.is_capturing() {
        let command = app.framework.keymap_pane.handle_capture_key(*bind);
        keymap_ui::handle_keymap_capture_command(app, command);
        return true;
    }

    if let Some(Mode::TextInput(handler)) = app.framework.focused_pane_mode(app) {
        handler(*bind, app);
        return true;
    }

    if handle_framework_overlay_editor_key(app, bind, overlay) {
        return true;
    }

    match overlay {
        FrameworkOverlayId::Settings => dispatch_settings_overlay(app, bind),
        FrameworkOverlayId::Keymap => dispatch_keymap_overlay(app, bind, normalized),
    }
    true
}

fn dispatch_settings_overlay(app: &mut App, bind: &KeyBind) {
    if let Some(action) = app.framework_keymap.settings_overlay().action_for(bind) {
        settings::dispatch_settings_action(action, app);
        return;
    }
    settings::handle_settings_navigation_key(app, bind.code);
}

fn dispatch_keymap_overlay(app: &mut App, bind: &KeyBind, normalized: &KeyEvent) {
    if let Some(action) = app.framework_keymap.keymap_overlay().action_for(bind) {
        keymap_ui::dispatch_keymap_action(action, app);
        return;
    }
    keymap_ui::handle_keymap_navigation_key(app, normalized);
}

fn dispatch_finder_overlay(app: &mut App, bind: &KeyBind) -> bool {
    if !app.overlays.is_finder_open() {
        return false;
    }
    match (FinderPane::mode())(app) {
        Mode::TextInput(handler) => handler(*bind, app),
        Mode::Static | Mode::Navigable => finder::handle_finder_text_key(app, bind.code),
    }
    true
}

fn dispatch_navigation(app: &mut App, focused: FocusedPane<AppPaneId>, bind: &KeyBind) -> bool {
    let keymap = Rc::clone(&app.framework_keymap);
    let Some(nav_scope) = keymap.navigation::<AppNavigation>() else {
        return false;
    };
    let Some(action) = nav_scope.action_for(bind) else {
        return false;
    };
    (AppNavigation::dispatcher())(action, focused, app);
    true
}

fn focused_text_input_mode(app: &App) -> bool {
    if app.framework.overlay() == Some(FrameworkOverlayId::Keymap)
        && app.framework.keymap_pane.is_capturing()
    {
        return true;
    }
    matches!(
        app.framework.focused_pane_mode(app),
        Some(Mode::TextInput(_))
    )
}

/// Normalize navigation keys only. Vim hjkl conversion applies only when
/// no modifiers are held (so `Ctrl+k` is never eaten by vim mode).
/// Arrow remapping in list panes also only applies to bare arrows.
fn normalize_nav(app: &App, raw: &KeyEvent) -> KeyEvent {
    if focused_text_input_mode(app) {
        return *raw;
    }

    let code = if raw.modifiers == KeyModifiers::NONE && app.config.navigation_keys().uses_vim() {
        match panes::behavior(app.focused_pane_id()) {
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
        match panes::behavior(app.focused_pane_id()) {
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
    // While the confirm is waiting for a `cargo metadata` re-fetch,
    // `y` is disabled — the plan isn't trustworthy yet. `n` cancels
    // regardless, so we let the Ignore path fall through to
    // take_confirm().
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

    if scroll_modal_overlay_at(app, pos, up) {
        return;
    }

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
        .panes
        .iter()
        .map(|resolved| (resolved.pane, resolved.area))
        .collect::<Vec<_>>();
    for (pane_id, pane_rect) in pane_regions {
        if pane_id == PaneId::ProjectList || !pane_rect.contains(pos) {
            continue;
        }
        if let Some(pane) = interaction::viewport_mut_for(app, pane_id) {
            if up {
                pane.up();
            } else {
                pane.down();
            }
        }
        return;
    }
}

const fn scroll_modal_overlay_at(app: &mut App, pos: Position, up: bool) -> bool {
    if app.overlays.is_finder_open() {
        scroll_viewport_if_contains(&mut app.overlays.finder_pane.viewport, pos, up);
        return true;
    }

    match app.framework.overlay() {
        Some(FrameworkOverlayId::Settings) => {
            scroll_viewport_if_contains(app.framework.settings_pane.viewport_mut(), pos, up);
            true
        },
        Some(FrameworkOverlayId::Keymap) => {
            scroll_viewport_if_contains(app.framework.keymap_pane.viewport_mut(), pos, up);
            true
        },
        None => false,
    }
}

const fn scroll_viewport_if_contains(viewport: &mut Viewport, pos: Position, up: bool) {
    if !viewport.content_area().contains(pos) {
        return;
    }
    if up {
        viewport.up();
    } else {
        viewport.down();
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

    if app.framework.overlay().is_some() || app.overlays.is_finder_open() {
        return;
    }

    let project_list = app.layout_cache.project_list_body;
    let pane_regions = app
        .layout_cache
        .tiled
        .panes
        .iter()
        .map(|resolved| (resolved.pane, resolved.area))
        .collect::<Vec<_>>();

    if project_list.contains(pos) {
        app.set_focus(FocusedPane::App(AppPaneId::ProjectList));
        return;
    }

    for (pane_id, pane_rect) in pane_regions {
        if pane_id != PaneId::ProjectList && pane_rect.contains(pos) {
            if let Some(id) = AppPaneId::from_legacy(pane_id) {
                app.set_focus(FocusedPane::App(id));
            }
            return;
        }
    }
}
pub(super) fn dispatch_project_list_action(action: ProjectListAction, app: &mut App) {
    let include_non_rust = app.config.include_non_rust().includes_non_rust();
    match action {
        ProjectListAction::ExpandAll => app.project_list.expand_all(include_non_rust),
        ProjectListAction::CollapseAll => app.project_list.collapse_all(include_non_rust),
        ProjectListAction::ExpandRow => {
            if !app.expand() {
                app.project_list.move_down();
            }
        },
        ProjectListAction::CollapseRow => {
            if !app.project_list.collapse(include_non_rust) {
                app.project_list.move_up();
            }
        },
        ProjectListAction::Clean => request_project_list_clean(app),
    }
}

pub(super) fn dispatch_output_action(action: OutputAction, app: &mut App) {
    match action {
        OutputAction::Cancel => {
            if !app.inflight.example_output().is_empty() {
                app.inflight.example_output_mut().clear();
                app.set_focus(FocusedPane::App(AppPaneId::Targets));
            }
        },
    }
}

fn request_project_list_clean(app: &mut App) {
    // Gate through `App::clean_selection` — the single source of
    // truth for clean eligibility.
    if let Some(selection) = app.project_list.clean_selection() {
        match selection {
            CleanSelection::Project { root } => {
                // `request_clean_confirm` re-fingerprints the workspace.
                // On drift it dispatches a metadata refresh and opens
                // the confirm in Verifying state; on match it opens
                // Ready.
                app.request_clean_confirm(root);
            },
            CleanSelection::WorktreeGroup { primary, linked } => {
                app.request_clean_group_confirm(primary, linked);
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::editor_terminal::framework_overlay_editor_target_path;
    use super::editor_terminal::terminal_shell_command;
    use super::*;
    use crate::project::AbsolutePath;

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
    fn framework_overlay_editor_target_path_uses_settings_config_path() {
        let config_path = Path::new("/tmp/config.toml");

        assert_eq!(
            framework_overlay_editor_target_path(
                FrameworkOverlayId::Settings,
                Some(config_path),
                None
            ),
            Some(AbsolutePath::from(config_path))
        );
    }

    #[test]
    fn framework_overlay_editor_target_path_uses_keymap_path() {
        let keymap_path = Path::new("/tmp/keymap.toml");

        assert_eq!(
            framework_overlay_editor_target_path(
                FrameworkOverlayId::Keymap,
                None,
                Some(keymap_path)
            ),
            Some(AbsolutePath::from(keymap_path))
        );
    }
}
