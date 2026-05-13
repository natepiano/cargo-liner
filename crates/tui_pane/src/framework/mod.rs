//! `Framework<Ctx>`: the framework aggregator owned by every binary
//! that uses `tui_pane`.

mod dispatch;
mod list_navigation;
mod tab_stop;

use std::collections::HashMap;
use std::path::Path;

pub(crate) use self::dispatch::dispatch_global;
pub use self::list_navigation::CycleDirection;
pub use self::list_navigation::ListNavigation;
use self::tab_stop::RegisteredTabStop;
pub use self::tab_stop::TabOrder;
pub use self::tab_stop::TabStop;
use crate::AppContext;
use crate::FocusedPane;
use crate::FrameworkFocusId;
use crate::FrameworkOverlayId;
use crate::KeymapPane;
use crate::LoadedSettings;
use crate::Mode;
use crate::SettingsPane;
use crate::SettingsStore;
use crate::ToastSettings;
use crate::Toasts;
use crate::pane::ModeQuery;

/// The framework aggregator owned by every binary that uses
/// `tui_pane`.
///
/// Tracks the currently focused pane, the open framework overlay (if
/// any), and the lifecycle flags (`quit_requested`, `restart_requested`)
/// that the framework's own dispatch flips when
/// [`GlobalAction::Quit`](crate::GlobalAction::Quit) or
/// [`GlobalAction::Restart`](crate::GlobalAction::Restart) fires. The
/// binary's main loop polls those flags every tick and tears down
/// accordingly.
///
/// The overlay layer ([`Self::overlay`]) and the focus layer
/// ([`Self::focused`]) are orthogonal: opening an overlay does not
/// move focus, and Toasts is reachable through focus without ever
/// being an overlay. The two id enums ([`FrameworkOverlayId`] and
/// [`FrameworkFocusId`]) keep that distinction enforced at the type
/// level.
pub struct Framework<Ctx: AppContext> {
    focused:           FocusedPane<Ctx::AppPaneId>,
    quit_requested:    bool,
    restart_requested: bool,
    mode_queries:      HashMap<Ctx::AppPaneId, ModeQuery<Ctx>>,
    pane_order:        Vec<Ctx::AppPaneId>,
    tab_stops:         Vec<RegisteredTabStop<Ctx>>,
    overlay:           Option<FrameworkOverlayId>,
    /// Keymap viewer/editor overlay, held inline. Reachable when
    /// [`Self::overlay`] is `Some(FrameworkOverlayId::Keymap)`.
    pub keymap_pane:   KeymapPane,
    /// Settings overlay, held inline. Reachable when
    /// [`Self::overlay`] is `Some(FrameworkOverlayId::Settings)`.
    pub settings_pane: SettingsPane,
    settings_store:    SettingsStore,
    toast_settings:    ToastSettings,
    /// Transient notification stack. Tab-focusable when
    /// [`Toasts::has_active`](crate::Toasts::has_active) returns `true`.
    pub toasts:        Toasts<Ctx>,
}

impl<Ctx: AppContext> Framework<Ctx> {
    /// Construct a fresh framework with the given initial focus and
    /// both lifecycle flags cleared.
    #[must_use]
    pub fn new(initial_focus: FocusedPane<Ctx::AppPaneId>) -> Self {
        Self {
            focused:           initial_focus,
            quit_requested:    false,
            restart_requested: false,
            mode_queries:      HashMap::new(),
            pane_order:        Vec::new(),
            tab_stops:         Vec::new(),
            overlay:           None,
            keymap_pane:       KeymapPane::new(),
            settings_pane:     SettingsPane::new(),
            settings_store:    SettingsStore::empty(),
            toast_settings:    ToastSettings::default(),
            toasts:            Toasts::new(),
        }
    }

    /// Construct a framework and install settings loaded through the
    /// framework settings store.
    #[must_use]
    pub fn new_with_settings(
        initial_focus: FocusedPane<Ctx::AppPaneId>,
        loaded_settings: LoadedSettings,
    ) -> Self {
        let mut framework = Self::new(initial_focus);
        framework.install_loaded_settings(loaded_settings);
        framework
    }

    /// Install framework settings loaded from disk.
    pub fn install_loaded_settings(&mut self, loaded_settings: LoadedSettings) {
        self.settings_store = loaded_settings.store;
        self.toast_settings = loaded_settings.toast_settings;
    }

    /// Install the framework-owned settings store.
    pub fn install_settings_store(&mut self, store: SettingsStore) { self.settings_store = store; }

    /// Borrow the framework-owned settings store.
    #[must_use]
    pub const fn settings_store(&self) -> &SettingsStore { &self.settings_store }

    /// Mutably borrow the framework-owned settings store.
    pub const fn settings_store_mut(&mut self) -> &mut SettingsStore { &mut self.settings_store }

    /// Replace framework-owned toast settings.
    pub const fn set_toast_settings(&mut self, settings: ToastSettings) {
        self.toast_settings = settings;
    }

    /// Borrow framework-owned toast settings.
    #[must_use]
    pub const fn toast_settings(&self) -> &ToastSettings { &self.toast_settings }

    /// Mutably borrow framework-owned toast settings.
    pub const fn toast_settings_mut(&mut self) -> &mut ToastSettings { &mut self.toast_settings }

    /// The currently focused pane.
    #[must_use]
    pub const fn focused(&self) -> &FocusedPane<Ctx::AppPaneId> { &self.focused }

    /// Update the currently focused pane.
    pub const fn set_focused(&mut self, focus: FocusedPane<Ctx::AppPaneId>) {
        self.focused = focus;
    }

    /// Whether the user has requested the application quit.
    ///
    /// Set by the framework's own dispatch when
    /// [`GlobalAction::Quit`](crate::GlobalAction::Quit) fires; the
    /// binary's main loop polls this and tears down accordingly.
    #[must_use]
    pub const fn quit_requested(&self) -> bool { self.quit_requested }

    /// Whether the user has requested the application restart.
    ///
    /// Set by the framework's own dispatch when
    /// [`GlobalAction::Restart`](crate::GlobalAction::Restart) fires;
    /// the binary's main loop polls this and tears down accordingly.
    #[must_use]
    pub const fn restart_requested(&self) -> bool { self.restart_requested }

    /// Flip `quit_requested` to `true`. `pub(super)` because only the
    /// framework's built-in [`GlobalAction::Quit`](crate::GlobalAction::Quit)
    /// dispatcher (sibling: `framework/dispatch.rs`) calls it.
    pub(super) const fn request_quit(&mut self) { self.quit_requested = true; }

    /// Flip `restart_requested` to `true`. `pub(super)` for the same
    /// reason as [`Self::request_quit`].
    pub(super) const fn request_restart(&mut self) { self.restart_requested = true; }

    /// Register an app-pane id with the framework. Called once per
    /// `P: Pane<Ctx>` from
    /// [`KeymapBuilder::build_into`](crate::KeymapBuilder::build_into).
    /// `pub(super)` so only the keymap builder (sibling crate module)
    /// can call it.
    pub(super) fn register_app_pane(
        &mut self,
        id: Ctx::AppPaneId,
        mode_query: ModeQuery<Ctx>,
        tab_stop: TabStop<Ctx>,
    ) {
        if self.mode_queries.insert(id, mode_query).is_none() {
            let registration_index = self.pane_order.len();
            self.pane_order.push(id);
            self.tab_stops
                .push(RegisteredTabStop::new(id, registration_index, tab_stop));
        }
    }

    /// Resolved mode for the focused pane.
    ///
    /// Overlay layer wins — when [`Self::overlay`] is `Some`, returns
    /// the overlay pane's mode regardless of [`Self::focused`].
    /// Otherwise dispatches by focus:
    ///
    /// - [`FocusedPane::App`] → looks up the registered mode query.
    /// - [`FocusedPane::Framework(FrameworkFocusId::Toasts)`](crate::FocusedPane) → the toast
    ///   manager's [`Mode`] (`Navigable` in Phase 12+).
    #[must_use]
    pub fn focused_pane_mode(&self, ctx: &Ctx) -> Option<Mode<Ctx>> {
        if let Some(overlay) = self.overlay {
            return Some(match overlay {
                FrameworkOverlayId::Keymap => self.keymap_pane.mode(ctx),
                FrameworkOverlayId::Settings => self.settings_pane.mode(ctx),
            });
        }
        match self.focused {
            FocusedPane::App(id) => self.mode_queries.get(&id).map(|q| q(ctx)),
            FocusedPane::Framework(FrameworkFocusId::Toasts) => Some(self.toasts.mode(ctx)),
        }
    }

    /// Registered app-pane ids in registration order. This remains
    /// registration metadata for diagnostics and tests; focus cycling
    /// uses tab-stop metadata recorded beside this slice.
    #[must_use]
    pub fn pane_order(&self) -> &[Ctx::AppPaneId] { &self.pane_order }

    /// Current focus cycle, filtered by each pane's live tab-stop
    /// predicate.
    fn live_focus_cycle(&self, ctx: &Ctx) -> Vec<FocusedPane<Ctx::AppPaneId>> {
        let mut explicit = Vec::new();
        let mut registration = Vec::new();
        for entry in &self.tab_stops {
            let tab_stop = entry.tab_stop();
            if matches!(tab_stop.order(), TabOrder::Never) || !tab_stop.is_tabbable(ctx) {
                continue;
            }
            match tab_stop.order() {
                TabOrder::Explicit(order) => {
                    explicit.push((order, entry.registration_index(), entry.id()));
                },
                TabOrder::Registration => registration.push(entry.id()),
                TabOrder::Never => {},
            }
        }
        explicit.sort_by_key(|entry| (entry.0, entry.1));
        let mut cycle: Vec<FocusedPane<Ctx::AppPaneId>> = explicit
            .into_iter()
            .map(|entry| FocusedPane::App(entry.2))
            .chain(registration.into_iter().map(FocusedPane::App))
            .collect();
        if self.toasts.has_active() {
            cycle.push(FocusedPane::Framework(FrameworkFocusId::Toasts));
        }
        cycle
    }

    /// The framework overlay currently open over the focused pane, if
    /// any. Overlays are an orthogonal modal layer:
    /// [`Self::focused`] keeps tracking the underlying pane while
    /// `overlay` is `Some(_)`.
    #[must_use]
    pub const fn overlay(&self) -> Option<FrameworkOverlayId> { self.overlay }

    /// Open a framework overlay over the currently focused pane.
    /// `pub(super)` so only the framework's own dispatch (sibling:
    /// `framework/dispatch.rs`) can call it; the binary opens overlays
    /// by firing [`GlobalAction::OpenKeymap`](crate::GlobalAction::OpenKeymap)
    /// or [`GlobalAction::OpenSettings`](crate::GlobalAction::OpenSettings).
    pub(super) const fn open_overlay(&mut self, overlay: FrameworkOverlayId) {
        self.overlay = Some(overlay);
    }

    /// Test-only helper: directly open a framework overlay without
    /// going through the [`GlobalAction`](crate::GlobalAction)
    /// dispatcher. Used by Phase 13 bar snapshot tests so they can
    /// place the framework in a specific overlay state without
    /// synthesizing a key event.
    #[cfg(test)]
    pub(crate) const fn set_overlay_for_test(&mut self, overlay: FrameworkOverlayId) {
        self.overlay = Some(overlay);
    }

    /// Close any open framework overlay. Returns `true` if an overlay
    /// was open and is now cleared, `false` otherwise. `pub(super)`
    /// because the dispatcher routes through [`Self::dismiss_framework`]
    /// rather than calling this directly.
    pub(super) const fn close_overlay(&mut self) -> bool {
        if self.overlay.is_some() {
            self.overlay = None;
            true
        } else {
            false
        }
    }

    /// Run the framework dismiss chain. Returns `true` when something
    /// was dismissed at the framework level. The free
    /// [`dismiss_chain`](crate::framework::dispatch::dismiss_chain)
    /// helper (called from the
    /// [`GlobalAction::Dismiss`](crate::GlobalAction::Dismiss)
    /// dispatcher) consults this; on `false`, it falls through to the
    /// binary's registered `dismiss_fallback` hook (if any).
    ///
    /// Order:
    /// 1. If [`Self::focused`] is the toast stack, pop the focused toast via
    ///    [`Toasts::dismiss_focused`](crate::Toasts::dismiss_focused).
    /// 2. Else if an overlay is open, close it.
    /// 3. Else return `false`.
    pub fn dismiss_framework(&mut self) -> bool {
        if matches!(
            self.focused,
            FocusedPane::Framework(FrameworkFocusId::Toasts)
        ) && self.toasts.dismiss_focused()
        {
            return true;
        }
        self.close_overlay()
    }

    /// File path of the editor currently active on a framework
    /// overlay, if any. Returns the keymap or settings pane's
    /// [`editor_target`](crate::KeymapPane::editor_target) when the
    /// matching overlay is open, `None` otherwise. Drives the binary's
    /// status line during edits.
    #[must_use]
    pub fn editor_target_path(&self) -> Option<&Path> {
        match self.overlay {
            Some(FrameworkOverlayId::Keymap) => self.keymap_pane.editor_target(),
            Some(FrameworkOverlayId::Settings) => self.settings_pane.editor_target(),
            None => None,
        }
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
    use super::Framework;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::FrameworkFocusId;
    use crate::LoadedSettings;
    use crate::SettingsStore;
    use crate::ToastSettings;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
        Bar,
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

    fn fresh_app(initial: FocusedPane<TestPaneId>) -> TestApp {
        TestApp {
            framework: Framework::new(initial),
        }
    }

    #[test]
    fn new_initializes_focus_and_clears_flags() {
        let app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        assert_eq!(
            app.framework().focused(),
            &FocusedPane::App(TestPaneId::Foo)
        );
        assert!(!app.framework().quit_requested());
        assert!(!app.framework().restart_requested());
    }

    #[test]
    fn new_with_settings_installs_loaded_settings() {
        let toast_settings = ToastSettings {
            enabled: false,
            ..ToastSettings::default()
        };

        let framework = Framework::<TestApp>::new_with_settings(
            FocusedPane::App(TestPaneId::Foo),
            LoadedSettings {
                store: SettingsStore::empty(),
                toast_settings,
            },
        );

        assert!(!framework.toast_settings().enabled);
    }

    #[test]
    fn set_focused_updates_focus() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        app.framework_mut()
            .set_focused(FocusedPane::App(TestPaneId::Bar));
        assert_eq!(
            app.framework().focused(),
            &FocusedPane::App(TestPaneId::Bar)
        );

        app.framework_mut()
            .set_focused(FocusedPane::Framework(FrameworkFocusId::Toasts));
        assert_eq!(
            app.framework().focused(),
            &FocusedPane::Framework(FrameworkFocusId::Toasts),
        );
    }

    #[test]
    fn default_set_focus_on_app_context_delegates_to_framework() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));
        assert_eq!(
            app.framework().focused(),
            &FocusedPane::Framework(FrameworkFocusId::Toasts),
        );
    }
}
