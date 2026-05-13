use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;
use toml::Table;

use super::SettingsRegistry;
use crate::toasts;
use crate::toasts::ToastSettings;

/// Error returned by settings load, validation, and save operations.
#[derive(Debug, Error)]
pub enum SettingsError {
    /// Reading or writing the settings file failed.
    #[error("settings I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// Parsing TOML failed.
    #[error("settings TOML parse failed: {0}")]
    Parse(#[from] toml::de::Error),
    /// Serializing TOML failed.
    #[error("settings TOML serialize failed: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// A setting value failed validation.
    #[error("invalid setting {section}.{key}: {message}")]
    Invalid {
        /// TOML table name.
        section: String,
        /// TOML key name.
        key:     String,
        /// Human-readable validation message.
        message: String,
    },
}

/// Settings file identity and optional explicit path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsFileSpec {
    /// Application id used under the OS config directory.
    pub app_id:    &'static str,
    /// Settings file name inside the application config directory.
    pub file_name: &'static str,
    /// Explicit path override for tests or custom launchers.
    pub path:      Option<PathBuf>,
}

impl SettingsFileSpec {
    /// Build a settings-file spec.
    #[must_use]
    pub const fn new(app_id: &'static str, file_name: &'static str) -> Self {
        Self {
            app_id,
            file_name,
            path: None,
        }
    }

    /// Build a settings-file spec pinned to `path`.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Resolve the file path.
    #[must_use]
    pub fn resolved_path(&self) -> Option<PathBuf> {
        if let Some(path) = &self.path {
            return Some(path.clone());
        }
        dirs::config_dir().map(|dir| dir.join(self.app_id).join(self.file_name))
    }
}

impl Default for SettingsFileSpec {
    fn default() -> Self { Self::new("tui_pane", "settings.toml") }
}

/// Framework settings store loaded before app construction.
pub struct SettingsStore {
    spec:     SettingsFileSpec,
    path:     Option<PathBuf>,
    registry: SettingsRegistry,
    table:    Table,
    dirty:    bool,
}

/// Settings produced by [`SettingsStore::load_for_startup`].
pub struct LoadedSettings {
    /// Store installed into [`Framework`](crate::Framework).
    pub store:          SettingsStore,
    /// Framework-owned toast settings.
    pub toast_settings: ToastSettings,
}

/// Settings reloaded from an existing [`SettingsStore`].
pub struct ReloadedSettings {
    /// Framework-owned toast settings loaded from disk.
    pub toast_settings: ToastSettings,
}

impl SettingsStore {
    /// Load settings and return the startup handoff.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] when the settings file cannot be read
    /// or parsed, or when a registered setting fails validation.
    pub fn load_for_startup(
        spec: SettingsFileSpec,
        registry: SettingsRegistry,
    ) -> Result<LoadedSettings, SettingsError> {
        let path = spec.resolved_path();
        let table = read_settings_table(path.as_deref())?;
        let toast_settings = ToastSettings::from_table(&table)?;
        let store = Self {
            spec,
            path,
            registry,
            table,
            dirty: false,
        };
        Ok(LoadedSettings {
            store,
            toast_settings,
        })
    }

    /// Empty settings store for frameworks constructed before a
    /// settings file is installed.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            spec:     SettingsFileSpec::default(),
            path:     None,
            registry: SettingsRegistry::new(),
            table:    Table::new(),
            dirty:    false,
        }
    }

    /// Borrow the settings file spec.
    #[must_use]
    pub const fn spec(&self) -> &SettingsFileSpec { &self.spec }

    /// Borrow the resolved path, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> { self.path.as_deref() }

    /// Borrow registered app settings.
    #[must_use]
    pub const fn registry(&self) -> &SettingsRegistry { &self.registry }

    /// Borrow the in-memory settings TOML table.
    #[must_use]
    pub const fn table(&self) -> &Table { &self.table }

    /// Mutably borrow the in-memory settings TOML table and mark it dirty.
    pub const fn table_mut(&mut self) -> &mut Table {
        self.dirty = true;
        &mut self.table
    }

    /// Replace the in-memory settings TOML table.
    pub fn replace_table(&mut self, table: Table) {
        self.table = table;
        self.dirty = true;
    }

    /// Reload app and framework settings from the store's configured path.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] when the file cannot be read or
    /// parsed, or when a registered setting fails validation.
    pub fn load_current(&mut self) -> Result<ReloadedSettings, SettingsError> {
        let table = read_settings_table(self.path.as_deref())?;
        let toast_settings = ToastSettings::from_table(&table)?;
        self.table = table;
        self.dirty = false;
        Ok(ReloadedSettings { toast_settings })
    }

    /// Reload app and framework settings from a specific file path and
    /// make that path the store's current save/reload target.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] when the file cannot be read or
    /// parsed, or when a registered setting fails validation.
    pub fn load_from_path(
        &mut self,
        path: impl Into<PathBuf>,
    ) -> Result<ReloadedSettings, SettingsError> {
        let path = path.into();
        let table = read_settings_table(Some(path.as_path()))?;
        let toast_settings = ToastSettings::from_table(&table)?;
        self.spec.path = Some(path.clone());
        self.path = Some(path);
        self.table = table;
        self.dirty = false;
        Ok(ReloadedSettings { toast_settings })
    }

    /// Whether settings have unsaved in-memory changes.
    #[must_use]
    pub const fn is_dirty(&self) -> bool { self.dirty }

    /// Mark the store dirty after a framework-owned setting changes.
    pub const fn mark_dirty(&mut self) { self.dirty = true; }

    /// Save the in-memory settings TOML table to disk.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] if serialization or writing fails.
    pub fn save(&mut self) -> Result<(), SettingsError> {
        toasts::remove_legacy_toast_keys(&mut self.table);
        let Some(path) = &self.path else {
            self.dirty = false;
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(&self.table)?;
        fs::write(path, text)?;
        self.dirty = false;
        Ok(())
    }
}

impl Default for SettingsStore {
    fn default() -> Self { Self::empty() }
}

fn read_settings_table(path: Option<&Path>) -> Result<Table, SettingsError> {
    let Some(path) = path else {
        return Ok(Table::new());
    };
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Table::new()),
        Err(err) => return Err(SettingsError::Io(err)),
    };
    let table = toml::from_str::<Table>(&text)?;
    Ok(table)
}
