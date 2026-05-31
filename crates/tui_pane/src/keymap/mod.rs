//! Keymap: types and traits for binding keys to actions.

mod action_enum;
mod bindings;
mod builder;
mod global_action;
mod globals;
mod key_bind;
mod key_outcome;
mod key_sequence;
mod load;
mod nav_action;
mod navigation;
mod runtime_scope;
mod scope_map;
mod shortcuts;
mod vim;

use core::fmt::Formatter;
use std::any::Any;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

pub use action_enum::Action;
pub use bindings::Bindings;
pub use builder::Configuring;
pub use builder::KeymapBuilder;
#[allow(
    unused_imports,
    reason = "re-exported at crate root for callers naming the typestate"
)]
pub use builder::Registering;
pub use global_action::GlobalAction;
pub use globals::Globals;
pub use key_bind::KeyBind;
pub use key_bind::KeyParseError;
pub use key_outcome::KeyOutcome;
pub use key_sequence::KeySequence;
pub use load::KeymapError;
pub use nav_action::NavAction;
pub use navigation::Navigation;
pub use runtime_scope::GlobalShortcutRow;
pub use runtime_scope::KeymapHelpRow;
pub use runtime_scope::RenderedSlot;
pub use scope_map::ScopeMap;
pub use shortcuts::Shortcuts;
pub use vim::VimMode;

use self::runtime_scope::RuntimeScope;
use crate::AppContext;
use crate::BarRegion;
use crate::OverlayAction;
use crate::SettingsPane;
use crate::framework;

/// `<N>`/`<G>`-monomorphized renderer the bar reads to materialize
/// bar slots without naming the action enum. Mirror of
/// [`builder::ScopeRenderFn`](self::builder::ScopeRenderFn) — the
/// keymap and the builder both store one of these per scope.
type ScopeRenderFn<Ctx> = fn(&Keymap<Ctx>) -> Vec<RenderedSlot>;

/// `<G>`-monomorphized renderer the global-shortcuts overlay reads to
/// materialize app-global help rows without naming the action enum.
type ScopeShortcutRowsFn<Ctx> = fn(&Keymap<Ctx>) -> Vec<GlobalShortcutRow>;

/// Monomorphized renderer for one scope's keymap-help rows. Stored on
/// [`Keymap<Ctx>`] at register time so the help overlay can walk every
/// scope without naming the action enum.
type ScopeHelpRowsFn<Ctx> = fn(&Keymap<Ctx>) -> Vec<KeymapHelpRow>;

/// Monomorphized TOML-action-key collector for one scope. Lets the
/// keymap TOML writer iterate every action even when no binding
/// currently exists.
type ScopeTomlActionKeysFn<Ctx> = fn(&Keymap<Ctx>) -> Vec<&'static str>;

/// The keymap container: anchor for every binding the framework
/// resolves at runtime.
///
/// Built with [`Self::builder`], which returns a
/// [`KeymapBuilder<Ctx, Configuring>`]. Settings phase first
/// (`config_path`, `load_toml`, `vim_mode`); on the first
/// [`KeymapBuilder::register`] call the type transitions to
/// [`KeymapBuilder<Ctx, Registering>`] and settings methods drop off
/// the type.
///
/// One [`Ctx::AppPaneId`](AppContext::AppPaneId)-keyed scope per
/// registered pane. Public callers reach pane operations through
/// [`Self::dispatch_app_pane`], [`Self::render_app_pane_bar_slots`],
/// [`Self::key_for_toml_key`], and [`Self::is_key_bound_to_toml_key`] — the underlying
/// [`RuntimeScope`](self::runtime_scope::RuntimeScope) trait is
/// crate-private.
///
/// Framework panes are not stored in this map — they are special-cased
/// by [`FocusedPane::Framework`](crate::FocusedPane::Framework) arms in
/// callers.
pub struct Keymap<Ctx: AppContext + 'static> {
    scopes:                       HashMap<Ctx::AppPaneId, Box<dyn RuntimeScope<Ctx>>>,
    navigation:                   Option<ScopeMap<NavAction>>,
    /// Monomorphized renderer for the navigation scope. Each
    /// [`KeymapBuilder::register_navigation::<N>`](crate::KeymapBuilder::register_navigation)
    /// call sets this to the `N`-specialized free fn in
    /// [`runtime_scope::render_navigation_slots`]. The bar renderer
    /// reads it via [`Self::render_navigation_slots`] without naming
    /// `N`.
    navigation_render_fn:         Option<ScopeRenderFn<Ctx>>,
    globals:                      Option<Box<dyn Any>>,
    /// Monomorphized renderer for the app-globals scope. See
    /// [`Self::navigation_render_fn`].
    app_globals_render_fn:        Option<ScopeRenderFn<Ctx>>,
    /// Monomorphized help-row renderer for the app-globals scope.
    app_globals_shortcut_rows_fn: Option<ScopeShortcutRowsFn<Ctx>>,
    /// Monomorphized keymap-help row renderer for navigation.
    navigation_help_rows_fn:      Option<ScopeHelpRowsFn<Ctx>>,
    /// Monomorphized keymap-help row renderer for app-globals.
    app_globals_help_rows_fn:     Option<ScopeHelpRowsFn<Ctx>>,
    /// Monomorphized TOML-action-key collector for navigation.
    navigation_toml_keys_fn:      Option<ScopeTomlActionKeysFn<Ctx>>,
    /// Monomorphized TOML-action-key collector for app-globals.
    app_globals_toml_keys_fn:     Option<ScopeTomlActionKeysFn<Ctx>>,
    framework_globals:            ScopeMap<GlobalAction>,
    overlay_scope:                ScopeMap<OverlayAction>,
    on_quit:                      Option<fn(&mut Ctx)>,
    on_restart:                   Option<fn(&mut Ctx)>,
    dismiss_fallback:             Option<fn(&mut Ctx) -> bool>,
    config_path:                  Option<PathBuf>,
    /// Warnings for TOML entries skipped during the build because the
    /// builder ran in
    /// [`ignore_unknown_entries`](KeymapBuilder::ignore_unknown_entries)
    /// mode. Empty in the default (strict) build, where such entries
    /// fail the build instead.
    unknown_warnings:             Vec<String>,
}

impl<Ctx: AppContext + 'static> Keymap<Ctx> {
    /// Canonical entry point for assembling a keymap. Returns the
    /// builder in [`Configuring`] state — settings methods first,
    /// then [`KeymapBuilder::register`] transitions to [`Registering`].
    #[must_use]
    pub fn builder() -> KeymapBuilder<Ctx, Configuring> { KeymapBuilder::new() }

    /// Empty keymap, no scopes registered. `pub(super)` because only
    /// [`KeymapBuilder::build`] (sibling) constructs one.
    pub(super) fn new(config_path: Option<PathBuf>) -> Self {
        Self {
            scopes: HashMap::new(),
            navigation: None,
            navigation_render_fn: None,
            globals: None,
            app_globals_render_fn: None,
            app_globals_shortcut_rows_fn: None,
            navigation_help_rows_fn: None,
            app_globals_help_rows_fn: None,
            navigation_toml_keys_fn: None,
            app_globals_toml_keys_fn: None,
            framework_globals: GlobalAction::defaults().into_scope_map(),
            overlay_scope: SettingsPane::defaults().into_scope_map(),
            on_quit: None,
            on_restart: None,
            dismiss_fallback: None,
            config_path,
            unknown_warnings: Vec::new(),
        }
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one. The navigation action set is framework-owned
    /// ([`NavAction`]), so the scope map is stored with its concrete
    /// type — no `Box<dyn Any>` erasure, no downcast on read.
    pub(super) fn set_navigation(&mut self, scope_map: ScopeMap<NavAction>) {
        self.navigation = Some(scope_map);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// stores it, alongside [`Self::set_navigation`].
    pub(super) const fn set_navigation_render_fn(&mut self, render: ScopeRenderFn<Ctx>) {
        self.navigation_render_fn = Some(render);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) fn set_globals(&mut self, scope_map: Box<dyn Any>) {
        self.globals = Some(scope_map);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// stores it, alongside [`Self::set_globals`].
    pub(super) const fn set_app_globals_render_fn(&mut self, render: ScopeRenderFn<Ctx>) {
        self.app_globals_render_fn = Some(render);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// stores it, alongside [`Self::set_globals`].
    pub(super) const fn set_app_globals_shortcut_rows_fn(
        &mut self,
        render: ScopeShortcutRowsFn<Ctx>,
    ) {
        self.app_globals_shortcut_rows_fn = Some(render);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// stores the registered navigation impl's help-row renderer.
    pub(super) const fn set_navigation_help_rows_fn(&mut self, render: ScopeHelpRowsFn<Ctx>) {
        self.navigation_help_rows_fn = Some(render);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// stores the registered app-globals impl's help-row renderer.
    pub(super) const fn set_app_globals_help_rows_fn(&mut self, render: ScopeHelpRowsFn<Ctx>) {
        self.app_globals_help_rows_fn = Some(render);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// stores the registered navigation impl's TOML-key collector.
    pub(super) const fn set_navigation_toml_keys_fn(&mut self, render: ScopeTomlActionKeysFn<Ctx>) {
        self.navigation_toml_keys_fn = Some(render);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// stores the registered app-globals impl's TOML-key collector.
    pub(super) const fn set_app_globals_toml_keys_fn(
        &mut self,
        render: ScopeTomlActionKeysFn<Ctx>,
    ) {
        self.app_globals_toml_keys_fn = Some(render);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) fn set_framework_globals(&mut self, map: ScopeMap<GlobalAction>) {
        self.framework_globals = map;
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// overlays user TOML onto the framework-owned overlay scope. One
    /// scope drives every framework overlay bar; those panes consume
    /// [`OverlayAction`].
    pub(super) fn set_overlay_scope(&mut self, map: ScopeMap<OverlayAction>) {
        self.overlay_scope = map;
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) const fn set_on_quit(&mut self, hook: fn(&mut Ctx)) { self.on_quit = Some(hook); }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) const fn set_on_restart(&mut self, hook: fn(&mut Ctx)) {
        self.on_restart = Some(hook);
    }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) const fn set_dismiss_fallback(&mut self, hook: fn(&mut Ctx) -> bool) {
        self.dismiss_fallback = Some(hook);
    }

    /// Insert one scope under its `AppPaneId`. `pub(super)` so only
    /// the builder writes.
    pub(super) fn insert_scope(
        &mut self,
        app_pane_id: Ctx::AppPaneId,
        scope: Box<dyn RuntimeScope<Ctx>>,
    ) {
        self.scopes.insert(app_pane_id, scope);
    }

    /// Path to the config file the loader read (or would read), if
    /// any. `None` when the binary registered no config path or the
    /// file is missing — the loader treats a missing file as "use
    /// defaults" and returns `Ok`.
    #[must_use]
    pub fn config_path(&self) -> Option<&Path> { self.config_path.as_deref() }

    /// `pub(super)` because only [`KeymapBuilder::build`] (sibling)
    /// constructs one.
    pub(super) fn set_unknown_warnings(&mut self, warnings: Vec<String>) {
        self.unknown_warnings = warnings;
    }

    /// Warnings for TOML entries skipped during the build (unknown
    /// actions and unknown scopes). Non-empty only when the keymap was
    /// built via
    /// [`KeymapBuilder::ignore_unknown_entries`]; the default strict
    /// build fails on such entries instead of recording them. The
    /// binary surfaces these to the user (e.g. a startup toast) so a
    /// stale on-disk keymap is visible without bricking startup.
    #[must_use]
    pub fn unknown_warnings(&self) -> &[String] { &self.unknown_warnings }

    /// Resolve `bind` to an action in the scope registered for
    /// `app_pane_id` and call its dispatcher. Returns
    /// [`KeyOutcome::Unhandled`] if no scope is registered for
    /// `app_pane_id` or no binding matches; the caller continues its
    /// dispatch chain (globals, dismiss, fallback) on `Unhandled`.
    pub fn dispatch_app_pane(
        &self,
        app_pane_id: Ctx::AppPaneId,
        bind: &KeyBind,
        ctx: &mut Ctx,
    ) -> KeyOutcome {
        self.scopes
            .get(&app_pane_id)
            .map_or(KeyOutcome::Unhandled, |s| s.dispatch_key(bind, ctx))
    }

    /// Bar slots for the scope registered for `app_pane_id`, fully
    /// resolved (label / key / state / visibility). Returns an empty
    /// `Vec` if no scope is registered.
    #[must_use]
    pub fn render_app_pane_bar_slots(
        &self,
        app_pane_id: Ctx::AppPaneId,
        ctx: &Ctx,
    ) -> Vec<RenderedSlot> {
        self.scopes
            .get(&app_pane_id)
            .map_or_else(Vec::new, |s| s.render_bar_slots(ctx))
    }

    /// Reverse lookup: TOML action key string → currently bound
    /// [`KeyBind`] in the scope registered for `app_pane_id`. Returns
    /// `None` if no scope is registered for `app_pane_id`, the action
    /// name is not recognized, or the named action has no binding.
    #[must_use]
    pub fn key_for_toml_key(
        &self,
        app_pane_id: Ctx::AppPaneId,
        action: &str,
    ) -> Option<KeySequence> {
        self.scopes.get(&app_pane_id)?.key_for_toml_key(action)
    }

    /// Reverse lookup: TOML action key string → every currently bound
    /// [`KeyBind`] in the scope registered for `app_pane_id`. Returns
    /// an empty vector if no scope is registered for `app_pane_id`, the
    /// action name is not recognized, or the named action has no
    /// binding.
    #[must_use]
    pub fn keys_for_toml_key(&self, app_pane_id: Ctx::AppPaneId, action: &str) -> Vec<KeySequence> {
        self.scopes
            .get(&app_pane_id)
            .map_or_else(Vec::new, |scope| scope.keys_for_toml_key(action))
    }

    /// Reverse lookup predicate: returns `true` when `bind` is one of
    /// the keys currently bound to the TOML action key string in the
    /// scope registered for `app_pane_id`.
    ///
    /// Unlike [`Self::key_for_toml_key`], this checks every binding
    /// for the action, not just the primary key rendered in compact
    /// UI. Structural input checks should use this predicate so TOML
    /// arrays like `cancel = ["Esc", "q"]` work for every key.
    #[must_use]
    pub fn is_key_bound_to_toml_key(
        &self,
        app_pane_id: Ctx::AppPaneId,
        action: &str,
        bind: &KeyBind,
    ) -> bool {
        self.scopes
            .get(&app_pane_id)
            .is_some_and(|scope| scope.is_key_bound_to_toml_key(action, bind))
    }

    /// The framework-globals scope ([`GlobalAction`] →
    /// [`KeyBind`]). Built from
    /// [`GlobalAction::defaults`](GlobalAction::defaults) plus any
    /// `[global]` overrides at builder time.
    #[must_use]
    pub const fn framework_globals(&self) -> &ScopeMap<GlobalAction> { &self.framework_globals }

    /// Resolved bindings for the framework-owned overlay bar
    /// ([`OverlayAction`] → [`KeyBind`]). Shared by both the settings
    /// and keymap overlays.
    ///
    /// Defaults are always present; calling
    /// [`KeymapBuilder::register_overlay`] additionally applies user
    /// TOML from `[overlay]`.
    #[must_use]
    pub const fn overlay(&self) -> &ScopeMap<OverlayAction> { &self.overlay_scope }

    /// Bar slots for the navigation scope, fully resolved (label,
    /// key, region tagged [`BarRegion::Nav`](crate::BarRegion::Nav)).
    /// Returns an empty `Vec` when no navigation impl was registered.
    ///
    /// The bar renderer reads this and partitions by region without
    /// naming the binary's `<N>` — the `N`-monomorphized renderer was
    /// stored at [`KeymapBuilder::register_navigation`](crate::KeymapBuilder::register_navigation)
    /// time.
    #[must_use]
    pub fn render_navigation_slots(&self) -> Vec<RenderedSlot> {
        self.navigation_render_fn
            .map_or_else(Vec::new, |render| render(self))
    }

    /// Bar slots for the app-globals scope, fully resolved (label,
    /// key, region tagged [`BarRegion::Global`](crate::BarRegion::Global)).
    /// Returns an empty `Vec` when no globals impl was registered.
    ///
    /// Mirrors [`Self::render_navigation_slots`].
    #[must_use]
    pub fn render_app_globals_slots(&self) -> Vec<RenderedSlot> {
        self.app_globals_render_fn
            .map_or_else(Vec::new, |render| render(self))
    }

    /// Bar slots for the framework-globals scope ([`GlobalAction`]
    /// variants in [`GlobalAction::ALL`] order, region tagged
    /// [`BarRegion::Global`](crate::BarRegion::Global)). Always
    /// available — the framework-globals scope ships fully populated.
    ///
    /// `BarRegion::Global` covers the full strip the bar emits at the
    /// far right; the `bar` renderer is free to split out
    /// [`GlobalAction::NextPane`] / [`GlobalAction::PrevPane`] into
    /// its own pane-cycle slot inside the nav region — that slot is
    /// rendered by direct lookup against
    /// [`Self::framework_globals`], not by filtering this `Vec`.
    #[must_use]
    pub fn render_framework_globals_slots(&self) -> Vec<RenderedSlot> {
        self::runtime_scope::slots_from_scope(
            BarRegion::Global,
            GlobalAction::ALL,
            &self.framework_globals,
        )
    }

    /// Rows for the framework-owned global shortcut viewer.
    ///
    /// Framework globals and registered app globals are combined here
    /// so embedding crates only register their `Globals` scope; they
    /// do not build or render the help list.
    #[must_use]
    pub fn global_shortcut_rows(&self) -> Vec<GlobalShortcutRow> {
        let mut rows = Vec::new();
        rows.push(self.framework_global_shortcut_row("Global Navigation", GlobalAction::NextPane));
        rows.push(self.framework_global_shortcut_row("Global Navigation", GlobalAction::PrevPane));

        let mut shortcuts = GlobalAction::ALL
            .iter()
            .copied()
            .filter(|action| !matches!(action, GlobalAction::NextPane | GlobalAction::PrevPane))
            .map(|action| self.framework_global_shortcut_row("Global Shortcuts", action))
            .collect::<Vec<_>>();
        if let Some(render) = self.app_globals_shortcut_rows_fn {
            shortcuts.extend(render(self));
        }
        shortcuts.sort_by_key(|row| row.description);
        rows.extend(shortcuts);
        rows
    }

    fn framework_global_shortcut_row(
        &self,
        section: &'static str,
        action: GlobalAction,
    ) -> GlobalShortcutRow {
        GlobalShortcutRow {
            section,
            description: action.description(),
            key: self.framework_globals.key_for(action).cloned(),
        }
    }

    /// TOML scope name registered for `app_pane_id`, or `None` when no
    /// scope is registered. Mirrors the inverse of
    /// [`Self::insert_scope`].
    fn scope_toml_name_for(&self, app_pane_id: Ctx::AppPaneId) -> Option<&'static str> {
        self.scopes
            .get(&app_pane_id)
            .map(|scope| scope.scope_name())
    }

    /// The registered navigation scope ([`NavAction`] → [`KeyBind`]).
    ///
    /// Returns `None` when [`KeymapBuilder::register_navigation`] was
    /// not called. The builder rejects missing-navigation builds with
    /// [`KeymapError::NavigationMissing`] for any non-empty pane set, so
    /// production callers can rely on `Some(_)`. The action set is
    /// framework-owned, so the map is stored and returned with its
    /// concrete type — a stored-vs-read type mismatch is unrepresentable.
    #[must_use]
    pub const fn navigation(&self) -> Option<&ScopeMap<NavAction>> { self.navigation.as_ref() }

    /// Typed singleton getter for the registered [`Globals`] impl.
    ///
    /// Returns `None` for the same reasons as [`Self::navigation`].
    #[must_use]
    pub fn globals<G: Globals<Ctx>>(&self) -> Option<&ScopeMap<G::Actions>> {
        self.globals
            .as_ref()
            .and_then(|stored| stored.downcast_ref::<ScopeMap<G::Actions>>())
    }

    /// Keymap-help-overlay rows for every registered scope, in the
    /// order the help panel renders them: framework Global Navigation,
    /// framework Global Shortcuts (with app-globals appended), the
    /// registered navigation scope, every app-pane scope in
    /// `app_pane_order`, then the framework overlay scope. Each scope
    /// emits one [`KeymapHelpRow::header`] followed by its action
    /// rows.
    ///
    /// `app_pane_order` controls display order across registered pane
    /// scopes; entries naming an unregistered pane are silently
    /// skipped.
    #[must_use]
    pub fn keymap_help_rows(&self, app_pane_order: &[Ctx::AppPaneId]) -> Vec<KeymapHelpRow> {
        let mut rows = Vec::new();

        rows.push(KeymapHelpRow::header("Global Navigation", "global"));
        for action in [GlobalAction::NextPane, GlobalAction::PrevPane] {
            rows.push(framework_global_help_row("Global Navigation", action, self));
        }

        rows.push(KeymapHelpRow::header("Global Shortcuts", "global"));
        for action in GlobalAction::ALL
            .iter()
            .copied()
            .filter(|a| !matches!(a, GlobalAction::NextPane | GlobalAction::PrevPane))
        {
            rows.push(framework_global_help_row("Global Shortcuts", action, self));
        }
        if let Some(render) = self.app_globals_help_rows_fn {
            rows.extend(render(self));
        }

        if let Some(render) = self.navigation_help_rows_fn {
            rows.extend(render(self));
        }

        for id in app_pane_order {
            if let Some(scope) = self.scopes.get(id) {
                rows.extend(scope.help_rows());
            }
        }

        rows.push(KeymapHelpRow::header("Overlay", "overlay"));
        for action in OverlayAction::ALL.iter().copied() {
            rows.push(KeymapHelpRow {
                section:     "Overlay",
                scope:       "overlay",
                action:      action.toml_key(),
                description: action.description(),
                bind:        self.overlay_scope.display_keys_for(action).first().cloned(),
                is_header:   false,
            });
        }

        rows
    }

    /// TOML action keys for every registered scope. Used by the
    /// keymap TOML writer to enumerate every action regardless of
    /// whether a binding exists. Keyed by scope name in the same
    /// order [`Self::keymap_help_rows`] emits scope headers.
    #[must_use]
    pub fn keymap_toml_scope_keys(
        &self,
        app_pane_order: &[Ctx::AppPaneId],
    ) -> Vec<(&'static str, Vec<&'static str>)> {
        let mut out: Vec<(&'static str, Vec<&'static str>)> = Vec::new();

        // The "global" section combines framework globals and app
        // globals. The keymap writer emits them under one [global]
        // table, so list both action-key collections.
        let mut global_keys: Vec<&'static str> =
            GlobalAction::ALL.iter().map(|a| a.toml_key()).collect();
        if let Some(collect) = self.app_globals_toml_keys_fn {
            global_keys.extend(collect(self));
        }
        out.push(("global", global_keys));

        if let Some(collect) = self.navigation_toml_keys_fn {
            out.push(("navigation", collect(self)));
        }

        for id in app_pane_order {
            if let Some(scope) = self.scopes.get(id)
                && let Some(name) = self.scope_toml_name_for(*id)
            {
                out.push((name, scope.toml_action_keys()));
            }
        }

        let overlay_keys: Vec<&'static str> =
            OverlayAction::ALL.iter().map(|a| a.toml_key()).collect();
        out.push(("overlay", overlay_keys));

        out
    }

    /// `pub(crate)` so [`crate::framework::dispatch_global`] can read
    /// the hook without widening the public surface.
    pub(crate) const fn on_quit_hook(&self) -> Option<fn(&mut Ctx)> { self.on_quit }

    /// `pub(crate)` so [`crate::framework::dispatch_global`] can read
    /// the hook without widening the public surface.
    pub(crate) const fn on_restart_hook(&self) -> Option<fn(&mut Ctx)> { self.on_restart }

    /// `pub(crate)` so [`crate::framework::dispatch_global`] can read
    /// the hook without widening the public surface.
    pub(crate) const fn dismiss_fallback_hook(&self) -> Option<fn(&mut Ctx) -> bool> {
        self.dismiss_fallback
    }

    /// Dispatch one [`GlobalAction`] through the framework's built-in
    /// behavior. The input dispatcher calls this on every
    /// framework-global hit.
    pub fn dispatch_framework_global(&self, action: GlobalAction, ctx: &mut Ctx) {
        framework::dispatch_global(action, self, ctx);
    }
}

/// Build one [`KeymapHelpRow`] for a framework global action. Free
/// fn so [`Keymap::keymap_help_rows`] can iterate without
/// monomorphizing.
fn framework_global_help_row<Ctx: AppContext + 'static>(
    section: &'static str,
    action: GlobalAction,
    keymap: &Keymap<Ctx>,
) -> KeymapHelpRow {
    KeymapHelpRow {
        section,
        scope: "global",
        action: action.toml_key(),
        description: action.description(),
        bind: keymap.framework_globals.key_for(action).cloned(),
        is_header: false,
    }
}

impl<Ctx: AppContext + 'static> core::fmt::Debug for Keymap<Ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Keymap")
            .field("scopes", &self.scopes.len())
            .field("config_path", &self.config_path)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::env;
    use std::fs;
    use std::process;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use crossterm::event::KeyCode;

    use super::Bindings;
    use super::Globals;
    use super::KeyBind;
    use super::KeyOutcome;
    use super::Keymap;
    use super::KeymapBuilder;
    use super::NavAction;
    use super::Navigation;
    use super::Shortcuts;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::Pane;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
        Bar,
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum FooAction {
            Activate => ("activate", "go",    "Activate row");
            Clean    => ("clean",    "clean", "Clean target");
        }
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum AppGlobalAction {
            Find => ("find", "find", "Open find");
        }
    }

    struct TestApp {
        framework: Framework<Self>,
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;
        type ToastAction = crate::NoToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    struct FooPane;

    impl Pane<TestApp> for FooPane {
        const APP_PANE_ID: TestPaneId = TestPaneId::Foo;
    }

    impl Shortcuts<TestApp> for FooPane {
        type Actions = FooAction;

        const SCOPE_NAME: &'static str = "foo";

        fn defaults() -> Bindings<Self::Actions> {
            crate::bindings! {
                KeyCode::Enter => FooAction::Activate,
                'c' => FooAction::Clean,
            }
        }

        fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
            |_action, _ctx| { /* no-op */ }
        }
    }

    struct AppNav;

    impl Navigation<TestApp> for AppNav {
        fn dispatcher() -> fn(NavAction, FocusedPane<TestPaneId>, &mut TestApp) {
            |_action, _focused, _ctx| { /* no-op */ }
        }
    }

    struct AppGlobals;

    impl Globals<TestApp> for AppGlobals {
        type Actions = AppGlobalAction;

        fn render_order() -> &'static [Self::Actions] { &[AppGlobalAction::Find] }

        fn defaults() -> Bindings<Self::Actions> {
            crate::bindings! { 'f' => AppGlobalAction::Find }
        }

        fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
            |_action, _ctx| { /* no-op */ }
        }
    }

    fn fresh_app() -> TestApp {
        TestApp {
            framework: Framework::new(FocusedPane::App(TestPaneId::Foo)),
        }
    }

    fn fresh_builder() -> KeymapBuilder<TestApp, super::builder::Configuring> {
        Keymap::<TestApp>::builder()
            .register_navigation::<AppNav>()
            .expect("nav register must succeed")
            .register_globals::<AppGlobals>()
            .expect("globals register must succeed")
    }

    #[test]
    fn empty_keymap_dispatches_unhandled_for_any_pane() {
        let keymap: Keymap<TestApp> = Keymap::new(None);
        let mut app = fresh_app();
        assert_eq!(
            keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app),
            KeyOutcome::Unhandled,
        );
        assert!(
            keymap
                .render_app_pane_bar_slots(TestPaneId::Foo, &fresh_app())
                .is_empty(),
        );
        assert!(
            keymap
                .key_for_toml_key(TestPaneId::Foo, "activate")
                .is_none()
        );
        assert!(keymap.config_path().is_none());
    }

    #[test]
    fn registered_scope_dispatches_keys() {
        let keymap = fresh_builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");

        let mut app = fresh_app();
        let outcome = keymap.dispatch_app_pane(TestPaneId::Foo, &KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Consumed);
    }

    #[test]
    fn unregistered_pane_id_returns_unhandled() {
        let keymap = fresh_builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");

        let mut app = fresh_app();
        let outcome = keymap.dispatch_app_pane(TestPaneId::Bar, &KeyCode::Enter.into(), &mut app);
        assert_eq!(outcome, KeyOutcome::Unhandled);
    }

    #[test]
    fn render_app_pane_bar_slots_resolves_through_keymap() {
        let keymap = fresh_builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");
        let slots = keymap.render_app_pane_bar_slots(TestPaneId::Foo, &fresh_app());
        let labels: Vec<&'static str> = slots.iter().map(|s| s.label).collect();
        assert_eq!(labels, vec!["go", "clean"]);
    }

    #[test]
    fn render_app_pane_bar_slots_empty_for_unregistered_pane() {
        let keymap = fresh_builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");
        assert!(
            keymap
                .render_app_pane_bar_slots(TestPaneId::Bar, &fresh_app())
                .is_empty(),
        );
    }

    #[test]
    fn key_for_toml_key_round_trips_through_keymap() {
        let keymap = fresh_builder()
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");

        assert_eq!(
            keymap.key_for_toml_key(TestPaneId::Foo, "activate"),
            Some(KeyCode::Enter.into()),
        );
        assert_eq!(
            keymap.key_for_toml_key(TestPaneId::Foo, "clean"),
            Some(KeyBind::from('c').into()),
        );
        assert!(
            keymap
                .key_for_toml_key(TestPaneId::Foo, "frobnicate")
                .is_none()
        );
        assert!(
            keymap
                .key_for_toml_key(TestPaneId::Bar, "activate")
                .is_none()
        );
    }

    #[test]
    fn is_key_bound_to_toml_key_checks_all_bindings() {
        let path = env::temp_dir().join(format!(
            "tui-pane-keymap-{}-{}.toml",
            process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time after epoch")
                .as_nanos(),
        ));
        fs::write(&path, "[foo]\nactivate = [\"Enter\", \"q\"]\n").expect("write keymap toml");

        let keymap = Keymap::<TestApp>::builder()
            .load_toml(path.clone())
            .expect("load toml")
            .register_navigation::<AppNav>()
            .expect("nav register must succeed")
            .register_globals::<AppGlobals>()
            .expect("globals register must succeed")
            .register::<FooPane>(FooPane)
            .build()
            .expect("build keymap with one scope");
        fs::remove_file(path).expect("remove keymap toml");

        assert_eq!(
            keymap.key_for_toml_key(TestPaneId::Foo, "activate"),
            Some(KeyCode::Enter.into()),
        );
        assert!(keymap.is_key_bound_to_toml_key(
            TestPaneId::Foo,
            "activate",
            &KeyCode::Enter.into(),
        ));
        assert!(keymap.is_key_bound_to_toml_key(TestPaneId::Foo, "activate", &KeyBind::from('q'),));
        assert!(
            !keymap.is_key_bound_to_toml_key(TestPaneId::Foo, "activate", &KeyBind::from('c'),)
        );
        assert!(!keymap.is_key_bound_to_toml_key(
            TestPaneId::Foo,
            "frobnicate",
            &KeyBind::from('q'),
        ));
        assert!(
            !keymap.is_key_bound_to_toml_key(TestPaneId::Bar, "activate", &KeyBind::from('q'),)
        );
    }
}
