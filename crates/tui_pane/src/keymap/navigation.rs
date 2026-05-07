//! `Navigation<Ctx>`: app-wide navigation scope.
//!
//! One impl per app — the binary defines a zero-sized type and impls
//! this trait for it. The framework dispatcher routes navigation
//! actions to whichever pane is focused; `Ctx::AppPaneId` lives in the
//! signature so the dispatcher can pick the correct surface.

use super::Action;
use super::Bindings;
use crate::AppContext;
use crate::FocusedPane;

/// App-wide navigation scope. One impl per app.
///
/// Carries four canonical directional variants ([`Self::UP`],
/// [`Self::DOWN`], [`Self::LEFT`], [`Self::RIGHT`]) so the framework
/// can name them without knowing the app's concrete enum. The
/// dispatcher receives the current [`FocusedPane`] so it can route to
/// the right scrollable surface.
///
/// `'static` is implied by the [`Action`] super-trait on
/// [`Self::Actions`].
pub trait Navigation<Ctx: AppContext>: 'static {
    /// The app's navigation-action enum.
    type Actions: Action;

    /// TOML table name. Defaults to `"navigation"` — apps rarely
    /// override.
    const SCOPE_NAME: &'static str = "navigation";

    /// The variant for "move up".
    const UP: Self::Actions;
    /// The variant for "move down".
    const DOWN: Self::Actions;
    /// The variant for "move left".
    const LEFT: Self::Actions;
    /// The variant for "move right".
    const RIGHT: Self::Actions;

    /// Default keybindings.
    fn defaults() -> Bindings<Self::Actions>;

    /// Free fn the framework calls when any navigation action fires.
    /// `focused` lets the dispatcher pick the right scrollable surface;
    /// callers read `ctx.framework().focused()` and pass it through.
    fn dispatcher() -> fn(Self::Actions, focused: FocusedPane<Ctx::AppPaneId>, ctx: &mut Ctx);
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crossterm::event::KeyCode;

    use super::Navigation;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::keymap::Bindings;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum NavAction {
            Up    => ("up",    "up",    "Move up");
            Down  => ("down",  "down",  "Move down");
            Left  => ("left",  "left",  "Move left");
            Right => ("right", "right", "Move right");
        }
    }

    struct TestApp {
        framework: Framework<Self>,
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    struct AppNavigation;

    impl Navigation<TestApp> for AppNavigation {
        type Actions = NavAction;

        const DOWN: Self::Actions = NavAction::Down;
        const LEFT: Self::Actions = NavAction::Left;
        const RIGHT: Self::Actions = NavAction::Right;
        const UP: Self::Actions = NavAction::Up;

        fn defaults() -> Bindings<Self::Actions> {
            crate::bindings! {
                KeyCode::Up    => NavAction::Up,
                KeyCode::Down  => NavAction::Down,
                KeyCode::Left  => NavAction::Left,
                KeyCode::Right => NavAction::Right,
            }
        }

        fn dispatcher() -> fn(Self::Actions, FocusedPane<TestPaneId>, &mut TestApp) {
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
    fn directional_consts_are_distinct() {
        assert_eq!(<AppNavigation as Navigation<TestApp>>::UP, NavAction::Up);
        assert_eq!(
            <AppNavigation as Navigation<TestApp>>::DOWN,
            NavAction::Down
        );
        assert_eq!(
            <AppNavigation as Navigation<TestApp>>::LEFT,
            NavAction::Left
        );
        assert_eq!(
            <AppNavigation as Navigation<TestApp>>::RIGHT,
            NavAction::Right
        );
    }

    #[test]
    fn defaults_round_trip_through_scope_map() {
        let map = AppNavigation::defaults().into_scope_map();
        assert_eq!(map.action_for(&KeyCode::Up.into()), Some(NavAction::Up));
        assert_eq!(map.action_for(&KeyCode::Down.into()), Some(NavAction::Down));
        assert_eq!(map.action_for(&KeyCode::Left.into()), Some(NavAction::Left));
        assert_eq!(
            map.action_for(&KeyCode::Right.into()),
            Some(NavAction::Right)
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
