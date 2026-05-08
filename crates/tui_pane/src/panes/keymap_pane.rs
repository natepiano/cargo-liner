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
#[allow(
    dead_code,
    reason = "Awaiting/Conflict are constructed in Phase 14 when handle_key wires the editor state machine"
)]
enum EditState {
    /// Default browse mode — scrollable list of bindings.
    Browse,
    /// Capturing the next keypress as a replacement binding.
    Awaiting,
    /// The captured key collides with an existing binding; the user is
    /// resolving the conflict.
    Conflict,
}

/// Framework-owned keymap viewer overlay.
///
/// Held inline on [`Framework<Ctx>`](crate::Framework) and reached via
/// `framework.keymap_pane`. The dispatcher consults
/// [`Framework::overlay`](crate::Framework::overlay) before routing
/// keys here.
pub struct KeymapPane<Ctx: AppContext> {
    edit_state:    EditState,
    editor_target: Option<PathBuf>,
    _ctx:          PhantomData<fn(&mut Ctx)>,
}

impl<Ctx: AppContext> KeymapPane<Ctx> {
    /// Construct a fresh overlay in [`EditState::Browse`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            edit_state:    EditState::Browse,
            editor_target: None,
            _ctx:          PhantomData,
        }
    }

    /// Default key bindings for the overlay's local actions. Phase 14
    /// folds these through the keymap builder so TOML overrides apply
    /// to the overlay, not just app panes.
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
    /// behavior. Phase 14 wires the captured key into the
    /// [`EditState`] transition.
    pub const fn handle_key(&mut self, _ctx: &mut Ctx, _bind: &KeyBind) -> KeyOutcome {
        KeyOutcome::Consumed
    }

    /// Current input mode for the overlay.
    ///
    /// - [`EditState::Browse`] → [`Mode::Navigable`].
    /// - [`EditState::Awaiting`] → [`Mode::TextInput`] with a stub handler (Phase 14 swaps in the
    ///   real key-capture function).
    /// - [`EditState::Conflict`] → [`Mode::Static`].
    #[must_use]
    pub fn mode(&self, _ctx: &Ctx) -> Mode<Ctx> {
        match self.edit_state {
            EditState::Awaiting => Mode::TextInput(keymap_capture_keys::<Ctx>),
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
    /// (Phase 12) consults this when [`Framework::overlay`](crate::Framework::overlay)
    /// is `Some(FrameworkPaneId::Keymap)`.
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

/// Stub key-capture handler. Phase 14 replaces this with logic that
/// records the captured `KeyBind` into the editor state and transitions
/// out of [`EditState::Awaiting`].
const fn keymap_capture_keys<Ctx: AppContext>(_bind: KeyBind, _ctx: &mut Ctx) {}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
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
    }

    impl AppContext for TestApp {
        type AppPaneId = TestPaneId;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    fn fresh_app() -> TestApp {
        TestApp {
            framework: Framework::new(FocusedPane::App(TestPaneId::Foo)),
        }
    }

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
}
