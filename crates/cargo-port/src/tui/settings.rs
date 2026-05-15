use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use toml::Table;
use toml::Value;
use tui_pane::ACTIVE_BORDER_COLOR;
use tui_pane::ERROR_COLOR;
use tui_pane::FrameworkOverlayId;
use tui_pane::INLINE_ERROR_COLOR;
use tui_pane::LABEL_COLOR;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::SUCCESS_COLOR;
use tui_pane::SettingCodecs;
use tui_pane::SettingsCommand;
use tui_pane::SettingsError;
use tui_pane::SettingsFileSpec;
use tui_pane::SettingsPane;
use tui_pane::SettingsPaneAction;
use tui_pane::SettingsRegistry;
use tui_pane::SettingsRenderOptions;
use tui_pane::SettingsRow as FrameworkSettingsRow;
use tui_pane::SettingsSection;
use tui_pane::SettingsStore;
use tui_pane::TITLE_COLOR;
use tui_pane::ToastDuration;
use tui_pane::ToastSettings;
use tui_pane::ViewportOverflow;
use tui_pane::read_array;
use tui_pane::read_bool;
use tui_pane::read_float;
use tui_pane::read_int;
use tui_pane::read_string;
use tui_pane::render_overflow_affordance;
use tui_pane::write_value;

use super::app::App;
use super::constants::SETTINGS_POPUP_WIDTH;
use super::keymap_ui;
use super::overlays::PopupFrame;
use super::pane;
use super::pane::PaneFocusState;
use super::pane::PaneRenderCtx;
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
    StatusToastVisibleSecs,
    FinishedTaskVisibleSecs,
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

type SettingsUiRow = (Option<SettingOption>, &'static str, String);

fn setting_at_selection(rows: &[SettingsUiRow], selection_index: usize) -> Option<SettingOption> {
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

fn settings_popup_height(line_count: usize, area_height: u16) -> u16 {
    let content_height = u16::try_from(line_count)
        .unwrap_or(u16::MAX)
        .saturating_add(3);
    content_height.min(area_height.saturating_sub(2))
}

fn settings_scroll_offset(selected_line: usize, visible_height: usize, line_count: usize) -> usize {
    if visible_height == 0 {
        return 0;
    }
    selected_line
        .saturating_sub(visible_height.saturating_sub(1))
        .min(line_count.saturating_sub(visible_height))
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

fn restore_settings_table(app: &mut App, snapshot: Table) {
    app.framework.settings_store_mut().replace_table(snapshot);
}

fn save_app_table_update(
    app: &mut App,
    mutate: impl FnOnce(&mut Table) -> Result<(), SettingsError>,
) -> Result<CargoPortConfig, String> {
    let snapshot = app.framework.settings_store().table().clone();
    if let Err(err) = mutate(app.framework.settings_store_mut().table_mut()) {
        restore_settings_table(app, snapshot);
        return Err(err.to_string());
    }
    let next = match CargoPortConfig::from_table(app.framework.settings_store().table()) {
        Ok(config) => config,
        Err(err) => {
            restore_settings_table(app, snapshot);
            return Err(err);
        },
    };
    if let Err(err) = app.framework.settings_store_mut().save() {
        restore_settings_table(app, snapshot);
        return Err(err.to_string());
    }
    app.apply_config(&next);
    app.config.sync_stamp();
    Ok(next)
}

fn save_app_setting(
    app: &mut App,
    mutate: impl FnOnce(&mut Table) -> Result<(), SettingsError>,
) -> bool {
    match save_app_table_update(app, mutate) {
        Ok(_) => true,
        Err(err) => {
            app.overlays.set_inline_error(err);
            false
        },
    }
}

fn save_app_setting_with_toast(
    app: &mut App,
    mutate: impl FnOnce(&mut Table) -> Result<(), SettingsError>,
) -> bool {
    let saved = save_app_setting(app, mutate);
    if saved {
        app.show_timed_toast("Settings", "Saved");
    }
    saved
}

fn save_number_setting(
    app: &mut App,
    value: &str,
    apply: impl FnOnce(&mut Table, f64) -> Result<(), SettingsError>,
) -> bool {
    let Ok(number) = value.parse::<f64>() else {
        finish_settings_edit_with_error(app, format!("Invalid number: {value}"));
        return false;
    };
    save_app_setting(app, |table| apply(table, number))
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
    save_toast_settings(app, &settings)
}

fn save_toast_settings(app: &mut App, settings: &ToastSettings) -> bool {
    let snapshot = app.framework.settings_store().table().clone();
    settings.write_to_table(app.framework.settings_store_mut().table_mut());
    let next = match ToastSettings::from_table(app.framework.settings_store().table()) {
        Ok(settings) => settings,
        Err(err) => {
            restore_settings_table(app, snapshot);
            app.overlays.set_inline_error(err.to_string());
            return false;
        },
    };
    match app.framework.settings_store_mut().save() {
        Ok(()) => {
            app.framework.set_toast_settings(next);
            app.show_timed_toast("Settings", "Saved");
            true
        },
        Err(err) => {
            restore_settings_table(app, snapshot);
            app.overlays.set_inline_error(err.to_string());
            false
        },
    }
}

fn save_sorted_list_setting(
    app: &mut App,
    value: &str,
    apply: impl FnOnce(&mut Table, Vec<String>) -> Result<(), SettingsError>,
) {
    let values = normalize_sorted_list(value);
    let _ = save_app_setting(app, |table| apply(table, values));
}

fn save_u32_setting(
    app: &mut App,
    value: &str,
    apply: impl FnOnce(&mut Table, u32) -> Result<(), SettingsError>,
) -> bool {
    let Ok(number) = value.parse::<u32>() else {
        finish_settings_edit_with_error(app, format!("Invalid number: {value}"));
        return false;
    };
    save_app_setting(app, |table| apply(table, number.max(1)))
}

fn bounded_u8_from_u32(value: u32) -> u8 {
    u8::try_from(value.min(u32::from(u8::MAX))).unwrap_or(u8::MAX)
}

fn save_string_setting(
    app: &mut App,
    value: &str,
    apply: impl FnOnce(&mut Table, String) -> Result<(), SettingsError>,
) {
    let value = value.trim().to_string();
    let _ = save_app_setting(app, |table| apply(table, value));
}

fn format_lint_commands_from_commands(commands: &[LintCommandConfig]) -> String {
    commands
        .iter()
        .map(|command| command.command.trim().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_lint_commands(config: &CargoPortConfig) -> String {
    let commands = if config.lint.commands.is_empty() {
        config.lint.resolved_commands()
    } else {
        config.lint.commands.clone()
    };
    format_lint_commands_from_commands(&commands)
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

fn format_status_toast_visible_secs(app: &App) -> String {
    format_toast_duration_secs(app.framework.toast_settings().status_toast_visible)
}

fn format_finished_task_visible_secs(app: &App) -> String {
    format_toast_duration_secs(app.framework.toast_settings().finished_task_visible)
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

pub(super) fn cargo_port_settings_registry() -> SettingsRegistry {
    let registry = SettingsRegistry::new();
    let registry = register_general_settings(registry);
    let registry = register_cpu_settings(registry);
    register_lint_settings(registry)
}

fn register_general_settings(registry: SettingsRegistry) -> SettingsRegistry {
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
                format: format_other_primary_branches_table,
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

fn register_cpu_settings(registry: SettingsRegistry) -> SettingsRegistry {
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

fn register_lint_settings(registry: SettingsRegistry) -> SettingsRegistry {
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
                format: format_lint_projects_table,
                parse:  set_lint_projects,
                adjust: None,
            },
        )
        .add_custom_in(
            SettingsSection::App("lint"),
            "commands",
            SettingCodecs {
                format: format_lint_commands_table,
                parse:  set_lint_commands,
                adjust: None,
            },
        )
        .add_string_in(
            SettingsSection::App("lint"),
            "cache_size",
            format_lint_cache_size_table,
            set_lint_cache_size,
        )
}

pub(super) struct StartupSettings {
    pub(super) config:         CargoPortConfig,
    pub(super) store:          SettingsStore,
    pub(super) toast_settings: ToastSettings,
}

pub(super) fn load_cargo_port_settings_for_startup() -> Result<StartupSettings, String> {
    let config_path = config::config_path();
    let should_seed_file = config_path
        .as_ref()
        .is_some_and(|path| !path.as_path().exists());
    let settings_spec = config_path.as_ref().map_or_else(
        || SettingsFileSpec::new(APP_NAME, CONFIG_FILE),
        |path| SettingsFileSpec::new(APP_NAME, CONFIG_FILE).with_path(path.as_path()),
    );
    let mut loaded_settings =
        SettingsStore::load_for_startup(settings_spec, cargo_port_settings_registry())
            .map_err(|err| err.to_string())?;
    if should_seed_file {
        *loaded_settings.store.table_mut() =
            settings_table_from_config(&default_config()).map_err(|err| err.to_string())?;
        loaded_settings
            .toast_settings
            .write_to_table(loaded_settings.store.table_mut());
        loaded_settings
            .store
            .save()
            .map_err(|err| err.to_string())?;
    }
    let config = CargoPortConfig::from_table(loaded_settings.store.table())?;
    Ok(StartupSettings {
        config,
        store: loaded_settings.store,
        toast_settings: loaded_settings.toast_settings,
    })
}

pub(super) fn settings_table_from_config(config: &CargoPortConfig) -> Result<Table, SettingsError> {
    let mut table = Table::new();
    set_invert_scroll(&mut table, config.mouse.invert_scroll.is_inverted())?;
    set_include_non_rust(&mut table, config.tui.include_non_rust.includes_non_rust())?;
    set_navigation_keys(&mut table, config.tui.navigation_keys.uses_vim())?;
    set_ci_run_count(&mut table, i64::from(config.tui.ci_run_count))?;
    set_editor(&mut table, &config.tui.editor)?;
    set_terminal_command(&mut table, &config.tui.terminal_command)?;
    set_main_branch(&mut table, &config.tui.main_branch)?;
    write_string_array(
        &mut table,
        "tui",
        "other_primary_branches",
        config.tui.other_primary_branches.clone(),
    )?;
    write_string_array(
        &mut table,
        "tui",
        "include_dirs",
        config.tui.include_dirs.clone(),
    )?;
    write_string_array(
        &mut table,
        "tui",
        "inline_dirs",
        config.tui.inline_dirs.clone(),
    )?;
    set_discovery_shimmer_secs(&mut table, config.tui.discovery_shimmer_secs)?;
    set_cpu_poll_ms(
        &mut table,
        i64::try_from(config.cpu.poll_ms).unwrap_or(i64::MAX),
    )?;
    set_cpu_green_max(&mut table, i64::from(config.cpu.green_max_percent))?;
    set_cpu_yellow_max(&mut table, i64::from(config.cpu.yellow_max_percent))?;
    set_lints_enabled(&mut table, config.lint.enabled)?;
    set_lint_on_discovery(&mut table, config.lint.on_discovery.is_immediate())?;
    write_string_array(&mut table, "lint", "include", config.lint.include.clone())?;
    write_value(
        &mut table,
        "lint",
        "commands",
        lint_commands_value(config.lint.commands.clone()),
    )?;
    set_lint_cache_size(&mut table, &config.lint.cache_size)?;
    Ok(table)
}

fn settings_invalid(section: &str, key: &str, message: impl Into<String>) -> SettingsError {
    SettingsError::Invalid {
        section: section.to_string(),
        key:     key.to_string(),
        message: message.into(),
    }
}

fn default_config() -> CargoPortConfig { CargoPortConfig::default() }

fn string_array_value(values: Vec<String>) -> Value {
    Value::Array(values.into_iter().map(Value::String).collect())
}

fn read_string_array(table: &Table, section: &str, key: &str, default: Vec<String>) -> Vec<String> {
    read_array(table, section, key).map_or(default, |values| {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>()
    })
}

fn write_string_array(
    table: &mut Table,
    section: &str,
    key: &str,
    values: Vec<String>,
) -> Result<(), SettingsError> {
    write_value(table, section, key, string_array_value(values))
}

fn get_invert_scroll(table: &Table) -> bool {
    read_bool(table, "mouse", "invert_scroll")
        .unwrap_or_else(|| default_config().mouse.invert_scroll.is_inverted())
}

fn set_invert_scroll(table: &mut Table, value: bool) -> Result<(), SettingsError> {
    write_value(table, "mouse", "invert_scroll", value.into())
}

fn get_include_non_rust(table: &Table) -> bool {
    read_bool(table, "tui", "include_non_rust")
        .unwrap_or_else(|| default_config().tui.include_non_rust.includes_non_rust())
}

fn set_include_non_rust(table: &mut Table, value: bool) -> Result<(), SettingsError> {
    write_value(table, "tui", "include_non_rust", value.into())
}

fn get_navigation_keys(table: &Table) -> bool {
    read_bool(table, "tui", "navigation_keys")
        .unwrap_or_else(|| default_config().tui.navigation_keys.uses_vim())
}

fn set_navigation_keys(table: &mut Table, value: bool) -> Result<(), SettingsError> {
    write_value(table, "tui", "navigation_keys", value.into())
}

fn get_ci_run_count(table: &Table) -> i64 {
    read_int(table, "tui", "ci_run_count")
        .unwrap_or_else(|| i64::from(default_config().tui.ci_run_count))
}

fn set_ci_run_count(table: &mut Table, value: i64) -> Result<(), SettingsError> {
    let count = u32::try_from(value)
        .map_err(|_| settings_invalid("tui", "ci_run_count", "expected positive integer"))?;
    write_value(table, "tui", "ci_run_count", i64::from(count.max(1)).into())
}

fn get_editor(table: &Table) -> String {
    read_string(table, "tui", "editor").map_or_else(|| default_config().tui.editor, str::to_string)
}

fn set_editor(table: &mut Table, value: &str) -> Result<(), SettingsError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(settings_invalid("tui", "editor", "must not be empty"));
    }
    write_value(table, "tui", "editor", value.into())
}

fn get_terminal_command(table: &Table) -> String {
    read_string(table, "tui", "terminal_command")
        .map_or_else(|| default_config().tui.terminal_command, str::to_string)
}

fn set_terminal_command(table: &mut Table, value: &str) -> Result<(), SettingsError> {
    write_value(table, "tui", "terminal_command", value.trim().into())
}

fn get_main_branch(table: &Table) -> String {
    read_string(table, "tui", "main_branch")
        .map_or_else(|| default_config().tui.main_branch, str::to_string)
}

fn set_main_branch(table: &mut Table, value: &str) -> Result<(), SettingsError> {
    let branch = config::normalize_branch_name(value, "tui.main_branch")
        .map_err(|err| settings_invalid("tui", "main_branch", err))?;
    write_value(table, "tui", "main_branch", branch.into())
}

fn set_other_primary_branches(value: &str, table: &mut Table) -> Result<(), SettingsError> {
    let branches =
        config::normalize_branch_list(&parse_dir_list(value), "tui.other_primary_branches")
            .map_err(|err| settings_invalid("tui", "other_primary_branches", err))?;
    write_string_array(table, "tui", "other_primary_branches", branches)
}

fn format_other_primary_branches_table(table: &Table) -> String {
    let values = read_string_array(
        table,
        "tui",
        "other_primary_branches",
        default_config().tui.other_primary_branches,
    );
    if values.is_empty() {
        "—".to_string()
    } else {
        values.join(", ")
    }
}

fn format_include_dirs(table: &Table) -> String {
    let default = default_config().tui.include_dirs;
    format_sorted_list(&read_string_array(table, "tui", "include_dirs", default))
}

fn set_include_dirs(value: &str, table: &mut Table) -> Result<(), SettingsError> {
    write_string_array(table, "tui", "include_dirs", normalize_sorted_list(value))
}

fn format_inline_dirs(table: &Table) -> String {
    let default = default_config().tui.inline_dirs;
    format_sorted_list(&read_string_array(table, "tui", "inline_dirs", default))
}

fn set_inline_dirs(value: &str, table: &mut Table) -> Result<(), SettingsError> {
    write_string_array(table, "tui", "inline_dirs", normalize_sorted_list(value))
}

fn get_discovery_shimmer_secs(table: &Table) -> f64 {
    read_float(table, "tui", "discovery_shimmer_secs")
        .unwrap_or_else(|| default_config().tui.discovery_shimmer_secs)
}

fn set_discovery_shimmer_secs(table: &mut Table, value: f64) -> Result<(), SettingsError> {
    if !value.is_finite() || value < 0.0 {
        return Err(settings_invalid(
            "tui",
            "discovery_shimmer_secs",
            "expected non-negative finite seconds",
        ));
    }
    write_value(table, "tui", "discovery_shimmer_secs", value.into())
}

fn get_cpu_poll_ms(table: &Table) -> i64 {
    read_int(table, "cpu", "poll_ms")
        .unwrap_or_else(|| i64::try_from(default_config().cpu.poll_ms).unwrap_or(i64::MAX))
}

fn set_cpu_poll_ms(table: &mut Table, value: i64) -> Result<(), SettingsError> {
    let poll_ms = u64::try_from(value)
        .map_err(|_| settings_invalid("cpu", "poll_ms", "expected positive integer"))?;
    write_value(
        table,
        "cpu",
        "poll_ms",
        i64::try_from(poll_ms.max(250)).unwrap_or(i64::MAX).into(),
    )
}

fn get_cpu_green_max(table: &Table) -> i64 {
    read_int(table, "cpu", "green_max_percent")
        .unwrap_or_else(|| i64::from(default_config().cpu.green_max_percent))
}

fn set_cpu_green_max(table: &mut Table, value: i64) -> Result<(), SettingsError> {
    let percent = u8::try_from(value.clamp(0, 100)).unwrap_or(100);
    write_value(table, "cpu", "green_max_percent", i64::from(percent).into())
}

fn get_cpu_yellow_max(table: &Table) -> i64 {
    read_int(table, "cpu", "yellow_max_percent")
        .unwrap_or_else(|| i64::from(default_config().cpu.yellow_max_percent))
}

fn set_cpu_yellow_max(table: &mut Table, value: i64) -> Result<(), SettingsError> {
    let percent = u8::try_from(value.clamp(0, 100)).unwrap_or(100);
    write_value(
        table,
        "cpu",
        "yellow_max_percent",
        i64::from(percent).into(),
    )
}

fn get_lints_enabled(table: &Table) -> bool {
    read_bool(table, "lint", "enabled").unwrap_or_else(|| default_config().lint.enabled)
}

fn set_lints_enabled(table: &mut Table, value: bool) -> Result<(), SettingsError> {
    write_value(table, "lint", "enabled", value.into())
}

fn get_lint_on_discovery(table: &Table) -> bool {
    read_bool(table, "lint", "on_discovery")
        .unwrap_or_else(|| default_config().lint.on_discovery.is_immediate())
}

fn set_lint_on_discovery(table: &mut Table, value: bool) -> Result<(), SettingsError> {
    write_value(table, "lint", "on_discovery", value.into())
}

fn format_lint_projects_table(table: &Table) -> String {
    let values = read_string_array(table, "lint", "include", default_config().lint.include);
    if values.is_empty() {
        "—".to_string()
    } else {
        format_sorted_list(&values)
    }
}

fn set_lint_projects(value: &str, table: &mut Table) -> Result<(), SettingsError> {
    write_string_array(table, "lint", "include", normalize_sorted_list(value))
}

fn read_lint_commands(table: &Table) -> Vec<LintCommandConfig> {
    read_array(table, "lint", "commands")
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_table)
        .map(|command| LintCommandConfig {
            name:    command
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            command: command
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .collect()
}

fn lint_commands_value(commands: Vec<LintCommandConfig>) -> Value {
    Value::Array(
        commands
            .into_iter()
            .map(|command| {
                let mut table = Table::new();
                table.insert("name".to_string(), command.name.into());
                table.insert("command".to_string(), command.command.into());
                Value::Table(table)
            })
            .collect(),
    )
}

fn format_lint_commands_table(table: &Table) -> String {
    let commands = read_lint_commands(table);
    let commands = if commands.is_empty() {
        default_config().lint.resolved_commands()
    } else {
        commands
    };
    format_lint_commands_from_commands(&commands)
}

fn set_lint_commands(value: &str, table: &mut Table) -> Result<(), SettingsError> {
    write_value(
        table,
        "lint",
        "commands",
        lint_commands_value(parse_lint_commands(value)),
    )
}

fn format_lint_cache_size_table(table: &Table) -> String {
    read_string(table, "lint", "cache_size")
        .map_or_else(|| default_config().lint.cache_size, str::to_string)
}

fn set_lint_cache_size(table: &mut Table, value: &str) -> Result<(), SettingsError> {
    let cache_size = parse_lint_cache_size(value).map_err(|_| {
        settings_invalid("lint", "cache_size", format!("Invalid cache size: {value}"))
    })?;
    write_value(table, "lint", "cache_size", cache_size.into())
}

fn settings_rows(app: &App, config: &CargoPortConfig) -> Vec<SettingsUiRow> {
    let mut rows = general_settings_rows(app, config);
    rows.extend(toast_settings_rows(app, config));
    rows.extend(cpu_settings_rows(config));
    rows.extend(lint_settings_rows(app, config));
    rows
}

fn general_settings_rows(app: &App, config: &CargoPortConfig) -> Vec<SettingsUiRow> {
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

fn toast_settings_rows(app: &App, config: &CargoPortConfig) -> Vec<SettingsUiRow> {
    vec![
        (None, "Toasts", String::new()),
        (
            Some(SettingOption::StatusToastVisibleSecs),
            "Status toast visible secs",
            format_status_toast_visible_secs(app),
        ),
        (
            Some(SettingOption::FinishedTaskVisibleSecs),
            "Finished task visible secs",
            format_finished_task_visible_secs(app),
        ),
        (
            Some(SettingOption::DiscoveryShimmerSecs),
            "Discovery shimmer secs",
            format_discovery_shimmer_secs(config),
        ),
    ]
}

fn cpu_settings_rows(config: &CargoPortConfig) -> Vec<SettingsUiRow> {
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

fn lint_settings_rows(app: &App, config: &CargoPortConfig) -> Vec<SettingsUiRow> {
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
    let next = !app.config.navigation_keys().uses_vim();
    let _ = save_app_setting_with_toast(app, |table| set_navigation_keys(table, next));
}

/// Precomputed render inputs for the Settings overlay. Built in
/// [`prepare_settings_render_inputs`] before `App::split_for_render`
/// runs; consumed by `SettingsPane`'s [`tui_pane::Renderable`] impl
/// via [`crate::tui::pane::PaneRenderCtx`].
pub(crate) struct SettingsRenderInputs {
    pub lines:            Vec<Line<'static>>,
    pub line_count:       usize,
    pub selectable_count: usize,
    pub popup_height:     u16,
}

/// Build the [`SettingsRenderInputs`] for the current frame. Takes
/// `&mut App` because [`tui_pane::SettingsPane::render_rows`]
/// records line-target metadata on the pane.
pub(super) fn prepare_settings_render_inputs(
    app: &mut App,
    frame_area_height: u16,
) -> SettingsRenderInputs {
    let rows = settings_rows(app, app.config.current());
    let content_width = usize::from(SETTINGS_POPUP_WIDTH.saturating_sub(2));
    let framework_rows = framework_settings_rows(app, &rows);
    let render_options = SettingsRenderOptions {
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
        active_style: pane::selection_style(PaneFocusState::Active),
        remembered_style: pane::selection_style(PaneFocusState::Remembered),
        hovered_style: Style::default().bg(tui_pane::HOVER_FOCUS_COLOR),
    };
    let rendered = app
        .framework
        .settings_pane
        .render_rows(&framework_rows, render_options);
    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.extend(rendered.lines);
    lines.push(Line::from(""));
    let line_count = lines.len();
    let popup_height = settings_popup_height(line_count, frame_area_height);
    SettingsRenderInputs {
        lines,
        line_count,
        selectable_count: rendered.selectable_count,
        popup_height,
    }
}

/// Body fn invoked by `SettingsPane`'s [`tui_pane::Renderable`]
/// impl. Reads precomputed inputs from `ctx`, viewport state from
/// `pane`, and draws the popup into `area`.
pub(super) fn render_settings_pane_body(
    frame: &mut Frame,
    pane: &mut SettingsPane,
    ctx: &PaneRenderCtx<'_>,
) {
    let Some(inputs) = ctx.settings_render_inputs else {
        return;
    };

    pane.viewport_mut().set_len(inputs.selectable_count);

    let popup = PopupFrame {
        title:        Some(" Settings ".to_string()),
        border_color: ACTIVE_BORDER_COLOR,
        width:        SETTINGS_POPUP_WIDTH,
        height:       inputs.popup_height,
    }
    .render_with_areas(frame);
    let inner = popup.inner;

    pane.viewport_mut().set_content_area(inner);
    let visible_height = usize::from(inner.height);
    let selected_line = pane
        .line_for_selection(pane.viewport().pos())
        .unwrap_or_else(|| pane.viewport().pos());
    let scroll_offset = settings_scroll_offset(selected_line, visible_height, inputs.line_count);
    pane.viewport_mut().set_viewport_rows(visible_height);
    pane.viewport_mut().set_scroll_offset(scroll_offset);

    let paragraph =
        Paragraph::new(inputs.lines.clone()).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
    frame.render_widget(paragraph, inner);
    render_overflow_affordance(
        frame,
        popup.outer,
        ViewportOverflow::new(inputs.line_count, scroll_offset, visible_height),
        Style::default().fg(LABEL_COLOR),
    );
}

fn framework_settings_rows(app: &App, rows: &[SettingsUiRow]) -> Vec<FrameworkSettingsRow> {
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
            let next = !app.config.invert_scroll().is_inverted();
            let _ = save_app_setting_with_toast(app, |table| set_invert_scroll(table, next));
        },
        Some(SettingOption::NavigationKeys) => {
            toggle_vim_mode(app);
        },
        Some(SettingOption::CiRunCount) => {
            let current = app.config.current().tui.ci_run_count;
            let next = if key == KeyCode::Right {
                current.saturating_add(1)
            } else {
                current.saturating_sub(1).max(1)
            };
            let _ =
                save_app_setting_with_toast(app, |table| set_ci_run_count(table, i64::from(next)));
        },
        Some(SettingOption::IncludeNonRust) => {
            let next = !app.config.include_non_rust().includes_non_rust();
            let _ = save_app_setting_with_toast(app, |table| set_include_non_rust(table, next));
        },
        Some(SettingOption::LintsEnabled) => {
            toggle_lints(app);
        },
        Some(SettingOption::LintOnDiscovery) => {
            let next = !app.config.current().lint.on_discovery.is_immediate();
            let _ = save_app_setting_with_toast(app, |table| set_lint_on_discovery(table, next));
        },
        Some(
            SettingOption::Editor
            | SettingOption::TerminalCommand
            | SettingOption::MainBranch
            | SettingOption::OtherPrimaryBranches
            | SettingOption::IncludeDirs
            | SettingOption::InlineDirs
            | SettingOption::StatusToastVisibleSecs
            | SettingOption::FinishedTaskVisibleSecs
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
            let next = !app.config.invert_scroll().is_inverted();
            let _ = save_app_setting_with_toast(app, |table| set_invert_scroll(table, next));
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
        Some(SettingOption::StatusToastVisibleSecs) => {
            begin_settings_edit(app, format_status_toast_visible_secs(app));
        },
        Some(SettingOption::FinishedTaskVisibleSecs) => {
            begin_settings_edit(app, format_finished_task_visible_secs(app));
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
            let next = !app.config.include_non_rust().includes_non_rust();
            let _ = save_app_setting_with_toast(app, |table| set_include_non_rust(table, next));
        },
        Some(SettingOption::LintsEnabled) => {
            toggle_lints(app);
        },
        Some(SettingOption::LintOnDiscovery) => {
            let next = !app.config.current().lint.on_discovery.is_immediate();
            let _ = save_app_setting_with_toast(app, |table| set_lint_on_discovery(table, next));
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
    if apply_lint_settings_edit(app, setting, value) {
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
            if !save_u32_setting(app, value, |table, count| {
                set_ci_run_count(table, i64::from(count))
            }) {
                return Ok(true);
            }
        },
        SettingOption::InlineDirs => save_sorted_list_setting(app, value, |table, dirs| {
            write_string_array(table, "tui", "inline_dirs", dirs)
        }),
        SettingOption::IncludeDirs => save_sorted_list_setting(app, value, |table, dirs| {
            write_string_array(table, "tui", "include_dirs", dirs)
        }),
        SettingOption::Editor if !value.trim().is_empty() => {
            save_string_setting(app, value, |table, editor| set_editor(table, &editor));
        },
        SettingOption::TerminalCommand => {
            save_string_setting(app, value, |table, command| {
                set_terminal_command(table, &command)
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
        SettingOption::StatusToastVisibleSecs => {
            if !save_toast_number_setting(
                app,
                value,
                "status_toast_visible",
                |settings, duration| {
                    settings.status_toast_visible = duration;
                },
            ) {
                return Ok(true);
            }
        },
        SettingOption::FinishedTaskVisibleSecs => {
            if !save_toast_number_setting(
                app,
                value,
                "finished_task_visible",
                |settings, duration| {
                    settings.finished_task_visible = duration;
                },
            ) {
                return Ok(true);
            }
        },
        SettingOption::MainBranch => {
            let branch = config::normalize_branch_name(value, "Main branch")?;
            let _ = save_app_setting_with_toast(app, |table| {
                write_value(table, "tui", "main_branch", branch.into())
            });
        },
        SettingOption::OtherPrimaryBranches => {
            let branches =
                config::normalize_branch_list(&parse_dir_list(value), "Other primary branches")?;
            let _ = save_app_setting_with_toast(app, |table| {
                write_string_array(table, "tui", "other_primary_branches", branches)
            });
        },
        SettingOption::DiscoveryShimmerSecs => {
            if !save_number_setting(app, value, |table, secs| {
                set_discovery_shimmer_secs(table, secs)
            }) {
                return Ok(true);
            }
        },
        SettingOption::CpuPollMs => {
            if !save_u32_setting(app, value, |table, poll_ms| {
                set_cpu_poll_ms(table, i64::from(poll_ms))
            }) {
                return Ok(true);
            }
        },
        SettingOption::CpuGreenMaxPercent => {
            if !save_u32_setting(app, value, |table, percent| {
                set_cpu_green_max(table, i64::from(bounded_u8_from_u32(percent.min(100))))
            }) {
                return Ok(true);
            }
        },
        SettingOption::CpuYellowMaxPercent => {
            if !save_u32_setting(app, value, |table, percent| {
                set_cpu_yellow_max(table, i64::from(bounded_u8_from_u32(percent.min(100))))
            }) {
                return Ok(true);
            }
        },
    }
    Ok(true)
}

fn apply_lint_settings_edit(app: &mut App, setting: SettingOption, value: &str) -> bool {
    match setting {
        SettingOption::LintProjects => {
            save_sorted_list_setting(app, value, |table, dirs| {
                write_string_array(table, "lint", "include", dirs)
            });
            if app.overlays.inline_error().is_none() {
                app.show_timed_toast("Settings", "Lint projects updated");
            }
        },
        SettingOption::LintCommands => {
            if save_app_setting(app, |table| set_lint_commands(value, table)) {
                app.show_timed_toast("Settings", "Lint commands updated");
            }
        },
        SettingOption::LintCacheSize => {
            if save_app_setting(app, |table| set_lint_cache_size(table, value)) {
                app.show_timed_toast("Settings", "Lint cache size updated");
            }
        },
        _ => return false,
    }
    true
}

pub(super) fn handle_settings_text_command(app: &mut App, command: SettingsCommand) {
    match command {
        SettingsCommand::None => {},
        SettingsCommand::Save => apply_settings_edit(app),
        SettingsCommand::Cancel => app.framework.settings_pane.enter_browse(),
    }
}

fn toggle_lints(app: &mut App) {
    let enabled = !app.config.current().lint.enabled;
    if !save_app_setting(app, |table| set_lints_enabled(table, enabled)) {
        return;
    }
    app.show_timed_toast(
        "Settings",
        format!("Lints {}", if enabled { "enabled" } else { "disabled" }),
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
                Some(SettingOption::StatusToastVisibleSecs),
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
            Some(SettingOption::StatusToastVisibleSecs)
        );
        assert_eq!(setting_at_selection(&rows, 2), None);
    }

    #[test]
    fn settings_popup_height_is_capped_to_terminal() {
        assert_eq!(settings_popup_height(10, 80), 13);
        assert_eq!(settings_popup_height(100, 20), 18);
    }

    #[test]
    fn settings_scroll_keeps_selected_line_visible() {
        assert_eq!(settings_scroll_offset(0, 5, 20), 0);
        assert_eq!(settings_scroll_offset(7, 5, 20), 3);
        assert_eq!(settings_scroll_offset(19, 5, 20), 15);
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
    fn settings_store_saves_table_settings_and_framework_toasts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let settings_spec = SettingsFileSpec::new(APP_NAME, CONFIG_FILE).with_path(&path);
        let mut loaded =
            SettingsStore::load_for_startup(settings_spec, cargo_port_settings_registry())
                .expect("load settings");
        let mut config = CargoPortConfig::default();
        config.tui.ci_run_count = 9;
        let toast_settings = ToastSettings {
            status_toast_visible: ToastDuration::try_from_secs("status_toast_visible", 3.0)
                .expect("toast duration"),
            ..ToastSettings::default()
        };

        *loaded.store.table_mut() = settings_table_from_config(&config).expect("settings table");
        toast_settings.write_to_table(loaded.store.table_mut());
        loaded.store.save().expect("save settings");

        let saved = std::fs::read_to_string(path).expect("read saved config");
        assert!(saved.contains("ci_run_count = 9"));
        assert!(saved.contains("[toasts]"));
        assert!(saved.contains("status_toast_visible = 3.0"));
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
