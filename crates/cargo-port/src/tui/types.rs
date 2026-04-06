use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::app::ClickAction;

/// A bounded cursor for scrollable lists. Replaces raw `usize` index + manual
/// bounds checking with a single type that enforces invariants.
#[derive(Default, Clone)]
pub(super) struct ScrollState {
    pos: usize,
}

impl ScrollState {
    pub const fn pos(&self) -> usize { self.pos }

    pub const fn set(&mut self, pos: usize) { self.pos = pos; }

    pub const fn up(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    pub const fn down(&mut self, len: usize) {
        if len > 0 && self.pos < len - 1 {
            self.pos += 1;
        }
    }

    pub const fn jump_home(&mut self) { self.pos = 0; }

    pub const fn jump_end(&mut self, len: usize) { self.pos = len.saturating_sub(1); }

    /// Clamp position to `0..len`. Useful after the backing list shrinks.
    pub const fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.pos = 0;
        } else if self.pos >= len {
            self.pos = len - 1;
        }
    }
}

/// Per-pane state shared by every scrollable panel in the TUI.
///
/// Each pane owns its cursor, knows its row count, and stores the screen
/// region it occupies so navigation and mouse hit-testing are self-contained.
#[derive(Default, Clone)]
pub(super) struct Pane {
    cursor:        ScrollState,
    len:           usize,
    content_area:  Rect,
    scroll_offset: usize,
}

impl Pane {
    pub const fn new() -> Self {
        Self {
            cursor:        ScrollState { pos: 0 },
            len:           0,
            content_area:  Rect::new(0, 0, 0, 0),
            scroll_offset: 0,
        }
    }

    // -- navigation (pane knows its own len) --

    pub const fn up(&mut self) { self.cursor.up(); }

    pub const fn down(&mut self) { self.cursor.down(self.len); }

    pub const fn home(&mut self) { self.cursor.jump_home(); }

    pub const fn end(&mut self) { self.cursor.jump_end(self.len); }

    // -- position --

    pub const fn pos(&self) -> usize { self.cursor.pos() }

    pub const fn set_pos(&mut self, pos: usize) { self.cursor.set(pos); }

    // -- length (auto-clamps cursor) --

    pub const fn set_len(&mut self, len: usize) {
        self.len = len;
        self.cursor.clamp(len);
    }

    // -- layout --

    pub const fn set_content_area(&mut self, area: Rect) { self.content_area = area; }

    pub const fn set_scroll_offset(&mut self, offset: usize) { self.scroll_offset = offset; }

    // -- mouse --

    /// Map a screen position to a row index within this pane, accounting for
    /// the viewport scroll offset. Returns `None` if the position is outside
    /// the content area or beyond the last row.
    pub const fn clicked_row(&self, pos: Position) -> Option<usize> {
        if !self.content_area.contains(pos) {
            return None;
        }
        let inner_y = (pos.y - self.content_area.y) as usize;
        let row = self.scroll_offset + inner_y;
        if row < self.len { Some(row) } else { None }
    }
}

/// Format a 1-based scroll position as `"{pos+1} of {len}"`.
pub(super) fn scroll_indicator(pos: usize, len: usize) -> String { format!("{} of {len}", pos + 1) }

#[derive(Default, PartialEq, Eq, Clone, Copy, Debug, Hash)]
pub(super) enum PaneId {
    #[default]
    ProjectList,
    Package,
    Git,
    Targets,
    CiRuns,
    Toasts,
    Search,
    Settings,
    Finder,
    Keymap,
}

impl PaneId {
    pub const fn is_overlay(self) -> bool {
        matches!(self, Self::Search | Self::Settings | Self::Finder)
    }
}

/// Toast card focus hitbox (separate from dismiss — card click changes focus).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ToastCardHitbox {
    pub id:        u64,
    pub card_rect: Rect,
}

/// Cached layout rectangles from the last render frame, used for mouse
/// hit-testing in the event handler.
#[derive(Default)]
pub(super) struct LayoutCache {
    pub project_list:        Rect,
    pub project_list_offset: usize,
    pub detail_columns:      Vec<Rect>,
    pub detail_targets_col:  Option<usize>,
    pub dismiss_hitboxes:    Vec<ClickAction>,
    pub toast_cards:         Vec<ToastCardHitbox>,
}
