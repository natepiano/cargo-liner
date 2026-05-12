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
use crate::Viewport;

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

/// Command produced by [`SettingsPane::handle_text_input`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsCommand {
    /// No commit/cancel command was produced.
    None,
    /// Apply the current edit buffer.
    Save,
    /// Cancel the current edit.
    Cancel,
}

/// Editor sub-state for the settings overlay.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    viewport:      Viewport,
    line_targets:  Vec<Option<usize>>,
    edit_buffer:   String,
    edit_cursor:   usize,
    _ctx:          PhantomData<fn(&mut Ctx)>,
}

impl<Ctx: AppContext> SettingsPane<Ctx> {
    /// Construct a fresh overlay in [`EditState::Browse`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            edit_state:    EditState::Browse,
            editor_target: None,
            viewport:      Viewport::new(),
            line_targets:  Vec::new(),
            edit_buffer:   String::new(),
            edit_cursor:   0,
            _ctx:          PhantomData,
        }
    }

    /// Default key bindings for the overlay's local actions. The
    /// keymap builder can fold these through overlay registration so
    /// TOML overrides apply to the overlay, not just app panes.
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
    /// behavior. Resolves `bind` against [`Self::defaults`] and flips
    /// [`EditState`] accordingly: `StartEdit` enters [`EditState::Editing`]
    /// from `Browse`; `Save` and `Cancel` return to `Browse`. Per-setting
    /// buffer mutation lives on this pane; [`Self::handle_text_input`]
    /// returns the command the binary applies after the framework
    /// borrow ends.
    pub fn handle_key(&mut self, _ctx: &mut Ctx, bind: &KeyBind) -> KeyOutcome {
        if let Some(action) = Self::defaults().into_scope_map().action_for(bind) {
            match action {
                SettingsPaneAction::StartEdit => self.enter_editing(),
                SettingsPaneAction::Save | SettingsPaneAction::Cancel => self.enter_browse(),
            }
        }
        KeyOutcome::Consumed
    }

    /// Mark the overlay as actively editing text. Binaries call this
    /// after their settings row dispatcher decides the selected row
    /// owns a text buffer.
    pub const fn enter_editing(&mut self) {
        if matches!(self.edit_state, EditState::Browse) {
            self.edit_state = EditState::Editing;
        }
    }

    /// Enter edit mode with an initial value.
    pub fn begin_editing(&mut self, value: String) {
        self.edit_cursor = value.len();
        self.edit_buffer = value;
        self.enter_editing();
    }

    /// Return the overlay to browse mode after saving, cancelling, or
    /// closing an edit.
    pub fn enter_browse(&mut self) {
        self.edit_state = EditState::Browse;
        self.edit_buffer.clear();
        self.edit_cursor = 0;
    }

    /// Whether the pane is actively editing text.
    #[must_use]
    pub const fn is_editing(&self) -> bool { matches!(self.edit_state, EditState::Editing) }

    /// Borrow the edit buffer.
    #[must_use]
    pub fn edit_buffer(&self) -> &str { &self.edit_buffer }

    /// Borrow the text currently being edited.
    #[must_use]
    pub fn edited_text(&self) -> &str { &self.edit_buffer }

    /// Edit cursor byte offset.
    #[must_use]
    pub const fn edit_cursor(&self) -> usize { self.edit_cursor }

    /// Replace the edit buffer and cursor.
    pub fn set_edit_buffer(&mut self, value: String, cursor: usize) {
        self.edit_cursor = cursor.min(value.len());
        self.edit_buffer = value;
    }

    /// Joint mutable handles on the edit buffer and cursor.
    pub const fn edit_parts_mut(&mut self) -> (&mut String, &mut usize) {
        (&mut self.edit_buffer, &mut self.edit_cursor)
    }

    /// Borrow the framework-owned viewport state.
    #[must_use]
    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    /// Mutably borrow the framework-owned viewport state.
    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    /// Store rendered-line to setting-row targets for hit testing.
    pub fn set_line_targets(&mut self, targets: Vec<Option<usize>>) { self.line_targets = targets; }

    /// Return the setting row target for a rendered line index.
    #[must_use]
    pub fn line_target(&self, line: usize) -> Option<usize> {
        self.line_targets.get(line).copied().flatten()
    }

    /// Consume one text-input key and return a command.
    pub fn handle_text_input(&mut self, bind: KeyBind) -> SettingsCommand {
        match bind.code {
            KeyCode::Enter => SettingsCommand::Save,
            KeyCode::Esc => SettingsCommand::Cancel,
            KeyCode::Left => {
                self.edit_cursor = prev_char_boundary(&self.edit_buffer, self.edit_cursor);
                SettingsCommand::None
            },
            KeyCode::Right => {
                self.edit_cursor = next_char_boundary(&self.edit_buffer, self.edit_cursor);
                SettingsCommand::None
            },
            KeyCode::Home => {
                self.edit_cursor = 0;
                SettingsCommand::None
            },
            KeyCode::End => {
                self.edit_cursor = self.edit_buffer.len();
                SettingsCommand::None
            },
            KeyCode::Backspace => {
                backspace_at_cursor(&mut self.edit_buffer, &mut self.edit_cursor);
                SettingsCommand::None
            },
            KeyCode::Delete => {
                delete_at_cursor(&mut self.edit_buffer, self.edit_cursor);
                SettingsCommand::None
            },
            KeyCode::Char(c) => {
                insert_char_at_cursor(&mut self.edit_buffer, &mut self.edit_cursor, c);
                SettingsCommand::None
            },
            _ => SettingsCommand::None,
        }
    }

    /// Current input mode for the overlay.
    ///
    /// - [`EditState::Browse`] → [`Mode::Navigable`].
    /// - [`EditState::Editing`] → [`Mode::TextInput`] with an inert handler. The input path calls
    ///   [`Self::handle_text_input`] directly and uses this mode only as a suppression signal for
    ///   global dispatch.
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

#[cfg(test)]
impl<Ctx: AppContext> SettingsPane<Ctx> {
    /// Test-only constructor placing the pane in
    /// [`EditState::Editing`] with an optional editor target. Phase
    /// 15 wires the `Browse → Editing` production transition; Phase
    /// 13 snapshot tests build this state directly so they can lock
    /// the bar output before the transition lands.
    pub(crate) fn for_test_editing(editor_target: Option<PathBuf>) -> Self {
        Self {
            edit_state: EditState::Editing,
            editor_target,
            viewport: Viewport::new(),
            line_targets: Vec::new(),
            edit_buffer: String::new(),
            edit_cursor: 0,
            _ctx: PhantomData,
        }
    }
}

/// Inert handler used only to mark settings editing as text-input
/// mode. The real mutation path is [`SettingsPane::handle_text_input`].
const fn settings_edit_keys<Ctx: AppContext>(_bind: KeyBind, _ctx: &mut Ctx) {}

fn prev_char_boundary(s: &str, cursor: usize) -> usize {
    s[..cursor.min(s.len())]
        .char_indices()
        .next_back()
        .map_or(0, |(idx, _)| idx)
}

fn next_char_boundary(s: &str, cursor: usize) -> usize {
    s[cursor.min(s.len())..]
        .char_indices()
        .nth(1)
        .map_or(s.len(), |(idx, _)| cursor + idx)
}

fn backspace_at_cursor(buf: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let start = prev_char_boundary(buf, *cursor);
    buf.replace_range(start..*cursor, "");
    *cursor = start;
}

fn delete_at_cursor(buf: &mut String, cursor: usize) {
    if cursor >= buf.len() {
        return;
    }
    let end = next_char_boundary(buf, cursor);
    buf.replace_range(cursor..end, "");
}

fn insert_char_at_cursor(buf: &mut String, cursor: &mut usize, c: char) {
    buf.insert(*cursor, c);
    *cursor += c.len_utf8();
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

    fn fresh_app() -> TestApp {
        TestApp {
            framework:    Framework::new(FocusedPane::App(TestPaneId::Foo)),
            app_settings: (),
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

    #[test]
    fn enter_in_browse_transitions_to_editing() {
        let mut pane: SettingsPane<TestApp> = SettingsPane::new();
        let mut app = fresh_app();
        let _ = pane.handle_key(&mut app, &KeyCode::Enter.into());
        assert!(matches!(pane.mode(&app), Mode::TextInput(_)));
    }

    #[test]
    fn esc_in_editing_returns_to_browse() {
        let mut pane: SettingsPane<TestApp> = SettingsPane::for_test_editing(None);
        let mut app = fresh_app();
        let _ = pane.handle_key(&mut app, &KeyCode::Esc.into());
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn save_in_editing_returns_to_browse() {
        let mut pane: SettingsPane<TestApp> = SettingsPane::for_test_editing(None);
        let mut app = fresh_app();
        let _ = pane.handle_key(&mut app, &KeyBind::from('s'));
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn begin_editing_sets_buffer_and_cursor() {
        let mut pane: SettingsPane<TestApp> = SettingsPane::new();

        pane.begin_editing("hana".to_string());

        assert_eq!(pane.edit_buffer(), "hana");
        assert_eq!(pane.edit_cursor(), 4);
    }

    #[test]
    fn handle_text_input_mutates_owned_buffer() {
        let mut pane: SettingsPane<TestApp> = SettingsPane::new();
        pane.begin_editing("ha".to_string());

        assert_eq!(
            pane.handle_text_input(KeyBind::from('n')),
            super::SettingsCommand::None
        );
        assert_eq!(
            pane.handle_text_input(KeyBind::from('a')),
            super::SettingsCommand::None
        );
        assert_eq!(pane.edit_buffer(), "hana");
        assert_eq!(pane.edit_cursor(), 4);

        let _ = pane.handle_text_input(KeyCode::Left.into());
        let _ = pane.handle_text_input(KeyCode::Backspace.into());

        assert_eq!(pane.edit_buffer(), "haa");
        assert_eq!(pane.edit_cursor(), 2);
    }

    #[test]
    fn handle_text_input_returns_save_and_cancel_commands() {
        let mut pane: SettingsPane<TestApp> = SettingsPane::new();
        pane.begin_editing(String::new());

        assert_eq!(
            pane.handle_text_input(KeyCode::Enter.into()),
            super::SettingsCommand::Save
        );
        assert_eq!(
            pane.handle_text_input(KeyCode::Esc.into()),
            super::SettingsCommand::Cancel
        );
    }
}
