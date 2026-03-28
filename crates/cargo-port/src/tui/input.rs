use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::MouseEventKind;

use super::app::App;
use super::app::CiState;
use super::detail;
use super::finder;
use super::settings;
use super::types::FocusTarget;

pub(super) fn handle_event(app: &mut App, event: Event) {
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
                return;
            }
            if key.code == KeyCode::Esc && !app.example_output.is_empty() {
                // Second Esc: clear the output panel
                app.example_output.clear();
                return;
            }
            // Text-input contexts consume all keys — dispatch directly
            if app.show_finder {
                finder::handle_finder_key(app, key.code);
            } else if app.show_settings {
                settings::handle_settings_key(app, key.code);
            } else if app.editing.is_some() {
                detail::handle_field_edit_key(app, key.code);
            } else if app.searching {
                handle_search_key(app, key.code);
            } else if !handle_global_key(app, key.code) {
                // Global key not consumed — fall through to context handler
                match app.focus {
                    FocusTarget::DetailFields => detail::handle_detail_key(app, key.code),
                    FocusTarget::CiRuns => detail::handle_ci_runs_key(app, key.code),
                    _ => handle_normal_key(app, key.code),
                }
            }
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => {
                if app.focus == FocusTarget::ScanLog {
                    if app.invert_scroll {
                        app.scan_log_scroll_down();
                    } else {
                        app.scan_log_scroll_up();
                    }
                } else if app.invert_scroll {
                    app.move_down();
                } else {
                    app.move_up();
                }
            },
            MouseEventKind::ScrollDown => {
                if app.focus == FocusTarget::ScanLog {
                    if app.invert_scroll {
                        app.scan_log_scroll_up();
                    } else {
                        app.scan_log_scroll_down();
                    }
                } else if app.invert_scroll {
                    app.move_up();
                } else {
                    app.move_down();
                }
            },
            _ => {},
        },
        _ => {},
    }

    // Track project selection changes for session persistence
    if app.focus == FocusTarget::ProjectList {
        super::terminal::track_selection(app);
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
    app.show_finder = true;
    app.finder_query.clear();
    app.finder_results.clear();
    app.finder_total = 0;
    app.finder_cursor.to_top();
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
        KeyCode::Char('s') => app.show_settings = true,
        KeyCode::Tab => advance_focus(app),
        KeyCode::BackTab => reverse_focus(app),
        KeyCode::Esc => app.focus = FocusTarget::ProjectList,
        _ => return false,
    }
    true
}

fn handle_normal_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Up => {
            if app.focus == FocusTarget::ScanLog {
                app.scan_log_scroll_up();
            } else {
                app.move_up();
            }
        },
        KeyCode::Down => {
            if app.focus == FocusTarget::ScanLog {
                app.scan_log_scroll_down();
            } else {
                app.move_down();
            }
        },
        KeyCode::Home => {
            if app.focus == FocusTarget::ScanLog {
                app.scan_log_to_top();
            } else {
                app.move_to_top();
            }
        },
        KeyCode::End => {
            if app.focus == FocusTarget::ScanLog {
                app.scan_log_to_bottom();
            } else {
                app.move_to_bottom();
            }
        },
        KeyCode::Enter => open_in_editor(app),
        KeyCode::Right => app.expand(),
        KeyCode::Left => app.collapse(),
        KeyCode::Char('r') => app.rescan(),
        _ => {},
    }
}

fn handle_search_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.confirm_search(),
        KeyCode::Tab => advance_focus(app),
        KeyCode::BackTab => reverse_focus(app),
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

pub fn advance_focus(app: &mut App) {
    let has_ci = app.selected_project().is_some_and(|p| {
        app.ci_state
            .get(&p.path)
            .is_some_and(|s: &CiState| !s.runs().is_empty())
            || app.git_info.get(&p.path).is_some_and(|g| g.url.is_some())
    });

    let max_detail_col = detail::detail_max_column(app);

    app.focus = match app.focus {
        FocusTarget::ProjectList => {
            app.detail_column.to_top();
            app.detail_cursor.to_top();
            FocusTarget::DetailFields
        },
        FocusTarget::DetailFields => {
            // Advance through detail columns first
            if app.detail_column.pos() < max_detail_col {
                app.detail_column.down(max_detail_col + 1);
                app.detail_cursor.to_top();
                let (_, targets_col) = detail::detail_layout_pub(app);
                if Some(app.detail_column.pos()) == targets_col {
                    app.examples_scroll.to_top();
                }
                FocusTarget::DetailFields
            } else if has_ci {
                app.ci_runs_cursor.to_top();
                FocusTarget::CiRuns
            } else if app.scan_complete {
                FocusTarget::ProjectList
            } else {
                FocusTarget::ScanLog
            }
        },
        FocusTarget::CiRuns => {
            if app.scan_complete {
                FocusTarget::ProjectList
            } else {
                FocusTarget::ScanLog
            }
        },
        FocusTarget::ScanLog => FocusTarget::ProjectList,
    };

    if app.focus == FocusTarget::ScanLog
        && !app.scan_log.is_empty()
        && app.scan_log_state.selected().is_none()
    {
        app.scan_log_state
            .select(Some(app.scan_log.len().saturating_sub(1)));
    }
}

pub fn reverse_focus(app: &mut App) {
    let has_ci = app.selected_project().is_some_and(|p| {
        app.ci_state
            .get(&p.path)
            .is_some_and(|s: &CiState| !s.runs().is_empty())
            || app.git_info.get(&p.path).is_some_and(|g| g.url.is_some())
    });

    let max_detail_col = detail::detail_max_column(app);
    let (_, targets_col) = detail::detail_layout_pub(app);

    app.focus = match app.focus {
        FocusTarget::ProjectList => {
            if !app.scan_complete {
                FocusTarget::ScanLog
            } else if has_ci {
                app.ci_runs_cursor.to_top();
                FocusTarget::CiRuns
            } else {
                app.detail_column.set(max_detail_col);
                app.detail_cursor.to_top();
                if Some(max_detail_col) == targets_col {
                    app.examples_scroll.to_top();
                }
                FocusTarget::DetailFields
            }
        },
        FocusTarget::DetailFields => {
            // Reverse through detail columns first
            if app.detail_column.pos() > 0 {
                app.detail_column.up();
                app.detail_cursor.to_top();
                FocusTarget::DetailFields
            } else {
                FocusTarget::ProjectList
            }
        },
        FocusTarget::CiRuns => {
            app.detail_column.set(max_detail_col);
            app.detail_cursor.to_top();
            if Some(max_detail_col) == targets_col {
                app.examples_scroll.to_top();
            }
            FocusTarget::DetailFields
        },
        FocusTarget::ScanLog => {
            if has_ci {
                app.ci_runs_cursor.to_top();
                FocusTarget::CiRuns
            } else {
                app.detail_column.set(max_detail_col);
                app.detail_cursor.to_top();
                if Some(max_detail_col) == targets_col {
                    app.examples_scroll.to_top();
                }
                FocusTarget::DetailFields
            }
        },
    };

    if app.focus == FocusTarget::ScanLog
        && !app.scan_log.is_empty()
        && app.scan_log_state.selected().is_none()
    {
        app.scan_log_state
            .select(Some(app.scan_log.len().saturating_sub(1)));
    }
}
