//! Runtime-swappable theme system.
//!
//! A [`Theme`] is a grouped set of [`StyleSpec`] values consumed by the
//! render layer. The active theme lives behind a global
//! `OnceLock<ThemeState>` and is read via [`theme()`]. The main render
//! loop snapshots the active theme into a per-frame `Arc<Theme>` so
//! every cell of one frame sees one consistent palette.
//!
//! Phase 1 ships two compiled-in built-ins ([`builtins::default_dark`]
//! and [`builtins::default_light`]); later phases add a user-themes
//! registry and TOML-file loading.

mod accessors;
mod builtins;
mod constants;
mod loader;
mod poller;
mod registry;
mod resolver;
mod runtime;
mod spec;
mod state;
mod watch;

use std::collections::BTreeMap;

use serde::Deserialize;

pub use self::accessors::accent_color;
pub use self::accessors::active_border_color;
pub use self::accessors::active_focus_color;
pub use self::accessors::error_color;
pub use self::accessors::finder_match_bg;
pub use self::accessors::hover_focus_color;
pub use self::accessors::inactive_border_color;
pub use self::accessors::inactive_title_color;
pub use self::accessors::inline_error_color;
pub use self::accessors::label_color;
pub use self::accessors::remembered_focus_color;
pub use self::accessors::role_color;
pub use self::accessors::role_spec;
pub use self::accessors::role_style;
pub use self::accessors::secondary_text_color;
pub use self::accessors::status_bar_color;
pub use self::accessors::success_color;
pub use self::accessors::text_default;
pub use self::accessors::title_color;
pub use self::accessors::warning_color;
pub use self::builtins::default_dark;
pub use self::builtins::default_light;
pub use self::builtins::high_contrast_dark;
pub use self::builtins::high_contrast_light;
pub use self::poller::spawn_appearance_poller;
pub use self::registry::BUILTIN_DARK_NAME;
pub use self::registry::BUILTIN_HC_DARK_NAME;
pub use self::registry::BUILTIN_HC_LIGHT_NAME;
pub use self::registry::BUILTIN_LIGHT_NAME;
pub use self::registry::RegisterOutcome;
pub use self::registry::RegistryStatus;
pub use self::registry::ThemeId;
pub use self::registry::ThemeLoadError;
pub use self::registry::ThemeRegistry;
pub use self::registry::ThemeVariant;
pub use self::resolver::AppearanceMode;
pub use self::resolver::ResolvedTheme;
pub use self::runtime::ThemeRuntime;
pub use self::spec::Modifiers;
pub use self::spec::StyleSpec;
pub use self::state::ThemeState;
pub use self::state::ensure_theme_state_installed;
pub use self::state::focused_pane_tint_enabled;
pub use self::state::install_theme_state;
pub use self::state::registry;
pub use self::state::replace_registry;
pub use self::state::set_active_theme;
pub use self::state::set_focused_pane_tint;
pub use self::state::theme;
pub use self::watch::ThemesWatch;

/// Light vs dark variant target. Identifies which slot in a
/// `(light_theme, dark_theme)` config pair a variant fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Variant designed for light terminals.
    Light,
    /// Variant designed for dark terminals.
    Dark,
}

/// Pane borders and titles (focused vs unfocused).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PaneChromeTheme {
    /// Border of the currently focused pane.
    pub active_border:   StyleSpec,
    /// Border of unfocused panes.
    pub inactive_border: StyleSpec,
    /// Title of the currently focused pane.
    pub active_title:    StyleSpec,
    /// Title of unfocused panes.
    pub inactive_title:  StyleSpec,
}

/// Row-highlight states for focused / hovered / remembered selection.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct FocusTheme {
    /// Background of the currently focused row.
    pub active:     StyleSpec,
    /// Background of the row currently under the mouse.
    pub hover:      StyleSpec,
    /// Background of the row that held focus before the pane lost focus.
    pub remembered: StyleSpec,
}

/// Semantic accents: success, error, warning, accent text, labels.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SemanticTheme {
    /// Spinners, shortcut hints, finder cursor.
    pub accent:       StyleSpec,
    /// Error text, failure icons, error backgrounds.
    pub error:        StyleSpec,
    /// Inline errors on selected settings rows (where `error` would
    /// clash with the selection highlight background).
    pub inline_error: StyleSpec,
    /// Clean / passed / synced states.
    pub success:      StyleSpec,
    /// Field labels, stat labels, countdowns, hints, chevrons.
    pub label:        StyleSpec,
    /// Cautionary text — service unavailability placeholders, pending
    /// data that depends on an unreachable service. Distinct from
    /// `error` (which means "the operation failed") — `warning` means
    /// "this is degraded but recoverable."
    pub warning:      StyleSpec,
}

/// Foreground text styles (default, secondary, dim, bright, focus bg).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct TextTheme {
    /// Universal "regular foreground" text.
    pub default:   StyleSpec,
    /// Dim secondary text (shortcut descriptions, scan progress).
    pub secondary: StyleSpec,
    /// Faded text one step below `secondary`.
    pub dim:       StyleSpec,
    /// Bright accent text (matches `semantic.accent` in built-ins).
    pub bright:    StyleSpec,
    /// Background under high-contrast focus text.
    pub bg_focus:  StyleSpec,
}

/// Status-bar background.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct StatusTheme {
    /// Bottom status bar background.
    pub bar: StyleSpec,
}

/// Finder overlay styles.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct FinderTheme {
    /// Background tint on fuzzy-matched characters.
    pub match_bg: StyleSpec,
}

/// Three stops of the per-row disk-usage gradient.
///
/// The interpolation math (low→mid→high via `mul_add` against each
/// row's percentile) stays in code; only `.color` is consumed by the
/// gradient. Modifiers on these specs are ignored.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DiskUsageTheme {
    /// Smallest disk-usage rows. Default dark: green.
    pub low:  StyleSpec,
    /// Mid-percentile rows. Default dark: white. Default light: a
    /// neutral gray (white-on-white is invisible).
    pub mid:  StyleSpec,
    /// Largest disk-usage rows. Default dark: red.
    pub high: StyleSpec,
}

/// Complete palette consumed by the render layer.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Theme {
    /// Pane borders and titles.
    pub pane_chrome: PaneChromeTheme,
    /// Row-highlight states.
    pub focus:       FocusTheme,
    /// Semantic accents.
    pub semantic:    SemanticTheme,
    /// Foreground text styles.
    pub text:        TextTheme,
    /// Status bar and accents.
    pub status:      StatusTheme,
    /// Finder overlay styles.
    pub finder:      FinderTheme,
    /// Per-row disk-usage gradient stops.
    pub disk_usage:  DiskUsageTheme,
    /// Client-defined style roles keyed by app-owned names.
    pub roles:       BTreeMap<String, StyleSpec>,
}

/// Wrapper accepted by the Phase 1 roundtrip test.
///
/// A family file holds one or more variants. Phase 2 will widen the
/// registry around this (overrides, ids, error reporting); Phase 1
/// uses it only to parse the starter templates and assert they match
/// the Rust constructors.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeFamily {
    /// Schema version. Phase 1 only accepts `1`.
    pub schema:   u32,
    /// Family display name.
    pub name:     String,
    /// One or more named variants.
    pub variants: Vec<ThemeVariantFile>,
}

/// Single variant inside a [`ThemeFamily`] TOML file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeVariantFile {
    /// Unique variant name.
    pub name:        String,
    /// Light or dark target.
    pub appearance:  Appearance,
    /// Pane borders and titles.
    pub pane_chrome: PaneChromeTheme,
    /// Row-highlight states.
    pub focus:       FocusTheme,
    /// Semantic accents.
    pub semantic:    SemanticTheme,
    /// Foreground text styles.
    pub text:        TextTheme,
    /// Status bar and accents.
    pub status:      StatusTheme,
    /// Finder overlay styles.
    pub finder:      FinderTheme,
    /// Per-row disk-usage gradient stops.
    pub disk_usage:  DiskUsageTheme,
    /// Client-defined style roles keyed by app-owned names.
    pub roles:       BTreeMap<String, StyleSpec>,
}

impl ThemeVariantFile {
    /// Convert a parsed variant to its [`Theme`] (drops the name and
    /// appearance fields the registry uses separately).
    #[must_use]
    pub fn into_theme(self) -> Theme {
        Theme {
            pane_chrome: self.pane_chrome,
            focus:       self.focus,
            semantic:    self.semantic,
            text:        self.text,
            status:      self.status,
            finder:      self.finder,
            disk_usage:  self.disk_usage,
            roles:       self.roles,
        }
    }
}
