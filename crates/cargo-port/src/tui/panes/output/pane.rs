use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::CopySelectionResult;
use tui_pane::Hittable;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use crate::tui::hit_test::HoverTarget;
use crate::tui::panes;
use crate::tui::panes::PaneId;
use crate::tui::render_context::PaneRenderCtx;

// ── Output ──────────────────────────────────────────────────────

/// The output pane's selection sub-mode.
///
/// In `Normal` the selection is the single row under the cursor and plain
/// motions move it whole (the anchor follows the cursor). In `Visual` —
/// the vim visual-line sub-mode (`V`) — plain motions grow the range from
/// the fixed anchor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectionMode {
    Normal,
    Visual,
}

/// Linewise selection state for the output pane.
///
/// There is always a selection — at minimum the single row under the
/// cursor — so the pane has no separate select/deselect mode. `anchor`
/// is the fixed end; the moving end is [`OutputPane::viewport`]'s `pos`,
/// and the selected range runs between them. `mode` is the
/// [`SelectionMode`] that decides how plain motions read.
///
/// `snapshot` freezes the buffer once the selection stops tracking the
/// live tail, so a streaming child process can't drift a pinned range.
/// While the selection follows the tail it stays `None` and render/yank
/// read the live buffer.
pub struct OutputSelection {
    anchor:         usize,
    selection_mode: SelectionMode,
    snapshot:       Option<Rc<[String]>>,
}

impl OutputSelection {
    const fn new() -> Self {
        Self {
            anchor:         0,
            selection_mode: SelectionMode::Normal,
            snapshot:       None,
        }
    }

    /// Whether the vim visual-line sub-mode is active.
    pub const fn is_visual(&self) -> bool { matches!(self.selection_mode, SelectionMode::Visual) }

    /// The frozen buffer snapshot, present once the selection has stopped
    /// following the live tail.
    pub const fn snapshot(&self) -> Option<&Rc<[String]>> { self.snapshot.as_ref() }
}

pub struct OutputPane {
    pub viewport: Viewport,
    pub focus:    RenderFocus,
    selection:    OutputSelection,
}

impl OutputPane {
    pub const fn new() -> Self {
        Self {
            viewport:  Viewport::new(),
            focus:     RenderFocus::inactive(),
            selection: OutputSelection::new(),
        }
    }

    /// The current selection state.
    pub const fn selection(&self) -> &OutputSelection { &self.selection }

    /// Whether the single-row selection is pinned to the streaming tail:
    /// not in visual mode and the cursor on the last row. Following means
    /// render and yank track the live tail.
    pub const fn is_following(&self) -> bool {
        matches!(self.selection.selection_mode, SelectionMode::Normal)
            && self.viewport.pos() >= self.viewport.len().saturating_sub(1)
    }

    /// Reset to the open-time state: a collapsed selection following the
    /// streaming tail.
    pub fn reset_for_open(&mut self) {
        self.selection = OutputSelection::new();
        self.viewport.end();
    }

    /// The source the selection reads from: the frozen snapshot once
    /// pinned, otherwise the live buffer it is following.
    fn source<'a>(&'a self, live: &'a [String]) -> &'a [String] {
        self.selection.snapshot.as_deref().unwrap_or(live)
    }

    /// Freeze the live buffer into the snapshot if it is not already
    /// frozen — called whenever the selection stops following the tail.
    fn freeze(&mut self, live: &[String]) {
        if self.selection.snapshot.is_none() {
            self.selection.snapshot = Some(Rc::from(live.to_vec()));
        }
    }

    /// Enter visual mode from the cursor: anchor the fixed end at the
    /// current cursor row and freeze `live`. A no-op when already in visual
    /// mode, so a started range keeps its anchor. The `anchor` field is
    /// meaningful only in [`SelectionMode::Visual`]; entering this mode is
    /// the one place it is set, so it can never drift from a plain cursor
    /// move.
    fn enter_visual(&mut self, live: &[String]) {
        if matches!(self.selection.selection_mode, SelectionMode::Normal) {
            self.selection.selection_mode = SelectionMode::Visual;
            self.selection.anchor = self.viewport.pos();
            self.freeze(live);
        }
    }

    /// Toggle the vim visual-line sub-mode. Entering anchors at the cursor
    /// and freezes `live`; leaving collapses the selection back to the
    /// single cursor row. Vim-mode only — bound to `V`.
    pub fn toggle_visual(&mut self, live: &[String]) {
        match self.selection.selection_mode {
            SelectionMode::Visual => self.exit_visual(),
            SelectionMode::Normal => self.enter_visual(live),
        }
    }

    /// Leave visual mode, collapsing the selection back to the single
    /// cursor row. A no-op when not in visual mode. Bound to `Esc` while a
    /// visual selection is active.
    pub const fn exit_visual(&mut self) { self.selection.selection_mode = SelectionMode::Normal; }

    /// Select every line: anchor on the first row, cursor on the last, so
    /// the range spans the whole buffer. Freezes `live` first. Bound to
    /// Ctrl-A.
    pub fn select_all(&mut self, live: &[String]) {
        self.freeze(live);
        self.selection.selection_mode = SelectionMode::Visual;
        self.selection.anchor = 0;
        let last = self.source(live).len().saturating_sub(1);
        self.viewport.set_pos(last);
    }

    /// Apply a plain navigation motion. In visual mode the motion grows
    /// the range from the anchor; otherwise it moves the single-row
    /// selection, which re-follows the tail when it lands on the last row
    /// or freezes `live` when it parks off the tail.
    pub fn navigate(&mut self, live: &[String], motion: impl FnOnce(&mut Viewport)) {
        motion(&mut self.viewport);
        match self.selection.selection_mode {
            SelectionMode::Visual => self.freeze(live),
            SelectionMode::Normal => {
                if self.viewport.pos() >= self.viewport.len().saturating_sub(1) {
                    self.selection.snapshot = None;
                } else {
                    self.freeze(live);
                }
            },
        }
    }

    /// Extend the selection up one row, entering visual mode at the cursor
    /// first if needed. Bound to Shift+Up: the editor-style select gesture.
    pub fn select_extend_up(&mut self, live: &[String]) {
        self.enter_visual(live);
        self.viewport.up();
    }

    /// Extend the selection down one row, the mirror of
    /// [`select_extend_up`](Self::select_extend_up). Bound to Shift+Down.
    pub fn select_extend_down(&mut self, live: &[String]) {
        self.enter_visual(live);
        self.viewport.down();
    }

    /// Extend the selection from the cursor to the first row. Bound to
    /// Ctrl+Shift+Up.
    pub fn select_extend_to_top(&mut self, live: &[String]) {
        self.enter_visual(live);
        self.viewport.home();
    }

    /// Extend the selection from the cursor to the last row, the mirror of
    /// [`select_extend_to_top`](Self::select_extend_to_top). Bound to
    /// Ctrl+Shift+Down.
    pub fn select_extend_to_bottom(&mut self, live: &[String]) {
        self.enter_visual(live);
        self.viewport.end();
    }

    /// Position the selection on `row` (a buffer index from
    /// [`Viewport::pos_to_local_row`]) as a fresh left-button press does:
    /// collapse any visual range back to the single clicked line — Normal
    /// mode, anchor on that row — so a release-then-click starts a new
    /// selection rather than extending the old one. Re-follows the tail
    /// when `row` is the last line, freezes `live` otherwise.
    pub fn click_select_row(&mut self, live: &[String], row: usize) {
        self.selection.selection_mode = SelectionMode::Normal;
        self.viewport.set_pos(row);
        self.selection.anchor = row;
        if self.viewport.pos() >= self.viewport.len().saturating_sub(1) {
            self.selection.snapshot = None;
        } else {
            self.freeze(live);
        }
    }

    /// Extend a mouse drag-select to `row` (a buffer index from
    /// [`Viewport::pos_to_local_row`]), entering visual mode anchored at
    /// the press row (the cursor [`click_select_row`](Self::click_select_row)
    /// just positioned) on the first call. Bound to a left-button drag in
    /// the output pane.
    pub fn select_drag_to(&mut self, live: &[String], row: usize) {
        self.enter_visual(live);
        self.viewport.set_pos(row);
    }

    /// Collapse the selection back to the single cursor row and resume
    /// following the tail. Used after a yank, where returning to the live
    /// tail is the expected next state.
    pub fn collapse_to_tail(&mut self) {
        self.selection = OutputSelection::new();
        self.viewport.end();
    }

    /// Number of lines the selection spans against `live` (the frozen
    /// snapshot when pinned). At rest this is `1` — the cursor row.
    pub fn selection_line_count(&self, live: &[String]) -> usize {
        self.selected_range(live).map_or(0, |(lo, hi)| hi - lo + 1)
    }

    /// Inclusive `[lo, hi]` row range of the selection, clamped to the
    /// source bounds (the frozen snapshot when pinned, else `live`).
    /// Outside visual mode the range is the single cursor row; the
    /// `anchor` is read only in visual mode. `None` only when the buffer
    /// is empty.
    pub fn selected_range(&self, live: &[String]) -> Option<(usize, usize)> {
        let last = self.source(live).len().checked_sub(1)?;
        let cursor = self.viewport.pos().min(last);
        match self.selection.selection_mode {
            SelectionMode::Visual => {
                let anchor = self.selection.anchor.min(last);
                Some((anchor.min(cursor), anchor.max(cursor)))
            },
            SelectionMode::Normal => Some((cursor, cursor)),
        }
    }

    /// Build the clipboard payload for the current selection, reading the
    /// frozen snapshot when pinned or `live` while following the tail.
    pub fn copy_payload(&self, live: &[String]) -> CopySelectionResult {
        let Some((lo, hi)) = self.selected_range(live) else {
            return CopySelectionResult::Nothing;
        };
        panes::copy_payload_for_output(self.source(live), lo, hi)
    }

    /// Resume following the tail when a process exits, unless the user is
    /// in visual mode selecting to copy. A collapsed single-row selection —
    /// the at-rest state, whether following or just scrolled — snaps to the
    /// new tail so the final output shows.
    pub fn on_process_exit(&mut self) {
        if matches!(self.selection.selection_mode, SelectionMode::Normal) {
            self.selection.snapshot = None;
            self.viewport.end();
        }
    }

    /// Sync the viewport surface to the rendered rows and compute the
    /// scroll offset. While the collapsed selection follows the tail, the
    /// cursor (and its anchor) stick to the new last row so streaming
    /// output stays visible; otherwise the offset keeps the cursor on
    /// screen at its pinned position.
    pub const fn sync_viewport(&mut self, len: usize, visible_rows: usize, content_area: Rect) {
        let following = self.is_following();
        self.viewport.set_len(len);
        self.viewport.set_viewport_rows(visible_rows);
        self.viewport.set_content_area(content_area);
        if following {
            self.viewport.end();
            self.selection.anchor = self.viewport.pos();
        }
        self.viewport.set_scroll_offset(scroll_to_show_cursor(
            self.viewport.pos(),
            self.viewport.scroll_offset(),
            visible_rows,
            len,
        ));
    }
}

/// Smallest scroll offset that keeps `cursor` on screen, starting from
/// the `current` offset and clamped so the view never scrolls past the
/// end.
const fn scroll_to_show_cursor(
    cursor: usize,
    current: usize,
    visible_rows: usize,
    len: usize,
) -> usize {
    if visible_rows == 0 {
        return 0;
    }
    let mut offset = if cursor < current { cursor } else { current };
    if cursor + 1 > offset + visible_rows {
        offset = cursor + 1 - visible_rows;
    }
    let max_offset = len.saturating_sub(visible_rows);
    if offset > max_offset {
        max_offset
    } else {
        offset
    }
}

impl Renderable<PaneRenderCtx<'_>> for OutputPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        super::render_output_pane_body(frame, area, self, ctx);
    }
}

impl Hittable<HoverTarget> for OutputPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Output,
            row,
        })
    }
}
