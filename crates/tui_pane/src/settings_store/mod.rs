//! Framework settings store, registry, rows, and TOML table helpers.
//!
//! `SettingsStore` owns generic settings persistence for apps that
//! embed `tui_pane`: path resolution, TOML load/save, dirty state, and
//! registered settings metadata. Apps register their own settings through
//! `SettingsRegistry`; framework-owned setting types live with their
//! owning framework module.

mod registry;
mod row;
mod store;
mod table;

pub use registry::AdjustDirection;
pub use registry::SettingAdjuster;
pub use registry::SettingCodecs;
pub use registry::SettingEntry;
pub use registry::SettingKind;
pub use registry::SettingsRegistry;
pub use registry::SettingsSection;
pub use row::SettingValue;
pub use row::SettingsRow;
pub use row::SettingsRowKind;
pub use row::SettingsRowPayload;
pub use store::LoadedSettings;
pub use store::ReloadedSettings;
pub use store::SettingsError;
pub use store::SettingsFileSpec;
pub use store::SettingsStore;
pub use table::read_array;
pub use table::read_bool;
pub use table::read_float;
pub use table::read_int;
pub use table::read_string;
pub use table::write_value;

pub(super) fn invalid(section: &str, key: &str, message: &str) -> SettingsError {
    SettingsError::Invalid {
        section: section.to_string(),
        key:     key.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use toml::Table;
    use toml::Value;

    use super::SettingCodecs;
    use super::SettingKind;
    use super::SettingsFileSpec;
    use super::SettingsRegistry;
    use super::SettingsRow;
    use super::SettingsRowPayload;
    use super::SettingsSection;
    use super::SettingsStore;
    use crate::ToastSettings;

    fn set_enabled(table: &mut Table, value: bool) -> Result<(), super::SettingsError> {
        super::write_value(table, "tui", "enabled", Value::Boolean(value))
    }

    fn enabled(table: &Table) -> bool { super::read_bool(table, "tui", "enabled").unwrap_or(false) }

    fn set_count(table: &mut Table, value: i64) -> Result<(), super::SettingsError> {
        if value < 0 {
            return Err(super::invalid(
                "tui",
                "count",
                "expected non-negative count",
            ));
        }
        super::write_value(table, "tui", "count", Value::Integer(value))
    }

    fn count(table: &Table) -> i64 { super::read_int(table, "tui", "count").unwrap_or_default() }

    fn set_name(table: &mut Table, value: &str) -> Result<(), super::SettingsError> {
        if value.is_empty() {
            return Err(super::invalid("tui", "name", "expected non-empty name"));
        }
        super::write_value(table, "tui", "name", Value::String(value.to_string()))
    }

    fn name(table: &Table) -> String {
        super::read_string(table, "tui", "name")
            .unwrap_or_default()
            .to_string()
    }

    fn items(table: &Table) -> String {
        super::read_array(table, "tui", "items")
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn set_items(value: &str, table: &mut Table) -> Result<(), super::SettingsError> {
        let values = parse_list(value)
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        super::write_value(table, "tui", "items", Value::Array(values))
    }

    fn command(table: &Table) -> String {
        super::read_string(table, "tui", "command")
            .unwrap_or_default()
            .to_string()
    }

    fn set_command(value: &str, table: &mut Table) -> Result<(), super::SettingsError> {
        super::write_value(table, "tui", "command", Value::String(value.to_string()))
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
        let row = SettingsRow::value(SettingsRowPayload::new(7), "Editor", "zed");

        assert_eq!(row.payload.map(SettingsRowPayload::get), Some(7));
    }

    #[test]
    fn empty_registry_has_no_entries() {
        let reg = SettingsRegistry::new();
        assert!(reg.entries().is_empty());
    }

    #[test]
    fn add_settings_record_entries() {
        let reg = SettingsRegistry::new()
            .add_bool_in(SettingsSection::App("tui"), "enabled", enabled, set_enabled)
            .add_int_in(SettingsSection::App("tui"), "count", count, set_count)
            .with_bounds(0, 10)
            .add_string_in(SettingsSection::App("tui"), "name", name, set_name);

        assert_eq!(reg.entries().len(), 3);
        assert_eq!(reg.entries()[0].section, SettingsSection::App("tui"));
        assert_eq!(reg.entries()[0].name, "enabled");
    }

    #[test]
    fn load_for_startup_reads_table_and_toasts() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "tui_pane_settings_{}_{}.toml",
            std::process::id(),
            "startup"
        ));
        std::fs::write(
            &path,
            "[tui]\nenabled = true\ncount = 7\nname = \"hana\"\n\n[toasts]\ndefault_timeout = 9.0\ntask_linger = 2.0\n",
        )
        .expect("write settings");
        let registry = SettingsRegistry::new()
            .add_bool_in(SettingsSection::App("tui"), "enabled", enabled, set_enabled)
            .add_int_in(SettingsSection::App("tui"), "count", count, set_count)
            .add_string_in(SettingsSection::App("tui"), "name", name, set_name);

        let loaded = SettingsStore::load_for_startup(
            SettingsFileSpec::new("test", "settings.toml").with_path(&path),
            registry,
        )
        .expect("load settings");

        assert!(enabled(loaded.store.table()));
        assert_eq!(count(loaded.store.table()), 7);
        assert_eq!(name(loaded.store.table()), "hana");
        assert_eq!(
            loaded.toast_settings.default_timeout.get(),
            Duration::from_secs(9)
        );
        assert_eq!(
            loaded.toast_settings.task_linger.get(),
            Duration::from_secs(2)
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_from_path_retargets_store_path() {
        let dir = std::env::temp_dir();
        let initial_path = dir.join(format!(
            "tui_pane_settings_{}_{}.toml",
            std::process::id(),
            "initial"
        ));
        let reload_path = dir.join(format!(
            "tui_pane_settings_{}_{}.toml",
            std::process::id(),
            "reload"
        ));
        std::fs::write(&initial_path, "[tui]\nname = \"initial\"\n").expect("write initial");
        std::fs::write(
            &reload_path,
            "[tui]\nname = \"reload\"\n\n[toasts]\ndefault_timeout = 6.0\n",
        )
        .expect("write reload");
        let registry = SettingsRegistry::new().add_string_in(
            SettingsSection::App("tui"),
            "name",
            name,
            set_name,
        );
        let mut loaded = SettingsStore::load_for_startup(
            SettingsFileSpec::new("test", "settings.toml").with_path(&initial_path),
            registry,
        )
        .expect("load settings");

        let reloaded = loaded
            .store
            .load_from_path(&reload_path)
            .expect("reload settings");

        assert_eq!(name(loaded.store.table()), "reload");
        assert_eq!(
            reloaded.toast_settings.default_timeout.get(),
            Duration::from_secs(6)
        );
        assert_eq!(loaded.store.path(), Some(reload_path.as_path()));
        let _ = std::fs::remove_file(initial_path);
        let _ = std::fs::remove_file(reload_path);
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

        let items_entry = &registry.entries()[0];
        let command_entry = &registry.entries()[1];
        let SettingKind::Custom {
            codecs: items_codecs,
        } = &items_entry.kind
        else {
            panic!("expected custom items entry");
        };
        let SettingKind::Custom {
            codecs: command_codecs,
        } = &command_entry.kind
        else {
            panic!("expected custom command entry");
        };

        (items_codecs.parse)("alpha, beta", &mut table).expect("set items");
        (command_codecs.parse)("cargo mend", &mut table).expect("set command");

        assert_eq!((items_codecs.format)(&table), "alpha, beta");
        assert_eq!((command_codecs.format)(&table), "cargo mend");
    }

    #[test]
    fn legacy_tui_toast_keys_seed_toast_settings() {
        let table: Table = "[tui]\nstatus_flash_secs = 4.0\ntask_linger_secs = 3.0\n"
            .parse()
            .expect("parse toml");
        let settings = ToastSettings::from_table(&table).expect("toast settings");

        assert_eq!(settings.default_timeout.get(), Duration::from_secs(4));
        assert_eq!(settings.task_linger.get(), Duration::from_secs(3));
    }

    #[test]
    fn save_writes_toasts_and_removes_legacy_keys() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "tui_pane_settings_{}_{}.toml",
            std::process::id(),
            "save"
        ));
        let registry = SettingsRegistry::new().add_bool_in(
            SettingsSection::App("tui"),
            "enabled",
            enabled,
            set_enabled,
        );
        let mut loaded = SettingsStore::load_for_startup(
            SettingsFileSpec::new("test", "settings.toml").with_path(&path),
            registry,
        )
        .expect("load settings");

        set_enabled(loaded.store.table_mut(), true).expect("set enabled");
        ToastSettings::default().write_to_table(loaded.store.table_mut());
        loaded.store.save().expect("save settings");
        let saved = std::fs::read_to_string(&path).expect("read saved settings");

        assert!(saved.contains("enabled = true"));
        assert!(saved.contains("[toasts]"));
        assert!(!saved.contains("status_flash_secs"));
        assert!(!saved.contains("task_linger_secs"));
        let _ = std::fs::remove_file(path);
    }
}
