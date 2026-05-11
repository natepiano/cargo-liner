//! `KeymapPane<Ctx>`: framework-owned keymap viewer/editor overlay.
//!
//! Lives behind [`Framework::keymap_pane`](crate::Framework). Phase 11
//! ships the struct, the [`EditState`] machine, and the inherent action
//! surface (`defaults`, `handle_key`, `mode`, `bar_slots`,
//! `editor_target`). Phase 14 reroutes the binary's keymap overlay
//! input path through this pane.

use core::marker::PhantomData;
use std::path::Path;
use std::path::PathBuf;

use crossterm::event::KeyCode;

use crate::Action;
use crate::AppContext;
use crate::BarRegion;
use crate::BarSlot;
use crate::Bindings;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::Mode;

crate::action_enum! {
    /// Actions reachable on the keymap overlay's local bar.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum KeymapPaneAction {
        /// Begin editing the selected binding.
        StartEdit => ("start_edit", "edit",   "Edit selected binding");
        /// Persist pending edits.
        Save      => ("save",       "save",   "Save changes");
        /// Discard pending edits.
        Cancel    => ("cancel",     "cancel", "Cancel edit");
    }
}

/// Editor sub-state for the keymap overlay.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum EditState {
    /// Default browse mode — scrollable list of bindings.
    Browse,
    /// Capturing the next keypress as a replacement binding.
    Awaiting,
    /// The captured key collides with an existing binding; the user is
    /// resolving the conflict.
    #[cfg(test)]
    Conflict,
}

/// Framework-owned keymap viewer overlay.
///
/// Held inline on [`Framework<Ctx>`](crate::Framework) and reached via
/// `framework.keymap_pane`. The dispatcher consults
/// [`Framework::overlay`](crate::Framework::overlay) before routing
/// keys here.
pub struct KeymapPane<Ctx: AppContext> {
    edit_state:         EditState,
    editor_target:      Option<PathBuf>,
    text_input_handler: fn(KeyBind, &mut Ctx),
    _ctx:               PhantomData<fn(&mut Ctx)>,
}

impl<Ctx: AppContext> KeymapPane<Ctx> {
    /// Construct a fresh overlay in [`EditState::Browse`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            edit_state:         EditState::Browse,
            editor_target:      None,
            text_input_handler: keymap_capture_keys::<Ctx>,
            _ctx:               PhantomData,
        }
    }

    /// Replace the [`Mode::TextInput`] payload used while the overlay
    /// is awaiting a captured key. Binaries use this to route captures
    /// into their own keymap state.
    #[must_use]
    pub const fn with_text_input_handler(mut self, handler: fn(KeyBind, &mut Ctx)) -> Self {
        self.text_input_handler = handler;
        self
    }

    /// Replace the [`Mode::TextInput`] payload after construction.
    pub const fn set_text_input_handler(&mut self, handler: fn(KeyBind, &mut Ctx)) {
        self.text_input_handler = handler;
    }

    /// Default key bindings for the overlay's local actions. The
    /// keymap builder can fold these through overlay registration so
    /// TOML overrides apply to the overlay, not just app panes.
    #[must_use]
    pub fn defaults() -> Bindings<KeymapPaneAction> {
        crate::bindings! {
            KeyCode::Enter => KeymapPaneAction::StartEdit,
            's'            => KeymapPaneAction::Save,
            KeyCode::Esc   => KeymapPaneAction::Cancel,
        }
    }

    /// Consume one keypress. Always returns
    /// [`KeyOutcome::Consumed`] — the overlay short-circuits all input
    /// when open, matching the existing cargo-port `keymap_open`
    /// behavior. Resolves `bind` against [`Self::defaults`] and flips
    /// [`EditState`] accordingly: `StartEdit` enters
    /// [`EditState::Awaiting`] from `Browse`; `Save` and `Cancel`
    /// return to `Browse` from any state. Capture-conflict resolution
    /// (`Awaiting → Conflict`) is driven by the binary's collision
    /// check; Phase 19 cutover folds that step onto this pane.
    pub fn handle_key(&mut self, _ctx: &mut Ctx, bind: &KeyBind) -> KeyOutcome {
        if let Some(action) = Self::defaults().into_scope_map().action_for(bind) {
            match action {
                KeymapPaneAction::StartEdit => self.enter_awaiting(),
                KeymapPaneAction::Save | KeymapPaneAction::Cancel => self.enter_browse(),
            }
        }
        KeyOutcome::Consumed
    }

    /// Mark the overlay as waiting for a replacement binding.
    pub const fn enter_awaiting(&mut self) {
        if matches!(self.edit_state, EditState::Browse) {
            self.edit_state = EditState::Awaiting;
        }
    }

    /// Return the overlay to browse mode after saving, cancelling, or
    /// accepting a captured binding.
    pub const fn enter_browse(&mut self) { self.edit_state = EditState::Browse; }

    /// Current input mode for the overlay.
    ///
    /// - [`EditState::Browse`] → [`Mode::Navigable`].
    /// - [`EditState::Awaiting`] → [`Mode::TextInput`] with the configured handler. The default
    ///   stub stays inert until the binary wires its handler during Phase 19.
    /// - [`EditState::Conflict`] → [`Mode::Static`].
    #[must_use]
    pub fn mode(&self, _ctx: &Ctx) -> Mode<Ctx> {
        match self.edit_state {
            EditState::Awaiting => Mode::TextInput(self.text_input_handler),
            #[cfg(test)]
            EditState::Conflict => Mode::Static,
            EditState::Browse => Mode::Navigable,
        }
    }

    /// File path of the binding being edited, if any. Drives the
    /// framework's [`editor_target_path`](crate::Framework::editor_target_path)
    /// surface so the binary's status line can show the active TOML
    /// file. Returns `None` outside [`EditState::Awaiting`] /
    /// [`EditState::Conflict`].
    #[must_use]
    pub fn editor_target(&self) -> Option<&Path> { self.editor_target.as_deref() }

    /// Bar slots for the overlay's local actions. The bar renderer
    /// (Phase 13) consults this when [`Framework::overlay`](crate::Framework::overlay)
    /// is `Some(FrameworkOverlayId::Keymap)`.
    #[must_use]
    pub fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<KeymapPaneAction>)> {
        KeymapPaneAction::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }
}

impl<Ctx: AppContext> Default for KeymapPane<Ctx> {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
impl<Ctx: AppContext> KeymapPane<Ctx> {
    /// Test-only constructor placing the pane in
    /// [`EditState::Awaiting`] with an optional editor target. Phase
    /// 15 wires the `Browse → Awaiting` production transition; Phase
    /// 13 snapshot tests build this state directly so they can lock
    /// the bar output before the transition lands.
    pub(crate) fn for_test_awaiting(editor_target: Option<PathBuf>) -> Self {
        Self {
            edit_state: EditState::Awaiting,
            editor_target,
            text_input_handler: keymap_capture_keys::<Ctx>,
            _ctx: PhantomData,
        }
    }

    /// Test-only constructor placing the pane in
    /// [`EditState::Conflict`]. See [`Self::for_test_awaiting`].
    pub(crate) fn for_test_conflict(editor_target: Option<PathBuf>) -> Self {
        Self {
            edit_state: EditState::Conflict,
            editor_target,
            text_input_handler: keymap_capture_keys::<Ctx>,
            _ctx: PhantomData,
        }
    }
}

/// Generic key-capture handler. The framework owns the
/// `Mode::TextInput` payload type but cannot mutate the binary's
/// per-binding capture cell — that state lives on `Ctx` (e.g.
/// `App::keymap_state`). The binary uses
/// [`KeymapPane::with_text_input_handler`] or
/// [`KeymapPane::set_text_input_handler`] to inject capture mutation.
const fn keymap_capture_keys<Ctx: AppContext>(_bind: KeyBind, _ctx: &mut Ctx) {}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crossterm::event::KeyCode;

    use super::KeymapPane;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::KeyBind;
    use crate::KeyOutcome;
    use crate::Mode;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum TestPaneId {
        Foo,
    }

    struct TestApp {
        framework: Framework<Self>,
        captures:  u32,
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    fn fresh_app() -> TestApp {
        TestApp {
            framework: Framework::new(FocusedPane::App(TestPaneId::Foo)),
            captures:  0,
        }
    }

    fn record_capture(_bind: KeyBind, app: &mut TestApp) { app.captures += 1; }

    #[test]
    fn new_starts_in_browse_mode() {
        let pane: KeymapPane<TestApp> = KeymapPane::new();
        let app = fresh_app();
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn editor_target_is_none_at_construction() {
        let pane: KeymapPane<TestApp> = KeymapPane::new();
        assert!(pane.editor_target().is_none());
    }

    #[test]
    fn handle_key_always_returns_consumed() {
        let mut pane: KeymapPane<TestApp> = KeymapPane::new();
        let mut app = fresh_app();
        assert_eq!(
            pane.handle_key(&mut app, &KeyBind::from('z')),
            KeyOutcome::Consumed,
        );
    }

    #[test]
    fn defaults_round_trip_through_scope_map() {
        let map = KeymapPane::<TestApp>::defaults().into_scope_map();
        assert!(
            map.action_for(&KeyBind::from(crossterm::event::KeyCode::Esc))
                .is_some()
        );
    }

    #[test]
    fn bar_slots_emit_one_slot_per_variant() {
        let pane: KeymapPane<TestApp> = KeymapPane::new();
        let app = fresh_app();
        let slots = pane.bar_slots(&app);
        assert_eq!(slots.len(), 3);
    }

    #[test]
    fn enter_in_browse_transitions_to_awaiting() {
        let mut pane: KeymapPane<TestApp> = KeymapPane::new();
        let mut app = fresh_app();
        let _ = pane.handle_key(&mut app, &KeyCode::Enter.into());
        assert!(matches!(pane.mode(&app), Mode::TextInput(_)));
    }

    #[test]
    fn esc_in_awaiting_returns_to_browse() {
        let mut pane: KeymapPane<TestApp> = KeymapPane::for_test_awaiting(None);
        let mut app = fresh_app();
        let _ = pane.handle_key(&mut app, &KeyCode::Esc.into());
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn save_in_conflict_returns_to_browse() {
        let mut pane: KeymapPane<TestApp> = KeymapPane::for_test_conflict(None);
        let mut app = fresh_app();
        let _ = pane.handle_key(&mut app, &KeyBind::from('s'));
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn with_text_input_handler_swaps_awaiting_payload() {
        let pane = KeymapPane::for_test_awaiting(None).with_text_input_handler(record_capture);
        let mut app = fresh_app();
        let Mode::TextInput(handler) = pane.mode(&app) else {
            panic!("awaiting mode must return text-input handler");
        };

        handler(KeyBind::from('x'), &mut app);

        assert_eq!(app.captures, 1);
    }

    #[test]
    fn set_text_input_handler_swaps_awaiting_payload_after_construction() {
        let mut pane = KeymapPane::for_test_awaiting(None);
        pane.set_text_input_handler(record_capture);
        let mut app = fresh_app();
        let Mode::TextInput(handler) = pane.mode(&app) else {
            panic!("awaiting mode must return text-input handler");
        };

        handler(KeyBind::from('x'), &mut app);

        assert_eq!(app.captures, 1);
    }
}
