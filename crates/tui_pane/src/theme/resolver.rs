//! Resolve which theme to install from `(mode_string, light_id,
//! dark_id, os_appearance)`.
//!
//! [`AppearanceMode`] parses the config string form (`"auto"` /
//! `"light"` / `"dark"`) and falls back to dark when an unknown
//! string arrives. [`ThemeRegistry::resolve_active`] composes the
//! mode with a registry lookup, returning the [`Arc<Theme>`] to
//! install plus optional diagnostics carried in [`ResolvedTheme`].

use std::sync::Arc;

use super::Appearance;
use super::Theme;
use super::ThemeId;
use super::ThemeRegistry;
use super::builtins;

/// Theme-selection strategy parsed from a config `mode` string.
///
/// `Auto` defers to the OS appearance; `Pinned` ignores the OS and
/// always returns the stored appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppearanceMode {
    /// Follow the OS appearance. The resolver returns
    /// [`Appearance::Dark`] when no OS setting is available.
    Auto,
    /// Always use the carried appearance, ignoring the OS.
    Pinned(Appearance),
}

impl AppearanceMode {
    /// Parse the string form used in app config.
    ///
    /// Accepts `"auto"`, `"light"`, `"dark"` (case-insensitive).
    ///
    /// # Errors
    ///
    /// Returns `Err` with a short reason on any other input so the
    /// caller can surface it.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "light" => Ok(Self::Pinned(Appearance::Light)),
            "dark" => Ok(Self::Pinned(Appearance::Dark)),
            other => Err(format!(
                "appearance.mode must be \"auto\", \"light\", or \"dark\" (got {other:?})"
            )),
        }
    }

    /// Resolve to a concrete [`Appearance`]. `os` is the last-known OS
    /// state. `Auto` falls back to `Dark` when no OS signal is
    /// available, matching the conservative default chosen by most
    /// terminal applications.
    #[must_use]
    pub const fn resolve(self, os: Option<Appearance>) -> Appearance {
        match self {
            Self::Pinned(appearance) => appearance,
            Self::Auto => match os {
                Some(appearance) => appearance,
                None => Appearance::Dark,
            },
        }
    }
}

/// Output of [`ThemeRegistry::resolve_active`].
///
/// Carries the [`Arc<Theme>`] to install, an optional `miss` for a
/// configured id that wasn't in the registry, and an optional
/// `mode_error` message if the mode string failed to parse.
pub struct ResolvedTheme {
    /// The theme to install via [`crate::set_active_theme`].
    pub theme:      Arc<Theme>,
    /// `Some(id)` when the configured theme name didn't exist in the
    /// registry. The caller decides whether to surface this.
    pub miss:       Option<ThemeId>,
    /// `Some(reason)` when the mode string failed to parse.
    /// Independent of `miss`; both can fire on the same call.
    pub mode_error: Option<String>,
}

impl ThemeRegistry {
    /// Resolve the active theme from `(mode_string, light_id,
    /// dark_id, os_appearance)`.
    ///
    /// `mode_string` is the app's config `appearance.mode` value
    /// (`"auto"` / `"light"` / `"dark"`). `light_name` and
    /// `dark_name` are the app's configured theme ids for each
    /// appearance. `os` is the last OS appearance reported by the
    /// poller (or `None` before the poller has emitted).
    ///
    /// A miss falls back to the appearance-matched built-in
    /// (`default_dark` / `default_light`) so the app stays usable
    /// even when the configured id is a typo. An invalid `mode_string`
    /// falls back to dark and is reported via `mode_error`.
    #[must_use]
    pub fn resolve_active(
        &self,
        mode_string: &str,
        light_name: &str,
        dark_name: &str,
        os: Option<Appearance>,
    ) -> ResolvedTheme {
        let (mode, mode_error) = match AppearanceMode::parse(mode_string) {
            Ok(mode) => (mode, None),
            Err(err) => (AppearanceMode::Pinned(Appearance::Dark), Some(err)),
        };
        let appearance = mode.resolve(os);
        let configured_name = match appearance {
            Appearance::Light => light_name,
            Appearance::Dark => dark_name,
        };
        let id = ThemeId::new(configured_name);
        let hit = self.find(&id);
        let theme = hit.map_or_else(
            || {
                Arc::new(match appearance {
                    Appearance::Light => builtins::default_light(),
                    Appearance::Dark => builtins::default_dark(),
                })
            },
            |variant| Arc::new(variant.theme.clone()),
        );
        let miss = if hit.is_none() { Some(id) } else { None };
        ResolvedTheme {
            theme,
            miss,
            mode_error,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn appearance_mode_parse_accepts_canonical_forms() {
        assert_eq!(AppearanceMode::parse("auto"), Ok(AppearanceMode::Auto));
        assert_eq!(
            AppearanceMode::parse("Light"),
            Ok(AppearanceMode::Pinned(Appearance::Light))
        );
        assert_eq!(
            AppearanceMode::parse("  DARK "),
            Ok(AppearanceMode::Pinned(Appearance::Dark))
        );
        assert!(AppearanceMode::parse("midnight").is_err());
    }

    #[test]
    fn appearance_mode_resolve_uses_os_only_in_auto() {
        assert_eq!(
            AppearanceMode::Auto.resolve(Some(Appearance::Light)),
            Appearance::Light
        );
        assert_eq!(AppearanceMode::Auto.resolve(None), Appearance::Dark);
        assert_eq!(
            AppearanceMode::Pinned(Appearance::Dark).resolve(Some(Appearance::Light)),
            Appearance::Dark
        );
    }

    #[test]
    fn resolve_active_hits_registry_for_pinned_dark() {
        let registry = ThemeRegistry::new_with_builtins();
        let resolved = registry.resolve_active("dark", "Default Light", "Default Dark", None);
        assert!(resolved.miss.is_none());
        assert!(resolved.mode_error.is_none());
        assert_eq!(*resolved.theme, builtins::default_dark());
    }

    #[test]
    fn resolve_active_miss_falls_back_to_builtin() {
        let registry = ThemeRegistry::new_with_builtins();
        let resolved = registry.resolve_active("dark", "Default Light", "Nonexistent", None);
        assert_eq!(resolved.miss, Some(ThemeId::new("Nonexistent")));
        assert_eq!(*resolved.theme, builtins::default_dark());
    }

    #[test]
    fn resolve_active_invalid_mode_falls_back_to_dark_with_error() {
        let registry = ThemeRegistry::new_with_builtins();
        let resolved = registry.resolve_active(
            "rainbow",
            "Default Light",
            "Default Dark",
            Some(Appearance::Light),
        );
        assert!(resolved.mode_error.is_some());
        assert!(resolved.miss.is_none());
        assert_eq!(*resolved.theme, builtins::default_dark());
    }

    #[test]
    fn resolve_active_auto_uses_os_appearance_when_present() {
        let registry = ThemeRegistry::new_with_builtins();
        let resolved = registry.resolve_active(
            "auto",
            "Default Light",
            "Default Dark",
            Some(Appearance::Light),
        );
        assert_eq!(*resolved.theme, builtins::default_light());
    }
}
