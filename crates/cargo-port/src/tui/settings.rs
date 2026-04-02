use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use super::app::App;
use super::constants::SETTINGS_POPUP_WIDTH;
use super::render;
use crate::config;
use crate::config::LintCommandConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingOption {
    InvertScroll,
    IncludeNonRust,
    CiRunCount,
    Editor,
    IncludeDirs,
    InlineDirs,
    PortReportEnabled,
    PortReportProjects,
    PortReportCommands,
}

impl SettingOption {
    pub(super) const fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(Self::InvertScroll),
            1 => Some(Self::IncludeNonRust),
            2 => Some(Self::CiRunCount),
            3 => Some(Self::Editor),
            4 => Some(Self::IncludeDirs),
            5 => Some(Self::InlineDirs),
            6 => Some(Self::PortReportEnabled),
            7 => Some(Self::PortReportProjects),
            8 => Some(Self::PortReportCommands),
            _ => None,
        }
    }

    pub(super) const fn count() -> usize { 9 }
}

fn parse_dir_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

type SettingsRow = (Option<SettingOption>, &'static str, String);

fn format_port_report_projects(cfg: &config::Config) -> String {
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

fn format_port_report_commands(cfg: &config::Config) -> String {
    let commands = if cfg.lint.commands.is_empty() {
        cfg.lint.resolved_commands()
    } else {
        cfg.lint.commands.clone()
    };
    commands
        .iter()
        .map(|command| {
            if command.name.trim().is_empty() {
                command.command.trim().to_string()
            } else {
                command.name.trim().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn settings_rows(app: &App, cfg: &config::Config) -> Vec<SettingsRow> {
    vec![
        (None, "General", String::new()),
        (
            Some(SettingOption::InvertScroll),
            "Invert scroll",
            if app.invert_scroll.is_inverted() {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            Some(SettingOption::IncludeNonRust),
            "Non-Rust projects",
            if app.include_non_rust.includes_non_rust() {
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
        (Some(SettingOption::Editor), "Editor", app.editor.clone()),
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
        (None, "Port Report", String::new()),
        (
            Some(SettingOption::PortReportEnabled),
            "Enabled",
            if app.lint_enabled { "ON" } else { "OFF" }.to_string(),
        ),
        (
            Some(SettingOption::PortReportProjects),
            "Projects",
            format_port_report_projects(cfg),
        ),
        (
            Some(SettingOption::PortReportCommands),
            "Commands",
            format_port_report_commands(cfg),
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

fn parse_port_report_commands(value: &str) -> Option<Vec<LintCommandConfig>> {
    let names = parse_dir_list(value);
    if names.is_empty() {
        return Some(Vec::new());
    }

    names
        .into_iter()
        .map(|name| config::builtin_lint_command(&name))
        .collect()
}

pub(super) fn render_settings_popup(frame: &mut Frame, app: &mut App) {
    let cfg = config::load();
    let rows = settings_rows(app, &cfg);
    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
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
    if app.settings_editing {
        lines.push(Line::from(vec![
            Span::styled("  Enter", key_style),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style),
            Span::raw(" cancel"),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  ↑/↓", key_style),
            Span::raw(" nav  "),
            Span::styled("Enter", key_style),
            Span::raw(" edit  "),
            Span::styled("←/→", key_style),
            Span::raw(" toggle  "),
            Span::styled("Esc", key_style),
            Span::raw(" close"),
        ]));
    }

    let popup_height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .saturating_add(1);
    let area = render::centered_rect(SETTINGS_POPUP_WIDTH, popup_height, frame.area());

    app.settings_pane.set_len(SettingOption::count());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Settings ")
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .border_style(Style::default().fg(Color::Cyan));

    app.settings_pane.set_content_area(block.inner(area));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
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
                Span::raw("  "),
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
        let label = format!("  {cursor}{name:<max_label$}  ");

        if app.settings_editing && is_selected {
            push_wrapped_value_row(
                lines,
                &label,
                &format!("{}_", app.settings_edit_buf),
                Style::default().fg(Color::Yellow),
                Style::default().fg(Color::Yellow),
                content_width,
            );
        } else if setting == Some(SettingOption::InvertScroll)
            || setting == Some(SettingOption::IncludeNonRust)
            || setting == Some(SettingOption::PortReportEnabled)
        {
            let is_on = match setting {
                Some(SettingOption::InvertScroll) => app.invert_scroll.is_inverted(),
                Some(SettingOption::IncludeNonRust) => app.include_non_rust.includes_non_rust(),
                Some(SettingOption::PortReportEnabled) => app.lint_enabled,
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
        } else if setting == Some(SettingOption::CiRunCount) && is_selected && !app.settings_editing
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
    if app.settings_editing {
        handle_settings_edit_key(app, key);
        return;
    }

    let setting = SettingOption::from_index(app.settings_pane.pos());

    match key {
        KeyCode::Esc | KeyCode::Char('s') => {
            app.show_settings = false;
            app.close_overlay();
        },
        KeyCode::Up => {
            app.settings_pane.up();
        },
        KeyCode::Down => {
            app.settings_pane.down();
        },
        KeyCode::Left | KeyCode::Right => match setting {
            Some(SettingOption::InvertScroll) => {
                app.invert_scroll.toggle();
                save_settings(app);
            },
            Some(SettingOption::CiRunCount) => {
                let mut cfg = config::load();
                if key == KeyCode::Right {
                    cfg.tui.ci_run_count = cfg.tui.ci_run_count.saturating_add(1);
                } else {
                    cfg.tui.ci_run_count = cfg.tui.ci_run_count.saturating_sub(1).max(1);
                }
                app.ci_run_count = cfg.tui.ci_run_count;
                let _ = config::save(&cfg);
            },
            Some(SettingOption::IncludeNonRust) => {
                app.include_non_rust.toggle();
                let mut cfg = config::load();
                cfg.tui.include_non_rust = app.include_non_rust;
                let _ = config::save(&cfg);
                app.rescan();
            },
            Some(SettingOption::PortReportEnabled) => {
                toggle_port_report(app);
            },
            _ => {},
        },
        KeyCode::Enter | KeyCode::Char(' ') => match setting {
            Some(SettingOption::InvertScroll) => {
                app.invert_scroll.toggle();
                save_settings(app);
            },
            Some(SettingOption::CiRunCount) => {
                let cfg = config::load();
                app.settings_edit_buf = cfg.tui.ci_run_count.to_string();
                app.settings_editing = true;
            },
            Some(SettingOption::InlineDirs) => {
                app.settings_edit_buf = app.inline_dirs.join(", ");
                app.settings_editing = true;
            },
            Some(SettingOption::IncludeDirs) => {
                app.settings_edit_buf = app.include_dirs.join(", ");
                app.settings_editing = true;
            },
            Some(SettingOption::PortReportProjects) => {
                let cfg = config::load();
                app.settings_edit_buf = cfg.lint.include.join(", ");
                app.settings_editing = true;
            },
            Some(SettingOption::PortReportCommands) => {
                let cfg = config::load();
                app.settings_edit_buf = format_port_report_commands(&cfg);
                app.settings_editing = true;
            },
            Some(SettingOption::IncludeNonRust) => {
                app.include_non_rust.toggle();
                let mut cfg = config::load();
                cfg.tui.include_non_rust = app.include_non_rust;
                let _ = config::save(&cfg);
                app.rescan();
            },
            Some(SettingOption::PortReportEnabled) => {
                toggle_port_report(app);
            },
            Some(SettingOption::Editor) => {
                app.settings_edit_buf.clone_from(&app.editor);
                app.settings_editing = true;
            },
            None => {},
        },
        _ => {},
    }
}

pub(super) fn handle_settings_edit_key(app: &mut App, key: KeyCode) {
    let setting = SettingOption::from_index(app.settings_pane.pos());

    match key {
        KeyCode::Enter => {
            let value = app.settings_edit_buf.clone();
            match setting {
                Some(SettingOption::CiRunCount) => {
                    if let Ok(n) = value.parse::<u32>() {
                        let count: u32 = n.max(1);
                        app.ci_run_count = count;
                        let mut cfg = config::load();
                        cfg.tui.ci_run_count = count;
                        let _ = config::save(&cfg);
                    }
                },
                Some(SettingOption::InlineDirs) => {
                    let dirs = normalize_sorted_list(&value);
                    app.inline_dirs.clone_from(&dirs);
                    let mut cfg = config::load();
                    cfg.tui.inline_dirs = dirs;
                    let _ = config::save(&cfg);
                    app.rebuild_tree();
                },
                Some(SettingOption::IncludeDirs) => {
                    let dirs = normalize_sorted_list(&value);
                    app.include_dirs.clone_from(&dirs);
                    let mut cfg = config::load();
                    cfg.tui.include_dirs = dirs;
                    let _ = config::save(&cfg);
                    app.rescan();
                },
                Some(SettingOption::Editor) => {
                    let editor = value.trim().to_string();
                    if !editor.is_empty() {
                        app.editor.clone_from(&editor);
                        let mut cfg = config::load();
                        cfg.tui.editor = editor;
                        let _ = config::save(&cfg);
                    }
                },
                Some(SettingOption::PortReportProjects) => {
                    let mut cfg = config::load();
                    cfg.lint.include = normalize_sorted_list(&value);
                    let _ = config::save(&cfg);
                    app.apply_lint_runtime_setting(&cfg);
                    app.status_flash = Some((
                        "Port Report projects updated".to_string(),
                        std::time::Instant::now(),
                    ));
                },
                Some(SettingOption::PortReportCommands) => {
                    if let Some(commands) = parse_port_report_commands(&value) {
                        let mut cfg = config::load();
                        cfg.lint.commands = commands;
                        let _ = config::save(&cfg);
                        app.apply_lint_runtime_setting(&cfg);
                        app.status_flash = Some((
                            "Port Report commands updated".to_string(),
                            std::time::Instant::now(),
                        ));
                    } else {
                        app.status_flash = Some((
                            "Unknown Port Report command preset".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                },
                _ => {},
            }
            app.settings_editing = false;
            app.settings_edit_buf.clear();
        },
        KeyCode::Esc => {
            app.settings_editing = false;
            app.settings_edit_buf.clear();
        },
        KeyCode::Backspace => {
            app.settings_edit_buf.pop();
        },
        KeyCode::Char(c) => {
            app.settings_edit_buf.push(c);
        },
        _ => {},
    }
}

pub(super) fn save_settings(app: &App) {
    let mut cfg = config::load();
    cfg.mouse.invert_scroll = app.invert_scroll;
    let _ = config::save(&cfg);
}

fn toggle_port_report(app: &mut App) {
    let mut cfg = config::load();
    cfg.lint.enabled = !cfg.lint.enabled;
    let _ = config::save(&cfg);
    app.apply_lint_runtime_setting(&cfg);
    app.status_flash = Some((
        format!(
            "Port Report {}",
            if cfg.lint.enabled {
                "enabled"
            } else {
                "disabled"
            }
        ),
        std::time::Instant::now(),
    ));
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
    fn port_report_setting_has_stable_index() {
        assert_eq!(
            SettingOption::from_index(6),
            Some(SettingOption::PortReportEnabled)
        );
        assert_eq!(
            SettingOption::from_index(7),
            Some(SettingOption::PortReportProjects)
        );
        assert_eq!(
            SettingOption::from_index(8),
            Some(SettingOption::PortReportCommands)
        );
        assert_eq!(SettingOption::count(), 9);
    }

    #[test]
    fn parse_port_report_commands_accepts_builtin_presets() {
        let commands =
            parse_port_report_commands("mend, clippy").expect("builtin presets should parse");
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].name, "mend");
        assert_eq!(commands[1].name, "clippy");
    }

    #[test]
    fn parse_port_report_commands_rejects_unknown_presets() {
        assert!(parse_port_report_commands("fmt").is_none());
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
}
