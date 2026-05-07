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
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::FrameworkPaneId;
use tui_pane::InputMode;
use tui_pane::KeyBind;
use tui_pane::ShortcutState;

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
