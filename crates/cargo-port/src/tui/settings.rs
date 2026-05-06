use crossterm::event::KeyCode;
use ratatui::Frame;
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
use super::constants::ACTIVE_BORDER_COLOR;
use super::constants::ERROR_COLOR;
use super::constants::INLINE_ERROR_COLOR;
use super::constants::LABEL_COLOR;
use super::constants::SECTION_HEADER_INDENT;
use super::constants::SECTION_ITEM_INDENT;
use super::constants::SETTINGS_POPUP_WIDTH;
use super::constants::SUCCESS_COLOR;
use super::constants::TITLE_COLOR;
use super::pane::PaneSelectionState;
use super::panes::PaneId;
use super::popup::PopupFrame;
use super::render;
use crate::config;
use crate::config::CargoPortConfig;
use crate::config::LintCommandConfig;
use crate::keymap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::EnumCount, strum::EnumIter)]
pub(super) enum SettingOption {
    InvertScroll,
    IncludeNonRust,
    NavigationKeys,
    CiRunCount,
    Editor,
    TerminalCommand,
    MainBranch,
    OtherPrimaryBranches,
    IncludeDirs,
    InlineDirs,
    StatusFlashSecs,
    TaskLingerSecs,
    DiscoveryShimmerSecs,
    CpuPollMs,
    CpuGreenMaxPercent,
    CpuYellowMaxPercent,
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

fn format_lint_projects(config: &CargoPortConfig) -> String {
    if config.lint.include.is_empty() {
        "—".to_string()
    } else {
        format_sorted_list(&config.lint.include)
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

fn save_number_setting(
    app: &mut App,
    value: &str,
    apply: impl FnOnce(&mut CargoPortConfig, f64),
) -> bool {
    let Ok(number) = value.parse::<f64>() else {
        finish_settings_edit_with_error(app, format!("Invalid number: {value}"));
        return false;
    };
    let mut config = app.config.current().clone();
    apply(&mut config, number.max(0.0));
    let _ = save_updated_config(app, &config);
    true
}

fn save_sorted_list_setting(
    app: &mut App,
    value: &str,
    apply: impl FnOnce(&mut CargoPortConfig, Vec<String>),
) {
    let mut config = app.config.current().clone();
    apply(&mut config, normalize_sorted_list(value));
    let _ = save_updated_config(app, &config);
}

fn save_u32_setting(
    app: &mut App,
    value: &str,
    apply: impl FnOnce(&mut CargoPortConfig, u32),
) -> bool {
    let Ok(number) = value.parse::<u32>() else {
        finish_settings_edit_with_error(app, format!("Invalid number: {value}"));
        return false;
    };
    let mut config = app.config.current().clone();
    apply(&mut config, number.max(1));
    let _ = save_updated_config(app, &config);
    true
}

fn bounded_u8_from_u32(value: u32) -> u8 {
    u8::try_from(value.min(u32::from(u8::MAX))).unwrap_or(u8::MAX)
}

fn save_string_setting(
    app: &mut App,
    value: &str,
    apply: impl FnOnce(&mut CargoPortConfig, String),
) {
    let mut config = app.config.current().clone();
    apply(&mut config, value.trim().to_string());
    let _ = save_updated_config(app, &config);
}

fn format_lint_commands(config: &CargoPortConfig) -> String {
    let commands = if config.lint.commands.is_empty() {
        config.lint.resolved_commands()
    } else {
        config.lint.commands.clone()
    };
    commands
        .iter()
        .map(|command| command.command.trim().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_lint_cache_size(config: &CargoPortConfig) -> String { config.lint.cache_size.clone() }

fn format_terminal_command(config: &CargoPortConfig) -> String {
    if config.tui.terminal_command.trim().is_empty() {
        "Not configured. Set this command to enable the global terminal shortcut.".to_string()
    } else {
        config.tui.terminal_command.clone()
    }
}

fn format_other_primary_branches(config: &CargoPortConfig) -> String {
    if config.tui.other_primary_branches.is_empty() {
        "—".to_string()
    } else {
        config.tui.other_primary_branches.join(", ")
    }
}

fn format_secs(secs: f64) -> String {
    // Display whole-number seconds without a decimal point.
    if secs.fract() == 0.0 {
        format!("{secs:.0}")
    } else {
        format!("{secs}")
    }
}

fn format_flash_secs(config: &CargoPortConfig) -> String {
    format_secs(config.tui.status_flash_secs)
}

fn format_linger_secs(config: &CargoPortConfig) -> String {
    format_secs(config.tui.task_linger_secs)
}

fn format_discovery_shimmer_secs(config: &CargoPortConfig) -> String {
    format_secs(config.tui.discovery_shimmer_secs)
}

fn format_cpu_poll_ms(config: &CargoPortConfig) -> String { config.cpu.poll_ms.to_string() }

fn format_cpu_green_max(config: &CargoPortConfig) -> String {
    config.cpu.green_max_percent.to_string()
}

fn format_cpu_yellow_max(config: &CargoPortConfig) -> String {
    config.cpu.yellow_max_percent.to_string()
}

fn settings_rows(app: &App, config: &CargoPortConfig) -> Vec<SettingsRow> {
    let mut rows = general_settings_rows(app, config);
    rows.extend(toast_settings_rows(config));
    rows.extend(cpu_settings_rows(config));
    rows.extend(lint_settings_rows(app, config));
    rows
}

fn general_settings_rows(app: &App, config: &CargoPortConfig) -> Vec<SettingsRow> {
    vec![
        (None, "General", String::new()),
        (
            Some(SettingOption::InvertScroll),
            "Invert scroll",
            if app.config.invert_scroll().is_inverted() {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            Some(SettingOption::IncludeNonRust),
            "Non-Rust projects",
            if app.config.include_non_rust().includes_non_rust() {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            Some(SettingOption::NavigationKeys),
            "Vim nav keys",
            if app.config.navigation_keys().uses_vim() {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            Some(SettingOption::CiRunCount),
            "CI run count",
            config.tui.ci_run_count.to_string(),
        ),
        (
            Some(SettingOption::Editor),
            "Editor",
            app.config.editor().to_string(),
        ),
        (
            Some(SettingOption::TerminalCommand),
            "Terminal",
            format_terminal_command(config),
        ),
        (
            Some(SettingOption::MainBranch),
            "Main branch",
            config.tui.main_branch.clone(),
        ),
        (
            Some(SettingOption::OtherPrimaryBranches),
            "Other primary branches",
            format_other_primary_branches(config),
        ),
        (
            Some(SettingOption::IncludeDirs),
            "Include dirs",
            format_sorted_list(&config.tui.include_dirs),
        ),
        (
            Some(SettingOption::InlineDirs),
            "Inline dirs",
            format_sorted_list(&config.tui.inline_dirs),
        ),
    ]
}

fn toast_settings_rows(config: &CargoPortConfig) -> Vec<SettingsRow> {
    vec![
        (None, "Toasts", String::new()),
        (
            Some(SettingOption::StatusFlashSecs),
            "Status flash secs",
            format_flash_secs(config),
        ),
        (
            Some(SettingOption::TaskLingerSecs),
            "Task linger secs",
            format_linger_secs(config),
        ),
        (
            Some(SettingOption::DiscoveryShimmerSecs),
            "Discovery shimmer secs",
            format_discovery_shimmer_secs(config),
        ),
    ]
}

fn cpu_settings_rows(config: &CargoPortConfig) -> Vec<SettingsRow> {
    vec![
        (None, "CPU", String::new()),
        (
            Some(SettingOption::CpuPollMs),
            "Poll ms",
            format_cpu_poll_ms(config),
        ),
        (
            Some(SettingOption::CpuGreenMaxPercent),
            "Green max %",
            format_cpu_green_max(config),
        ),
        (
            Some(SettingOption::CpuYellowMaxPercent),
            "Yellow max %",
            format_cpu_yellow_max(config),
        ),
    ]
}

fn lint_settings_rows(app: &App, config: &CargoPortConfig) -> Vec<SettingsRow> {
    vec![
        (None, "Lints", String::new()),
        (
            Some(SettingOption::LintsEnabled),
            "Enabled",
            if app.config.lint_enabled() {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            Some(SettingOption::LintOnDiscovery),
            "Lint on discovery",
            if config.lint.on_discovery.is_immediate() {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            Some(SettingOption::LintProjects),
            "Projects",
            format_lint_projects(config),
        ),
        (
            Some(SettingOption::LintCommands),
            "Commands",
            format_lint_commands(config),
        ),
        (
            Some(SettingOption::LintCacheSize),
            "Cache size",
            format_lint_cache_size(config),
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
    line_targets: &mut Vec<Option<usize>>,
    target: Option<usize>,
    row: &WrappedValueRow<'_>,
) {
    let prefix_width = row.prefix.width();
    let value_width = row.content_width.saturating_sub(prefix_width).max(1);
    let wrapped = wrap_text_to_width(row.value, value_width);
    let continuation_prefix = " ".repeat(prefix_width);

    for (index, chunk) in wrapped.into_iter().enumerate() {
        let visible_prefix = if index == 0 {
            row.prefix.to_string()
        } else {
            continuation_prefix.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(visible_prefix, row.prefix_style),
            Span::styled(chunk, row.value_style),
        ]));
        line_targets.push(target);
    }
}

struct WrappedValueRow<'a> {
    prefix:        &'a str,
    value:         &'a str,
    prefix_style:  Style,
    value_style:   Style,
    content_width: usize,
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

fn parse_lint_commands(value: &str) -> Vec<LintCommandConfig> {
    config::normalize_lint_commands(
        &parse_dir_list(value)
            .into_iter()
            .map(|command| LintCommandConfig {
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
    if !app.config.navigation_keys().uses_vim() {
        // Enabling vim mode — check for hjkl conflicts.
        let conflicts = keymap::vim_mode_conflicts(app.keymap.current());
        if !conflicts.is_empty() {
            let msg = format!(
                "Cannot enable vim mode — these bindings use h/j/k/l:\n{}",
                conflicts.join(", ")
            );
            app.overlays_mut().set_inline_error(msg);
            return;
        }
    }
    let mut config = app.config.current().clone();
    config.tui.navigation_keys.toggle();
    let _ = save_updated_config(app, &config);
}

fn save_updated_config(app: &mut App, config: &CargoPortConfig) -> bool {
    match app.save_and_apply_config(config) {
        Ok(()) => {
            app.show_timed_toast("Settings", "Saved");
            true
        },
        Err(err) => {
            app.overlays_mut().set_inline_error(err);
            false
        },
    }
}

pub(super) fn render_settings_popup(frame: &mut Frame, app: &mut App) {
    let rows = settings_rows(app, app.config.current());
    let label_style = Style::default().fg(LABEL_COLOR);
    let content_width = usize::from(SETTINGS_POPUP_WIDTH.saturating_sub(2));

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    let mut line_targets = vec![None];
    build_settings_lines(
        app,
        &rows,
        &mut lines,
        &mut line_targets,
        label_style,
        content_width,
    );
    lines.push(Line::from(""));
    line_targets.push(None);

    let popup_height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .saturating_add(1);

    app.overlays_mut()
        .settings_pane_mut()
        .viewport_mut()
        .set_len(SettingOption::COUNT);

    let inner = PopupFrame {
        title:        Some(" Settings ".to_string()),
        border_color: ACTIVE_BORDER_COLOR,
        width:        SETTINGS_POPUP_WIDTH,
        height:       popup_height,
    }
    .render(frame);

    app.overlays_mut()
        .settings_pane_mut()
        .viewport_mut()
        .set_content_area(inner);

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);

    app.overlays_mut()
        .settings_pane_mut()
        .set_line_targets(line_targets);
}

const fn is_toggle_setting(setting: Option<SettingOption>) -> bool {
    matches!(
        setting,
        Some(
            SettingOption::InvertScroll
                | SettingOption::IncludeNonRust
                | SettingOption::NavigationKeys
                | SettingOption::LintsEnabled
                | SettingOption::LintOnDiscovery,
        )
    )
}

fn push_toggle_row(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    value: &str,
    ctx: &SettingsLineContext<'_>,
    suffix: Option<&str>,
) {
    let is_on = value == "ON";
    let toggle_style = if is_on {
        Style::default()
            .fg(SUCCESS_COLOR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(ERROR_COLOR)
            .add_modifier(Modifier::BOLD)
    };
    let row_style = ctx.selection.patch(ctx.label_style);
    lines.push(Line::from(vec![
        Span::styled(ctx.label.to_owned(), row_style),
        Span::styled("< ", ctx.selection.patch(Style::default().fg(LABEL_COLOR))),
        Span::styled(value.to_owned(), ctx.selection.patch(toggle_style)),
        Span::styled(" >", ctx.selection.patch(Style::default().fg(LABEL_COLOR))),
        Span::styled(suffix.unwrap_or_default().to_owned(), row_style),
    ]));
    line_targets.push(Some(ctx.selection_index));
}

struct SettingsLineContext<'a> {
    selection_index: usize,
    label:           &'a str,
    selection:       PaneSelectionState,
    label_style:     Style,
    content_width:   usize,
}

fn push_wrapped_setting_value(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    ctx: &SettingsLineContext<'_>,
    value: &str,
    value_style: Style,
) {
    let row = WrappedValueRow {
        prefix: ctx.label,
        value,
        prefix_style: ctx.selection.patch(ctx.label_style),
        value_style,
        content_width: ctx.content_width,
    };
    push_wrapped_value_row(lines, line_targets, Some(ctx.selection_index), &row);
}

fn nav_keys_toggle_suffix(
    app: &App,
    setting: Option<SettingOption>,
    selection: PaneSelectionState,
) -> Option<&'static str> {
    if setting == Some(SettingOption::NavigationKeys)
        && selection != PaneSelectionState::Unselected
        && !app.overlays().is_settings_editing()
    {
        Some("  maps h/j/k/l to arrow navigation")
    } else {
        None
    }
}

fn push_ci_run_count_row(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    ctx: &SettingsLineContext<'_>,
    value: &str,
) {
    lines.push(Line::from(vec![
        Span::styled(ctx.label.to_owned(), ctx.selection.patch(ctx.label_style)),
        Span::styled("< ", ctx.selection.patch(Style::default().fg(LABEL_COLOR))),
        Span::styled(value.to_owned(), ctx.selection.patch(Style::default())),
        Span::styled(" >", ctx.selection.patch(Style::default().fg(LABEL_COLOR))),
    ]));
    line_targets.push(Some(ctx.selection_index));
}

fn push_lint_cache_size_row(
    app: &App,
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    ctx: &SettingsLineContext<'_>,
    value: &str,
) {
    let used = render::format_bytes(app.lint.cache_usage().bytes);
    let limit = &app.config.current().lint.cache_size;
    let usage_suffix = format!("  {used} / {limit}");
    lines.push(Line::from(vec![
        Span::styled(ctx.label.to_owned(), ctx.selection.patch(ctx.label_style)),
        Span::styled(value.to_owned(), ctx.selection.patch(Style::default())),
        Span::styled(usage_suffix, Style::default().fg(LABEL_COLOR)),
    ]));
    line_targets.push(Some(ctx.selection_index));
}

fn push_setting_row(
    app: &App,
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    ctx: &SettingsLineContext<'_>,
    setting: Option<SettingOption>,
    value: &str,
) {
    if let Some(error) = selected_inline_error(app, ctx.selection) {
        push_wrapped_setting_value(
            lines,
            line_targets,
            ctx,
            &error,
            ctx.selection.patch(Style::default().fg(INLINE_ERROR_COLOR)),
        );
    } else if app.overlays().is_settings_editing()
        && ctx.selection != PaneSelectionState::Unselected
    {
        let edit_buffer = render_edit_buffer(
            app.config.edit_buffer().buf(),
            app.config.edit_buffer().cursor(),
        );
        push_wrapped_setting_value(
            lines,
            line_targets,
            ctx,
            &edit_buffer,
            ctx.selection.patch(Style::default()),
        );
    } else if is_toggle_setting(setting) {
        push_toggle_row(
            lines,
            line_targets,
            value,
            ctx,
            nav_keys_toggle_suffix(app, setting, ctx.selection),
        );
    } else if setting == Some(SettingOption::CiRunCount)
        && ctx.selection != PaneSelectionState::Unselected
        && !app.overlays().is_settings_editing()
    {
        push_ci_run_count_row(lines, line_targets, ctx, value);
    } else if setting == Some(SettingOption::TerminalCommand)
        && value.starts_with("Not configured.")
    {
        push_wrapped_setting_value(
            lines,
            line_targets,
            ctx,
            value,
            ctx.selection.patch(Style::default().fg(INLINE_ERROR_COLOR)),
        );
    } else if setting == Some(SettingOption::LintCacheSize) {
        push_lint_cache_size_row(app, lines, line_targets, ctx, value);
    } else {
        push_wrapped_setting_value(
            lines,
            line_targets,
            ctx,
            value,
            ctx.selection.patch(Style::default()),
        );
    }
}

pub(super) fn build_settings_lines(
    app: &App,
    settings: &[SettingsRow],
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
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
            push_settings_header(lines, line_targets, name);
            continue;
        }

        let cursor = if app.overlays().settings_pane().viewport().pos() == selection_index {
            "▶ "
        } else {
            "  "
        };
        let selection = app
            .overlays()
            .settings_pane()
            .viewport()
            .selection_state(selection_index, app.focus.pane_state(PaneId::Settings));
        let setting = *setting;
        let label = format!("{SECTION_ITEM_INDENT}{cursor}{name:<max_label$}  ");
        let ctx = SettingsLineContext {
            selection_index,
            label: &label,
            selection,
            label_style,
            content_width,
        };
        push_setting_row(app, lines, line_targets, &ctx, setting, value);
        selection_index += 1;
    }
}

fn push_settings_header(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<usize>>,
    name: &str,
) {
    lines.push(Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            format!("{name}:"),
            Style::default()
                .fg(TITLE_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    line_targets.push(None);
}

fn selected_inline_error(app: &App, selection: PaneSelectionState) -> Option<String> {
    (selection != PaneSelectionState::Unselected && !app.overlays().is_settings_editing())
        .then(|| app.overlays().inline_error().cloned())
        .flatten()
}

pub(super) fn handle_settings_key(app: &mut App, key: KeyCode) {
    if app.overlays().is_settings_editing() {
        handle_settings_edit_key(app, key);
        return;
    }

    let setting = SettingOption::from_index(app.overlays().settings_pane().viewport().pos());

    match key {
        KeyCode::Esc | KeyCode::Char('s') => {
            if app.config.current().tui.include_dirs.is_empty() {
                app.overlays_mut()
                    .set_inline_error("Configure at least one include directory before continuing");
                return;
            }
            app.overlays_mut().close_settings();
            app.focus.close_overlay();
        },
        KeyCode::Up => {
            app.overlays_mut().clear_inline_error();
            app.overlays_mut().settings_pane_mut().viewport_mut().up();
        },
        KeyCode::Down => {
            app.overlays_mut().clear_inline_error();
            app.overlays_mut().settings_pane_mut().viewport_mut().down();
        },
        KeyCode::Left | KeyCode::Right => {
            app.overlays_mut().clear_inline_error();
            handle_settings_adjust_key(app, key, setting);
        },
        KeyCode::Enter | KeyCode::Char(' ') => {
            app.overlays_mut().clear_inline_error();
            handle_settings_activate_key(app, setting);
        },
        _ => {},
    }
}

fn handle_settings_adjust_key(app: &mut App, key: KeyCode, setting: Option<SettingOption>) {
    match setting {
        Some(SettingOption::InvertScroll) => {
            let mut config = app.config.current().clone();
            config.mouse.invert_scroll.toggle();
            let _ = save_updated_config(app, &config);
        },
        Some(SettingOption::NavigationKeys) => {
            toggle_vim_mode(app);
        },
        Some(SettingOption::CiRunCount) => {
            let mut config = app.config.current().clone();
            if key == KeyCode::Right {
                config.tui.ci_run_count = config.tui.ci_run_count.saturating_add(1);
            } else {
                config.tui.ci_run_count = config.tui.ci_run_count.saturating_sub(1).max(1);
            }
            let _ = save_updated_config(app, &config);
        },
        Some(SettingOption::IncludeNonRust) => {
            let mut config = app.config.current().clone();
            config.tui.include_non_rust.toggle();
            let _ = save_updated_config(app, &config);
        },
        Some(SettingOption::LintsEnabled) => {
            toggle_lints(app);
        },
        Some(SettingOption::LintOnDiscovery) => {
            let mut config = app.config.current().clone();
            config.lint.on_discovery.toggle();
            let _ = save_updated_config(app, &config);
        },
        Some(
            SettingOption::Editor
            | SettingOption::TerminalCommand
            | SettingOption::MainBranch
            | SettingOption::OtherPrimaryBranches
            | SettingOption::IncludeDirs
            | SettingOption::InlineDirs
            | SettingOption::StatusFlashSecs
            | SettingOption::TaskLingerSecs
            | SettingOption::DiscoveryShimmerSecs
            | SettingOption::CpuPollMs
            | SettingOption::CpuGreenMaxPercent
            | SettingOption::CpuYellowMaxPercent
            | SettingOption::LintProjects
            | SettingOption::LintCommands
            | SettingOption::LintCacheSize,
        )
        | None => {},
    }
}

fn finish_settings_edit_with_error(app: &mut App, error: impl Into<String>) {
    app.overlays_mut().end_settings_editing();
    app.config.edit_buffer_mut().set(String::new(), 0);
    app.overlays_mut().set_inline_error(error.into());
}

fn begin_settings_edit(app: &mut App, value: String) {
    app.overlays_mut().begin_settings_editing();
    let cursor = value.len();
    app.config.edit_buffer_mut().set(value, cursor);
}

fn handle_settings_activate_key(app: &mut App, setting: Option<SettingOption>) {
    match setting {
        Some(SettingOption::InvertScroll) => {
            let mut config = app.config.current().clone();
            config.mouse.invert_scroll.toggle();
            let _ = save_updated_config(app, &config);
        },
        Some(SettingOption::NavigationKeys) => {
            toggle_vim_mode(app);
        },
        Some(SettingOption::CiRunCount) => {
            begin_settings_edit(app, app.config.current().tui.ci_run_count.to_string());
        },
        Some(SettingOption::InlineDirs) => {
            begin_settings_edit(app, app.config.current().tui.inline_dirs.join(", "));
        },
        Some(SettingOption::IncludeDirs) => {
            begin_settings_edit(app, app.config.current().tui.include_dirs.join(", "));
        },
        Some(SettingOption::LintProjects) => {
            begin_settings_edit(app, app.config.current().lint.include.join(", "));
        },
        Some(SettingOption::LintCommands) => {
            begin_settings_edit(app, format_lint_commands(app.config.current()));
        },
        Some(SettingOption::LintCacheSize) => {
            begin_settings_edit(app, app.config.current().lint.cache_size.clone());
        },
        Some(SettingOption::StatusFlashSecs) => {
            begin_settings_edit(app, format_flash_secs(app.config.current()));
        },
        Some(SettingOption::TaskLingerSecs) => {
            begin_settings_edit(app, format_linger_secs(app.config.current()));
        },
        Some(SettingOption::DiscoveryShimmerSecs) => {
            begin_settings_edit(app, format_discovery_shimmer_secs(app.config.current()));
        },
        Some(SettingOption::CpuPollMs) => {
            begin_settings_edit(app, format_cpu_poll_ms(app.config.current()));
        },
        Some(SettingOption::CpuGreenMaxPercent) => {
            begin_settings_edit(app, format_cpu_green_max(app.config.current()));
        },
        Some(SettingOption::CpuYellowMaxPercent) => {
            begin_settings_edit(app, format_cpu_yellow_max(app.config.current()));
        },
        Some(SettingOption::IncludeNonRust) => {
            let mut config = app.config.current().clone();
            config.tui.include_non_rust.toggle();
            let _ = save_updated_config(app, &config);
        },
        Some(SettingOption::LintsEnabled) => {
            toggle_lints(app);
        },
        Some(SettingOption::LintOnDiscovery) => {
            let mut config = app.config.current().clone();
            config.lint.on_discovery.toggle();
            let _ = save_updated_config(app, &config);
        },
        Some(SettingOption::Editor) => {
            begin_settings_edit(app, app.config.editor().to_string());
        },
        Some(SettingOption::TerminalCommand) => {
            begin_settings_edit(app, app.config.current().tui.terminal_command.clone());
        },
        Some(SettingOption::MainBranch) => {
            begin_settings_edit(app, app.config.current().tui.main_branch.clone());
        },
        Some(SettingOption::OtherPrimaryBranches) => {
            begin_settings_edit(
                app,
                app.config.current().tui.other_primary_branches.join(", "),
            );
        },
        None => {},
    }
}

fn apply_settings_edit(app: &mut App) {
    let setting = SettingOption::from_index(app.overlays().settings_pane().viewport().pos());
    let value = app.config.edit_buffer().buf().to_string();
    let result = setting.map_or(Ok(()), |setting| {
        apply_settings_edit_for(app, setting, &value)
    });
    if let Err(err) = result {
        finish_settings_edit_with_error(app, err);
        return;
    }
    app.overlays_mut().end_settings_editing();
    app.config.edit_buffer_mut().set(String::new(), 0);
}

fn apply_settings_edit_for(
    app: &mut App,
    setting: SettingOption,
    value: &str,
) -> Result<(), String> {
    if apply_general_settings_edit(app, setting, value)? {
        return Ok(());
    }
    if apply_lint_settings_edit(app, setting, value)? {
        return Ok(());
    }
    Ok(())
}

fn apply_general_settings_edit(
    app: &mut App,
    setting: SettingOption,
    value: &str,
) -> Result<bool, String> {
    match setting {
        SettingOption::CiRunCount => {
            if !save_u32_setting(app, value, |config, count| config.tui.ci_run_count = count) {
                return Ok(true);
            }
        },
        SettingOption::InlineDirs => save_sorted_list_setting(app, value, |config, dirs| {
            config.tui.inline_dirs = dirs;
        }),
        SettingOption::IncludeDirs => save_sorted_list_setting(app, value, |config, dirs| {
            config.tui.include_dirs = dirs;
        }),
        SettingOption::Editor if !value.trim().is_empty() => {
            save_string_setting(app, value, |config, editor| config.tui.editor = editor);
        },
        SettingOption::TerminalCommand => {
            save_string_setting(app, value, |config, command| {
                config.tui.terminal_command = command;
            });
        },
        SettingOption::Editor
        | SettingOption::InvertScroll
        | SettingOption::IncludeNonRust
        | SettingOption::NavigationKeys
        | SettingOption::LintsEnabled
        | SettingOption::LintOnDiscovery
        | SettingOption::LintProjects
        | SettingOption::LintCommands
        | SettingOption::LintCacheSize => return Ok(false),
        SettingOption::MainBranch => {
            let mut config = app.config.current().clone();
            config.tui.main_branch = config::normalize_branch_name(value, "Main branch")?;
            let _ = save_updated_config(app, &config);
        },
        SettingOption::OtherPrimaryBranches => {
            let mut config = app.config.current().clone();
            config.tui.other_primary_branches =
                config::normalize_branch_list(&parse_dir_list(value), "Other primary branches")?;
            let _ = save_updated_config(app, &config);
        },
        SettingOption::StatusFlashSecs => {
            if !save_number_setting(app, value, |config, secs| {
                config.tui.status_flash_secs = secs;
            }) {
                return Ok(true);
            }
        },
        SettingOption::TaskLingerSecs => {
            if !save_number_setting(app, value, |config, secs| {
                config.tui.task_linger_secs = secs;
            }) {
                return Ok(true);
            }
        },
        SettingOption::DiscoveryShimmerSecs => {
            if !save_number_setting(app, value, |config, secs| {
                config.tui.discovery_shimmer_secs = secs;
            }) {
                return Ok(true);
            }
        },
        SettingOption::CpuPollMs => {
            if !save_u32_setting(app, value, |config, poll_ms| {
                config.cpu.poll_ms = u64::from(poll_ms);
            }) {
                return Ok(true);
            }
        },
        SettingOption::CpuGreenMaxPercent => {
            if !save_u32_setting(app, value, |config, percent| {
                config.cpu.green_max_percent = bounded_u8_from_u32(percent.min(100));
            }) {
                return Ok(true);
            }
        },
        SettingOption::CpuYellowMaxPercent => {
            if !save_u32_setting(app, value, |config, percent| {
                config.cpu.yellow_max_percent = bounded_u8_from_u32(percent.min(100));
            }) {
                return Ok(true);
            }
        },
    }
    Ok(true)
}

fn apply_lint_settings_edit(
    app: &mut App,
    setting: SettingOption,
    value: &str,
) -> Result<bool, String> {
    match setting {
        SettingOption::LintProjects => {
            save_sorted_list_setting(app, value, |config, dirs| config.lint.include = dirs);
            if app.overlays().inline_error().is_none() {
                app.show_timed_toast("Settings", "Lint projects updated");
            }
        },
        SettingOption::LintCommands => {
            let mut config = app.config.current().clone();
            config.lint.commands = parse_lint_commands(value);
            if save_updated_config(app, &config) {
                app.show_timed_toast("Settings", "Lint commands updated");
            }
        },
        SettingOption::LintCacheSize => {
            let mut config = app.config.current().clone();
            config.lint.cache_size =
                parse_lint_cache_size(value).map_err(|_| format!("Invalid cache size: {value}"))?;
            if save_updated_config(app, &config) {
                app.show_timed_toast("Settings", "Lint cache size updated");
            }
        },
        _ => return Ok(false),
    }
    Ok(true)
}

pub(super) fn handle_settings_edit_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Enter => {
            apply_settings_edit(app);
        },
        KeyCode::Esc => {
            app.overlays_mut().end_settings_editing();
            app.config.edit_buffer_mut().set(String::new(), 0);
        },
        KeyCode::Left => {
            let cursor = prev_char_boundary(
                app.config.edit_buffer().buf(),
                app.config.edit_buffer().cursor(),
            );
            let value = app.config.edit_buffer().buf().to_string();
            app.config.edit_buffer_mut().set(value, cursor);
        },
        KeyCode::Right => {
            let cursor = next_char_boundary(
                app.config.edit_buffer().buf(),
                app.config.edit_buffer().cursor(),
            );
            let value = app.config.edit_buffer().buf().to_string();
            app.config.edit_buffer_mut().set(value, cursor);
        },
        KeyCode::Home => {
            let value = app.config.edit_buffer().buf().to_string();
            app.config.edit_buffer_mut().set(value, 0);
        },
        KeyCode::End => {
            let value = app.config.edit_buffer().buf().to_string();
            app.config.edit_buffer_mut().set(value.clone(), value.len());
        },
        KeyCode::Backspace => {
            let (buf, cursor) = app.config.edit_buffer_mut().parts_mut();
            backspace_at_cursor(buf, cursor);
        },
        KeyCode::Delete => {
            let cursor = app.config.edit_buffer().cursor();
            let (buf, _) = app.config.edit_buffer_mut().parts_mut();
            delete_at_cursor(buf, cursor);
        },
        KeyCode::Char(c) => {
            let (buf, cursor) = app.config.edit_buffer_mut().parts_mut();
            insert_char_at_cursor(buf, cursor, c);
        },
        _ => {},
    }
}

fn toggle_lints(app: &mut App) {
    let mut config = app.config.current().clone();
    config.lint.enabled = !config.lint.enabled;
    if !save_updated_config(app, &config) {
        return;
    }
    app.show_timed_toast(
        "Settings",
        format!(
            "Lints {}",
            if config.lint.enabled {
                "enabled"
            } else {
                "disabled"
            }
        ),
    );
}

pub(super) fn focus_terminal_command(app: &mut App) {
    if let Some(index) =
        SettingOption::iter().position(|setting| setting == SettingOption::TerminalCommand)
    {
        app.overlays_mut()
            .settings_pane_mut()
            .viewport_mut()
            .set_pos(index);
    }
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
            SettingOption::from_index(5),
            Some(SettingOption::TerminalCommand)
        );
        assert_eq!(
            SettingOption::from_index(13),
            Some(SettingOption::CpuPollMs)
        );
        assert_eq!(
            SettingOption::from_index(14),
            Some(SettingOption::CpuGreenMaxPercent)
        );
        assert_eq!(
            SettingOption::from_index(15),
            Some(SettingOption::CpuYellowMaxPercent)
        );
        assert_eq!(
            SettingOption::from_index(16),
            Some(SettingOption::LintsEnabled)
        );
        assert_eq!(
            SettingOption::from_index(17),
            Some(SettingOption::LintOnDiscovery)
        );
        assert_eq!(
            SettingOption::from_index(18),
            Some(SettingOption::LintProjects)
        );
        assert_eq!(
            SettingOption::from_index(19),
            Some(SettingOption::LintCommands)
        );
        assert_eq!(
            SettingOption::from_index(20),
            Some(SettingOption::LintCacheSize)
        );
        assert_eq!(
            SettingOption::from_index(12),
            Some(SettingOption::DiscoveryShimmerSecs)
        );
        assert_eq!(
            SettingOption::from_index(6),
            Some(SettingOption::MainBranch)
        );
        assert_eq!(
            SettingOption::from_index(7),
            Some(SettingOption::OtherPrimaryBranches)
        );
        assert_eq!(SettingOption::COUNT, 21);
    }

    #[test]
    fn format_discovery_shimmer_secs_renders_whole_numbers_cleanly() {
        let mut config = config::CargoPortConfig::default();
        config.tui.discovery_shimmer_secs = 4.0;
        assert_eq!(format_discovery_shimmer_secs(&config), "4");
    }

    #[test]
    fn format_terminal_command_marks_blank_value_as_unconfigured() {
        let config = config::CargoPortConfig::default();

        assert!(format_terminal_command(&config).contains("Not configured"));
    }

    #[test]
    fn format_terminal_command_preserves_configured_value() {
        let mut config = config::CargoPortConfig::default();
        config.tui.terminal_command = "open -a Terminal .".to_string();

        assert_eq!(format_terminal_command(&config), "open -a Terminal .");
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
    fn other_primary_branches_preserve_input_order() {
        assert_eq!(
            parse_dir_list("release, main, primary"),
            vec![
                "release".to_string(),
                "main".to_string(),
                "primary".to_string()
            ]
        );
    }

    #[test]
    fn wrapped_rows_continue_at_value_column() {
        let mut lines = Vec::new();
        let mut line_targets = Vec::new();
        push_wrapped_value_row(
            &mut lines,
            &mut line_targets,
            Some(0),
            &WrappedValueRow {
                prefix:        "  Projects      ",
                value:         "alpha beta gamma delta epsilon",
                prefix_style:  Style::default(),
                value_style:   Style::default(),
                content_width: 24,
            },
        );

        assert!(lines.len() > 1);
        assert_eq!(line_targets.len(), lines.len());
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

    #[test]
    fn navigation_keys_selected_toggle_row_inlines_hint() {
        let mut lines = Vec::new();
        let mut line_targets = Vec::new();
        let ctx = SettingsLineContext {
            selection_index: 0,
            label:           "  ▶ Vim nav keys  ",
            selection:       PaneSelectionState::Active,
            label_style:     Style::default(),
            content_width:   80,
        };
        push_toggle_row(
            &mut lines,
            &mut line_targets,
            "ON",
            &ctx,
            Some("  maps h/j/k/l to arrow navigation"),
        );

        let rendered = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("< ON >  maps h/j/k/l to arrow navigation"));
    }
}
