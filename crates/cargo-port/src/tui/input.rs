use std::time::Instant;

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::MouseButton;
use crossterm::event::MouseEventKind;
use ratatui::layout::Position;

use super::app::App;
use super::app::ConfirmAction;
use super::detail;
use super::finder;
use super::settings;
use super::types::PaneId;
use crate::project::ProjectLanguage;

pub(super) fn handle_event(app: &mut App, event: &Event) {
    let started = Instant::now();
    match event {
        Event::Key(key) => {
            // Esc: if running, kill process (keep output). If not running, clear output.
            if key.code == KeyCode::Esc && app.example_running.is_some() {
                // First Esc: kill the process, keep output visible
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
                app.terminal_dirty = true;
                return;
            }
            if key.code == KeyCode::Esc && !app.example_output.is_empty() {
                // Second Esc: clear the output panel
                app.example_output.clear();
                return;
            }
            // Confirmation dialog: y confirms, anything else cancels
            if app.confirm.is_some() {
                if key.code == KeyCode::Char('y') {
                    if let Some(action) = app.confirm.take() {
                        match action {
                            ConfirmAction::Clean(abs_path) => {
                                app.pending_clean = Some(abs_path);
                            },
                        }
                    }
                } else {
                    app.confirm = None;
                }
                return;
            }
            // Text-input contexts consume all keys — dispatch directly
            if app.show_finder {
                finder::handle_finder_key(app, key.code);
            } else if app.show_settings {
                settings::handle_settings_key(app, key.code);
            } else if app.searching {
                handle_search_key(app, key.code);
            } else if !handle_global_key(app, key.code) {
                // Global key not consumed — fall through to context handler
                match app.focused_pane {
                    PaneId::Package | PaneId::Git | PaneId::Targets => {
                        detail::handle_detail_key(app, key.code);
                    },
                    PaneId::CiRuns => detail::handle_ci_runs_key(app, key.code),
                    _ => handle_normal_key(app, key.code),
                }
            }
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => {
                if app.focused_pane == PaneId::ScanLog {
                    if app.invert_scroll.is_inverted() {
                        app.scan_log_scroll_down();
                    } else {
                        app.scan_log_scroll_up();
                    }
                } else if app.invert_scroll.is_inverted() {
                    app.move_down();
                } else {
                    app.move_up();
                }
            },
            MouseEventKind::ScrollDown => {
                if app.focused_pane == PaneId::ScanLog {
                    if app.invert_scroll.is_inverted() {
                        app.scan_log_scroll_up();
                    } else {
                        app.scan_log_scroll_down();
                    }
                } else if app.invert_scroll.is_inverted() {
                    app.move_up();
                } else {
                    app.move_down();
                }
            },
            MouseEventKind::Down(MouseButton::Left) => {
                handle_mouse_click(app, mouse.column, mouse.row);
            },
            _ => {},
        },
        _ => {},
    }

    app.sync_selected_project();

    super::perf::log_duration(
        "input_event",
        started.elapsed(),
        &format!(
            "kind={} focus={} scan_complete={} selected={}",
            event_label(event),
            pane_label(app.focused_pane),
            app.scan_complete,
            app.selected_project().map_or("-", |p| p.path.as_str())
        ),
        super::perf::slow_input_event_threshold_ms(),
    );
}

const fn pane_label(pane: PaneId) -> &'static str {
    match pane {
        PaneId::ProjectList => "project_list",
        PaneId::Package => "package",
        PaneId::Git => "git",
        PaneId::Targets => "targets",
        PaneId::CiRuns => "ci_runs",
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

    // Popups consume all clicks
    if app.confirm.is_some() {
        return;
    }
    if app.show_finder {
        handle_finder_click(app, pos);
        return;
    }
    if app.show_settings {
        handle_settings_click(app, pos);
        return;
    }

    let project_list = app.layout_cache.project_list;
    let scan_log = app.layout_cache.scan_log;
    let detail_columns = app.layout_cache.detail_columns.clone();
    let detail_targets_col = app.layout_cache.detail_targets_col;

    // Project list
    if project_list.contains(pos) {
        app.focus_pane(PaneId::ProjectList);
        let inner_y = row.saturating_sub(project_list.y + 1);
        let scroll_offset = app.list_state.offset();
        let clicked_index = scroll_offset + inner_y as usize;
        if clicked_index < app.row_count() {
            app.list_state.select(Some(clicked_index));
        }
        return;
    }

    // Scan log
    if let Some(scan_rect) = scan_log
        && scan_rect.contains(pos)
    {
        app.focus_pane(PaneId::ScanLog);
        let inner_y = row.saturating_sub(scan_rect.y + 1);
        let scroll_offset = app.scan_log_state.offset();
        let clicked_index = scroll_offset + inner_y as usize;
        if clicked_index < app.scan_log.len() {
            app.scan_log_state.select(Some(clicked_index));
        }
        return;
    }

    // Detail columns (project, git, targets)
    for (col_idx, col_rect) in detail_columns.iter().enumerate() {
        if col_rect.contains(pos) {
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
    }

    // CI panel
    let clicked_row = if app.showing_port_report() {
        app.port_report_pane.clicked_row(pos)
    } else {
        app.ci_pane.clicked_row(pos)
    };
    if let Some(clicked_row) = clicked_row {
        app.focus_pane(PaneId::CiRuns);
        if app.showing_port_report() {
            app.port_report_pane.set_pos(clicked_row);
        } else {
            app.ci_pane.set_pos(clicked_row);
        }
    }
}

const fn handle_finder_click(app: &mut App, pos: Position) {
    if let Some(clicked_row) = app.finder_pane.clicked_row(pos) {
        app.finder_pane.set_pos(clicked_row);
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
    // For workspace members, open the workspace root instead
    let abs_path = app
        .nodes
        .iter()
        .find(|n| {
            n.groups
                .iter()
                .any(|g| g.members.iter().any(|m| m.path == project.path))
        })
        .map_or_else(|| project.abs_path.clone(), |n| n.project.abs_path.clone());

    let editor = app.editor.clone();
    let _ = std::process::Command::new(&editor)
        .arg(&abs_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn open_finder(app: &mut App) {
    if app.finder_dirty {
        let (index, col_widths) = super::finder::build_finder_index(&app.nodes, &app.git_info);
        app.finder_index = index;
        app.finder_col_widths = col_widths;
        app.finder_dirty = false;
    }
    app.open_overlay(PaneId::Finder);
    app.show_finder = true;
    app.finder_query.clear();
    app.finder_results.clear();
    app.finder_total = 0;
    app.finder_pane.home();
}

/// Handle keys that work in every non-text-input context.
/// Returns `true` if the key was consumed.
fn handle_global_key(app: &mut App, key: KeyCode) -> bool {
    match key {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('R') => {
            app.should_quit = true;
            app.should_restart = true;
        },
        KeyCode::Char('/') => open_finder(app),
        KeyCode::Char('s') => {
            app.open_overlay(PaneId::Settings);
            app.show_settings = true;
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
        KeyCode::Right => app.expand(),
        KeyCode::Left => app.collapse(),
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
