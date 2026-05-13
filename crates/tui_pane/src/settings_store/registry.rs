use toml::Table;

use super::SettingsError;

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
