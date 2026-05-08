//! `Framework<Ctx>`: the framework aggregator owned by every binary
//! that uses `tui_pane`.

mod dispatch;

use std::collections::HashMap;

pub(crate) use self::dispatch::dispatch_global;
use crate::AppContext;
use crate::FocusedPane;
use crate::FrameworkPaneId;
use crate::Mode;

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

    /// Resolved mode for the focused app pane. Returns `None` for
    /// framework-focused panes (callers special-case those) or for
    /// app panes whose id was not registered through
    /// [`KeymapBuilder::build_into`](crate::KeymapBuilder::build_into).
    #[must_use]
    pub fn focused_pane_mode(&self, ctx: &Ctx) -> Option<Mode<Ctx>> {
        match self.focused {
            FocusedPane::App(id) => self.mode_queries.get(&id).map(|q| q(ctx)),
            FocusedPane::Framework(_) => None,
        }
    }

    /// Registered app-pane ids in registration order. The
    /// [`GlobalAction::NextPane`](crate::GlobalAction::NextPane) /
    /// [`GlobalAction::PrevPane`](crate::GlobalAction::PrevPane)
    /// dispatchers walk this slice.
    pub(super) fn pane_order(&self) -> &[Ctx::AppPaneId] { &self.pane_order }

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
    /// this in a full `dismiss()` chain (toasts → overlay → fallback);
    /// the Phase 10 dispatcher uses it directly for the
    /// [`GlobalAction::Dismiss`](crate::GlobalAction::Dismiss) arm.
    pub(super) const fn close_overlay(&mut self) -> bool {
        if self.overlay.is_some() {
            self.overlay = None;
            true
        } else {
            false
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
