use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;

use super::app::App;
use crate::keymap::CiRunsAction;
use crate::keymap::GitAction;
use crate::keymap::GlobalAction;
use crate::keymap::KeyBind;
use crate::keymap::LintsAction;
use crate::keymap::PackageAction;
use crate::keymap::ProjectListAction;
use crate::keymap::ResolvedKeymap;
use crate::keymap::TargetsAction;
use crate::keymap::ToastsAction;

// ── Row model ────────────────────────────────────────────────────────

struct KeymapRow {
    scope:       &'static str,
    action:      &'static str,
    description: &'static str,
    key_display: String,
    is_header:   bool,
}

fn build_rows(km: &ResolvedKeymap) -> Vec<KeymapRow> {
    let mut rows = Vec::new();

    fn header(scope: &'static str) -> KeymapRow {
        KeymapRow {
            scope,
            action: "",
            description: "",
            key_display: String::new(),
            is_header: true,
        }
    }

    fn action_row<A: Copy + Eq + std::hash::Hash>(
        scope: &'static str,
        action: A,
        toml_key: fn(A) -> &'static str,
        description: fn(A) -> &'static str,
        scope_map: &crate::keymap::ScopeMap<A>,
    ) -> KeymapRow {
        KeymapRow {
            scope,
            action: toml_key(action),
            description: description(action),
            key_display: scope_map.display_key_for(action),
            is_header: false,
        }
    }

    rows.push(header("Global"));
    for &a in GlobalAction::ALL {
        rows.push(action_row(
            "global",
            a,
            GlobalAction::toml_key,
            GlobalAction::description,
            &km.global,
        ));
    }

    rows.push(header("Project List"));
    for &a in ProjectListAction::ALL {
        rows.push(action_row(
            "project_list",
            a,
            ProjectListAction::toml_key,
            ProjectListAction::description,
            &km.project_list,
        ));
    }

    rows.push(header("Package"));
    for &a in PackageAction::ALL {
        rows.push(action_row(
            "package",
            a,
            PackageAction::toml_key,
            PackageAction::description,
            &km.package,
        ));
    }

    rows.push(header("Git"));
    for &a in GitAction::ALL {
        rows.push(action_row(
            "git",
            a,
            GitAction::toml_key,
            GitAction::description,
            &km.git,
        ));
    }

    rows.push(header("Targets"));
    for &a in TargetsAction::ALL {
        rows.push(action_row(
            "targets",
            a,
            TargetsAction::toml_key,
            TargetsAction::description,
            &km.targets,
        ));
    }

    rows.push(header("CI Runs"));
    for &a in CiRunsAction::ALL {
        rows.push(action_row(
            "ci_runs",
            a,
            CiRunsAction::toml_key,
            CiRunsAction::description,
            &km.ci_runs,
        ));
    }

    rows.push(header("Lints"));
    for &a in LintsAction::ALL {
        rows.push(action_row(
            "lints",
            a,
            LintsAction::toml_key,
            LintsAction::description,
            &km.lints,
        ));
    }

    rows.push(header("Toasts"));
    for &a in ToastsAction::ALL {
        rows.push(action_row(
            "toasts",
            a,
            ToastsAction::toml_key,
            ToastsAction::description,
            &km.toasts,
        ));
    }

    rows
}

/// Total number of selectable (non-header) rows.
pub(super) fn selectable_row_count(km: &ResolvedKeymap) -> usize {
    GlobalAction::ALL.len()
        + ProjectListAction::ALL.len()
        + PackageAction::ALL.len()
        + GitAction::ALL.len()
        + TargetsAction::ALL.len()
        + CiRunsAction::ALL.len()
        + LintsAction::ALL.len()
        + ToastsAction::ALL.len()
}

// ── Key handling ─────────────────────────────────────────────────────

pub(super) fn handle_keymap_key(app: &mut App, key: KeyCode) {
    if app.ui_modes.keymap.is_awaiting_key() {
        handle_awaiting_key(app, key);
        return;
    }

    match key {
        KeyCode::Esc => {
            app.close_keymap();
            app.close_overlay();
        },
        KeyCode::Up => app.keymap_pane.up(),
        KeyCode::Down => app.keymap_pane.down(),
        KeyCode::Home => app.keymap_pane.home(),
        KeyCode::End => app
            .keymap_pane
            .set_pos(selectable_row_count(&app.current_keymap).saturating_sub(1)),
        KeyCode::Enter => app.keymap_begin_awaiting(),
        _ => {},
    }
}

fn handle_awaiting_key(app: &mut App, key: KeyCode) {
    if key == KeyCode::Esc {
        app.keymap_end_awaiting();
        return;
    }

    let bind = KeyBind::new(key, KeyModifiers::NONE);
    let rows = build_rows(&app.current_keymap);
    let selectable: Vec<&KeymapRow> = rows.iter().filter(|r| !r.is_header).collect();
    let Some(row) = selectable.get(app.keymap_pane.pos()) else {
        return;
    };

    // Check vim reservation.
    if app.navigation_keys().uses_vim()
        && bind.modifiers == KeyModifiers::NONE
        && matches!(
            bind.code,
            KeyCode::Char('h') | KeyCode::Char('j') | KeyCode::Char('k') | KeyCode::Char('l')
        )
    {
        app.keymap_conflict = Some(format!(
            "\"{}\" reserved for vim navigation",
            bind.display()
        ));
        return;
    }

    // Check global conflict (if pane scope).
    if row.scope != "global" {
        if let Some(global_action) = app.current_keymap.global.action_for(&bind) {
            app.keymap_conflict = Some(format!(
                "\"{}\" used by Global → {}",
                bind.display(),
                global_action.toml_key()
            ));
            return;
        }
    }

    // Check intra-scope conflict.
    let conflict = check_scope_conflict(&app.current_keymap, row.scope, row.action, &bind);
    if let Some(msg) = conflict {
        app.keymap_conflict = Some(msg);
        return;
    }

    // Valid — apply the rebind.
    apply_rebind(app, row.scope, row.action, bind);
    app.keymap_end_awaiting();
}

fn check_scope_conflict(
    km: &ResolvedKeymap,
    scope: &str,
    current_action: &str,
    bind: &KeyBind,
) -> Option<String> {
    fn check<A: Copy + Eq + std::hash::Hash>(
        scope_map: &crate::keymap::ScopeMap<A>,
        current_action: &str,
        bind: &KeyBind,
        toml_key: fn(A) -> &'static str,
        scope_label: &str,
    ) -> Option<String> {
        if let Some(existing) = scope_map.action_for(bind) {
            let existing_key = toml_key(existing);
            if existing_key != current_action {
                return Some(format!(
                    "\"{}\" used by {scope_label} → {existing_key}",
                    bind.display()
                ));
            }
        }
        None
    }

    match scope {
        "global" => check(
            &km.global,
            current_action,
            bind,
            GlobalAction::toml_key,
            "Global",
        ),
        "project_list" => check(
            &km.project_list,
            current_action,
            bind,
            ProjectListAction::toml_key,
            "Project List",
        ),
        "package" => check(
            &km.package,
            current_action,
            bind,
            PackageAction::toml_key,
            "Package",
        ),
        "git" => check(&km.git, current_action, bind, GitAction::toml_key, "Git"),
        "targets" => check(
            &km.targets,
            current_action,
            bind,
            TargetsAction::toml_key,
            "Targets",
        ),
        "ci_runs" => check(
            &km.ci_runs,
            current_action,
            bind,
            CiRunsAction::toml_key,
            "CI Runs",
        ),
        "lints" => check(
            &km.lints,
            current_action,
            bind,
            LintsAction::toml_key,
            "Lints",
        ),
        "toasts" => check(
            &km.toasts,
            current_action,
            bind,
            ToastsAction::toml_key,
            "Toasts",
        ),
        _ => None,
    }
}

fn apply_rebind(app: &mut App, scope: &str, action: &str, bind: KeyBind) {
    fn rebind<A: Copy + Eq + std::hash::Hash>(
        scope_map: &mut crate::keymap::ScopeMap<A>,
        action_key: &str,
        bind: KeyBind,
        from_toml_key: fn(&str) -> Option<A>,
    ) {
        let Some(action) = from_toml_key(action_key) else {
            return;
        };
        // Remove old binding for this action.
        if let Some(old_bind) = scope_map.by_action.get(&action).cloned() {
            scope_map.by_key.remove(&old_bind);
        }
        scope_map.insert(bind, action);
    }

    match scope {
        "global" => rebind(
            &mut app.current_keymap.global,
            action,
            bind,
            GlobalAction::from_toml_key,
        ),
        "project_list" => rebind(
            &mut app.current_keymap.project_list,
            action,
            bind,
            ProjectListAction::from_toml_key,
        ),
        "package" => rebind(
            &mut app.current_keymap.package,
            action,
            bind,
            PackageAction::from_toml_key,
        ),
        "git" => rebind(
            &mut app.current_keymap.git,
            action,
            bind,
            GitAction::from_toml_key,
        ),
        "targets" => rebind(
            &mut app.current_keymap.targets,
            action,
            bind,
            TargetsAction::from_toml_key,
        ),
        "ci_runs" => rebind(
            &mut app.current_keymap.ci_runs,
            action,
            bind,
            CiRunsAction::from_toml_key,
        ),
        "lints" => rebind(
            &mut app.current_keymap.lints,
            action,
            bind,
            LintsAction::from_toml_key,
        ),
        "toasts" => rebind(
            &mut app.current_keymap.toasts,
            action,
            bind,
            ToastsAction::from_toml_key,
        ),
        _ => {},
    }

    // Save to disk and update stamp to prevent redundant reload.
    save_keymap_to_disk(app);
}

fn save_keymap_to_disk(app: &mut App) {
    let Some(path) = &app.keymap_path else {
        return;
    };
    // Write full TOML with current bindings.
    // TODO(toml_edit): use toml_edit for targeted updates preserving comments.
    let content = ResolvedKeymap::default_toml_from(&app.current_keymap);
    let _ = std::fs::write(path, &content);
    // Update stamp so hot-reload skips this write.
    app.sync_keymap_stamp();
}

// ── Rendering ────────────────────────────────────────────────────────

const POPUP_WIDTH: u16 = 70;
const POPUP_HEIGHT: u16 = 40;

pub(super) fn render_keymap_popup(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let width = POPUP_WIDTH.min(area.width.saturating_sub(4));
    let height = POPUP_HEIGHT.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(" Keymap ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let rows = build_rows(&app.current_keymap);
    let is_awaiting = app.ui_modes.keymap.is_awaiting_key();
    let selected_pos = app.keymap_pane.pos();

    let mut selectable_index = 0usize;
    let mut lines = Vec::new();

    for row in &rows {
        if row.is_header {
            lines.push(Line::from(Span::styled(
                format!("── {} ──", row.scope),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        let is_selected = selectable_index == selected_pos;
        let key_text = if is_selected && is_awaiting {
            if let Some(conflict) = &app.keymap_conflict {
                conflict.clone()
            } else {
                "Press key...".to_string()
            }
        } else {
            row.key_display.clone()
        };

        let desc_width = 25usize;
        let padded_desc = format!("{:<width$}", row.description, width = desc_width);

        let line = if is_selected && is_awaiting && app.keymap_conflict.is_some() {
            Line::from(vec![
                Span::styled(
                    format!("  {padded_desc}"),
                    Style::default().fg(Color::White),
                ),
                Span::styled(key_text, Style::default().fg(Color::Red)),
            ])
        } else if is_selected {
            Line::from(vec![
                Span::styled(
                    format!("▸ {padded_desc}"),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    key_text,
                    if is_awaiting {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    },
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    format!("  {padded_desc}"),
                    Style::default().fg(Color::White),
                ),
                Span::styled(key_text, Style::default().fg(Color::DarkGray)),
            ])
        };

        lines.push(line);
        selectable_index += 1;
    }

    // Scroll to keep selection visible.
    let visible_height = usize::from(inner.height);
    let scroll_offset = if selected_pos >= visible_height {
        selected_pos - visible_height + 1
    } else {
        0
    };

    // We need to account for headers in the scroll. Build a flat index
    // mapping selectable positions to line indices.
    let para = Paragraph::new(lines).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
    frame.render_widget(para, inner);
}
