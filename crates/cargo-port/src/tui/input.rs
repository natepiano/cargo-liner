use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::MouseButton;
use crossterm::event::MouseEventKind;
use ratatui::layout::Position;

use super::app::App;
use super::app::CiState;
use super::app::ConfirmAction;
use super::constants::CI_EXTRA_ROWS;
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
            MouseEventKind::Down(MouseButton::Left) => {
                handle_mouse_click(app, mouse.column, mouse.row);
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

    let cache = &app.layout_cache;

    // Project list
    if cache.project_list.contains(pos) {
        app.focus = FocusTarget::ProjectList;
        let inner_y = row.saturating_sub(cache.project_list.y + 1);
        let scroll_offset = app.list_state.offset();
        let clicked_index = scroll_offset + inner_y as usize;
        if clicked_index < app.row_count() {
            app.list_state.select(Some(clicked_index));
        }
        return;
    }

    // Scan log
    if let Some(scan_rect) = cache.scan_log
        && scan_rect.contains(pos)
    {
        app.focus = FocusTarget::ScanLog;
        let inner_y = row.saturating_sub(scan_rect.y + 1);
        let scroll_offset = app.scan_log_state.offset();
        let clicked_index = scroll_offset + inner_y as usize;
        if clicked_index < app.scan_log.len() {
            app.scan_log_state.select(Some(clicked_index));
        }
        return;
    }

    // Detail columns (project, git, targets)
    // Clone to release borrow on `app.layout_cache` before mutating `app`.
    let detail_columns = cache.detail_columns.clone();
    let detail_targets_col = cache.detail_targets_col;
    let targets_offset = cache.targets_table_offset;
    for (col_idx, col_rect) in detail_columns.iter().enumerate() {
        if col_rect.contains(pos) {
            app.focus = FocusTarget::DetailFields;
            app.detail_column.set(col_idx);
            let inner_y = row.saturating_sub(col_rect.y + 1) as usize;

            if Some(col_idx) == detail_targets_col {
                let total = detail::target_list_len(app);
                let clicked_row = targets_offset + inner_y;
                if clicked_row < total {
                    app.examples_scroll.set(clicked_row);
                }
            } else {
                let field_count = detail::detail_column_field_count(app, col_idx);
                if inner_y < field_count {
                    app.detail_cursor.set(inner_y);
                }
            }
            return;
        }
    }

    // CI panel (header row adds +1 beyond the border)
    let ci_panel = cache.ci_panel;
    let ci_offset = cache.ci_table_offset;
    if ci_panel.contains(pos) {
        app.focus = FocusTarget::CiRuns;
        let inner_y = row.saturating_sub(ci_panel.y + 2) as usize; // +1 border, +1 header
        let clicked_row = ci_offset + inner_y;
        let ci_run_count = app
            .selected_project()
            .and_then(|p| app.ci_state_for(p))
            .map_or(0, |s| s.runs().len());
        let total_rows = ci_run_count + CI_EXTRA_ROWS;
        if clicked_row < total_rows {
            app.ci_runs_cursor.set(clicked_row);
        }
    }
}

const fn handle_finder_click(app: &mut App, pos: Position) {
    let Some(results_area) = app.layout_cache.finder_results_area else {
        return;
    };
    if !results_area.contains(pos) {
        return;
    }
    // +1 for the header row inside the results table
    let inner_y = pos.y.saturating_sub(results_area.y + 1) as usize;
    let clicked_index = app.layout_cache.finder_table_offset + inner_y;
    if clicked_index < app.finder_results.len() {
        app.finder_cursor.set(clicked_index);
    }
}

const fn handle_settings_click(app: &mut App, pos: Position) {
    let Some(area) = app.layout_cache.settings_area else {
        return;
    };
    if !area.contains(pos) {
        return;
    }
    // Settings layout inside border: 1 empty line, then settings rows
    let inner_y = pos.y.saturating_sub(area.y + 1 + 1) as usize;
    if inner_y < settings::SettingOption::count() {
        app.settings_cursor.set(inner_y);
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
    app.finder_cursor.jump_home();
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
        KeyCode::Char('c') => {
            if let Some(project) = app.selected_project()
                && project.is_rust
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

pub(super) fn advance_focus(app: &mut App) {
    let has_ci = app.selected_project().is_some_and(|p| {
        app.ci_state
            .get(&p.path)
            .is_some_and(|s: &CiState| !s.runs().is_empty())
            || app.git_info.get(&p.path).is_some_and(|g| g.url.is_some())
    });

    let max_detail_col = detail::detail_max_column(app);

    app.focus = match app.focus {
        FocusTarget::ProjectList => {
            app.detail_column.clamp(max_detail_col + 1);
            clamp_detail_or_targets(app);
            FocusTarget::DetailFields
        },
        FocusTarget::DetailFields => {
            // Advance through detail columns first
            if app.detail_column.pos() < max_detail_col {
                app.detail_column.down(max_detail_col + 1);
                clamp_detail_or_targets(app);
                FocusTarget::DetailFields
            } else if has_ci {
                app.ci_runs_cursor.clamp(ci_total_rows(app));
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

pub(super) fn reverse_focus(app: &mut App) {
    let has_ci = app.selected_project().is_some_and(|p| {
        app.ci_state
            .get(&p.path)
            .is_some_and(|s: &CiState| !s.runs().is_empty())
            || app.git_info.get(&p.path).is_some_and(|g| g.url.is_some())
    });

    let max_detail_col = detail::detail_max_column(app);

    app.focus = match app.focus {
        FocusTarget::ProjectList => {
            if !app.scan_complete {
                FocusTarget::ScanLog
            } else if has_ci {
                app.ci_runs_cursor.clamp(ci_total_rows(app));
                FocusTarget::CiRuns
            } else {
                app.detail_column.set(max_detail_col);
                clamp_detail_or_targets(app);
                FocusTarget::DetailFields
            }
        },
        FocusTarget::DetailFields => {
            // Reverse through detail columns first
            if app.detail_column.pos() > 0 {
                app.detail_column.up();
                clamp_detail_or_targets(app);
                FocusTarget::DetailFields
            } else {
                FocusTarget::ProjectList
            }
        },
        FocusTarget::CiRuns => {
            app.detail_column.set(max_detail_col);
            clamp_detail_or_targets(app);
            FocusTarget::DetailFields
        },
        FocusTarget::ScanLog => {
            if has_ci {
                app.ci_runs_cursor.clamp(ci_total_rows(app));
                FocusTarget::CiRuns
            } else {
                app.detail_column.set(max_detail_col);
                clamp_detail_or_targets(app);
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

fn ci_total_rows(app: &App) -> usize {
    app.selected_project()
        .and_then(|p| app.ci_state_for(p))
        .map_or(0, |s| s.runs().len() + CI_EXTRA_ROWS)
}

/// Clamp `detail_cursor` or `examples_scroll` based on the current
/// `detail_column` position so the remembered row stays in bounds.
fn clamp_detail_or_targets(app: &mut App) {
    let (_, targets_col) = detail::detail_layout_pub(app);
    if Some(app.detail_column.pos()) == targets_col {
        app.examples_scroll.clamp(detail::target_list_len(app));
    } else {
        let field_count = detail::detail_column_field_count(app, app.detail_column.pos());
        app.detail_cursor.clamp(field_count);
    }
}
