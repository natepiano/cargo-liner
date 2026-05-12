use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use tui_pane::FrameworkOverlayId;
use tui_pane::SettingCodecs;
use tui_pane::SettingsCommand;
use tui_pane::SettingsError;
use tui_pane::SettingsFileSpec;
use tui_pane::SettingsPaneAction;
use tui_pane::SettingsRegistry;
use tui_pane::SettingsRenderOptions;
use tui_pane::SettingsRow as FrameworkSettingsRow;
use tui_pane::SettingsSection;
use tui_pane::SettingsStore;
use tui_pane::ToastDuration;
use tui_pane::ToastSettings;

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
use super::keymap_ui;
use super::pane::PaneFocusState;
use super::popup::PopupFrame;
use super::render;
use crate::config;
use crate::config::CargoPortConfig;
use crate::config::LintCommandConfig;
use crate::constants::APP_NAME;
use crate::constants::CONFIG_FILE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

fn parse_dir_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

type AppSettingsRow = (Option<SettingOption>, &'static str, String);

fn setting_at_selection(rows: &[AppSettingsRow], selection_index: usize) -> Option<SettingOption> {
    rows.iter()
        .filter_map(|(setting, _, _)| *setting)
        .nth(selection_index)
}

fn selected_setting(app: &App) -> Option<SettingOption> {
    let rows = settings_rows(app, app.config.current());
    setting_at_selection(&rows, app.framework.settings_pane.viewport().pos())
}

pub(super) fn selection_index_for_setting(app: &App, target: SettingOption) -> Option<usize> {
    settings_rows(app, app.config.current())
        .iter()
        .filter_map(|(setting, _, _)| *setting)
        .position(|setting| setting == target)
}

#[cfg(test)]
pub(super) fn selection_index_for_setting_for_test(
    app: &App,
    target: SettingOption,
) -> Option<usize> {
    selection_index_for_setting(app, target)
}

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

fn save_toast_number_setting(
    app: &mut App,
    value: &str,
    key: &'static str,
    apply: impl FnOnce(&mut ToastSettings, ToastDuration),
) -> bool {
    let Ok(number) = value.parse::<f64>() else {
        finish_settings_edit_with_error(app, format!("Invalid number: {value}"));
        return false;
    };
    let Ok(duration) = ToastDuration::try_from_secs(key, number) else {
        finish_settings_edit_with_error(app, format!("Invalid number: {value}"));
        return false;
    };
    let mut settings = app.framework.toast_settings().clone();
    apply(&mut settings, duration);
    save_toast_settings(app, settings)
}

fn save_toast_settings(app: &mut App, settings: ToastSettings) -> bool {
    let config = app.config.current().clone();
    match app.framework.settings_store_mut().save(&config, &settings) {
        Ok(()) => {
            app.framework.set_toast_settings(settings);
            app.show_timed_toast("Settings", "Saved");
            true
        },
        Err(err) => {
            app.overlays.set_inline_error(err.to_string());
            false
        },
    }
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

fn format_toast_duration_secs(duration: ToastDuration) -> String {
    format_secs(duration.as_secs_f64())
}

fn format_flash_secs(app: &App) -> String {
    format_toast_duration_secs(app.framework.toast_settings().default_timeout)
}

fn format_linger_secs(app: &App) -> String {
    format_toast_duration_secs(app.framework.toast_settings().task_linger)
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

pub(super) fn cargo_port_settings_registry() -> SettingsRegistry<App> {
    let registry = SettingsRegistry::<App>::new();
    let registry = register_general_settings(registry);
    let registry = register_cpu_settings(registry);
    register_lint_settings(registry)
}

fn register_general_settings(registry: SettingsRegistry<App>) -> SettingsRegistry<App> {
    registry
        .add_bool_in(
            SettingsSection::App("mouse"),
            "invert_scroll",
            get_invert_scroll,
            set_invert_scroll,
        )
        .add_bool_in(
            SettingsSection::App("tui"),
            "include_non_rust",
            get_include_non_rust,
            set_include_non_rust,
        )
        .add_bool_in(
            SettingsSection::App("tui"),
            "navigation_keys",
            get_navigation_keys,
            set_navigation_keys,
        )
        .add_int_in(
            SettingsSection::App("tui"),
            "ci_run_count",
            get_ci_run_count,
            set_ci_run_count,
        )
        .add_string_in(
            SettingsSection::App("tui"),
            "editor",
            get_editor,
            set_editor,
        )
        .add_string_in(
            SettingsSection::App("tui"),
            "terminal_command",
            get_terminal_command,
            set_terminal_command,
        )
        .add_string_in(
            SettingsSection::App("tui"),
            "main_branch",
            get_main_branch,
            set_main_branch,
        )
        .add_custom_in(
            SettingsSection::App("tui"),
            "other_primary_branches",
            SettingCodecs {
                format: format_other_primary_branches,
                parse:  set_other_primary_branches,
                adjust: None,
            },
        )
        .add_custom_in(
            SettingsSection::App("tui"),
            "include_dirs",
            SettingCodecs {
                format: format_include_dirs,
                parse:  set_include_dirs,
                adjust: None,
            },
        )
        .add_custom_in(
            SettingsSection::App("tui"),
            "inline_dirs",
            SettingCodecs {
                format: format_inline_dirs,
                parse:  set_inline_dirs,
                adjust: None,
            },
        )
        .add_float_in(
            SettingsSection::App("tui"),
            "discovery_shimmer_secs",
            get_discovery_shimmer_secs,
            set_discovery_shimmer_secs,
        )
}

fn register_cpu_settings(registry: SettingsRegistry<App>) -> SettingsRegistry<App> {
    registry
        .add_int_in(
            SettingsSection::App("cpu"),
            "poll_ms",
            get_cpu_poll_ms,
            set_cpu_poll_ms,
        )
        .add_int_in(
            SettingsSection::App("cpu"),
            "green_max_percent",
            get_cpu_green_max,
            set_cpu_green_max,
        )
        .add_int_in(
            SettingsSection::App("cpu"),
            "yellow_max_percent",
            get_cpu_yellow_max,
            set_cpu_yellow_max,
        )
}

fn register_lint_settings(registry: SettingsRegistry<App>) -> SettingsRegistry<App> {
    registry
        .add_bool_in(
            SettingsSection::App("lint"),
            "enabled",
            get_lints_enabled,
            set_lints_enabled,
        )
        .add_bool_in(
            SettingsSection::App("lint"),
            "on_discovery",
            get_lint_on_discovery,
            set_lint_on_discovery,
        )
        .add_custom_in(
            SettingsSection::App("lint"),
            "include",
            SettingCodecs {
                format: format_lint_projects,
                parse:  set_lint_projects,
                adjust: None,
            },
        )
        .add_custom_in(
            SettingsSection::App("lint"),
            "commands",
            SettingCodecs {
                format: format_lint_commands,
                parse:  set_lint_commands,
                adjust: None,
            },
        )
        .add_string_in(
            SettingsSection::App("lint"),
            "cache_size",
            format_lint_cache_size,
            set_lint_cache_size,
        )
}

pub(super) fn load_cargo_port_config_for_startup() -> Result<CargoPortConfig, String> {
    let config_path = config::config_path();
    let should_seed_file = config_path
        .as_ref()
        .is_some_and(|path| !path.as_path().exists());
    let settings_spec = config_path.as_ref().map_or_else(
        || SettingsFileSpec::new(APP_NAME, CONFIG_FILE),
        |path| SettingsFileSpec::new(APP_NAME, CONFIG_FILE).with_path(path.as_path()),
    );
    let mut loaded_settings =
        SettingsStore::<App>::load_for_startup(settings_spec, cargo_port_settings_registry())
            .map_err(|err| err.to_string())?;
    if should_seed_file {
        loaded_settings
            .store
            .save(
                &loaded_settings.app_settings,
                &loaded_settings.toast_settings,
            )
            .map_err(|err| err.to_string())?;
    }
    Ok(loaded_settings.app_settings)
}

fn settings_invalid(section: &str, key: &str, message: impl Into<String>) -> SettingsError {
    SettingsError::Invalid {
        section: section.to_string(),
        key:     key.to_string(),
        message: message.into(),
    }
}

fn validate_registered_config(config: &CargoPortConfig) -> Result<(), SettingsError> {
    config::normalize_config(config.clone())
        .map(|_| ())
        .map_err(|err| settings_invalid("app", "config", err))
}

const fn get_invert_scroll(config: &CargoPortConfig) -> bool {
    config.mouse.invert_scroll.is_inverted()
}

fn set_invert_scroll(config: &mut CargoPortConfig, value: bool) -> Result<(), SettingsError> {
    config.mouse.invert_scroll = value.into();
    validate_registered_config(config)
}

const fn get_include_non_rust(config: &CargoPortConfig) -> bool {
    config.tui.include_non_rust.includes_non_rust()
}

fn set_include_non_rust(config: &mut CargoPortConfig, value: bool) -> Result<(), SettingsError> {
    config.tui.include_non_rust = value.into();
    validate_registered_config(config)
}

const fn get_navigation_keys(config: &CargoPortConfig) -> bool {
    config.tui.navigation_keys.uses_vim()
}

fn set_navigation_keys(config: &mut CargoPortConfig, value: bool) -> Result<(), SettingsError> {
    config.tui.navigation_keys = value.into();
    validate_registered_config(config)
}

fn get_ci_run_count(config: &CargoPortConfig) -> i64 { i64::from(config.tui.ci_run_count) }

fn set_ci_run_count(config: &mut CargoPortConfig, value: i64) -> Result<(), SettingsError> {
    let count = u32::try_from(value)
        .map_err(|_| settings_invalid("tui", "ci_run_count", "expected positive integer"))?;
    config.tui.ci_run_count = count.max(1);
    Ok(())
}

fn get_editor(config: &CargoPortConfig) -> String { config.tui.editor.clone() }

fn set_editor(config: &mut CargoPortConfig, value: &str) -> Result<(), SettingsError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(settings_invalid("tui", "editor", "must not be empty"));
    }
    config.tui.editor = value.to_string();
    Ok(())
}

fn get_terminal_command(config: &CargoPortConfig) -> String { config.tui.terminal_command.clone() }

fn set_terminal_command(config: &mut CargoPortConfig, value: &str) -> Result<(), SettingsError> {
    config.tui.terminal_command = value.trim().to_string();
    validate_registered_config(config)
}

fn get_main_branch(config: &CargoPortConfig) -> String { config.tui.main_branch.clone() }

fn set_main_branch(config: &mut CargoPortConfig, value: &str) -> Result<(), SettingsError> {
    config.tui.main_branch = config::normalize_branch_name(value, "tui.main_branch")
        .map_err(|err| settings_invalid("tui", "main_branch", err))?;
    Ok(())
}

fn set_other_primary_branches(
    value: &str,
    config: &mut CargoPortConfig,
) -> Result<(), SettingsError> {
    config.tui.other_primary_branches =
        config::normalize_branch_list(&parse_dir_list(value), "tui.other_primary_branches")
            .map_err(|err| settings_invalid("tui", "other_primary_branches", err))?;
    Ok(())
}

fn format_include_dirs(config: &CargoPortConfig) -> String {
    format_sorted_list(&config.tui.include_dirs)
}

fn set_include_dirs(value: &str, config: &mut CargoPortConfig) -> Result<(), SettingsError> {
    config.tui.include_dirs = normalize_sorted_list(value);
    validate_registered_config(config)
}

fn format_inline_dirs(config: &CargoPortConfig) -> String {
    format_sorted_list(&config.tui.inline_dirs)
}

fn set_inline_dirs(value: &str, config: &mut CargoPortConfig) -> Result<(), SettingsError> {
    config.tui.inline_dirs = normalize_sorted_list(value);
    validate_registered_config(config)
}

const fn get_discovery_shimmer_secs(config: &CargoPortConfig) -> f64 {
    config.tui.discovery_shimmer_secs
}

fn set_discovery_shimmer_secs(
    config: &mut CargoPortConfig,
    value: f64,
) -> Result<(), SettingsError> {
    if !value.is_finite() || value < 0.0 {
        return Err(settings_invalid(
            "tui",
            "discovery_shimmer_secs",
            "expected non-negative finite seconds",
        ));
    }
    config.tui.discovery_shimmer_secs = value;
    Ok(())
}

fn get_cpu_poll_ms(config: &CargoPortConfig) -> i64 {
    i64::try_from(config.cpu.poll_ms).unwrap_or(i64::MAX)
}

fn set_cpu_poll_ms(config: &mut CargoPortConfig, value: i64) -> Result<(), SettingsError> {
    let poll_ms = u64::try_from(value)
        .map_err(|_| settings_invalid("cpu", "poll_ms", "expected positive integer"))?;
    config.cpu.poll_ms = poll_ms.max(250);
    Ok(())
}

fn get_cpu_green_max(config: &CargoPortConfig) -> i64 { i64::from(config.cpu.green_max_percent) }

fn set_cpu_green_max(config: &mut CargoPortConfig, value: i64) -> Result<(), SettingsError> {
    let percent = u8::try_from(value.clamp(0, 100)).unwrap_or(100);
    config.cpu.green_max_percent = percent;
    validate_registered_config(config)
}

fn get_cpu_yellow_max(config: &CargoPortConfig) -> i64 { i64::from(config.cpu.yellow_max_percent) }

fn set_cpu_yellow_max(config: &mut CargoPortConfig, value: i64) -> Result<(), SettingsError> {
    let percent = u8::try_from(value.clamp(0, 100)).unwrap_or(100);
    config.cpu.yellow_max_percent = percent;
    validate_registered_config(config)
}

const fn get_lints_enabled(config: &CargoPortConfig) -> bool { config.lint.enabled }

fn set_lints_enabled(config: &mut CargoPortConfig, value: bool) -> Result<(), SettingsError> {
    config.lint.enabled = value;
    validate_registered_config(config)
}

const fn get_lint_on_discovery(config: &CargoPortConfig) -> bool {
    config.lint.on_discovery.is_immediate()
}

fn set_lint_on_discovery(config: &mut CargoPortConfig, value: bool) -> Result<(), SettingsError> {
    config.lint.on_discovery = value.into();
    validate_registered_config(config)
}

fn set_lint_projects(value: &str, config: &mut CargoPortConfig) -> Result<(), SettingsError> {
    config.lint.include = normalize_sorted_list(value);
    validate_registered_config(config)
}

fn set_lint_commands(value: &str, config: &mut CargoPortConfig) -> Result<(), SettingsError> {
    config.lint.commands = parse_lint_commands(value);
    validate_registered_config(config)
}

fn set_lint_cache_size(config: &mut CargoPortConfig, value: &str) -> Result<(), SettingsError> {
    config.lint.cache_size = parse_lint_cache_size(value).map_err(|_| {
        settings_invalid("lint", "cache_size", format!("Invalid cache size: {value}"))
    })?;
    Ok(())
}

fn settings_rows(app: &App, config: &CargoPortConfig) -> Vec<AppSettingsRow> {
    let mut rows = general_settings_rows(app, config);
    rows.extend(toast_settings_rows(app, config));
    rows.extend(cpu_settings_rows(config));
    rows.extend(lint_settings_rows(app, config));
    rows
}

fn general_settings_rows(app: &App, config: &CargoPortConfig) -> Vec<AppSettingsRow> {
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

fn toast_settings_rows(app: &App, config: &CargoPortConfig) -> Vec<AppSettingsRow> {
    vec![
        (None, "Toasts", String::new()),
        (
            Some(SettingOption::StatusFlashSecs),
            "Status flash secs",
            format_flash_secs(app),
        ),
        (
            Some(SettingOption::TaskLingerSecs),
            "Task linger secs",
            format_linger_secs(app),
        ),
        (
            Some(SettingOption::DiscoveryShimmerSecs),
            "Discovery shimmer secs",
            format_discovery_shimmer_secs(config),
        ),
    ]
}

fn cpu_settings_rows(config: &CargoPortConfig) -> Vec<AppSettingsRow> {
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

fn lint_settings_rows(app: &App, config: &CargoPortConfig) -> Vec<AppSettingsRow> {
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
        let conflicts = keymap_ui::vim_mode_conflicts(app);
        if !conflicts.is_empty() {
            let msg = format!(
                "Cannot enable vim mode — these bindings use h/j/k/l:\n{}",
                conflicts.join(", ")
            );
            app.overlays.set_inline_error(msg);
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
            app.overlays.set_inline_error(err);
            false
        },
    }
}

pub(super) fn render_settings_popup(frame: &mut Frame, app: &mut App) {
    let rows = settings_rows(app, app.config.current());
    let content_width = usize::from(SETTINGS_POPUP_WIDTH.saturating_sub(2));

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    let framework_rows = framework_settings_rows(app, &rows);
    let rendered = app.framework.settings_pane.render_rows(
        &framework_rows,
        SettingsRenderOptions {
            active: app.framework.overlay() == Some(FrameworkOverlayId::Settings),
            inline_error: app.overlays.inline_error().map(String::as_str),
            content_width,
            section_header_indent: SECTION_HEADER_INDENT,
            section_item_indent: SECTION_ITEM_INDENT,
            title_style: Style::default().fg(TITLE_COLOR),
            label_style: Style::default().fg(LABEL_COLOR),
            muted_style: Style::default().fg(LABEL_COLOR),
            success_style: Style::default().fg(SUCCESS_COLOR),
            error_style: Style::default().fg(ERROR_COLOR),
            inline_error_style: Style::default().fg(INLINE_ERROR_COLOR),
            active_style: super::pane::Viewport::selection_style(PaneFocusState::Active),
            remembered_style: super::pane::Viewport::selection_style(PaneFocusState::Remembered),
            hovered_style: Style::default().bg(super::constants::HOVER_FOCUS_COLOR),
        },
    );
    lines.extend(rendered.lines);
    lines.push(Line::from(""));

    let popup_height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .saturating_add(1);

    app.framework
        .settings_pane
        .viewport_mut()
        .set_len(rendered.selectable_count);

    let inner = PopupFrame {
        title:        Some(" Settings ".to_string()),
        border_color: ACTIVE_BORDER_COLOR,
        width:        SETTINGS_POPUP_WIDTH,
        height:       popup_height,
    }
    .render(frame);

    app.framework
        .settings_pane
        .viewport_mut()
        .set_content_area(inner);

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn framework_settings_rows(app: &App, rows: &[AppSettingsRow]) -> Vec<FrameworkSettingsRow> {
    let selected = selected_setting(app);
    let mut selection_index = 0;
    let mut framework_rows = Vec::with_capacity(rows.len());
    for (setting, label, value) in rows {
        let Some(setting) = *setting else {
            framework_rows.push(FrameworkSettingsRow::section(*label));
            continue;
        };
        let mut row = if is_toggle_setting(Some(setting)) {
            FrameworkSettingsRow::toggle(selection_index, *label, value == "ON")
        } else if setting == SettingOption::CiRunCount {
            FrameworkSettingsRow::stepper(selection_index, *label, value.clone())
        } else {
            FrameworkSettingsRow::value(selection_index, *label, value.clone())
        };
        if setting == SettingOption::NavigationKeys
            && selected == Some(SettingOption::NavigationKeys)
            && !settings_is_editing(app)
        {
            row = row.with_suffix("  maps h/j/k/l to arrow navigation");
        }
        if setting == SettingOption::LintCacheSize {
            let used = render::format_bytes(app.lint.cache_usage.bytes);
            let limit = &app.config.current().lint.cache_size;
            row = row.with_suffix(format!("  {used} / {limit}"));
        }
        framework_rows.push(row);
        selection_index += 1;
    }
    framework_rows
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

const fn settings_is_editing(app: &App) -> bool { app.framework.settings_pane.is_editing() }

pub(super) fn dispatch_settings_action(action: SettingsPaneAction, app: &mut App) {
    let setting = selected_setting(app);
    match action {
        SettingsPaneAction::StartEdit => {
            app.overlays.clear_inline_error();
            handle_settings_activate_key(app, setting);
        },
        SettingsPaneAction::Save | SettingsPaneAction::Cancel => close_settings_overlay(app),
    }
}

pub(super) fn handle_settings_navigation_key(app: &mut App, key: KeyCode) {
    let setting = selected_setting(app);
    match key {
        KeyCode::Up => {
            app.overlays.clear_inline_error();
            app.framework.settings_pane.viewport_mut().up();
        },
        KeyCode::Down => {
            app.overlays.clear_inline_error();
            app.framework.settings_pane.viewport_mut().down();
        },
        KeyCode::Left | KeyCode::Right => {
            app.overlays.clear_inline_error();
            handle_settings_adjust_key(app, key, setting);
        },
        KeyCode::Enter | KeyCode::Char(' ') => {
            app.overlays.clear_inline_error();
            handle_settings_activate_key(app, setting);
        },
        _ => {},
    }
}

fn close_settings_overlay(app: &mut App) {
    if app.config.current().tui.include_dirs.is_empty() {
        app.overlays
            .set_inline_error("Configure at least one include directory before continuing");
        return;
    }
    app.overlays.close_settings();
    app.framework.settings_pane.enter_browse();
    app.close_framework_overlay_if_open();
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
    app.framework.settings_pane.enter_browse();
    app.overlays.set_inline_error(error.into());
}

fn begin_settings_edit(app: &mut App, value: String) {
    app.framework.settings_pane.begin_editing(value);
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
            begin_settings_edit(app, format_flash_secs(app));
        },
        Some(SettingOption::TaskLingerSecs) => {
            begin_settings_edit(app, format_linger_secs(app));
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
    let setting = selected_setting(app);
    let value = app.framework.settings_pane.edited_text().to_string();
    let result = setting.map_or(Ok(()), |setting| {
        apply_settings_edit_for(app, setting, &value)
    });
    if let Err(err) = result {
        finish_settings_edit_with_error(app, err);
        return;
    }
    app.framework.settings_pane.enter_browse();
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
        SettingOption::StatusFlashSecs => {
            if !save_toast_number_setting(app, value, "default_timeout", |settings, duration| {
                settings.default_timeout = duration;
            }) {
                return Ok(true);
            }
        },
        SettingOption::TaskLingerSecs => {
            if !save_toast_number_setting(app, value, "task_linger", |settings, duration| {
                settings.task_linger = duration;
            }) {
                return Ok(true);
            }
        },
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
            if app.overlays.inline_error().is_none() {
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

pub(super) fn handle_settings_text_command(app: &mut App, command: SettingsCommand) {
    match command {
        SettingsCommand::None => {},
        SettingsCommand::Save => apply_settings_edit(app),
        SettingsCommand::Cancel => app.framework.settings_pane.enter_browse(),
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
    if let Some(index) = selection_index_for_setting(app, SettingOption::TerminalCommand) {
        app.framework.settings_pane.viewport_mut().set_pos(index);
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn setting_selection_ignores_section_headers() {
        let rows = vec![
            (None, "General", String::new()),
            (
                Some(SettingOption::InvertScroll),
                "Invert scroll",
                "ON".to_string(),
            ),
            (None, "Toasts", String::new()),
            (
                Some(SettingOption::StatusFlashSecs),
                "Status flash secs",
                "5".to_string(),
            ),
        ];

        assert_eq!(
            setting_at_selection(&rows, 0),
            Some(SettingOption::InvertScroll)
        );
        assert_eq!(
            setting_at_selection(&rows, 1),
            Some(SettingOption::StatusFlashSecs)
        );
        assert_eq!(setting_at_selection(&rows, 2), None);
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
            "cargo mend --manifest-path \"$MANIFEST_PATH\" --all-targets, cargo clippy --workspace",
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
    fn settings_store_saves_app_settings_and_framework_toasts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let settings_spec = SettingsFileSpec::new(APP_NAME, CONFIG_FILE).with_path(&path);
        let mut loaded =
            SettingsStore::<App>::load_for_startup(settings_spec, cargo_port_settings_registry())
                .expect("load settings");
        let mut config = CargoPortConfig::default();
        config.tui.ci_run_count = 9;
        let toast_settings = ToastSettings {
            default_timeout: ToastDuration::try_from_secs("default_timeout", 3.0)
                .expect("toast duration"),
            ..ToastSettings::default()
        };

        loaded
            .store
            .save(&config, &toast_settings)
            .expect("save settings");

        let saved = std::fs::read_to_string(path).expect("read saved config");
        assert!(saved.contains("ci_run_count = 9"));
        assert!(saved.contains("[toasts]"));
        assert!(saved.contains("default_timeout = 3.0"));
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
}
