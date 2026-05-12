//! `Globals<Ctx>`: app-extension globals scope.
//!
//! One impl per app — the binary defines a zero-sized type and impls
//! this trait for it. Distinct from
//! [`GlobalAction`](crate::GlobalAction): the framework owns its own
//! pane-management / lifecycle globals; this trait is the binary's
//! extension point for app-specific globals (e.g. find, rescan) that
//! share the `[global]` TOML table at load time.

use super::Action;
use super::Bindings;
use crate::AppContext;

/// App-extension globals scope. One impl per app.
///
/// The framework's own pane-management / lifecycle globals live in
/// [`GlobalAction`](crate::GlobalAction) and are not part of this
/// scope. The trait deliberately omits any `bar_label` method — every
/// action enum already provides [`Action::bar_label`], so bar code
/// calls `action.bar_label()` regardless of scope.
pub trait Globals<Ctx: AppContext>: 'static {
    /// The app-globals action enum.
    type Actions: Action;

    /// TOML table name. Defaults to `"global"` so app and framework
    /// globals share one table at load time.
    const SCOPE_NAME: &'static str = "global";

    /// Bar render order for the global region. The bar walks this
    /// slice in order, emitting one slot per variant.
    fn render_order() -> &'static [Self::Actions];

    /// Default keybindings.
    fn defaults() -> Bindings<Self::Actions>;

    /// Free fn the framework calls when an app-globals action fires.
    fn dispatcher() -> fn(Self::Actions, &mut Ctx);
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

    use super::Globals;
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
        pub enum AppGlobalAction {
            Find   => ("find",   "find",   "Open the finder");
            Rescan => ("rescan", "rescan", "Rescan workspaces");
        }
    }

    struct TestApp {
        framework:    Framework<Self>,
        app_settings: (),
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;
        type AppSettings = ();
        type ToastAction = crate::NoToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
        fn app_settings(&self) -> &Self::AppSettings { &self.app_settings }
        fn app_settings_mut(&mut self) -> &mut Self::AppSettings { &mut self.app_settings }
    }

    struct AppGlobals;

    impl Globals<TestApp> for AppGlobals {
        type Actions = AppGlobalAction;

        fn render_order() -> &'static [Self::Actions] {
            &[AppGlobalAction::Find, AppGlobalAction::Rescan]
        }

        fn defaults() -> Bindings<Self::Actions> {
            crate::bindings! {
                'f' => AppGlobalAction::Find,
                KeyCode::F(5) => AppGlobalAction::Rescan,
            }
        }

        fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
            |_action, _ctx| { /* no-op for the test impl */ }
        }
    }

    #[test]
    fn default_scope_name_is_global() {
        assert_eq!(<AppGlobals as Globals<TestApp>>::SCOPE_NAME, "global");
    }

    #[test]
    fn render_order_matches_declaration() {
        assert_eq!(
            AppGlobals::render_order(),
            &[AppGlobalAction::Find, AppGlobalAction::Rescan],
        );
    }

    #[test]
    fn defaults_round_trip_through_scope_map() {
        let map = AppGlobals::defaults().into_scope_map();
        assert_eq!(map.action_for(&'f'.into()), Some(AppGlobalAction::Find));
        assert_eq!(
            map.action_for(&KeyCode::F(5).into()),
            Some(AppGlobalAction::Rescan),
        );
    }

    #[test]
    fn dispatcher_is_callable() {
        let mut app = TestApp {
            framework:    Framework::new(FocusedPane::App(TestPaneId::Foo)),
            app_settings: (),
        };
        let dispatch: fn(AppGlobalAction, &mut TestApp) = AppGlobals::dispatcher();
        dispatch(AppGlobalAction::Find, &mut app);
    }
}
