use ratatui::layout::Position;
use ratatui::layout::Rect;

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

    pub fn up(&mut self) { self.cursor.up(); }

    pub fn down(&mut self) { self.cursor.down(self.len); }

    pub fn home(&mut self) { self.cursor.jump_home(); }

    pub fn end(&mut self) { self.cursor.jump_end(self.len); }

    pub fn clamp(&mut self) { self.cursor.clamp(self.len); }

    pub fn page_up(&mut self) {
        let page = self.content_area.height as usize;
        self.cursor.set(self.cursor.pos().saturating_sub(page));
    }

    pub fn page_down(&mut self) {
        let page = self.content_area.height as usize;
        let new_pos = (self.cursor.pos() + page).min(self.len.saturating_sub(1));
        self.cursor.set(new_pos);
    }

    // -- position --

    pub const fn pos(&self) -> usize { self.cursor.pos() }

    pub fn set_pos(&mut self, pos: usize) { self.cursor.set(pos); }

    // -- length (auto-clamps cursor) --

    pub const fn len(&self) -> usize { self.len }

    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        self.cursor.clamp(len);
    }

    // -- layout --

    pub fn set_content_area(&mut self, area: Rect) { self.content_area = area; }

    pub const fn content_area(&self) -> Rect { self.content_area }

    pub fn set_scroll_offset(&mut self, offset: usize) { self.scroll_offset = offset; }

    // -- mouse --

    pub fn contains(&self, pos: Position) -> bool { self.content_area.contains(pos) }

    /// Map a screen position to a row index within this pane, accounting for
    /// the viewport scroll offset. Returns `None` if the position is outside
    /// the content area or beyond the last row.
    pub fn clicked_row(&self, pos: Position) -> Option<usize> {
        if !self.contains(pos) {
            return None;
        }
        let inner_y = (pos.y - self.content_area.y) as usize;
        let row = self.scroll_offset + inner_y;
        (row < self.len).then_some(row)
    }
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub(super) enum FocusTarget {
    #[default]
    ProjectList,
    DetailFields,
    CiRuns,
    ScanLog,
}

/// Cached layout rectangles from the last render frame, used for mouse
/// hit-testing in the event handler.
#[derive(Default)]
pub(super) struct LayoutCache {
    pub project_list:       Rect,
    pub scan_log:           Option<Rect>,
    pub detail_columns:     Vec<Rect>,
    pub detail_targets_col: Option<usize>,
}
