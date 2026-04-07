use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use strum::EnumCount;
use strum::IntoEnumIterator;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use super::app::App;
use super::constants::SECTION_HEADER_INDENT;
use super::constants::SECTION_ITEM_INDENT;
use super::constants::SETTINGS_POPUP_WIDTH;
use crate::config;

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::EnumCount, strum::EnumIter)]
pub(super) enum SettingOption {
    InvertScroll,
    IncludeNonRust,
    NavigationKeys,
    CiRunCount,
    Editor,
    IncludeDirs,
    InlineDirs,
    StatusFlashSecs,
    TaskLingerSecs,
    LintsEnabled,
    LintOnDiscovery,
    LintProjects,
    LintCommands,
    LintCacheSize,
}

impl SettingOption {
    pub(super) fn from_index(i: usize) -> Option<Self> { Self::iter().nth(i) }
}

fn parse_dir_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

type SettingsRow = (Option<SettingOption>, &'static str, String);

fn format_lint_projects(cfg: &config::CargoPortConfig) -> String {
    if cfg.lint.include.is_empty() {
        "—".to_string()
    } else {
        format_sorted_list(&cfg.lint.include)
    }
}

fn format_sorted_list(values: &[String]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by_key(|value| value.to_lowercase());
    sorted.join(", ")
}

fn normalize_sorted_list(value: &str) -> Vec<String> {
    let mut entries = parse_dir_list(value);
    entries.sort_unstable_by_key(|entry| entry.to_lowercase());
    entries
}

fn format_lint_commands(cfg: &config::CargoPortConfig) -> String {
    let commands = if cfg.lint.commands.is_empty() {
        cfg.lint.resolved_commands()
    } else {
        cfg.lint.commands.clone()
    };
    commands
        .iter()
        .map(|command| command.command.trim().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_lint_cache_size(cfg: &config::CargoPortConfig) -> String { cfg.lint.cache_size.clone() }

fn format_secs(secs: f64) -> String {
    // Display whole-number seconds without a decimal point.
    if secs.fract() == 0.0 {
        format!("{secs:.0}")
    } else {
        format!("{secs}")
    }
}

fn format_flash_secs(cfg: &config::CargoPortConfig) -> String {
    format_secs(cfg.tui.status_flash_secs)
}

fn format_linger_secs(cfg: &config::CargoPortConfig) -> String {
    format_secs(cfg.tui.task_linger_secs)
}

fn settings_rows(app: &App, cfg: &config::CargoPortConfig) -> Vec<SettingsRow> {
    vec![
        (None, "General", String::new()),
        (
            Some(SettingOption::InvertScroll),
            "Invert scroll",
            if app.invert_scroll().is_inverted() {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            Some(SettingOption::IncludeNonRust),
            "Non-Rust projects",
            if app.include_non_rust().includes_non_rust() {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            Some(SettingOption::NavigationKeys),
            "Vim nav keys",
            if app.navigation_keys().uses_vim() {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            Some(SettingOption::CiRunCount),
            "CI run count",
            cfg.tui.ci_run_count.to_string(),
        ),
        (
            Some(SettingOption::Editor),
            "Editor",
            app.editor().to_string(),
        ),
        (
            Some(SettingOption::IncludeDirs),
            "Include dirs",
            format_sorted_list(&cfg.tui.include_dirs),
        ),
        (
            Some(SettingOption::InlineDirs),
            "Inline dirs",
            format_sorted_list(&cfg.tui.inline_dirs),
        ),
        (None, "Toasts", String::new()),
        (
            Some(SettingOption::StatusFlashSecs),
            "Status flash secs",
            format_flash_secs(cfg),
        ),
        (
            Some(SettingOption::TaskLingerSecs),
            "Task linger secs",
            format_linger_secs(cfg),
        ),
        (None, "Lints", String::new()),
        (
            Some(SettingOption::LintsEnabled),
            "Enabled",
            if app.lint_enabled() { "ON" } else { "OFF" }.to_string(),
        ),
        (
            Some(SettingOption::LintOnDiscovery),
            "Lint on discovery",
            if cfg.lint.on_discovery.is_immediate() {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            Some(SettingOption::LintProjects),
            "Projects",
            format_lint_projects(cfg),
        ),
        (
            Some(SettingOption::LintCommands),
            "Commands",
            format_lint_commands(cfg),
        ),
        (
            Some(SettingOption::LintCacheSize),
            "Cache size",
            format_lint_cache_size(cfg),
        ),
    ]
}

fn wrap_text_to_width(value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    if value.trim().is_empty() {
        return vec![String::new()];
    }

    let mut wrapped = Vec::new();
    let mut current = String::new();

    for word in value.split_whitespace() {
        let separator = if current.is_empty() { "" } else { " " };
        let candidate = format!("{current}{separator}{word}");
        if candidate.width() <= width {
            current = candidate;
            continue;
        }

        if !current.is_empty() {
            wrapped.push(std::mem::take(&mut current));
        }

        if word.width() <= width {
            current = word.to_string();
            continue;
        }

        let mut segment = String::new();
        for ch in word.chars() {
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if !segment.is_empty() && segment.width() + char_width > width {
                wrapped.push(std::mem::take(&mut segment));
            }
            segment.push(ch);
        }
        current = segment;
    }

    if !current.is_empty() {
        wrapped.push(current);
    }

    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    wrapped
}

fn push_wrapped_value_row(
    lines: &mut Vec<Line<'static>>,
    prefix: &str,
    value: &str,
    prefix_style: Style,
    value_style: Style,
    content_width: usize,
) {
    let prefix_width = prefix.width();
    let value_width = content_width.saturating_sub(prefix_width).max(1);
    let wrapped = wrap_text_to_width(value, value_width);
    let continuation_prefix = " ".repeat(prefix_width);

    for (index, chunk) in wrapped.into_iter().enumerate() {
        let visible_prefix = if index == 0 {
            prefix.to_string()
        } else {
            continuation_prefix.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(visible_prefix, prefix_style),
            Span::styled(chunk, value_style),
        ]));
    }
}

fn prev_char_boundary(s: &str, cursor: usize) -> usize {
    s[..cursor].char_indices().last().map_or(0, |(idx, _)| idx)
}

fn next_char_boundary(s: &str, cursor: usize) -> usize {
    s[cursor..]
        .chars()
        .next()
        .map_or(s.len(), |ch| cursor + ch.len_utf8())
}

fn render_edit_buffer(buf: &str, cursor: usize) -> String {
    let mut rendered = String::with_capacity(buf.len() + 1);
    rendered.push_str(&buf[..cursor]);
    rendered.push('_');
    rendered.push_str(&buf[cursor..]);
    rendered
}

fn insert_char_at_cursor(buf: &mut String, cursor: &mut usize, ch: char) {
    buf.insert(*cursor, ch);
    *cursor += ch.len_utf8();
}

fn backspace_at_cursor(buf: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let prev = prev_char_boundary(buf, *cursor);
    buf.drain(prev..*cursor);
    *cursor = prev;
}

fn delete_at_cursor(buf: &mut String, cursor: usize) {
    if cursor >= buf.len() {
        return;
    }
    let next = next_char_boundary(buf, cursor);
    buf.drain(cursor..next);
}

fn parse_lint_commands(value: &str) -> Vec<config::LintCommandConfig> {
    config::normalize_lint_commands(
        &parse_dir_list(value)
            .into_iter()
            .map(|command| config::LintCommandConfig {
                name: String::new(),
                command,
            })
            .collect::<Vec<_>>(),
    )
}

fn parse_lint_cache_size(value: &str) -> Result<String, String> {
    config::parse_cache_size(value).map(|parsed| parsed.normalized)
}

fn toggle_vim_mode(app: &mut App) {
    if !app.navigation_keys().uses_vim() {
        // Enabling vim mode — check for hjkl conflicts.
        let conflicts = crate::keymap::vim_mode_conflicts(&app.current_keymap);
        if !conflicts.is_empty() {
            let msg = format!(
                "Cannot enable vim mode — these bindings use h/j/k/l:\n{}",
                conflicts.join(", ")
            );
            app.inline_error = Some(msg);
            return;
        }
    }
    let mut cfg = app.current_config.clone();
    cfg.tui.navigation_keys.toggle();
    let _ = save_updated_config(app, &cfg);
}

fn save_updated_config(app: &mut App, cfg: &config::CargoPortConfig) -> bool {
    match app.save_and_apply_config(cfg) {
        Ok(()) => true,
        Err(err) => {
            app.inline_error = Some(err);
            false
        },
    }
}

pub(super) fn render_settings_popup(frame: &mut Frame, app: &mut App) {
    let rows = settings_rows(app, &app.current_config);
    let label_style = Style::default().fg(Color::DarkGray);
    let highlight_style = Style::default().fg(Color::Black).bg(Color::Cyan);
    let content_width = usize::from(SETTINGS_POPUP_WIDTH.saturating_sub(2));

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    build_settings_lines(
        app,
        &rows,
        &mut lines,
        highlight_style,
        label_style,
        content_width,
    );
    lines.push(Line::from(""));
    let nav_keys_index = SettingOption::iter().position(|s| s == SettingOption::NavigationKeys);
    if !app.is_settings_editing() && Some(app.settings_pane.pos()) == nav_keys_index {
        lines.push(Line::from(vec![
            Span::styled("  Note: ", label_style),
            Span::styled("maps h/j/k/l to arrow navigation", label_style),
        ]));
    }

    let popup_height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .saturating_add(1);

    app.settings_pane.set_len(SettingOption::COUNT);

    let inner = super::popup::PopupFrame {
        title:        Some(" Settings ".to_string()),
        border_color: Color::Cyan,
        width:        SETTINGS_POPUP_WIDTH,
        height:       popup_height,
    }
    .render(frame);

    app.settings_pane.set_content_area(inner);

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

pub(super) fn build_settings_lines(
    app: &App,
    settings: &[SettingsRow],
    lines: &mut Vec<Line<'static>>,
    highlight_style: Style,
    label_style: Style,
    content_width: usize,
) {
    let max_label = settings
        .iter()
        .filter_map(|(setting, name, _)| setting.map(|_| name.len()))
        .max()
        .unwrap_or(0);

    let mut selection_index = 0;
    for (setting, name, value) in settings {
        if setting.is_none() {
            lines.push(Line::from(vec![
                Span::raw(SECTION_HEADER_INDENT),
                Span::styled(
                    format!("{name}:"),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            continue;
        }

        let cursor = if app.settings_pane.pos() == selection_index {
            "▶ "
        } else {
            "  "
        };
        let is_selected = app.settings_pane.pos() == selection_index;
        let setting = *setting;
        let label = format!("{SECTION_ITEM_INDENT}{cursor}{name:<max_label$}  ");

        if is_selected && app.inline_error.is_some() && !app.is_settings_editing() {
            let error = app.inline_error.clone().unwrap_or_default();
            push_wrapped_value_row(
                lines,
                &label,
                &error,
                highlight_style,
                Style::default().fg(Color::Red),
                content_width,
            );
        } else if app.is_settings_editing() && is_selected {
            push_wrapped_value_row(
                lines,
                &label,
                &render_edit_buffer(&app.settings_edit_buf, app.settings_edit_cursor),
                Style::default().fg(Color::Yellow),
                Style::default().fg(Color::Yellow),
                content_width,
            );
        } else if setting == Some(SettingOption::InvertScroll)
            || setting == Some(SettingOption::IncludeNonRust)
            || setting == Some(SettingOption::NavigationKeys)
            || setting == Some(SettingOption::LintsEnabled)
            || setting == Some(SettingOption::LintOnDiscovery)
        {
            let is_on = match setting {
                Some(SettingOption::InvertScroll) => app.invert_scroll().is_inverted(),
                Some(SettingOption::IncludeNonRust) => app.include_non_rust().includes_non_rust(),
                Some(SettingOption::NavigationKeys) => app.navigation_keys().uses_vim(),
                Some(SettingOption::LintsEnabled) => app.lint_enabled(),
                Some(SettingOption::LintOnDiscovery) => {
                    app.current_config.lint.on_discovery.is_immediate()
                },
                _ => false,
            };
            let toggle_style = if is_on {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            };
            let row_style = if is_selected {
                highlight_style
            } else {
                label_style
            };
            lines.push(Line::from(vec![
                Span::styled(label, row_style),
                Span::styled("< ", Style::default().fg(Color::DarkGray)),
                Span::styled((*value).clone(), toggle_style),
                Span::styled(" >", Style::default().fg(Color::DarkGray)),
            ]));
        } else if setting == Some(SettingOption::CiRunCount)
            && is_selected
            && !app.is_settings_editing()
        {
            lines.push(Line::from(vec![
                Span::styled(label, highlight_style),
                Span::styled("< ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    (*value).clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(" >", Style::default().fg(Color::DarkGray)),
            ]));
        } else {
            let style = if is_selected {
                highlight_style
            } else {
                label_style
            };
            push_wrapped_value_row(lines, &label, value, style, style, content_width);
        }
        selection_index += 1;
    }
}

pub(super) fn handle_settings_key(app: &mut App, key: KeyCode) {
    if app.is_settings_editing() {
        handle_settings_edit_key(app, key);
        return;
    }

    let setting = SettingOption::from_index(app.settings_pane.pos());

    match key {
        KeyCode::Esc | KeyCode::Char('s') => {
            app.close_settings();
            app.close_overlay();
        },
        KeyCode::Up => {
            app.inline_error = None;
            app.settings_pane.up();
        },
        KeyCode::Down => {
            app.inline_error = None;
            app.settings_pane.down();
        },
        KeyCode::Left | KeyCode::Right => {
            app.inline_error = None;
            handle_settings_adjust_key(app, key, setting);
        },
        KeyCode::Enter | KeyCode::Char(' ') => {
            app.inline_error = None;
            handle_settings_activate_key(app, setting);
        },
        _ => {},
    }
}

fn handle_settings_adjust_key(app: &mut App, key: KeyCode, setting: Option<SettingOption>) {
    match setting {
        Some(SettingOption::InvertScroll) => {
            let mut cfg = app.current_config.clone();
            cfg.mouse.invert_scroll.toggle();
            let _ = save_updated_config(app, &cfg);
        },
        Some(SettingOption::NavigationKeys) => {
            toggle_vim_mode(app);
        },
        Some(SettingOption::CiRunCount) => {
            let mut cfg = app.current_config.clone();
            if key == KeyCode::Right {
                cfg.tui.ci_run_count = cfg.tui.ci_run_count.saturating_add(1);
            } else {
                cfg.tui.ci_run_count = cfg.tui.ci_run_count.saturating_sub(1).max(1);
            }
            let _ = save_updated_config(app, &cfg);
        },
        Some(SettingOption::IncludeNonRust) => {
            let mut cfg = app.current_config.clone();
            cfg.tui.include_non_rust.toggle();
            let _ = save_updated_config(app, &cfg);
        },
        Some(SettingOption::LintsEnabled) => {
            toggle_lints(app);
        },
        Some(SettingOption::LintOnDiscovery) => {
            let mut cfg = app.current_config.clone();
            cfg.lint.on_discovery.toggle();
            let _ = save_updated_config(app, &cfg);
        },
        Some(
            SettingOption::Editor
            | SettingOption::IncludeDirs
            | SettingOption::InlineDirs
            | SettingOption::StatusFlashSecs
            | SettingOption::TaskLingerSecs
            | SettingOption::LintProjects
            | SettingOption::LintCommands
            | SettingOption::LintCacheSize,
        )
        | None => {},
    }
}

fn finish_settings_edit_with_error(app: &mut App, error: impl Into<String>) {
    app.end_settings_editing();
    app.settings_edit_buf.clear();
    app.settings_edit_cursor = 0;
    app.inline_error = Some(error.into());
}

fn begin_settings_edit(app: &mut App, value: String) {
    app.settings_edit_buf = value;
    app.begin_settings_editing();
    app.settings_edit_cursor = app.settings_edit_buf.len();
}

fn handle_settings_activate_key(app: &mut App, setting: Option<SettingOption>) {
    match setting {
        Some(SettingOption::InvertScroll) => {
            let mut cfg = app.current_config.clone();
            cfg.mouse.invert_scroll.toggle();
            let _ = save_updated_config(app, &cfg);
        },
        Some(SettingOption::NavigationKeys) => {
            toggle_vim_mode(app);
        },
        Some(SettingOption::CiRunCount) => {
            begin_settings_edit(app, app.current_config.tui.ci_run_count.to_string());
        },
        Some(SettingOption::InlineDirs) => {
            begin_settings_edit(app, app.current_config.tui.inline_dirs.join(", "));
        },
        Some(SettingOption::IncludeDirs) => {
            begin_settings_edit(app, app.current_config.tui.include_dirs.join(", "));
        },
        Some(SettingOption::LintProjects) => {
            begin_settings_edit(app, app.current_config.lint.include.join(", "));
        },
        Some(SettingOption::LintCommands) => {
            begin_settings_edit(app, format_lint_commands(&app.current_config));
        },
        Some(SettingOption::LintCacheSize) => {
            begin_settings_edit(app, format_lint_cache_size(&app.current_config));
        },
        Some(SettingOption::StatusFlashSecs) => {
            begin_settings_edit(app, format_flash_secs(&app.current_config));
        },
        Some(SettingOption::TaskLingerSecs) => {
            begin_settings_edit(app, format_linger_secs(&app.current_config));
        },
        Some(SettingOption::IncludeNonRust) => {
            let mut cfg = app.current_config.clone();
            cfg.tui.include_non_rust.toggle();
            let _ = save_updated_config(app, &cfg);
        },
        Some(SettingOption::LintsEnabled) => {
            toggle_lints(app);
        },
        Some(SettingOption::LintOnDiscovery) => {
            let mut cfg = app.current_config.clone();
            cfg.lint.on_discovery.toggle();
            let _ = save_updated_config(app, &cfg);
        },
        Some(SettingOption::Editor) => {
            begin_settings_edit(app, app.editor().to_string());
        },
        None => {},
    }
}

fn apply_settings_edit(app: &mut App) {
    let setting = SettingOption::from_index(app.settings_pane.pos());
    let value = app.settings_edit_buf.clone();
    match setting {
        Some(SettingOption::CiRunCount) => match value.parse::<u32>() {
            Ok(n) => {
                let mut cfg = app.current_config.clone();
                cfg.tui.ci_run_count = n.max(1);
                let _ = save_updated_config(app, &cfg);
            },
            Err(_) => {
                finish_settings_edit_with_error(app, format!("Invalid number: {value}"));
                return;
            },
        },
        Some(SettingOption::InlineDirs) => {
            let dirs = normalize_sorted_list(&value);
            let mut cfg = app.current_config.clone();
            cfg.tui.inline_dirs = dirs;
            let _ = save_updated_config(app, &cfg);
        },
        Some(SettingOption::IncludeDirs) => {
            let dirs = normalize_sorted_list(&value);
            let mut cfg = app.current_config.clone();
            cfg.tui.include_dirs = dirs;
            let _ = save_updated_config(app, &cfg);
        },
        Some(SettingOption::Editor) => {
            let editor = value.trim().to_string();
            if !editor.is_empty() {
                let mut cfg = app.current_config.clone();
                cfg.tui.editor = editor;
                let _ = save_updated_config(app, &cfg);
            }
        },
        Some(SettingOption::LintProjects) => {
            let mut cfg = app.current_config.clone();
            cfg.lint.include = normalize_sorted_list(&value);
            if save_updated_config(app, &cfg) {
                app.show_timed_toast("Settings", "Lint projects updated");
            }
        },
        Some(SettingOption::LintCommands) => {
            let mut cfg = app.current_config.clone();
            cfg.lint.commands = parse_lint_commands(&value);
            if save_updated_config(app, &cfg) {
                app.show_timed_toast("Settings", "Lint commands updated");
            }
        },
        Some(SettingOption::LintCacheSize) => match parse_lint_cache_size(&value) {
            Ok(cache_size) => {
                let mut cfg = app.current_config.clone();
                cfg.lint.cache_size = cache_size;
                if save_updated_config(app, &cfg) {
                    app.show_timed_toast("Settings", "Lint cache size updated");
                }
            },
            Err(err) => {
                finish_settings_edit_with_error(app, err);
                return;
            },
        },
        Some(SettingOption::StatusFlashSecs) => match value.parse::<f64>() {
            Ok(secs) => {
                let mut cfg = app.current_config.clone();
                cfg.tui.status_flash_secs = secs.max(0.0);
                let _ = save_updated_config(app, &cfg);
            },
            Err(_) => {
                finish_settings_edit_with_error(app, format!("Invalid number: {value}"));
                return;
            },
        },
        Some(SettingOption::TaskLingerSecs) => match value.parse::<f64>() {
            Ok(secs) => {
                let mut cfg = app.current_config.clone();
                cfg.tui.task_linger_secs = secs.max(0.0);
                let _ = save_updated_config(app, &cfg);
            },
            Err(_) => {
                finish_settings_edit_with_error(app, format!("Invalid number: {value}"));
                return;
            },
        },
        Some(
            SettingOption::InvertScroll
            | SettingOption::IncludeNonRust
            | SettingOption::NavigationKeys
            | SettingOption::LintsEnabled
            | SettingOption::LintOnDiscovery,
        )
        | None => {},
    }
    app.end_settings_editing();
    app.settings_edit_buf.clear();
    app.settings_edit_cursor = 0;
}

pub(super) fn handle_settings_edit_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Enter => {
            apply_settings_edit(app);
        },
        KeyCode::Esc => {
            app.end_settings_editing();
            app.settings_edit_buf.clear();
            app.settings_edit_cursor = 0;
        },
        KeyCode::Left => {
            app.settings_edit_cursor =
                prev_char_boundary(&app.settings_edit_buf, app.settings_edit_cursor);
        },
        KeyCode::Right => {
            app.settings_edit_cursor =
                next_char_boundary(&app.settings_edit_buf, app.settings_edit_cursor);
        },
        KeyCode::Home => {
            app.settings_edit_cursor = 0;
        },
        KeyCode::End => {
            app.settings_edit_cursor = app.settings_edit_buf.len();
        },
        KeyCode::Backspace => {
            backspace_at_cursor(&mut app.settings_edit_buf, &mut app.settings_edit_cursor);
        },
        KeyCode::Delete => {
            delete_at_cursor(&mut app.settings_edit_buf, app.settings_edit_cursor);
        },
        KeyCode::Char(c) => {
            insert_char_at_cursor(&mut app.settings_edit_buf, &mut app.settings_edit_cursor, c);
        },
        _ => {},
    }
}

fn toggle_lints(app: &mut App) {
    let mut cfg = app.current_config.clone();
    cfg.lint.enabled = !cfg.lint.enabled;
    if !save_updated_config(app, &cfg) {
        return;
    }
    app.show_timed_toast(
        "Settings",
        format!(
            "Lints {}",
            if cfg.lint.enabled {
                "enabled"
            } else {
                "disabled"
            }
        ),
    );
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::style::Style;

    use super::*;

    #[test]
    fn lint_settings_have_stable_indices() {
        assert_eq!(
            SettingOption::from_index(2),
            Some(SettingOption::NavigationKeys)
        );
        assert_eq!(
            SettingOption::from_index(9),
            Some(SettingOption::LintsEnabled)
        );
        assert_eq!(
            SettingOption::from_index(10),
            Some(SettingOption::LintOnDiscovery)
        );
        assert_eq!(
            SettingOption::from_index(11),
            Some(SettingOption::LintProjects)
        );
        assert_eq!(
            SettingOption::from_index(12),
            Some(SettingOption::LintCommands)
        );
        assert_eq!(
            SettingOption::from_index(13),
            Some(SettingOption::LintCacheSize)
        );
        assert_eq!(SettingOption::COUNT, 14);
    }

    #[test]
    fn parse_lint_commands_accepts_builtin_commands() {
        let commands = parse_lint_commands(
            "cargo mend --manifest-path \"$MANIFEST_PATH\", cargo clippy --workspace",
        );
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].name, "mend");
        assert_eq!(commands[1].name, "clippy");
    }

    #[test]
    fn parse_lint_commands_accepts_arbitrary_shell_commands() {
        let commands = parse_lint_commands("something --else");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "something");
        assert_eq!(commands[0].command, "something --else");
    }

    #[test]
    fn parse_lint_cache_size_normalizes_units() {
        assert_eq!(
            parse_lint_cache_size("1.5 gib").expect("cache size"),
            "1.5 GiB"
        );
    }

    #[test]
    fn parse_dir_list_sorts_alphabetically() {
        assert_eq!(
            normalize_sorted_list("zeta, alpha, beta"),
            vec!["alpha", "beta", "zeta"]
        );
    }

    #[test]
    fn wrapped_rows_continue_at_value_column() {
        let mut lines = Vec::new();
        push_wrapped_value_row(
            &mut lines,
            "  Projects      ",
            "alpha beta gamma delta epsilon",
            Style::default(),
            Style::default(),
            24,
        );

        assert!(lines.len() > 1);
        assert_eq!(lines[0].spans[0].content.as_ref(), "  Projects      ");
        for line in &lines[1..] {
            assert_eq!(line.spans[0].content.as_ref(), "                ");
        }
    }

    #[test]
    fn edit_buffer_renders_cursor_in_place() {
        assert_eq!(render_edit_buffer("hana", 0), "_hana");
        assert_eq!(render_edit_buffer("hana", 2), "ha_na");
        assert_eq!(render_edit_buffer("hana", 4), "hana_");
    }

    #[test]
    fn cursor_edit_helpers_support_in_place_editing() {
        let mut buf = "hana".to_string();
        let mut cursor = 2;

        insert_char_at_cursor(&mut buf, &mut cursor, 'X');
        assert_eq!(buf, "haXna");
        assert_eq!(cursor, 3);

        backspace_at_cursor(&mut buf, &mut cursor);
        assert_eq!(buf, "hana");
        assert_eq!(cursor, 2);

        delete_at_cursor(&mut buf, cursor);
        assert_eq!(buf, "haa");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn cursor_movement_respects_char_boundaries() {
        let text = "a🦀b";
        let crab = "🦀".len();

        assert_eq!(next_char_boundary(text, 0), 1);
        assert_eq!(next_char_boundary(text, 1), 1 + crab);
        assert_eq!(prev_char_boundary(text, text.len()), 1 + crab);
        assert_eq!(prev_char_boundary(text, 1 + crab), 1);
    }
}
