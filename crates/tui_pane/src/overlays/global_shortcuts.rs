//! `GlobalShortcutsPane`: framework-owned read-only shortcut overlay.
//!
//! The pane owns only generic overlay state: viewport, focus snapshot,
//! local bar actions, and rendering. App-global actions reach this
//! pane through the registered [`Keymap`](crate::Keymap) globals scope.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::constants::GLOBAL_SHORTCUTS_POPUP_MAX_HEIGHT;
use super::constants::GLOBAL_SHORTCUTS_POPUP_MIN_WIDTH;
use super::constants::SHORTCUT_DESCRIPTION_WIDTH;
use crate::AppContext;
use crate::BarRegion;
use crate::BarSlot;
use crate::GlobalShortcutRow;
use crate::Keymap;
use crate::Mode;
use crate::OverlayAction;
use crate::PopupFrame;
use crate::RenderFocus;
use crate::SECTION_HEADER_INDENT;
use crate::SECTION_ITEM_INDENT;
use crate::Viewport;
use crate::ViewportOverflow;
use crate::active_border_color;
use crate::label_color;
use crate::render_overflow_affordance;
use crate::text_default;
use crate::title_color;

struct RenderInputs {
    lines:         Vec<Line<'static>>,
    content_width: u16,
}

/// Framework-owned read-only global-shortcuts overlay.
pub struct GlobalShortcutsPane {
    viewport:  Viewport,
    /// Render-time focus snapshot stamped by the embedding crate's
    /// overlay dispatcher right before render.
    pub focus: RenderFocus,
}

impl GlobalShortcutsPane {
    /// Construct a fresh read-only shortcut overlay.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            focus:    RenderFocus::inactive(),
        }
    }

    /// Borrow the framework-owned viewport state.
    #[must_use]
    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    /// Mutably borrow the framework-owned viewport state.
    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    /// Move the read-only viewport for navigation keys not claimed by
    /// the overlay action scope.
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
                let last = self.viewport.len().saturating_sub(1);
                self.viewport.set_pos(last);
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

    /// Render the global shortcuts modal.
    pub fn render<Ctx>(&mut self, frame: &mut Frame<'_>, area: Rect, keymap: &Keymap<Ctx>)
    where
        Ctx: AppContext + 'static,
    {
        let rows = keymap.global_shortcut_rows();
        let inputs = render_inputs(&rows);
        let width = inputs.content_width.min(area.width.saturating_sub(4));
        let line_count = inputs.lines.len();
        let height = popup_height(line_count, area.height);
        let popup = PopupFrame {
            title: Some(" Global Shortcuts ".to_string()),
            border_color: active_border_color(),
            width,
            height,
        }
        .render_with_areas(frame);
        let inner = popup.inner;

        let visible_height = usize::from(inner.height);
        let max_offset = line_count.saturating_sub(visible_height);
        let scroll_offset = self.viewport.scroll_offset().min(max_offset);
        self.viewport.set_len(line_count);
        self.viewport.set_content_area(inner);
        self.viewport.set_viewport_rows(visible_height);
        self.viewport.set_scroll_offset(scroll_offset);

        let para =
            Paragraph::new(inputs.lines).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
        frame.render_widget(para, inner);
        render_overflow_affordance(
            frame,
            popup.outer,
            ViewportOverflow::new(line_count, scroll_offset, visible_height, scroll_offset),
            Style::default().fg(label_color()),
        );
    }

    /// Current input mode for the overlay.
    #[must_use]
    pub const fn mode<Ctx: AppContext>(&self, _ctx: &Ctx) -> Mode<Ctx> { Mode::Navigable }

    /// Bar slots for the overlay's local actions.
    #[must_use]
    pub fn bar_slots(&self) -> Vec<(BarRegion, BarSlot<OverlayAction>)> {
        vec![(
            BarRegion::PaneAction,
            BarSlot::Single(OverlayAction::Cancel),
        )]
    }
}

impl Default for GlobalShortcutsPane {
    fn default() -> Self { Self::new() }
}

fn render_inputs(rows: &[GlobalShortcutRow]) -> RenderInputs {
    let lines = build_lines(rows);
    let content_width = lines
        .iter()
        .map(line_width)
        .max()
        .and_then(|width| u16::try_from(width.saturating_add(2)).ok())
        .unwrap_or(GLOBAL_SHORTCUTS_POPUP_MIN_WIDTH)
        .max(GLOBAL_SHORTCUTS_POPUP_MIN_WIDTH);
    RenderInputs {
        lines,
        content_width,
    }
}

fn build_lines(rows: &[GlobalShortcutRow]) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from("")];
    let mut current_section = None;
    for row in rows {
        if current_section != Some(row.section) {
            current_section = Some(row.section);
            lines.push(header_line(row.section));
        }
        lines.push(row_line(row));
    }
    lines.push(Line::from(""));
    lines
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

fn row_line<'a>(row: &GlobalShortcutRow) -> Line<'a> {
    let padded_desc = format!(
        "{:<width$}",
        row.description,
        width = SHORTCUT_DESCRIPTION_WIDTH
    );
    let key_display = row
        .key
        .as_ref()
        .map_or_else(String::new, crate::KeySequence::display);
    Line::from(vec![
        Span::styled(
            format!("{SECTION_ITEM_INDENT}  {padded_desc}"),
            Style::default().fg(label_color()),
        ),
        Span::styled(key_display, Style::default().fg(text_default())),
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
