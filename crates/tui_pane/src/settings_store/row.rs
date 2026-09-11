/// One renderable row in a framework-owned settings pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsRow {
    /// Row label. Section rows use this as the section title.
    pub label:    String,
    /// Displayed value for selectable setting rows.
    pub value:    String,
    /// Row behavior.
    pub kind:     SettingsRowKind,
    /// Optional app-provided suffix shown after compact controls.
    pub suffix:   Option<String>,
    /// Whether this row is decoration or has a selectable-row index.
    pub identity: SettingsRowIdentity,
}

/// Selection identity of a settings row.
///
/// A selectable row's payload must equal its zero-based position among the
/// selectable rows supplied to [`crate::SettingsPane::render_rows`]. Decoration
/// rows do not count. The pane uses that index for cursor placement, scrolling,
/// and mouse selection; an arbitrary stable id or discriminant is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsRowIdentity {
    /// A section heading with no selectable row.
    Decoration,
    /// A selectable row with its positional index among selectable rows.
    Selectable(SettingsRowPayload),
}

/// Zero-based positional index used by settings hit testing and dispatch.
///
/// The value must equal the row's position among selectable rows in the current
/// input to [`crate::SettingsPane::render_rows`], excluding decoration rows.
/// Reordering selectable rows requires updating their payloads. The pane uses
/// this value for cursor placement and scrolling as well as mouse selection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SettingsRowPayload(usize);

impl SettingsRowPayload {
    /// Build a payload from the row's zero-based position among selectable rows.
    #[must_use]
    pub const fn new(value: usize) -> Self { Self(value) }

    /// Return the zero-based position among selectable rows.
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
            label:    label.into(),
            value:    String::new(),
            kind:     SettingsRowKind::Section,
            suffix:   None,
            identity: SettingsRowIdentity::Decoration,
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
            label:    label.into(),
            value:    value.into(),
            kind:     SettingsRowKind::Value,
            suffix:   None,
            identity: SettingsRowIdentity::Selectable(payload.into()),
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
            label:    label.into(),
            value:    if enabled { "ON" } else { "OFF" }.to_string(),
            kind:     SettingsRowKind::Toggle,
            suffix:   None,
            identity: SettingsRowIdentity::Selectable(payload.into()),
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
            label:    label.into(),
            value:    value.into(),
            kind:     SettingsRowKind::Stepper,
            suffix:   None,
            identity: SettingsRowIdentity::Selectable(payload.into()),
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
