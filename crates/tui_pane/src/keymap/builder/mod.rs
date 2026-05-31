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
use super::nav_action;
use super::runtime_scope;
use super::runtime_scope::PaneScope;
use super::runtime_scope::RuntimeScope;
use super::scope_map::ScopeMap;
use super::vim::VimMode;
use crate::AppContext;
use crate::CopySelection;
use crate::Framework;
use crate::NavAction;
use crate::OverlayAction;
use crate::Pane;
use crate::SettingsPane;
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
use registration::check_reserved_vim_navigation_keys;
use registration::reserved_vim_navigation_keys;

/// `Box<dyn Any>`-erased typed singleton. The builder stores the
/// `ScopeMap<X::Actions>` from a `Navigation` / `Globals` impl behind
/// this so [`Keymap`] can hold heterogeneous singletons in one field.
type ErasedSingleton = Box<dyn Any>;

/// `<N>`/`<G>`-monomorphized renderer that materializes a scope's
/// bar slots without naming the action enum. Captured at `register_*`
/// time and copied onto [`Keymap`] for the bar to read.
type ScopeRenderFn<Ctx> = fn(&Keymap<Ctx>) -> Vec<super::RenderedSlot>;

struct PaneRegistration<Ctx: AppContext> {
    app_pane_id: Ctx::AppPaneId,
    mode_query:  ModeQuery<Ctx>,
    tab_stop:    TabStop<Ctx>,
}

struct CopyRegistration<Ctx: AppContext> {
    register: fn(&mut Framework<Ctx>),
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
    scopes:                   HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>,
    pane_registrations:       Vec<PaneRegistration<Ctx>>,
    copy_registrations:       Vec<CopyRegistration<Ctx>>,
    registered_scopes:        HashSet<&'static str>,
    duplicate_scope:          Option<&'static str>,
    config_path:              Option<PathBuf>,
    toml_table:               Option<Table>,
    vim_mode:                 VimMode,
    on_quit:                  Option<fn(&mut Ctx)>,
    on_restart:               Option<fn(&mut Ctx)>,
    dismiss_fallback:         Option<fn(&mut Ctx) -> bool>,
    navigation_scope:         Option<ScopeMap<NavAction>>,
    navigation_scope_name:    Option<&'static str>,
    /// `N`-monomorphized renderer captured at
    /// [`Self::register_navigation`] time; copied onto the keymap in
    /// [`finalize`]. The bar uses
    /// [`Keymap::render_navigation_slots`] without naming `N`.
    navigation_render_fn:     Option<ScopeRenderFn<Ctx>>,
    globals_scope:            Option<ErasedSingleton>,
    globals_scope_name:       Option<&'static str>,
    globals_action_keys:      Option<HashSet<&'static str>>,
    /// `G`-monomorphized renderer captured at
    /// [`Self::register_globals`] time. See
    /// [`Self::navigation_render_fn`].
    globals_render_fn:        Option<ScopeRenderFn<Ctx>>,
    /// `G`-monomorphized help-row renderer captured at
    /// [`Self::register_globals`] time for the global shortcut
    /// viewer.
    globals_shortcut_rows_fn: Option<super::ScopeShortcutRowsFn<Ctx>>,
    /// `N`-monomorphized help-row renderer captured at
    /// [`Self::register_navigation`] time for the keymap-help overlay.
    navigation_help_rows_fn:  Option<super::ScopeHelpRowsFn<Ctx>>,
    /// `G`-monomorphized help-row renderer captured at
    /// [`Self::register_globals`] time for the keymap-help overlay.
    app_globals_help_rows_fn: Option<super::ScopeHelpRowsFn<Ctx>>,
    /// `N`-monomorphized TOML-action-key collector for the keymap
    /// TOML writer.
    navigation_toml_keys_fn:  Option<super::ScopeTomlActionKeysFn<Ctx>>,
    /// `G`-monomorphized TOML-action-key collector for the keymap
    /// TOML writer.
    app_globals_toml_keys_fn: Option<super::ScopeTomlActionKeysFn<Ctx>>,
    overlay_scope:            Option<ScopeMap<OverlayAction>>,
    vim_reserved_keys:        Vec<super::KeySequence>,
    deferred_error:           Option<KeymapError>,
    /// When set, unknown actions / scopes in the loaded TOML are
    /// skipped and recorded in [`Self::unknown_warnings`] instead of
    /// failing the build. See [`Self::ignore_unknown_entries`].
    ignore_unknown:           bool,
    /// Human-readable warnings for each TOML entry skipped because
    /// [`Self::ignore_unknown`] is set. Moved onto the finalized
    /// [`Keymap`] so the binary can surface them.
    unknown_warnings:         Vec<String>,
    _state:                   PhantomData<State>,
}

impl<Ctx: AppContext + 'static> KeymapBuilder<Ctx, Configuring> {
    /// Empty builder.
    pub(super) fn new() -> Self {
        Self {
            scopes:                   HashMap::new(),
            pane_registrations:       Vec::new(),
            copy_registrations:       Vec::new(),
            registered_scopes:        HashSet::new(),
            duplicate_scope:          None,
            config_path:              None,
            toml_table:               None,
            vim_mode:                 VimMode::Disabled,
            on_quit:                  None,
            on_restart:               None,
            dismiss_fallback:         None,
            navigation_scope:         None,
            navigation_scope_name:    None,
            navigation_render_fn:     None,
            globals_scope:            None,
            globals_scope_name:       None,
            globals_action_keys:      None,
            globals_render_fn:        None,
            globals_shortcut_rows_fn: None,
            navigation_help_rows_fn:  None,
            app_globals_help_rows_fn: None,
            navigation_toml_keys_fn:  None,
            app_globals_toml_keys_fn: None,
            overlay_scope:            None,
            vim_reserved_keys:        Vec::new(),
            deferred_error:           None,
            ignore_unknown:           false,
            unknown_warnings:         Vec::new(),
            _state:                   PhantomData,
        }
    }

    /// Make subsequent overlay steps tolerant of unknown TOML entries:
    /// an action key with no matching variant, or a top-level scope
    /// table that no registered scope claims, is skipped and recorded
    /// as a warning (retrievable via
    /// [`Keymap::unknown_warnings`](super::Keymap::unknown_warnings))
    /// rather than surfaced as a [`KeymapError`].
    ///
    /// Lets a binary tolerate a stale on-disk keymap — e.g. a binding
    /// for an action that was renamed or removed in a newer version —
    /// without bricking startup, while still surfacing the dropped
    /// entries to the user. Parse errors, in-scope collisions, and
    /// reserved-key conflicts stay fatal.
    #[must_use]
    pub const fn ignore_unknown_entries(mut self) -> Self {
        self.ignore_unknown = true;
        self
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
    /// [`N::defaults()`](Navigation::defaults) (with TOML overlay,
    /// then vim extras) into a [`ScopeMap<N::Actions>`].
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError`] on TOML parse / validation failures
    /// inside the `[N::SCOPE_NAME]` table.
    pub fn register_navigation<N: Navigation<Ctx>>(mut self) -> Result<Self, KeymapError> {
        let scope_name = <N as Navigation<Ctx>>::SCOPE_NAME;
        let mut bindings = apply_toml_overlay_with_peer::<NavAction>(
            scope_name,
            nav_action::default_bindings(),
            self.toml_table.as_ref(),
            None,
            self.ignore_unknown.then_some(&mut self.unknown_warnings),
            true,
        )?;
        if matches!(self.vim_mode, VimMode::Enabled) {
            apply_vim_navigation_extras(&mut bindings);
            self.vim_reserved_keys = reserved_vim_navigation_keys();
        }
        overlay::check_cross_action_collision(scope_name, &bindings)?;
        self.navigation_scope = Some(bindings.into_scope_map());
        self.navigation_scope_name = Some(scope_name);
        self.navigation_render_fn = Some(runtime_scope::render_navigation_slots::<Ctx>);
        self.navigation_help_rows_fn =
            Some(runtime_scope::keymap_help_rows_for_navigation::<Ctx, N>);
        self.navigation_toml_keys_fn = Some(runtime_scope::navigation_toml_action_keys::<Ctx>);
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
            self.ignore_unknown.then_some(&mut self.unknown_warnings),
            false,
        )?;
        check_reserved_vim_navigation_keys(scope_name, &bindings, &self.vim_reserved_keys)?;
        let scope_map: ScopeMap<G::Actions> = bindings.into_scope_map();
        self.globals_scope = Some(Box::new(scope_map));
        self.globals_scope_name = Some(scope_name);
        if scope_name == "global" {
            self.globals_action_keys = Some(action_key_set::<G::Actions>());
        }
        self.globals_render_fn = Some(runtime_scope::render_app_globals_slots::<Ctx, G>);
        self.globals_shortcut_rows_fn =
            Some(runtime_scope::render_app_global_shortcut_rows::<Ctx, G>);
        self.app_globals_help_rows_fn =
            Some(runtime_scope::keymap_help_rows_for_app_globals::<Ctx, G>);
        self.app_globals_toml_keys_fn = Some(runtime_scope::app_globals_toml_action_keys::<Ctx, G>);
        self.registered_scopes.insert(scope_name);
        Ok(self)
    }

    /// Register the framework-owned overlay scope. This makes
    /// `[overlay]` a known TOML table and applies its overrides to the
    /// single [`OverlayAction`] scope shared by both the settings and
    /// keymap overlay panes.
    ///
    /// # Errors
    ///
    /// Returns [`KeymapError`] on TOML parse / validation failures
    /// inside the `[overlay]` table.
    pub fn register_overlay(mut self) -> Result<Self, KeymapError> {
        let bindings = apply_toml_overlay::<OverlayAction>(
            "overlay",
            SettingsPane::defaults(),
            self.toml_table.as_ref(),
            self.ignore_unknown.then_some(&mut self.unknown_warnings),
        )?;
        self.overlay_scope = Some(bindings.into_scope_map());
        self.registered_scopes.insert("overlay");
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

    /// Register a [`Pane<Ctx>`] that has no pane-local actions. Only
    /// records the `(APP_PANE_ID, mode, tab_stop)` registration so the
    /// pane participates in focus cycling; no scope is inserted, and
    /// the pane contributes nothing to the bar's `PaneAction` region.
    /// Use this for panes whose only interactions live on the global
    /// or navigation scopes.
    #[must_use]
    pub fn register_pane<P: Pane<Ctx>>(self) -> KeymapBuilder<Ctx, Registering> {
        let mut next = transition::<Ctx>(self);
        next.insert_pane_no_shortcuts::<P>();
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

    /// Register copy support for a pane already known to the framework.
    #[must_use]
    pub fn register_copy_selection<P>(mut self) -> Self
    where
        P: CopySelection<Ctx> + Pane<Ctx>,
    {
        self.copy_registrations.push(CopyRegistration {
            register: Framework::<Ctx>::register_copy_selection::<P>,
        });
        self
    }

    /// Register an additional [`Pane<Ctx>`] without a `Shortcuts`
    /// impl. See [`KeymapBuilder::<Configuring>::register_pane`].
    #[must_use]
    pub fn register_pane<P: Pane<Ctx>>(mut self) -> Self {
        self.insert_pane_no_shortcuts::<P>();
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
                registration.app_pane_id,
                registration.mode_query,
                registration.tab_stop,
            );
        }
        for registration in &self.copy_registrations {
            (registration.register)(framework);
        }
        finalize(self)
    }
}

impl<Ctx: AppContext + 'static, State> KeymapBuilder<Ctx, State> {
    /// Insert one pane scope. Records the `(app_pane_id, mode_fn)` pair for
    /// `build_into` and the `SCOPE_NAME` for cross-scope validation.
    /// Detects `APP_PANE_ID` duplicates and stashes the offender's
    /// type name for `build` / `build_into` to surface as
    /// [`KeymapError::DuplicateScope`].
    fn insert_pane<P: Shortcuts<Ctx>>(&mut self, pane: P) {
        if self.scopes.contains_key(&P::APP_PANE_ID) && self.duplicate_scope.is_none() {
            self.duplicate_scope = Some(type_name::<P>());
        }
        let bindings = match build_pane_bindings::<Ctx, P>(
            self.toml_table.as_ref(),
            self.vim_mode,
            self.ignore_unknown.then_some(&mut self.unknown_warnings),
        ) {
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
            app_pane_id: P::APP_PANE_ID,
            mode_query:  <P as Pane<Ctx>>::mode(),
            tab_stop:    <P as Pane<Ctx>>::tab_stop(),
        });
        self.registered_scopes
            .insert(<P as Shortcuts<Ctx>>::SCOPE_NAME);
    }

    /// Record a pane registration without inserting a scope. Used by
    /// [`register_pane`](Self::register_pane) for panes that have no
    /// pane-local actions but still need to appear in focus cycling.
    fn insert_pane_no_shortcuts<P: Pane<Ctx>>(&mut self) {
        self.pane_registrations.push(PaneRegistration {
            app_pane_id: P::APP_PANE_ID,
            mode_query:  <P as Pane<Ctx>>::mode(),
            tab_stop:    <P as Pane<Ctx>>::tab_stop(),
        });
    }
}

/// Move from the [`Configuring`] type to [`Registering`]. Field-by-
/// field move so the typestate transition is purely a type change at
/// runtime.
fn transition<Ctx: AppContext + 'static>(
    src: KeymapBuilder<Ctx, Configuring>,
) -> KeymapBuilder<Ctx, Registering> {
    KeymapBuilder {
        scopes:                   src.scopes,
        pane_registrations:       src.pane_registrations,
        copy_registrations:       src.copy_registrations,
        registered_scopes:        src.registered_scopes,
        duplicate_scope:          src.duplicate_scope,
        config_path:              src.config_path,
        toml_table:               src.toml_table,
        vim_mode:                 src.vim_mode,
        on_quit:                  src.on_quit,
        on_restart:               src.on_restart,
        dismiss_fallback:         src.dismiss_fallback,
        navigation_scope:         src.navigation_scope,
        navigation_scope_name:    src.navigation_scope_name,
        navigation_render_fn:     src.navigation_render_fn,
        globals_scope:            src.globals_scope,
        globals_scope_name:       src.globals_scope_name,
        globals_action_keys:      src.globals_action_keys,
        globals_render_fn:        src.globals_render_fn,
        globals_shortcut_rows_fn: src.globals_shortcut_rows_fn,
        navigation_help_rows_fn:  src.navigation_help_rows_fn,
        app_globals_help_rows_fn: src.app_globals_help_rows_fn,
        navigation_toml_keys_fn:  src.navigation_toml_keys_fn,
        app_globals_toml_keys_fn: src.app_globals_toml_keys_fn,
        overlay_scope:            src.overlay_scope,
        vim_reserved_keys:        src.vim_reserved_keys,
        deferred_error:           src.deferred_error,
        ignore_unknown:           src.ignore_unknown,
        unknown_warnings:         src.unknown_warnings,
        _state:                   PhantomData,
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
