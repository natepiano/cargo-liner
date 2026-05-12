//! `SettingsPane`: framework-owned settings overlay.
//!
//! Lives behind [`Framework::settings_pane`](crate::Framework). Phase 11
//! ships the struct, the [`EditState`] machine, and the inherent action
//! surface (`defaults`, `handle_key`, `mode`, `bar_slots`,
//! `editor_target`). Phase 14 reroutes the binary's settings overlay
//! input path through this pane.

use std::path::Path;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::Action;
use crate::AppContext;
use crate::BarRegion;
use crate::BarSlot;
use crate::Bindings;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::Mode;
use crate::SettingsRow;
use crate::SettingsRowKind;
use crate::SettingsRowPayload;
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

/// Styling and layout inputs for [`SettingsPane::render_rows`].
#[derive(Clone, Copy, Debug)]
pub struct SettingsRenderOptions<'a> {
    /// Whether the settings pane has active focus.
    pub active:                bool,
    /// Inline error for the selected row.
    pub inline_error:          Option<&'a str>,
    /// Available content width inside the popup.
    pub content_width:         usize,
    /// Section header indentation.
    pub section_header_indent: &'a str,
    /// Selectable row indentation.
    pub section_item_indent:   &'a str,
    /// Section title style.
    pub title_style:           Style,
    /// Row label style.
    pub label_style:           Style,
    /// Low-emphasis style.
    pub muted_style:           Style,
    /// Enabled toggle style.
    pub success_style:         Style,
    /// Disabled toggle / warning style.
    pub error_style:           Style,
    /// Inline validation error style.
    pub inline_error_style:    Style,
    /// Active selected row overlay style.
    pub active_style:          Style,
    /// Remembered selected row overlay style.
    pub remembered_style:      Style,
    /// Hovered row overlay style.
    pub hovered_style:         Style,
}

/// Render output from [`SettingsPane::render_rows`].
pub struct SettingsRender {
    /// Renderable text lines.
    pub lines:            Vec<Line<'static>>,
    /// Number of selectable rows.
    pub selectable_count: usize,
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
pub struct SettingsPane {
    edit_state:    EditState,
    editor_target: Option<PathBuf>,
    viewport:      Viewport,
    line_targets:  Vec<Option<SettingsRowPayload>>,
    edit_buffer:   String,
    edit_cursor:   usize,
}

impl SettingsPane {
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
    pub fn handle_key(&mut self, bind: &KeyBind) -> KeyOutcome {
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
    pub fn set_line_targets(&mut self, targets: Vec<Option<SettingsRowPayload>>) {
        self.line_targets = targets;
    }

    /// Return the setting row target for a rendered line index.
    #[must_use]
    pub fn line_target(&self, line: usize) -> Option<usize> {
        self.line_targets
            .get(line)
            .copied()
            .flatten()
            .map(SettingsRowPayload::get)
    }

    /// Render generic settings rows and update the pane's line-target
    /// map for mouse hit testing.
    #[must_use]
    pub fn render_rows(
        &mut self,
        rows: &[SettingsRow],
        options: SettingsRenderOptions<'_>,
    ) -> SettingsRender {
        let max_label = rows
            .iter()
            .filter(|row| row.kind != SettingsRowKind::Section)
            .map(|row| row.label.len())
            .max()
            .unwrap_or(0);
        let mut lines = Vec::new();
        let mut line_targets = Vec::new();
        let mut selection_index = 0;
        for row in rows {
            if row.kind == SettingsRowKind::Section {
                push_settings_header(&mut lines, &mut line_targets, &row.label, &options);
                continue;
            }
            let cursor = if self.viewport.pos() == selection_index {
                "▶ "
            } else {
                "  "
            };
            let selection = self.selection_state(selection_index, options.active);
            let label = format!(
                "{}{cursor}{:<max_label$}  ",
                options.section_item_indent, row.label,
            );
            let context = SettingsLineContext {
                target: row
                    .payload
                    .unwrap_or_else(|| SettingsRowPayload::new(selection_index)),
                label: &label,
                selection,
                options: &options,
            };
            self.push_setting_row(&mut lines, &mut line_targets, &context, row);
            selection_index += 1;
        }
        self.line_targets = line_targets;
        SettingsRender {
            lines,
            selectable_count: selection_index,
        }
    }

    fn selection_state(&self, selection_index: usize, active: bool) -> SettingsSelectionState {
        if selection_index == self.viewport.pos() && active {
            SettingsSelectionState::Active
        } else if self.viewport.hovered() == Some(selection_index) {
            SettingsSelectionState::Hovered
        } else if selection_index == self.viewport.pos() {
            SettingsSelectionState::Remembered
        } else {
            SettingsSelectionState::Unselected
        }
    }

    fn push_setting_row(
        &self,
        lines: &mut Vec<Line<'static>>,
        line_targets: &mut Vec<Option<SettingsRowPayload>>,
        context: &SettingsLineContext<'_>,
        row: &SettingsRow,
    ) {
        if let Some(error) = context.selected_inline_error(self) {
            push_wrapped_setting_value(
                lines,
                line_targets,
                context,
                error,
                context
                    .selection
                    .patch(context.options, context.options.inline_error_style),
            );
        } else if self.is_editing() && context.selection != SettingsSelectionState::Unselected {
            let edited_text = render_editor_text(self.edited_text(), self.edit_cursor());
            push_wrapped_setting_value(
                lines,
                line_targets,
                context,
                &edited_text,
                context.selection.patch(context.options, Style::default()),
            );
        } else {
            match row.kind {
                SettingsRowKind::Section => {},
                SettingsRowKind::Toggle => {
                    push_toggle_row(
                        lines,
                        line_targets,
                        &row.value,
                        context,
                        row.suffix.as_deref(),
                    );
                },
                SettingsRowKind::Stepper => {
                    push_stepper_row(lines, line_targets, context, &row.value);
                },
                SettingsRowKind::Value => {
                    let value_style = if row.value.starts_with("Not configured.") {
                        context
                            .selection
                            .patch(context.options, context.options.inline_error_style)
                    } else {
                        context.selection.patch(context.options, Style::default())
                    };
                    let value = row.suffix.as_ref().map_or_else(
                        || row.value.clone(),
                        |suffix| format!("{}{suffix}", row.value),
                    );
                    push_wrapped_setting_value(lines, line_targets, context, &value, value_style);
                },
            }
        }
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
    pub fn mode<Ctx: AppContext>(&self, _ctx: &Ctx) -> Mode<Ctx> {
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
    pub fn bar_slots(&self) -> Vec<(BarRegion, BarSlot<SettingsPaneAction>)> {
        SettingsPaneAction::ALL
            .iter()
            .copied()
            .map(|a| (BarRegion::PaneAction, BarSlot::Single(a)))
            .collect()
    }
}

impl Default for SettingsPane {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
impl SettingsPane {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsSelectionState {
    Active,
    Hovered,
    Remembered,
    Unselected,
}

impl SettingsSelectionState {
    fn overlay_style(self, options: &SettingsRenderOptions<'_>) -> Style {
        match self {
            Self::Active => options.active_style,
            Self::Hovered => options.hovered_style,
            Self::Remembered => options.remembered_style,
            Self::Unselected => Style::default(),
        }
    }

    fn patch(self, options: &SettingsRenderOptions<'_>, style: Style) -> Style {
        style.patch(self.overlay_style(options))
    }
}

struct SettingsLineContext<'a> {
    target:    SettingsRowPayload,
    label:     &'a str,
    selection: SettingsSelectionState,
    options:   &'a SettingsRenderOptions<'a>,
}

impl SettingsLineContext<'_> {
    fn selected_inline_error<'a>(&'a self, pane: &SettingsPane) -> Option<&'a str> {
        if self.selection != SettingsSelectionState::Unselected && !pane.is_editing() {
            self.options.inline_error
        } else {
            None
        }
    }
}

fn push_settings_header(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<SettingsRowPayload>>,
    name: &str,
    options: &SettingsRenderOptions<'_>,
) {
    lines.push(Line::from(vec![
        Span::raw(options.section_header_indent.to_string()),
        Span::styled(
            format!("{name}:"),
            options.title_style.add_modifier(Modifier::BOLD),
        ),
    ]));
    line_targets.push(None);
}

fn push_toggle_row(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<SettingsRowPayload>>,
    value: &str,
    context: &SettingsLineContext<'_>,
    suffix: Option<&str>,
) {
    let is_on = value == "ON";
    let toggle_style = if is_on {
        context.options.success_style.add_modifier(Modifier::BOLD)
    } else {
        context.options.error_style.add_modifier(Modifier::BOLD)
    };
    let row_style = context
        .selection
        .patch(context.options, context.options.label_style);
    lines.push(Line::from(vec![
        Span::styled(context.label.to_owned(), row_style),
        Span::styled(
            "< ",
            context
                .selection
                .patch(context.options, context.options.muted_style),
        ),
        Span::styled(
            value.to_owned(),
            context.selection.patch(context.options, toggle_style),
        ),
        Span::styled(
            " >",
            context
                .selection
                .patch(context.options, context.options.muted_style),
        ),
        Span::styled(suffix.unwrap_or_default().to_owned(), row_style),
    ]));
    line_targets.push(Some(context.target));
}

fn push_stepper_row(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<SettingsRowPayload>>,
    context: &SettingsLineContext<'_>,
    value: &str,
) {
    lines.push(Line::from(vec![
        Span::styled(
            context.label.to_owned(),
            context
                .selection
                .patch(context.options, context.options.label_style),
        ),
        Span::styled(
            "< ",
            context
                .selection
                .patch(context.options, context.options.muted_style),
        ),
        Span::styled(
            value.to_owned(),
            context.selection.patch(context.options, Style::default()),
        ),
        Span::styled(
            " >",
            context
                .selection
                .patch(context.options, context.options.muted_style),
        ),
    ]));
    line_targets.push(Some(context.target));
}

fn push_wrapped_setting_value(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<SettingsRowPayload>>,
    context: &SettingsLineContext<'_>,
    value: &str,
    value_style: Style,
) {
    let row = WrappedValueRow {
        prefix: context.label,
        value,
        prefix_style: context
            .selection
            .patch(context.options, context.options.label_style),
        value_style,
        content_width: context.options.content_width,
    };
    push_wrapped_value_row(lines, line_targets, Some(context.target), &row);
}

fn push_wrapped_value_row(
    lines: &mut Vec<Line<'static>>,
    line_targets: &mut Vec<Option<SettingsRowPayload>>,
    target: Option<SettingsRowPayload>,
    row: &WrappedValueRow<'_>,
) {
    let prefix_width = row.prefix.width();
    let value_width = row.content_width.saturating_sub(prefix_width).max(1);
    let wrapped = wrap_text_to_width(row.value, value_width);
    let continuation_prefix = " ".repeat(prefix_width);

    for (index, chunk) in wrapped.into_iter().enumerate() {
        let visible_prefix = if index == 0 {
            row.prefix.to_string()
        } else {
            continuation_prefix.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(visible_prefix, row.prefix_style),
            Span::styled(chunk, row.value_style),
        ]));
        line_targets.push(target);
    }
}

struct WrappedValueRow<'a> {
    prefix:        &'a str,
    value:         &'a str,
    prefix_style:  Style,
    value_style:   Style,
    content_width: usize,
}

fn wrap_text_to_width(value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    if value.trim().is_empty() {
        return vec![String::new()];
    }

    let mut wrapped = Vec::new();
    let mut current = String::new();

    for word in value.split_whitespace() {
        let separator = if current.is_empty() { "" } else { " " };
        let candidate = format!("{current}{separator}{word}");
        if candidate.width() <= width {
            current = candidate;
            continue;
        }

        if !current.is_empty() {
            wrapped.push(std::mem::take(&mut current));
        }

        if word.width() <= width {
            current = word.to_string();
            continue;
        }

        let mut segment = String::new();
        for ch in word.chars() {
            let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if !segment.is_empty() && segment.width() + char_width > width {
                wrapped.push(std::mem::take(&mut segment));
            }
            segment.push(ch);
        }
        current = segment;
    }

    if !current.is_empty() {
        wrapped.push(current);
    }

    if wrapped.is_empty() {
        wrapped.push(String::new());
    }
    wrapped
}

fn render_editor_text(buf: &str, cursor: usize) -> String {
    let mut rendered = String::with_capacity(buf.len() + 1);
    rendered.push_str(&buf[..cursor]);
    rendered.push('_');
    rendered.push_str(&buf[cursor..]);
    rendered
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
    use ratatui::style::Color;
    use ratatui::style::Style;

    use super::SettingsPane;
    use super::SettingsRenderOptions;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::KeyBind;
    use crate::KeyOutcome;
    use crate::Mode;
    use crate::SettingsRow;

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

    fn render_options() -> SettingsRenderOptions<'static> {
        SettingsRenderOptions {
            active:                true,
            inline_error:          None,
            content_width:         24,
            section_header_indent: "",
            section_item_indent:   "",
            title_style:           Style::default().fg(Color::Blue),
            label_style:           Style::default(),
            muted_style:           Style::default(),
            success_style:         Style::default().fg(Color::Green),
            error_style:           Style::default().fg(Color::Red),
            inline_error_style:    Style::default().fg(Color::Red),
            active_style:          Style::default().bg(Color::DarkGray),
            remembered_style:      Style::default().bg(Color::Gray),
            hovered_style:         Style::default().bg(Color::Black),
        }
    }

    #[test]
    fn new_starts_in_browse_mode() {
        let pane = SettingsPane::new();
        let app = fresh_app();
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn editor_target_is_none_at_construction() {
        let pane = SettingsPane::new();
        assert!(pane.editor_target().is_none());
    }

    #[test]
    fn handle_key_always_returns_consumed() {
        let mut pane = SettingsPane::new();
        assert_eq!(pane.handle_key(&KeyBind::from('z')), KeyOutcome::Consumed,);
    }

    #[test]
    fn bar_slots_emit_one_slot_per_variant() {
        let pane = SettingsPane::new();
        let slots = pane.bar_slots();
        assert_eq!(slots.len(), 3);
    }

    #[test]
    fn enter_in_browse_transitions_to_editing() {
        let mut pane = SettingsPane::new();
        let app = fresh_app();
        let _ = pane.handle_key(&KeyCode::Enter.into());
        assert!(matches!(pane.mode(&app), Mode::TextInput(_)));
    }

    #[test]
    fn esc_in_editing_returns_to_browse() {
        let mut pane = SettingsPane::for_test_editing(None);
        let app = fresh_app();
        let _ = pane.handle_key(&KeyCode::Esc.into());
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn save_in_editing_returns_to_browse() {
        let mut pane = SettingsPane::for_test_editing(None);
        let app = fresh_app();
        let _ = pane.handle_key(&KeyBind::from('s'));
        assert!(matches!(pane.mode(&app), Mode::Navigable));
    }

    #[test]
    fn begin_editing_sets_buffer_and_cursor() {
        let mut pane = SettingsPane::new();

        pane.begin_editing("hana".to_string());

        assert_eq!(pane.edit_buffer(), "hana");
        assert_eq!(pane.edit_cursor(), 4);
    }

    #[test]
    fn handle_text_input_mutates_owned_buffer() {
        let mut pane = SettingsPane::new();
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
        let mut pane = SettingsPane::new();
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

    #[test]
    fn render_rows_wraps_continuations_at_value_column() {
        let mut pane = SettingsPane::new();
        let rows = vec![SettingsRow::value(
            0,
            "Projects",
            "alpha beta gamma delta epsilon",
        )];

        let rendered = pane.render_rows(&rows, render_options());

        assert!(rendered.lines.len() > 1);
        assert_eq!(pane.line_target(0), Some(0));
        assert_eq!(pane.line_target(1), Some(0));
        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "▶ Projects  ");
        assert_eq!(rendered.lines[1].spans[0].content.as_ref(), "            ");
    }

    #[test]
    fn render_rows_selected_toggle_inlines_suffix() {
        let mut pane = SettingsPane::new();
        let rows = vec![
            SettingsRow::toggle(0, "Vim nav keys", true)
                .with_suffix("  maps h/j/k/l to arrow navigation"),
        ];

        let rendered = pane.render_rows(&rows, render_options());
        let line = rendered.lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(line.contains("< ON >  maps h/j/k/l to arrow navigation"));
    }

    #[test]
    fn render_rows_editing_shows_cursor_in_buffer() {
        let mut pane = SettingsPane::new();
        pane.begin_editing("hana".to_string());
        pane.set_edit_buffer("hana".to_string(), 2);
        let rows = vec![SettingsRow::value(0, "Editor", "zed")];

        let rendered = pane.render_rows(&rows, render_options());
        let line = rendered.lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(line.contains("ha_na"));
    }
}
