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

use super::app::App;

const SETTINGS_POPUP_PADDING: u16 = 6;
use super::render;
use crate::config;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingOption {
    InvertScroll,
    CiRunCount,
    InlineDirs,
    ExcludeDirs,
    IncludeNonRust,
    Editor,
}

impl SettingOption {
    pub(super) const fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(Self::InvertScroll),
            1 => Some(Self::CiRunCount),
            2 => Some(Self::InlineDirs),
            3 => Some(Self::ExcludeDirs),
            4 => Some(Self::IncludeNonRust),
            5 => Some(Self::Editor),
            _ => None,
        }
    }

    pub(super) const fn count() -> usize { 6 }
}

fn parse_dir_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub(super) fn render_settings_popup(frame: &mut Frame, app: &mut App) {
    #[allow(clippy::cast_possible_truncation)]
    let area = render::centered_rect(
        60,
        SettingOption::count() as u16 + SETTINGS_POPUP_PADDING,
        frame.area(),
    );

    app.layout_cache.settings_area = Some(area);

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

    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(Color::DarkGray);
    let highlight_style = Style::default().fg(Color::Black).bg(Color::Cyan);

    let cfg = config::load();

    let settings: Vec<(&str, String)> = vec![
        (
            "Invert scroll",
            if app.invert_scroll { "ON" } else { "OFF" }.to_string(),
        ),
        ("CI run count", cfg.tui.ci_run_count.to_string()),
        ("Inline dirs", cfg.tui.inline_dirs.join(", ")),
        ("Exclude dirs", cfg.tui.exclude_dirs.join(", ")),
        (
            "Non-Rust projects",
            if app.include_non_rust { "ON" } else { "OFF" }.to_string(),
        ),
        ("Editor", app.editor.clone()),
    ];

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    build_settings_lines(app, &settings, &mut lines, highlight_style, label_style);
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

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

pub(super) fn build_settings_lines(
    app: &App,
    settings: &[(&str, String)],
    lines: &mut Vec<Line<'static>>,
    highlight_style: Style,
    label_style: Style,
) {
    let max_label = settings
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);

    for (i, (name, value)) in settings.iter().enumerate() {
        let cursor = if app.settings_cursor.pos() == i {
            "▶ "
        } else {
            "  "
        };
        let is_selected = app.settings_cursor.pos() == i;
        let setting = SettingOption::from_index(i);
        let label = format!("  {cursor}{name:<max_label$}  ");

        if app.settings_editing && is_selected {
            lines.push(Line::from(vec![
                Span::styled(label, Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("{}_", app.settings_edit_buf),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        } else if setting == Some(SettingOption::InvertScroll)
            || setting == Some(SettingOption::IncludeNonRust)
        {
            let is_on = match setting {
                Some(SettingOption::InvertScroll) => app.invert_scroll,
                Some(SettingOption::IncludeNonRust) => app.include_non_rust,
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
            lines.push(Line::from(vec![
                Span::styled(label, style),
                Span::styled((*value).clone(), style),
            ]));
        }
    }
}

pub(super) fn handle_settings_key(app: &mut App, key: KeyCode) {
    if app.settings_editing {
        handle_settings_edit_key(app, key);
        return;
    }

    let setting = SettingOption::from_index(app.settings_cursor.pos());

    match key {
        KeyCode::Esc | KeyCode::Char('s') => {
            app.show_settings = false;
            app.settings_cursor.jump_home();
        },
        KeyCode::Up => {
            app.settings_cursor.up();
        },
        KeyCode::Down => {
            app.settings_cursor.down(SettingOption::count());
        },
        KeyCode::Left | KeyCode::Right => match setting {
            Some(SettingOption::InvertScroll) => {
                app.invert_scroll = !app.invert_scroll;
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
                app.include_non_rust = !app.include_non_rust;
                let mut cfg = config::load();
                cfg.tui.include_non_rust = app.include_non_rust;
                let _ = config::save(&cfg);
                app.rescan();
            },
            _ => {},
        },
        KeyCode::Enter | KeyCode::Char(' ') => match setting {
            Some(SettingOption::InvertScroll) => {
                app.invert_scroll = !app.invert_scroll;
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
            Some(SettingOption::ExcludeDirs) => {
                app.settings_edit_buf = app.exclude_dirs.join(", ");
                app.settings_editing = true;
            },
            Some(SettingOption::IncludeNonRust) => {
                app.include_non_rust = !app.include_non_rust;
                let mut cfg = config::load();
                cfg.tui.include_non_rust = app.include_non_rust;
                let _ = config::save(&cfg);
                app.rescan();
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
    let setting = SettingOption::from_index(app.settings_cursor.pos());

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
                    let dirs = parse_dir_list(&value);
                    app.inline_dirs.clone_from(&dirs);
                    let mut cfg = config::load();
                    cfg.tui.inline_dirs = dirs;
                    let _ = config::save(&cfg);
                    app.rebuild_tree();
                },
                Some(SettingOption::ExcludeDirs) => {
                    let dirs = parse_dir_list(&value);
                    app.exclude_dirs.clone_from(&dirs);
                    let mut cfg = config::load();
                    cfg.tui.exclude_dirs = dirs;
                    let _ = config::save(&cfg);
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
