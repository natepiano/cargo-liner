//! The key dispatch ladder, and the keys each framework overlay owns.

use std::rc::Rc;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;

use super::app::TerminalApp;
use crate::FrameworkOverlayId;
use crate::GlobalAction;
use crate::Globals;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::Keymap;
use crate::Navigation;
use crate::OverlayAction;
use crate::SettingStep;
use crate::SettingsNavigation;
use crate::matches_open_overlay_toggle;
use crate::overlay_is_in_text_mode;

/// Dispatch one key press through the ladder.
///
/// The app's modal is asked first and owns every key while it is open
/// ([`TerminalApp::modal_key`]). An open framework overlay comes next:
/// the key that opened it closes it, and everything else is the
/// overlay's. Then [`TerminalApp::attract_key`], then the framework
/// globals, then the app's own globals scope.
pub fn dispatch_key<A: TerminalApp>(app: &mut A, key: KeyEvent) {
    // Re-borrowed every key: rebinding a key in the keymap overlay
    // swaps the whole map out from under the dispatch.
    let keymap: Rc<Keymap<A>> = app.keymap();
    let bind = KeyBind::from(key);
    if app.modal_key(&keymap, &bind) == KeyOutcome::Consumed {
        return;
    }
    if let Some(overlay) = app.framework().overlay() {
        let in_text_mode = overlay_is_in_text_mode(app.framework(), overlay);
        if let Some(action) = keymap.framework_globals().action_for(&bind)
            && !in_text_mode
            && matches_open_overlay_toggle(action, overlay)
        {
            keymap.dispatch_framework_global(action, app);
            return;
        }
        dispatch_overlay_key(app, &keymap, overlay, bind);
        return;
    }
    if app.attract_key(&keymap, &bind) == KeyOutcome::Consumed {
        return;
    }
    if let Some(action) = keymap.framework_globals().action_for(&bind) {
        keymap.dispatch_framework_global(action, app);
        return;
    }
    if let Some(action) = keymap
        .globals::<A::AppGlobals>()
        .and_then(|scope| scope.action_for(&bind))
    {
        A::AppGlobals::dispatcher()(action, app);
    }
}

/// Route a key the open overlay owns.
///
/// Capture comes first: while the keymap overlay is waiting for a
/// replacement binding, every key is the candidate rather than a
/// command.
fn dispatch_overlay_key<A: TerminalApp>(
    app: &mut A,
    keymap: &Keymap<A>,
    overlay: FrameworkOverlayId,
    bind: KeyBind,
) {
    if overlay == FrameworkOverlayId::Keymap && app.framework().keymap_pane.is_capturing() {
        let command = app.framework_mut().keymap_pane.handle_capture_key(bind);
        crate::handle_keymap_capture_command(app, keymap, command);
        return;
    }
    match overlay {
        FrameworkOverlayId::Settings => dispatch_settings_key(app, keymap, bind),
        FrameworkOverlayId::Keymap => dispatch_keymap_key(app, keymap, bind),
        FrameworkOverlayId::GlobalShortcuts => dispatch_global_shortcuts_key(app, keymap, bind),
    }
}

/// Keys the keymap overlay owns. The framework runs the whole editor —
/// selection, capture, conflict checks, and the write back to
/// `keymap.toml` — through the app's
/// [`KeymapEditContext`](crate::KeymapEditContext) impl.
fn dispatch_keymap_key<A: TerminalApp>(app: &mut A, keymap: &Keymap<A>, bind: KeyBind) {
    if let Some(action) = keymap.overlay().action_for(&bind) {
        crate::dispatch_keymap_action(action, app, keymap);
        return;
    }
    crate::handle_keymap_navigation_key(app, keymap, bind.code);
}

/// Keys the `?` overlay owns. Editing a row here hands off to the full
/// keymap editor with that row already selected.
fn dispatch_global_shortcuts_key<A: TerminalApp>(app: &mut A, keymap: &Keymap<A>, bind: KeyBind) {
    match keymap.overlay().action_for(&bind) {
        Some(OverlayAction::StartEdit) => crate::edit_selected_global_shortcut(app, keymap),
        Some(OverlayAction::Cancel) => keymap.dispatch_framework_global(GlobalAction::Dismiss, app),
        None => app
            .framework_mut()
            .global_shortcuts_pane
            .handle_navigation_key(bind.code),
    }
}

/// Keys the settings overlay owns: move the selection, step the value.
///
/// Movement comes from the navigation scope, so the keys that walk
/// this list are the ones `keymap.toml` says they are. Enter and space
/// stay here: they are this overlay's own way of saying "the next
/// value", not a direction anyone would rebind.
///
/// Nothing here reaches [`crate::SettingsPane::handle_key`]: that
/// would put the pane into its text-edit state, which the settings
/// overlay has no commit path for.
fn dispatch_settings_key<A: TerminalApp>(app: &mut A, keymap: &Keymap<A>, bind: KeyBind) {
    if keymap.overlay().action_for(&bind) == Some(OverlayAction::Cancel) {
        keymap.dispatch_framework_global(GlobalAction::Dismiss, app);
        return;
    }
    if let Some(action) = keymap
        .navigation()
        .and_then(|scope| scope.action_for(&bind))
    {
        let focused = *app.framework().focused();
        SettingsNavigation::<A>::dispatcher()(action, focused, app);
        return;
    }
    if matches!(bind.code, KeyCode::Enter | KeyCode::Char(' ')) {
        app.step_setting(SettingStep::Next);
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;
    use std::rc::Rc;

    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::Frame;
    use ratatui::layout::Position;

    use super::dispatch_key;
    use crate::AppContext;
    use crate::AppIdentity;
    use crate::Bindings;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::FrameworkOverlayId;
    use crate::Globals;
    use crate::KeyBind;
    use crate::KeyOutcome;
    use crate::Keymap;
    use crate::KeymapEditContext;
    use crate::KeymapUiContext;
    use crate::NoProbe;
    use crate::NoToastAction;
    use crate::SettingStep;
    use crate::SettingsHost;
    use crate::SettingsNavigation;
    use crate::TerminalApp;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Main,
    }

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum TestAppGlobalAction {
            OpenModal => ("open_modal", "modal", "Open the app modal");
            Toggle => ("toggle", "toggle", "Toggle a flag");
        }
    }

    enum TestIdentity {}

    impl AppIdentity for TestIdentity {
        const BINARY_NAME: &'static str = "runner-test";
        const CONFIG_DIRNAME: &'static str = "runner-test";
        const DEFAULT_LIGHT_THEME: &'static str = "light";
        const DEFAULT_DARK_THEME: &'static str = "dark";
    }

    /// An app with one modal of its own, opened by an app global and
    /// closed by Esc, and a flag another app global toggles.
    struct TestApp {
        framework:  Framework<Self>,
        keymap:     Rc<Keymap<Self>>,
        modal_open: bool,
        toggled:    bool,
        steps:      Vec<SettingStep>,
    }

    impl TestApp {
        fn new() -> Self {
            let keymap = Keymap::<Self>::builder()
                .register_navigation::<SettingsNavigation<Self>>()
                .expect("navigation registers")
                .register_globals::<TestAppGlobals>()
                .expect("app globals register")
                .register_overlay()
                .expect("overlay scope registers")
                .build()
                .expect("test keymap builds");
            Self {
                framework:  Framework::new(FocusedPane::App(TestPaneId::Main)),
                keymap:     Rc::new(keymap),
                modal_open: false,
                toggled:    false,
                steps:      Vec::new(),
            }
        }
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;
        type ToastAction = NoToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }

        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    impl KeymapUiContext for TestApp {
        fn keymap_inline_error(&self) -> Option<&str> { None }

        fn keymap_pane_display_order(&self) -> &[TestPaneId] { &[] }
    }

    struct TestAppGlobals;

    impl Globals<TestApp> for TestAppGlobals {
        type Actions = TestAppGlobalAction;

        fn render_order() -> &'static [Self::Actions] {
            &[TestAppGlobalAction::OpenModal, TestAppGlobalAction::Toggle]
        }

        fn defaults() -> Bindings<Self::Actions> {
            crate::bindings! {
                'o' => TestAppGlobalAction::OpenModal,
                'f' => TestAppGlobalAction::Toggle,
            }
        }

        fn dispatcher() -> fn(Self::Actions, &mut TestApp) {
            |action, app| match action {
                TestAppGlobalAction::OpenModal => app.modal_open = true,
                TestAppGlobalAction::Toggle => app.toggled = !app.toggled,
            }
        }
    }

    impl KeymapEditContext for TestApp {
        type AppGlobals = TestAppGlobals;

        const KEYMAP_TOML_HEADER: &'static str = "";

        fn keymap_file_path(&self) -> Option<PathBuf> { None }

        fn set_keymap_inline_error(&mut self, _message: String) {}

        fn clear_keymap_inline_error(&mut self) {}

        fn reload_keymap(&mut self, _content: &str) {}
    }

    impl SettingsHost for TestApp {
        fn step_setting(&mut self, step: SettingStep) { self.steps.push(step); }
    }

    impl TerminalApp for TestApp {
        type Identity = TestIdentity;
        type Probe = NoProbe;

        fn keymap(&self) -> Rc<Keymap<Self>> { Rc::clone(&self.keymap) }

        fn draw(&mut self, _frame: &mut Frame, _keymap: &Keymap<Self>) {}

        fn click(&mut self, _position: Position) {}

        fn modal_key(&mut self, _keymap: &Keymap<Self>, bind: &KeyBind) -> KeyOutcome {
            if !self.modal_open {
                return KeyOutcome::Unhandled;
            }
            if bind.code == KeyCode::Esc {
                self.modal_open = false;
            }
            KeyOutcome::Consumed
        }
    }

    fn key(code: KeyCode) -> KeyEvent { KeyEvent::new(code, KeyModifiers::NONE) }

    #[test]
    fn app_modal_consumes_app_and_framework_globals_until_escape() {
        let mut app = TestApp::new();
        dispatch_key(&mut app, key(KeyCode::Char('o')));
        assert!(app.modal_open);

        dispatch_key(&mut app, key(KeyCode::Char('f')));
        dispatch_key(&mut app, key(KeyCode::Char('?')));
        assert!(!app.toggled);
        assert_eq!(app.framework.overlay(), None);
        assert!(app.modal_open);

        dispatch_key(&mut app, key(KeyCode::Esc));
        assert!(!app.modal_open);
        dispatch_key(&mut app, key(KeyCode::Char('f')));
        assert!(
            app.toggled,
            "with the modal closed the app globals answer again"
        );
    }

    #[test]
    fn x_leaves_each_framework_overlay_open_while_escape_closes_it() {
        let mut app = TestApp::new();
        for (overlay, opener) in [
            (FrameworkOverlayId::Settings, key(KeyCode::Char('s'))),
            (
                FrameworkOverlayId::Keymap,
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            ),
            (FrameworkOverlayId::GlobalShortcuts, key(KeyCode::Char('?'))),
        ] {
            dispatch_key(&mut app, opener);
            assert_eq!(app.framework.overlay(), Some(overlay));
            dispatch_key(&mut app, key(KeyCode::Char('x')));
            assert_eq!(app.framework.overlay(), Some(overlay));
            dispatch_key(&mut app, key(KeyCode::Esc));
            assert_eq!(app.framework.overlay(), None);
        }
    }

    #[test]
    fn framework_overlay_prevents_the_app_modal_from_opening() {
        let mut app = TestApp::new();
        dispatch_key(&mut app, key(KeyCode::Char('s')));
        assert_eq!(app.framework.overlay(), Some(FrameworkOverlayId::Settings));

        dispatch_key(&mut app, key(KeyCode::Char('o')));
        assert_eq!(app.framework.overlay(), Some(FrameworkOverlayId::Settings));
        assert!(!app.modal_open);
    }

    #[test]
    fn settings_overlay_steps_on_enter_space_and_the_sideways_keys() {
        let mut app = TestApp::new();
        dispatch_key(&mut app, key(KeyCode::Char('s')));
        for code in [
            KeyCode::Enter,
            KeyCode::Char(' '),
            KeyCode::Left,
            KeyCode::Right,
        ] {
            dispatch_key(&mut app, key(code));
        }
        assert_eq!(
            app.steps,
            [
                SettingStep::Next,
                SettingStep::Next,
                SettingStep::Prev,
                SettingStep::Next
            ]
        );
        assert_eq!(app.framework.overlay(), Some(FrameworkOverlayId::Settings));
    }
}
