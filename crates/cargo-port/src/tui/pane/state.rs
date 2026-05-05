use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::tui::constants::ACTIVE_FOCUS_COLOR;
use crate::tui::constants::HOVER_FOCUS_COLOR;
use crate::tui::constants::REMEMBERED_FOCUS_COLOR;

/// A bounded cursor for scrollable lists. Replaces raw `usize` index + manual
/// bounds checking with a single type that enforces invariants.
#[derive(Default, Clone)]
pub(super) struct ScrollState {
    pos: usize,
}

impl ScrollState {
    pub(super) const fn pos(&self) -> usize { self.pos }

    pub(super) const fn set(&mut self, pos: usize) { self.pos = pos; }

    pub(super) const fn up(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    pub(super) const fn down(&mut self, len: usize) {
        if len > 0 && self.pos < len - 1 {
            self.pos += 1;
        }
    }

    pub(super) const fn jump_home(&mut self) { self.pos = 0; }

    pub(super) const fn jump_end(&mut self, len: usize) { self.pos = len.saturating_sub(1); }

    /// Clamp position to `0..len`. Useful after the backing list shrinks.
    pub(super) const fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.pos = 0;
        } else if self.pos >= len {
            self.pos = len - 1;
        }
    }
}

/// The shared UI-mechanics state every pane carries: cursor, scroll,
/// viewport rows, content area, hovered row, len.
///
/// Each per-pane struct embeds a `Viewport` and exposes it via the
/// `Pane` trait's `viewport()` / `viewport_mut()` accessors. Default
/// methods on the trait (cursor moves, scroll, hover, etc.) delegate to
/// the embedded `Viewport`, so per-pane impls only write the
/// genuinely-different methods.
#[derive(Default, Clone)]
pub struct Viewport {
    cursor:        ScrollState,
    hovered:       Option<usize>,
    len:           usize,
    content_area:  Rect,
    scroll_offset: usize,
    visible_rows:  usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneFocusState {
    Active,
    Remembered,
    Inactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneSelectionState {
    Active,
    Hovered,
    Remembered,
    Unselected,
}

impl Viewport {
    pub const fn new() -> Self {
        Self {
            cursor:        ScrollState { pos: 0 },
            hovered:       None,
            len:           0,
            content_area:  Rect::new(0, 0, 0, 0),
            scroll_offset: 0,
            visible_rows:  0,
        }
    }

    pub const fn up(&mut self) { self.cursor.up(); }

    pub const fn down(&mut self) { self.cursor.down(self.len); }

    pub const fn home(&mut self) { self.cursor.jump_home(); }

    pub const fn end(&mut self) { self.cursor.jump_end(self.len); }

    pub const fn pos(&self) -> usize { self.cursor.pos() }

    pub const fn set_pos(&mut self, pos: usize) { self.cursor.set(pos); }

    pub const fn set_len(&mut self, len: usize) {
        self.len = len;
        self.cursor.clamp(len);
        if let Some(row) = self.hovered
            && row >= len
        {
            self.hovered = None;
        }
    }

    pub const fn clear_surface(&mut self) {
        self.len = 0;
        self.hovered = None;
        self.content_area = Rect::ZERO;
        self.scroll_offset = 0;
        self.visible_rows = 0;
        self.cursor.clamp(0);
    }

    pub const fn set_content_area(&mut self, area: Rect) { self.content_area = area; }

    pub const fn set_scroll_offset(&mut self, offset: usize) { self.scroll_offset = offset; }

    pub const fn set_viewport_rows(&mut self, rows: usize) { self.visible_rows = rows; }

    pub const fn set_hovered(&mut self, hovered: Option<usize>) { self.hovered = hovered; }

    pub const fn content_area(&self) -> Rect { self.content_area }

    pub const fn scroll_offset(&self) -> usize { self.scroll_offset }

    /// Convert a screen-space position to a local row within this
    /// viewport's content area, accounting for scroll offset and the
    /// pane's `len`. Returns `None` if `pos` is outside the content
    /// area or maps past the last valid row.
    pub const fn pos_to_local_row(&self, pos: Position) -> Option<usize> {
        if self.content_area.width == 0 || self.content_area.height == 0 {
            return None;
        }
        if !self.content_area.contains(pos) {
            return None;
        }
        let visual_row = pos.y.saturating_sub(self.content_area.y);
        let row = self.scroll_offset + visual_row as usize;
        if row >= self.len {
            return None;
        }
        Some(row)
    }

    pub const fn len(&self) -> usize { self.len }

    pub const fn overflow_affordance(&self) -> Option<&'static str> {
        let visible_rows = self.visible_rows;
        if visible_rows == 0 || self.len <= visible_rows {
            return None;
        }

        let has_above = self.scroll_offset > 0;
        let has_below = self.scroll_offset.saturating_add(visible_rows) < self.len;
        match (has_above, has_below) {
            (true, true) => Some("▲ more ▼"),
            (true, false) => Some("▲ more"),
            (false, true) => Some("more ▼"),
            (false, false) => None,
        }
    }

    pub const fn selection_state(&self, row: usize, focus: PaneFocusState) -> PaneSelectionState {
        self.selection_state_for(self.pos(), row, focus)
    }

    /// `selection_state` variant that takes the cursor explicitly,
    /// for callers whose cursor lives outside this viewport (e.g.,
    /// the project-list cursor lives on `Selection.cursor`). The
    /// hovered-row check still reads the viewport's own `hovered`
    /// field — hover is always per-pane.
    pub const fn selection_state_for(
        &self,
        cursor: usize,
        row: usize,
        focus: PaneFocusState,
    ) -> PaneSelectionState {
        if row == cursor && matches!(focus, PaneFocusState::Active) {
            PaneSelectionState::Active
        } else if matches!(self.hovered, Some(hovered_row) if hovered_row == row) {
            PaneSelectionState::Hovered
        } else if row == cursor && matches!(focus, PaneFocusState::Remembered) {
            PaneSelectionState::Remembered
        } else {
            PaneSelectionState::Unselected
        }
    }

    pub fn selection_style(focus: PaneFocusState) -> Style {
        match focus {
            PaneFocusState::Active => Style::default().bg(ACTIVE_FOCUS_COLOR),
            PaneFocusState::Remembered => Style::default().bg(REMEMBERED_FOCUS_COLOR),
            PaneFocusState::Inactive => Style::default(),
        }
    }
}

impl PaneSelectionState {
    pub fn overlay_style(self) -> Style {
        match self {
            Self::Active => Viewport::selection_style(PaneFocusState::Active),
            Self::Hovered => Style::default().bg(HOVER_FOCUS_COLOR),
            Self::Remembered => Viewport::selection_style(PaneFocusState::Remembered),
            Self::Unselected => Style::default(),
        }
    }

    pub fn patch(self, style: Style) -> Style { style.patch(self.overlay_style()) }
}

/// Format a 1-based scroll position as `"{pos+1} of {len}"`.
pub fn scroll_indicator(pos: usize, len: usize) -> String { format!("{} of {len}", pos + 1) }

#[cfg(test)]
mod tests {
    use ratatui::style::Color;
    use ratatui::style::Modifier;
    use ratatui::style::Style;

    use super::PaneFocusState;
    use super::PaneSelectionState;
    use super::Viewport;

    #[test]
    fn active_selection_style_only_adds_background_and_emphasis() {
        let style = Viewport::selection_style(PaneFocusState::Active);

        assert_eq!(style.fg, None);
        assert_eq!(style.bg, Some(super::ACTIVE_FOCUS_COLOR));
        assert_eq!(style.add_modifier, Modifier::default());
    }

    #[test]
    fn selection_patch_preserves_existing_foreground() {
        let base = Style::default().fg(Color::Red);
        let patched = PaneSelectionState::Active.patch(base);

        assert_eq!(patched.fg, Some(Color::Red));
        assert_eq!(patched.bg, Some(super::ACTIVE_FOCUS_COLOR));
        assert_eq!(patched.add_modifier, Modifier::default());
    }

    #[test]
    fn remembered_selection_patch_preserves_existing_foreground() {
        let base = Style::default().fg(Color::Green);
        let patched = PaneSelectionState::Remembered.patch(base);

        assert_eq!(patched.fg, Some(Color::Green));
        assert_eq!(patched.bg, Some(super::REMEMBERED_FOCUS_COLOR));
    }

    #[test]
    fn hovered_selection_patch_preserves_existing_foreground() {
        let base = Style::default().fg(Color::Blue);
        let patched = PaneSelectionState::Hovered.patch(base);

        assert_eq!(patched.fg, Some(Color::Blue));
        assert_eq!(patched.bg, Some(super::HOVER_FOCUS_COLOR));
    }

    #[test]
    fn selection_state_returns_hovered_for_non_selected_hovered_row() {
        let mut pane = Viewport::new();
        pane.set_len(3);
        pane.set_hovered(Some(2));

        assert_eq!(
            pane.selection_state(2, PaneFocusState::Inactive),
            PaneSelectionState::Hovered
        );
    }

    #[test]
    fn selection_state_prefers_cursor_over_hovered_row() {
        let mut pane = Viewport::new();
        pane.set_len(3);
        pane.set_pos(1);
        pane.set_hovered(Some(1));

        assert_eq!(
            pane.selection_state(1, PaneFocusState::Active),
            PaneSelectionState::Active
        );
    }

    #[test]
    fn selection_state_prefers_hover_for_inactive_selected_row() {
        let mut pane = Viewport::new();
        pane.set_len(3);
        pane.set_pos(0);
        pane.set_hovered(Some(0));

        assert_eq!(
            pane.selection_state(0, PaneFocusState::Inactive),
            PaneSelectionState::Hovered
        );
    }

    #[test]
    fn overflow_affordance_is_hidden_when_all_rows_fit() {
        let mut pane = Viewport::new();
        pane.set_len(3);
        pane.set_viewport_rows(3);

        assert_eq!(pane.overflow_affordance(), None);
    }

    #[test]
    fn overflow_affordance_shows_bottom_only_at_top() {
        let mut pane = Viewport::new();
        pane.set_len(5);
        pane.set_viewport_rows(3);
        pane.set_scroll_offset(0);

        assert_eq!(pane.overflow_affordance(), Some("more ▼"));
    }

    #[test]
    fn overflow_affordance_shows_both_in_middle() {
        let mut pane = Viewport::new();
        pane.set_len(7);
        pane.set_viewport_rows(3);
        pane.set_scroll_offset(2);

        assert_eq!(pane.overflow_affordance(), Some("▲ more ▼"));
    }

    #[test]
    fn overflow_affordance_shows_top_only_at_bottom() {
        let mut pane = Viewport::new();
        pane.set_len(5);
        pane.set_viewport_rows(3);
        pane.set_scroll_offset(2);

        assert_eq!(pane.overflow_affordance(), Some("▲ more"));
    }
}
