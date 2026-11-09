use super::SettingsError;

pub(crate) fn invalid(section: &str, key: &str, message: &str) -> SettingsError {
    SettingsError::Invalid {
        section: section.to_string(),
        key:     key.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use tempfile::TempDir;
    use toml::Table;
    use toml::Value;

    use crate::ToastSettings;
    use crate::settings_store::SettingCodecs;
    use crate::settings_store::SettingsFileSpec;
    use crate::settings_store::SettingsRegistry;
    use crate::settings_store::SettingsRow;
    use crate::settings_store::SettingsRowIdentity;
    use crate::settings_store::SettingsRowPayload;
    use crate::settings_store::SettingsSection;
    use crate::settings_store::SettingsStore;
    use crate::settings_store::registry::SettingKind;

    fn set_enabled(table: &mut Table, value: bool) -> Result<(), super::super::SettingsError> {
        super::super::write_value(table, "tui", "enabled", Value::Boolean(value))
    }

    fn enabled(table: &Table) -> bool {
        super::super::read_bool(table, "tui", "enabled").unwrap_or(false)
    }

    fn set_count(table: &mut Table, value: i64) -> Result<(), super::super::SettingsError> {
        if value < 0 {
            return Err(super::super::invalid(
                "tui",
                "count",
                "expected non-negative count",
            ));
        }
        super::super::write_value(table, "tui", "count", Value::Integer(value))
    }

    fn count(table: &Table) -> i64 {
        super::super::read_int(table, "tui", "count").unwrap_or_default()
    }

    fn set_name(table: &mut Table, value: &str) -> Result<(), super::super::SettingsError> {
        if value.is_empty() {
            return Err(super::super::invalid(
                "tui",
                "name",
                "expected non-empty name",
            ));
        }
        super::super::write_value(table, "tui", "name", Value::String(value.to_string()))
    }

    fn name(table: &Table) -> String {
        super::super::read_string(table, "tui", "name")
            .unwrap_or_default()
            .to_string()
    }

    fn items(table: &Table) -> String {
        super::super::read_array(table, "tui", "items")
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn set_items(value: &str, table: &mut Table) -> Result<(), super::super::SettingsError> {
        let toml_values = parse_list(value)
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        super::super::write_value(table, "tui", "items", Value::Array(toml_values))
    }

    fn command(table: &Table) -> String {
        super::super::read_string(table, "tui", "command")
            .unwrap_or_default()
            .to_string()
    }

    fn set_command(value: &str, table: &mut Table) -> Result<(), super::super::SettingsError> {
        super::super::write_value(table, "tui", "command", Value::String(value.to_string()))
    }

    fn parse_list(value: &str) -> Vec<String> {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn settings_row_payload_round_trips_typed_id() {
        let settings_row = SettingsRow::value(SettingsRowPayload::new(7), "Editor", "zed");

        assert_eq!(
            settings_row.identity,
            SettingsRowIdentity::Selectable(SettingsRowPayload::new(7))
        );
    }

    #[test]
    fn empty_registry_has_no_entries() {
        let settings_registry = SettingsRegistry::new();
        assert!(settings_registry.entries().is_empty());
    }

    #[test]
    fn add_settings_record_entries() {
        let settings_registry = SettingsRegistry::new()
            .add_bool_in(SettingsSection::App("tui"), "enabled", enabled, set_enabled)
            .add_int_in(SettingsSection::App("tui"), "count", count, set_count)
            .with_bounds(0, 10)
            .add_string_in(SettingsSection::App("tui"), "name", name, set_name);

        assert_eq!(settings_registry.entries().len(), 3);
        assert_eq!(
            settings_registry.entries()[0].section,
            SettingsSection::App("tui")
        );
        assert_eq!(settings_registry.entries()[0].name, "enabled");
    }

    #[test]
    fn load_for_startup_reads_table_and_toasts() {
        let temp_dir = TempDir::new().expect("create temp settings dir");
        let path_buf = temp_dir.path().join("startup.toml");
        std::fs::write(
            &path_buf,
            "[tui]\nenabled = true\ncount = 7\nname = \"hana\"\n\n[toasts]\ndefault_timeout = 9.0\ntask_linger = 2.0\n",
        )
        .expect("write settings");
        let registry = SettingsRegistry::new()
            .add_bool_in(SettingsSection::App("tui"), "enabled", enabled, set_enabled)
            .add_int_in(SettingsSection::App("tui"), "count", count, set_count)
            .add_string_in(SettingsSection::App("tui"), "name", name, set_name);

        let loaded_settings = SettingsStore::load_for_startup(
            SettingsFileSpec::new("test", "settings.toml").with_path(&path_buf),
            registry,
        )
        .expect("load settings");

        assert!(enabled(loaded_settings.store.table()));
        assert_eq!(count(loaded_settings.store.table()), 7);
        assert_eq!(name(loaded_settings.store.table()), "hana");
        assert_eq!(
            loaded_settings.toast_settings.status_toast_visible.get(),
            Duration::from_secs(9)
        );
        assert_eq!(
            loaded_settings.toast_settings.finished_task_visible.get(),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn load_from_path_retargets_store_path() {
        let temp_dir = TempDir::new().expect("create temp settings dir");
        let initial_path_buf = temp_dir.path().join("initial.toml");
        let reload_path_buf = temp_dir.path().join("reload.toml");
        std::fs::write(&initial_path_buf, "[tui]\nname = \"initial\"\n").expect("write initial");
        std::fs::write(
            &reload_path_buf,
            "[tui]\nname = \"reload\"\n\n[toasts]\ndefault_timeout = 6.0\n",
        )
        .expect("write reload");
        let registry = SettingsRegistry::new().add_string_in(
            SettingsSection::App("tui"),
            "name",
            name,
            set_name,
        );
        let mut loaded_settings = SettingsStore::load_for_startup(
            SettingsFileSpec::new("test", "settings.toml").with_path(&initial_path_buf),
            registry,
        )
        .expect("load settings");

        let reloaded_settings = loaded_settings
            .store
            .load_from_path(&reload_path_buf)
            .expect("reload settings");

        assert_eq!(name(loaded_settings.store.table()), "reload");
        assert_eq!(
            reloaded_settings.toast_settings.status_toast_visible.get(),
            Duration::from_secs(6)
        );
        assert_eq!(
            loaded_settings.store.path(),
            Some(reload_path_buf.as_path())
        );
    }

    #[test]
    fn custom_codecs_mutate_table_values() {
        let mut table = Table::new();
        let registry = SettingsRegistry::new()
            .add_custom_in(
                SettingsSection::App("tui"),
                "items",
                SettingCodecs {
                    format: items,
                    parse:  set_items,
                    adjust: None,
                },
            )
            .add_custom_in(
                SettingsSection::App("tui"),
                "commands",
                SettingCodecs {
                    format: command,
                    parse:  set_command,
                    adjust: None,
                },
            );

        let items_setting_entry = &registry.entries()[0];
        let command_setting_entry = &registry.entries()[1];
        let SettingKind::Custom {
            codecs: items_setting_codecs,
        } = &items_setting_entry.kind
        else {
            panic!("expected custom items entry");
        };
        let SettingKind::Custom {
            codecs: command_setting_codecs,
        } = &command_setting_entry.kind
        else {
            panic!("expected custom command entry");
        };

        (items_setting_codecs.parse)("alpha, beta", &mut table).expect("set items");
        (command_setting_codecs.parse)("cargo mend", &mut table).expect("set command");

        assert_eq!((items_setting_codecs.format)(&table), "alpha, beta");
        assert_eq!((command_setting_codecs.format)(&table), "cargo mend");
    }

    #[test]
    fn legacy_tui_toast_keys_seed_toast_settings() {
        let table: Table = "[tui]\nstatus_flash_secs = 4.0\ntask_linger_secs = 3.0\n"
            .parse()
            .expect("parse toml");
        let settings = ToastSettings::from_table(&table).expect("toast settings");

        assert_eq!(settings.status_toast_visible.get(), Duration::from_secs(4));
        assert_eq!(settings.finished_task_visible.get(), Duration::from_secs(3));
    }

    #[test]
    fn legacy_toast_keys_default_timeout_and_task_linger_migrate() {
        let table: Table = "[toasts]\ndefault_timeout = 9.0\ntask_linger = 2.0\n"
            .parse()
            .expect("parse toml");
        let settings = ToastSettings::from_table(&table).expect("toast settings");

        assert_eq!(settings.status_toast_visible.get(), Duration::from_secs(9));
        assert_eq!(settings.finished_task_visible.get(), Duration::from_secs(2));
    }

    #[test]
    fn new_toast_keys_take_precedence_over_legacy_keys() {
        let table: Table = "[toasts]\nstatus_toast_visible = 7.0\ndefault_timeout = 99.0\n\
             finished_task_visible = 4.0\ntask_linger = 88.0\n"
            .parse()
            .expect("parse toml");
        let settings = ToastSettings::from_table(&table).expect("toast settings");

        assert_eq!(settings.status_toast_visible.get(), Duration::from_secs(7));
        assert_eq!(settings.finished_task_visible.get(), Duration::from_secs(4));
    }

    #[test]
    fn save_removes_legacy_default_timeout_and_task_linger_keys() {
        let temp_dir = TempDir::new().expect("create temp settings dir");
        let path_buf = temp_dir.path().join("migrate.toml");
        // `std::fs::write` seeds the settings file's `[toasts]` table with the
        // legacy keys `default_timeout` and `task_linger`.
        std::fs::write(
            &path_buf,
            "[toasts]\ndefault_timeout = 9.0\ntask_linger = 2.0\n",
        )
        .expect("write legacy");
        let registry = SettingsRegistry::new();
        let mut loaded_settings = SettingsStore::load_for_startup(
            SettingsFileSpec::new("test", "settings.toml").with_path(&path_buf),
            registry,
        )
        .expect("load settings");

        // `ToastSettings::write_to_table` and `SettingsStore::save` re-save the file;
        // confirm legacy keys are gone and new keys are written in their place.
        loaded_settings
            .toast_settings
            .write_to_table(loaded_settings.store.table_mut());
        loaded_settings.store.save().expect("save settings");
        let saved = std::fs::read_to_string(&path_buf).expect("read saved");

        assert!(!saved.contains("default_timeout"));
        assert!(!saved.contains("task_linger"));
        assert!(saved.contains("status_toast_visible"));
        assert!(saved.contains("finished_task_visible"));
    }

    #[test]
    fn save_writes_toasts_and_removes_legacy_keys() {
        let temp_dir = TempDir::new().expect("create temp settings dir");
        let path_buf = temp_dir.path().join("save.toml");
        std::fs::write(
            &path_buf,
            "[tui]\nstatus_flash_secs = 4.0\ntask_linger_secs = 3.0\n",
        )
        .expect("write legacy settings");
        let registry = SettingsRegistry::new().add_bool_in(
            SettingsSection::App("tui"),
            "enabled",
            enabled,
            set_enabled,
        );
        let mut loaded_settings = SettingsStore::load_for_startup(
            SettingsFileSpec::new("test", "settings.toml").with_path(&path_buf),
            registry,
        )
        .expect("load settings");

        set_enabled(loaded_settings.store.table_mut(), true).expect("set enabled");
        ToastSettings::default().write_to_table(loaded_settings.store.table_mut());
        loaded_settings.store.save().expect("save settings");
        let saved = std::fs::read_to_string(&path_buf).expect("read saved settings");

        assert!(saved.contains("enabled = true"));
        assert!(saved.contains("[toasts]"));
        assert!(!saved.contains("status_flash_secs"));
        assert!(!saved.contains("task_linger_secs"));
    }
}
