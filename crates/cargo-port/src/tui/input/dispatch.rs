use std::rc::Rc;
use std::time::Instant;

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseButton;
use crossterm::event::MouseEventKind;
use ratatui::layout::Position;
use tui_pane::Action;
use tui_pane::AppContext;
use tui_pane::FocusedPane;
use tui_pane::FrameworkFocusId;
use tui_pane::FrameworkOverlayId;
use tui_pane::GlobalAction;
use tui_pane::Globals;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::KeySequence;
use tui_pane::Mode;
use tui_pane::Navigation;
use tui_pane::OverlayAction;
use tui_pane::PERF_LOG_TARGET;
use tui_pane::Pane;
use tui_pane::SLOW_INPUT_EVENT_MS;
use tui_pane::Viewport;

use super::editor_terminal;
use crate::tui::app::App;
use crate::tui::app::ConfirmAction;
use crate::tui::app::ConfirmationAcceptance;
use crate::tui::app::PendingClean;
use crate::tui::finder;
use crate::tui::integration::AppGlobalAction;
use crate::tui::integration::AppNavigation;
use crate::tui::integration::AppPaneId;
use crate::tui::integration::FinderPane;
use crate::tui::integration::NavAction;
use crate::tui::integration::OutputPane;
use crate::tui::interaction;
use crate::tui::interaction::ClickMode;
use crate::tui::keymap;
use crate::tui::keymap::OutputAction;
use crate::tui::keymap::ProjectListAction;
use crate::tui::keymap_ui;
use crate::tui::panes;
use crate::tui::panes::CapturedOutputRow;
use crate::tui::panes::ColumnSelection;
use crate::tui::panes::OutputCopyAvailability;
use crate::tui::panes::OutputTabStep;
use crate::tui::panes::OwnedColumnSelection;
use crate::tui::panes::PaneBehavior;
use crate::tui::panes::PaneId;
use crate::tui::panes::VisualSelectionPermission;
use crate::tui::sccache;
use crate::tui::settings;
use crate::tui::terminal;

pub fn handle_event(app: &mut App, event: &Event) {
    let started = Instant::now();
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => handle_key_event(app, key),
        Event::Mouse(_) if app.confirmation_is_open() => {},
        Event::Mouse(mouse) => {
            tui_pane::record_mouse_pos(mouse.column, mouse.row);
            app.mouse_pos = Some(Position::new(mouse.column, mouse.row));
            handle_mouse_event(app, mouse.kind, mouse.column, mouse.row);
        },
        Event::FocusGained => {
            let _ = terminal::rearm_input_modes();
            if let Some((column, row)) = tui_pane::last_mouse_pos() {
                app.mouse_pos = Some(Position::new(column, row));
                handle_mouse_click(app, column, row, ClickMode::FocusOnly);
            }
        },
        _ => {},
    }

    app.sync_selected_project();

    let elapsed = started.elapsed();
    if elapsed.as_millis() >= SLOW_INPUT_EVENT_MS {
        tracing::trace!(
            target: PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(elapsed.as_millis()),
            kind = %tui_pane::event_label(event),
            focus = pane_label(app.focused_pane_id()),
            scan_complete = app.scan.is_complete(),
            selected = %app.project_list.selected_project_path()
                .map_or_else(|| "-".to_string(), |path| path.display().to_string()),
            "input_event"
        );
    }
}

#[derive(Clone, Copy)]
enum KeyDispatchLayer {
    FrameworkOverlay,
    AppSurface(AppSurfaceKey),
}

#[derive(Clone, Copy)]
struct AppSurfaceKey {
    focused: FocusedPane<AppPaneId>,
}

impl KeyDispatchLayer {
    const fn current(app: &App) -> Self {
        if app.confirmation_is_open() {
            Self::AppSurface(AppSurfaceKey {
                focused: *app.framework.focused(),
            })
        } else if app.framework.overlay().is_some() {
            Self::FrameworkOverlay
        } else {
            Self::AppSurface(AppSurfaceKey {
                focused: *app.framework.focused(),
            })
        }
    }
}

#[derive(Clone, Copy)]
enum OutputCancelPreflight {
    ExitVisualSelection,
    StopRunningExample,
    CloseMonitor,
    CloseVisibleOutput,
    Pass,
}

fn handle_key_event(app: &mut App, raw: &KeyEvent) {
    app.mouse_pos = None;
    // Drop stale focus on a bottom-row pane the current layout hides so
    // dispatch routes keys to the visible pane, not the overlaid one.
    app.reconcile_bottom_row_focus();

    let normalized = normalize_nav(app, raw);
    let bind = key_bind_from_event(raw);

    match KeyDispatchLayer::current(app) {
        KeyDispatchLayer::FrameworkOverlay => {
            dispatch_framework_overlay_key(app, &bind, &normalized);
            app.pending_nav_chord.clear();
        },
        KeyDispatchLayer::AppSurface(surface) => {
            handle_app_surface_key(app, surface, raw, &bind);
        },
    }
}

fn handle_app_surface_key(app: &mut App, surface: AppSurfaceKey, raw: &KeyEvent, bind: &KeyBind) {
    let code = raw.code;
    if handle_confirm_key(app, code) {
        app.pending_nav_chord.clear();
        return;
    }
    let output_preflight = classify_output_cancel_preflight(app, code, bind);
    if dispatch_output_cancel_preflight(app, output_preflight) {
        app.pending_nav_chord.clear();
        return;
    }
    if dispatch_finder_overlay(app, bind) {
        app.pending_nav_chord.clear();
        return;
    }
    if sccache::dispatch_sccache_overlay(app, bind) {
        app.pending_nav_chord.clear();
        return;
    }
    let focused = surface.focused;
    let focused_on_toasts = matches!(focused, FocusedPane::Framework(FrameworkFocusId::Toasts));
    if focused_on_toasts && tui_pane::dispatch_focused_toasts(app, bind) {
        app.pending_nav_chord.clear();
        return;
    }
    if dispatch_output_tab_preflight(app, bind) {
        app.pending_nav_chord.clear();
        return;
    }
    if dispatch_framework_global(app, bind) {
        app.pending_nav_chord.clear();
        return;
    }
    if dispatch_app_global(app, bind) {
        app.pending_nav_chord.clear();
        return;
    }
    if let FocusedPane::App(id) = focused
        && dispatch_focused_app_pane(app, id, bind)
    {
        app.pending_nav_chord.clear();
        return;
    }
    if app.focus_is(PaneId::Output)
        && !focused_text_input_mode(app)
        && dispatch_output_selection_gesture(app, raw)
    {
        app.pending_nav_chord.clear();
        return;
    }
    let _ = dispatch_navigation(app, focused, bind);
}

fn classify_output_cancel_preflight(
    app: &App,
    code: KeyCode,
    bind: &KeyBind,
) -> OutputCancelPreflight {
    let app_overlay_open = app.overlays.is_finder_open() || app.overlays.is_sccache_open();
    let is_output_cancel = !app_overlay_open
        && !focused_text_input_mode(app)
        && app.framework_keymap.is_key_bound_to_toml_key(
            OutputPane::APP_PANE_ID,
            OutputAction::Cancel.toml_key(),
            bind,
        );
    // A hidden visual selection stays stored, but it intercepts Esc only while
    // the owned output region it covers is the selected one: with an external
    // column selected, Esc must not drop a selection the user cannot see.
    let owned_column_selected =
        matches!(app.owned_column_selection(), OwnedColumnSelection::Selected);
    let output_visual = is_output_cancel
        && app.focus_is(PaneId::Output)
        && owned_column_selected
        && app.panes.output.selection().is_visual();
    // Esc acts on the owned run only while the owned column is the one under
    // the cursor. With an external column selected it neither stops the run nor
    // clears its output: the monitor's columns describe host processes Cargo
    // Port did not launch, and Esc must not reach past the selection to the one
    // it did.
    let running_example =
        code == KeyCode::Esc && owned_column_selected && app.inflight.owned_run().is_running();
    let visible_monitor = is_output_cancel && app.compile_visibility_state.is_on();
    let visible_output = is_output_cancel
        && owned_column_selected
        && matches!(
            app.output_copy_availability(),
            OutputCopyAvailability::CapturedOutput
        );

    match (
        output_visual,
        running_example,
        visible_monitor,
        visible_output,
    ) {
        (true, _, _, _) => OutputCancelPreflight::ExitVisualSelection,
        (false, true, _, _) => OutputCancelPreflight::StopRunningExample,
        (false, false, true, _) => OutputCancelPreflight::CloseMonitor,
        (false, false, false, true) => OutputCancelPreflight::CloseVisibleOutput,
        (false, false, false, false) => OutputCancelPreflight::Pass,
    }
}

fn dispatch_output_cancel_preflight(app: &mut App, preflight: OutputCancelPreflight) -> bool {
    match preflight {
        OutputCancelPreflight::ExitVisualSelection => {
            app.panes.output.exit_visual();
            true
        },
        OutputCancelPreflight::StopRunningExample => {
            let _ = terminal::signal_owned_run(&mut app.inflight);
            true
        },
        OutputCancelPreflight::CloseMonitor => {
            app.toggle_compile_visibility(Instant::now());
            app.reconcile_bottom_row_focus();
            true
        },
        OutputCancelPreflight::CloseVisibleOutput => {
            let was_on_output = app.focus_is(PaneId::Output);
            app.inflight.clear_owned_run_output();
            if was_on_output {
                app.set_focus(FocusedPane::App(AppPaneId::Targets));
            }
            true
        },
        OutputCancelPreflight::Pass => false,
    }
}

/// Tab / Shift-Tab traverse the monitor's ordered session list before the
/// framework's pane snaking sees them.
///
/// Action-aware rather than key-aware: the pane-cycle pair is read off the
/// framework keymap, so a rebound Tab traverses columns too. Traversal covers
/// the complete session list including columns held off screen; at either end
/// of it, and with fewer than two sessions, the key falls through and the
/// framework moves focus to the next pane.
fn dispatch_output_tab_preflight(app: &mut App, bind: &KeyBind) -> bool {
    if !app.focus_is(PaneId::Output) || focused_text_input_mode(app) {
        return false;
    }
    let keymap = Rc::clone(&app.framework_keymap);
    let output_tab_step = match keymap.framework_globals().action_for(bind) {
        Some(GlobalAction::NextPane) => OutputTabStep::NextColumn,
        Some(GlobalAction::PrevPane) => OutputTabStep::PreviousColumn,
        _ => return false,
    };
    let borrows = app.split_output_for_navigation();
    match borrows
        .output
        .tab_to_adjacent_column(&borrows.output_presentation, output_tab_step)
    {
        ColumnSelection::Moved => true,
        ColumnSelection::NoSuchColumn => false,
    }
}

fn dispatch_framework_overlay_key(app: &mut App, bind: &KeyBind, normalized: &KeyEvent) {
    if app
        .framework
        .overlay()
        .is_some_and(|overlay| !tui_pane::overlay_is_in_text_mode(&app.framework, overlay))
        && dispatch_lint_pause_global(app, bind)
    {
        return;
    }
    if !dispatch_framework_overlay(app, bind, normalized) {
        let _ = dispatch_framework_global(app, bind);
    }
}

/// Output-pane selection gestures, built in and not rebindable:
///   V (vim mode only)                toggle vim visual-line mode
///   Shift+Up / Shift+Down            extend the range one row
///   Ctrl+Shift+Up / Ctrl+Shift+Down  extend the range to top / bottom
///
/// The cursor row is always a one-line selection, so these grow it from
/// the anchor rather than entering a mode. `V` is the vim affordance:
/// with vim keys off it does nothing. Returns whether the key was
/// consumed. Caller guards on Output focus and non-text-input mode.
fn dispatch_output_selection_gesture(app: &mut App, raw: &KeyEvent) -> bool {
    // A visual range only ever covers Cargo Port's own transcript, so this path
    // is gated the same way the drag and Ctrl-A paths already are: on a monitor
    // row there is nothing of the pane's own to grow a selection over.
    match app.output_visual_selection_permission() {
        VisualSelectionPermission::Denied => return false,
        VisualSelectionPermission::CapturedOutput => {},
    }
    let code = raw.code;
    if app.config.navigation_keys().uses_vim()
        && code == KeyCode::Char('V')
        && !raw.modifiers.contains(KeyModifiers::CONTROL)
        && !raw.modifiers.contains(KeyModifiers::ALT)
    {
        let live = app.inflight.owned_run().output().to_vec();
        app.panes.output.toggle_visual(&live);
        return true;
    }
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    let to_edge = raw.modifiers == ctrl_shift;
    let one_row = raw.modifiers == KeyModifiers::SHIFT;
    if (one_row || to_edge) && matches!(code, KeyCode::Up | KeyCode::Down) {
        let live = app.inflight.owned_run().output().to_vec();
        let output = &mut app.panes.output;
        match (to_edge, code) {
            (false, KeyCode::Up) => output.select_extend_up(&live),
            (false, KeyCode::Down) => output.select_extend_down(&live),
            (true, KeyCode::Up) => output.select_extend_to_top(&live),
            (true, _) => output.select_extend_to_bottom(&live),
            (false, _) => {},
        }
        return true;
    }
    false
}

fn key_bind_from_event(event: &KeyEvent) -> KeyBind {
    let bind = KeyBind::from(*event);
    let (code, mods) = keymap::canonical_event_code_and_mods(bind.code, bind.mods);
    KeyBind { code, mods }
}

fn dispatch_framework_global(app: &mut App, bind: &KeyBind) -> bool {
    let keymap = Rc::clone(&app.framework_keymap);
    let Some(action) = keymap.framework_globals().action_for(bind) else {
        return false;
    };
    let overlay_before = app.framework.overlay();
    keymap.dispatch_framework_global(action, app);
    if app.framework.overlay().is_none()
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
        FrameworkOverlayId::GlobalShortcuts => {},
    }
}

fn dispatch_app_global(app: &mut App, bind: &KeyBind) -> bool {
    let Some(action) = app_global_action_for(app, bind) else {
        return false;
    };
    (AppGlobalAction::dispatcher())(action, app);
    true
}

fn dispatch_lint_pause_global(app: &mut App, bind: &KeyBind) -> bool {
    let Some(action) = app_global_action_for(app, bind) else {
        return false;
    };
    if !matches!(
        action,
        AppGlobalAction::PauseSelectedLint | AppGlobalAction::PauseAllLints
    ) {
        return false;
    }
    (AppGlobalAction::dispatcher())(action, app);
    true
}

fn app_global_action_for(app: &App, bind: &KeyBind) -> Option<AppGlobalAction> {
    let keymap = Rc::clone(&app.framework_keymap);
    keymap.globals::<AppGlobalAction>()?.action_for(bind)
}

fn dispatch_focused_app_pane(app: &mut App, app_pane_id: AppPaneId, bind: &KeyBind) -> bool {
    let keymap = Rc::clone(&app.framework_keymap);
    matches!(
        keymap.dispatch_app_pane(app_pane_id, bind, app),
        KeyOutcome::Consumed
    )
}

fn dispatch_framework_overlay(app: &mut App, bind: &KeyBind, normalized: &KeyEvent) -> bool {
    let Some(overlay) = app.framework.overlay() else {
        return false;
    };

    // When the user presses the global open-overlay key for the
    // currently-open overlay, fall through to the global dispatcher
    // so it can toggle the overlay closed. Keeps overlay open/close
    // symmetric across every framework overlay regardless of what the
    // overlay-local scope binds. Editing and capture modes still
    // short-circuit below — text input should not be hijacked into a
    // close.
    if let Some(action) = app.framework_keymap.framework_globals().action_for(bind)
        && tui_pane::matches_open_overlay_toggle(action, overlay)
        && !tui_pane::overlay_is_in_text_mode(&app.framework, overlay)
    {
        return false;
    }

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

    if editor_terminal::handle_framework_overlay_editor_key(app, bind, overlay) {
        return true;
    }

    match overlay {
        FrameworkOverlayId::Settings => dispatch_settings_overlay(app, bind),
        FrameworkOverlayId::Keymap => dispatch_keymap_overlay(app, bind, normalized),
        FrameworkOverlayId::GlobalShortcuts => {
            dispatch_global_shortcuts_overlay(app, bind, normalized);
        },
    }
    true
}

fn dispatch_settings_overlay(app: &mut App, bind: &KeyBind) {
    if let Some(action) = app.framework_keymap.overlay().action_for(bind) {
        settings::dispatch_settings_action(action, app);
        return;
    }
    settings::handle_settings_navigation_key(app, bind.code);
}

fn dispatch_keymap_overlay(app: &mut App, bind: &KeyBind, normalized: &KeyEvent) {
    if let Some(action) = app.framework_keymap.overlay().action_for(bind) {
        keymap_ui::dispatch_keymap_action(action, app);
        return;
    }
    keymap_ui::handle_keymap_navigation_key(app, normalized);
}

fn dispatch_global_shortcuts_overlay(app: &mut App, bind: &KeyBind, normalized: &KeyEvent) {
    if let Some(action) = app.framework_keymap.overlay().action_for(bind) {
        match action {
            OverlayAction::StartEdit => {
                keymap_ui::edit_selected_global_shortcut(app);
            },
            OverlayAction::Cancel => {
                app.close_framework_overlay_if_open();
            },
        }
    } else {
        app.framework
            .global_shortcuts_pane
            .handle_navigation_key(normalized.code);
    }
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
    let Some(nav_scope) = keymap.navigation() else {
        return false;
    };
    app.pending_nav_chord.push(*bind);
    let pending = KeySequence::new(app.pending_nav_chord.clone());
    if let Some(action) = nav_scope.action_for_sequence(&pending)
        && !nav_scope.has_prefix(pending.keys())
    {
        app.pending_nav_chord.clear();
        (AppNavigation::dispatcher())(action, focused, app);
        return true;
    }
    if nav_scope.has_prefix(pending.keys()) {
        return true;
    }

    app.pending_nav_chord.clear();
    let single = KeySequence::from(*bind);
    if let Some(action) = nav_scope.action_for_sequence(&single)
        && !nav_scope.has_prefix(single.keys())
    {
        (AppNavigation::dispatcher())(action, focused, app);
        return true;
    }
    if nav_scope.has_prefix(single.keys()) {
        app.pending_nav_chord.push(*bind);
        return true;
    }
    false
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
    if !app.confirmation_is_open() {
        return false;
    }
    match key {
        KeyCode::Char('y') => match app.accept_confirmation() {
            ConfirmationAcceptance::Closed => return false,
            ConfirmationAcceptance::Verifying => {},
            ConfirmationAcceptance::Ready(action) => execute_confirmed_action(app, action),
        },
        KeyCode::Char('n') | KeyCode::Esc => app.dismiss_confirmation(),
        _ => {},
    }
    true
}

fn execute_confirmed_action(app: &mut App, action: ConfirmAction) {
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
        ConfirmAction::KillTarget {
            termination_capability,
            ..
        } => {
            panes::execute_target_kill(app, termination_capability);
        },
        ConfirmAction::TerminateSelectedBuild {
            selected_build_termination_confirmation_display,
            selected_build_termination_authorization,
        } => app.submit_selected_build_termination(
            selected_build_termination_confirmation_display,
            *selected_build_termination_authorization,
        ),
        ConfirmAction::TerminateOutputBuildSet {
            output_build_set_termination_confirmation_display,
            output_build_set_termination_authorization,
        } => app.submit_output_build_set_termination(
            output_build_set_termination_confirmation_display,
            *output_build_set_termination_authorization,
        ),
        ConfirmAction::PauseLintProject(project_root) => {
            app.pause_project_lints(&project_root);
        },
        ConfirmAction::PauseAllLints => app.pause_all_lints(),
    }
}

fn handle_mouse_event(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    if app.confirmation_is_open() {
        return;
    }

    match kind {
        MouseEventKind::ScrollUp => scroll_pane_at(app, column, row, true),
        MouseEventKind::ScrollDown => scroll_pane_at(app, column, row, false),
        MouseEventKind::Down(MouseButton::Left) => {
            handle_mouse_click(app, column, row, ClickMode::Dispatch);
        },
        MouseEventKind::Drag(MouseButton::Left) => handle_output_drag(app, column, row),
        _ => {},
    }
}

/// Extend the output pane's linewise selection to the row under the
/// pointer while the left button is held. Gated on output focus, so a
/// drag that began in another pane never reaches here, and on the cursor
/// sitting in captured output, so a drag never extends a selection the
/// monitor's rows cannot take part in. Motion off the captured-output
/// rows yields no row and leaves the range as-is.
fn handle_output_drag(app: &mut App, column: u16, row: u16) {
    if !app.focus_is(PaneId::Output) {
        return;
    }
    match app.panes.output.visual_selection_permission() {
        VisualSelectionPermission::Denied => return,
        VisualSelectionPermission::CapturedOutput => {},
    }
    let pos = Position::new(column, row);
    let CapturedOutputRow::Row(row) = app.panes.output.captured_output_row_at(pos) else {
        return;
    };
    let live = app.inflight.owned_run().output().to_vec();
    app.panes.output.select_drag_to(&live, row);
}

fn scroll_pane_at(app: &mut App, column: u16, row: u16, scroll_up: bool) {
    let up = scroll_up ^ app.config.invert_scroll().is_inverted();
    let pos = Position::new(column, row);

    if scroll_modal_overlay_at(app, pos, up) {
        return;
    }

    if app.panes.project_list.body_rect.contains(pos) {
        if up {
            app.project_list.move_up();
        } else {
            app.project_list.move_down();
        }
        return;
    }

    let pane_regions = app
        .panes
        .tiled_layout
        .panes
        .iter()
        .map(|resolved| (resolved.pane, resolved.area))
        .collect::<Vec<_>>();
    for (pane_id, pane_rect) in pane_regions {
        if pane_id == PaneId::ProjectList || !pane_rect.contains(pos) {
            continue;
        }
        if pane_id == PaneId::Package {
            let action = if up { NavAction::Up } else { NavAction::Down };
            panes::navigate_package_detail(app, action);
            return;
        }
        if let Some(pane) = interaction::viewport_mut_for(app, pane_id) {
            if up {
                pane.up();
            } else {
                pane.down();
            }
            if pane_id == PaneId::Targets {
                // A wheel step is a user-driven cursor move: re-derive
                // the Running-box PID anchor from the new row.
                panes::sync_running_targets_cursor(app);
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
    if app.overlays.is_sccache_open() {
        scroll_viewport_if_contains(app.overlays.sccache_pane.viewport_mut(), pos, up);
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
        Some(FrameworkOverlayId::GlobalShortcuts) => {
            scroll_viewport_if_contains(
                app.framework.global_shortcuts_pane.viewport_mut(),
                pos,
                up,
            );
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
        PaneId::Sccache => "sccache",
    }
}

fn handle_mouse_click(app: &mut App, column: u16, row: u16, mode: ClickMode) {
    let pos = Position::new(column, row);

    if app.confirmation_is_open() {
        return;
    }

    // A fresh left-press in the output body collapses any prior drag
    // selection back to the single clicked line, so release-then-click
    // starts over instead of extending from the old anchor. Handled here
    // because the generic hit-test only moves the cursor.
    if matches!(mode, interaction::ClickMode::Dispatch)
        && let Some(hovered) = interaction::hovered_pane_row_at(app, pos)
        && hovered.pane == PaneId::Output
    {
        app.set_focus(FocusedPane::App(AppPaneId::Output));
        let live = app.inflight.owned_run().output().to_vec();
        app.panes.output.click_select_row(&live, hovered.row);
        return;
    }

    if interaction::handle_click(app, pos, mode) {
        return;
    }

    if app.framework.overlay().is_some()
        || app.overlays.is_finder_open()
        || app.overlays.is_sccache_open()
    {
        return;
    }

    let project_list = app.panes.project_list.body_rect;
    let pane_regions = app
        .panes
        .tiled_layout
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
pub fn dispatch_project_list_action(action: ProjectListAction, app: &mut App) {
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
    }
}

pub fn dispatch_output_action(action: OutputAction, app: &mut App) {
    match action {
        // Select-all covers the captured-output buffer, so it only applies
        // while the cursor is in it; on a monitor row there is nothing of
        // Cargo Port's own to select.
        OutputAction::SelectAll => match app.panes.output.visual_selection_permission() {
            VisualSelectionPermission::Denied => {},
            VisualSelectionPermission::CapturedOutput => {
                let live = app.inflight.owned_run().output().to_vec();
                app.panes.output.select_all(&live);
            },
        },
        // The preflight disables the compile monitor before this focused-pane
        // dispatcher runs. Stopping and clearing act only on Cargo Port's own
        // run, so both require its column to be selected.
        OutputAction::Cancel => match app.owned_column_selection() {
            OwnedColumnSelection::NotSelected => {},
            OwnedColumnSelection::Selected => cancel_owned_output(app),
        },
        OutputAction::KillSelectedBuild => app.request_selected_build_termination_confirmation(),
        OutputAction::TerminateOutputBuildSet => {
            app.request_output_build_set_termination_confirmation();
        },
    }
}

/// Stop the owned run if it is still running, and otherwise close its captured
/// output and hand focus back to Targets.
fn cancel_owned_output(app: &mut App) {
    if app.inflight.owned_run().is_running() {
        let _ = terminal::signal_owned_run(&mut app.inflight);
    } else if matches!(
        app.output_copy_availability(),
        OutputCopyAvailability::CapturedOutput
    ) {
        app.inflight.clear_owned_run_output();
        app.set_focus(FocusedPane::App(AppPaneId::Targets));
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::path::Path;
    #[cfg(unix)]
    use std::process::Child;
    #[cfg(unix)]
    use std::process::Command;

    use crossterm::event::MouseEvent;

    use super::*;
    use crate::build_monitor;
    use crate::build_monitor::BuildSessionId;
    use crate::build_monitor::ClassifiedRoot;
    use crate::build_monitor::FixtureRootOwnership;
    use crate::build_monitor::MonitorSessionOwnership;
    use crate::build_monitor::MonitorSnapshot;
    use crate::build_monitor::OwnedTerminationFixtureAuthorityInstallation;
    use crate::config::CargoPortConfig;
    use crate::config::LintIndicator;
    use crate::process_observation::identity;
    #[cfg(unix)]
    use crate::process_observation::identity::CurrentProcessIdentityObservation;
    use crate::process_observation::identity::ProcessIdentity;
    use crate::process_observation::identity::ProcessIncarnation;
    use crate::project::AbsolutePath;
    use crate::project::Package;
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::tui::app::ConfirmationModalState;
    use crate::tui::app::ConfirmationReadiness;
    use crate::tui::background::ProcessTerminatorReadinessForTest;
    use crate::tui::compile_visibility::CompileVisibilityState;
    use crate::tui::compile_visibility::MonitorScopeKey;
    use crate::tui::compile_visibility::MonitorScopeResolution;
    use crate::tui::input::editor_terminal;
    use crate::tui::panes::OutputMonitorHit;
    use crate::tui::panes::OutputPaneVisibility;
    #[cfg(unix)]
    use crate::tui::running_targets::RunningTargetTerminationCapability;
    use crate::tui::state::Inflight;
    use crate::tui::state::OwnedRunTermination;
    use crate::tui::state::inflight::OwnedRunFixture;
    use crate::tui::test_support;

    /// A child process this test owns, together with authority bound to the
    /// child lifetime. Drop always reaps it if the assertion path aborts.
    #[cfg(unix)]
    struct TestOwnedTerminationFixture {
        child:                  Child,
        termination_capability: RunningTargetTerminationCapability,
    }

    #[cfg(unix)]
    impl TestOwnedTerminationFixture {
        fn spawn() -> Self {
            let mut child = Command::new("/bin/sleep")
                .arg("30")
                .spawn()
                .unwrap_or_else(|error| panic!("the termination fixture should spawn: {error}"));
            let termination_capability = match identity::observe_current_process_identity(
                child.id(),
            ) {
                CurrentProcessIdentityObservation::Verified(verified_process_identity) => {
                    RunningTargetTerminationCapability::from_observed_identity(
                        verified_process_identity.into_process_identity(),
                    )
                },
                observation => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "the termination fixture should have a verified identity: {observation:?}"
                    );
                },
            };
            Self {
                child,
                termination_capability,
            }
        }

        fn termination_capability(&self) -> RunningTargetTerminationCapability {
            self.termination_capability.clone()
        }

        fn wait_for_termination(&mut self) {
            let status = self
                .child
                .wait()
                .unwrap_or_else(|error| panic!("the termination fixture should exit: {error}"));
            assert!(
                !status.success(),
                "the confirmation sends SIGTERM to the test-owned child"
            );
        }
    }

    #[cfg(unix)]
    impl Drop for TestOwnedTerminationFixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// The Cargo root Cargo Port launched, beside one run by someone else.
    const OWNED_ROOT_PID: u32 = 8100;
    const EXTERNAL_ROOT_PID: u32 = 8200;

    /// The test actor's identity, mirrored by the classified owned root.
    const SELECTED_BUILD_OWNED_ROOT_PID: u32 = 4242;
    const SELECTED_BUILD_EXTERNAL_ROOT_PID: u32 = 4243;
    const SELECTED_BUILD_COMPILER_CHILDREN: &[u32] = &[4244, 4245];

    /// One click target per column of a staged two-column monitor, with the
    /// owned run's output retained so the owned column has a body to select.
    struct StagedOutput {
        app:            App,
        owned_hit:      OutputMonitorHit,
        external_hit:   OutputMonitorHit,
        captured_lines: Vec<String>,
    }

    /// An app focused on the Output pane, showing one owned column and one
    /// external column over a scope the index resolved.
    fn staged_output() -> Option<StagedOutput> {
        let OwnedRunFixture::Built { inflight, producer } =
            Inflight::with_retained_output_and_next_run_queued("retained line")
        else {
            return None;
        };
        let monitor_snapshot = build_monitor::classified_monitor_snapshot_with_ownership(
            &[
                ClassifiedRoot {
                    root_pid:      OWNED_ROOT_PID,
                    compiler_pids: &[8101],
                },
                ClassifiedRoot {
                    root_pid:      EXTERNAL_ROOT_PID,
                    compiler_pids: &[8201],
                },
            ],
            &FixtureRootOwnership::OwnedRoot {
                root_pid:     OWNED_ROOT_PID,
                owned_run_id: producer,
            },
        )
        .ok()?;
        let compile_visibility_state = CompileVisibilityState::on_for_test(
            MonitorScopeResolution::Ready(MonitorScopeKey::for_test(AbsolutePath::from(
                Path::new("/tmp/cargo-port-esc-precedence"),
            ))),
        );

        let MonitorSnapshot::Fresh(monitor_data) = &monitor_snapshot else {
            return None;
        };
        let mut owned_hit = None;
        let mut external_hit = None;
        for (column_index, monitor_session_row) in monitor_data.session_rows().iter().enumerate() {
            let hit = OutputMonitorHit::Header {
                build_session_id: monitor_session_row.build_session_id().clone(),
                column_index,
            };
            match monitor_session_row.session_ownership() {
                MonitorSessionOwnership::Owned(_) => owned_hit = Some(hit),
                MonitorSessionOwnership::External => external_hit = Some(hit),
            }
        }
        let (owned_hit, external_hit) = (owned_hit?, external_hit?);
        let captured_lines = vec!["retained line".to_string()];

        let mut app = test_support::make_app(&[]);
        app.inflight = *inflight;
        app.compile_visibility_state = compile_visibility_state;
        app.build_monitor.show_for_test(monitor_snapshot);
        app.set_focus_to_pane(PaneId::Output);
        Some(StagedOutput {
            app,
            owned_hit,
            external_hit,
            captured_lines,
        })
    }

    /// One App-owned selected-build fixture. It owns every runtime resource,
    /// so teardown drops the App only after all test references have gone.
    struct SelectedBuildDispatchFixture {
        app:                 Option<App>,
        owned_header:        OutputMonitorHit,
        captured_output:     OutputMonitorHit,
        stale_session_id:    BuildSessionId,
        selected_session_id: BuildSessionId,
    }

    /// An App fixture with one fully actionable Output root and a selected
    /// project row, so the real output-build-set input route can freeze a
    /// confirmation without constructing authority in UI code.
    struct OutputBuildSetDispatchFixture {
        app:          Option<App>,
        owned_header: OutputMonitorHit,
    }

    /// The monitor rows an Output build-set fixture presents to the action.
    #[derive(Clone, Copy)]
    enum OutputBuildSetFixtureRows {
        OwnedOnly,
        OwnedAndObservedOnly,
    }

    /// Whether the selected-build fixture starts its owned process actor.
    #[derive(Clone, Copy)]
    enum OwnedTerminationActorFixture {
        /// The actor accepts the submitted termination token.
        Running,
        /// No actor is started, so token submission is refused synchronously.
        Unavailable,
    }

    impl SelectedBuildDispatchFixture {
        fn app(&self) -> &App {
            self.app
                .as_ref()
                .unwrap_or_else(|| panic!("selected-build App fixture should be live"))
        }

        fn app_mut(&mut self) -> &mut App {
            self.app
                .as_mut()
                .unwrap_or_else(|| panic!("selected-build App fixture should be live"))
        }
    }

    impl Drop for SelectedBuildDispatchFixture {
        fn drop(&mut self) {
            // This fixture intentionally retains no cloned runtime handles.
            // Taking `app` during fixture teardown closes its actor and
            // background endpoints before the remaining value fields drop.
            drop(self.app.take());
        }
    }

    impl OutputBuildSetDispatchFixture {
        fn app(&self) -> &App {
            self.app
                .as_ref()
                .unwrap_or_else(|| panic!("output-build-set App fixture should be live"))
        }

        fn app_mut(&mut self) -> &mut App {
            self.app
                .as_mut()
                .unwrap_or_else(|| panic!("output-build-set App fixture should be live"))
        }
    }

    impl Drop for OutputBuildSetDispatchFixture {
        fn drop(&mut self) { drop(self.app.take()); }
    }

    /// An actionable selected-build App fixture whose live actor either serves
    /// a request or remains unavailable for the synchronous-refusal path.
    fn selected_build_dispatch_fixture(
        owned_termination_actor_fixture: OwnedTerminationActorFixture,
    ) -> SelectedBuildDispatchFixture {
        let OwnedRunFixture::Live { inflight } =
            Inflight::with_live_owned_run_output("selected build output")
        else {
            panic!("the live owned-run fixture should build");
        };
        let mut app = test_support::make_app(&[]);
        app.inflight = *inflight;
        let OwnedRunTermination::Available {
            owned_run_id,
            owned_run_termination_token,
        } = app.inflight.owned_run_termination()
        else {
            panic!("the live owned-run fixture should issue one termination token");
        };
        if matches!(
            owned_termination_actor_fixture,
            OwnedTerminationActorFixture::Running
        ) {
            app.inflight.start_owned_run_process_actor(owned_run_id);
        }

        let monitor_snapshot = build_monitor::classified_monitor_snapshot_with_ownership(
            &[
                ClassifiedRoot {
                    root_pid:      SELECTED_BUILD_EXTERNAL_ROOT_PID,
                    compiler_pids: &[],
                },
                ClassifiedRoot {
                    root_pid:      SELECTED_BUILD_OWNED_ROOT_PID,
                    compiler_pids: SELECTED_BUILD_COMPILER_CHILDREN,
                },
            ],
            &FixtureRootOwnership::OwnedRoot {
                root_pid: SELECTED_BUILD_OWNED_ROOT_PID,
                owned_run_id,
            },
        )
        .unwrap_or_else(|error| panic!("selected-build fixture snapshot should classify: {error}"));
        let MonitorSnapshot::Fresh(monitor_data) = &monitor_snapshot else {
            panic!("selected-build fixture should have fresh monitor data");
        };
        let mut owned_header = None;
        let mut selected_session_id = None;
        for (column_index, monitor_session_row) in monitor_data.session_rows().iter().enumerate() {
            if matches!(
                monitor_session_row.session_ownership(),
                MonitorSessionOwnership::Owned(current_owned_run_id)
                    if current_owned_run_id == owned_run_id
            ) {
                let build_session_id = monitor_session_row.build_session_id().clone();
                owned_header = Some(OutputMonitorHit::Header {
                    build_session_id: build_session_id.clone(),
                    column_index,
                });
                selected_session_id = Some(build_session_id);
            }
        }
        let owned_header = owned_header
            .unwrap_or_else(|| panic!("fixture should include the owned monitor column"));
        let selected_session_id = selected_session_id
            .unwrap_or_else(|| panic!("fixture should retain the owned session identity"));
        let stale_session_id = BuildSessionId::for_test(ProcessIncarnation::for_test(
            ProcessIdentity::for_test(SELECTED_BUILD_EXTERNAL_ROOT_PID, 99),
            "stale cargo build",
        ));
        // Enable this through the App so every normal input event re-resolves
        // the same scope it started with. Assigning a synthetic scope here
        // would let `handle_event` replace the staged snapshot after Alt-K.
        app.toggle_compile_visibility(Instant::now());
        let authorized_session_id = match app
            .build_monitor
            .show_with_owned_termination_authority_for_test(
                monitor_snapshot,
                owned_run_id,
                owned_run_termination_token,
            ) {
            OwnedTerminationFixtureAuthorityInstallation::Installed(build_session_id) => {
                build_session_id
            },
            OwnedTerminationFixtureAuthorityInstallation::SnapshotNotActionable => {
                panic!("selected-build fixture must stage an actionable monitor snapshot")
            },
            OwnedTerminationFixtureAuthorityInstallation::MatchingOwnedSessionAbsent => {
                panic!("selected-build fixture must stage the matching owned monitor row")
            },
        };
        assert_eq!(
            authorized_session_id, selected_session_id,
            "the monitor authority belongs to the same current owned column the fixture selects"
        );
        app.set_process_terminator_readiness_for_test(ProcessTerminatorReadinessForTest::Available);
        app.set_focus_to_pane(PaneId::Output);
        app.framework.toasts = tui_pane::Toasts::default();
        SelectedBuildDispatchFixture {
            app: Some(app),
            owned_header,
            captured_output: OutputMonitorHit::CapturedOutput {
                producer: owned_run_id,
                row:      0,
            },
            stale_session_id,
            selected_session_id,
        }
    }

    fn output_build_set_dispatch_fixture(
        owned_termination_actor_fixture: OwnedTerminationActorFixture,
        output_build_set_fixture_rows: OutputBuildSetFixtureRows,
    ) -> OutputBuildSetDispatchFixture {
        let OwnedRunFixture::Live { inflight } =
            Inflight::with_live_owned_run_output("output build-set output")
        else {
            panic!("the live owned-run fixture should build");
        };
        let project = RootItem::Rust(RustProject::Package(Package {
            path: AbsolutePath::from(Path::new("/tmp/output-build-set-context")),
            name: Some("output-build-set-context".to_string()),
            ..Package::default()
        }));
        let mut app = test_support::make_app(&[project]);
        app.inflight = *inflight;
        let OwnedRunTermination::Available {
            owned_run_id,
            owned_run_termination_token,
        } = app.inflight.owned_run_termination()
        else {
            panic!("the live owned-run fixture should issue one termination token");
        };
        if matches!(
            owned_termination_actor_fixture,
            OwnedTerminationActorFixture::Running
        ) {
            app.inflight.start_owned_run_process_actor(owned_run_id);
        }
        let classified_roots = match output_build_set_fixture_rows {
            OutputBuildSetFixtureRows::OwnedOnly => vec![ClassifiedRoot {
                root_pid:      SELECTED_BUILD_OWNED_ROOT_PID,
                compiler_pids: SELECTED_BUILD_COMPILER_CHILDREN,
            }],
            OutputBuildSetFixtureRows::OwnedAndObservedOnly => vec![
                ClassifiedRoot {
                    root_pid:      SELECTED_BUILD_OWNED_ROOT_PID,
                    compiler_pids: SELECTED_BUILD_COMPILER_CHILDREN,
                },
                ClassifiedRoot {
                    root_pid:      SELECTED_BUILD_EXTERNAL_ROOT_PID,
                    compiler_pids: &[],
                },
            ],
        };
        let monitor_snapshot = build_monitor::classified_monitor_snapshot_with_ownership(
            &classified_roots,
            &FixtureRootOwnership::OwnedRoot {
                root_pid: SELECTED_BUILD_OWNED_ROOT_PID,
                owned_run_id,
            },
        )
        .unwrap_or_else(|error| {
            panic!("output-build-set fixture snapshot should classify: {error}")
        });
        let MonitorSnapshot::Fresh(monitor_data) = &monitor_snapshot else {
            panic!("output-build-set fixture should have fresh monitor data");
        };
        let owned_header = monitor_data
            .session_rows()
            .iter()
            .enumerate()
            .find_map(|(column_index, monitor_session_row)| {
                matches!(
                    monitor_session_row.session_ownership(),
                    MonitorSessionOwnership::Owned(current_owned_run_id)
                        if current_owned_run_id == owned_run_id
                )
                .then_some(OutputMonitorHit::Header {
                    build_session_id: monitor_session_row.build_session_id().clone(),
                    column_index,
                })
            })
            .unwrap_or_else(|| {
                panic!("output-build-set fixture should retain the owned monitor column")
            });
        app.compile_visibility_state = CompileVisibilityState::on_for_test(
            MonitorScopeResolution::Ready(MonitorScopeKey::for_test(AbsolutePath::from(
                Path::new("/tmp/output-build-set-context"),
            ))),
        );
        let OwnedTerminationFixtureAuthorityInstallation::Installed(_) = app
            .build_monitor
            .show_with_owned_termination_authority_for_test(
                monitor_snapshot,
                owned_run_id,
                owned_run_termination_token,
            )
        else {
            panic!("the output-build-set fixture must install the shown root authority");
        };
        app.set_process_terminator_readiness_for_test(ProcessTerminatorReadinessForTest::Available);
        app.framework.toasts = tui_pane::Toasts::default();
        OutputBuildSetDispatchFixture {
            app: Some(app),
            owned_header,
        }
    }

    fn assert_selected_build_confirmation(app: &App) {
        match app.confirmation_modal_state() {
            ConfirmationModalState::Open {
                action:
                    ConfirmAction::TerminateSelectedBuild {
                        selected_build_termination_confirmation_display,
                        selected_build_termination_authorization: _,
                    },
                readiness: ConfirmationReadiness::Ready,
            } => {
                assert_eq!(
                    selected_build_termination_confirmation_display.root_pid(),
                    SELECTED_BUILD_OWNED_ROOT_PID,
                    "the confirmation names the exact selected root"
                );
                assert_eq!(
                    selected_build_termination_confirmation_display.compiler_child_count(),
                    SELECTED_BUILD_COMPILER_CHILDREN.len(),
                    "the confirmation keeps the selected root's compiler-child count"
                );
            },
            ConfirmationModalState::Closed => {
                panic!("Alt-K should retain a ready selected-build confirmation")
            },
            ConfirmationModalState::Open { .. } => {
                panic!("the open confirmation should retain selected-build authorization")
            },
        }
    }

    fn assert_output_build_set_confirmation(app: &App) {
        match app.confirmation_modal_state() {
            ConfirmationModalState::Open {
                action:
                    ConfirmAction::TerminateOutputBuildSet {
                        output_build_set_termination_confirmation_display,
                        output_build_set_termination_authorization: _,
                    },
                readiness: ConfirmationReadiness::Ready,
            } => {
                let target_summaries =
                    output_build_set_termination_confirmation_display.target_summaries();
                assert_eq!(target_summaries.len(), 1);
                assert_eq!(
                    target_summaries[0].root_pid(),
                    SELECTED_BUILD_OWNED_ROOT_PID,
                    "the modal keeps the exact Output root frozen at confirmation"
                );
            },
            ConfirmationModalState::Closed => {
                panic!("the output-build-set action should retain a ready confirmation")
            },
            ConfirmationModalState::Open { .. } => {
                panic!("the open confirmation should retain output-build-set authority")
            },
        }
    }

    fn press(app: &mut App, code: KeyCode) {
        handle_event(app, &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn press_with_modifiers(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        handle_event(app, &Event::Key(KeyEvent::new(code, modifiers)));
    }

    fn press_alt_k(app: &mut App) {
        press_with_modifiers(app, KeyCode::Char('k'), KeyModifiers::ALT);
    }

    #[test]
    fn confirmation_escape_keeps_a_live_owned_run_and_its_output_open() {
        let mut app = test_support::make_app(&[]);
        let OwnedRunFixture::Live { inflight } =
            Inflight::with_live_owned_run_output("live output")
        else {
            panic!("the live owned-run fixture builds");
        };
        app.inflight = *inflight;
        let output_before_escape = app.inflight.owned_run().output().to_vec();
        assert!(
            app.inflight.owned_run().is_running(),
            "the fixture has a live owned run before Esc"
        );
        app.open_ready_confirmation_for_test(ConfirmAction::Clean(AbsolutePath::from(Path::new(
            "/tmp/confirmation-precedence",
        ))));

        press(&mut app, KeyCode::Esc);

        assert!(
            !app.confirmation_is_open(),
            "Esc dismisses the confirmation before Output can cancel"
        );
        assert!(
            matches!(
                app.inflight.owned_run_termination(),
                crate::tui::state::OwnedRunTermination::Available { .. }
            ),
            "Esc does not submit an owned-run termination while confirmation is open"
        );
        assert!(
            app.inflight.owned_run().is_running(),
            "the active owned run is not stopped while confirmation consumes Esc"
        );
        assert_eq!(
            app.inflight.owned_run().output(),
            output_before_escape.as_slice(),
            "the live Output remains open while the confirmation consumes Esc"
        );
    }

    #[test]
    fn confirmation_consumes_unrelated_keys_and_mouse_input() {
        let mut app = test_support::make_app(&[]);
        app.open_ready_confirmation_for_test(ConfirmAction::Clean(AbsolutePath::from(Path::new(
            "/tmp/confirmation-input",
        ))));

        press_with_modifiers(&mut app, KeyCode::Char('C'), KeyModifiers::SHIFT);

        assert!(
            app.confirmation_is_open(),
            "a global action key leaves the confirmation unchanged"
        );
        assert!(matches!(
            app.compile_visibility_state,
            CompileVisibilityState::Off
        ));
        assert!(app.mouse_pos.is_none(), "test setup has no mouse position");
        handle_event(
            &mut app,
            &Event::Mouse(MouseEvent {
                kind:      MouseEventKind::ScrollDown,
                column:    7,
                row:       9,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert!(
            app.confirmation_is_open(),
            "mouse events leave the confirmation unchanged"
        );
        assert!(
            app.mouse_pos.is_none(),
            "an open confirmation ignores mouse input"
        );
    }

    #[test]
    fn n_and_escape_dismiss_a_confirmation_without_executing_it() {
        for key in [KeyCode::Char('n'), KeyCode::Esc] {
            let mut app = test_support::make_app(&[]);
            app.open_ready_confirmation_for_test(ConfirmAction::Clean(AbsolutePath::from(
                Path::new("/tmp/confirmation-dismiss"),
            )));

            press(&mut app, key);

            assert!(
                !app.confirmation_is_open(),
                "{key:?} dismisses the confirmation"
            );
            assert!(
                app.inflight.pending_cleans_mut().is_empty(),
                "{key:?} does not start the clean action"
            );
        }
    }

    #[test]
    fn verifying_clean_confirmation_consumes_y_without_closing() {
        let mut app = test_support::make_app(&[]);
        app.open_verifying_clean_confirmation_for_test(AbsolutePath::from(Path::new(
            "/tmp/confirmation-verifying",
        )));

        press(&mut app, KeyCode::Char('y'));

        assert!(
            app.confirmation_is_open(),
            "y cannot accept a confirmation still verifying metadata"
        );
        assert!(
            app.inflight.pending_cleans_mut().is_empty(),
            "the clean action remains unstarted while verification is pending"
        );
    }

    #[test]
    fn ready_y_accepts_clean_confirmation_actions() {
        let temp_dir_result = tempfile::tempdir();
        assert!(
            temp_dir_result.is_ok(),
            "create confirmation test directory: {temp_dir_result:?}"
        );
        let Ok(temp_dir) = temp_dir_result else {
            return;
        };
        let clean = temp_dir.path().join("clean");
        let group_primary = temp_dir.path().join("group-primary");
        let group_linked = temp_dir.path().join("group-linked");
        for project_path in [&clean, &group_primary, &group_linked] {
            let create_target_result = std::fs::create_dir_all(project_path.join("target"));
            assert!(
                create_target_result.is_ok(),
                "create clean target directory: {create_target_result:?}"
            );
            if create_target_result.is_err() {
                return;
            }
        }
        let mut app = test_support::make_app(&[]);

        app.open_ready_confirmation_for_test(ConfirmAction::Clean(AbsolutePath::from(clean)));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.inflight.pending_cleans_mut().len(), 1);
        assert!(!app.confirmation_is_open());

        app.open_ready_confirmation_for_test(ConfirmAction::CleanGroup {
            primary: AbsolutePath::from(group_primary),
            linked:  vec![AbsolutePath::from(group_linked)],
        });
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.inflight.pending_cleans_mut().len(), 3);
        assert!(!app.confirmation_is_open());
    }

    #[cfg(unix)]
    #[test]
    fn ready_y_terminates_the_test_owned_kill_target() {
        let mut app = test_support::make_app(&[]);
        let mut termination_fixture = TestOwnedTerminationFixture::spawn();
        let pid = termination_fixture.child.id();
        app.open_ready_confirmation_for_test(ConfirmAction::KillTarget {
            label: "test-owned target".to_string(),
            pid,
            create_time: 0,
            termination_capability: termination_fixture.termination_capability(),
        });

        press(&mut app, KeyCode::Char('y'));

        assert!(
            !app.confirmation_is_open(),
            "y accepts the kill confirmation"
        );
        termination_fixture.wait_for_termination();
    }

    #[test]
    fn alt_k_retains_selected_authorization_until_an_available_worker_submits_it() {
        let mut fixture = selected_build_dispatch_fixture(OwnedTerminationActorFixture::Running);
        let owned_header = fixture.owned_header.clone();
        fixture.app_mut().panes.output.focus_hit(&owned_header);

        press_alt_k(fixture.app_mut());
        assert_selected_build_confirmation(fixture.app());

        fixture
            .app_mut()
            .set_process_terminator_readiness_for_test(ProcessTerminatorReadinessForTest::Starting);
        press(fixture.app_mut(), KeyCode::Char('y'));
        assert_selected_build_confirmation(fixture.app());

        fixture.app_mut().set_process_terminator_readiness_for_test(
            ProcessTerminatorReadinessForTest::Unavailable,
        );
        press(fixture.app_mut(), KeyCode::Char('y'));
        assert_selected_build_confirmation(fixture.app());

        fixture.app_mut().set_process_terminator_readiness_for_test(
            ProcessTerminatorReadinessForTest::Available,
        );
        press(fixture.app_mut(), KeyCode::Char('y'));

        assert!(
            !fixture.app().confirmation_is_open(),
            "an available worker consumes the retained selected authorization"
        );
        assert!(
            matches!(
                fixture.app().inflight.owned_run_termination(),
                OwnedRunTermination::RequestPending { .. }
            ),
            "the selected authorization immediately fans its owned token into Inflight"
        );
    }

    #[test]
    fn output_build_set_dispatch_retains_its_exact_confirmation_until_the_worker_is_ready() {
        let mut fixture = output_build_set_dispatch_fixture(
            OwnedTerminationActorFixture::Running,
            OutputBuildSetFixtureRows::OwnedOnly,
        );

        dispatch_output_action(OutputAction::TerminateOutputBuildSet, fixture.app_mut());
        assert_output_build_set_confirmation(fixture.app());

        assert!(
            handle_confirm_key(fixture.app_mut(), KeyCode::Char('x')),
            "the confirmation layer consumes an unrelated Output shortcut first"
        );
        assert_output_build_set_confirmation(fixture.app());

        fixture
            .app_mut()
            .set_process_terminator_readiness_for_test(ProcessTerminatorReadinessForTest::Starting);
        assert!(handle_confirm_key(fixture.app_mut(), KeyCode::Char('y')));
        assert_output_build_set_confirmation(fixture.app());

        fixture.app_mut().set_process_terminator_readiness_for_test(
            ProcessTerminatorReadinessForTest::Unavailable,
        );
        assert!(handle_confirm_key(fixture.app_mut(), KeyCode::Char('y')));
        assert_output_build_set_confirmation(fixture.app());

        fixture.app_mut().set_process_terminator_readiness_for_test(
            ProcessTerminatorReadinessForTest::Available,
        );
        assert!(handle_confirm_key(fixture.app_mut(), KeyCode::Char('y')));

        assert!(
            !fixture.app().confirmation_is_open(),
            "an available worker consumes the retained output-build-set authority"
        );
        assert!(
            matches!(
                fixture.app().inflight.owned_run_termination(),
                OwnedRunTermination::RequestPending { .. }
            ),
            "the shared submit path immediately fans the owned token into Inflight"
        );
    }

    #[test]
    fn output_build_set_completion_uses_aggregate_wording_once() {
        let mut fixture = output_build_set_dispatch_fixture(
            OwnedTerminationActorFixture::Unavailable,
            OutputBuildSetFixtureRows::OwnedOnly,
        );
        dispatch_output_action(OutputAction::TerminateOutputBuildSet, fixture.app_mut());
        assert_output_build_set_confirmation(fixture.app());

        press(fixture.app_mut(), KeyCode::Char('y'));

        assert!(
            !fixture.app().confirmation_is_open(),
            "the synchronous refusal completes the accepted output-build-set confirmation"
        );
        let output_completion_toasts = fixture
            .app()
            .framework
            .toasts
            .active_now()
            .iter()
            .filter(|toast| toast.title() == "Output build-set termination incomplete")
            .count();
        assert_eq!(
            output_completion_toasts, 1,
            "the output-set aggregate transition has its own wording and occurs once"
        );

        fixture.app_mut().poll_background();
        assert_eq!(
            fixture
                .app()
                .framework
                .toasts
                .active_now()
                .iter()
                .filter(|toast| toast.title() == "Output build-set termination incomplete")
                .count(),
            1,
            "retained terminal records do not replay output-set completion"
        );
    }

    #[test]
    fn output_build_set_dispatch_refuses_mixed_rows_without_consuming_owned_authority() {
        let mut fixture = output_build_set_dispatch_fixture(
            OwnedTerminationActorFixture::Unavailable,
            OutputBuildSetFixtureRows::OwnedAndObservedOnly,
        );
        let MonitorSnapshot::Fresh(monitor_data) = fixture.app().build_monitor.monitor_snapshot()
        else {
            panic!("mixed Output fixture should keep a fresh monitor snapshot");
        };
        assert_eq!(
            monitor_data.session_rows().len(),
            2,
            "the mixed Output fixture shows both the owned and observed roots"
        );
        assert!(
            monitor_data
                .session_rows()
                .iter()
                .any(|monitor_session_row| {
                    matches!(
                        monitor_session_row.session_ownership(),
                        MonitorSessionOwnership::Owned(_)
                    )
                }),
            "one shown root owns current termination authority"
        );
        assert!(
            monitor_data
                .session_rows()
                .iter()
                .any(|monitor_session_row| {
                    matches!(
                        monitor_session_row.session_ownership(),
                        MonitorSessionOwnership::External
                    )
                }),
            "one shown root is observed only"
        );

        dispatch_output_action(OutputAction::TerminateOutputBuildSet, fixture.app_mut());

        assert!(
            !fixture.app().confirmation_is_open(),
            "the mixed Output set refuses before opening a confirmation"
        );
        assert!(
            matches!(
                fixture.app().inflight.owned_run_termination(),
                OwnedRunTermination::Available { .. }
            ),
            "the refusal does not submit the owned termination token"
        );

        let owned_header = fixture.owned_header.clone();
        fixture.app_mut().panes.output.focus_hit(&owned_header);
        dispatch_output_action(OutputAction::KillSelectedBuild, fixture.app_mut());
        assert_selected_build_confirmation(fixture.app());
    }

    #[test]
    fn captured_output_targets_its_owned_root_and_unattributed_or_stale_cursor_refuses() {
        let mut captured_fixture =
            selected_build_dispatch_fixture(OwnedTerminationActorFixture::Unavailable);
        let captured_output = captured_fixture.captured_output.clone();
        captured_fixture
            .app_mut()
            .panes
            .output
            .focus_hit(&captured_output);
        press_alt_k(captured_fixture.app_mut());
        assert_selected_build_confirmation(captured_fixture.app());

        let mut unattributed_fixture =
            selected_build_dispatch_fixture(OwnedTerminationActorFixture::Unavailable);
        unattributed_fixture
            .app_mut()
            .panes
            .output
            .focus_hit(&OutputMonitorHit::Unattributed {
                compile_activity_id: crate::build_monitor::CompileActivityId::for_test(
                    ProcessIncarnation::for_test(
                        ProcessIdentity::for_test(SELECTED_BUILD_EXTERNAL_ROOT_PID, 98),
                        "unattributed rustc",
                    ),
                ),
                row_index:           0,
            });
        press_alt_k(unattributed_fixture.app_mut());
        assert!(
            !unattributed_fixture.app().confirmation_is_open(),
            "an unattributed row cannot fall through to the first build column"
        );
        assert_eq!(
            unattributed_fixture
                .app()
                .build_monitor
                .selected_termination_availability(&unattributed_fixture.selected_session_id),
            crate::build_monitor::SelectedBuildTerminationAvailability::Available,
            "an unattributed attempt leaves the owned authority untouched"
        );

        let mut stale_fixture =
            selected_build_dispatch_fixture(OwnedTerminationActorFixture::Unavailable);
        let stale_session_id = stale_fixture.stale_session_id.clone();
        stale_fixture
            .app_mut()
            .panes
            .output
            .focus_hit(&OutputMonitorHit::Header {
                build_session_id: stale_session_id,
                column_index:     0,
            });
        press_alt_k(stale_fixture.app_mut());
        assert!(
            !stale_fixture.app().confirmation_is_open(),
            "a stale session identity cannot target the current first column"
        );
        assert_eq!(
            stale_fixture
                .app()
                .build_monitor
                .selected_termination_availability(&stale_fixture.selected_session_id),
            crate::build_monitor::SelectedBuildTerminationAvailability::Available,
            "a stale attempt leaves the owned authority untouched"
        );
    }

    #[test]
    fn captured_output_replacement_before_render_refuses_alt_k_and_preserves_current_authority() {
        let mut fixture =
            selected_build_dispatch_fixture(OwnedTerminationActorFixture::Unavailable);
        let captured_output = fixture.captured_output.clone();
        let retained_producer = match &captured_output {
            OutputMonitorHit::CapturedOutput { producer, .. } => *producer,
            OutputMonitorHit::Header { .. }
            | OutputMonitorHit::Activity { .. }
            | OutputMonitorHit::Unattributed { .. }
            | OutputMonitorHit::EmptyMonitor => {
                panic!("the selected-build fixture should retain a captured-output hit")
            },
        };
        fixture.app_mut().panes.output.focus_hit(&captured_output);

        let app = fixture.app_mut();
        app.inflight
            .set_example_running(Some("replacement selected build output".to_string()));
        let OwnedRunTermination::Available {
            owned_run_id,
            owned_run_termination_token,
        } = app.inflight.owned_run_termination()
        else {
            panic!("the replacement owned run should issue one termination token");
        };
        assert_ne!(
            retained_producer, owned_run_id,
            "the replacement run must not reuse the retained output producer"
        );
        let monitor_snapshot = build_monitor::classified_monitor_snapshot_with_ownership(
            &[
                ClassifiedRoot {
                    root_pid:      SELECTED_BUILD_EXTERNAL_ROOT_PID,
                    compiler_pids: &[],
                },
                ClassifiedRoot {
                    root_pid:      SELECTED_BUILD_OWNED_ROOT_PID,
                    compiler_pids: SELECTED_BUILD_COMPILER_CHILDREN,
                },
            ],
            &FixtureRootOwnership::OwnedRoot {
                root_pid: SELECTED_BUILD_OWNED_ROOT_PID,
                owned_run_id,
            },
        )
        .unwrap_or_else(|error| {
            panic!("replacement selected-build fixture snapshot should classify: {error}")
        });
        let replacement_session_id = match app
            .build_monitor
            .show_with_owned_termination_authority_for_test(
                monitor_snapshot,
                owned_run_id,
                owned_run_termination_token,
            ) {
            OwnedTerminationFixtureAuthorityInstallation::Installed(build_session_id) => {
                build_session_id
            },
            OwnedTerminationFixtureAuthorityInstallation::SnapshotNotActionable => {
                panic!(
                    "replacement selected-build fixture must stage an actionable monitor snapshot"
                )
            },
            OwnedTerminationFixtureAuthorityInstallation::MatchingOwnedSessionAbsent => {
                panic!(
                    "replacement selected-build fixture must stage the matching owned monitor row"
                )
            },
        };

        press_alt_k(fixture.app_mut());

        assert!(
            !fixture.app().confirmation_is_open(),
            "Alt-K cannot select the replacement owned column before cursor reconciliation"
        );
        assert_eq!(
            fixture
                .app()
                .build_monitor
                .selected_termination_availability(&replacement_session_id),
            crate::build_monitor::SelectedBuildTerminationAvailability::Available,
            "the refused key leaves the replacement owned authority available"
        );
    }

    #[test]
    fn synchronous_owned_actor_refusal_completes_and_toasts_once() {
        let mut fixture =
            selected_build_dispatch_fixture(OwnedTerminationActorFixture::Unavailable);
        let owned_header = fixture.owned_header.clone();
        fixture.app_mut().panes.output.focus_hit(&owned_header);
        press_alt_k(fixture.app_mut());
        assert_selected_build_confirmation(fixture.app());
        let toast_count_before_acceptance = fixture.app().framework.toasts.active_now().len();

        press(fixture.app_mut(), KeyCode::Char('y'));

        assert!(
            !fixture.app().confirmation_is_open(),
            "the synchronous actor refusal completes the accepted confirmation"
        );
        let terminal_records: Vec<_> = fixture
            .app()
            .build_monitor
            .termination_lifecycle_registry()
            .terminal_records()
            .collect();
        assert_eq!(terminal_records.len(), 1);
        assert_eq!(
            terminal_records[0].session_completion(),
            crate::build_monitor::BuildTerminationSessionCompletion::RetryUnavailable,
            "the retained record preserves actor submission refusal"
        );
        let completion_toasts = fixture
            .app()
            .framework
            .toasts
            .active_now()
            .iter()
            .filter(|toast| toast.title() == "Build termination incomplete")
            .count();
        assert_eq!(
            completion_toasts, 1,
            "the terminal transition produces one toast"
        );
        assert_eq!(
            fixture.app().framework.toasts.active_now().len(),
            toast_count_before_acceptance + 1,
            "the synchronous completion adds exactly one toast"
        );

        fixture.app_mut().poll_background();
        assert_eq!(
            fixture
                .app()
                .framework
                .toasts
                .active_now()
                .iter()
                .filter(|toast| toast.title() == "Build termination incomplete")
                .count(),
            1,
            "polling the persistent lifecycle registry cannot replay the completion toast"
        );
    }

    #[test]
    fn ready_y_pauses_the_selected_lint_project_and_all_lints() {
        let project_path = AbsolutePath::from(Path::new("/tmp/cargo-port-lint-project"));
        let project = RootItem::Rust(RustProject::Package(Package {
            path: project_path.clone(),
            name: Some("lint-project".to_string()),
            ..Package::default()
        }));
        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config.lint.enabled = LintIndicator::Enabled;
        cargo_port_config.lint.include = vec!["lint-project".to_string()];

        let mut project_app = test_support::make_app_with_lint_runtime(
            std::slice::from_ref(&project),
            &cargo_port_config,
        );
        let project_runtime = project_app
            .lint
            .runtime()
            .cloned()
            .expect("the lint-enabled fixture owns an active runtime");
        assert!(
            !project_runtime.is_project_paused(&project_path),
            "the selected lint project starts active"
        );
        assert_eq!(
            project_app.project_list.selected_project_path(),
            Some(project_path.as_path()),
            "the lint-enabled fixture selects the active project"
        );
        press(&mut project_app, KeyCode::Char(' '));
        assert!(matches!(
            project_app.confirmation_modal_state(),
            crate::tui::app::ConfirmationModalState::Open {
                action: ConfirmAction::PauseLintProject(path),
                ..
            } if path == &project_path
        ));

        press(&mut project_app, KeyCode::Char('y'));

        assert!(
            !project_app.confirmation_is_open(),
            "y accepts the selected-project pause confirmation"
        );
        assert!(
            project_runtime.is_project_paused(&project_path),
            "the accepted confirmation pauses the selected active lint project"
        );
        drop(project_runtime);
        drop(project_app);

        let mut all_lints_app =
            test_support::make_app_with_lint_runtime(&[project], &cargo_port_config);
        let all_lints_runtime = all_lints_app
            .lint
            .runtime()
            .cloned()
            .expect("the lint-enabled fixture owns an active runtime");
        assert!(
            !all_lints_runtime.is_globally_paused(),
            "all lint work starts active"
        );
        press_with_modifiers(&mut all_lints_app, KeyCode::Char(' '), KeyModifiers::SHIFT);
        assert!(matches!(
            all_lints_app.confirmation_modal_state(),
            crate::tui::app::ConfirmationModalState::Open {
                action: ConfirmAction::PauseAllLints,
                ..
            }
        ));

        press(&mut all_lints_app, KeyCode::Char('y'));

        assert!(
            !all_lints_app.confirmation_is_open(),
            "y accepts the all-lints pause confirmation"
        );
        assert!(
            all_lints_runtime.is_globally_paused(),
            "the accepted confirmation pauses active lint work globally"
        );
        drop(all_lints_runtime);
        drop(all_lints_app);
    }

    /// Esc on the owned column collapses the visual selection first: the run is
    /// not stopped and the output is not closed while a range is up.
    #[test]
    fn esc_on_the_owned_column_collapses_the_visual_selection_first() {
        let staged = staged_output();
        assert!(staged.is_some(), "the staged monitor fixture builds");
        let Some(mut staged) = staged else { return };

        staged.app.panes.output.focus_hit(&staged.owned_hit);
        staged
            .app
            .panes
            .output
            .toggle_visual(&staged.captured_lines);
        assert!(staged.app.panes.output.selection().is_visual());

        assert!(matches!(
            classify_output_cancel_preflight(
                &staged.app,
                KeyCode::Esc,
                &KeyBind::from(KeyCode::Esc)
            ),
            OutputCancelPreflight::ExitVisualSelection
        ));
    }

    #[test]
    fn esc_closes_monitor_opened_from_the_project_list() {
        let mut app = test_support::make_app(&[]);

        press_with_modifiers(&mut app, KeyCode::Char('C'), KeyModifiers::SHIFT);
        assert!(app.compile_visibility_state.is_on());
        assert_eq!(app.output_pane_visibility(), OutputPaneVisibility::Visible);

        press(&mut app, KeyCode::Esc);

        assert!(matches!(
            app.compile_visibility_state,
            CompileVisibilityState::Off
        ));
        assert_eq!(app.output_pane_visibility(), OutputPaneVisibility::Hidden);
    }

    /// Esc on an external column disables the monitor without changing the
    /// stored owned-column selection or captured output.
    #[test]
    fn esc_on_an_external_column_closes_only_the_monitor() {
        let staged = staged_output();
        assert!(staged.is_some(), "the staged monitor fixture builds");
        let Some(mut staged) = staged else { return };

        staged.app.panes.output.focus_hit(&staged.owned_hit);
        staged
            .app
            .panes
            .output
            .toggle_visual(&staged.captured_lines);
        staged.app.panes.output.focus_hit(&staged.external_hit);
        assert!(
            staged.app.panes.output.selection().is_visual(),
            "the selection is stored, just not on screen"
        );
        let captured_output = staged.app.inflight.example_output().to_vec();

        press(&mut staged.app, KeyCode::Esc);

        assert!(matches!(
            staged.app.compile_visibility_state,
            CompileVisibilityState::Off
        ));
        assert!(staged.app.panes.output.selection().is_visual());
        assert_eq!(staged.app.inflight.example_output(), captured_output);
    }

    #[test]
    fn terminal_shell_command_leaves_command_without_path_placeholder_unchanged() {
        assert_eq!(
            editor_terminal::terminal_shell_command(
                "open -a Terminal .",
                Path::new("/tmp/my project")
            ),
            "open -a Terminal ."
        );
    }

    #[test]
    fn terminal_shell_command_substitutes_shell_escaped_path() {
        assert_eq!(
            editor_terminal::terminal_shell_command(
                "cd {path} && exec zsh",
                Path::new("/tmp/my project")
            ),
            "cd '/tmp/my project' && exec zsh"
        );
    }

    #[test]
    fn terminal_shell_command_escapes_single_quotes() {
        assert_eq!(
            editor_terminal::terminal_shell_command("cd {path}", Path::new("/tmp/bob's project")),
            "cd '/tmp/bob'\\''s project'"
        );
    }

    #[test]
    fn framework_overlay_editor_target_path_uses_settings_config_path() {
        let config_path = Path::new("/tmp/config.toml");

        assert_eq!(
            editor_terminal::framework_overlay_editor_target_path(
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
            editor_terminal::framework_overlay_editor_target_path(
                FrameworkOverlayId::Keymap,
                None,
                Some(keymap_path)
            ),
            Some(AbsolutePath::from(keymap_path))
        );
    }
}
