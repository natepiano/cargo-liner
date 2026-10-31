use std::rc::Rc;

use ratatui::layout::Rect;

use crate::Viewport;

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

/// Which buffer the visual selection reads.
///
/// A selection that has stopped tracking the streaming tail reads the buffer it
/// was frozen against, so a child process still writing cannot move a range the
/// user already picked.
#[derive(Clone)]
pub enum VisualSelectionSource {
    /// The selection follows the tail and reads whatever output is live.
    LiveOutput,
    /// The selection is pinned against this frozen buffer.
    Frozen(Rc<[String]>),
}

impl VisualSelectionSource {
    /// The lines the selection and the copy payload read.
    #[must_use]
    pub fn lines<'a>(&'a self, live: &'a [String]) -> &'a [String] {
        match self {
            Self::LiveOutput => live,
            Self::Frozen(frozen) => frozen,
        }
    }
}

/// The rows the selection currently covers.
///
/// An empty buffer has no rows to name, which is a different fact from a
/// one-row selection at index zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputSelectionRange {
    /// There is nothing to select.
    Empty,
    /// The inclusive row range the selection covers.
    Rows {
        /// First selected row, inclusive.
        first: usize,
        /// Last selected row, inclusive.
        last:  usize,
    },
}

impl OutputSelectionRange {
    /// How many rows the range covers.
    #[must_use]
    pub const fn line_count(self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Rows { first, last } => last - first + 1,
        }
    }

    /// Whether `row` is inside the range.
    #[must_use]
    pub const fn contains(self, row: usize) -> bool {
        match self {
            Self::Empty => false,
            Self::Rows { first, last } => row >= first && row <= last,
        }
    }
}

/// Linewise selection state for the output pane.
///
/// There is always a selection — at minimum the single row under the
/// cursor — so the pane has no separate select/deselect mode. `anchor`
/// is the fixed end; the moving end is `StreamingTextSelection::viewport`'s `pos`,
/// and the selected range runs between them. `mode` is the
/// selection mode that decides how plain motions read.
///
/// `visual_selection_source` names which buffer the range reads, so a streaming
/// child process can't drift a pinned range.
pub struct OutputSelection {
    anchor:                  usize,
    selection_mode:          SelectionMode,
    visual_selection_source: VisualSelectionSource,
}

impl OutputSelection {
    const fn new() -> Self {
        Self {
            anchor:                  0,
            selection_mode:          SelectionMode::Normal,
            visual_selection_source: VisualSelectionSource::LiveOutput,
        }
    }

    /// Whether the vim visual-line sub-mode is active.
    #[must_use]
    pub const fn is_visual(&self) -> bool { matches!(self.selection_mode, SelectionMode::Visual) }

    /// Which buffer the selection reads.
    #[must_use]
    pub const fn visual_selection_source(&self) -> &VisualSelectionSource {
        &self.visual_selection_source
    }
}

/// Selection and viewport state over a live or frozen stream of text lines.
pub struct StreamingTextSelection {
    /// Cursor, scroll position and the rendered text surface.
    pub viewport: Viewport,
    selection:    OutputSelection,
}

impl Default for StreamingTextSelection {
    fn default() -> Self { Self::new() }
}

impl StreamingTextSelection {
    /// Start with a collapsed selection following the stream's tail.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            viewport:  Viewport::new(),
            selection: OutputSelection::new(),
        }
    }

    /// The current selection state.
    #[must_use]
    pub const fn selection(&self) -> &OutputSelection { &self.selection }

    /// Whether the single-row selection is pinned to the streaming tail:
    /// not in visual mode and the cursor on the last row. Following means
    /// render and yank track the live tail.
    #[must_use]
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

    /// The source the selection reads from: the frozen buffer once
    /// pinned, otherwise the live buffer it is following.
    fn source<'a>(&'a self, live: &'a [String]) -> &'a [String] {
        self.selection.visual_selection_source.lines(live)
    }

    /// Freeze the live buffer if the selection is not already pinned —
    /// called whenever the selection stops following the tail.
    fn freeze(&mut self, live: &[String]) {
        if matches!(
            self.selection.visual_selection_source,
            VisualSelectionSource::LiveOutput
        ) {
            self.selection.visual_selection_source =
                VisualSelectionSource::Frozen(Rc::from(live.to_vec()));
        }
    }

    /// Resume reading the live buffer, which the collapsed selection does
    /// whenever it lands back on the streaming tail.
    fn follow_live(&mut self) {
        self.selection.visual_selection_source = VisualSelectionSource::LiveOutput;
    }

    /// Enter visual mode from the cursor: anchor the fixed end at the
    /// current cursor row and freeze `live`. A no-op when already in visual
    /// mode, so a started range keeps its anchor. The `anchor` field is
    /// meaningful only in visual mode; entering this mode is
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
                    self.follow_live();
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
            self.follow_live();
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
    #[must_use]
    pub fn selection_line_count(&self, live: &[String]) -> usize {
        self.selected_range(live).line_count()
    }

    /// The rows the selection covers, clamped to the source bounds (the frozen
    /// buffer when pinned, else `live`). Outside visual mode the range is the
    /// single cursor row; the `anchor` is read only in visual mode.
    #[must_use]
    pub fn selected_range(&self, live: &[String]) -> OutputSelectionRange {
        let Some(last) = self.source(live).len().checked_sub(1) else {
            return OutputSelectionRange::Empty;
        };
        let cursor = self.viewport.pos().min(last);
        match self.selection.selection_mode {
            SelectionMode::Visual => {
                let anchor = self.selection.anchor.min(last);
                OutputSelectionRange::Rows {
                    first: anchor.min(cursor),
                    last:  anchor.max(cursor),
                }
            },
            SelectionMode::Normal => OutputSelectionRange::Rows {
                first: cursor,
                last:  cursor,
            },
        }
    }

    /// Resume following the tail when a stream finishes, unless the user is
    /// in visual mode selecting to copy. A collapsed single-row selection —
    /// the at-rest state, whether following or just scrolled — snaps to the
    /// new tail so the final output shows.
    pub fn finish_stream(&mut self) {
        if matches!(self.selection.selection_mode, SelectionMode::Normal) {
            self.follow_live();
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

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::OutputSelectionRange;
    use super::StreamingTextSelection;
    use super::scroll_to_show_cursor;
    use crate::Viewport;

    fn lines() -> Vec<String> { vec!["first".into(), "second".into(), "third".into()] }

    #[test]
    fn leaving_the_tail_freezes_once_and_end_resumes_live_following() {
        let mut live = lines();
        let mut selection = StreamingTextSelection::new();
        selection.sync_viewport(live.len(), 2, Rect::new(1, 2, 20, 2));
        assert_eq!(selection.viewport.pos(), 2);
        assert!(selection.is_following());
        selection.navigate(&live, Viewport::up);
        live[1] = "replacement".into();
        live.push("fourth".into());
        selection.navigate(&live, Viewport::up);
        assert_eq!(
            selection.selection().visual_selection_source().lines(&live),
            lines()
        );
        selection.navigate(&live, Viewport::end);
        selection.sync_viewport(live.len(), 2, Rect::new(1, 2, 20, 2));
        assert!(selection.is_following());
        assert_eq!(selection.viewport.pos(), 3);
        assert_eq!(selection.viewport.scroll_offset(), 2);
        assert_eq!(
            selection.selection().visual_selection_source().lines(&live),
            live
        );
    }

    #[test]
    fn drag_ranges_survive_stream_completion_and_clicks_collapse_them() {
        let live = lines();
        let mut selection = StreamingTextSelection::new();
        selection.sync_viewport(3, 2, Rect::new(0, 0, 20, 2));
        selection.click_select_row(&live, 0);
        selection.select_drag_to(&live, 2);
        selection.finish_stream();
        assert!(selection.selection().is_visual());
        assert_eq!(
            selection.selected_range(&live),
            OutputSelectionRange::Rows { first: 0, last: 2 }
        );
        selection.click_select_row(&live, 1);
        assert!(!selection.selection().is_visual());
        assert_eq!(
            selection.selected_range(&live),
            OutputSelectionRange::Rows { first: 1, last: 1 }
        );
        selection.finish_stream();
        assert!(selection.is_following());
        assert_eq!(selection.viewport.pos(), 2);
    }

    #[test]
    fn visual_exit_keeps_the_snapshot_and_reset_preserves_surface_geometry() {
        let mut live = lines();
        let mut selection = StreamingTextSelection::new();
        let area = Rect::new(3, 4, 30, 2);
        selection.sync_viewport(3, 2, area);
        selection.select_extend_up(&live);
        selection.select_extend_to_top(&live);
        selection.select_extend_down(&live);
        assert_eq!(selection.selection_line_count(&live), 2);
        selection.exit_visual();
        live[1] = "replacement".into();
        assert_eq!(
            selection.selection().visual_selection_source().lines(&live)[1],
            "second"
        );
        selection.reset_for_open();
        assert_eq!(selection.viewport.content_area(), area);
        assert!(selection.is_following());
        assert_eq!(
            selection.selection().visual_selection_source().lines(&live)[1],
            "replacement"
        );
        selection.select_all(&live);
        assert_eq!(selection.selection_line_count(&live), 3);
        selection.collapse_to_tail();
        assert!(!selection.selection().is_visual());
        assert!(selection.is_following());
    }

    #[test]
    fn empty_buffers_and_zero_height_surfaces_have_no_selected_rows_or_scroll() {
        let mut selection = StreamingTextSelection::new();
        selection.sync_viewport(0, 0, Rect::ZERO);
        selection.select_all(&[]);
        assert_eq!(selection.selected_range(&[]), OutputSelectionRange::Empty);
        assert_eq!(selection.selection_line_count(&[]), 0);
        assert_eq!(scroll_to_show_cursor(8, 5, 0, 10), 0);
        assert_eq!(scroll_to_show_cursor(2, 5, 3, 10), 2);
        assert_eq!(scroll_to_show_cursor(9, 0, 3, 10), 7);
        assert_eq!(scroll_to_show_cursor(0, 0, 3, 1), 0);
    }
}
