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
/// [`OutputMonitorHit::CapturedOutput`] the renderer recorded over an owned
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
#[derive(Clone, Debug, Eq, PartialEq)]
enum OutputCursorColumn {
    /// The cursor is not on a session column.
    Detached,
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
            column:       OutputCursorColumn::Detached,
            column_index: 0,
            row_index:    0,
        }
    }

    /// What the cursor is on.
    pub(super) const fn target(&self) -> &OutputCursorTarget { &self.target }

    /// Which column the cursor sits in, for the horizontal window.
    pub(super) const fn column_index(&self) -> usize { self.column_index }

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
        self.column = OutputCursorColumn::Detached;
        self.row_index = row_index;
    }

    /// Aim the cursor at the captured output `producer` retained. Naming the
    /// producer is what keeps this target out of an external column.
    pub(super) fn focus_captured_output(&mut self, producer: OwnedRunId) {
        self.target = OutputCursorTarget::CapturedOutput(producer.into());
        self.column = OutputCursorColumn::Detached;
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
