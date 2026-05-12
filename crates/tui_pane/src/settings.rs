//! Framework settings registry, file store, and framework-owned setting groups.
//!
//! `SettingsStore` owns generic settings persistence for apps that
//! embed `tui_pane`: path resolution, TOML load/save, dirty state, and
//! registered settings metadata. Apps register their own settings through
//! `SettingsRegistry`; framework-owned settings, such as
//! `ToastSettings`, live directly on `Framework<Ctx>`.

use core::num::NonZeroU16;
use core::num::NonZeroUsize;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;
use toml::Table;
use toml::Value;

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

/// App-owned or framework-owned settings section.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsSection {
    /// App-owned section, such as `"tui"` or `"lint"`.
    App(&'static str),
    /// Framework-owned section, such as `"toasts"`.
    Framework(&'static str),
}

/// Direction for setting adjustment controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdjustDirection {
    /// Move to the previous value.
    Back,
    /// Move to the next value.
    Forward,
}

/// Common setting values used by built-in `SettingsPane` widgets.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    /// Boolean setting.
    Bool(bool),
    /// Integer setting.
    Int(i64),
    /// Floating-point setting.
    Float(f64),
    /// String setting.
    String(String),
    /// Closed-set enum setting.
    Enum(String),
}

/// One renderable row in a framework-owned settings pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsRow {
    /// Row label. Section rows use this as the section title.
    pub label:   String,
    /// Displayed value for selectable setting rows.
    pub value:   String,
    /// Row behavior.
    pub kind:    SettingsRowKind,
    /// Optional app-provided suffix shown after compact controls.
    pub suffix:  Option<String>,
    /// Optional stable app payload for hit testing / dispatch.
    pub payload: Option<usize>,
}

impl SettingsRow {
    /// Build a section header row.
    #[must_use]
    pub fn section(label: impl Into<String>) -> Self {
        Self {
            label:   label.into(),
            value:   String::new(),
            kind:    SettingsRowKind::Section,
            suffix:  None,
            payload: None,
        }
    }

    /// Build a selectable value row.
    #[must_use]
    pub fn value(payload: usize, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label:   label.into(),
            value:   value.into(),
            kind:    SettingsRowKind::Value,
            suffix:  None,
            payload: Some(payload),
        }
    }

    /// Build a selectable toggle row.
    #[must_use]
    pub fn toggle(payload: usize, label: impl Into<String>, enabled: bool) -> Self {
        Self {
            label:   label.into(),
            value:   if enabled { "ON" } else { "OFF" }.to_string(),
            kind:    SettingsRowKind::Toggle,
            suffix:  None,
            payload: Some(payload),
        }
    }

    /// Build a selectable stepper row.
    #[must_use]
    pub fn stepper(payload: usize, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label:   label.into(),
            value:   value.into(),
            kind:    SettingsRowKind::Stepper,
            suffix:  None,
            payload: Some(payload),
        }
    }

    /// Attach a suffix to a row.
    #[must_use]
    pub fn with_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }
}

/// Render behavior for one [`SettingsRow`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsRowKind {
    /// Non-selectable section header.
    Section,
    /// Normal editable value.
    Value,
    /// Boolean-style row rendered as `< ON >` / `< OFF >`.
    Toggle,
    /// Direction-adjustable row rendered as `< value >`.
    Stepper,
}

/// Formatting and parsing callbacks for app-specific settings.
pub struct SettingCodecs {
    /// Format the setting for display/editing.
    pub format: fn(&Table) -> String,
    /// Parse an edited string into the backing store.
    pub parse:  fn(&str, &mut Table) -> Result<(), SettingsError>,
    /// Optional direction-aware adjustment.
    pub adjust: Option<SettingAdjuster>,
}

/// Direction-aware adjustment callback for a setting.
pub type SettingAdjuster = fn(AdjustDirection, &mut Table) -> Result<(), SettingsError>;

/// One declared setting kept on a [`SettingsRegistry`].
pub enum SettingKind {
    /// A `bool`-typed setting.
    Bool {
        /// Read the current value.
        get: fn(&Table) -> bool,
        /// Write a new value.
        set: fn(&mut Table, bool) -> Result<(), SettingsError>,
    },
    /// A closed-set enum-typed setting.
    Enum {
        /// Read the current label.
        get:      fn(&Table) -> String,
        /// Write a new label.
        set:      fn(&mut Table, &str) -> Result<(), SettingsError>,
        /// The closed set of valid labels.
        variants: &'static [&'static str],
    },
    /// An integer-typed setting.
    Int {
        /// Read the current value.
        get:    fn(&Table) -> i64,
        /// Write a new value.
        set:    fn(&mut Table, i64) -> Result<(), SettingsError>,
        /// Inclusive `(min, max)` bounds, or `None` for unbounded.
        bounds: Option<(i64, i64)>,
    },
    /// A floating-point setting.
    Float {
        /// Read the current value.
        get: fn(&Table) -> f64,
        /// Write a new value.
        set: fn(&mut Table, f64) -> Result<(), SettingsError>,
    },
    /// A string setting.
    String {
        /// Read the current value.
        get: fn(&Table) -> String,
        /// Write a new value.
        set: fn(&mut Table, &str) -> Result<(), SettingsError>,
    },
    /// Custom app-defined parse/format/adjust behavior.
    Custom {
        /// App-provided codecs.
        codecs: SettingCodecs,
    },
}

/// One entry in a [`SettingsRegistry`].
pub struct SettingEntry {
    /// Settings section.
    pub section: SettingsSection,
    /// Stable TOML key name.
    pub name:    &'static str,
    /// Type and accessors for this setting.
    pub kind:    SettingKind,
}

/// Declarative settings registry, one per app.
pub struct SettingsRegistry {
    entries: Vec<SettingEntry>,
}

impl SettingsRegistry {
    /// Empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a boolean setting to the default app section.
    #[must_use]
    pub fn add_bool(
        self,
        name: &'static str,
        get: fn(&Table) -> bool,
        set: fn(&mut Table, bool) -> Result<(), SettingsError>,
    ) -> Self {
        self.add_bool_in(SettingsSection::App("app"), name, get, set)
    }

    /// Add a boolean setting to `section`.
    #[must_use]
    pub fn add_bool_in(
        mut self,
        section: SettingsSection,
        name: &'static str,
        get: fn(&Table) -> bool,
        set: fn(&mut Table, bool) -> Result<(), SettingsError>,
    ) -> Self {
        self.entries.push(SettingEntry {
            section,
            name,
            kind: SettingKind::Bool { get, set },
        });
        self
    }

    /// Add a closed-set enum setting to the default app section.
    #[must_use]
    pub fn add_enum(
        self,
        name: &'static str,
        get: fn(&Table) -> String,
        set: fn(&mut Table, &str) -> Result<(), SettingsError>,
        variants: &'static [&'static str],
    ) -> Self {
        self.add_enum_in(SettingsSection::App("app"), name, get, set, variants)
    }

    /// Add a closed-set enum setting to `section`.
    #[must_use]
    pub fn add_enum_in(
        mut self,
        section: SettingsSection,
        name: &'static str,
        get: fn(&Table) -> String,
        set: fn(&mut Table, &str) -> Result<(), SettingsError>,
        variants: &'static [&'static str],
    ) -> Self {
        self.entries.push(SettingEntry {
            section,
            name,
            kind: SettingKind::Enum { get, set, variants },
        });
        self
    }

    /// Add an integer setting to the default app section.
    #[must_use]
    pub fn add_int(
        self,
        name: &'static str,
        get: fn(&Table) -> i64,
        set: fn(&mut Table, i64) -> Result<(), SettingsError>,
    ) -> Self {
        self.add_int_in(SettingsSection::App("app"), name, get, set)
    }

    /// Add an integer setting to `section`.
    #[must_use]
    pub fn add_int_in(
        mut self,
        section: SettingsSection,
        name: &'static str,
        get: fn(&Table) -> i64,
        set: fn(&mut Table, i64) -> Result<(), SettingsError>,
    ) -> Self {
        self.entries.push(SettingEntry {
            section,
            name,
            kind: SettingKind::Int {
                get,
                set,
                bounds: None,
            },
        });
        self
    }

    /// Add a floating-point setting to `section`.
    #[must_use]
    pub fn add_float_in(
        mut self,
        section: SettingsSection,
        name: &'static str,
        get: fn(&Table) -> f64,
        set: fn(&mut Table, f64) -> Result<(), SettingsError>,
    ) -> Self {
        self.entries.push(SettingEntry {
            section,
            name,
            kind: SettingKind::Float { get, set },
        });
        self
    }

    /// Add a string setting to `section`.
    #[must_use]
    pub fn add_string_in(
        mut self,
        section: SettingsSection,
        name: &'static str,
        get: fn(&Table) -> String,
        set: fn(&mut Table, &str) -> Result<(), SettingsError>,
    ) -> Self {
        self.entries.push(SettingEntry {
            section,
            name,
            kind: SettingKind::String { get, set },
        });
        self
    }

    /// Add a custom app setting to `section`.
    #[must_use]
    pub fn add_custom_in(
        mut self,
        section: SettingsSection,
        name: &'static str,
        codecs: SettingCodecs,
    ) -> Self {
        self.entries.push(SettingEntry {
            section,
            name,
            kind: SettingKind::Custom { codecs },
        });
        self
    }

    /// Set inclusive `(min, max)` bounds on the most recently added integer setting.
    #[must_use]
    pub fn with_bounds(mut self, min: i64, max: i64) -> Self {
        if let Some(SettingEntry {
            kind: SettingKind::Int { bounds, .. },
            ..
        }) = self.entries.last_mut()
        {
            *bounds = Some((min, max));
        }
        self
    }

    /// Borrow all entries in declaration order.
    #[must_use]
    pub fn entries(&self) -> &[SettingEntry] { &self.entries }
}

impl Default for SettingsRegistry {
    fn default() -> Self { Self::new() }
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
        remove_legacy_toast_keys(&mut self.table);
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

/// Read an integer value from `section.key`.
#[must_use]
pub fn read_int(table: &Table, section: &str, key: &str) -> Option<i64> {
    table
        .get(section)
        .and_then(Value::as_table)
        .and_then(|section| section.get(key))
        .and_then(Value::as_integer)
}

/// Read a floating-point value from `section.key`.
#[must_use]
pub fn read_float(table: &Table, section: &str, key: &str) -> Option<f64> {
    table
        .get(section)
        .and_then(Value::as_table)
        .and_then(|section| section.get(key))
        .and_then(|value| {
            value.as_float().or_else(|| {
                value
                    .as_integer()
                    .and_then(|integer| integer.to_string().parse().ok())
            })
        })
}

/// Read a boolean value from `section.key`.
#[must_use]
pub fn read_bool(table: &Table, section: &str, key: &str) -> Option<bool> {
    table
        .get(section)
        .and_then(Value::as_table)
        .and_then(|section| section.get(key))
        .and_then(Value::as_bool)
}

/// Read a string value from `section.key`.
#[must_use]
pub fn read_string<'a>(table: &'a Table, section: &str, key: &str) -> Option<&'a str> {
    table
        .get(section)
        .and_then(Value::as_table)
        .and_then(|section| section.get(key))
        .and_then(Value::as_str)
}

/// Read an array value from `section.key`.
#[must_use]
pub fn read_array<'a>(table: &'a Table, section: &str, key: &str) -> Option<&'a [Value]> {
    table
        .get(section)
        .and_then(Value::as_table)
        .and_then(|section| section.get(key))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
}

/// Write a TOML value to `section.key`, creating `section` when needed.
///
/// # Errors
///
/// Returns [`SettingsError`] if `section` exists but is not a TOML table.
pub fn write_value(
    table: &mut Table,
    section: &str,
    key: &str,
    value: Value,
) -> Result<(), SettingsError> {
    let section_value = table
        .entry(section.to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    let Value::Table(section_table) = section_value else {
        return Err(invalid(section, key, "section is not a table"));
    };
    section_table.insert(key.to_string(), value);
    Ok(())
}

fn invalid(section: &str, key: &str, message: &str) -> SettingsError {
    SettingsError::Invalid {
        section: section.to_string(),
        key:     key.to_string(),
        message: message.to_string(),
    }
}

/// Framework-owned toast settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastSettings {
    /// Whether toast rendering is enabled.
    pub enabled:         bool,
    /// Toast card width.
    pub width:           ToastWidth,
    /// Gap between visible toast cards.
    pub gap:             ToastGap,
    /// Default timeout for timed toasts.
    pub default_timeout: ToastDuration,
    /// Linger for completed task toasts.
    pub task_linger:     ToastDuration,
    /// Maximum number of visible toasts.
    pub max_visible:     MaxVisibleToasts,
    /// Toast placement.
    pub placement:       ToastPlacement,
    /// Toast animation timing.
    pub animation:       ToastAnimationSettings,
}

impl ToastSettings {
    /// Load toast settings from a TOML table.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] when a value fails validation.
    pub fn from_table(table: &Table) -> Result<Self, SettingsError> {
        let mut settings = Self::default();
        if let Some(toasts) = table.get("toasts").and_then(Value::as_table) {
            settings.apply_toasts_table(toasts)?;
        } else if let Some(tui) = table.get("tui").and_then(Value::as_table) {
            settings.apply_legacy_tui_table(tui)?;
        }
        Ok(settings)
    }

    fn apply_toasts_table(&mut self, table: &Table) -> Result<(), SettingsError> {
        if let Some(value) = table.get("enabled") {
            self.enabled = value
                .as_bool()
                .ok_or_else(|| invalid("toasts", "enabled", "expected bool"))?;
        }
        if let Some(value) = table.get("width") {
            self.width = ToastWidth::try_from_i64(
                value
                    .as_integer()
                    .ok_or_else(|| invalid("toasts", "width", "expected integer"))?,
            )?;
        }
        if let Some(value) = table.get("gap") {
            self.gap = ToastGap::try_from_i64(
                value
                    .as_integer()
                    .ok_or_else(|| invalid("toasts", "gap", "expected integer"))?,
            )?;
        }
        if let Some(value) = table.get("default_timeout") {
            self.default_timeout = ToastDuration::try_from_value("default_timeout", value)?;
        }
        if let Some(value) = table.get("task_linger") {
            self.task_linger = ToastDuration::try_from_value("task_linger", value)?;
        }
        if let Some(value) = table.get("max_visible") {
            self.max_visible = MaxVisibleToasts::try_from_i64(
                value
                    .as_integer()
                    .ok_or_else(|| invalid("toasts", "max_visible", "expected integer"))?,
            )?;
        }
        if let Some(value) = table.get("placement") {
            self.placement = ToastPlacement::parse(
                value
                    .as_str()
                    .ok_or_else(|| invalid("toasts", "placement", "expected string"))?,
            )?;
        }
        Ok(())
    }

    fn apply_legacy_tui_table(&mut self, table: &Table) -> Result<(), SettingsError> {
        if let Some(value) = table.get("status_flash_secs") {
            self.default_timeout = ToastDuration::try_from_value("status_flash_secs", value)?;
        }
        if let Some(value) = table.get("task_linger_secs") {
            self.task_linger = ToastDuration::try_from_value("task_linger_secs", value)?;
        }
        Ok(())
    }

    /// Write toast settings into the shared settings TOML table.
    pub fn write_to_table(&self, table: &mut Table) {
        let mut toasts = Table::new();
        toasts.insert("enabled".to_string(), Value::Boolean(self.enabled));
        toasts.insert(
            "width".to_string(),
            Value::Integer(i64::from(self.width.get())),
        );
        toasts.insert("gap".to_string(), Value::Integer(i64::from(self.gap.get())));
        toasts.insert(
            "default_timeout".to_string(),
            Value::Float(self.default_timeout.as_secs_f64()),
        );
        toasts.insert(
            "task_linger".to_string(),
            Value::Float(self.task_linger.as_secs_f64()),
        );
        toasts.insert(
            "max_visible".to_string(),
            Value::Integer(i64::try_from(self.max_visible.get()).unwrap_or(i64::MAX)),
        );
        toasts.insert(
            "placement".to_string(),
            Value::String(self.placement.as_str().to_string()),
        );
        table.insert("toasts".to_string(), Value::Table(toasts));
    }
}

impl Default for ToastSettings {
    fn default() -> Self {
        Self {
            enabled:         true,
            width:           ToastWidth::default(),
            gap:             ToastGap::default(),
            default_timeout: ToastDuration::DEFAULT_TIMEOUT,
            task_linger:     ToastDuration::TASK_LINGER,
            max_visible:     MaxVisibleToasts::default(),
            placement:       ToastPlacement::BottomRight,
            animation:       ToastAnimationSettings::default(),
        }
    }
}

/// Toast width in terminal cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastWidth(NonZeroU16);

impl ToastWidth {
    /// Raw width value.
    #[must_use]
    pub const fn get(self) -> u16 { self.0.get() }

    fn try_from_i64(value: i64) -> Result<Self, SettingsError> {
        let value = u16::try_from(value).map_err(|_| invalid("toasts", "width", "out of range"))?;
        let value =
            NonZeroU16::new(value).ok_or_else(|| invalid("toasts", "width", "must be nonzero"))?;
        Ok(Self(value))
    }
}

impl Default for ToastWidth {
    fn default() -> Self { Self(NonZeroU16::new(60).unwrap_or(NonZeroU16::MIN)) }
}

/// Gap between toasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastGap(u16);

impl ToastGap {
    /// Raw gap value.
    #[must_use]
    pub const fn get(self) -> u16 { self.0 }

    fn try_from_i64(value: i64) -> Result<Self, SettingsError> {
        let value = u16::try_from(value).map_err(|_| invalid("toasts", "gap", "out of range"))?;
        Ok(Self(value))
    }
}

impl Default for ToastGap {
    fn default() -> Self { Self(1) }
}

/// Toast duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastDuration(Duration);

impl ToastDuration {
    /// Default timed-toast timeout.
    pub const DEFAULT_TIMEOUT: Self = Self(Duration::from_secs(5));
    /// Default completed-task linger.
    pub const TASK_LINGER: Self = Self(Duration::from_secs(1));

    /// Build from seconds.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] when `secs` is negative or not finite.
    pub fn try_from_secs(key: &str, secs: f64) -> Result<Self, SettingsError> {
        if secs.is_finite() && secs >= 0.0 {
            Ok(Self(Duration::from_secs_f64(secs)))
        } else {
            Err(invalid(
                "toasts",
                key,
                "expected non-negative finite seconds",
            ))
        }
    }

    fn try_from_value(key: &str, value: &Value) -> Result<Self, SettingsError> {
        if let Some(secs) = value.as_float() {
            return Self::try_from_secs(key, secs);
        }
        if let Some(secs) = value.as_integer() {
            let secs = u64::try_from(secs)
                .map_err(|_| invalid("toasts", key, "expected non-negative seconds"))?;
            return Ok(Self(Duration::from_secs(secs)));
        }
        Err(invalid("toasts", key, "expected number"))
    }

    /// Raw duration.
    #[must_use]
    pub const fn get(self) -> Duration { self.0 }

    /// Duration in seconds.
    #[must_use]
    pub const fn as_secs_f64(self) -> f64 { self.0.as_secs_f64() }
}

/// Maximum number of visible toasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaxVisibleToasts(NonZeroUsize);

impl MaxVisibleToasts {
    /// Raw maximum.
    #[must_use]
    pub const fn get(self) -> usize { self.0.get() }

    fn try_from_i64(value: i64) -> Result<Self, SettingsError> {
        let value =
            usize::try_from(value).map_err(|_| invalid("toasts", "max_visible", "out of range"))?;
        let value = NonZeroUsize::new(value)
            .ok_or_else(|| invalid("toasts", "max_visible", "must be nonzero"))?;
        Ok(Self(value))
    }
}

impl Default for MaxVisibleToasts {
    fn default() -> Self { Self(NonZeroUsize::new(5).unwrap_or(NonZeroUsize::MIN)) }
}

/// Toast placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastPlacement {
    /// Bottom-right corner.
    BottomRight,
    /// Top-right corner.
    TopRight,
}

impl ToastPlacement {
    fn parse(value: &str) -> Result<Self, SettingsError> {
        match value {
            "bottom_right" => Ok(Self::BottomRight),
            "top_right" => Ok(Self::TopRight),
            _ => Err(invalid("toasts", "placement", "unknown placement")),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::BottomRight => "bottom_right",
            Self::TopRight => "top_right",
        }
    }
}

/// Toast animation timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastAnimationSettings {
    /// Entrance animation duration.
    pub entrance_duration: ToastDuration,
    /// Exit animation duration.
    pub exit_duration:     ToastDuration,
}

impl Default for ToastAnimationSettings {
    fn default() -> Self {
        Self {
            entrance_duration: ToastDuration(Duration::from_millis(150)),
            exit_duration:     ToastDuration(Duration::from_millis(150)),
        }
    }
}

fn remove_legacy_toast_keys(table: &mut Table) {
    if let Some(Value::Table(tui)) = table.get_mut("tui") {
        tui.remove("status_flash_secs");
        tui.remove("task_linger_secs");
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
    use super::SettingsFileSpec;
    use super::SettingsRegistry;
    use super::SettingsSection;
    use super::SettingsStore;
    use super::ToastSettings;

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
        let super::SettingKind::Custom {
            codecs: items_codecs,
        } = &items_entry.kind
        else {
            panic!("expected custom items entry");
        };
        let super::SettingKind::Custom {
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
