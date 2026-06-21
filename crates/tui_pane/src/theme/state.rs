use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use super::Theme;
use super::ThemeRegistry;
use super::default_dark;

/// Global container for the active theme and the variant registry.
///
/// Held in a single `OnceLock` so init happens once and ordering is
/// explicit. Both slots are `RwLock<Arc<...>>` so readers take a
/// read lock + `Arc` clone (sub-µs, unmeasurable against ratatui's
/// per-cell work) while hot-reload and theme swap take a write lock
/// to publish a new value.
///
/// The registry and the active theme share an invariant — "the
/// active theme's id should exist in the registry, or be a
/// compiled-in fallback" — that one struct enforces better than two
/// independently-managed statics.
pub struct ThemeState {
    registry:          RwLock<Arc<ThemeRegistry>>,
    current:           RwLock<Arc<Theme>>,
    /// When true, [`PaneChrome::block`](crate::PaneChrome::block) paints
    /// a subtle background tint behind the focused pane to lift it
    /// from neighbours. Defaults to true; client apps can mirror their
    /// focused-pane-tint config bit into this slot at startup and on
    /// config reload.
    focused_pane_tint: AtomicBool,
}

impl ThemeState {
    /// Build a [`ThemeState`] with a seeded built-ins registry and the
    /// given initial active theme. Phase 1 callers that don't yet
    /// supply a registry use this constructor.
    #[must_use]
    pub fn new(initial: Theme) -> Self {
        Self::with_registry(ThemeRegistry::new_with_builtins(), initial)
    }

    /// Build a [`ThemeState`] with a caller-supplied registry and
    /// initial active theme. Phase 2's app startup uses this after
    /// scanning the user themes directory.
    #[must_use]
    pub fn with_registry(registry: ThemeRegistry, initial: Theme) -> Self {
        Self {
            registry:          RwLock::new(Arc::new(registry)),
            current:           RwLock::new(Arc::new(initial)),
            focused_pane_tint: AtomicBool::new(true),
        }
    }
}

static THEME_STATE: OnceLock<ThemeState> = OnceLock::new();

/// Install the global theme state if no state is present yet.
///
/// Idempotent — a second call is a silent no-op so test binaries
/// that re-run startup can call this without panicking. Use
/// [`replace_registry`] or [`set_active_theme`] to update a
/// previously-installed state.
pub fn install_theme_state(state: ThemeState) { let _ = THEME_STATE.set(state); }

/// Install the dark built-in plus the built-ins registry if no theme
/// state is present yet.
///
/// Idempotent — repeated calls are a no-op once installation has
/// succeeded. Use this from app startup paths that may run more than
/// once per process; production startup prefers [`install_theme_state`]
/// with an explicit registry.
pub fn ensure_theme_state_installed() { install_theme_state(ThemeState::new(default_dark())); }

/// Snapshot of the currently active theme.
///
/// Cheap to call (`RwLock` read + `Arc` clone). If no theme state has
/// been installed yet (tests that exercise render code without going
/// through full app startup, for example), the dark built-in plus a
/// built-ins-only registry are installed on first access. App startup
/// may call [`install_theme_state`] or [`ensure_theme_state_installed`]
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
pub fn registry() -> Arc<ThemeRegistry> {
    let state = THEME_STATE.get_or_init(|| ThemeState::new(default_dark()));
    #[expect(
        clippy::expect_used,
        reason = "RwLock poisoning here means a previous panic during a registry swap; \
                  we cannot recover"
    )]
    state
        .registry
        .read()
        .expect("registry RwLock poisoned")
        .clone()
}

/// Replace the active theme. Subsequent calls to [`theme()`] return
/// the new value.
///
/// # Panics
///
/// Panics if the underlying `RwLock` is poisoned.
pub fn set_active_theme(new_theme: Arc<Theme>) {
    let state = THEME_STATE.get_or_init(|| ThemeState::new(default_dark()));
    #[expect(
        clippy::expect_used,
        reason = "RwLock poisoning here means a previous panic during a theme swap; \
                  we cannot recover"
    )]
    let mut slot = state.current.write().expect("theme RwLock poisoned");
    *slot = new_theme;
}

/// Whether the focused-pane background tint is enabled.
///
/// Read by [`PaneChrome::block`](crate::PaneChrome::block) every
/// render; client apps can mirror their focused-pane-tint config bit
/// into this slot. Defaults to true when no state has been installed
/// yet.
#[must_use]
pub fn focused_pane_tint_enabled() -> bool {
    let state = THEME_STATE.get_or_init(|| ThemeState::new(default_dark()));
    state.focused_pane_tint.load(Ordering::Relaxed)
}

/// Enable or disable the focused-pane background tint.
///
/// Idempotent; subsequent renders pick up the new value on the next
/// frame.
pub fn set_focused_pane_tint(enabled: bool) {
    let state = THEME_STATE.get_or_init(|| ThemeState::new(default_dark()));
    state.focused_pane_tint.store(enabled, Ordering::Relaxed);
}

/// Replace the theme registry. Subsequent calls to [`registry()`]
/// return the new value. Used by client hot-reload paths when theme
/// files change.
///
/// # Panics
///
/// Panics if the underlying `RwLock` is poisoned.
pub fn replace_registry(new_registry: ThemeRegistry) {
    let state = THEME_STATE.get_or_init(|| ThemeState::new(default_dark()));
    #[expect(
        clippy::expect_used,
        reason = "RwLock poisoning here means a previous panic during a registry swap; \
                  we cannot recover"
    )]
    let mut slot = state.registry.write().expect("registry RwLock poisoned");
    *slot = Arc::new(new_registry);
}
