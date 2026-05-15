//! `KeymapBuilder<Ctx, State>`: typestate construction surface for
//! [`Keymap<Ctx>`](super::Keymap).
//!
//! Two states:
//!
//! - [`Configuring`]: settings phase. Settings methods (`config_path`, `load_toml`, `vim_mode`,
//!   `on_quit`, `on_restart`, `dismiss_fallback`, `register_navigation`, `register_globals`,
//!   `register_settings_overlay`, `register_keymap_overlay`) are reachable here only.
//! - [`Registering`]: panes phase. Entered on the first [`KeymapBuilder::register`] call. Settings
//!   methods drop off the type — the compiler enforces "settings before panes" at compile time.
//!   `build_into(&mut Framework<Ctx>)` is the production finalizer.

use core::any::Any;
use core::any::type_name;
use core::marker::PhantomData;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::PathBuf;

use toml::Table;

use super::Bindings;
use super::Globals;
use super::Keymap;
use super::Navigation;
use super::Shortcuts;
use super::load::KeymapError;
use super::runtime_scope;
use super::runtime_scope::PaneScope;
use super::runtime_scope::RuntimeScope;
use super::scope_map::ScopeMap;
use super::vim::VimMode;
use crate::AppContext;
use crate::Framework;
use crate::KeymapPane;
use crate::KeymapPaneAction;
use crate::Pane;
use crate::SettingsPane;
use crate::SettingsPaneAction;
use crate::TabStop;
use crate::pane::ModeQuery;

mod finalize;
mod overlay;
mod registration;

use finalize::finalize;
use overlay::action_key_set;
use overlay::apply_toml_overlay;
use overlay::apply_toml_overlay_with_peer;
use overlay::framework_global_action_key_set;
use registration::apply_vim_navigation_extras;
use registration::build_pane_bindings;

/// `Box<dyn Any>`-erased typed singleton. The builder stores the
/// `ScopeMap<X::Actions>` from a `Navigation` / `Globals` impl behind
/// this so [`Keymap`] can hold heterogeneous singletons in one field.
type ErasedSingleton = Box<dyn Any>;

/// `<N>`/`<G>`-monomorphized renderer that materializes a scope's
/// bar slots without naming the action enum. Captured at `register_*`
/// time and copied onto [`Keymap`] for the bar to read.
type ScopeRenderFn<Ctx> = fn(&Keymap<Ctx>) -> Vec<super::RenderedSlot>;

struct PaneRegistration<Ctx: AppContext> {
    id:         Ctx::AppPaneId,
    mode_query: ModeQuery<Ctx>,
    tab_stop:   TabStop<Ctx>,
}

/// Marker: builder is in the settings phase. Consumes settings
/// chained methods (`config_path`, etc.). Transitions to
/// [`Registering`] on the first [`KeymapBuilder::register`] call.
pub struct Configuring;

/// Marker: builder is in the panes phase. Settings methods are no
/// longer reachable on the type.
pub struct Registering;

/// Builder for [`Keymap<Ctx>`].
///
/// Constructed via [`Keymap::<Ctx>::builder()`]. Type parameter
/// `State` is one of [`Configuring`] (default) or [`Registering`];
/// the first [`Self::register`] call transitions the type.
///
/// `build()` returns [`Result<Keymap<Ctx>, KeymapError>`] so loader
/// and validation failures surface uniformly.
///
/// # Compile-time ordering
///
/// Settings methods are not callable on a builder in the
/// [`Registering`] state — the type system enforces "settings before
/// panes":
///
/// ```compile_fail
/// fn check<Ctx: tui_pane::AppContext + 'static>(
///     b: tui_pane::KeymapBuilder<Ctx, tui_pane::Registering>,
///     path: std::path::PathBuf,
/// ) {
///     // ERROR: no method `config_path` on `KeymapBuilder<Ctx, Registering>`.
///     let _ = b.config_path(path);
/// }
/// ```
pub struct KeymapBuilder<Ctx: AppContext + 'static, State = Configuring> {
    scopes:                HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>,
    pane_registrations:    Vec<PaneRegistration<Ctx>>,
    registered_scopes:     HashSet<&'static str>,
    duplicate_scope:       Option<&'static str>,
    config_path:           Option<PathBuf>,
    toml_table:            Option<Table>,
    vim_mode:              VimMode,
    on_quit:               Option<fn(&mut Ctx)>,
    on_restart:            Option<fn(&mut Ctx)>,
    dismiss_fallback:      Option<fn(&mut Ctx) -> bool>,
    navigation_scope:      Option<ErasedSingleton>,
    navigation_scope_name: Option<&'static str>,
    /// `N`-monomorphized renderer captured at
    /// [`Self::register_navigation`] time; copied onto the keymap in
    /// [`finalize`]. The bar uses
    /// [`Keymap::render_navigation_slots`] without naming `N`.
    navigation_render_fn:  Option<ScopeRenderFn<Ctx>>,
    globals_scope:         Option<ErasedSingleton>,
    globals_scope_name:    Option<&'static str>,
    globals_action_keys:   Option<HashSet<&'static str>>,
    /// `G`-monomorphized renderer captured at
    /// [`Self::register_globals`] time. See
    /// [`Self::navigation_render_fn`].
    globals_render_fn:     Option<ScopeRenderFn<Ctx>>,
    settings_overlay:      Option<ScopeMap<SettingsPaneAction>>,
    keymap_overlay:        Option<ScopeMap<KeymapPaneAction>>,
    deferred_error:        Option<KeymapError>,
    _state:                PhantomData<State>,
}

impl<Ctx: AppContext + 'static> KeymapBuilder<Ctx, Configuring> {
    /// Empty builder.
    pub(super) fn new() -> Self {
        Self {
            scopes:                HashMap::new(),
            pane_registrations:    Vec::new(),
            registered_scopes:     HashSet::new(),
            duplicate_scope:       None,
            config_path:           None,
            toml_table:            None,
            vim_mode:              VimMode::Disabled,
            on_quit:               None,
            on_restart:            None,
            dismiss_fallback:      None,
            navigation_scope:      None,
            navigation_scope_name: None,
            navigation_render_fn:  None,
            globals_scope:         None,
            globals_scope_name:    None,
            globals_action_keys:   None,
            globals_render_fn:     None,
            settings_overlay:      None,
            keymap_overlay:        None,
            deferred_error:        None,
            _state:                PhantomData,
        }
    }

    /// Override the config path the loader will read.
    #[must_use]
    pub fn config_path(mut self, path: PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    /// Read and parse the keymap TOML file at `path`. The parsed table
    /// is stored on the builder; subsequent `register*` calls overlay
    /// each scope's overrides onto its defaults.
    ///
    /// A missing file is treated as "use defaults" — `Ok(self)` is
    /// returned with no overlay table set. Parse failures and
    /// non-`NotFound` I/O errors surface as [`KeymapError`].
    ///
    /// Also records the path on the builder, equivalent to calling
    /// [`Self::config_path`] with the same path.
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError::Io`] on a read failure other than
    /// `NotFound`, or [`KeymapError::Parse`] on a TOML syntax error.
    pub fn load_toml(mut self, path: PathBuf) -> Result<Self, KeymapError> {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                self.config_path = Some(path);
                return Ok(self);
            },
            Err(err) => return Err(KeymapError::Io(err)),
        };
        let table: Table = toml::from_str(&text)?;
        self.toml_table = Some(table);
        self.config_path = Some(path);
        Ok(self)
    }

    /// Set the [`VimMode`] flag. When [`VimMode::Enabled`], each
    /// subsequent `register*` call appends vim navigation extras after
    /// applying TOML overrides.
    #[must_use]
    pub const fn vim_mode(mut self, mode: VimMode) -> Self {
        self.vim_mode = mode;
        self
    }

    /// Register a hook called after [`crate::GlobalAction::Quit`]
    /// flips
    /// `framework.quit_requested`. The hook can rely on
    /// `ctx.framework().quit_requested() == true`.
    #[must_use]
    pub const fn on_quit(mut self, hook: fn(&mut Ctx)) -> Self {
        self.on_quit = Some(hook);
        self
    }

    /// Register a hook called after [`crate::GlobalAction::Restart`]
    /// flips
    /// `framework.restart_requested`.
    #[must_use]
    pub const fn on_restart(mut self, hook: fn(&mut Ctx)) -> Self {
        self.on_restart = Some(hook);
        self
    }

    /// Register a fallback the framework calls when its own dismiss
    /// chain matches nothing. Returns `true` if the binary handled the
    /// dismiss (so the dispatcher can stop), `false` otherwise.
    #[must_use]
    pub const fn dismiss_fallback(mut self, hook: fn(&mut Ctx) -> bool) -> Self {
        self.dismiss_fallback = Some(hook);
        self
    }

    /// Register the [`Navigation`] singleton. Eagerly collapses
    /// [`N::defaults()`](Navigation::defaults) (with TOML and vim
    /// extras overlay) into a [`ScopeMap<N::Actions>`].
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError`] on TOML parse / validation failures
    /// inside the `[N::SCOPE_NAME]` table.
    pub fn register_navigation<N: Navigation<Ctx>>(mut self) -> Result<Self, KeymapError> {
        let defaults = N::defaults();
        let scope_name = <N as Navigation<Ctx>>::SCOPE_NAME;
        let mut bindings =
            apply_toml_overlay::<N::Actions>(scope_name, defaults, self.toml_table.as_ref())?;
        if matches!(self.vim_mode, VimMode::Enabled) {
            apply_vim_navigation_extras::<Ctx, N>(&mut bindings);
        }
        let scope_map: ScopeMap<N::Actions> = bindings.into_scope_map();
        self.navigation_scope = Some(Box::new(scope_map));
        self.navigation_scope_name = Some(scope_name);
        self.navigation_render_fn = Some(runtime_scope::render_navigation_slots::<Ctx, N>);
        self.registered_scopes.insert(scope_name);
        Ok(self)
    }

    /// Register the [`Globals`] singleton. Eagerly collapses
    /// [`G::defaults()`](Globals::defaults) (with TOML overlay from
    /// the shared `[global]` table) into a [`ScopeMap<G::Actions>`].
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError`] on TOML parse / validation failures
    /// inside the `[G::SCOPE_NAME]` table.
    pub fn register_globals<G: Globals<Ctx>>(mut self) -> Result<Self, KeymapError> {
        let defaults = G::defaults();
        let scope_name = <G as Globals<Ctx>>::SCOPE_NAME;
        let framework_keys = framework_global_action_key_set();
        let peer_keys = (scope_name == "global").then_some(&framework_keys);
        let bindings = apply_toml_overlay_with_peer::<G::Actions>(
            scope_name,
            defaults,
            self.toml_table.as_ref(),
            peer_keys,
        )?;
        let scope_map: ScopeMap<G::Actions> = bindings.into_scope_map();
        self.globals_scope = Some(Box::new(scope_map));
        self.globals_scope_name = Some(scope_name);
        if scope_name == "global" {
            self.globals_action_keys = Some(action_key_set::<G::Actions>());
        }
        self.globals_render_fn = Some(runtime_scope::render_app_globals_slots::<Ctx, G>);
        self.registered_scopes.insert(scope_name);
        Ok(self)
    }

    /// Register the framework-owned settings overlay scope. This
    /// makes `[settings]` a known TOML table and applies its overrides
    /// to the settings overlay bindings.
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError`] on TOML parse / validation failures
    /// inside the `[settings]` table.
    pub fn register_settings_overlay(mut self) -> Result<Self, KeymapError> {
        let bindings = apply_toml_overlay::<SettingsPaneAction>(
            "settings",
            SettingsPane::defaults(),
            self.toml_table.as_ref(),
        )?;
        self.settings_overlay = Some(bindings.into_scope_map());
        self.registered_scopes.insert("settings");
        Ok(self)
    }

    /// Register the framework-owned keymap overlay scope. This makes
    /// `[keymap]` a known TOML table and applies its overrides to the
    /// keymap overlay bindings.
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError`] on TOML parse / validation failures
    /// inside the `[keymap]` table.
    pub fn register_keymap_overlay(mut self) -> Result<Self, KeymapError> {
        let bindings = apply_toml_overlay::<KeymapPaneAction>(
            "keymap",
            KeymapPane::defaults(),
            self.toml_table.as_ref(),
        )?;
        self.keymap_overlay = Some(bindings.into_scope_map());
        self.registered_scopes.insert("keymap");
        Ok(self)
    }

    /// Register a [`Shortcuts<Ctx>`] impl. Eagerly collapses
    /// [`P::defaults()`](Shortcuts::defaults) (with TOML and vim
    /// extras overlay) into a [`ScopeMap<P::Actions>`] and stores the
    /// typed pane behind a [`RuntimeScope`] trait object keyed on
    /// `P::APP_PANE_ID`. Transitions the builder to [`Registering`].
    ///
    /// Errors during overlay are deferred until [`Self::build`] /
    /// [`Self::build_into`] so the chain stays a `Self`-returning
    /// flow. Today the overlay logic does not emit deferred errors per
    /// pane; that becomes relevant only when the loader's validation
    /// surface widens.
    ///
    /// Duplicate `APP_PANE_ID`s — two distinct `P` types claiming the
    /// same id — are recorded and surfaced as
    /// [`KeymapError::DuplicateScope`] from `build` / `build_into`.
    pub fn register<P: Shortcuts<Ctx>>(self, pane: P) -> KeymapBuilder<Ctx, Registering> {
        let mut next = transition::<Ctx>(self);
        next.insert_pane::<P>(pane);
        next
    }

    /// Finalize the builder with no scopes registered. Returns the
    /// built [`Keymap<Ctx>`].
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError::NavigationMissing`] /
    /// [`KeymapError::GlobalsMissing`] when the matching singleton
    /// was not registered. Loader / overlay errors propagate from the
    /// `register*` methods that ran earlier.
    pub fn build(self) -> Result<Keymap<Ctx>, KeymapError> { finalize(self) }
}

impl<Ctx: AppContext + 'static> KeymapBuilder<Ctx, Registering> {
    /// Register an additional [`Shortcuts<Ctx>`] impl. Same body as
    /// the [`Configuring`]-state form, but stays in [`Registering`].
    #[must_use]
    pub fn register<P: Shortcuts<Ctx>>(mut self, pane: P) -> Self {
        self.insert_pane::<P>(pane);
        self
    }

    /// Finalize the builder. Returns the built [`Keymap<Ctx>`].
    ///
    /// Production code should call [`Self::build_into`] instead so
    /// the framework's per-pane mode-query and tab-stop registries are
    /// populated.
    ///
    /// # Errors
    ///
    /// Same as [`KeymapBuilder::<Configuring>::build`].
    pub fn build(self) -> Result<Keymap<Ctx>, KeymapError> { finalize(self) }

    /// Production finalizer. Builds the [`Keymap<Ctx>`] *and* writes
    /// the registered `(AppPaneId, mode_fn)` pairs into the
    /// framework's per-pane registries so
    /// [`Framework::focused_pane_mode`](crate::Framework::focused_pane_mode)
    /// can answer for every registered pane and focus cycling can
    /// read each pane's tab-stop metadata.
    ///
    /// # Errors
    ///
    /// Same as [`Self::build`].
    pub fn build_into(self, framework: &mut Framework<Ctx>) -> Result<Keymap<Ctx>, KeymapError> {
        for registration in &self.pane_registrations {
            framework.register_app_pane(
                registration.id,
                registration.mode_query,
                registration.tab_stop,
            );
        }
        finalize(self)
    }
}

impl<Ctx: AppContext + 'static, State> KeymapBuilder<Ctx, State> {
    /// Insert one pane scope. Records the `(id, mode_fn)` pair for
    /// `build_into` and the `SCOPE_NAME` for cross-scope validation.
    /// Detects `APP_PANE_ID` duplicates and stashes the offender's
    /// type name for `build` / `build_into` to surface as
    /// [`KeymapError::DuplicateScope`].
    fn insert_pane<P: Shortcuts<Ctx>>(&mut self, pane: P) {
        if self.scopes.contains_key(&P::APP_PANE_ID) && self.duplicate_scope.is_none() {
            self.duplicate_scope = Some(type_name::<P>());
        }
        let bindings = match build_pane_bindings::<Ctx, P>(self.toml_table.as_ref(), self.vim_mode)
        {
            Ok(b) => b,
            Err(err) => {
                if self.deferred_error.is_none() {
                    self.deferred_error = Some(err);
                }
                Bindings::new()
            },
        };
        let scope: Box<dyn RuntimeScope<Ctx>> = Box::new(PaneScope {
            pane,
            bindings: bindings.into_scope_map(),
        });
        self.scopes.insert(P::APP_PANE_ID, scope);
        self.pane_registrations.push(PaneRegistration {
            id:         P::APP_PANE_ID,
            mode_query: <P as Pane<Ctx>>::mode(),
            tab_stop:   <P as Pane<Ctx>>::tab_stop(),
        });
        self.registered_scopes
            .insert(<P as Shortcuts<Ctx>>::SCOPE_NAME);
    }
}

/// Move from the [`Configuring`] type to [`Registering`]. Field-by-
/// field move so the typestate transition is purely a type change at
/// runtime.
fn transition<Ctx: AppContext + 'static>(
    src: KeymapBuilder<Ctx, Configuring>,
) -> KeymapBuilder<Ctx, Registering> {
    KeymapBuilder {
        scopes:                src.scopes,
        pane_registrations:    src.pane_registrations,
        registered_scopes:     src.registered_scopes,
        duplicate_scope:       src.duplicate_scope,
        config_path:           src.config_path,
        toml_table:            src.toml_table,
        vim_mode:              src.vim_mode,
        on_quit:               src.on_quit,
        on_restart:            src.on_restart,
        dismiss_fallback:      src.dismiss_fallback,
        navigation_scope:      src.navigation_scope,
        navigation_scope_name: src.navigation_scope_name,
        navigation_render_fn:  src.navigation_render_fn,
        globals_scope:         src.globals_scope,
        globals_scope_name:    src.globals_scope_name,
        globals_action_keys:   src.globals_action_keys,
        globals_render_fn:     src.globals_render_fn,
        settings_overlay:      src.settings_overlay,
        keymap_overlay:        src.keymap_overlay,
        deferred_error:        src.deferred_error,
        _state:                PhantomData,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests;
