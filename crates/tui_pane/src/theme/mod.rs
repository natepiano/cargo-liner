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
mod spec;

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::RwLock;

use serde::Deserialize;

pub use self::accessors::accent_color;
pub use self::accessors::active_border_color;
pub use self::accessors::active_focus_color;
pub use self::accessors::column_header_color;
pub use self::accessors::discovery_shimmer_color;
pub use self::accessors::error_color;
pub use self::accessors::finder_match_bg;
pub use self::accessors::git_ignored_color;
pub use self::accessors::git_modified_color;
pub use self::accessors::git_untracked_color;
pub use self::accessors::hover_focus_color;
pub use self::accessors::inactive_border_color;
pub use self::accessors::inactive_title_color;
pub use self::accessors::inline_error_color;
pub use self::accessors::label_color;
pub use self::accessors::remembered_focus_color;
pub use self::accessors::secondary_text_color;
pub use self::accessors::status_bar_color;
pub use self::accessors::success_color;
pub use self::accessors::target_bench_color;
pub use self::accessors::text_default;
pub use self::accessors::title_color;
pub use self::builtins::default_dark;
pub use self::builtins::default_light;
pub use self::spec::Modifiers;
pub use self::spec::StyleSpec;

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

/// Semantic accents: success, error, accent text, labels.
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

/// Git-status markers (ignored, modified, untracked).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct GitTheme {
    /// Git ignored entries.
    pub ignored:   StyleSpec,
    /// Git modified entries.
    pub modified:  StyleSpec,
    /// Git untracked entries.
    pub untracked: StyleSpec,
}

/// Status-bar background and per-purpose accents.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct StatusTheme {
    /// Bottom status bar background.
    pub bar:           StyleSpec,
    /// Bench target type accent.
    pub target_bench:  StyleSpec,
    /// Project list column headers (defaults to Bold).
    pub column_header: StyleSpec,
}

/// Finder overlay styles.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct FinderTheme {
    /// Background tint on fuzzy-matched characters.
    pub match_bg:          StyleSpec,
    /// Shimmer on newly discovered projects.
    pub discovery_shimmer: StyleSpec,
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
    /// Git-status markers.
    pub git:         GitTheme,
    /// Status bar and accents.
    pub status:      StatusTheme,
    /// Finder overlay styles.
    pub finder:      FinderTheme,
    /// Per-row disk-usage gradient stops.
    pub disk_usage:  DiskUsageTheme,
}

/// Global container for the active theme.
///
/// Held in a single `OnceLock` so init happens once and ordering is
/// explicit. Reads via [`theme()`] take a `RwLock` read and clone the
/// `Arc`; sub-µs and unmeasurable against ratatui's per-cell work.
pub struct ThemeState {
    current: RwLock<Arc<Theme>>,
}

impl ThemeState {
    /// Build a [`ThemeState`] with the given initial active theme.
    #[must_use]
    pub fn new(initial: Theme) -> Self {
        Self {
            current: RwLock::new(Arc::new(initial)),
        }
    }
}

static THEME_STATE: OnceLock<ThemeState> = OnceLock::new();

/// Install the global theme state. Must be called once before any
/// render runs.
///
/// # Panics
///
/// Panics if called more than once.
pub fn install_theme_state(state: ThemeState) {
    assert!(
        THEME_STATE.set(state).is_ok(),
        "theme state already installed"
    );
}

/// Install the dark built-in if no theme state is present yet.
///
/// Idempotent — repeated calls are a no-op once installation has
/// succeeded. Use this from app startup and test setups that may
/// run more than once per process.
pub fn ensure_theme_state_installed() {
    if THEME_STATE.get().is_none() {
        let _ = THEME_STATE.set(ThemeState::new(default_dark()));
    }
}

/// Snapshot of the currently active theme.
///
/// Cheap to call (`RwLock` read + `Arc` clone). If no theme state has
/// been installed yet (tests that exercise render code without going
/// through full app startup, for example), the dark built-in is
/// installed on first access. App startup may call
/// [`install_theme_state`] or [`ensure_theme_state_installed`]
/// explicitly to make the initial value deterministic.
///
/// # Panics
///
/// Panics if the underlying `RwLock` is poisoned — that means a
/// previous theme swap panicked mid-write and the slot is no longer
/// in a recoverable state.
#[must_use]
pub fn theme() -> Arc<Theme> {
    let state = THEME_STATE.get_or_init(|| ThemeState::new(default_dark()));
    #[expect(
        clippy::expect_used,
        reason = "RwLock poisoning here means a previous panic during a theme swap; \
                  we cannot recover"
    )]
    state.current.read().expect("theme RwLock poisoned").clone()
}

/// Replace the active theme. Subsequent calls to [`theme()`] return
/// the new value.
///
/// # Panics
///
/// Panics if called before [`install_theme_state`], or if the
/// underlying `RwLock` is poisoned.
pub fn set_active_theme(new_theme: Arc<Theme>) {
    #[expect(
        clippy::expect_used,
        reason = "theme state must be installed before swap"
    )]
    let state = THEME_STATE.get().expect("theme state not installed");
    #[expect(
        clippy::expect_used,
        reason = "RwLock poisoning here means a previous panic during a theme swap; \
                  we cannot recover"
    )]
    let mut slot = state.current.write().expect("theme RwLock poisoned");
    *slot = new_theme;
}

/// Wrapper accepted by the Phase 1 roundtrip test.
///
/// A family file holds one or more variants. Phase 2 will widen the
/// registry around this (overrides, ids, error reporting); Phase 1
/// uses it only to parse the starter templates and assert they match
/// the Rust constructors.
#[derive(Debug, Deserialize)]
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
    /// Git-status markers.
    pub git:         GitTheme,
    /// Status bar and accents.
    pub status:      StatusTheme,
    /// Finder overlay styles.
    pub finder:      FinderTheme,
    /// Per-row disk-usage gradient stops.
    pub disk_usage:  DiskUsageTheme,
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
            git:         self.git,
            status:      self.status,
            finder:      self.finder,
            disk_usage:  self.disk_usage,
        }
    }
}
