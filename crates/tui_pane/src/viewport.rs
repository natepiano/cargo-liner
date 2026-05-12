//! Framework-owned viewport state for built-in panes.

use ratatui::layout::Position;
use ratatui::layout::Rect;

/// Cursor, hover, and rendered-area state for framework-owned panes.
#[derive(Clone, Debug, Default)]
pub struct Viewport {
    pos:           usize,
    hovered:       Option<usize>,
    len:           usize,
    content_area:  Rect,
    scroll_offset: usize,
    visible_rows:  usize,
}

impl Viewport {
    /// Construct an empty viewport.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pos:           0,
            hovered:       None,
            len:           0,
            content_area:  Rect::ZERO,
            scroll_offset: 0,
            visible_rows:  0,
        }
    }

    /// Move the cursor up one row.
    pub const fn up(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    /// Move the cursor down one row.
    pub const fn down(&mut self) {
        if self.len > 0 && self.pos < self.len - 1 {
            self.pos += 1;
        }
    }

    /// Move the cursor to the first row.
    pub const fn home(&mut self) { self.pos = 0; }

    /// Move the cursor to the last row.
    pub const fn end(&mut self) { self.pos = self.len.saturating_sub(1); }

    /// Current cursor row.
    #[must_use]
    pub const fn pos(&self) -> usize { self.pos }

    /// Set the current cursor row.
    pub const fn set_pos(&mut self, pos: usize) { self.pos = pos; }

    /// Set the backing row count.
    pub const fn set_len(&mut self, len: usize) {
        self.len = len;
        if len == 0 {
            self.pos = 0;
        } else if self.pos >= len {
            self.pos = len - 1;
        }
        if let Some(row) = self.hovered
            && row >= len
        {
            self.hovered = None;
        }
    }

    /// Current backing row count.
    #[must_use]
    pub const fn len(&self) -> usize { self.len }

    /// Whether the backing row set is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool { self.len == 0 }

    /// Set the screen-space content area.
    pub const fn set_content_area(&mut self, area: Rect) { self.content_area = area; }

    /// Screen-space content area.
    #[must_use]
    pub const fn content_area(&self) -> Rect { self.content_area }

    /// Set the current scroll offset.
    pub const fn set_scroll_offset(&mut self, offset: usize) { self.scroll_offset = offset; }

    /// Current scroll offset.
    #[must_use]
    pub const fn scroll_offset(&self) -> usize { self.scroll_offset }

    /// Set the visible row count.
    pub const fn set_viewport_rows(&mut self, rows: usize) { self.visible_rows = rows; }

    /// Visible row count.
    #[must_use]
    pub const fn visible_rows(&self) -> usize { self.visible_rows }

    /// Set the currently hovered row.
    pub const fn set_hovered(&mut self, hovered: Option<usize>) { self.hovered = hovered; }

    /// Currently hovered row.
    #[must_use]
    pub const fn hovered(&self) -> Option<usize> { self.hovered }

    /// Convert a screen-space position to a row in this viewport.
    #[must_use]
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
}
