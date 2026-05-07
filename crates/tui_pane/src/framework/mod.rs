//! `Framework<Ctx>`: the framework aggregator owned by every binary
//! that uses `tui_pane`.
//!
//! Phase 6 ships the **skeleton** — three fields and five methods,
//! frozen by the Phase 6 → Phase 10 contract. Phase 10 fills in the
//! framework panes (keymap viewer, settings, toasts), the dismiss
//! chain, and the input-mode plumbing as a purely additive extension;
//! the Phase 6 surface stays verbatim so tests written in Phases 7–9
//! against this skeleton continue to pass.

use crate::AppContext;
use crate::FocusedPane;

/// The framework aggregator owned by every binary that uses
/// `tui_pane`.
///
/// Tracks the currently focused pane and the lifecycle flags
/// (`quit_requested`, `restart_requested`) that the framework's own
/// dispatch flips when [`GlobalAction::Quit`](crate::GlobalAction::Quit)
/// or [`GlobalAction::Restart`](crate::GlobalAction::Restart) fires.
/// The binary's main loop polls those flags every tick and tears down
/// accordingly.
///
/// Phase 10 adds: framework pane fields (`keymap_pane`,
/// `settings_pane`, `toasts`), the `dismiss()` chain, and per-pane
/// input-mode queries.
pub struct Framework<Ctx: AppContext> {
    focused:           FocusedPane<Ctx::AppPaneId>,
    quit_requested:    bool,
    restart_requested: bool,
}

impl<Ctx: AppContext> Framework<Ctx> {
    /// Construct a fresh framework with the given initial focus and
    /// both lifecycle flags cleared.
    #[must_use]
    pub const fn new(initial_focus: FocusedPane<Ctx::AppPaneId>) -> Self {
        Self {
            focused:           initial_focus,
            quit_requested:    false,
            restart_requested: false,
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
