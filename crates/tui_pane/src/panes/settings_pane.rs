//! `SettingsPane<Ctx>`: framework-owned settings overlay.
//!
//! Lives behind [`Framework::settings_pane`](crate::Framework). Phase 11
//! ships the struct, the [`EditState`] machine, and the inherent action
//! surface (`defaults`, `handle_key`, `mode`, `bar_slots`,
//! `editor_target`). Phase 14 reroutes the binary's settings overlay
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
    /// Actions reachable on the settings overlay's local bar.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum SettingsPaneAction {
        /// Begin editing the selected setting.
        StartEdit => ("start_edit", "edit",   "Edit selected setting");
        /// Persist pending edits.
        Save      => ("save",       "save",   "Save changes");
        /// Discard pending edits.
        Cancel    => ("cancel",     "cancel", "Cancel edit");
    }
}

/// Editor sub-state for the settings overlay.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[allow(
    dead_code,
    reason = "Editing is constructed in Phase 14 when handle_key wires the editor state machine"
)]
enum EditState {
    /// Default browse mode — the user is paging through settings.
    Browse,
    /// The user is typing a new value into the focused setting.
    Editing,
}

/// Framework-owned settings overlay.
///
/// Held inline on [`Framework<Ctx>`](crate::Framework) and reached via
/// `framework.settings_pane`. The dispatcher consults
/// [`Framework::overlay`](crate::Framework::overlay) before routing
/// keys here.
pub struct SettingsPane<Ctx: AppContext> {
    edit_state:    EditState,
    editor_target: Option<PathBuf>,
    _ctx:          PhantomData<fn(&mut Ctx)>,
}

impl<Ctx: AppContext> SettingsPane<Ctx> {
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
    pub fn defaults() -> Bindings<SettingsPaneAction> {
        crate::bindings! {
            KeyCode::Enter => SettingsPaneAction::StartEdit,
            's'            => SettingsPaneAction::Save,
            KeyCode::Esc   => SettingsPaneAction::Cancel,
        }
    }

    /// Consume one keypress. Always returns
    /// [`KeyOutcome::Consumed`] — the overlay short-circuits all input
    /// when open, matching the existing cargo-port `settings_open`
    /// behavior. Phase 14 wires the captured key into the
    /// [`EditState`] transition.
    pub const fn handle_key(&mut self, _ctx: &mut Ctx, _bind: &KeyBind) -> KeyOutcome {
        KeyOutcome::Consumed
    }

    /// Current input mode for the overlay.
    ///
    /// - [`EditState::Browse`] → [`Mode::Navigable`].
    /// - [`EditState::Editing`] → [`Mode::TextInput`] with a stub handler (Phase 14 swaps in the
    ///   real edit-routing function).
    #[must_use]
    pub fn mode(&self, _ctx: &Ctx) -> Mode<Ctx> {
        match self.edit_state {
            EditState::Editing => Mode::TextInput(settings_edit_keys::<Ctx>),
            EditState::Browse => Mode::Navigable,
        }
    }

    /// File path of the setting being edited, if any. Drives the
    /// framework's [`editor_target_path`](crate::Framework::editor_target_path)
    /// surface. Returns `None` outside [`EditState::Editing`].
    #[must_use]
    pub fn editor_target(&self) -> Option<&Path> { self.editor_target.as_deref() }

    /// Bar slots for the overlay's local actions. The bar renderer
    /// (Phase 13) consults this when [`Framework::overlay`](crate::Framework::overlay)
    /// is `Some(FrameworkOverlayId::Settings)`.
    #[must_use]
    pub fn bar_slots(&self, _ctx: &Ctx) -> Vec<(BarRegion, BarSlot<SettingsPaneAction>)> {
        SettingsPaneAction::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }
}

impl<Ctx: AppContext> Default for SettingsPane<Ctx> {
    fn default() -> Self { Self::new() }
}

/// Stub edit-routing handler. Phase 14 replaces this with logic that
/// applies the typed character to the focused setting's editing buffer.
const fn settings_edit_keys<Ctx: AppContext>(_bind: KeyBind, _ctx: &mut Ctx) {}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::SettingsPane;
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
        let pane: SettingsPane<TestApp> = SettingsPane::new();
        let app = fresh_app();
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn editor_target_is_none_at_construction() {
        let pane: SettingsPane<TestApp> = SettingsPane::new();
        assert!(pane.editor_target().is_none());
    }

    #[test]
    fn handle_key_always_returns_consumed() {
        let mut pane: SettingsPane<TestApp> = SettingsPane::new();
        let mut app = fresh_app();
        assert_eq!(
            pane.handle_key(&mut app, &KeyBind::from('z')),
            KeyOutcome::Consumed,
        );
    }

    #[test]
    fn bar_slots_emit_one_slot_per_variant() {
        let pane: SettingsPane<TestApp> = SettingsPane::new();
        let app = fresh_app();
        let slots = pane.bar_slots(&app);
        assert_eq!(slots.len(), 3);
    }
}
