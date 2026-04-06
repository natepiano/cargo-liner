use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
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
use crate::keymap::ScopeMap;
use crate::keymap::TargetsAction;

// ── Row model ────────────────────────────────────────────────────────

struct KeymapRow {
    scope:       &'static str,
    action:      &'static str,
    description: &'static str,
    key_display: String,
    is_header:   bool,
}

const fn header(scope: &'static str) -> KeymapRow {
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
    scope_map: &ScopeMap<A>,
) -> KeymapRow {
    KeymapRow {
        scope,
        action: toml_key(action),
        description: description(action),
        key_display: scope_map.display_key_for(action),
        is_header: false,
    }
}

fn push_scope<A: Copy + Eq + std::hash::Hash>(
    rows: &mut Vec<KeymapRow>,
    scope_name: &'static str,
    scope_key: &'static str,
    actions: &[A],
    toml_key: fn(A) -> &'static str,
    description: fn(A) -> &'static str,
    scope_map: &ScopeMap<A>,
) {
    rows.push(header(scope_name));
    let mut section: Vec<KeymapRow> = actions
        .iter()
        .map(|&a| action_row(scope_key, a, toml_key, description, scope_map))
        .collect();
    section.sort_by_key(|r| r.description);
    rows.extend(section);
}

const GLOBAL_NAV: &[GlobalAction] = &[GlobalAction::NextPane, GlobalAction::PrevPane];
const GLOBAL_SHORTCUTS: &[GlobalAction] = &[
    GlobalAction::Quit,
    GlobalAction::Restart,
    GlobalAction::Find,
    GlobalAction::Settings,
    GlobalAction::OpenKeymap,
    GlobalAction::Dismiss,
];

fn build_rows(km: &ResolvedKeymap) -> Vec<KeymapRow> {
    let mut rows = Vec::new();
    push_scope(
        &mut rows,
        "Global Navigation",
        "global",
        GLOBAL_NAV,
        GlobalAction::toml_key,
        GlobalAction::description,
        &km.global,
    );
    push_scope(
        &mut rows,
        "Global Shortcuts",
        "global",
        GLOBAL_SHORTCUTS,
        GlobalAction::toml_key,
        GlobalAction::description,
        &km.global,
    );
    push_scope(
        &mut rows,
        "Project List",
        "project_list",
        ProjectListAction::ALL,
        ProjectListAction::toml_key,
        ProjectListAction::description,
        &km.project_list,
    );
    push_scope(
        &mut rows,
        "Package",
        "package",
        PackageAction::ALL,
        PackageAction::toml_key,
        PackageAction::description,
        &km.package,
    );
    push_scope(
        &mut rows,
        "Git",
        "git",
        GitAction::ALL,
        GitAction::toml_key,
        GitAction::description,
        &km.git,
    );
    push_scope(
        &mut rows,
        "Targets",
        "targets",
        TargetsAction::ALL,
        TargetsAction::toml_key,
        TargetsAction::description,
        &km.targets,
    );
    push_scope(
        &mut rows,
        "CI Runs",
        "ci_runs",
        CiRunsAction::ALL,
        CiRunsAction::toml_key,
        CiRunsAction::description,
        &km.ci_runs,
    );
    push_scope(
        &mut rows,
        "Lints",
        "lints",
        LintsAction::ALL,
        LintsAction::toml_key,
        LintsAction::description,
        &km.lints,
    );
    rows
}

/// Total number of selectable (non-header) rows.
pub(super) const fn selectable_row_count() -> usize {
    GlobalAction::ALL.len()
        + ProjectListAction::ALL.len()
        + PackageAction::ALL.len()
        + GitAction::ALL.len()
        + TargetsAction::ALL.len()
        + CiRunsAction::ALL.len()
        + LintsAction::ALL.len()
}

// ── Key handling ─────────────────────────────────────────────────────

pub(super) fn handle_keymap_key(app: &mut App, raw: &KeyEvent, normalized: &KeyEvent) {
    if app.ui_modes.keymap.is_awaiting_key() {
        // Awaiting mode uses the raw event so vim-normalized keys
        // don't interfere with the user's intended binding.
        handle_awaiting_key(app, raw);
        return;
    }

    // Navigation uses the normalized event (vim hjkl → arrows).
    match normalized.code {
        KeyCode::Esc => {
            app.close_keymap();
            app.close_overlay();
        },
        KeyCode::Up => app.keymap_pane.up(),
        KeyCode::Down => app.keymap_pane.down(),
        KeyCode::Home => app.keymap_pane.home(),
        KeyCode::End => app
            .keymap_pane
            .set_pos(selectable_row_count().saturating_sub(1)),
        KeyCode::Enter => app.keymap_begin_awaiting(),
        _ => {},
    }
}

fn handle_awaiting_key(app: &mut App, event: &KeyEvent) {
    if event.code == KeyCode::Esc {
        app.keymap_end_awaiting();
        return;
    }

    // Enter clears a conflict message so the user can try another key.
    if event.code == KeyCode::Enter && app.keymap_conflict.is_some() {
        app.keymap_conflict = None;
        return;
    }

    let bind = KeyBind::new(event.code, event.modifiers);
    let rows = build_rows(&app.current_keymap);
    let selectable: Vec<&KeymapRow> = rows.iter().filter(|r| !r.is_header).collect();
    let Some(row) = selectable.get(app.keymap_pane.pos()) else {
        return;
    };

    // Check navigation reservation.
    if bind.modifiers == KeyModifiers::NONE
        && matches!(
            bind.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
        )
    {
        app.keymap_conflict = Some(format!("\"{}\" reserved for navigation", bind.display()));
        return;
    }

    // Check vim reservation.
    if app.navigation_keys().uses_vim()
        && bind.modifiers == KeyModifiers::NONE
        && matches!(bind.code, KeyCode::Char('h' | 'j' | 'k' | 'l'))
    {
        app.keymap_conflict = Some(format!(
            "\"{}\" reserved for vim navigation",
            bind.display()
        ));
        return;
    }

    // Check global conflict (if pane scope).
    if row.scope != "global"
        && let Some(global_action) = app.current_keymap.global.action_for(&bind)
    {
        app.keymap_conflict = Some(format!(
            "\"{}\" used by Global → {}",
            bind.display(),
            global_action.toml_key()
        ));
        return;
    }

    // Check pane conflicts (if global scope) — a global key that
    // shadows a pane binding would silently steal the key.
    if row.scope == "global"
        && let Some(msg) = check_pane_conflict(&app.current_keymap, &bind)
    {
        app.keymap_conflict = Some(msg);
        return;
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
        scope_map: &ScopeMap<A>,
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
        _ => None,
    }
}

/// Check whether `bind` would shadow a key in any pane scope.
fn check_pane_conflict(km: &ResolvedKeymap, bind: &KeyBind) -> Option<String> {
    fn hit<A: Copy + Eq + std::hash::Hash>(
        scope_map: &ScopeMap<A>,
        bind: &KeyBind,
        toml_key: fn(A) -> &'static str,
        scope_label: &str,
    ) -> Option<String> {
        scope_map.action_for(bind).map(|a| {
            format!(
                "\"{}\" used by {} → {}",
                bind.display(),
                scope_label,
                toml_key(a),
            )
        })
    }

    None.or_else(|| {
        hit(
            &km.project_list,
            bind,
            ProjectListAction::toml_key,
            "Project List",
        )
    })
    .or_else(|| hit(&km.package, bind, PackageAction::toml_key, "Package"))
    .or_else(|| hit(&km.git, bind, GitAction::toml_key, "Git"))
    .or_else(|| hit(&km.targets, bind, TargetsAction::toml_key, "Targets"))
    .or_else(|| hit(&km.ci_runs, bind, CiRunsAction::toml_key, "CI Runs"))
    .or_else(|| hit(&km.lints, bind, LintsAction::toml_key, "Lints"))
}

fn apply_rebind(app: &mut App, scope: &str, action: &str, bind: KeyBind) {
    fn rebind<A: Copy + Eq + std::hash::Hash>(
        scope_map: &mut ScopeMap<A>,
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

const BASE_POPUP_WIDTH: u16 = 48;

fn build_lines<'a>(
    rows: &[KeymapRow],
    app: &App,
    inner: Rect,
    selected_pos: usize,
    is_awaiting: bool,
) -> Vec<Line<'a>> {
    let inner_w = usize::from(inner.width);
    let mut selectable_index = 0usize;
    let mut lines = Vec::new();

    for row in rows {
        if row.is_header {
            let label = format!(" {} ", row.scope);
            let label_len = label.len();
            let dash_total = inner_w.saturating_sub(label_len);
            let left_dashes = dash_total / 2;
            let right_dashes = dash_total - left_dashes;
            lines.push(Line::from(Span::styled(
                format!(
                    "{}{}{}",
                    "─".repeat(left_dashes),
                    label,
                    "─".repeat(right_dashes),
                ),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        let is_selected = selectable_index == selected_pos;
        let key_text = if is_selected && is_awaiting {
            app.keymap_conflict
                .as_ref()
                .map_or_else(|| "Press key...".to_string(), Clone::clone)
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

    lines
}

pub(super) fn render_keymap_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let rows = build_rows(&app.current_keymap);

    // Dynamic width: base fits all normal keys, expands for conflict messages.
    let content_width = app
        .keymap_conflict
        .as_ref()
        .map_or(BASE_POPUP_WIDTH, |msg| {
            // 2 indent + 25 desc + msg len + 2 pad
            let needed = u16::try_from(2 + 25 + msg.len() + 2).unwrap_or(u16::MAX);
            BASE_POPUP_WIDTH.max(needed)
        });
    // +2 for left/right border
    let width = (content_width + 2).min(area.width.saturating_sub(4));

    // Dynamic height: rows + 2 for top/bottom border.
    let content_height = u16::try_from(rows.len()).unwrap_or(u16::MAX);
    let height = (content_height + 2).min(area.height.saturating_sub(2));

    let inner = super::popup::PopupFrame {
        title: Some(" Keymap ".to_string()),
        border_color: Color::Cyan,
        width,
        height,
    }
    .render(frame);

    let selected_pos = app.keymap_pane.pos();
    let is_awaiting = app.ui_modes.keymap.is_awaiting_key();
    let lines = build_lines(&rows, app, inner, selected_pos, is_awaiting);

    // Scroll to keep selection visible.
    let visible_height = usize::from(inner.height);
    let scroll_offset = if selected_pos >= visible_height {
        selected_pos - visible_height + 1
    } else {
        0
    };

    let para = Paragraph::new(lines).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
    frame.render_widget(para, inner);
}
