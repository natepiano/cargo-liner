//! `Framework<Ctx>`: the framework aggregator owned by every binary
//! that uses `tui_pane`.

mod dispatch;

use std::collections::HashMap;
use std::path::Path;

pub(crate) use self::dispatch::dispatch_global;
use crate::AppContext;
use crate::FocusedPane;
use crate::FrameworkPaneId;
use crate::KeymapPane;
use crate::Mode;
use crate::SettingsPane;
use crate::Toasts;

/// `fn` pointer the framework stores per registered pane to query
/// the pane's current input mode.
pub(crate) type ModeQuery<Ctx> = fn(&Ctx) -> Mode<Ctx>;

/// The framework aggregator owned by every binary that uses
/// `tui_pane`.
///
/// Tracks the currently focused pane and the lifecycle flags
/// (`quit_requested`, `restart_requested`) that the framework's own
/// dispatch flips when [`GlobalAction::Quit`](crate::GlobalAction::Quit)
/// or [`GlobalAction::Restart`](crate::GlobalAction::Restart) fires.
/// The binary's main loop polls those flags every tick and tears down
/// accordingly.
pub struct Framework<Ctx: AppContext> {
    focused:           FocusedPane<Ctx::AppPaneId>,
    quit_requested:    bool,
    restart_requested: bool,
    mode_queries:      HashMap<Ctx::AppPaneId, ModeQuery<Ctx>>,
    pane_order:        Vec<Ctx::AppPaneId>,
    overlay:           Option<FrameworkPaneId>,
    /// Keymap viewer/editor overlay, held inline. Reachable when
    /// [`Self::overlay`] is `Some(FrameworkPaneId::Keymap)`.
    pub keymap_pane:   KeymapPane<Ctx>,
    /// Settings overlay, held inline. Reachable when
    /// [`Self::overlay`] is `Some(FrameworkPaneId::Settings)`.
    pub settings_pane: SettingsPane<Ctx>,
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
            overlay:           None,
            keymap_pane:       KeymapPane::new(),
            settings_pane:     SettingsPane::new(),
            toasts:            Toasts::new(),
        }
    }

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
    pub(super) fn register_app_pane(&mut self, id: Ctx::AppPaneId, mode_query: ModeQuery<Ctx>) {
        if self.mode_queries.insert(id, mode_query).is_none() {
            self.pane_order.push(id);
        }
    }

    /// Resolved mode for the focused pane.
    ///
    /// Overlay layer wins — when [`Self::overlay`] is `Some`, returns
    /// the overlay pane's mode regardless of [`Self::focused`].
    /// Otherwise dispatches by focus:
    ///
    /// - [`FocusedPane::App`] → looks up the registered mode query.
    /// - [`FocusedPane::Framework(Toasts)`](crate::FocusedPane) → [`Mode::Static`].
    /// - [`FocusedPane::Framework(Keymap | Settings)`](crate::FocusedPane) is unreachable
    ///   post-overlay-switch (the dispatcher writes only the overlay layer, never these focus
    ///   states); the arm is left in place to keep [`FrameworkPaneId`] unified.
    #[must_use]
    pub fn focused_pane_mode(&self, ctx: &Ctx) -> Option<Mode<Ctx>> {
        if let Some(overlay) = self.overlay {
            return Some(match overlay {
                FrameworkPaneId::Keymap => self.keymap_pane.mode(ctx),
                FrameworkPaneId::Settings => self.settings_pane.mode(ctx),
                FrameworkPaneId::Toasts => self.toasts.mode(ctx),
            });
        }
        match self.focused {
            FocusedPane::App(id) => self.mode_queries.get(&id).map(|q| q(ctx)),
            FocusedPane::Framework(FrameworkPaneId::Toasts) => Some(self.toasts.mode(ctx)),
            // unreachable post-overlay-switch
            FocusedPane::Framework(FrameworkPaneId::Keymap | FrameworkPaneId::Settings) => None,
        }
    }

    /// Registered app-pane ids in registration order. The
    /// [`GlobalAction::NextPane`](crate::GlobalAction::NextPane) /
    /// [`GlobalAction::PrevPane`](crate::GlobalAction::PrevPane)
    /// dispatchers walk this slice; Phase 12's bar renderer and
    /// Phase 18's regression tests also need it through the public
    /// surface, hence the `pub` visibility.
    #[must_use]
    pub fn pane_order(&self) -> &[Ctx::AppPaneId] { &self.pane_order }

    /// The framework overlay currently open over the focused pane, if
    /// any. Overlays are an orthogonal modal layer:
    /// [`Self::focused`] keeps tracking the underlying pane while
    /// `overlay` is `Some(_)`.
    #[must_use]
    pub const fn overlay(&self) -> Option<FrameworkPaneId> { self.overlay }

    /// Open a framework overlay over the currently focused pane.
    /// `pub(super)` so only the framework's own dispatch (sibling:
    /// `framework/dispatch.rs`) can call it; the binary opens overlays
    /// by firing [`GlobalAction::OpenKeymap`](crate::GlobalAction::OpenKeymap)
    /// or [`GlobalAction::OpenSettings`](crate::GlobalAction::OpenSettings).
    pub(super) const fn open_overlay(&mut self, overlay: FrameworkPaneId) {
        self.overlay = Some(overlay);
    }

    /// Close any open framework overlay. Returns `true` if an overlay
    /// was open and is now cleared, `false` otherwise. Phase 11 wraps
    /// this in a full [`Self::dismiss`] chain (toasts → overlay →
    /// fallback); the dispatcher routes through `dismiss` rather than
    /// calling this directly.
    pub(super) const fn close_overlay(&mut self) -> bool {
        if self.overlay.is_some() {
            self.overlay = None;
            true
        } else {
            false
        }
    }

    /// Run the framework dismiss chain. Returns `true` if anything was
    /// dismissed at the framework level. The
    /// [`GlobalAction::Dismiss`](crate::GlobalAction::Dismiss)
    /// dispatcher consults this; on `false`, it falls through to the
    /// binary's registered `dismiss_fallback` hook (if any).
    ///
    /// Order:
    /// 1. If [`Self::focused`] is the toast stack, pop the top toast.
    /// 2. Else if an overlay is open, close it.
    /// 3. Else return `false`.
    pub fn dismiss(&mut self) -> bool {
        if matches!(
            self.focused,
            FocusedPane::Framework(FrameworkPaneId::Toasts)
        ) && self.toasts.try_pop_top()
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
            Some(FrameworkPaneId::Keymap) => self.keymap_pane.editor_target(),
            Some(FrameworkPaneId::Settings) => self.settings_pane.editor_target(),
            Some(FrameworkPaneId::Toasts) | None => None,
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
    use crate::FrameworkPaneId;

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
    fn set_focused_updates_focus() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        app.framework_mut()
            .set_focused(FocusedPane::App(TestPaneId::Bar));
        assert_eq!(
            app.framework().focused(),
            &FocusedPane::App(TestPaneId::Bar)
        );

        app.framework_mut()
            .set_focused(FocusedPane::Framework(FrameworkPaneId::Keymap));
        assert_eq!(
            app.framework().focused(),
            &FocusedPane::Framework(FrameworkPaneId::Keymap),
        );
    }

    #[test]
    fn default_set_focus_on_app_context_delegates_to_framework() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        app.set_focus(FocusedPane::Framework(FrameworkPaneId::Settings));
        assert_eq!(
            app.framework().focused(),
            &FocusedPane::Framework(FrameworkPaneId::Settings),
        );
    }
}
