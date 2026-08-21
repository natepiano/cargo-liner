//! `GlobalShortcutsPane`: framework-owned selectable shortcut overlay.
//!
//! The pane owns generic overlay state: selectable-row geometry,
//! viewport, focus snapshot, local bar actions, and rendering.
//! App-global actions reach this pane through the registered
//! [`Keymap`](crate::Keymap) globals scope.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::constants::GLOBAL_SHORTCUTS_POPUP_MAX_HEIGHT;
use super::constants::GLOBAL_SHORTCUTS_POPUP_MIN_WIDTH;
use super::constants::GLOBAL_SHORTCUTS_RIGHT_PADDING_WIDTH;
use super::constants::SHORTCUT_DESCRIPTION_WIDTH;
use crate::Action;
use crate::AppContext;
use crate::BLOCK_BORDER_WIDTH;
use crate::BarRegion;
use crate::BarSlot;
use crate::GlobalShortcutRow;
use crate::Keymap;
use crate::Mode;
use crate::OverlayAction;
use crate::PaneSelectionState;
use crate::PopupFrame;
use crate::RenderFocus;
use crate::SECTION_HEADER_INDENT;
use crate::SECTION_ITEM_INDENT;
use crate::Viewport;
use crate::ViewportOverflow;
use crate::label_color;
use crate::layout;
use crate::render_overflow_affordance;
use crate::text_default;
use crate::title_color;

struct RenderInputs {
    lines:          Vec<Line<'static>>,
    line_targets:   Vec<Option<usize>>,
    selectable_len: usize,
    content_width:  u16,
}

/// Framework-owned selectable global-shortcuts overlay.
pub struct GlobalShortcutsPane {
    line_targets: Vec<Option<usize>>,
    viewport:     Viewport,
    /// Render-time focus snapshot stamped by the embedding crate's
    /// overlay dispatcher right before render.
    pub focus:    RenderFocus,
}

impl GlobalShortcutsPane {
    /// Construct a fresh selectable shortcut overlay.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            line_targets: Vec::new(),
            viewport:     Viewport::new(),
            focus:        RenderFocus::inactive(),
        }
    }

    /// Borrow the framework-owned viewport state.
    #[must_use]
    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    /// Mutably borrow the framework-owned viewport state.
    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    /// Move the selection for navigation keys not claimed by the
    /// overlay action scope.
    pub fn handle_navigation_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => self.viewport.up(),
            KeyCode::Down => {
                self.viewport.down();
            },
            KeyCode::Home => {
                self.viewport.home();
            },
            KeyCode::End => {
                self.viewport.end();
            },
            KeyCode::PageUp => {
                self.viewport.page_up();
            },
            KeyCode::PageDown => {
                self.viewport.page_down();
            },
            _ => {},
        }
    }

    /// Selected action row rendered at `line`, if the line is selectable.
    #[must_use]
    pub fn line_target(&self, line: usize) -> Option<usize> {
        self.line_targets.get(line).copied().flatten()
    }

    /// First rendered line for the given selectable action row.
    #[must_use]
    pub fn line_for_selection(&self, selection: usize) -> Option<usize> {
        self.line_targets
            .iter()
            .position(|target| *target == Some(selection))
    }

    /// Selectable action row at `pos`, if the position is inside an action line.
    #[must_use]
    pub fn row_at(&self, pos: Position) -> Option<usize> {
        let inner = self.viewport.content_area();
        if inner.width == 0 || inner.height == 0 || !inner.contains(pos) {
            return None;
        }
        let line = usize::from(pos.y.saturating_sub(inner.y)) + self.viewport.scroll_offset();
        self.line_target(line)
    }

    /// Render the global shortcuts modal.
    pub fn render<Ctx>(&mut self, frame: &mut Frame<'_>, area: Rect, keymap: &Keymap<Ctx>)
    where
        Ctx: AppContext + 'static,
    {
        let rows = keymap.global_shortcut_rows();
        self.viewport.set_len(rows.len());
        let inputs = render_inputs(&rows, &self.viewport, self.focus.pane_focus_state);
        let width = inputs.content_width.min(area.width.saturating_sub(4));
        let line_count = inputs.lines.len();
        let height = popup_height(line_count, area.height);
        let popup = PopupFrame {
            title: Some(" Global Shortcuts ".to_string()),
            border_color: title_color(),
            width,
            height,
        }
        .render_with_areas(frame);
        let inner = popup.inner;

        let visible_height = usize::from(inner.height);
        self.viewport.set_len(inputs.selectable_len);
        self.viewport.set_content_area(inner);
        self.viewport.set_viewport_rows(visible_height);
        self.line_targets.clone_from(&inputs.line_targets);
        let selected_line = self
            .line_for_selection(self.viewport.pos())
            .unwrap_or_else(|| self.viewport.pos());
        let scroll_offset =
            layout::keep_visible_scroll_offset(selected_line, visible_height, line_count);
        self.viewport.set_scroll_offset(scroll_offset);

        let para =
            Paragraph::new(inputs.lines).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
        frame.render_widget(para, inner);
        render_overflow_affordance(
            frame,
            popup.outer,
            ViewportOverflow::new(line_count, scroll_offset, visible_height, selected_line),
            Style::default().fg(label_color()),
        );
    }

    /// Current input mode for the overlay.
    #[must_use]
    pub const fn mode<Ctx: AppContext>(&self, _: &Ctx) -> Mode<Ctx> { Mode::Navigable }

    /// Bar slots for the overlay's local actions.
    #[must_use]
    pub fn bar_slots(&self) -> Vec<(BarRegion, BarSlot<OverlayAction>)> {
        OverlayAction::ALL
            .iter()
            .copied()
            .map(|action| (BarRegion::PaneAction, BarSlot::Single(action)))
            .collect()
    }
}

impl Default for GlobalShortcutsPane {
    fn default() -> Self { Self::new() }
}

fn render_inputs(
    rows: &[GlobalShortcutRow],
    viewport: &Viewport,
    focus: crate::PaneFocusState,
) -> RenderInputs {
    let (lines, line_targets) = build_lines(rows, viewport, focus);
    let content_width = lines
        .iter()
        .map(line_width)
        .max()
        .and_then(|width| {
            let width = width
                .saturating_add(BLOCK_BORDER_WIDTH)
                .saturating_add(GLOBAL_SHORTCUTS_RIGHT_PADDING_WIDTH);
            u16::try_from(width).ok()
        })
        .unwrap_or(GLOBAL_SHORTCUTS_POPUP_MIN_WIDTH)
        .max(GLOBAL_SHORTCUTS_POPUP_MIN_WIDTH);
    RenderInputs {
        lines,
        line_targets,
        selectable_len: rows.len(),
        content_width,
    }
}

fn build_lines(
    rows: &[GlobalShortcutRow],
    viewport: &Viewport,
    focus: crate::PaneFocusState,
) -> (Vec<Line<'static>>, Vec<Option<usize>>) {
    let mut lines = vec![Line::from("")];
    let mut line_targets = vec![None];
    let mut current_section = None;
    for (selectable_index, row) in rows.iter().enumerate() {
        if current_section != Some(row.section) {
            current_section = Some(row.section);
            lines.push(header_line(row.section));
            line_targets.push(None);
        }
        let selection = crate::selection_state(viewport, selectable_index, focus);
        lines.push(row_line(row, selection));
        line_targets.push(Some(selectable_index));
    }
    lines.push(Line::from(""));
    line_targets.push(None);
    (lines, line_targets)
}

fn header_line<'a>(section: &'static str) -> Line<'a> {
    Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            format!("{section}:"),
            Style::default()
                .fg(title_color())
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn row_line<'a>(row: &GlobalShortcutRow, selection: PaneSelectionState) -> Line<'a> {
    let padded_desc = format!(
        "{:<width$}",
        row.description,
        width = SHORTCUT_DESCRIPTION_WIDTH
    );
    let key_display = row.key.as_ref().map_or_else(String::new, |bind| {
        bind.platform_label(crate::AltModifierLabel::current())
    });
    let marker = if selection == PaneSelectionState::Unselected {
        "  "
    } else {
        "▸ "
    };
    Line::from(vec![
        Span::styled(
            format!("{SECTION_ITEM_INDENT}{marker}{padded_desc}"),
            selection.patch(Style::default().fg(label_color())),
        ),
        Span::styled(
            key_display,
            selection.patch(Style::default().fg(text_default())),
        ),
    ])
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

fn popup_height(row_count: usize, area_height: u16) -> u16 {
    let content_height = u16::try_from(row_count).unwrap_or(u16::MAX);
    content_height
        .saturating_add(2)
        .min(area_height.saturating_sub(2))
        .min(GLOBAL_SHORTCUTS_POPUP_MAX_HEIGHT)
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::GLOBAL_SHORTCUTS_POPUP_MIN_WIDTH;
    use super::GLOBAL_SHORTCUTS_RIGHT_PADDING_WIDTH;
    use super::SHORTCUT_DESCRIPTION_WIDTH;
    use super::line_width;
    use super::render_inputs;
    use super::row_line;
    use crate::BLOCK_BORDER_WIDTH;
    use crate::GlobalShortcutRow;
    use crate::KeyBind;
    use crate::KeySequence;
    use crate::PaneFocusState;
    use crate::PaneSelectionState;
    use crate::SECTION_ITEM_INDENT;
    use crate::Viewport;

    #[test]
    fn long_descriptions_keep_space_before_the_key_column() {
        let row = GlobalShortcutRow {
            section:     "Global Shortcuts",
            scope:       "global",
            action:      "pause_all_lints",
            description: "Pause or resume selected lints",
            key:         Some(KeySequence::from(KeyBind::shift(' '))),
        };

        let line = row_line(&row, PaneSelectionState::Unselected);
        let description_width = line.spans[0].content.as_ref().width();
        let indent_width = format!("{SECTION_ITEM_INDENT}  ").width();

        assert_eq!(description_width, indent_width + SHORTCUT_DESCRIPTION_WIDTH);
    }

    #[test]
    fn popup_width_keeps_right_padding_after_the_longest_key() {
        let rows = [GlobalShortcutRow {
            section:     "Global Shortcuts",
            scope:       "global",
            action:      "pause_all_lints",
            description: "Pause or resume all lints",
            key:         Some(KeySequence::from(KeyBind::shift(' '))),
        }];
        let inputs = render_inputs(&rows, &Viewport::new(), PaneFocusState::Active);
        let longest_line = inputs.lines.iter().map(line_width).max().unwrap_or(0);
        let expected = longest_line
            .saturating_add(BLOCK_BORDER_WIDTH)
            .saturating_add(GLOBAL_SHORTCUTS_RIGHT_PADDING_WIDTH)
            .max(usize::from(GLOBAL_SHORTCUTS_POPUP_MIN_WIDTH));

        assert_eq!(usize::from(inputs.content_width), expected);
    }
}
