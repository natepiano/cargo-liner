use std::path::Path;
use std::time::Instant;

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::MouseButton;
use crossterm::event::MouseEventKind;
use ratatui::layout::Position;

use super::app::App;
use super::app::ConfirmAction;
use super::app::PendingClean;
use super::detail;
use super::finder;
use super::settings;
use super::types::PaneId;
use crate::project;
use crate::project::ProjectLanguage;

pub(super) fn handle_event(app: &mut App, event: &Event) {
    let started = Instant::now();
    match event {
        Event::Key(key) => handle_key_event(app, key.code),
        Event::Mouse(mouse) => handle_mouse_event(app, mouse.kind, mouse.column, mouse.row),
        _ => {},
    }

    app.sync_selected_project();

    crate::perf_log::log_duration(
        "input_event",
        started.elapsed(),
        &format!(
            "kind={} focus={} scan_complete={} selected={}",
            event_label(event),
            pane_label(app.focused_pane),
            app.is_scan_complete(),
            app.selected_project()
                .map_or("-", |project| project.path.as_str())
        ),
        crate::perf_log::slow_input_event_threshold_ms(),
    );
}

fn handle_key_event(app: &mut App, key: KeyCode) {
    let key = normalize_key(app, key);
    if key == KeyCode::Esc && app.example_running.is_some() {
        let pid = *app
            .example_child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(pid) = pid {
            let _ = std::process::Command::new("kill")
                .arg(pid.to_string())
                .output();
        }
        app.example_running = None;
        app.example_output.push("── killed ──".to_string());
        app.mark_terminal_dirty();
        return;
    }
    if key == KeyCode::Esc && !app.example_output.is_empty() {
        app.example_output.clear();
        return;
    }
    if handle_confirm_key(app, key) {
        return;
    }
    if app.is_finder_open() {
        finder::handle_finder_key(app, key);
        return;
    }
    if app.is_settings_open() {
        settings::handle_settings_key(app, key);
        return;
    }
    if app.is_searching() {
        handle_search_key(app, key);
        return;
    }
    if handle_global_key(app, key) {
        return;
    }

    match app.focused_pane {
        PaneId::Package | PaneId::Git | PaneId::Targets => detail::handle_detail_key(app, key),
        PaneId::CiRuns => detail::handle_ci_runs_key(app, key),
        PaneId::Toasts => handle_toast_key(app, key),
        _ => handle_normal_key(app, key),
    }
}

const fn normalize_key(app: &App, key: KeyCode) -> KeyCode {
    if app.is_searching() || app.is_finder_open() || app.is_settings_editing() {
        return key;
    }
    let key = if app.navigation_keys().uses_vim() {
        match app.focused_pane {
            PaneId::Package | PaneId::Git | PaneId::Targets | PaneId::CiRuns | PaneId::Toasts => {
                match key {
                    KeyCode::Char('h' | 'k') => KeyCode::Up,
                    KeyCode::Char('j' | 'l') => KeyCode::Down,
                    _ => key,
                }
            },
            _ => match key {
                KeyCode::Char('h') => KeyCode::Left,
                KeyCode::Char('j') => KeyCode::Down,
                KeyCode::Char('k') => KeyCode::Up,
                KeyCode::Char('l') => KeyCode::Right,
                _ => key,
            },
        }
    } else {
        key
    };
    match app.focused_pane {
        PaneId::Package | PaneId::Git | PaneId::Targets | PaneId::CiRuns | PaneId::Toasts => {
            match key {
                KeyCode::Left => KeyCode::Up,
                KeyCode::Right => KeyCode::Down,
                _ => key,
            }
        },
        _ => key,
    }
}

fn handle_confirm_key(app: &mut App, key: KeyCode) -> bool {
    let Some(action) = app.confirm.take() else {
        return false;
    };
    if key == KeyCode::Char('y') {
        match action {
            ConfirmAction::Clean(abs_path) => {
                let toast = app.start_task_toast(
                    "cargo clean",
                    project::home_relative_path(Path::new(&abs_path)),
                );
                app.pending_cleans
                    .push_back(PendingClean { abs_path, toast });
            },
        }
    }
    true
}

fn handle_mouse_event(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    match kind {
        MouseEventKind::ScrollUp => {
            if app.focused_pane == PaneId::ScanLog {
                if app.invert_scroll().is_inverted() {
                    app.scan_log_scroll_down();
                } else {
                    app.scan_log_scroll_up();
                }
            } else {
                scroll_main(app, true);
            }
        },
        MouseEventKind::ScrollDown => {
            if app.focused_pane == PaneId::ScanLog {
                if app.invert_scroll().is_inverted() {
                    app.scan_log_scroll_up();
                } else {
                    app.scan_log_scroll_down();
                }
            } else {
                scroll_main(app, false);
            }
        },
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
        PaneId::CiRuns => "ci_runs",
        PaneId::Toasts => "toasts",
        PaneId::ScanLog => "scan_log",
        PaneId::Search => "search",
        PaneId::Settings => "settings",
        PaneId::Finder => "finder",
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

    if app.confirm.is_some() {
        return;
    }
    if app.is_finder_open() {
        handle_finder_click(app, pos);
        return;
    }
    if app.is_settings_open() {
        handle_settings_click(app, pos);
        return;
    }

    if handle_toast_click(app, pos) {
        return;
    }

    let project_list = app.layout_cache.project_list;
    let scan_log = app.layout_cache.scan_log;
    let detail_columns = app.layout_cache.detail_columns.clone();
    let detail_targets_col = app.layout_cache.detail_targets_col;

    if project_list.contains(pos) {
        app.focus_pane(PaneId::ProjectList);
        let inner_y = row.saturating_sub(project_list.y + 1);
        let clicked_index = app.list_state.offset() + inner_y as usize;
        if clicked_index < app.row_count() {
            app.list_state.select(Some(clicked_index));
        }
        return;
    }

    if let Some(scan_rect) = scan_log
        && scan_rect.contains(pos)
    {
        app.focus_pane(PaneId::ScanLog);
        let inner_y = row.saturating_sub(scan_rect.y + 1);
        let clicked_index = app.scan_log_state.offset() + inner_y as usize;
        if clicked_index < app.scan_log.len() {
            app.scan_log_state.select(Some(clicked_index));
        }
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
        let pane = match pane_id {
            PaneId::Targets => &mut app.targets_pane,
            PaneId::Package => &mut app.package_pane,
            PaneId::Git => &mut app.git_pane,
            _ => unreachable!(),
        };
        if let Some(clicked_row) = pane.clicked_row(pos) {
            pane.set_pos(clicked_row);
        }
        return;
    }

    let clicked_row = if app.showing_lints() {
        app.lint_pane.clicked_row(pos)
    } else {
        app.ci_pane.clicked_row(pos)
    };
    if let Some(clicked_row) = clicked_row {
        app.focus_pane(PaneId::CiRuns);
        if app.showing_lints() {
            app.lint_pane.set_pos(clicked_row);
        } else {
            app.ci_pane.set_pos(clicked_row);
        }
    }
}

fn handle_toast_click(app: &mut App, pos: Position) -> bool {
    for hitbox in app.layout_cache.toast_hitboxes.clone() {
        if hitbox.close_rect.contains(pos) {
            app.dismiss_toast(hitbox.id);
            return true;
        }
        if !hitbox.card_rect.contains(pos) {
            continue;
        }
        let active = app.active_toasts();
        if let Some(index) = active.iter().position(|toast| toast.id() == hitbox.id) {
            app.toast_pane.set_pos(index);
            app.focus_pane(PaneId::Toasts);
        }
        return true;
    }
    false
}

const fn handle_finder_click(app: &mut App, pos: Position) {
    if let Some(clicked_row) = app.finder.pane.clicked_row(pos) {
        app.finder.pane.set_pos(clicked_row);
    }
}

const fn handle_settings_click(app: &mut App, pos: Position) {
    if let Some(clicked_row) = app.settings_pane.clicked_row(pos) {
        app.settings_pane.set_pos(clicked_row);
    }
}

fn open_in_editor(app: &App) {
    let Some(project) = app.selected_project() else {
        return;
    };
    let abs_path = app
        .nodes
        .iter()
        .find(|node| {
            node.groups.iter().any(|group| {
                group
                    .members
                    .iter()
                    .any(|member| member.path == project.path)
            })
        })
        .map_or_else(
            || project.abs_path.clone(),
            |node| node.project.abs_path.clone(),
        );

    let _ = std::process::Command::new(app.editor())
        .arg(&abs_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn open_finder(app: &mut App) {
    if app.dirty.finder.is_dirty() {
        let (index, col_widths) = super::finder::build_finder_index(&app.nodes, &app.git_info);
        app.finder.index = index;
        app.finder.col_widths = col_widths;
        app.dirty.finder.mark_clean();
    }
    app.open_overlay(PaneId::Finder);
    app.open_finder();
    app.finder.query.clear();
    app.finder.results.clear();
    app.finder.total = 0;
    app.finder.pane.home();
}

fn handle_global_key(app: &mut App, key: KeyCode) -> bool {
    match key {
        KeyCode::Char('q') => app.request_quit(),
        KeyCode::Char('R') => app.request_restart(),
        KeyCode::Char('/') => open_finder(app),
        KeyCode::Char('s') => {
            app.open_overlay(PaneId::Settings);
            app.open_settings();
        },
        KeyCode::Tab => app.focus_next_pane(),
        KeyCode::BackTab => app.focus_previous_pane(),
        KeyCode::Esc => app.focus_pane(PaneId::ProjectList),
        _ => return false,
    }
    true
}

fn handle_normal_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Up => {
            if app.focused_pane == PaneId::ScanLog {
                app.scan_log_scroll_up();
            } else {
                app.move_up();
            }
        },
        KeyCode::Down => {
            if app.focused_pane == PaneId::ScanLog {
                app.scan_log_scroll_down();
            } else {
                app.move_down();
            }
        },
        KeyCode::Home => {
            if app.focused_pane == PaneId::ScanLog {
                app.scan_log_to_top();
            } else {
                app.move_to_top();
            }
        },
        KeyCode::End => {
            if app.focused_pane == PaneId::ScanLog {
                app.scan_log_to_bottom();
            } else {
                app.move_to_bottom();
            }
        },
        KeyCode::Enter => open_in_editor(app),
        KeyCode::Right => {
            if !app.expand() {
                app.move_down();
            }
        },
        KeyCode::Left => {
            if !app.collapse() {
                app.move_up();
            }
        },
        KeyCode::Char('+' | '=') => app.expand_all(),
        KeyCode::Char('-') => app.collapse_all(),
        KeyCode::Char('r') => app.rescan(),
        KeyCode::Char('c') => {
            if let Some(project) = app.selected_project()
                && project.is_rust == ProjectLanguage::Rust
            {
                app.confirm = Some(ConfirmAction::Clean(project.abs_path.clone()));
            }
        },
        _ => {},
    }
}

fn handle_toast_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Up => app.toast_pane.up(),
        KeyCode::Down => app.toast_pane.down(),
        KeyCode::Home => app.toast_pane.home(),
        KeyCode::End => app
            .toast_pane
            .set_pos(app.active_toasts().len().saturating_sub(1)),
        KeyCode::Char('x') => app.dismiss_focused_toast(),
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
            let mut query = app.search_query.clone();
            query.pop();
            app.update_search(&query);
        },
        KeyCode::Char(c) => {
            let query = format!("{}{c}", app.search_query);
            app.update_search(&query);
        },
        _ => {},
    }
}
