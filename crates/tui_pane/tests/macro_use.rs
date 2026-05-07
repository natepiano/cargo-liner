//! Cross-crate use of the `tui_pane` macros and public types.
//!
//! Compiled as a separate crate that depends on `tui_pane`. Locks the
//! `$crate::*` paths inside the macro expansions and the flat-namespace
//! root re-exports (`tui_pane::BarSlot`, `tui_pane::BarRegion`, etc.)
//! against accidental breakage when the trait or re-export layout
//! shifts.

#![allow(
    missing_docs,
    reason = "test-only enum; macro does not propagate variant docs"
)]

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use tui_pane::Action;
use tui_pane::AppContext;
use tui_pane::BarRegion;
use tui_pane::BarSlot;
use tui_pane::Bindings;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::FrameworkPaneId;
use tui_pane::Globals;
use tui_pane::InputMode;
use tui_pane::KeyBind;
use tui_pane::Navigation;
use tui_pane::ShortcutState;
use tui_pane::Shortcuts;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum CrossCrateAction {
        Alpha => ("alpha", "alpha", "Alpha");
        Beta  => ("beta",  "beta",  "Beta");
        Gamma => ("gamma", "gamma", "Gamma");
    }
}

#[test]
fn action_enum_macro_works_from_outside_crate() {
    assert_eq!(
        CrossCrateAction::ALL,
        &[
            CrossCrateAction::Alpha,
            CrossCrateAction::Beta,
            CrossCrateAction::Gamma,
        ]
    );
    assert_eq!(CrossCrateAction::Alpha.toml_key(), "alpha");
    assert_eq!(CrossCrateAction::Beta.bar_label(), "beta");
    assert_eq!(CrossCrateAction::Beta.description(), "Beta");
    assert_eq!(
        CrossCrateAction::from_toml_key("gamma"),
        Some(CrossCrateAction::Gamma),
    );
    assert_eq!(CrossCrateAction::from_toml_key("zzz"), None);
}

#[test]
fn display_impl_works_from_outside_crate() {
    assert_eq!(format!("{}", CrossCrateAction::Beta), "Beta");
}

#[test]
fn bindings_macro_works_from_outside_crate() {
    let table = tui_pane::bindings! {
        KeyCode::Enter => CrossCrateAction::Alpha,
        [KeyBind::from('b'), KeyBind::from(KeyCode::F(1))] => CrossCrateAction::Beta,
        KeyBind::ctrl(KeyBind::shift('g')) => CrossCrateAction::Gamma,
    };
    let map = table.into_scope_map();

    assert_eq!(
        map.action_for(&KeyBind::from(KeyCode::Enter)),
        Some(CrossCrateAction::Alpha),
    );
    assert_eq!(
        map.action_for(&KeyBind::from('b')),
        Some(CrossCrateAction::Beta),
    );
    assert_eq!(
        map.action_for(&KeyBind::from(KeyCode::F(1))),
        Some(CrossCrateAction::Beta),
    );
    assert_eq!(
        map.key_for(CrossCrateAction::Beta),
        Some(&KeyBind::from('b')),
        "first key in list arm is primary",
    );

    let composed = KeyBind {
        code: KeyCode::Char('g'),
        mods: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    };
    assert_eq!(
        map.action_for(&composed),
        Some(CrossCrateAction::Gamma),
        "Ctrl+Shift composition survives macro expansion",
    );
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum CrossCratePaneId {
    Alpha,
}

struct CrossCrateApp {
    framework: Framework<Self>,
}

impl AppContext for CrossCrateApp {
    type AppPaneId = CrossCratePaneId;

    fn framework(&self) -> &Framework<Self> { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

#[test]
fn framework_skeleton_reachable_from_outside_crate() {
    let mut app = CrossCrateApp {
        framework: Framework::new(FocusedPane::App(CrossCratePaneId::Alpha)),
    };

    assert_eq!(
        app.framework().focused(),
        &FocusedPane::App(CrossCratePaneId::Alpha),
    );
    assert!(!app.framework().quit_requested());
    assert!(!app.framework().restart_requested());

    app.set_focus(FocusedPane::Framework(FrameworkPaneId::Toasts));
    assert_eq!(
        app.framework().focused(),
        &FocusedPane::Framework(FrameworkPaneId::Toasts),
    );
}

#[test]
fn bar_primitives_reachable_from_outside_crate() {
    let single: BarSlot<CrossCrateAction> = BarSlot::Single(CrossCrateAction::Alpha);
    let paired: BarSlot<CrossCrateAction> =
        BarSlot::Paired(CrossCrateAction::Alpha, CrossCrateAction::Beta, "/");

    assert_eq!(single, BarSlot::Single(CrossCrateAction::Alpha));
    assert_eq!(
        paired,
        BarSlot::Paired(CrossCrateAction::Alpha, CrossCrateAction::Beta, "/"),
    );

    assert_eq!(
        BarRegion::ALL,
        &[BarRegion::Nav, BarRegion::PaneAction, BarRegion::Global],
    );

    assert_ne!(ShortcutState::Enabled, ShortcutState::Disabled);

    let mode = InputMode::Navigable;
    assert_eq!(mode, InputMode::Navigable);
    assert_ne!(mode, InputMode::Static);
    assert_ne!(mode, InputMode::TextInput);
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum CrossCrateNavAction {
        Up    => ("up",    "up",    "Move up");
        Down  => ("down",  "down",  "Move down");
        Left  => ("left",  "left",  "Move left");
        Right => ("right", "right", "Move right");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum CrossCrateGlobalAction {
        Find   => ("find",   "find",   "Open finder");
        Rescan => ("rescan", "rescan", "Rescan");
    }
}

struct CrossCratePane;

impl Shortcuts<CrossCrateApp> for CrossCratePane {
    type Variant = CrossCrateAction;

    const APP_PANE_ID: CrossCratePaneId = CrossCratePaneId::Alpha;
    const SCOPE_NAME: &'static str = "cross_crate";

    fn defaults() -> Bindings<Self::Variant> {
        tui_pane::bindings! {
            KeyCode::Enter => CrossCrateAction::Alpha,
            'b' => CrossCrateAction::Beta,
            'g' => CrossCrateAction::Gamma,
        }
    }

    fn dispatcher() -> fn(Self::Variant, &mut CrossCrateApp) {
        |_action, _ctx| { /* no-op for the smoke test */ }
    }
}

struct CrossCrateNavigation;

impl Navigation<CrossCrateApp> for CrossCrateNavigation {
    type Variant = CrossCrateNavAction;

    const DOWN: Self::Variant = CrossCrateNavAction::Down;
    const LEFT: Self::Variant = CrossCrateNavAction::Left;
    const RIGHT: Self::Variant = CrossCrateNavAction::Right;
    const UP: Self::Variant = CrossCrateNavAction::Up;

    fn defaults() -> Bindings<Self::Variant> {
        tui_pane::bindings! {
            KeyCode::Up    => CrossCrateNavAction::Up,
            KeyCode::Down  => CrossCrateNavAction::Down,
            KeyCode::Left  => CrossCrateNavAction::Left,
            KeyCode::Right => CrossCrateNavAction::Right,
        }
    }

    fn dispatcher() -> fn(Self::Variant, FocusedPane<CrossCratePaneId>, &mut CrossCrateApp) {
        |_action, _focused, _ctx| { /* no-op for the smoke test */ }
    }
}

struct CrossCrateGlobals;

impl Globals<CrossCrateApp> for CrossCrateGlobals {
    type Variant = CrossCrateGlobalAction;

    fn render_order() -> &'static [Self::Variant] {
        &[CrossCrateGlobalAction::Find, CrossCrateGlobalAction::Rescan]
    }

    fn defaults() -> Bindings<Self::Variant> {
        tui_pane::bindings! {
            'f' => CrossCrateGlobalAction::Find,
            KeyCode::F(5) => CrossCrateGlobalAction::Rescan,
        }
    }

    fn dispatcher() -> fn(Self::Variant, &mut CrossCrateApp) {
        |_action, _ctx| { /* no-op for the smoke test */ }
    }
}

#[test]
fn shortcuts_trait_works_from_outside_crate() {
    let app = CrossCrateApp {
        framework: Framework::new(FocusedPane::App(CrossCratePaneId::Alpha)),
    };
    let pane = CrossCratePane;

    assert_eq!(
        <CrossCratePane as Shortcuts<CrossCrateApp>>::SCOPE_NAME,
        "cross_crate"
    );
    assert_eq!(
        <CrossCratePane as Shortcuts<CrossCrateApp>>::APP_PANE_ID,
        CrossCratePaneId::Alpha,
    );
    assert_eq!(pane.label(CrossCrateAction::Alpha, &app), Some("alpha"));
    assert_eq!(
        pane.state(CrossCrateAction::Beta, &app),
        ShortcutState::Enabled
    );

    let slots = pane.bar_slots(&app);
    assert_eq!(slots.len(), 3);
    assert_eq!(
        slots[0],
        (
            BarRegion::PaneAction,
            BarSlot::Single(CrossCrateAction::Alpha)
        )
    );

    let query: fn(&CrossCrateApp) -> InputMode = CrossCratePane::input_mode();
    assert_eq!(query(&app), InputMode::Navigable);
    assert!(CrossCratePane::vim_extras().is_empty());

    let map = CrossCratePane::defaults().into_scope_map();
    assert_eq!(
        map.action_for(&KeyCode::Enter.into()),
        Some(CrossCrateAction::Alpha),
    );
}

#[test]
fn navigation_trait_works_from_outside_crate() {
    assert_eq!(
        <CrossCrateNavigation as Navigation<CrossCrateApp>>::SCOPE_NAME,
        "navigation",
    );
    assert_eq!(
        <CrossCrateNavigation as Navigation<CrossCrateApp>>::UP,
        CrossCrateNavAction::Up,
    );
    let map = CrossCrateNavigation::defaults().into_scope_map();
    assert_eq!(
        map.action_for(&KeyCode::Up.into()),
        Some(CrossCrateNavAction::Up),
    );
}

#[test]
fn globals_trait_works_from_outside_crate() {
    assert_eq!(
        <CrossCrateGlobals as Globals<CrossCrateApp>>::SCOPE_NAME,
        "global",
    );
    assert_eq!(
        CrossCrateGlobals::render_order(),
        &[CrossCrateGlobalAction::Find, CrossCrateGlobalAction::Rescan],
    );
    let map = CrossCrateGlobals::defaults().into_scope_map();
    assert_eq!(
        map.action_for(&'f'.into()),
        Some(CrossCrateGlobalAction::Find),
    );
}
