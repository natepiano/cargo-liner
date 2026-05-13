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
    pub payload: Option<SettingsRowPayload>,
}

/// Stable row payload used by settings hit testing and dispatch.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SettingsRowPayload(usize);

impl SettingsRowPayload {
    /// Build a settings row payload from an app-owned row id.
    #[must_use]
    pub const fn new(value: usize) -> Self { Self(value) }

    /// Return the app-owned row id.
    #[must_use]
    pub const fn get(self) -> usize { self.0 }
}

impl From<usize> for SettingsRowPayload {
    fn from(value: usize) -> Self { Self::new(value) }
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
    pub fn value(
        payload: impl Into<SettingsRowPayload>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            label:   label.into(),
            value:   value.into(),
            kind:    SettingsRowKind::Value,
            suffix:  None,
            payload: Some(payload.into()),
        }
    }

    /// Build a selectable toggle row.
    #[must_use]
    pub fn toggle(
        payload: impl Into<SettingsRowPayload>,
        label: impl Into<String>,
        enabled: bool,
    ) -> Self {
        Self {
            label:   label.into(),
            value:   if enabled { "ON" } else { "OFF" }.to_string(),
            kind:    SettingsRowKind::Toggle,
            suffix:  None,
            payload: Some(payload.into()),
        }
    }

    /// Build a selectable stepper row.
    #[must_use]
    pub fn stepper(
        payload: impl Into<SettingsRowPayload>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            label:   label.into(),
            value:   value.into(),
            kind:    SettingsRowKind::Stepper,
            suffix:  None,
            payload: Some(payload.into()),
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
