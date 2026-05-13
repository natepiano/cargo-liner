//! `KeymapPane`: framework-owned keymap viewer/editor overlay.
//!
//! Lives behind [`Framework::keymap_pane`](crate::Framework). Owns the
//! [`EditState`] machine and the inherent action surface (`defaults`,
//! `handle_key`, `mode`, `bar_slots`, `editor_target`). The binary's
//! keymap overlay input path routes through this pane.

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
use crate::Viewport;

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
    Conflict,
}

/// Command produced by [`KeymapPane::handle_capture_key`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeymapCaptureCommand {
    /// No app-side work is needed.
    None,
    /// The user cancelled capture.
    Cancel,
    /// A conflict message was cleared; continue waiting for a new key.
    ClearConflict,
    /// The pane captured a candidate key for the app to validate and persist.
    Captured(KeyBind),
}

/// Framework-owned keymap viewer overlay.
///
/// Held inline on [`Framework<Ctx>`](crate::Framework) and reached via
/// `framework.keymap_pane`. The dispatcher consults
/// [`Framework::overlay`](crate::Framework::overlay) before routing
/// keys here.
pub struct KeymapPane {
    edit_state:    EditState,
    editor_target: Option<PathBuf>,
    line_targets:  Vec<Option<usize>>,
    viewport:      Viewport,
}

impl KeymapPane {
    /// Construct a fresh overlay in [`EditState::Browse`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            edit_state:    EditState::Browse,
            editor_target: None,
            line_targets:  Vec::new(),
            viewport:      Viewport::new(),
        }
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
    /// check.
    pub fn handle_key(&mut self, bind: &KeyBind) -> KeyOutcome {
        if let Some(action) = Self::defaults().into_scope_map().action_for(bind) {
            match action {
                KeymapPaneAction::StartEdit => self.enter_awaiting(),
                KeymapPaneAction::Save | KeymapPaneAction::Cancel => self.enter_browse(),
            }
        }
        KeyOutcome::Consumed
    }

    /// Mark the overlay as waiting for a replacement binding.
    pub const fn enter_awaiting(&mut self) { self.edit_state = EditState::Awaiting; }

    /// Mark the overlay as showing a capture conflict.
    pub const fn enter_conflict(&mut self) { self.edit_state = EditState::Conflict; }

    /// Return the overlay to browse mode after saving, cancelling, or
    /// accepting a captured binding.
    pub const fn enter_browse(&mut self) { self.edit_state = EditState::Browse; }

    /// Whether the pane is waiting for a replacement binding.
    #[must_use]
    pub const fn is_awaiting(&self) -> bool { matches!(self.edit_state, EditState::Awaiting) }

    /// Whether the pane is in any key-capture state.
    #[must_use]
    pub const fn is_capturing(&self) -> bool {
        matches!(self.edit_state, EditState::Awaiting | EditState::Conflict)
    }

    /// Consume one text-input key while the pane is capturing a replacement binding.
    pub fn handle_capture_key(&mut self, bind: KeyBind) -> KeymapCaptureCommand {
        if bind.code == KeyCode::Esc {
            self.enter_browse();
            return KeymapCaptureCommand::Cancel;
        }
        if bind.code == KeyCode::Enter && matches!(self.edit_state, EditState::Conflict) {
            self.enter_awaiting();
            return KeymapCaptureCommand::ClearConflict;
        }
        KeymapCaptureCommand::Captured(bind)
    }

    /// Borrow the framework-owned viewport state.
    #[must_use]
    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    /// Mutably borrow the framework-owned viewport state.
    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    /// Replace the rendered-line to selectable-row map used for mouse hit-testing.
    pub fn replace_line_targets(&mut self, targets: Vec<Option<usize>>) {
        self.line_targets = targets;
    }

    /// Selectable row rendered at `line`, if the line is clickable.
    #[must_use]
    pub fn line_target(&self, line: usize) -> Option<usize> {
        self.line_targets.get(line).copied().flatten()
    }

    /// Current input mode for the overlay.
    ///
    /// - [`EditState::Browse`] → [`Mode::Navigable`].
    /// - [`EditState::Awaiting`] → [`Mode::TextInput`] with an inert marker handler.
    /// - [`EditState::Conflict`] → [`Mode::Static`] so the conflict bar actions remain visible. The
    ///   input path still calls [`Self::handle_capture_key`] directly for both capture states.
    #[must_use]
    pub fn mode<Ctx: AppContext>(&self, _ctx: &Ctx) -> Mode<Ctx> {
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
    /// consults this when [`Framework::overlay`](crate::Framework::overlay)
    /// is `Some(FrameworkOverlayId::Keymap)`.
    #[must_use]
    pub fn bar_slots(&self) -> Vec<(BarRegion, BarSlot<KeymapPaneAction>)> {
        KeymapPaneAction::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }
}

impl Default for KeymapPane {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
impl KeymapPane {
    /// Test-only constructor placing the pane in
    /// [`EditState::Awaiting`] with an optional editor target. Snapshot
    /// tests build this state directly so they can lock the bar output
    /// without driving the production `Browse → Awaiting` transition.
    pub(crate) fn for_test_awaiting(editor_target: Option<PathBuf>) -> Self {
        Self {
            edit_state: EditState::Awaiting,
            editor_target,
            line_targets: Vec::new(),
            viewport: Viewport::new(),
        }
    }

    /// Test-only constructor placing the pane in
    /// [`EditState::Conflict`]. See [`Self::for_test_awaiting`].
    pub(crate) fn for_test_conflict(editor_target: Option<PathBuf>) -> Self {
        Self {
            edit_state: EditState::Conflict,
            editor_target,
            line_targets: Vec::new(),
            viewport: Viewport::new(),
        }
    }
}

/// Inert handler used only to mark key capture as text-input mode.
/// The input path calls [`KeymapPane::handle_capture_key`] directly.
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

    use super::KeymapCaptureCommand;
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
        type ToastAction = crate::NoToastAction;

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
        let pane = KeymapPane::new();
        let app = fresh_app();
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn editor_target_is_none_at_construction() {
        let pane = KeymapPane::new();
        assert!(pane.editor_target().is_none());
    }

    #[test]
    fn handle_key_always_returns_consumed() {
        let mut pane = KeymapPane::new();
        assert_eq!(pane.handle_key(&KeyBind::from('z')), KeyOutcome::Consumed,);
    }

    #[test]
    fn defaults_round_trip_through_scope_map() {
        let map = KeymapPane::defaults().into_scope_map();
        assert!(
            map.action_for(&KeyBind::from(crossterm::event::KeyCode::Esc))
                .is_some()
        );
    }

    #[test]
    fn bar_slots_emit_one_slot_per_variant() {
        let pane = KeymapPane::new();
        let slots = pane.bar_slots();
        assert_eq!(slots.len(), 3);
    }

    #[test]
    fn enter_in_browse_transitions_to_awaiting() {
        let mut pane = KeymapPane::new();
        let app = fresh_app();
        let _ = pane.handle_key(&KeyCode::Enter.into());
        assert!(matches!(pane.mode(&app), Mode::TextInput(_)));
    }

    #[test]
    fn esc_in_awaiting_returns_to_browse() {
        let mut pane = KeymapPane::for_test_awaiting(None);
        let app = fresh_app();
        let _ = pane.handle_key(&KeyCode::Esc.into());
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn save_in_conflict_returns_to_browse() {
        let mut pane = KeymapPane::for_test_conflict(None);
        let app = fresh_app();
        let _ = pane.handle_key(&KeyBind::from('s'));
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn handle_capture_key_returns_captured_bind() {
        let mut pane = KeymapPane::for_test_awaiting(None);

        assert_eq!(
            pane.handle_capture_key(KeyBind::from('x')),
            KeymapCaptureCommand::Captured(KeyBind::from('x')),
        );
    }

    #[test]
    fn handle_capture_esc_cancels_and_returns_to_browse() {
        let mut pane = KeymapPane::for_test_awaiting(None);
        let app = fresh_app();

        assert_eq!(
            pane.handle_capture_key(KeyCode::Esc.into()),
            KeymapCaptureCommand::Cancel,
        );

        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn handle_capture_enter_clears_conflict() {
        let mut pane = KeymapPane::for_test_conflict(None);
        let app = fresh_app();

        assert_eq!(
            pane.handle_capture_key(KeyCode::Enter.into()),
            KeymapCaptureCommand::ClearConflict,
        );

        assert!(matches!(pane.mode(&app), Mode::TextInput(_)));
    }
}
