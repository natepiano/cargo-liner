//! `Navigation<Ctx>`: app-wide navigation scope.
//!
//! One impl per app — the binary defines a zero-sized type and impls
//! this trait for it. The action set, default keymap, and vim aliases
//! are framework-owned ([`NavAction`](crate::NavAction)); the impl only
//! names the scope, names the keymap-help section, and supplies the
//! dispatcher that routes a resolved [`NavAction`] to whichever pane is
//! focused. `Ctx::AppPaneId` lives in the dispatcher signature so the
//! routing can pick the correct surface.

use crate::AppContext;
use crate::FocusedPane;
use crate::NavAction;

/// App-wide navigation scope. One impl per app.
///
/// The framework owns the navigation action set
/// ([`NavAction`](crate::NavAction)), its default keymap, and the vim
/// letter aliases, so a navigation action with no key and a page key
/// that collapses onto a single-line move are both unrepresentable. The
/// impl supplies only the routing: [`Self::dispatcher`] returns a free
/// fn the framework calls with the resolved [`NavAction`] and the
/// current [`FocusedPane`].
pub trait Navigation<Ctx: AppContext>: 'static {
    /// TOML table name. Defaults to `"navigation"` — apps rarely
    /// override.
    const SCOPE_NAME: &'static str = "navigation";

    /// Human-readable section name for the keymap-overlay help. Empty
    /// default keeps test impls ergonomic; apps that render the help
    /// overlay set this to `"List Navigation"` or similar.
    const SECTION_NAME: &'static str = "";

    /// Free fn the framework calls when any navigation action fires.
    /// `focused` lets the dispatcher pick the right scrollable surface;
    /// callers read `ctx.framework().focused()` and pass it through.
    fn dispatcher() -> fn(NavAction, focused: FocusedPane<Ctx::AppPaneId>, ctx: &mut Ctx);
}

#[cfg(test)]
mod tests {
    use super::Navigation;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::NavAction;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
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

    struct AppNavigation;

    impl Navigation<TestApp> for AppNavigation {
        fn dispatcher() -> fn(NavAction, FocusedPane<TestPaneId>, &mut TestApp) {
            |_action, _focused, _ctx| { /* no-op for the test impl */ }
        }
    }

    #[test]
    fn default_scope_name_is_navigation() {
        assert_eq!(
            <AppNavigation as Navigation<TestApp>>::SCOPE_NAME,
            "navigation"
        );
    }

    #[test]
    fn dispatcher_is_callable() {
        let mut app = TestApp {
            framework: Framework::new(FocusedPane::App(TestPaneId::Foo)),
        };
        let dispatch: fn(NavAction, FocusedPane<TestPaneId>, &mut TestApp) =
            AppNavigation::dispatcher();
        dispatch(NavAction::Up, FocusedPane::App(TestPaneId::Foo), &mut app);
    }
}
