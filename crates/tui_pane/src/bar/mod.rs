//! Bar primitives + framework bar renderer.
//!
//! The `bar` module owns the public bar surface: leaf primitives
//! ([`BarRegion`], [`BarSlot`], [`ShortcutState`], [`Visibility`])
//! plus [`render`] / [`StatusBar`] — the renderer that the binary
//! drives once per frame.
//!
//! The renderer's contract:
//!
//! 1. Resolve `pane_slots: Vec<RenderedSlot>` for the focused pane. Overlay-first dispatch (Keymap
//!    / Settings overlays read `framework.{keymap,settings}_pane.bar_slots()`); else
//!    [`FocusedPane::App(id)`](crate::FocusedPane::App) flows through
//!    [`Keymap::render_app_pane_bar_slots`](crate::Keymap::render_app_pane_bar_slots); else
//!    [`FocusedPane::Framework(FrameworkFocusId::Toasts)`](crate::FocusedPane::Framework) reads
//!    from `framework.toasts.bar_slots(ctx)`.
//! 2. Walk [`BarRegion::ALL`](crate::BarRegion::ALL) and dispatch to each region module. Each
//!    module owns its own suppression rule based on
//!    [`Framework::focused_pane_mode`](crate::Framework::focused_pane_mode).
//! 3. Concatenate the per-region span vectors into one [`StatusBar`].

mod constants;
mod global_region;
mod nav_region;
mod palette;
mod pane_action_region;
mod region;
mod slot;
mod status_bar;
mod status_line;
mod support;
mod visibility;

pub use palette::BarPalette;
pub use region::BarRegion;
pub use slot::BarSlot;
pub use slot::ShortcutState;
pub use status_bar::StatusBar;
pub use status_line::StatusLine;
pub use status_line::StatusLineGlobal;
pub use status_line::render as render_status_line;
pub use status_line::status_line_global_spans;
pub use visibility::Visibility;

use crate::Action;
use crate::AppContext;
use crate::FocusedPane;
use crate::Framework;
use crate::FrameworkFocusId;
use crate::FrameworkOverlayId;
use crate::Keymap;
use crate::ScopeMap;
use crate::ShortcutState as ShortcutStateAlias;
use crate::Toasts;
use crate::Visibility as VisibilityAlias;
use crate::keymap::RenderedSlot;

/// Resolve the framework's bar for the current frame.
///
/// `focused` is the framework's current focus (overlay open or not),
/// `ctx` is the binary's app state, `keymap` is the live keymap, and
/// `framework` is the framework aggregator. Returns one
/// [`StatusBar`] value the binary draws to its status-line area.
///
/// Mode suppression rules:
///
/// - [`Mode::Static`](crate::Mode::Static) — `Nav` suppressed; `PaneAction` and `Global` render.
/// - [`Mode::Navigable`](crate::Mode::Navigable) — every region renders.
/// - [`Mode::TextInput`](crate::Mode::TextInput) — every region suppressed (the embedded handler
///   owns the keys; advertising globals here would lie about reachability).
/// - `None` (no pane registered for the focused id) — every region suppressed.
#[must_use]
pub fn render<Ctx: AppContext + 'static>(
    focused: &FocusedPane<Ctx::AppPaneId>,
    ctx: &Ctx,
    keymap: &Keymap<Ctx>,
    framework: &Framework<Ctx>,
    palette: &BarPalette,
) -> StatusBar {
    let pane_slots = pane_slots_for(focused, ctx, keymap, framework);
    let mode = framework.focused_pane_mode(ctx);

    let mut bar = StatusBar::empty();
    for region in BarRegion::ALL {
        match region {
            BarRegion::Nav => {
                bar.nav = nav_region::render::<Ctx>(mode.as_ref(), keymap, &pane_slots, palette);
            },
            BarRegion::PaneAction => {
                bar.pane_action =
                    pane_action_region::render::<Ctx>(mode.as_ref(), &pane_slots, palette);
            },
            BarRegion::Global => {
                bar.global = global_region::render::<Ctx>(mode.as_ref(), keymap, palette);
            },
        }
    }
    bar
}

/// Materialize the focused pane's bar slots, resolved to
/// `Vec<RenderedSlot>`. Overlay-first; otherwise dispatch by
/// [`FocusedPane`].
fn pane_slots_for<Ctx: AppContext + 'static>(
    focused: &FocusedPane<Ctx::AppPaneId>,
    ctx: &Ctx,
    keymap: &Keymap<Ctx>,
    framework: &Framework<Ctx>,
) -> Vec<RenderedSlot> {
    if let Some(overlay) = framework.overlay() {
        let scope = keymap.overlay();
        return match overlay {
            FrameworkOverlayId::Keymap => {
                render_overlay_slots(framework.keymap_pane.bar_slots(), scope)
            },
            FrameworkOverlayId::Settings => {
                render_overlay_slots(framework.settings_pane.bar_slots(), scope)
            },
            FrameworkOverlayId::GlobalShortcuts => {
                render_overlay_slots(framework.global_shortcuts_pane.bar_slots(), scope)
            },
        };
    }
    match focused {
        FocusedPane::App(id) => keymap.render_app_pane_bar_slots(*id, ctx),
        FocusedPane::Framework(FrameworkFocusId::Toasts) => {
            let scope = Toasts::<Ctx>::defaults().into_scope_map();
            render_overlay_slots(framework.toasts.bar_slots(ctx), &scope)
        },
    }
}

fn render_overlay_slots<A: Action>(
    slots: Vec<(BarRegion, crate::BarSlot<A>)>,
    scope: &ScopeMap<A>,
) -> Vec<RenderedSlot> {
    slots
        .into_iter()
        .filter_map(|(region, slot)| match slot {
            BarSlot::Single(action) => {
                let key = scope.key_for(action).cloned()?;
                Some(RenderedSlot {
                    region,
                    label: action.bar_label(),
                    key,
                    shortcut_state: ShortcutStateAlias::Enabled,
                    visibility: VisibilityAlias::Visible,
                    secondary_key: None,
                })
            },
            BarSlot::Paired(primary, secondary, label) => {
                let key = scope.key_for(primary).cloned()?;
                let secondary_key = scope.key_for(secondary).cloned()?;
                Some(RenderedSlot {
                    region,
                    label,
                    key,
                    shortcut_state: ShortcutStateAlias::Enabled,
                    visibility: VisibilityAlias::Visible,
                    secondary_key: Some(secondary_key),
                })
            },
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    //! Framework-pane bar snapshot tests.
    //!
    //! Covers the framework's overlay panes (Keymap / Settings) in every
    //! `EditState` reachable through the in-crate test scaffolding plus
    //! focused-Toasts.

    use std::path::PathBuf;

    use crossterm::event::KeyCode;
    use ratatui::text::Span;

    use super::BarPalette;
    use super::StatusBar;
    use super::render as render_inner;
    use crate::AppContext;
    use crate::Bindings;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::FrameworkFocusId;
    use crate::FrameworkOverlayId;
    use crate::Globals;
    use crate::KeyBind;
    use crate::KeymapPane;
    use crate::Mode;
    use crate::NavAction;
    use crate::Navigation;
    use crate::Pane;
    use crate::SettingsPane;
    use crate::Shortcuts;
    use crate::keymap::Keymap;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
        Bar,
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum FooAction {
            Activate => ("activate", "go",     "Activate row");
            Clean    => ("clean",    "clean",  "Clean target");
        }
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum AppGlobalAction {
            Find => ("find", "find", "Open finder");
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

    /// Static-mode pane: returns [`Mode::Static`] from `Pane::mode()`.
    /// Used to assert that `Nav` is suppressed but `PaneAction` and
    /// `Global` still render.
    struct StaticPane;

    impl Pane<TestApp> for StaticPane {
        const APP_PANE_ID: TestPaneId = TestPaneId::Bar;

        fn mode() -> fn(&TestApp) -> Mode<TestApp> { |_ctx| Mode::Static }
    }

    impl Shortcuts<TestApp> for StaticPane {
        type Actions = FooAction;

        const SCOPE_NAME: &'static str = "bar";

        fn defaults() -> Bindings<Self::Actions> {
            crate::bindings! { 'a' => FooAction::Activate }
        }

        fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
            |_action, _ctx| { /* no-op */ }
        }
    }

    /// TextInput-mode pane: returns [`Mode::TextInput(...)`]. Used to
    /// assert that every region is suppressed.
    struct TextInputPane;

    impl Pane<TestApp> for TextInputPane {
        const APP_PANE_ID: TestPaneId = TestPaneId::Bar;

        fn mode() -> fn(&TestApp) -> Mode<TestApp> { |_ctx| Mode::TextInput(text_input_handler) }
    }

    const fn text_input_handler(_: KeyBind, _: &mut TestApp) {}

    impl Shortcuts<TestApp> for TextInputPane {
        type Actions = FooAction;

        const SCOPE_NAME: &'static str = "bar";

        fn defaults() -> Bindings<Self::Actions> {
            crate::bindings! { KeyCode::Esc => FooAction::Clean }
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

    fn fresh_app(initial: FocusedPane<TestPaneId>) -> TestApp {
        TestApp {
            framework: Framework::new(initial),
        }
    }

    /// Build a keymap that registers `FooPane` (Navigable), navigation,
    /// and globals. Uses `build_into` so the framework's per-pane mode
    /// registry is populated.
    fn build_keymap_with_foo(framework: &mut Framework<TestApp>) -> Keymap<TestApp> {
        Keymap::<TestApp>::builder()
            .register_navigation::<AppNav>()
            .expect("register_navigation")
            .register_globals::<AppGlobals>()
            .expect("register_globals")
            .register::<FooPane>(FooPane)
            .build_into(framework)
            .expect("build_into")
    }

    /// Same as [`build_keymap_with_foo`] but registers a static-mode pane
    /// as the second registered pane.
    fn build_keymap_with_foo_and_static(framework: &mut Framework<TestApp>) -> Keymap<TestApp> {
        Keymap::<TestApp>::builder()
            .register_navigation::<AppNav>()
            .expect("register_navigation")
            .register_globals::<AppGlobals>()
            .expect("register_globals")
            .register::<FooPane>(FooPane)
            .register::<StaticPane>(StaticPane)
            .build_into(framework)
            .expect("build_into")
    }

    /// Same as [`build_keymap_with_foo`] but registers a text-input-mode
    /// pane.
    fn build_keymap_with_foo_and_textinput(framework: &mut Framework<TestApp>) -> Keymap<TestApp> {
        Keymap::<TestApp>::builder()
            .register_navigation::<AppNav>()
            .expect("register_navigation")
            .register_globals::<AppGlobals>()
            .expect("register_globals")
            .register::<FooPane>(FooPane)
            .register::<TextInputPane>(TextInputPane)
            .build_into(framework)
            .expect("build_into")
    }

    /// Test-only wrapper that drives [`render`](super::render) with the
    /// theme-neutral [`BarPalette::default`]. In-crate tests assert on
    /// `Span::content` and don't inspect styling — the default palette
    /// keeps every span unstyled.
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "matches `render_inner`'s signature 1:1; rewriting call sites \
                  would mean `&FocusedPane::App(...)` becomes `FocusedPane::App(...)` \
                  everywhere in the file"
    )]
    fn render(
        focused: &FocusedPane<TestPaneId>,
        ctx: &TestApp,
        keymap: &Keymap<TestApp>,
        framework: &Framework<TestApp>,
    ) -> StatusBar {
        let palette = BarPalette::default();
        render_inner(focused, ctx, keymap, framework, &palette)
    }

    /// Render every `Span`'s content into one string, with no styling.
    fn flatten(spans: &[Span<'static>]) -> String {
        let mut s = String::new();
        for span in spans {
            s.push_str(&span.content);
        }
        s
    }

    fn flatten_bar(bar: &StatusBar) -> (String, String, String) {
        (
            flatten(&bar.nav),
            flatten(&bar.pane_action),
            flatten(&bar.global),
        )
    }

    // ── Per-region rule fixture ──────────────────────────────────────────

    #[test]
    fn navigable_mode_renders_every_region() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        let keymap = build_keymap_with_foo(&mut app.framework);
        let bar = render(
            &FocusedPane::App(TestPaneId::Foo),
            &app,
            &keymap,
            app.framework(),
        );
        let (nav, pane_action, global) = flatten_bar(&bar);
        assert!(!nav.is_empty(), "Nav region must render in Navigable mode");
        assert!(
            !pane_action.is_empty(),
            "PaneAction region must render in Navigable mode",
        );
        assert!(
            !global.is_empty(),
            "Global region must render in Navigable mode",
        );
    }

    #[test]
    fn static_mode_suppresses_nav_only() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Bar));
        let keymap = build_keymap_with_foo_and_static(&mut app.framework);
        let bar = render(
            &FocusedPane::App(TestPaneId::Bar),
            &app,
            &keymap,
            app.framework(),
        );
        let (nav, pane_action, global) = flatten_bar(&bar);
        assert!(nav.is_empty(), "Nav must be suppressed in Static mode");
        assert!(!pane_action.is_empty(), "PaneAction renders in Static mode");
        assert!(!global.is_empty(), "Global renders in Static mode");
    }

    #[test]
    fn textinput_mode_suppresses_every_region() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Bar));
        let keymap = build_keymap_with_foo_and_textinput(&mut app.framework);
        let bar = render(
            &FocusedPane::App(TestPaneId::Bar),
            &app,
            &keymap,
            app.framework(),
        );
        assert!(
            bar.is_empty(),
            "Every region must be suppressed in TextInput mode (got {bar:?})",
        );
    }

    #[test]
    fn unregistered_pane_id_renders_empty_bar() {
        // Nothing registered; the focus is on `Foo` but no scope answers
        // `mode_queries`, so `focused_pane_mode` returns `None`.
        let app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        let keymap: Keymap<TestApp> = Keymap::new(None);
        let bar = render(
            &FocusedPane::App(TestPaneId::Foo),
            &app,
            &keymap,
            app.framework(),
        );
        assert!(bar.is_empty(), "None mode → empty bar (got {bar:?})");
    }

    // ── Framework overlay panes ──────────────────────────────────────────

    #[test]
    fn keymap_browse_renders_pane_actions_and_globals() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        let keymap = build_keymap_with_foo(&mut app.framework);
        app.framework_mut().dispatch_global_for_test_open_keymap();
        let focused = *app.framework().focused();
        let bar = render(&focused, &app, &keymap, app.framework());
        let (nav, pane_action, global) = flatten_bar(&bar);
        // Browse → Mode::Navigable → Nav, PaneAction, Global all render.
        assert!(
            !pane_action.is_empty(),
            "Keymap Browse must show local actions"
        );
        assert!(
            pane_action.contains("edit"),
            "got pane_action={pane_action:?}"
        );
        assert!(
            pane_action.contains("cancel"),
            "got pane_action={pane_action:?}"
        );
        assert!(!global.is_empty(), "Keymap Browse must show globals");
        // Nav region: pane-cycle row from framework globals (NextPane/PrevPane)
        // — even though no app pane is in focus, the cycle row resolves
        // through the framework_globals scope.
        assert!(
            !nav.is_empty(),
            "Keymap Browse must show pane-cycle / nav row"
        );
    }

    #[test]
    fn keymap_awaiting_text_input_suppresses_every_region() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        let keymap = build_keymap_with_foo(&mut app.framework);
        app.framework_mut().dispatch_global_for_test_open_keymap();
        // Replace the framework's keymap pane with one in Awaiting.
        app.framework_mut().keymap_pane =
            KeymapPane::for_test_awaiting(Some(PathBuf::from("/tmp/keys.toml")));
        let focused = *app.framework().focused();
        let bar = render(&focused, &app, &keymap, app.framework());
        assert!(
            bar.is_empty(),
            "Keymap Awaiting (TextInput mode) suppresses every region (got {bar:?})",
        );
    }

    #[test]
    fn keymap_conflict_renders_pane_actions_and_globals() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        let keymap = build_keymap_with_foo(&mut app.framework);
        app.framework_mut().dispatch_global_for_test_open_keymap();
        app.framework_mut().keymap_pane =
            KeymapPane::for_test_conflict(Some(PathBuf::from("/tmp/keys.toml")));
        let focused = *app.framework().focused();
        let bar = render(&focused, &app, &keymap, app.framework());
        let (nav, pane_action, global) = flatten_bar(&bar);
        // Conflict → Mode::Static → Nav suppressed; PaneAction + Global render.
        assert!(
            nav.is_empty(),
            "Conflict mode suppresses Nav (got nav={nav:?})"
        );
        assert!(
            !pane_action.is_empty(),
            "Conflict shows local Cancel (got pane_action={pane_action:?})",
        );
        assert!(
            !global.is_empty(),
            "Conflict shows globals (got global={global:?})"
        );
    }

    #[test]
    fn settings_browse_renders_pane_actions_and_globals() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        let keymap = build_keymap_with_foo(&mut app.framework);
        app.framework_mut().dispatch_global_for_test_open_settings();
        let focused = *app.framework().focused();
        let bar = render(&focused, &app, &keymap, app.framework());
        let (_, pane_action, global) = flatten_bar(&bar);
        assert!(pane_action.contains("edit"));
        assert!(pane_action.contains("cancel"));
        assert!(!global.is_empty());
    }

    #[test]
    fn settings_editing_text_input_suppresses_every_region() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        let keymap = build_keymap_with_foo(&mut app.framework);
        app.framework_mut().dispatch_global_for_test_open_settings();
        app.framework_mut().settings_pane =
            SettingsPane::for_test_editing(Some(PathBuf::from("/tmp/settings.toml")));
        let focused = *app.framework().focused();
        let bar = render(&focused, &app, &keymap, app.framework());
        assert!(
            bar.is_empty(),
            "Settings Editing (TextInput mode) suppresses every region (got {bar:?})",
        );
    }

    // ── Focused Toasts ───────────────────────────────────────────────────

    #[test]
    fn focused_toasts_renders_nav_pane_actions_and_globals() {
        let mut app = fresh_app(FocusedPane::Framework(FrameworkFocusId::Toasts));
        let keymap = build_keymap_with_foo(&mut app.framework);
        let _ = app.framework_mut().toasts.push("Build done", "ok");
        let focused = FocusedPane::Framework(FrameworkFocusId::Toasts);
        let bar = render(&focused, &app, &keymap, app.framework());
        let (nav, pane_action, global) = flatten_bar(&bar);
        assert!(!nav.is_empty(), "Toasts must show nav (got nav={nav:?})");
        assert!(
            pane_action.contains("enter open"),
            "Toasts must show activate pane action (got pane_action={pane_action:?})",
        );
        assert!(
            !global.is_empty(),
            "Toasts must show globals (got global={global:?})"
        );
        // The focused-Toasts dismiss path goes through `GlobalAction::Dismiss`,
        // which renders by its bar_label (`dismiss`).
        assert!(global.contains("dismiss"), "got global={global:?}");
    }

    #[test]
    fn nav_row_renders_resolved_keys_via_display_short() {
        let mut app = fresh_app(FocusedPane::App(TestPaneId::Foo));
        let keymap = build_keymap_with_foo(&mut app.framework);
        let bar = render(
            &FocusedPane::App(TestPaneId::Foo),
            &app,
            &keymap,
            app.framework(),
        );
        let nav = flatten(&bar.nav);
        // Default nav binds Up/Down to KeyCode::Up/Down → renders as ↑ /
        // ↓ via `display_short`. The pane-cycle row uses Tab.
        assert!(
            nav.contains('↑'),
            "default nav must show ↑ glyph (got {nav:?})"
        );
        assert!(
            nav.contains('↓'),
            "default nav must show ↓ glyph (got {nav:?})"
        );
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    trait FrameworkTestHelpers {
        fn dispatch_global_for_test_open_keymap(&mut self);
        fn dispatch_global_for_test_open_settings(&mut self);
    }

    impl FrameworkTestHelpers for Framework<TestApp> {
        fn dispatch_global_for_test_open_keymap(&mut self) {
            self.set_overlay_for_test(FrameworkOverlayId::Keymap);
        }

        fn dispatch_global_for_test_open_settings(&mut self) {
            self.set_overlay_for_test(FrameworkOverlayId::Settings);
        }
    }
}
