use std::rc::Rc;

use super::presentation::MonitorColumn;
use super::presentation::MonitorColumns;
use super::presentation::MonitorPresentation;
use super::presentation::MonitorVisibility;
use super::presentation::OutputPresentation;
use super::presentation::OwnedOutputPresentation;
use super::presentation::OwnedOutputVisibility;
use crate::build_monitor::BuildSessionId;
use crate::build_monitor::CompileActivityId;
use crate::build_monitor::MonitorSessionOwnership;
use crate::tui::OwnedRunId;

/// The output pane's selection sub-mode.
///
/// In `Normal` the selection is the single row under the cursor and plain
/// motions move it whole (the anchor follows the cursor). In `Visual` —
/// the vim visual-line sub-mode (`V`) — plain motions grow the range from
/// the fixed anchor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectionMode {
    Normal,
    Visual,
}

/// Which buffer the visual selection reads.
///
/// A selection that has stopped tracking the streaming tail reads the buffer it
/// was frozen against, so a child process still writing cannot move a range the
/// user already picked.
#[derive(Clone)]
pub(super) enum VisualSelectionSource {
    /// The selection follows the tail and reads whatever output is live.
    LiveOutput,
    /// The selection is pinned against this frozen buffer.
    Frozen(Rc<[String]>),
}

impl VisualSelectionSource {
    /// The lines the selection and the copy payload read.
    pub(super) fn lines<'a>(&'a self, live: &'a [String]) -> &'a [String] {
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
    Rows { first: usize, last: usize },
}

impl OutputSelectionRange {
    /// How many rows the range covers.
    pub const fn line_count(self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Rows { first, last } => last - first + 1,
        }
    }

    /// Whether `row` is inside the range.
    pub const fn contains(self, row: usize) -> bool {
        match self {
            Self::Empty => false,
            Self::Rows { first, last } => row >= first && row <= last,
        }
    }
}

/// The owned column a captured-output cursor was built for.
///
/// It carries the [`OwnedRunId`] that produced the retained output, and that id
/// comes only from [`OwnedOutputPresentation::producer`] or from a
/// [`super::OutputMonitorHit::CapturedOutput`] the renderer recorded over an owned
/// body. An external column has no producer to supply, so it cannot build one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OwnedColumnWitness(OwnedRunId);

impl OwnedColumnWitness {
    /// The run whose captured output the cursor is sitting in.
    pub(super) const fn producer(self) -> OwnedRunId { self.0 }
}

impl From<OwnedRunId> for OwnedColumnWitness {
    fn from(producer: OwnedRunId) -> Self { Self(producer) }
}

/// What the Output pane's cursor is on.
///
/// The identities are the exec-sensitive ones: a same-PID exec produces a new
/// [`BuildSessionId`] and [`CompileActivityId`], so a cursor retained across
/// that transition no longer matches anything and falls back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OutputCursorTarget {
    /// The pane has no row to sit on.
    Empty,
    /// A session's header row.
    Header(BuildSessionId),
    /// One activity row under a session.
    Activity(CompileActivityId),
    /// One row of the scope-level unattributed section.
    Unattributed(CompileActivityId),
    /// A row of Cargo Port's own captured output, under the run that produced
    /// it.
    CapturedOutput(OwnedColumnWitness),
}

impl OutputCursorTarget {
    /// Whether a visual selection, a drag-select, or Ctrl-A is allowed here.
    /// Only captured output is a transcript; every other row is a live sample.
    pub(super) const fn visual_selection_permission(&self) -> VisualSelectionPermission {
        match self {
            Self::CapturedOutput(_) => VisualSelectionPermission::CapturedOutput,
            Self::Empty | Self::Header(_) | Self::Activity(_) | Self::Unattributed(_) => {
                VisualSelectionPermission::Denied
            },
        }
    }
}

/// Whether the row under the cursor may be selected, dragged over, or copied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualSelectionPermission {
    /// The cursor is on a live sample of an observed process, which the user
    /// may look at but not select from.
    Denied,
    /// The cursor is in Cargo Port's own captured output.
    CapturedOutput,
}

/// Whether Cargo Port's own column is the one the Output cursor is in.
///
/// Esc reads this before stopping or clearing the owned run: with an external
/// column selected it must not reach past the selection to the run Cargo Port
/// launched. The whole column is the unit, not the row: a header or an activity row of
/// Cargo Port's own column is as much part of that column as its captured
/// output, so Esc stops the run from any of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnedColumnSelection {
    /// The cursor is in an external column, in the scope-level unattributed
    /// section, or on nothing at all.
    NotSelected,
    /// The cursor is somewhere in Cargo Port's own column.
    Selected,
}

/// Linewise selection state for the output pane.
///
/// There is always a selection — at minimum the single row under the
/// cursor — so the pane has no separate select/deselect mode. `anchor`
/// is the fixed end; the moving end is `OutputPane::viewport`'s `pos`,
/// and the selected range runs between them. `mode` is the
/// [`SelectionMode`] that decides how plain motions read.
///
/// `visual_selection_source` names which buffer the range reads, so a streaming
/// child process can't drift a pinned range.
pub struct OutputSelection {
    pub(super) anchor:                  usize,
    pub(super) selection_mode:          SelectionMode,
    pub(super) visual_selection_source: VisualSelectionSource,
}

impl OutputSelection {
    pub(super) const fn new() -> Self {
        Self {
            anchor:                  0,
            selection_mode:          SelectionMode::Normal,
            visual_selection_source: VisualSelectionSource::LiveOutput,
        }
    }

    /// Whether the vim visual-line sub-mode is active.
    pub const fn is_visual(&self) -> bool { matches!(self.selection_mode, SelectionMode::Visual) }

    /// Which buffer the selection reads.
    pub(super) const fn visual_selection_source(&self) -> &VisualSelectionSource {
        &self.visual_selection_source
    }
}

/// Whether Cargo Port's own output is on screen for the cursor to fall back to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OwnedPinPresence {
    /// No owned output is drawn this frame.
    Absent,
    /// The output of this run is drawn, pinned or in its own column.
    Pinned(OwnedRunId),
}

impl From<OwnedOutputVisibility<'_>> for OwnedPinPresence {
    fn from(owned_output_visibility: OwnedOutputVisibility<'_>) -> Self {
        match owned_output_visibility {
            OwnedOutputVisibility::Absent => Self::Absent,
            OwnedOutputVisibility::OnScreen(owned_output_presentation) => Self::Pinned(
                OwnedOutputPresentation::producer(&owned_output_presentation),
            ),
        }
    }
}

/// The column a cursor is retained against.
///
/// Held beside [`OutputCursorTarget::Activity`], which names the activity but
/// not the session it runs under: after that activity exits, the fallback still
/// has to find the same column rather than whatever now sits at its old index.
/// Each variant is a distinct place the cursor can sit, because the vertical
/// scroll offset is keyed on this value: collapsing the sections that sit
/// outside any column into one variant would let an offset measured against
/// the unattributed section survive a move into captured output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OutputCursorColumn {
    /// The cursor has no row under it.
    Absent,
    /// The cursor is on a row of the scope-level unattributed section, which
    /// sits under every column and belongs to none.
    UnattributedSection,
    /// The cursor is in Cargo Port's own captured output, drawn either under
    /// its session's column or pinned as its own.
    OwnedCapturedOutput,
    /// The cursor is on this session's column.
    Session(BuildSessionId),
}

/// Where the Output pane's cursor is, and what it falls back to when the thing
/// under it exits.
///
/// The identities decide what is retained; the indices decide where the cursor
/// lands when the identity is gone, so a finished unit leaves the cursor at the
/// same place on screen rather than jumping to the top.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OutputCursor {
    target:       OutputCursorTarget,
    column:       OutputCursorColumn,
    column_index: usize,
    row_index:    usize,
}

impl OutputCursor {
    /// A cursor with nothing under it.
    pub(super) const fn empty() -> Self {
        Self {
            target:       OutputCursorTarget::Empty,
            column:       OutputCursorColumn::Absent,
            column_index: 0,
            row_index:    0,
        }
    }

    /// What the cursor is on.
    pub(super) const fn target(&self) -> &OutputCursorTarget { &self.target }

    /// Which column the cursor sits in, for the horizontal window.
    pub(super) const fn column_index(&self) -> usize { self.column_index }

    /// Which column the cursor sits in by identity, not by position.
    ///
    /// The vertical scroll offset belongs to this value: an index would follow
    /// whatever column slid into that position, and a `BuildSessionId` alone
    /// cannot express the sections that sit outside any column.
    pub(super) const fn column(&self) -> &OutputCursorColumn { &self.column }

    /// Which row within the column's body the cursor is on.
    pub(super) const fn row_index(&self) -> usize { self.row_index }

    /// Aim the cursor at a session's header row.
    pub(super) fn focus_header(&mut self, build_session_id: BuildSessionId, column_index: usize) {
        self.target = OutputCursorTarget::Header(build_session_id.clone());
        self.column = OutputCursorColumn::Session(build_session_id);
        self.column_index = column_index;
        self.row_index = 0;
    }

    /// Aim the cursor at one activity row under a session's column.
    pub(super) const fn focus_activity(
        &mut self,
        compile_activity_id: CompileActivityId,
        build_session_id: BuildSessionId,
        column_index: usize,
        row_index: usize,
    ) {
        self.target = OutputCursorTarget::Activity(compile_activity_id);
        self.column = OutputCursorColumn::Session(build_session_id);
        self.column_index = column_index;
        self.row_index = row_index;
    }

    /// Aim the cursor at one row of the scope-level unattributed section.
    pub(super) const fn focus_unattributed(
        &mut self,
        compile_activity_id: CompileActivityId,
        row_index: usize,
    ) {
        self.target = OutputCursorTarget::Unattributed(compile_activity_id);
        self.column = OutputCursorColumn::UnattributedSection;
        self.row_index = row_index;
    }

    /// Aim the cursor at the captured output `producer` retained. Naming the
    /// producer is what keeps this target out of an external column.
    pub(super) fn focus_captured_output(&mut self, producer: OwnedRunId) {
        self.target = OutputCursorTarget::CapturedOutput(producer.into());
        self.column = OutputCursorColumn::OwnedCapturedOutput;
        self.row_index = 0;
    }
}

impl OutputCursor {
    /// Move the cursor to whatever survived this frame.
    ///
    /// A scope change that leaves no columns keeps only a still-present pinned
    /// owned selection: nothing else the cursor named is on screen any more.
    pub(super) fn reconcile(&mut self, output_presentation: &OutputPresentation<'_>) {
        let owned_pin_presence = OwnedPinPresence::from(output_presentation.owned_output());
        let MonitorVisibility::On(MonitorPresentation::Columns(monitor_columns)) =
            output_presentation.monitor()
        else {
            self.reduce_to_owned_or_empty(owned_pin_presence);
            return;
        };
        let columns: Vec<MonitorColumn<'_>> = monitor_columns.columns().collect();
        match self.target.clone() {
            OutputCursorTarget::CapturedOutput(owned_column_witness) => {
                self.retain_captured_output(owned_column_witness, owned_pin_presence);
            },
            OutputCursorTarget::Empty => {
                self.place_on_first(&columns, owned_pin_presence);
            },
            OutputCursorTarget::Header(build_session_id) => {
                match column_index_of(&columns, &build_session_id) {
                    Some(column_index) => self.column_index = column_index,
                    None => self.after_session_exit(&columns, owned_pin_presence),
                }
            },
            OutputCursorTarget::Activity(compile_activity_id) => {
                self.after_activity(&columns, &compile_activity_id, owned_pin_presence);
            },
            OutputCursorTarget::Unattributed(compile_activity_id) => {
                self.after_unattributed(
                    &columns,
                    monitor_columns,
                    &compile_activity_id,
                    owned_pin_presence,
                );
            },
        }
    }

    /// A captured-output cursor survives only while the run it was placed in is
    /// still the one on screen. Once a later run's output takes the column, the
    /// cursor moves to that run rather than holding a position in a buffer that
    /// no longer exists.
    fn retain_captured_output(
        &mut self,
        owned_column_witness: OwnedColumnWitness,
        owned_pin_presence: OwnedPinPresence,
    ) {
        match owned_pin_presence {
            OwnedPinPresence::Absent => *self = Self::empty(),
            OwnedPinPresence::Pinned(producer) => {
                if producer != owned_column_witness.producer() {
                    self.focus_captured_output(producer);
                }
            },
        }
    }

    /// Keep a pinned owned selection, and otherwise leave the cursor with
    /// nothing under it.
    fn reduce_to_owned_or_empty(&mut self, owned_pin_presence: OwnedPinPresence) {
        match owned_pin_presence {
            OwnedPinPresence::Pinned(producer) => self.focus_captured_output(producer),
            OwnedPinPresence::Absent => *self = Self::empty(),
        }
    }

    /// Put a cursor with nothing under it on the first column's header.
    fn place_on_first(
        &mut self,
        columns: &[MonitorColumn<'_>],
        owned_pin_presence: OwnedPinPresence,
    ) {
        match columns.first() {
            Some(column) => self.focus_header(column.build_session_id().clone(), 0),
            None => self.reduce_to_owned_or_empty(owned_pin_presence),
        }
    }

    /// The session under the cursor exited: take the session now at its ordered
    /// index, then the one preceding it, then the pinned owned output.
    fn after_session_exit(
        &mut self,
        columns: &[MonitorColumn<'_>],
        owned_pin_presence: OwnedPinPresence,
    ) {
        let fallback_index = if columns.get(self.column_index).is_some() {
            self.column_index
        } else {
            self.column_index.saturating_sub(1)
        };
        match columns.get(fallback_index) {
            Some(column) => self.focus_header(column.build_session_id().clone(), fallback_index),
            None => self.reduce_to_owned_or_empty(owned_pin_presence),
        }
    }

    /// Reconcile an activity cursor: keep it while the unit is running, and
    /// otherwise take the row now at its index, then the previous row, then its
    /// column's header.
    fn after_activity(
        &mut self,
        columns: &[MonitorColumn<'_>],
        compile_activity_id: &CompileActivityId,
        owned_pin_presence: OwnedPinPresence,
    ) {
        if let Some((column_index, row_index)) = find_activity(columns, compile_activity_id) {
            self.column_index = column_index;
            self.row_index = row_index;
            return;
        }
        let OutputCursorColumn::Session(build_session_id) = self.column.clone() else {
            self.after_session_exit(columns, owned_pin_presence);
            return;
        };
        let Some(column_index) = column_index_of(columns, &build_session_id) else {
            self.after_session_exit(columns, owned_pin_presence);
            return;
        };
        let compile_activities = columns[column_index].session_row().compile_activities();
        let row_index = if compile_activities.get(self.row_index).is_some() {
            self.row_index
        } else {
            self.row_index.saturating_sub(1)
        };
        match compile_activities.get(row_index) {
            Some(compile_activity) => self.focus_activity(
                compile_activity.compile_activity_id().clone(),
                build_session_id,
                column_index,
                row_index,
            ),
            None => self.focus_header(build_session_id, column_index),
        }
    }

    /// Reconcile an unattributed-section cursor by the same row rules, falling
    /// back to the first column's header once the section empties.
    fn after_unattributed(
        &mut self,
        columns: &[MonitorColumn<'_>],
        monitor_columns: MonitorColumns<'_>,
        compile_activity_id: &CompileActivityId,
        owned_pin_presence: OwnedPinPresence,
    ) {
        let unattributed_activities = monitor_columns.unattributed_activities();
        if let Some(row_index) = unattributed_activities
            .iter()
            .position(|activity| activity.compile_activity_id() == compile_activity_id)
        {
            self.row_index = row_index;
            return;
        }
        let row_index = if unattributed_activities.get(self.row_index).is_some() {
            self.row_index
        } else {
            self.row_index.saturating_sub(1)
        };
        match unattributed_activities.get(row_index) {
            Some(unattributed_activity) => self.focus_unattributed(
                unattributed_activity.compile_activity_id().clone(),
                row_index,
            ),
            None => self.place_on_first(columns, owned_pin_presence),
        }
    }
}

/// One position in the selected column's body, as a keyboard motion names it.
///
/// The `-- Output --` separator is not a position, which is what makes a single
/// Down cross it from the last activity row into the captured output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ColumnBodyRow {
    /// The column's header block, which the cursor treats as one row.
    Header,
    /// The activity row at this index among the column's compile activities.
    Activity(usize),
    /// The captured-output line at this buffer index.
    CapturedOutput(usize),
}

/// The column a keyboard motion walks, resolved from where the cursor is.
///
/// The body reads top to bottom as one list, so every vertical motion is
/// arithmetic on a position in it rather than a special case per row kind.
#[derive(Clone, Copy)]
pub(super) enum SelectedColumn<'a> {
    /// The cursor has no column to walk: the monitor drew no columns and no
    /// owned output is on screen.
    Nothing,
    /// Cargo Port's own output pinned beside the scope's columns, with no
    /// session of its own. Its captured lines are the whole body.
    PinnedOwnedOutput { captured: usize },
    /// One of the scope's columns: its header, its activity rows, and — when
    /// this session is the run Cargo Port launched — its captured output.
    Session {
        column:   MonitorColumn<'a>,
        index:    usize,
        captured: usize,
    },
}

impl SelectedColumn<'_> {
    /// How many positions the body has.
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Nothing => 0,
            Self::PinnedOwnedOutput { captured } => *captured,
            Self::Session {
                column, captured, ..
            } => {
                COLUMN_HEADER_POSITIONS + column.session_row().compile_activities().len() + captured
            },
        }
    }

    /// What sits at `position`, which callers clamp to `len() - 1` first.
    fn row_at(&self, position: usize) -> ColumnBodyRow {
        match self {
            Self::Nothing | Self::PinnedOwnedOutput { .. } => {
                ColumnBodyRow::CapturedOutput(position)
            },
            Self::Session { column, .. } => {
                let activities = column.session_row().compile_activities().len();
                match position.checked_sub(COLUMN_HEADER_POSITIONS) {
                    None => ColumnBodyRow::Header,
                    Some(below_header) if below_header < activities => {
                        ColumnBodyRow::Activity(below_header)
                    },
                    Some(below_header) => ColumnBodyRow::CapturedOutput(below_header - activities),
                }
            },
        }
    }

    /// Where `column_body_row` sits in the body.
    fn position_of(&self, column_body_row: ColumnBodyRow) -> usize {
        match self {
            Self::Nothing | Self::PinnedOwnedOutput { .. } => match column_body_row {
                ColumnBodyRow::Header | ColumnBodyRow::Activity(_) => 0,
                ColumnBodyRow::CapturedOutput(row) => row,
            },
            Self::Session { column, .. } => {
                let activities = column.session_row().compile_activities().len();
                match column_body_row {
                    ColumnBodyRow::Header => 0,
                    ColumnBodyRow::Activity(row) => COLUMN_HEADER_POSITIONS + row,
                    ColumnBodyRow::CapturedOutput(row) => {
                        COLUMN_HEADER_POSITIONS + activities + row
                    },
                }
            },
        }
    }
}

/// The header block is one position in the body however many lines it draws.
const COLUMN_HEADER_POSITIONS: usize = 1;

/// Whether an adjacent-column motion found a column to move to.
///
/// Tab traversal reads this to decide whether the key was consumed or falls
/// through to pane snaking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnSelection {
    /// There is no column at that index; the caller keeps the key.
    NoSuchColumn,
    /// The cursor moved to the column's header.
    Moved,
}

impl OutputCursor {
    /// The column a keyboard motion walks from here.
    ///
    /// A captured-output cursor resolves to its producer's own column when the
    /// scope still covers that run, so Up out of the captured output lands on
    /// the activity rows above it rather than on a separate body.
    pub(super) fn selected_column<'a>(
        &self,
        columns: &'a [MonitorColumn<'a>],
        owned_pin_presence: OwnedPinPresence,
        captured_line_count: usize,
    ) -> SelectedColumn<'a> {
        match self.column.clone() {
            OutputCursorColumn::Absent | OutputCursorColumn::UnattributedSection => columns
                .first()
                .map_or(SelectedColumn::Nothing, |column| SelectedColumn::Session {
                    column:   *column,
                    index:    0,
                    captured: captured_for(*column, owned_pin_presence, captured_line_count),
                }),
            OutputCursorColumn::OwnedCapturedOutput => {
                let OwnedPinPresence::Pinned(producer) = owned_pin_presence else {
                    return SelectedColumn::Nothing;
                };
                columns
                    .iter()
                    .position(|column| column_produced_by(*column, producer))
                    .map_or(
                        SelectedColumn::PinnedOwnedOutput {
                            captured: captured_line_count,
                        },
                        |index| SelectedColumn::Session {
                            column: columns[index],
                            index,
                            captured: captured_line_count,
                        },
                    )
            },
            OutputCursorColumn::Session(build_session_id) => column_index_of(
                columns,
                &build_session_id,
            )
            .map_or(SelectedColumn::Nothing, |index| SelectedColumn::Session {
                column: columns[index],
                index,
                captured: captured_for(columns[index], owned_pin_presence, captured_line_count),
            }),
        }
    }

    /// Where the cursor sits in `selected_column`'s body. `captured_row` is the
    /// viewport's row, which the pane owns rather than the cursor.
    pub(super) fn body_position(
        &self,
        selected_column: &SelectedColumn<'_>,
        captured_row: usize,
    ) -> usize {
        let column_body_row = match self.target {
            OutputCursorTarget::Activity(_) => ColumnBodyRow::Activity(self.row_index),
            OutputCursorTarget::CapturedOutput(_) => ColumnBodyRow::CapturedOutput(captured_row),
            OutputCursorTarget::Empty
            | OutputCursorTarget::Header(_)
            | OutputCursorTarget::Unattributed(_) => ColumnBodyRow::Header,
        };
        selected_column
            .position_of(column_body_row)
            .min(selected_column.len().saturating_sub(1))
    }

    /// What `position` names in `selected_column`'s body.
    pub(super) fn body_row_at(
        selected_column: &SelectedColumn<'_>,
        position: usize,
    ) -> ColumnBodyRow {
        selected_column.row_at(position)
    }

    /// Put the cursor on `column_body_row` of `selected_column`.
    ///
    /// A captured-output row moves only the cursor's target; the pane positions
    /// its viewport on the same row, because the viewport is what scrolls the
    /// captured buffer.
    pub(super) fn place_body_row(
        &mut self,
        selected_column: &SelectedColumn<'_>,
        column_body_row: ColumnBodyRow,
        owned_pin_presence: OwnedPinPresence,
    ) {
        match (selected_column, column_body_row) {
            (SelectedColumn::Nothing, _) => {},
            (_, ColumnBodyRow::CapturedOutput(_)) => {
                if let OwnedPinPresence::Pinned(producer) = owned_pin_presence {
                    self.focus_captured_output(producer);
                }
            },
            (SelectedColumn::PinnedOwnedOutput { .. }, _) => {},
            (SelectedColumn::Session { column, index, .. }, ColumnBodyRow::Header) => {
                self.focus_header(column.build_session_id().clone(), *index);
            },
            (SelectedColumn::Session { column, index, .. }, ColumnBodyRow::Activity(row)) => {
                let compile_activities = column.session_row().compile_activities();
                if let Some(compile_activity) = compile_activities.get(row) {
                    self.focus_activity(
                        compile_activity.compile_activity_id().clone(),
                        column.build_session_id().clone(),
                        *index,
                        row,
                    );
                }
            },
        }
    }

    /// Whether the cursor's column is the one Cargo Port launched.
    ///
    /// Column identity, not row kind: a header or an activity row of the owned
    /// column is as much part of that column as its captured output, so Esc
    /// stops the run from any of them. An external column answers
    /// [`OwnedColumnSelection::NotSelected`] however far down it the cursor is.
    pub(super) fn owned_column_selection(
        &self,
        columns: &[MonitorColumn<'_>],
        owned_pin_presence: OwnedPinPresence,
    ) -> OwnedColumnSelection {
        let OwnedPinPresence::Pinned(producer) = owned_pin_presence else {
            return OwnedColumnSelection::NotSelected;
        };
        match self.column.clone() {
            OutputCursorColumn::OwnedCapturedOutput => OwnedColumnSelection::Selected,
            OutputCursorColumn::Absent | OutputCursorColumn::UnattributedSection => {
                OwnedColumnSelection::NotSelected
            },
            OutputCursorColumn::Session(build_session_id) => {
                match column_index_of(columns, &build_session_id) {
                    Some(index) if column_produced_by(columns[index], producer) => {
                        OwnedColumnSelection::Selected
                    },
                    Some(_) | None => OwnedColumnSelection::NotSelected,
                }
            },
        }
    }

    /// Move to the column at `column_index`, landing on its header.
    pub(super) fn select_column(
        &mut self,
        columns: &[MonitorColumn<'_>],
        column_index: usize,
    ) -> ColumnSelection {
        let Some(column) = columns.get(column_index) else {
            return ColumnSelection::NoSuchColumn;
        };
        self.focus_header(column.build_session_id().clone(), column_index);
        ColumnSelection::Moved
    }
}

/// How many captured lines belong under `column`: its own when it is the run
/// Cargo Port launched, and none otherwise.
fn captured_for(
    column: MonitorColumn<'_>,
    owned_pin_presence: OwnedPinPresence,
    captured_line_count: usize,
) -> usize {
    match owned_pin_presence {
        OwnedPinPresence::Pinned(producer) if column_produced_by(column, producer) => {
            captured_line_count
        },
        OwnedPinPresence::Absent | OwnedPinPresence::Pinned(_) => 0,
    }
}

/// Whether `column`'s session is the run that produced the retained output.
fn column_produced_by(column: MonitorColumn<'_>, producer: OwnedRunId) -> bool {
    match column.session_ownership() {
        MonitorSessionOwnership::Owned(owned_run_id) => owned_run_id == producer,
        MonitorSessionOwnership::External => false,
    }
}

/// Where `build_session_id` sits among the columns.
fn column_index_of(
    columns: &[MonitorColumn<'_>],
    build_session_id: &BuildSessionId,
) -> Option<usize> {
    columns
        .iter()
        .position(|column| column.build_session_id() == build_session_id)
}

/// The column and row holding `compile_activity_id`, if it is still running.
fn find_activity(
    columns: &[MonitorColumn<'_>],
    compile_activity_id: &CompileActivityId,
) -> Option<(usize, usize)> {
    columns
        .iter()
        .enumerate()
        .find_map(|(column_index, column)| {
            column
                .session_row()
                .compile_activities()
                .iter()
                .position(|activity| activity.compile_activity_id() == compile_activity_id)
                .map(|row_index| (column_index, row_index))
        })
}
