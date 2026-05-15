use core::num::NonZeroU16;
use core::num::NonZeroUsize;
use std::time::Duration;

use toml::Table;
use toml::Value;

use crate::settings_store::SettingsError;

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
    pub enabled:               bool,
    /// Toast card width.
    pub width:                 ToastWidth,
    /// Gap between visible toast cards.
    ///
    /// Kept for settings-file compatibility; toast cards render adjacent today.
    pub gap:                   ToastGap,
    /// How many seconds a "status" toast (a quick `push_timed`
    /// pop-up like "Saved" or "Already clean") stays on screen
    /// before auto-closing. Was previously named `default_timeout`.
    pub status_toast_visible:  ToastDuration,
    /// How many seconds a task toast stays on screen after the
    /// tracked task finishes (and how long each completed
    /// tracked item lingers in the toast body). Was previously
    /// named `task_linger`.
    pub finished_task_visible: ToastDuration,
    /// Maximum number of visible toasts.
    pub max_visible:           MaxVisibleToasts,
    /// Toast placement.
    pub placement:             ToastPlacement,
    /// Toast animation timing.
    pub animation:             ToastAnimationSettings,
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
            ToastGap::try_from_i64(
                value
                    .as_integer()
                    .ok_or_else(|| invalid("toasts", "gap", "expected integer"))?,
            )?;
        }
        if let Some(value) = table.get("status_toast_visible") {
            self.status_toast_visible =
                ToastDuration::try_from_value("status_toast_visible", value)?;
        } else if let Some(value) = table.get("default_timeout") {
            // Legacy key (pre-`status_toast_visible` rename). Read
            // it as a fall-through; the next save removes it via
            // [`remove_legacy_toast_keys`].
            self.status_toast_visible = ToastDuration::try_from_value("default_timeout", value)?;
        }
        if let Some(value) = table.get("finished_task_visible") {
            self.finished_task_visible =
                ToastDuration::try_from_value("finished_task_visible", value)?;
        } else if let Some(value) = table.get("task_linger") {
            // Legacy key (pre-`finished_task_visible` rename).
            self.finished_task_visible = ToastDuration::try_from_value("task_linger", value)?;
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
            self.status_toast_visible = ToastDuration::try_from_value("status_flash_secs", value)?;
        }
        if let Some(value) = table.get("task_linger_secs") {
            self.finished_task_visible = ToastDuration::try_from_value("task_linger_secs", value)?;
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
            "status_toast_visible".to_string(),
            Value::Float(self.status_toast_visible.as_secs_f64()),
        );
        toasts.insert(
            "finished_task_visible".to_string(),
            Value::Float(self.finished_task_visible.as_secs_f64()),
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
            enabled:               true,
            width:                 ToastWidth::default(),
            gap:                   ToastGap::default(),
            status_toast_visible:  ToastDuration::STATUS_TOAST_VISIBLE,
            finished_task_visible: ToastDuration::FINISHED_TASK_VISIBLE,
            max_visible:           MaxVisibleToasts::default(),
            placement:             ToastPlacement::BottomRight,
            animation:             ToastAnimationSettings::default(),
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
///
/// Existing settings files may contain this key. Rendering currently normalizes
/// it to zero so separate toast cards remain adjacent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// Toast duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastDuration(Duration);

impl ToastDuration {
    /// Default visible duration for status toasts.
    pub const STATUS_TOAST_VISIBLE: Self = Self(Duration::from_secs(5));
    /// Default visible duration for finished task toasts.
    pub const FINISHED_TASK_VISIBLE: Self = Self(Duration::from_secs(1));

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

pub(crate) fn remove_legacy_toast_keys(table: &mut Table) {
    if let Some(Value::Table(tui)) = table.get_mut("tui") {
        tui.remove("status_flash_secs");
        tui.remove("task_linger_secs");
    }
    if let Some(Value::Table(toasts)) = table.get_mut("toasts") {
        // One-shot migration: `default_timeout` →
        // `status_toast_visible`, `task_linger` →
        // `finished_task_visible`. Remove the old keys after the
        // load path has copied their values into the new fields.
        toasts.remove("default_timeout");
        toasts.remove("task_linger");
    }
}
