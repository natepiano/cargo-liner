use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use super::Appearance;
use super::Theme;
use super::ThemeRegistry;
use super::fallback_theme;
use super::resolution_notice;
use crate::AppIdentity;
use crate::AppearanceConfig;

/// The system appearance retained for resolving `appearance.mode = "auto"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RememberedAppearance {
    /// The system setting has not supplied a light or dark value.
    NotObserved,
    /// The system most recently selected a light appearance.
    Light,
    /// The system most recently selected a dark appearance.
    Dark,
}

impl RememberedAppearance {
    /// Convert at the boundary of the existing resolver API.
    pub(crate) const fn observed(self) -> Option<Appearance> {
        match self {
            Self::NotObserved => None,
            Self::Light => Some(Appearance::Light),
            Self::Dark => Some(Appearance::Dark),
        }
    }
}

impl From<Appearance> for RememberedAppearance {
    fn from(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Light => Self::Light,
            Appearance::Dark => Self::Dark,
        }
    }
}

/// Global container for the active theme and the variant registry.
///
/// Held in a single `OnceLock` so init happens once and ordering is
/// explicit. The registry and theme slots are `RwLock<Arc<...>>` so readers take a
/// read lock + `Arc` clone (sub-µs, unmeasurable against ratatui's
/// per-cell work) while hot-reload and theme swap take a write lock
/// to publish a new value.
///
/// The registry and the active theme share an invariant — "the
/// active theme's id should exist in the registry, or be a
/// compiled-in fallback" — that one struct enforces better than two
/// independently-managed statics.
pub struct ThemeState {
    registry:              RwLock<Arc<ThemeRegistry>>,
    current:               RwLock<Arc<Theme>>,
    remembered_appearance: RwLock<RememberedAppearance>,
    /// When true, [`PaneChrome::block`](crate::PaneChrome::block) paints
    /// a subtle background tint behind the focused pane to lift it
    /// from neighbours. Defaults to true; client apps can mirror their
    /// focused-pane-tint config bit into this slot at startup and on
    /// config reload. Painted only while [`Self::transparent`] is off.
    focused_pane_tint:     AtomicBool,
    /// When true, nothing paints a background under the app's main
    /// screen, so a transparent terminal window shows the desktop behind
    /// every cell. When false, the screen is painted solid in the
    /// theme's ground. Defaults to true, as the `[appearance]`
    /// `transparent` key does; apps mirror that key into this slot at
    /// startup and when it changes.
    transparent:           AtomicBool,
}

impl ThemeState {
    /// Build a [`ThemeState`] with an empty registry and the given
    /// initial active theme. For callers that render before any app
    /// variants exist; production startup uses
    /// [`with_registry`](Self::with_registry).
    #[must_use]
    pub fn new(initial: Theme) -> Self { Self::with_registry(ThemeRegistry::empty(), initial) }

    /// Build a [`ThemeState`] with a caller-supplied registry and
    /// initial active theme. App startup uses this after scanning the
    /// user themes directory.
    #[must_use]
    pub fn with_registry(registry: ThemeRegistry, initial: Theme) -> Self {
        Self {
            registry:              RwLock::new(Arc::new(registry)),
            current:               RwLock::new(Arc::new(initial)),
            remembered_appearance: RwLock::new(RememberedAppearance::NotObserved),
            focused_pane_tint:     AtomicBool::new(true),
            transparent:           AtomicBool::new(true),
        }
    }

    fn active_theme(&self) -> Arc<Theme> {
        #[expect(
            clippy::expect_used,
            reason = "a panic while replacing the active theme leaves no reliable palette"
        )]
        self.current.read().expect("theme RwLock poisoned").clone()
    }

    fn theme_registry(&self) -> Arc<ThemeRegistry> {
        #[expect(
            clippy::expect_used,
            reason = "a panic while replacing the registry leaves no reliable variant set"
        )]
        self.registry
            .read()
            .expect("registry RwLock poisoned")
            .clone()
    }

    fn remembered_appearance(&self) -> RememberedAppearance {
        #[expect(
            clippy::expect_used,
            reason = "a panic while recording the system appearance leaves no reliable value"
        )]
        *self
            .remembered_appearance
            .read()
            .expect("appearance RwLock poisoned")
    }

    fn replace_active_theme(&self, theme: Arc<Theme>) {
        #[expect(
            clippy::expect_used,
            reason = "a panic while replacing the active theme leaves no reliable palette"
        )]
        let mut current = self.current.write().expect("theme RwLock poisoned");
        *current = theme;
    }
}

/// Record a system appearance and select the theme requested by `appearance`.
///
/// Returns the notice for a configured theme id that matched nothing.
#[must_use]
pub fn apply_system_appearance<I: AppIdentity>(
    observed: Appearance,
    appearance: &AppearanceConfig<I>,
) -> Option<String> {
    apply_system_appearance_to(installed_state(), observed, appearance)
}

fn apply_system_appearance_to<I: AppIdentity>(
    state: &ThemeState,
    observed: Appearance,
    appearance: &AppearanceConfig<I>,
) -> Option<String> {
    #[expect(
        clippy::expect_used,
        reason = "a panic while recording the system appearance leaves no reliable value"
    )]
    let mut remembered = state
        .remembered_appearance
        .write()
        .expect("appearance RwLock poisoned");
    *remembered = observed.into();
    let registry = state.theme_registry();
    let resolved = registry.resolve_active(
        &appearance.mode,
        &appearance.light_theme,
        &appearance.dark_theme,
        remembered.observed(),
    );
    let notice = resolution_notice(&resolved);
    state.replace_active_theme(resolved.theme);
    drop(remembered);
    notice
}

static THEME_STATE: OnceLock<ThemeState> = OnceLock::new();

/// Install the global theme state if no state is present yet.
///
/// Idempotent — a second call is a silent no-op so test binaries
/// that re-run startup can call this without panicking. Use
/// [`replace_registry`] or [`set_active_theme`] to update a
/// previously-installed state.
pub fn install_theme_state(state: ThemeState) { let _ = THEME_STATE.set(state); }

/// Install the neutral dark [`fallback_theme`] and an empty registry
/// if no theme state is present yet.
///
/// Idempotent — repeated calls are a no-op once installation has
/// succeeded. Use this from app startup paths that may run more than
/// once per process; production startup prefers [`install_theme_state`]
/// with the app's own registry.
pub fn ensure_theme_state_installed() {
    install_theme_state(ThemeState::new(fallback_theme(Appearance::Dark)));
}

/// Snapshot of the currently active theme.
///
/// Cheap to call (`RwLock` read + `Arc` clone). If no theme state has
/// been installed yet (tests that exercise render code without going
/// through full app startup, for example), the neutral dark
/// [`fallback_theme`] plus an empty registry are installed on first
/// access. App startup
/// may call [`install_theme_state`] or [`ensure_theme_state_installed`]
/// explicitly to make the initial value deterministic.
///
/// # Panics
///
/// Panics if the underlying `RwLock` is poisoned — that means a
/// previous theme swap panicked mid-write and the slot is no longer
/// in a recoverable state.
#[must_use]
pub fn theme() -> Arc<Theme> { installed_state().active_theme() }

/// Snapshot of the currently-installed theme registry.
///
/// Returns an `Arc<ThemeRegistry>` so callers can hold it for the
/// duration of a settings render or a config-apply step without
/// racing against [`replace_registry`].
///
/// # Panics
///
/// Panics if the underlying `RwLock` is poisoned.
#[must_use]
pub fn registry() -> Arc<ThemeRegistry> { installed_state().theme_registry() }

/// Replace the active theme. Subsequent calls to [`theme()`] return
/// the new value.
///
/// # Panics
///
/// Panics if the underlying `RwLock` is poisoned.
pub fn set_active_theme(new_theme: Arc<Theme>) {
    installed_state().replace_active_theme(new_theme);
}

/// Whether the focused-pane background tint is enabled.
///
/// Read by [`PaneChrome::block`](crate::PaneChrome::block) every
/// render; client apps can mirror their focused-pane-tint config bit
/// into this slot. Defaults to true when no state has been installed
/// yet.
#[must_use]
pub fn focused_pane_tint_enabled() -> bool {
    let state = installed_state();
    state.focused_pane_tint.load(Ordering::Relaxed)
}

/// Enable or disable the focused-pane background tint.
///
/// Idempotent; subsequent renders pick up the new value on the next
/// frame.
pub fn set_focused_pane_tint(enabled: bool) {
    let state = installed_state();
    state.focused_pane_tint.store(enabled, Ordering::Relaxed);
}

/// Whether the main screen is left transparent: nothing paints a
/// background under it, the focused-pane tint included.
///
/// Read every render. Defaults to true when no state has been
/// installed yet.
#[must_use]
pub fn transparent_background() -> bool {
    let state = installed_state();
    state.transparent.load(Ordering::Relaxed)
}

/// Leave the main screen transparent, or paint it solid.
///
/// Idempotent; subsequent renders pick up the new value on the next
/// frame.
pub fn set_transparent_background(transparent: bool) {
    let state = installed_state();
    state.transparent.store(transparent, Ordering::Relaxed);
}

/// Replace the theme registry. Subsequent calls to [`registry()`]
/// return the new value. Used by client hot-reload paths when theme
/// files change.
///
/// # Panics
///
/// Panics if the underlying `RwLock` is poisoned.
pub fn replace_registry(new_registry: ThemeRegistry) {
    let state = installed_state();
    #[expect(
        clippy::expect_used,
        reason = "RwLock poisoning here means a previous panic during a registry swap; \
                  we cannot recover"
    )]
    let mut slot = state.registry.write().expect("registry RwLock poisoned");
    *slot = Arc::new(new_registry);
}

/// The installed [`ThemeState`], installing a neutral one first if
/// startup has not run.
///
/// Every accessor below funnels through here, so render code that
/// runs before an app has seeded its registry gets a coherent — if
/// deliberately plain — palette instead of a panic. See
/// [`fallback_theme`].
fn installed_state() -> &'static ThemeState {
    THEME_STATE.get_or_init(|| ThemeState::new(fallback_theme(Appearance::Dark)))
}

pub(crate) fn remembered_system_appearance() -> RememberedAppearance {
    THEME_STATE.get().map_or(
        RememberedAppearance::NotObserved,
        ThemeState::remembered_appearance,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThemeId;
    use crate::ThemeVariant;

    struct TestApp;

    impl AppIdentity for TestApp {
        const BINARY_NAME: &'static str = "test-app";
        const CONFIG_DIRNAME: &'static str = "test-app";
        const DEFAULT_LIGHT_THEME: &'static str = "App Light";
        const DEFAULT_DARK_THEME: &'static str = "App Dark";
    }

    fn variant(id: &str, appearance: Appearance) -> ThemeVariant {
        ThemeVariant {
            id: ThemeId::new(id),
            appearance,
            theme: fallback_theme(appearance),
        }
    }

    #[test]
    fn a_newly_observed_appearance_changes_the_active_theme() {
        let state = ThemeState::with_registry(
            ThemeRegistry::new_with_builtins(vec![
                variant("App Dark", Appearance::Dark),
                variant("App Light", Appearance::Light),
            ]),
            fallback_theme(Appearance::Dark),
        );
        let appearance = AppearanceConfig::<TestApp>::default();

        let notice = apply_system_appearance_to(&state, Appearance::Light, &appearance);

        assert!(notice.is_none());
        assert_eq!(state.remembered_appearance(), RememberedAppearance::Light);
        assert_eq!(*state.active_theme(), fallback_theme(Appearance::Light));
    }

    #[test]
    fn a_missing_light_theme_notice_names_the_selected_id() {
        let state = ThemeState::with_registry(
            ThemeRegistry::new_with_builtins(vec![
                variant("App Dark", Appearance::Dark),
                variant("App Light", Appearance::Light),
            ]),
            fallback_theme(Appearance::Dark),
        );
        let mut appearance = AppearanceConfig::<TestApp>::default();
        appearance.light_theme = "Missing Light".to_string();

        let notice = apply_system_appearance_to(&state, Appearance::Light, &appearance);

        assert_eq!(
            notice.as_deref(),
            Some("theme `Missing Light` not found — using a built-in")
        );
    }

    #[test]
    fn the_notice_follows_the_observed_appearance() {
        let state = ThemeState::with_registry(
            ThemeRegistry::new_with_builtins(vec![
                variant("App Dark", Appearance::Dark),
                variant("App Light", Appearance::Light),
            ]),
            fallback_theme(Appearance::Dark),
        );
        let mut appearance = AppearanceConfig::<TestApp>::default();
        appearance.dark_theme = "Missing Dark".to_string();

        let light_notice = apply_system_appearance_to(&state, Appearance::Light, &appearance);
        let dark_notice = apply_system_appearance_to(&state, Appearance::Dark, &appearance);

        assert!(light_notice.is_none());
        assert_eq!(
            dark_notice.as_deref(),
            Some("theme `Missing Dark` not found — using a built-in")
        );
    }
}
