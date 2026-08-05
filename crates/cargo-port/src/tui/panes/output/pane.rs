use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::CopySelectionResult;
use tui_pane::Hittable;
use tui_pane::NavAction;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use super::hit_map::MonitorHitMap;
use super::hit_map::OutputMonitorHit;
use super::presentation::MonitorColumn;
use super::presentation::MonitorPresentation;
use super::presentation::MonitorVisibility;
use super::presentation::OutputPresentation;
use super::selection::ColumnBodyRow;
use super::selection::ColumnSelection;
use super::selection::OutputCursor;
use super::selection::OutputCursorColumn;
use super::selection::OutputCursorTarget;
use super::selection::OutputSelection;
use super::selection::OutputSelectionRange;
use super::selection::OwnedColumnSelection;
use super::selection::OwnedPinPresence;
use super::selection::SelectionMode;
use super::selection::VisualSelectionPermission;
use super::selection::VisualSelectionSource;
use crate::tui::hit_test::HoverTarget;
use crate::tui::panes;
use crate::tui::panes::PaneId;
use crate::tui::render_context::PaneRenderCtx;

/// The vertical scroll of the column the Output cursor is in.
///
/// One offset, not a map keyed by session: the monitor snapshot is replaced
/// wholesale on every poll, so a per-session offset would outlive the column it
/// describes and be restored onto a different run. Pairing the offset with the
/// column identity it was measured against makes the reset unconditional —
/// moving the cursor to another column cannot leave a stale offset behind,
/// because the offset is only ever read after
/// [`OutputPane::sync_column_scroll`] has confirmed the identity still matches.
struct ColumnScroll {
    column:       OutputCursorColumn,
    offset:       usize,
    /// How many of the column's rows the last frame could draw. The page and
    /// half-page motions step by this, so paging moves by what the user can
    /// see rather than by a fixed guess.
    visible_rows: usize,
}

impl ColumnScroll {
    const fn new() -> Self {
        Self {
            column:       OutputCursorColumn::Absent,
            offset:       0,
            visible_rows: 0,
        }
    }
}

/// Which way Tab walks the monitor's ordered session list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputTabStep {
    /// Tab: the next column in first-seen order.
    NextColumn,
    /// Shift-Tab: the previous column.
    PreviousColumn,
}

pub struct OutputPane {
    pub viewport:    Viewport,
    pub focus:       RenderFocus,
    selection:       OutputSelection,
    cursor:          OutputCursor,
    column_scroll:   ColumnScroll,
    monitor_hit_map: MonitorHitMap,
}

impl OutputPane {
    pub const fn new() -> Self {
        Self {
            viewport:        Viewport::new(),
            focus:           RenderFocus::inactive(),
            selection:       OutputSelection::new(),
            cursor:          OutputCursor::empty(),
            column_scroll:   ColumnScroll::new(),
            monitor_hit_map: MonitorHitMap::new(),
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

    /// Where the cursor is and what it falls back to.
    pub(super) const fn cursor(&self) -> &OutputCursor { &self.cursor }

    /// Whether the row the cursor names can take part in a visual selection.
    /// Only Cargo Port's own captured output can: the monitor's rows describe
    /// host processes and hold no text the pane owns.
    pub const fn visual_selection_permission(&self) -> VisualSelectionPermission {
        self.cursor.target().visual_selection_permission()
    }

    /// Take the rows this frame drew, so a click can only ever name something
    /// that was on screen.
    pub(super) fn set_monitor_hit_map(&mut self, monitor_hit_map: MonitorHitMap) {
        self.monitor_hit_map = monitor_hit_map;
    }

    /// Forget the recorded rows, for a frame that did not draw the pane at all.
    /// Without this the last drawn frame's rows keep answering clicks while the
    /// pane is hidden.
    pub fn clear_monitor_hit_map(&mut self) { self.monitor_hit_map.clear(); }

    /// The captured-output row under `pos`, or [`CapturedOutputRow::Outside`]
    /// when the position is over monitor chrome rather than Cargo Port's own
    /// output. With the monitor on only the owned column's recorded rows
    /// answer; with it off there are no recorded rows and the viewport
    /// resolves the row across the whole pane.
    pub fn captured_output_row_at(&self, pos: Position) -> CapturedOutputRow {
        if !self.monitor_hit_map.is_empty() {
            return match self.monitor_hit_map.hit_at(pos) {
                Some(&OutputMonitorHit::CapturedOutput { row, .. }) => CapturedOutputRow::Row(row),
                Some(_) | None => CapturedOutputRow::Outside,
            };
        }
        self.viewport
            .pos_to_local_row(pos)
            .map_or(CapturedOutputRow::Outside, CapturedOutputRow::Row)
    }

    /// Move the cursor onto the row a click landed on.
    pub fn focus_hit(&mut self, output_monitor_hit: &OutputMonitorHit) {
        match output_monitor_hit {
            OutputMonitorHit::Header {
                build_session_id,
                column_index,
            } => self
                .cursor
                .focus_header(build_session_id.clone(), *column_index),
            OutputMonitorHit::Activity {
                compile_activity_id,
                build_session_id,
                column_index,
                row_index,
            } => self.cursor.focus_activity(
                compile_activity_id.clone(),
                build_session_id.clone(),
                *column_index,
                *row_index,
            ),
            OutputMonitorHit::Unattributed {
                compile_activity_id,
                row_index,
            } => self
                .cursor
                .focus_unattributed(compile_activity_id.clone(), *row_index),
            OutputMonitorHit::CapturedOutput { producer, row } => {
                self.cursor.focus_captured_output(*producer);
                self.viewport.set_pos(*row);
            },
            OutputMonitorHit::EmptyMonitor => {},
        }
    }

    /// How far the selected column's rows are scrolled.
    ///
    /// Only meaningful for the column the cursor is in; every other column
    /// draws from its first row.
    pub(super) const fn column_scroll_offset(&self) -> usize { self.column_scroll.offset }

    /// Reconcile the selected column's scroll offset against the cursor, the
    /// way [`Self::sync_viewport`] does for the owned captured output.
    ///
    /// `visible_rows` is how many of the column's rows the pane can draw and
    /// `row_count` how many it has. Called once per frame, after the cursor has
    /// been reconciled, so the offset is always measured against the column the
    /// cursor actually ended up in.
    pub(super) fn sync_column_scroll(&mut self, visible_rows: usize, row_count: usize) {
        if self.column_scroll.column != *self.cursor.column() {
            self.column_scroll = ColumnScroll {
                column: self.cursor.column().clone(),
                offset: 0,
                visible_rows,
            };
        }
        self.column_scroll.visible_rows = visible_rows;
        self.column_scroll.offset = scroll_to_show_cursor(
            self.cursor.row_index(),
            self.column_scroll.offset,
            visible_rows,
            row_count,
        );
    }

    /// Move the cursor to whatever survived this frame's model.
    pub fn reconcile_cursor(&mut self, output_presentation: &OutputPresentation<'_>) {
        self.cursor.reconcile(output_presentation);
    }

    /// Whether Cargo Port's own column is the one the cursor is in.
    pub fn owned_column_selection(
        &self,
        output_presentation: &OutputPresentation<'_>,
    ) -> OwnedColumnSelection {
        let owned_pin_presence = OwnedPinPresence::from(output_presentation.owned_output());
        let MonitorVisibility::On(MonitorPresentation::Columns(monitor_columns)) =
            output_presentation.monitor()
        else {
            // No columns are drawn, so there is no external column for the
            // cursor to be in: the owned body is the whole pane.
            return match owned_pin_presence {
                OwnedPinPresence::Absent => OwnedColumnSelection::NotSelected,
                OwnedPinPresence::Pinned(_) => OwnedColumnSelection::Selected,
            };
        };
        let columns: Vec<MonitorColumn<'_>> = monitor_columns.columns().collect();
        self.cursor
            .owned_column_selection(&columns, owned_pin_presence)
    }

    /// Apply a keyboard motion.
    ///
    /// With the monitor off the pane is Cargo Port's own captured output and
    /// nothing else, so `viewport_motion` drives it exactly as it did before
    /// the monitor existed. With the monitor on the motion walks the selected
    /// column's body instead, and Left/Right select adjacent columns.
    pub fn navigate_action(
        &mut self,
        output_presentation: &OutputPresentation<'_>,
        nav_action: NavAction,
        viewport_motion: impl FnOnce(&mut Viewport),
    ) {
        let live = output_presentation.captured_lines();
        let MonitorVisibility::On(MonitorPresentation::Columns(monitor_columns)) =
            output_presentation.monitor()
        else {
            self.navigate(live, viewport_motion);
            return;
        };
        let columns: Vec<MonitorColumn<'_>> = monitor_columns.columns().collect();
        let owned_pin_presence = OwnedPinPresence::from(output_presentation.owned_output());
        match nav_action {
            NavAction::Left => {
                let _ = self
                    .cursor
                    .select_column(&columns, self.cursor.column_index().saturating_sub(1));
            },
            NavAction::Right => {
                let _ = self
                    .cursor
                    .select_column(&columns, self.cursor.column_index().saturating_add(1));
            },
            vertical => self.move_within_column(live, &columns, owned_pin_presence, vertical),
        }
    }

    /// Walk the selected column's body by one vertical motion.
    fn move_within_column(
        &mut self,
        live: &[String],
        columns: &[MonitorColumn<'_>],
        owned_pin_presence: OwnedPinPresence,
        nav_action: NavAction,
    ) {
        let selected_column = self
            .cursor
            .selected_column(columns, owned_pin_presence, live.len());
        let Some(last) = selected_column.len().checked_sub(1) else {
            return;
        };
        let position = self
            .cursor
            .body_position(&selected_column, self.viewport.pos());
        let page = self.column_scroll.visible_rows.max(1);
        let target = match nav_action {
            NavAction::Up => position.saturating_sub(1),
            NavAction::Down => position.saturating_add(1).min(last),
            NavAction::Home => 0,
            NavAction::End => last,
            NavAction::PageUp => position.saturating_sub(page),
            NavAction::PageDown => position.saturating_add(page).min(last),
            NavAction::HalfPageUp => position.saturating_sub((page / 2).max(1)),
            NavAction::HalfPageDown => position.saturating_add((page / 2).max(1)).min(last),
            // Horizontal motions select a column; they never move the cursor
            // down the body.
            NavAction::Left | NavAction::Right => position,
        };
        let column_body_row = OutputCursor::body_row_at(&selected_column, target);
        self.cursor
            .place_body_row(&selected_column, column_body_row, owned_pin_presence);
        if let ColumnBodyRow::CapturedOutput(row) = column_body_row {
            self.navigate(live, |viewport| viewport.set_pos(row));
        }
    }

    /// Step to the adjacent session column for Tab / Shift-Tab.
    ///
    /// Reports [`ColumnSelection::NoSuchColumn`] at either end of the ordered
    /// session list, and with fewer than two sessions, so the key falls through
    /// to the framework's pane snaking.
    pub fn tab_to_adjacent_column(
        &mut self,
        output_presentation: &OutputPresentation<'_>,
        output_tab_step: OutputTabStep,
    ) -> ColumnSelection {
        let MonitorVisibility::On(MonitorPresentation::Columns(monitor_columns)) =
            output_presentation.monitor()
        else {
            return ColumnSelection::NoSuchColumn;
        };
        let columns: Vec<MonitorColumn<'_>> = monitor_columns.columns().collect();
        if columns.len() < 2 {
            return ColumnSelection::NoSuchColumn;
        }
        let column_index = self.cursor.column_index();
        let target = match output_tab_step {
            OutputTabStep::NextColumn => column_index.saturating_add(1),
            OutputTabStep::PreviousColumn => match column_index.checked_sub(1) {
                Some(previous) => previous,
                None => return ColumnSelection::NoSuchColumn,
            },
        };
        self.cursor.select_column(&columns, target)
    }

    /// The clipboard payload for whatever the cursor is on.
    ///
    /// An activity row copies that row's text. Captured output copies the
    /// selected range. A column header and the scope-level unattributed
    /// caption have no transcript behind them, so they copy nothing rather
    /// than reaching down to captured output the cursor is not in.
    pub fn copy_payload_for_cursor(
        &self,
        output_presentation: &OutputPresentation<'_>,
    ) -> CopySelectionResult {
        let live = output_presentation.captured_lines();
        let MonitorVisibility::On(monitor_presentation) = output_presentation.monitor() else {
            return self.copy_payload(live);
        };
        match self.cursor.target() {
            OutputCursorTarget::CapturedOutput(_) => self.copy_payload(live),
            OutputCursorTarget::Empty | OutputCursorTarget::Header(_) => {
                CopySelectionResult::Nothing
            },
            OutputCursorTarget::Activity(compile_activity_id) => {
                super::monitor_render::activity_row_text(monitor_presentation, compile_activity_id)
            },
            OutputCursorTarget::Unattributed(compile_activity_id) => {
                super::monitor_render::unattributed_row_text(
                    monitor_presentation,
                    compile_activity_id,
                )
            },
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
    pub fn selection_line_count(&self, live: &[String]) -> usize {
        self.selected_range(live).line_count()
    }

    /// The rows the selection covers, clamped to the source bounds (the frozen
    /// buffer when pinned, else `live`). Outside visual mode the range is the
    /// single cursor row; the `anchor` is read only in visual mode.
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

    /// Build the clipboard payload for the current selection, reading the
    /// frozen snapshot when pinned or `live` while following the tail.
    pub fn copy_payload(&self, live: &[String]) -> CopySelectionResult {
        let OutputSelectionRange::Rows { first, last } = self.selected_range(live) else {
            return CopySelectionResult::Nothing;
        };
        panes::copy_payload_for_output(self.source(live), first, last)
    }

    /// Resume following the tail when a process exits, unless the user is
    /// in visual mode selecting to copy. A collapsed single-row selection —
    /// the at-rest state, whether following or just scrolled — snaps to the
    /// new tail so the final output shows.
    pub fn on_process_exit(&mut self) {
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

/// What a screen position names in Cargo Port's own captured output. Anything
/// the monitor drew — headers, activity rows, the unattributed section — is
/// [`Outside`](Self::Outside), so a drag over monitor chrome cannot select
/// captured-output rows it never covered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapturedOutputRow {
    /// The position is not over a captured-output row.
    Outside,
    /// The buffer index of the captured-output row under the position.
    Row(usize),
}

impl Renderable<PaneRenderCtx<'_>> for OutputPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        super::render_output_pane_body(frame, area, self, ctx);
    }
}

impl Hittable<HoverTarget> for OutputPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        // With the monitor on, every drawn row names the model row behind it;
        // with it off there are no recorded rows and the pane falls back to the
        // captured-output row the viewport resolves.
        if let Some(output_monitor_hit) = self.monitor_hit_map.hit_at(pos) {
            return Some(HoverTarget::OutputMonitorRow(output_monitor_hit.clone()));
        }
        if !self.monitor_hit_map.is_empty() {
            return None;
        }
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Output,
            row,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::OutputPane;
    use crate::build_monitor::BuildSessionId;
    use crate::build_monitor::CompileActivityId;
    use crate::process_observation::identity::ProcessIdentity;
    use crate::process_observation::identity::ProcessIncarnation;

    const COLUMN_CAPACITY: usize = 4;
    const COLUMN_ROWS: usize = 12;

    fn build_session_id(pid: u32) -> BuildSessionId {
        BuildSessionId::for_test(ProcessIncarnation::for_test(
            ProcessIdentity::for_test(pid, 1),
            "cargo build",
        ))
    }

    fn compile_activity_id(pid: u32) -> CompileActivityId {
        CompileActivityId::for_test(ProcessIncarnation::for_test(
            ProcessIdentity::for_test(pid, 1),
            "rustc",
        ))
    }

    /// Place the cursor on `row_index` of the column belonging to `pid`.
    fn focus_row(pane: &mut OutputPane, pid: u32, column_index: usize, row_index: usize) {
        pane.cursor.focus_activity(
            compile_activity_id(pid.wrapping_add(u32::try_from(row_index).unwrap_or(0))),
            build_session_id(pid),
            column_index,
            row_index,
        );
    }

    /// A column with more activity rows than the pane can draw stays fully
    /// reachable: the offset follows the cursor down and back up.
    #[test]
    fn column_scroll_offset_follows_the_cursor_through_a_long_column() {
        let mut pane = OutputPane::new();

        for row_index in 0..COLUMN_ROWS {
            focus_row(&mut pane, 4000, 0, row_index);
            pane.sync_column_scroll(COLUMN_CAPACITY, COLUMN_ROWS);
            let offset = pane.column_scroll_offset();
            assert!(
                offset <= row_index && row_index < offset + COLUMN_CAPACITY,
                "row {row_index} should be drawn at offset {offset}"
            );
        }
        assert_eq!(
            pane.column_scroll_offset(),
            COLUMN_ROWS - COLUMN_CAPACITY,
            "the last row scrolls the column to its end, not past it"
        );

        focus_row(&mut pane, 4000, 0, 0);
        pane.sync_column_scroll(COLUMN_CAPACITY, COLUMN_ROWS);
        assert_eq!(
            pane.column_scroll_offset(),
            0,
            "returning to the first row scrolls the column back to its top"
        );
    }

    /// The offset belongs to one column. Selecting a different one starts that
    /// column at its first row rather than inheriting the previous scroll.
    #[test]
    fn column_scroll_offset_resets_when_the_selected_column_changes() {
        let mut pane = OutputPane::new();

        focus_row(&mut pane, 4000, 0, COLUMN_ROWS - 1);
        pane.sync_column_scroll(COLUMN_CAPACITY, COLUMN_ROWS);
        assert!(pane.column_scroll_offset() > 0);

        focus_row(&mut pane, 5000, 1, 0);
        pane.sync_column_scroll(COLUMN_CAPACITY, COLUMN_ROWS);
        assert_eq!(pane.column_scroll_offset(), 0);
    }

    /// A column whose rows all fit never scrolls.
    #[test]
    fn column_scroll_offset_stays_zero_when_every_row_fits() {
        let mut pane = OutputPane::new();

        for row_index in 0..COLUMN_CAPACITY {
            focus_row(&mut pane, 4000, 0, row_index);
            pane.sync_column_scroll(COLUMN_CAPACITY, COLUMN_CAPACITY);
            assert_eq!(pane.column_scroll_offset(), 0);
        }
    }
}
