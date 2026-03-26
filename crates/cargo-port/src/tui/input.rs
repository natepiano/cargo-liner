use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::MouseEventKind;

use super::App;
use super::FocusTarget;
use super::detail;
use super::settings;

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
            if app.show_settings {
                settings::handle_settings_key(app, key.code);
            } else if app.editing.is_some() {
                detail::handle_field_edit_key(app, key.code);
            } else if app.searching {
                handle_search_key(app, key.code);
            } else if app.focus == FocusTarget::DetailFields {
                detail::handle_detail_key(app, key.code);
            } else if app.focus == FocusTarget::CiRuns {
                detail::handle_ci_runs_key(app, key.code);
            } else {
                handle_normal_key(app, key.code);
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

fn handle_normal_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Tab => advance_focus(app),
        KeyCode::BackTab => reverse_focus(app),
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
        KeyCode::Enter | KeyCode::Right => app.expand(),
        KeyCode::Left => app.collapse(),
        KeyCode::Char('/') => app.start_search(),
        KeyCode::Char('s') => app.show_settings = true,
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
        app.ci_runs.get(&p.path).is_some_and(|r| !r.is_empty())
            || app.git_info.get(&p.path).is_some_and(|g| g.url.is_some())
    });

    let max_detail_col = detail::detail_max_column(app);

    app.focus = match app.focus {
        FocusTarget::ProjectList => {
            app.detail_column = 0;
            app.detail_cursor = 0;
            FocusTarget::DetailFields
        },
        FocusTarget::DetailFields => {
            // Advance through detail columns first
            if app.detail_column < max_detail_col {
                app.detail_column += 1;
                app.detail_cursor = 0;
                let (_, targets_col) = detail::detail_layout_pub(app);
                if Some(app.detail_column) == targets_col {
                    app.examples_scroll = 0;
                }
                FocusTarget::DetailFields
            } else if has_ci {
                app.ci_runs_cursor = 0;
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
        app.ci_runs.get(&p.path).is_some_and(|r| !r.is_empty())
            || app.git_info.get(&p.path).is_some_and(|g| g.url.is_some())
    });

    let max_detail_col = detail::detail_max_column(app);
    let (_, targets_col) = detail::detail_layout_pub(app);

    app.focus = match app.focus {
        FocusTarget::ProjectList => {
            if !app.scan_complete {
                FocusTarget::ScanLog
            } else if has_ci {
                app.ci_runs_cursor = 0;
                FocusTarget::CiRuns
            } else {
                app.detail_column = max_detail_col;
                app.detail_cursor = 0;
                if Some(max_detail_col) == targets_col {
                    app.examples_scroll = 0;
                }
                FocusTarget::DetailFields
            }
        },
        FocusTarget::DetailFields => {
            // Reverse through detail columns first
            if app.detail_column > 0 {
                app.detail_column -= 1;
                app.detail_cursor = 0;
                FocusTarget::DetailFields
            } else {
                FocusTarget::ProjectList
            }
        },
        FocusTarget::CiRuns => {
            app.detail_column = max_detail_col;
            app.detail_cursor = 0;
            if Some(max_detail_col) == targets_col {
                app.examples_scroll = 0;
            }
            FocusTarget::DetailFields
        },
        FocusTarget::ScanLog => {
            if has_ci {
                app.ci_runs_cursor = 0;
                FocusTarget::CiRuns
            } else {
                app.detail_column = max_detail_col;
                app.detail_cursor = 0;
                if Some(max_detail_col) == targets_col {
                    app.examples_scroll = 0;
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
