//! Replay a cell painting with its command visibility proof.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use ratatui::buffer::Cell;

use super::Buffer;
use super::CellMeasurementKey;
use super::Color;
use super::InvocationId;
use super::PinnedGroup;
use super::Rc;
use super::Rect;
use super::RowInk;
use super::Text;
use super::TrackedGroup;
use super::TrackedRow;
use super::VisibleCommandText;
use super::VisibleRoster;
use super::placed_values::PlacedValueText;
use crate::app::GridView;
use crate::probe;
use crate::probe::PaintStage;

/// Rows read or written by the painter, including continuation-marker searches.
#[derive(Default)]
pub(super) struct CellPaintReach {
    pub(super) rows:      u16,
    pub(super) commands:  Vec<PlacedCommandText>,
    pub(super) header:    CellColumnHeader,
    pub(super) resources: Vec<super::RowResourceReading>,
    pub(super) values:    Vec<PlacedValueText>,
    pub(super) table:     PaintedTableColumns,
}

/// Solved columns prove whether a frame policy changes this table's drawing.
#[derive(Clone, Default)]
pub(super) enum PaintedTableColumns {
    #[default]
    Absent,
    Drawn {
        area:    Rect,
        columns: Vec<usize>,
        widths:  Vec<u16>,
        spacing: u16,
    },
}

impl PaintedTableColumns {
    pub(super) fn of(area: Rect, layout: &super::CellTableLayout) -> Self {
        Self::Drawn {
            area,
            columns: layout.columns.clone(),
            widths: layout.column_widths.clone(),
            spacing: layout.column_spacing,
        }
    }

    fn matches(
        &self,
        grid: &super::FrameTableColumnGrid,
        rows: &[&TrackedRow],
        pinned: super::PinnedGroup<'_>,
    ) -> bool {
        match self {
            Self::Absent => rows.is_empty(),
            Self::Drawn {
                area,
                columns,
                widths,
                spacing,
            } => {
                let layout =
                    super::CellTableLayout::for_table(grid, *area, Color::Reset, rows, pinned);
                *columns == layout.columns
                    && *widths == layout.column_widths
                    && *spacing == layout.column_spacing
            },
        }
    }
}

/// A row fragment belongs to one invocation and process.
#[derive(Clone, Eq, Hash, PartialEq)]
pub(super) struct RowFragmentIdentity {
    pub(super) invocation: InvocationId,
    pub(super) pid:        u32,
}

/// Exact text, columns, placement and destination permit reuse of one row's writes.
pub(super) struct RowPaintingFragment {
    area:        Rect,
    buffer_area: Rect,
    columns:     Vec<usize>,
    widths:      Vec<u16>,
    spacing:     u16,
    cells:       super::PreparedRowCells,
    before:      CellPaintSurface,
    changes:     CellPaintPatch,
}

impl RowPaintingFragment {
    pub(super) fn draw(
        buffer: &mut Buffer,
        area: Rect,
        row: &super::LaidOutProcessRow<'_>,
        layout: &super::CellTableLayout,
    ) {
        let identity = RowFragmentIdentity {
            invocation: row.source.process.invocation_id.clone(),
            pid:        row.source.process.pid,
        };
        if row.source.is_ended() {
            layout
                .prepared_rows
                .borrow_mut()
                .paintings
                .remove(&identity);
            Self::draw_text(buffer, area, row, layout);
            return;
        }
        let buffer_area = buffer.area;
        let mut prepared = layout.prepared_rows.borrow_mut();
        let paintings = &mut prepared.paintings;
        if let Some(previous) = paintings.get(&identity)
            && previous.area == area
            && previous.buffer_area == buffer_area
            && previous.columns == layout.columns
            && previous.widths == layout.column_widths
            && previous.spacing == layout.column_spacing
            && previous
                .cells
                .0
                .iter()
                .zip(&row.cells.0)
                .all(|(before, after)| Rc::ptr_eq(before, after) || before == after)
            && previous.before.matches(buffer)
        {
            previous.changes.apply(buffer);
            #[cfg(test)]
            super::tests::ROW_FRAGMENT_HITS.with(|hits| hits.set(hits.get() + 1));
            return;
        }
        let before = CellPaintSurface::read(buffer, area);
        Self::draw_text(buffer, area, row, layout);
        let changes = CellPaintPatch::between(&before, buffer, &RetainedInk::default());
        paintings.insert(
            identity,
            Self {
                area,
                buffer_area: buffer.area,
                columns: layout.columns.clone(),
                widths: layout.column_widths.clone(),
                spacing: layout.column_spacing,
                cells: row.cells.clone(),
                before,
                changes,
            },
        );
    }

    fn draw_text(
        buffer: &mut Buffer,
        area: Rect,
        row: &super::LaidOutProcessRow<'_>,
        layout: &super::CellTableLayout,
    ) {
        layout.draw_columns(
            buffer,
            area,
            row.cells
                .iter()
                .enumerate()
                .filter(|(column, _)| layout.columns.contains(column))
                .map(|(column, cell)| {
                    if super::numeric_column(column) && !cell.lines.is_empty() {
                        Cow::Owned(cell.clone().right_aligned())
                    } else {
                        Cow::Borrowed(cell)
                    }
                }),
        );
    }
}

/// The table painter identifies its column header in source coordinates.
#[derive(Clone, Copy, Default)]
pub(super) enum CellColumnHeader {
    #[default]
    Absent,
    Row(u16),
}

/// The table owns each command fragment's position and shared prepared text.
pub(super) struct PlacedCommandText {
    pub(super) invocation: InvocationId,
    pub(super) pid:        u32,
    pub(super) area:       Rect,
    pub(super) text:       Rc<Text<'static>>,
}

impl PlacedCommandText {
    pub(super) fn show(&self, buffer: &Buffer, row: &TrackedRow) {
        tui_pane::PaneFrame::with_visible_text(buffer, self.area, &self.text, || {
            row.show_command_text(VisibleCommandText(()));
        });
    }

    pub(super) fn show_in_roster(&self, buffer: &Buffer, roster: &VisibleRoster) {
        if let Some(row) = roster
            .groups()
            .iter()
            .flat_map(TrackedGroup::rows)
            .find(|row| row.process.invocation_id == self.invocation && row.process.pid == self.pid)
        {
            self.show(buffer, row);
        }
    }
}

/// A cell painting belongs to one source snapshot, geometry, policy and destination.
#[derive(Eq, PartialEq)]
pub(super) struct CellPaintInputs {
    pub(super) measurement: CellMeasurementKey,
    pub(super) view:        GridView,
    pub(super) inner:       Rect,
    pub(super) buffer_area: Rect,
    pub(super) ground:      Color,
}

impl CellPaintInputs {
    pub(super) fn same_destination(&self, after: &Self) -> bool {
        self.inner == after.inner
            && self.buffer_area == after.buffer_area
            && self.view == after.view
            && self.ground == after.ground
    }

    pub(super) fn describe_change(&self, after: &Self) -> String {
        macro_rules! changed {
            ($field:ident) => {
                if self.$field != after.$field {
                    return format!(
                        "input {}: {:?} -> {:?}",
                        stringify!($field),
                        self.$field,
                        after.$field
                    );
                }
            };
        }
        changed!(inner);
        changed!(buffer_area);
        changed!(view);
        changed!(ground);
        format!(
            "input measurement: {:?} -> {:?}",
            self.measurement, after.measurement
        )
    }
}

/// Completion changes the row's display and resource ownership.
#[derive(Debug, Eq, PartialEq)]
enum PaintedCompletion {
    Live,
    Finished,
}

impl PaintedCompletion {
    const fn of(row: &TrackedRow) -> Self {
        if row.is_ended() {
            Self::Finished
        } else {
            Self::Live
        }
    }
}

/// A token stays equal only while all static painting dependencies stay equal.
#[derive(Clone, Debug, Default)]
pub(super) struct RowPaintRevision(Rc<()>);

impl PartialEq for RowPaintRevision {
    fn eq(&self, other: &Self) -> bool { Rc::ptr_eq(&self.0, &other.0) }
}
impl Eq for RowPaintRevision {}

/// The frame owns exact painting revisions without retaining older process snapshots.
#[derive(Clone)]
pub(super) struct RowPaintRevisions {
    rows: HashMap<RowFragmentIdentity, RowPaintRevision>,
}

impl RowPaintRevisions {
    pub(super) fn new(roster: &VisibleRoster) -> Self {
        Self {
            rows: roster
                .groups()
                .iter()
                .flat_map(TrackedGroup::rows)
                .map(|row| {
                    (
                        RowFragmentIdentity {
                            invocation: row.process.invocation_id.clone(),
                            pid:        row.process.pid,
                        },
                        RowPaintRevision::default(),
                    )
                })
                .collect(),
        }
    }

    pub(super) fn reuse(
        &mut self,
        roster: &VisibleRoster,
        before: &VisibleRoster,
        previous: &Self,
    ) {
        let old_rows: HashMap<_, _> = before
            .groups()
            .iter()
            .flat_map(TrackedGroup::rows)
            .map(|row| ((&row.process.invocation_id, row.process.pid), row))
            .collect();
        for row in roster.groups().iter().flat_map(TrackedGroup::rows) {
            let identity = RowFragmentIdentity {
                invocation: row.process.invocation_id.clone(),
                pid:        row.process.pid,
            };
            if let Some(old) = old_rows.get(&(&row.process.invocation_id, row.process.pid))
                && Self::same_readings(old, row)
                && let Some(revision) = previous.rows.get(&identity)
            {
                self.rows.insert(identity, revision.clone());
            }
        }
    }

    /// Live resource values have their own exact comparison after table planning.
    fn same_readings(before: &TrackedRow, after: &TrackedRow) -> bool {
        let a = &before.process;
        let b = &after.process;
        PaintedCompletion::of(before) == PaintedCompletion::of(after)
            && before.family() == after.family()
            && before.parent_family() == after.parent_family()
            && super::progress_ownership(before) == super::progress_ownership(after)
            && (Rc::ptr_eq(&before.snapshot(), &after.snapshot())
                || (a.invocation_id == b.invocation_id
                    && a.pid == b.pid
                    && a.path == b.path
                    && a.directory_identity == b.directory_identity
                    && a.parent == b.parent
                    && a.provenance == b.provenance
                    && a.state == b.state
                    && a.nested == b.nested
                    && a.command == b.command
                    && super::managed_text(a) == super::managed_text(b)
                    && (a.origin_start == b.origin_start || a.start() == b.start())))
    }

    fn revision(&self, row: &TrackedRow) -> RowPaintRevision {
        let identity = RowFragmentIdentity {
            invocation: row.process.invocation_id.clone(),
            pid:        row.process.pid,
        };
        self.rows
            .get(&identity)
            .map_or_else(RowPaintRevision::default, Clone::clone)
    }
}

/// A row painting keeps its identity and revision instead of a process snapshot.
struct PaintedRowSource {
    identity:   RowFragmentIdentity,
    revision:   RowPaintRevision,
    completion: PaintedCompletion,
}

impl PaintedRowSource {
    fn read(row: &TrackedRow, revisions: &RowPaintRevisions) -> Self {
        Self {
            identity:   RowFragmentIdentity {
                invocation: row.process.invocation_id.clone(),
                pid:        row.process.pid,
            },
            revision:   revisions.revision(row),
            completion: PaintedCompletion::of(row),
        }
    }

    fn matches(&self, row: &TrackedRow, revisions: &RowPaintRevisions) -> bool {
        let unchanged = self.identity.invocation == row.process.invocation_id
            && self.identity.pid == row.process.pid
            && self.revision == revisions.revision(row);
        if !unchanged {
            probe::source_rejection(|| format!("row {} paint revision changed", self.identity.pid));
        }
        unchanged
    }
}

/// Each cell retains the source details that its painter reads.
#[derive(PartialEq)]
enum CellSourceDetails {
    Empty,
    Group(super::AncestryWalk<super::Ancestor>),
    Summary(Vec<&'static str>),
}

/// A painting keeps column decisions without retaining frame preparation.
struct PaintedColumnGrid {
    theme:                    Arc<super::Theme>,
    slots:                    Vec<usize>,
    separation:               super::DirectorySeparation,
    parent_content:           super::ParentColumnContent,
    parent_links:             super::AncestryParentLinks,
    row_progress:             Vec<InvocationId>,
    intrinsic_widths:         [u16; super::TABLE_HEADERS.len()],
    value_widths:             [u16; super::TABLE_HEADERS.len()],
    trailing_clearance:       [u16; super::TABLE_HEADERS.len()],
    command_title_width:      u16,
    longest_command_width:    u16,
    longest_subcommand_width: u16,
    continuation_clearance:   u16,
    table_budgets:            Vec<super::TableWidthBudget>,
    policy:                   super::CellMeasurementPolicy,
    empty_tail_columns:       Vec<usize>,
    state_column:             super::TableStateColumn,
    kind:                     super::TableKind,
    view:                     super::CommandView,
}

impl PaintedColumnGrid {
    fn same_column_inputs(&self, grid: &super::FrameTableColumnGrid) -> bool {
        self.intrinsic_widths == grid.intrinsic_widths
            && self.trailing_clearance
                == std::array::from_fn(|column| grid.trailing_column_clearance(column))
            && self.command_title_width == grid.command_title_width
            && self.longest_command_width == grid.longest_command_width
            && self.longest_subcommand_width == grid.longest_subcommand_width
            && self.continuation_clearance == grid.continuation_clearance
            && self.table_budgets == grid.table_budgets
            && self.policy == super::CellMeasurementPolicy::of(grid)
    }

    fn of(grid: &super::FrameTableColumnGrid) -> Self {
        Self {
            theme:                    std::sync::Arc::clone(&grid.theme),
            slots:                    grid.slots.clone(),
            separation:               grid.separation,
            parent_content:           grid.parent_content,
            parent_links:             grid.parent_links.clone(),
            row_progress:             grid.row_progress.clone(),
            intrinsic_widths:         grid.intrinsic_widths,
            value_widths:             grid.value_widths,
            trailing_clearance:       std::array::from_fn(|column| {
                grid.trailing_column_clearance(column)
            }),
            command_title_width:      grid.command_title_width,
            longest_command_width:    grid.longest_command_width,
            longest_subcommand_width: grid.longest_subcommand_width,
            continuation_clearance:   grid.continuation_clearance,
            table_budgets:            grid.table_budgets.clone(),
            policy:                   super::CellMeasurementPolicy::of(grid),
            empty_tail_columns:       grid.empty_tail_columns.clone(),
            state_column:             grid.state_column,
            kind:                     grid.kind,
            view:                     grid.view.clone(),
        }
    }
}

/// A painting retains its row inputs and the frame's column decisions.
pub(super) struct CellPaintSource {
    details:   CellSourceDetails,
    rows:      Vec<PaintedRowSource>,
    grid:      PaintedColumnGrid,
    resources: Vec<PaintedResourceReading>,
    table:     PaintedTableColumns,
}

/// A painting is valid for the current source, with the readings its live values show.
pub(super) enum CellSourceMatch {
    Unchanged(Vec<super::RowResourceReading>),
    Changed,
}

/// The painting retains compiler observations. Live values draw on each frame.
#[derive(Debug, PartialEq)]
struct PaintedResourceReading {
    invocation: InvocationId,
    pid:        u32,
    compiler:   super::CompilerObservation,
}

impl From<&super::RowResourceReading> for PaintedResourceReading {
    fn from(reading: &super::RowResourceReading) -> Self {
        Self {
            invocation: reading.invocation.clone(),
            pid:        reading.pid,
            compiler:   reading.resources.compiler.clone(),
        }
    }
}

impl CellPaintSource {
    fn rows<'a>(roster: &'a VisibleRoster, content: &super::TileContent) -> Vec<&'a TrackedRow> {
        match content {
            super::TileContent::Summary => roster
                .groups()
                .iter()
                .flat_map(TrackedGroup::rows)
                .collect(),
            super::TileContent::Group(id) => roster
                .groups()
                .iter()
                .find(|group| &group.id == id)
                .map_or_else(Vec::new, |group| group.rows().collect()),
            super::TileContent::Empty(_) => Vec::new(),
        }
    }

    fn details(roster: &VisibleRoster, content: &super::TileContent) -> CellSourceDetails {
        match content {
            super::TileContent::Summary => CellSourceDetails::Summary(
                super::summary_memory_rows(roster)
                    .map(|(label, _)| label)
                    .collect(),
            ),
            super::TileContent::Group(id) => roster
                .groups()
                .iter()
                .find(|group| &group.id == id)
                .map_or(CellSourceDetails::Empty, |group| {
                    CellSourceDetails::Group(group.ancestry().clone())
                }),
            super::TileContent::Empty(_) => CellSourceDetails::Empty,
        }
    }

    const fn grid<'a>(
        grids: &'a super::FrameColumnGrids,
        content: &super::TileContent,
    ) -> &'a super::FrameTableColumnGrid {
        match content {
            super::TileContent::Group(_) => &grids.commands.0,
            super::TileContent::Summary | super::TileContent::Empty(_) => &grids.summary.0,
        }
    }

    pub(super) fn read(
        roster: &VisibleRoster,
        content: &super::TileContent,
        grids: &super::FrameColumnGrids,
    ) -> Self {
        let rows = Self::rows(roster, content);
        let grid = Self::grid(grids, content);
        Self {
            details:   Self::details(roster, content),
            rows:      rows
                .iter()
                .map(|row| PaintedRowSource::read(row, &grids.paint_revisions))
                .collect(),
            grid:      PaintedColumnGrid::of(grid),
            resources: Vec::new(),
            table:     PaintedTableColumns::Absent,
        }
    }

    pub(super) fn has_finished_rows(&self) -> bool {
        self.rows
            .iter()
            .any(|row| row.completion == PaintedCompletion::Finished)
    }

    pub(super) fn record_resources(&mut self, reach: &CellPaintReach) {
        self.table = reach.table.clone();
        self.resources = reach
            .resources
            .iter()
            .map(PaintedResourceReading::from)
            .collect();
    }

    fn resources_match(
        &self,
        rows: &[&TrackedRow],
        grid: &super::FrameTableColumnGrid,
    ) -> CellSourceMatch {
        let selected: Vec<_> = self
            .resources
            .iter()
            .filter_map(|before| {
                rows.iter().copied().find(|row| {
                    before.invocation == row.process.invocation_id && before.pid == row.process.pid
                })
            })
            .collect();
        if selected.len() != self.resources.len() {
            probe::source_rejection(|| "resource row identity changed".to_owned());
            return CellSourceMatch::Changed;
        }
        let readings = super::row_resource_readings(&selected, &grid.contributions);
        for (before, reading) in self.resources.iter().zip(&readings) {
            let after = PaintedResourceReading::from(reading);
            if *before != after {
                probe::source_rejection(|| format!("resources: {before:?} -> {after:?}"));
                return CellSourceMatch::Changed;
            }
        }
        CellSourceMatch::Unchanged(readings)
    }

    pub(super) fn matches(
        &self,
        roster: &VisibleRoster,
        content: &super::TileContent,
        grids: &super::FrameColumnGrids,
    ) -> CellSourceMatch {
        let grid = Self::grid(grids, content);
        if !self.layout_matches(grid) {
            return CellSourceMatch::Changed;
        }
        if self.details != Self::details(roster, content) {
            probe::source_rejection(|| "source details changed".to_owned());
            return CellSourceMatch::Changed;
        }
        let rows = Self::rows(roster, content);
        if self.rows.len() != rows.len() {
            probe::source_rejection(|| {
                format!("source row count: {} -> {}", self.rows.len(), rows.len())
            });
            return CellSourceMatch::Changed;
        }
        if !self
            .rows
            .iter()
            .zip(&rows)
            .all(|(before, row)| before.matches(row, &grids.paint_revisions))
        {
            return CellSourceMatch::Changed;
        }
        if !self.grid.same_column_inputs(grid) && !self.table_columns_match(roster, content, grid) {
            probe::source_rejection(|| "layout column decisions changed".to_owned());
            return CellSourceMatch::Changed;
        }
        self.resources_match(&rows, grid)
    }

    fn table_columns_match(
        &self,
        roster: &VisibleRoster,
        content: &super::TileContent,
        grid: &super::FrameTableColumnGrid,
    ) -> bool {
        match content {
            super::TileContent::Group(_) => with_painted_rows(roster, content, |rows| {
                let pinned = rows
                    .first()
                    .map_or(PinnedGroup::Unpinned, |row| PinnedGroup::Lead(&row.process));
                self.table
                    .matches(&super::table_grid_after_ancestry(grid, 0), rows, pinned)
            }),
            super::TileContent::Summary => {
                self.table
                    .matches(grid, &super::summary_rows(roster), PinnedGroup::Unpinned)
            },
            super::TileContent::Empty(_) => true,
        }
    }

    fn layout_matches(&self, grid: &super::FrameTableColumnGrid) -> bool {
        let before = &self.grid;
        if !std::sync::Arc::ptr_eq(&before.theme, &grid.theme) {
            probe::source_rejection(|| "layout theme changed".to_owned());
            return false;
        }
        macro_rules! compare {
            ($field:ident, values) => {
                if before.$field != grid.$field {
                    probe::source_rejection(|| {
                        format!(
                            "layout {}: {:?} -> {:?}",
                            stringify!($field),
                            before.$field,
                            grid.$field
                        )
                    });
                    return false;
                }
            };
        }
        compare!(slots, values);
        compare!(separation, values);
        compare!(parent_content, values);
        for row in &self.rows {
            let id = &row.identity.invocation;
            if before.parent_links.hidden.contains(id) != grid.parent_links.hidden.contains(id) {
                probe::source_rejection(|| "layout hidden parent changed".to_owned());
                return false;
            }
            if before.row_progress.contains(id) != grid.row_progress.contains(id) {
                probe::source_rejection(|| "layout row progress changed".to_owned());
                return false;
            }
        }
        if before.value_widths[super::MEMORY_COLUMN] != grid.value_widths[super::MEMORY_COLUMN] {
            probe::source_rejection(|| "layout memory width changed".to_owned());
            return false;
        }
        compare!(empty_tail_columns, values);
        compare!(state_column, values);
        compare!(kind, values);
        compare!(view, values);
        true
    }
}

/// A row identity selects the ink that its retained foreground uses.
#[derive(Clone)]
pub(super) struct InkRowIdentity {
    invocation: InvocationId,
    pid:        u32,
}

impl InkRowIdentity {
    fn of(row: &TrackedRow) -> Self {
        Self {
            invocation: row.process.invocation_id.clone(),
            pid:        row.process.pid,
        }
    }

    fn matches(&self, row: &TrackedRow) -> bool {
        self.invocation == row.process.invocation_id && self.pid == row.process.pid
    }
}

/// Shared headings use the minimum fade of their owning rows.
#[derive(Clone)]
pub(super) enum InkSource {
    Row(InkRowIdentity),
    OrdinaryRow(InkRowIdentity),
    Heading(Vec<InkRowIdentity>),
}

impl InkSource {
    pub(super) fn row(row: &TrackedRow) -> Self { Self::Row(InkRowIdentity::of(row)) }

    pub(super) fn ordinary(row: &TrackedRow) -> Self { Self::OrdinaryRow(InkRowIdentity::of(row)) }

    pub(super) fn heading(rows: &[&TrackedRow]) -> Self {
        Self::Heading(rows.iter().map(|row| InkRowIdentity::of(row)).collect())
    }

    fn ink(&self, rows: &[&TrackedRow]) -> super::RowInk {
        match self {
            Self::Row(identity) => rows
                .iter()
                .find(|row| identity.matches(row))
                .map_or(RowInk::Ordinary(0), |row| row.ink()),
            Self::OrdinaryRow(identity) => RowInk::Ordinary(
                rows.iter()
                    .find(|row| identity.matches(row))
                    .map_or(0, |row| row.faded()),
            ),
            Self::Heading(identities) => RowInk::Ordinary(
                identities
                    .iter()
                    .filter_map(|identity| rows.iter().find(|row| identity.matches(row)))
                    .map(|row| row.faded())
                    .min()
                    .unwrap_or_default(),
            ),
        }
    }
}

/// Each foreground keeps its source color before blending.
struct InkPixel {
    index: usize,
    role:  usize,
}

struct RetainedInkRole {
    source:    Color,
    displayed: Color,
}

/// A layer recolors foregrounds without repeating text layout or painting.
struct RetainedInkLayer {
    source: InkSource,
    pixels: Vec<InkPixel>,
    roles:  Vec<RetainedInkRole>,
}

/// A capture records only the foregrounds that a paint operation writes.
#[derive(Default)]
pub(super) struct RetainedInk {
    layers: Vec<RetainedInkLayer>,
}

enum InkRecording {
    Disabled,
    Recording { marker: Color, ink: RetainedInk },
}

thread_local! {
    static INK_RECORDING: std::cell::RefCell<InkRecording> = const { std::cell::RefCell::new(InkRecording::Disabled) };
}

/// The capture ends before its foregrounds enter the retained patch.
pub(super) struct InkCapture;

impl InkCapture {
    pub(super) fn without_ink() -> Self {
        INK_RECORDING.set(InkRecording::Disabled);
        Self
    }

    pub(super) fn start() -> Self {
        INK_RECORDING.set(InkRecording::Recording {
            marker: foreground_marker(),
            ink:    RetainedInk::default(),
        });
        Self
    }

    pub(super) fn finish(self) -> RetainedInk {
        let recorded = match INK_RECORDING.replace(InkRecording::Disabled) {
            InkRecording::Recording { ink, .. } => ink,
            InkRecording::Disabled => RetainedInk::default(),
        };
        drop(self);
        recorded
    }
}

impl Drop for InkCapture {
    fn drop(&mut self) { INK_RECORDING.set(InkRecording::Disabled); }
}

pub(super) fn records_ink() -> bool {
    INK_RECORDING.with_borrow(|state| matches!(state, InkRecording::Recording { .. }))
}

/// Prepare source colors only while a retained paint records its foregrounds.
pub(super) fn ink_ground(ground: Color) -> Color {
    if records_ink() { Color::Reset } else { ground }
}

/// Select a marker that no source foreground in this painter uses.
fn foreground_marker() -> Color {
    let mut colors = vec![
        super::label_color(),
        super::text_default(),
        super::secondary_text_color(),
        super::accent_color(),
        super::success_color(),
        super::warning_color(),
    ];
    colors.extend((0..super::theme::family_color_count()).map(super::theme::family_color));
    (0..=u8::MAX)
        .map(Color::Indexed)
        .find(|color| !colors.contains(color))
        .unwrap_or(Color::Reset)
}

/// Record exact foreground coverage while the operation draws its source colors.
pub(super) fn paint_ink<T>(
    buffer: &mut Buffer,
    area: Rect,
    source: InkSource,
    ink: super::RowInk,
    ground: Color,
    draw: impl FnOnce(&mut Buffer) -> T,
) -> T {
    if !records_ink() {
        return draw(buffer);
    }
    let recording_timer = probe::paint_timer(PaintStage::Recording);
    let area = area.intersection(buffer.area);
    probe::paint_work(|work| {
        work.recording_visits += 2 * usize::from(area.width) * usize::from(area.height);
    });
    #[cfg(test)]
    super::tests::FOREGROUND_RECORDING_VISITS.with(|count| {
        count.set(count.get() + 2 * usize::from(area.width) * usize::from(area.height));
    });
    let marker = INK_RECORDING.with_borrow(|state| match state {
        InkRecording::Recording { marker, .. } => *marker,
        InkRecording::Disabled => Color::Reset,
    });
    let mut before = Vec::with_capacity(usize::from(area.width) * usize::from(area.height));
    for y in area.top()..area.bottom() {
        let start = buffer.index_of(area.x, y);
        for cell in &mut buffer.content[start..start + usize::from(area.width)] {
            before.push(cell.fg);
            cell.fg = marker;
        }
    }
    drop(recording_timer);
    let result = draw(buffer);
    let recording_timer = probe::paint_timer(PaintStage::Recording);
    let mut pixels = Vec::new();
    let mut roles: Vec<RetainedInkRole> = Vec::new();
    let mut previous = before.into_iter();
    for y in area.top()..area.bottom() {
        let start = buffer.index_of(area.x, y);
        for (offset, cell) in buffer.content[start..start + usize::from(area.width)]
            .iter_mut()
            .enumerate()
        {
            let index = start + offset;
            let original = previous.next().unwrap_or(Color::Reset);
            if cell.fg == marker {
                cell.fg = original;
            } else {
                let color = cell.fg;
                let role = roles
                    .iter()
                    .position(|role| role.source == color)
                    .unwrap_or_else(|| {
                        roles.push(RetainedInkRole {
                            source:    color,
                            displayed: ink.color(color, ground),
                        });
                        roles.len() - 1
                    });
                pixels.push(InkPixel { index, role });
                cell.fg = roles[role].displayed;
            }
        }
    }
    INK_RECORDING.with_borrow_mut(|state| {
        if let InkRecording::Recording { ink, .. } = state {
            ink.layers.push(RetainedInkLayer {
                source,
                pixels,
                roles,
            });
        }
    });
    drop(recording_timer);
    result
}

/// Each patch position can supply the last foreground writer once.
#[derive(Clone, Copy)]
enum PatchForeground {
    Unchanged,
    Available(usize),
    Claimed,
}

impl RetainedInk {
    pub(super) fn bind(&mut self, patch: &CellPaintPatch) {
        if self.layers.is_empty() {
            return;
        }
        let (Some(first), Some(last)) = (patch.0.first(), patch.0.last()) else {
            self.layers.clear();
            return;
        };
        let origin = first.index;
        let mut positions = vec![PatchForeground::Unchanged; last.index - origin + 1];
        for (position, change) in patch.0.iter().enumerate() {
            positions[change.index - origin] = PatchForeground::Available(position);
        }
        for layer in self.layers.iter_mut().rev() {
            layer.pixels.retain_mut(|pixel| {
                let Some(entry) = pixel
                    .index
                    .checked_sub(origin)
                    .and_then(|index| positions.get_mut(index))
                else {
                    return false;
                };
                match *entry {
                    PatchForeground::Available(position) => {
                        pixel.index = position;
                        *entry = PatchForeground::Claimed;
                        true
                    },
                    PatchForeground::Unchanged | PatchForeground::Claimed => false,
                }
            });
        }
    }
}

/// An exact destination snapshot guards replay of the painter's writes.
pub(super) struct PaintedCellContents {
    pub(super) header:   CellColumnHeader,
    pub(super) inputs:   CellPaintInputs,
    pub(super) source:   CellPaintSource,
    pub(super) before:   CellPaintSurface,
    pub(super) changes:  CellPaintPatch,
    pub(super) commands: Vec<PlacedCommandText>,
    pub(super) values:   Vec<PlacedValueText>,
    pub(super) ink:      RetainedInk,
}

/// Run `read` over the rows whose ink the cell painter used.
pub(super) fn with_painted_rows<T>(
    roster: &VisibleRoster,
    content: &super::TileContent,
    read: impl FnOnce(&[&TrackedRow]) -> T,
) -> T {
    let group = match content {
        super::TileContent::Group(id) => roster
            .groups()
            .iter()
            .find(|group| &group.id == id)
            .map(|group| group.for_painting()),
        super::TileContent::Summary | super::TileContent::Empty(_) => None,
    };
    let rows = group.as_ref().map_or_else(
        || CellPaintSource::rows(roster, content),
        |group| group.rows().collect(),
    );
    read(&rows)
}

impl PaintedCellContents {
    pub(super) fn dispose_previous(previous: Option<Self>) {
        let _timer = probe::paint_timer(PaintStage::Disposal);
        if let Some(previous) = previous {
            previous.dispose();
        }
    }

    pub(super) fn dispose(self) {
        let Self {
            inputs,
            source,
            before,
            changes,
            commands,
            values,
            ink,
            header: _,
        } = self;
        let timer = probe::paint_timer(PaintStage::DisposalInputs);
        drop(inputs);
        drop(timer);
        let CellPaintSource {
            rows,
            grid,
            details,
            resources,
            table,
        } = source;
        let timer = probe::paint_timer(PaintStage::DisposalSource);
        drop((grid, details, resources, table));
        drop(timer);
        let timer = probe::paint_timer(PaintStage::DisposalSnapshots);
        drop(rows);
        drop(timer);
        let timer = probe::paint_timer(PaintStage::DisposalSurface);
        drop(before);
        drop(timer);
        let timer = probe::paint_timer(PaintStage::DisposalPatch);
        drop(changes);
        drop(timer);
        let timer = probe::paint_timer(PaintStage::DisposalText);
        drop((commands, values));
        drop(timer);
        let timer = probe::paint_timer(PaintStage::DisposalInk);
        drop(ink);
        drop(timer);
    }

    pub(super) fn recolor(&mut self, rows: &[&TrackedRow]) {
        for layer in &mut self.ink.layers {
            let ink = layer.source.ink(rows);
            let mut changed = false;
            for role in &mut layer.roles {
                let next = ink.color(role.source, self.inputs.ground);
                changed |= next != role.displayed;
                role.displayed = next;
            }
            if changed {
                for pixel in &layer.pixels {
                    self.changes.0[pixel.index].cell.fg = layer.roles[pixel.role].displayed;
                }
            }
        }
    }
}

/// Uniform destinations retain one exact cell; mixed destinations retain their full image.
pub(super) enum CellPaintSurface {
    Uniform { area: Rect, cell: Cell },
    Mixed(Buffer),
}

impl CellPaintSurface {
    pub(super) fn read(buffer: &Buffer, inner: Rect) -> Self {
        let area = inner.intersection(buffer.area);
        let width = usize::from(area.width);
        if !area.is_empty() {
            let cell = &buffer[(area.x, area.y)];
            if Self::uniform_matches(area, cell, buffer) {
                return Self::Uniform {
                    area,
                    cell: cell.clone(),
                };
            }
        }
        let mut content = Vec::with_capacity(width * usize::from(area.height));
        if width > 0 {
            for y in area.top()..area.bottom() {
                let start = buffer.index_of(area.x, y);
                content.extend_from_slice(&buffer.content[start..start + width]);
            }
        }
        probe::paint_work(|work| work.copied += content.len());
        Self::Mixed(Buffer { area, content })
    }

    pub(super) fn with_reach(self, inner: Rect, reach: &CellPaintReach) -> Self {
        let area = Rect {
            height: reach.rows.min(inner.height),
            ..inner
        }
        .intersection(self.area());
        match self {
            Self::Uniform { cell, .. } => Self::Uniform { area, cell },
            Self::Mixed(mut before) => {
                before
                    .content
                    .truncate(usize::from(area.width) * usize::from(area.height));
                before.area = area;
                Self::Mixed(before)
            },
        }
    }

    const fn area(&self) -> Rect {
        match self {
            Self::Uniform { area, .. } => *area,
            Self::Mixed(buffer) => buffer.area,
        }
    }

    fn cell(&self, index: usize) -> &Cell {
        match self {
            Self::Uniform { cell, .. } => cell,
            Self::Mixed(buffer) => &buffer.content[index],
        }
    }

    fn uniform_matches(area: Rect, cell: &Cell, buffer: &Buffer) -> bool {
        let width = usize::from(area.width);
        let mut scanned = 0;
        let matches = (area.top()..area.bottom()).all(|y| {
            let start = buffer.index_of(area.x, y);
            buffer.content[start..start + width].iter().all(|other| {
                scanned += 1;
                other == cell
            })
        });
        probe::paint_work(|work| work.scanned += scanned);
        matches
    }

    pub(super) fn matches(&self, buffer: &Buffer) -> bool {
        let area = self.area();
        let width = usize::from(area.width);
        if width == 0 {
            return true;
        }
        match self {
            Self::Uniform { cell, .. } => Self::uniform_matches(area, cell, buffer),
            Self::Mixed(before) => before
                .content
                .chunks(width)
                .zip(area.top()..area.bottom())
                .all(|(row, y)| {
                    let start = buffer.index_of(area.x, y);
                    let mut scanned = 0;
                    let matches =
                        row.iter()
                            .zip(&buffer.content[start..start + width])
                            .all(|(a, b)| {
                                scanned += 1;
                                a == b
                            });
                    probe::paint_work(|work| work.scanned += scanned);
                    matches
                }),
        }
    }
}

/// Writes made by one painting; untouched destination cells need no copy or storage.
pub(super) struct CellPaintPatch(Vec<PaintedCellChange>);

struct PaintedCellChange {
    index: usize,
    cell:  Cell,
}

impl CellPaintPatch {
    pub(super) fn between(before: &CellPaintSurface, buffer: &Buffer, ink: &RetainedInk) -> Self {
        let area = before.area();
        let width = usize::from(area.width);
        let origin = if area.is_empty() {
            0
        } else {
            buffer.index_of(area.x, area.y)
        };
        let span =
            usize::from(area.height.saturating_sub(1)) * usize::from(buffer.area.width) + width;
        let mut foregrounds = if ink.layers.is_empty() {
            Vec::new()
        } else {
            vec![false; span]
        };
        for pixel in ink.layers.iter().flat_map(|layer| &layer.pixels) {
            if let Some(written) = pixel
                .index
                .checked_sub(origin)
                .and_then(|index| foregrounds.get_mut(index))
            {
                *written = true;
            }
        }
        let mut changes = Vec::with_capacity(width * usize::from(area.height));
        if width > 0 {
            for (row, y) in (area.top()..area.bottom()).enumerate() {
                let start = buffer.index_of(area.x, y);
                for (offset, cell) in buffer.content[start..start + width].iter().enumerate() {
                    if before.cell(row * width + offset) != cell
                        || foregrounds
                            .get(start + offset - origin)
                            .copied()
                            .unwrap_or(false)
                    {
                        changes.push(PaintedCellChange {
                            index: start + offset,
                            cell:  cell.clone(),
                        });
                    }
                }
            }
        }
        probe::paint_work(|work| {
            work.scanned += width * usize::from(area.height);
            work.copied += changes.len();
        });
        Self(changes)
    }

    pub(super) fn apply(&self, buffer: &mut Buffer) {
        probe::paint_work(|work| work.copied += self.0.len());
        for change in &self.0 {
            buffer.content[change.index].clone_from(&change.cell);
        }
    }
}

#[cfg(test)]
mod cell_paint_tests {
    use std::time::Duration;
    use std::time::Instant;

    use crate::census::CargoGroup;
    use crate::census::CompilerObservation;
    use crate::census::Measurement;
    use crate::census::invocation_cpu_accounting::CompileMemoryShares;
    use crate::census::process_identity::CaptureMembership;
    use crate::census::scan::Compiler;
    use crate::census::scan::OriginStart;
    use crate::constants::BYTES_PER_GIBIBYTE;
    use crate::render;
    use crate::render::Ancestor;
    use crate::render::AncestryWalk;
    use crate::render::Buffer;
    use crate::render::CaptureLookup;
    use crate::render::CaptureRead;
    use crate::render::Cells;
    use crate::render::Color;
    use crate::render::CommandText;
    use crate::render::CommandView;
    use crate::render::FreeDiskSpace;
    use crate::render::InvocationId;
    use crate::render::Phase;
    use crate::render::Progress;
    use crate::render::Rect;
    use crate::render::Roster;
    use crate::render::RunState;
    use crate::render::SccacheStats;
    use crate::render::TableRowEnd;
    use crate::render::TileCells;
    use crate::render::TileContent;
    use crate::render::VisibleParent;
    use crate::render::WorkingDirectoryIdentity;
    use crate::render::frame_preparation::PreparedTables;
    use crate::render::tests;
    use crate::render::tests::CELL_CONTENT_PAINTS;

    #[test]
    fn painting_sources_release_frame_preparation() {
        let mut app = tests::argument_toggle_app();
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let content = TileContent::Group(app.roster.visible(&[]).groups()[0].id.clone());
        let prepared = tables.for_frame(
            &mut app.roster,
            &[],
            CommandView::long(),
            80,
            FreeDiskSpace::Unavailable,
        );
        let cells = Cells::from_prepared(prepared, &stats);
        let rows = std::rc::Rc::strong_count(&cells.grids.commands.0.prepared_rows);
        let plans = std::rc::Rc::strong_count(&cells.grids.commands.0.prepared_plans);
        let snapshot = cells.roster.groups()[0].lead.snapshot();
        let snapshot_owners = std::rc::Rc::strong_count(&snapshot);
        let source = super::CellPaintSource::read(&cells.roster, &content, &cells.grids);
        assert_eq!(
            std::rc::Rc::strong_count(&snapshot),
            snapshot_owners,
            "painting sources do not retain process snapshots"
        );
        assert_eq!(
            std::rc::Rc::strong_count(&cells.grids.commands.0.prepared_rows),
            rows
        );
        assert_eq!(
            std::rc::Rc::strong_count(&cells.grids.commands.0.prepared_plans),
            plans
        );
        assert!(matches!(
            source.matches(&cells.roster, &content, &cells.grids),
            super::CellSourceMatch::Unchanged(_)
        ));
    }

    #[test]
    fn roster_revisions_reuse_prepared_process_rows() {
        let mut app = tests::argument_toggle_app();
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let content = TileContent::Group(app.roster.visible(&[]).groups()[0].id.clone());
        let view = CommandView::long();
        let area = Rect::new(0, 0, 160, 80);
        let first = Cells::from_prepared(
            tables.for_frame(&mut app.roster, &[], view, 80, FreeDiskSpace::Unavailable),
            &stats,
        );
        first.prepare_frame(|| vec![(content.clone(), area)]);
        first.draw(&mut Buffer::empty(area), &content, area, Color::Black);
        app.roster.observe_memory_shares(CompileMemoryShares {
            shared_compiler_cache: BYTES_PER_GIBIBYTE,
            unattributed:          0,
        });
        let next = Cells::from_prepared(
            tables.for_frame(&mut app.roster, &[], view, 80, FreeDiskSpace::Unavailable),
            &stats,
        );
        assert!(std::rc::Rc::ptr_eq(
            &first.grids.commands.0.prepared_rows,
            &next.grids.commands.0.prepared_rows
        ));
        assert!(std::rc::Rc::ptr_eq(
            &first.grids.summary.0.prepared_rows,
            &next.grids.summary.0.prepared_rows
        ));
        next.prepare_frame(|| vec![(content.clone(), area)]);
        let mut actual = Buffer::empty(area);
        next.draw(&mut actual, &content, area, Color::Black);
        let mut fresh = Buffer::empty(area);
        render::frame_draw_contents_and_values(
            &mut fresh,
            &next.roster,
            &content,
            area,
            Color::Black,
            &next.grids,
        );
        assert_eq!(actual, fresh);
    }

    #[test]
    fn summary_height_changes_validate_cached_progress() {
        let mut roster = Roster::new();
        let groups = (0..5)
            .map(|index| {
                let mut process = tests::invocation(8100 + index, &["build"]);
                process.path = format!("/project/{index}");
                process.directory_identity =
                    WorkingDirectoryIdentity::Absolute(process.path.clone().into());
                process.state =
                    CaptureLookup::Registered(CaptureRead::Progress(RunState::Working {
                        phase:    Phase::Building,
                        progress: Progress {
                            done:  10 + index as usize,
                            total: 100,
                        },
                    }));
                tests::cargo_group(process)
            })
            .collect();
        roster.observe(groups, Instant::now());
        let stats = SccacheStats::default();
        for rebuild in [false, true] {
            let mut tables = PreparedTables::default();
            let view = CommandView::long();
            for height in [6, 10] {
                let area = Rect::new(0, 0, 160, height);
                let frame_height = if rebuild { height } else { 80 };
                let cells = Cells::from_prepared(
                    tables.for_frame(
                        &mut roster,
                        &[],
                        view,
                        frame_height,
                        FreeDiskSpace::Unavailable,
                    ),
                    &stats,
                );
                cells.prepare_frame(|| vec![(TileContent::Summary, area)]);
                let mut actual = Buffer::empty(area);
                cells.draw(&mut actual, &TileContent::Summary, area, Color::Black);
                let fresh_cells = Cells::for_frame(
                    roster.visible(&[]),
                    view,
                    &stats,
                    FreeDiskSpace::Unavailable,
                    frame_height,
                );
                let mut fresh = Buffer::empty(area);
                fresh_cells.draw(&mut fresh, &TileContent::Summary, area, Color::Black);
                assert_eq!(actual, fresh, "summary height {height}");
            }
        }
    }

    #[test]
    fn geometry_repaints_reuse_unchanged_source_snapshots() {
        let mut app = tests::argument_toggle_app();
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let content = TileContent::Group(app.roster.visible(&[]).groups()[0].id.clone());
        let identity = super::super::CellRowIdentity::from(&content);
        let first = Cells::from_prepared(
            tables.for_frame(
                &mut app.roster,
                &[],
                CommandView::long(),
                80,
                FreeDiskSpace::Unavailable,
            ),
            &stats,
        );
        let area = Rect::new(0, 0, 160, 30);
        first.draw(&mut Buffer::empty(area), &content, area, Color::Black);
        let snapshot = first.roster.groups()[0].lead.snapshot();
        let revision = first.paintings.borrow()[&identity].source.rows[0]
            .revision
            .clone();
        let metadata = std::rc::Rc::clone(
            &first
                .grids
                .commands
                .0
                .prepared_rows
                .borrow()
                .ordinary
                .by_pid[&4100][0]
                .cells
                .0[crate::render::PID_COLUMN],
        );
        let mut group = current_lead_group(&mut app.roster);
        group.lead.resource_use.cpu = Measurement::Reading(34.0);
        app.roster.observe(vec![group], Instant::now());
        let next = Cells::from_prepared(
            tables.for_frame(
                &mut app.roster,
                &[],
                CommandView::long(),
                80,
                FreeDiskSpace::Unavailable,
            ),
            &stats,
        );
        assert!(!std::rc::Rc::ptr_eq(
            &snapshot,
            &next.roster.groups()[0].lead.snapshot()
        ));
        let area = Rect { height: 28, ..area };
        let mut actual = Buffer::empty(area);
        let snapshot_owners = std::rc::Rc::strong_count(&next.roster.groups()[0].lead.snapshot());
        next.draw(&mut actual, &content, area, Color::Black);
        assert_eq!(
            revision,
            next.paintings.borrow()[&identity].source.rows[0].revision
        );
        assert_eq!(
            std::rc::Rc::strong_count(&next.roster.groups()[0].lead.snapshot()),
            snapshot_owners
        );
        let fresh_cells = Cells::for_frame(
            app.roster.visible(&[]),
            CommandView::long(),
            &stats,
            FreeDiskSpace::Unavailable,
            80,
        );
        let mut fresh = Buffer::empty(area);
        fresh_cells.draw(&mut fresh, &content, area, Color::Black);
        assert_eq!(actual, fresh);
        assert!(
            next.grids.commands.0.prepared_rows.borrow().ordinary.by_pid[&4100]
                .iter()
                .any(|row| std::rc::Rc::ptr_eq(&metadata, &row.cells.0[crate::render::PID_COLUMN]))
        );
        assert_command_change_gets_a_new_revision(
            &mut app.roster,
            &mut tables,
            &stats,
            &content,
            area,
            &revision,
        );
    }

    fn assert_command_change_gets_a_new_revision(
        roster: &mut Roster,
        tables: &mut PreparedTables,
        stats: &SccacheStats,
        content: &TileContent,
        area: Rect,
        revision: &super::RowPaintRevision,
    ) {
        let mut group = current_lead_group(roster);
        group.lead.command = CommandText::of("cargo", &["test", "--all-targets"]);
        roster.observe(vec![group], Instant::now());
        let cells = Cells::from_prepared(
            tables.for_frame(
                roster,
                &[],
                CommandView::long(),
                80,
                FreeDiskSpace::Unavailable,
            ),
            stats,
        );
        let mut actual = Buffer::empty(area);
        cells.draw(&mut actual, content, area, Color::Black);
        let identity = super::super::CellRowIdentity::from(content);
        assert_ne!(
            revision,
            &cells.paintings.borrow()[&identity].source.rows[0].revision
        );
        let fresh = Cells::for_frame(
            roster.visible(&[]),
            CommandView::long(),
            stats,
            FreeDiskSpace::Unavailable,
            80,
        );
        let mut expected = Buffer::empty(area);
        fresh.draw(&mut expected, content, area, Color::Black);
        assert_eq!(actual, expected);
    }

    #[test]
    fn source_rejections_name_details_and_row_count() {
        let mut app = tests::argument_toggle_app();
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let content = TileContent::Group(app.roster.visible(&[]).groups()[0].id.clone());
        let prepared = tables.for_frame(
            &mut app.roster,
            &[],
            CommandView::long(),
            80,
            FreeDiskSpace::Unavailable,
        );
        let cells = Cells::from_prepared(prepared, &stats);
        for reason in ["source details changed", "source row count:"] {
            let mut source = super::CellPaintSource::read(&cells.roster, &content, &cells.grids);
            if reason == "source details changed" {
                source.details = super::CellSourceDetails::Empty;
            } else {
                source.rows.pop();
            }
            let work = crate::probe::measure_paint(|| {
                assert!(matches!(
                    source.matches(&cells.roster, &content, &cells.grids),
                    super::CellSourceMatch::Changed
                ));
            });
            assert!(
                work.first_rejection.starts_with(reason),
                "{}",
                work.first_rejection
            );
        }
    }

    #[test]
    fn roster_revisions_keep_only_unchanged_cell_paintings() {
        assert_source_row_transitions_reuse_fragments();
        for (summary, changed_command, added_group, changed_other_ancestry) in [
            (false, false, false, false),
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            if changed_other_ancestry {
                assert_other_hidden_parent_revision();
                continue;
            }
            let mut app = tests::argument_toggle_app();
            let stats = SccacheStats::default();
            let mut tables = PreparedTables::default();
            let content = if summary {
                TileContent::Summary
            } else {
                TileContent::Group(app.roster.visible(&[]).groups()[0].id.clone())
            };
            let view = CommandView::long();
            let prepared =
                tables.for_frame(&mut app.roster, &[], view, 80, FreeDiskSpace::Unavailable);
            let cells = Cells::from_prepared(prepared, &stats);
            let area = Rect::new(0, 0, 160, 80);
            let original = Buffer::empty(area);
            let mut first = original.clone();
            cells.prepare_frame(|| vec![(content.clone(), area)]);
            cells.draw(&mut first, &content, area, Color::Black);
            if added_group {
                let group = current_lead_group(&mut app.roster);
                app.roster.observe(
                    vec![
                        group,
                        tests::cargo_group(tests::invocation(9900, &["check"])),
                    ],
                    Instant::now(),
                );
            } else if changed_command {
                let mut group = current_lead_group(&mut app.roster);
                group.lead.command = CommandText::of("cargo", &["test", "--all-targets"]);
                app.roster.observe(vec![group], Instant::now());
            } else {
                app.roster.observe_memory_shares(CompileMemoryShares {
                    shared_compiler_cache: BYTES_PER_GIBIBYTE,
                    unattributed:          0,
                });
            }
            let prepared =
                tables.for_frame(&mut app.roster, &[], view, 80, FreeDiskSpace::Unavailable);
            let cells = Cells::from_prepared(prepared, &stats);
            let mut actual = original.clone();
            cells.prepare_frame(|| vec![(content.clone(), area)]);
            CELL_CONTENT_PAINTS.with(|count| count.set(0));
            cells.draw(&mut actual, &content, area, Color::Black);
            CELL_CONTENT_PAINTS.with(|count| {
                assert_eq!(
                    count.get(),
                    usize::from(summary || changed_command),
                    "only changed cell sources need painting"
                );
            });
            let mut fresh = original.clone();
            render::frame_draw_contents_and_values(
                &mut fresh,
                &cells.roster,
                &content,
                area,
                Color::Black,
                &cells.grids,
            );
            assert_eq!(
                actual, fresh,
                "retained paintings match the current visible source"
            );
        }
        for ground in [Color::Reset, Color::Black, Color::Indexed(16)] {
            assert_brighten_revision(ground);
        }
        tests::assert_first_frame_painting_revisions();
        for change in [
            DisplayRevision::Subsecond,
            DisplayRevision::Second,
            DisplayRevision::CpuBelowBoundary,
            DisplayRevision::CpuAcrossBoundary,
            DisplayRevision::MemoryBelowBoundary,
            DisplayRevision::MemoryAcrossBoundary,
            DisplayRevision::HiddenCpuAcrossBoundary,
            DisplayRevision::HiddenCpuSameTotal,
            DisplayRevision::ProgressOwnership,
            DisplayRevision::CpuShorterValue,
            DisplayRevision::CpuNarrowerValue,
            DisplayRevision::MemoryWider,
            DisplayRevision::CompilerStarts,
            DisplayRevision::IntrinsicCommandWidth,
            DisplayRevision::RowEndPolicy,
        ] {
            assert_display_revision(change);
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum SourceRowChange {
        Insert,
        Remove,
        Reorder,
        Hidden,
    }

    fn assert_source_row_transitions_reuse_fragments() {
        for change in [
            SourceRowChange::Insert,
            SourceRowChange::Remove,
            SourceRowChange::Reorder,
            SourceRowChange::Hidden,
        ] {
            let mut before = tests::cargo_group(tests::invocation(4100, &["build"]));
            before.rest = (4101..4105)
                .map(|pid| tests::invocation(pid, &["check"]))
                .collect();
            let mut after = before.clone();
            match change {
                SourceRowChange::Insert | SourceRowChange::Hidden => {
                    after.rest.push(tests::invocation(4105, &["check"]));
                },
                SourceRowChange::Remove => {
                    after.rest.remove(1);
                },
                SourceRowChange::Reorder => {
                    after.rest[1].path = "/project/other".to_owned();
                },
            }
            let mut roster = Roster::new();
            roster.observe(vec![before], Instant::now());
            let stats = SccacheStats::default();
            let mut tables = PreparedTables::default();
            let view = CommandView::long();
            let first = Cells::from_prepared(
                tables.for_frame(&mut roster, &[], view, 80, FreeDiskSpace::Unavailable),
                &stats,
            );
            let content = TileContent::Group(first.roster.groups()[0].id.clone());
            let area = Rect::new(
                0,
                0,
                160,
                if matches!(change, SourceRowChange::Hidden) {
                    4
                } else {
                    16
                },
            );
            first.prepare_frame(|| vec![(content.clone(), area)]);
            first.draw(&mut Buffer::empty(area), &content, area, Color::Black);
            roster.observe(vec![after], Instant::now());
            let next = Cells::from_prepared(
                tables.for_frame(&mut roster, &[], view, 80, FreeDiskSpace::Unavailable),
                &stats,
            );
            next.prepare_frame(|| vec![(content.clone(), area)]);
            tests::ROW_FRAGMENT_HITS.with(|hits| hits.set(0));
            let mut actual = Buffer::empty(area);
            next.draw(&mut actual, &content, area, Color::Black);
            if matches!(change, SourceRowChange::Insert | SourceRowChange::Hidden) {
                tests::ROW_FRAGMENT_HITS.with(|hits| assert!(hits.get() > 0, "{change:?}"));
            }
            let fresh = Cells::for_frame(
                roster.visible(&[]),
                view,
                &stats,
                FreeDiskSpace::Unavailable,
                80,
            );
            fresh.prepare_frame(|| vec![(content.clone(), area)]);
            let mut expected = Buffer::empty(area);
            fresh.draw(&mut expected, &content, area, Color::Black);
            assert_eq!(actual, expected, "{change:?}");
        }
    }

    fn current_lead_group(roster: &mut Roster) -> CargoGroup {
        let visible = roster.visible(&[]);
        let before = &visible.groups()[0];
        let mut group = tests::cargo_group(before.lead.process.clone());
        group.ancestry = before.ancestry().clone();
        group
    }

    fn assert_other_hidden_parent_revision() {
        let mut app = tests::argument_toggle_app();
        let visible = app.roster.visible(&[]);
        let before = &visible.groups()[0];
        let mut group = tests::cargo_group(before.lead.process.clone());
        group.ancestry = before.ancestry().clone();
        let content = TileContent::Group(before.id.clone());
        let mut other = tests::invocation(9900, &["check"]);
        other.parent = VisibleParent::Ancestor(9800);
        let mut other = tests::cargo_group(other);
        other.ancestry = AncestryWalk::Ended(vec![Ancestor {
            pid:            9800,
            command:        CommandText::from_display_line("bash --login"),
            passes_through: false,
        }]);
        app.roster.observe(vec![group, other], Instant::now());
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let view = CommandView::long();
        let area = Rect::new(0, 0, 160, 80);
        let original = Buffer::empty(area);
        let cells = Cells::from_prepared(
            tables.for_frame(&mut app.roster, &[], view, 80, FreeDiskSpace::Unavailable),
            &stats,
        );
        cells.prepare_frame(|| vec![(content.clone(), area)]);
        let mut first = original.clone();
        cells.draw(&mut first, &content, area, Color::Black);
        let widths = cells.grids.commands.0.intrinsic_widths;
        assert_eq!(
            cells.grids.commands.0.parent_links.hidden,
            [] as [InvocationId; 0]
        );
        let groups = app
            .roster
            .visible(&[])
            .groups()
            .iter()
            .map(|group| CargoGroup {
                lead:     group.lead.process.clone(),
                rest:     Vec::new(),
                ancestry: if group.lead.process.pid == 9900 {
                    AncestryWalk::Ended(Vec::new())
                } else {
                    group.ancestry().clone()
                },
            })
            .collect();
        app.roster.observe(groups, Instant::now());
        let cells = Cells::from_prepared(
            tables.for_frame(&mut app.roster, &[], view, 80, FreeDiskSpace::Unavailable),
            &stats,
        );
        assert_eq!(cells.grids.commands.0.parent_links.hidden.len(), 1);
        assert_eq!(cells.grids.commands.0.intrinsic_widths, widths);
        cells.prepare_frame(|| vec![(content.clone(), area)]);
        let mut actual = original.clone();
        CELL_CONTENT_PAINTS.with(|count| count.set(0));
        cells.draw(&mut actual, &content, area, Color::Black);
        CELL_CONTENT_PAINTS.with(|count| {
            assert_eq!(count.get(), 0, "only changed cell sources need painting");
        });
        let mut fresh = original;
        render::frame_draw_contents_and_values(
            &mut fresh,
            &cells.roster,
            &content,
            area,
            Color::Black,
            &cells.grids,
        );
        assert_eq!(
            actual, fresh,
            "retained paintings match the current visible source"
        );
    }

    #[derive(Clone, Copy, Debug)]
    enum DisplayRevision {
        Subsecond,
        Second,
        CpuBelowBoundary,
        CpuAcrossBoundary,
        MemoryBelowBoundary,
        MemoryAcrossBoundary,
        HiddenCpuAcrossBoundary,
        HiddenCpuSameTotal,
        ProgressOwnership,
        CpuShorterValue,
        CpuNarrowerValue,
        MemoryWider,
        CompilerStarts,
        IntrinsicCommandWidth,
        RowEndPolicy,
    }

    /// Whether a revision changes what the cell shows.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum DisplayedValue {
        Changes,
        Stays,
    }

    fn display_groups(
        change: DisplayRevision,
    ) -> (CargoGroup, CargoGroup, usize, DisplayedValue, u16) {
        let mut lead = tests::invocation(4100, &["build"]);
        lead.origin_start = OriginStart::Process {
            epoch:   1_790_000_000,
            elapsed: Duration::from_millis(18_100),
        };
        lead.resource_use.cpu = Measurement::Reading(12.1);
        lead.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        if matches!(change, DisplayRevision::ProgressOwnership) {
            lead.state = CaptureLookup::Registered(CaptureRead::Progress(RunState::Working {
                phase:    Phase::Building,
                progress: Progress {
                    done:  5,
                    total: 10,
                },
            }));
        }
        let mut before = tests::cargo_group(lead);
        let hidden = matches!(
            change,
            DisplayRevision::HiddenCpuAcrossBoundary | DisplayRevision::HiddenCpuSameTotal
        );
        if hidden {
            before.lead.resource_use.cpu = Measurement::Reading(0.26);
            let mut child = tests::invocation(4101, &["check"]);
            child.parent = VisibleParent::Invocation {
                id:  before.lead.invocation_id.clone(),
                pid: before.lead.pid,
            };
            child.resource_use.cpu = Measurement::Reading(0.26);
            before.rest.push(child);
        }
        if matches!(change, DisplayRevision::CpuShorterValue) {
            // A sibling at 100% holds the CPU column width.
            before.lead.resource_use.cpu = Measurement::Reading(100.0);
            let mut child = tests::invocation(4101, &["check"]);
            child.parent = VisibleParent::Invocation {
                id:  before.lead.invocation_id.clone(),
                pid: before.lead.pid,
            };
            child.resource_use.cpu = Measurement::Reading(100.0);
            before.rest.push(child);
        }
        let mut after = before.clone();
        revise(change, &mut after);
        let (paints, displayed) = expected(change);
        (
            before,
            after,
            paints,
            displayed,
            if hidden { 3 } else { 80 },
        )
    }

    /// Apply one revision to the group drawn second.
    fn revise(change: DisplayRevision, after: &mut CargoGroup) {
        match change {
            DisplayRevision::IntrinsicCommandWidth | DisplayRevision::RowEndPolicy => {},
            DisplayRevision::Subsecond => {
                after.lead.origin_start = OriginStart::Process {
                    epoch:   1_790_000_000,
                    elapsed: Duration::from_millis(18_900),
                }
            },
            DisplayRevision::Second => {
                after.lead.origin_start = OriginStart::Process {
                    epoch:   1_790_000_000,
                    elapsed: Duration::from_millis(19_100),
                }
            },
            DisplayRevision::CpuBelowBoundary => {
                after.lead.resource_use.cpu = Measurement::Reading(12.4);
            },
            DisplayRevision::CpuAcrossBoundary => {
                after.lead.resource_use.cpu = Measurement::Reading(12.6);
            },
            DisplayRevision::MemoryBelowBoundary => {
                after.lead.resource_use.memory =
                    Measurement::Reading(BYTES_PER_GIBIBYTE * 104 / 100);
            },
            DisplayRevision::MemoryAcrossBoundary => {
                after.lead.resource_use.memory =
                    Measurement::Reading(BYTES_PER_GIBIBYTE * 106 / 100);
            },
            DisplayRevision::HiddenCpuAcrossBoundary => {
                after.rest[0].resource_use.cpu = Measurement::Reading(0.20);
            },
            DisplayRevision::ProgressOwnership => {
                after.lead.capture_membership = CaptureMembership::EnclosingUnconfirmed;
            },
            DisplayRevision::HiddenCpuSameTotal => {
                after.lead.resource_use.cpu = Measurement::Reading(0.20);
                after.rest[0].resource_use.cpu = Measurement::Reading(0.32);
            },
            DisplayRevision::CpuShorterValue => {
                after.lead.resource_use.cpu = Measurement::Reading(9.5);
            },
            DisplayRevision::CpuNarrowerValue => {
                after.lead.resource_use.cpu = Measurement::Reading(9.4);
            },
            DisplayRevision::MemoryWider => {
                after.lead.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE * 10);
            },
            DisplayRevision::CompilerStarts => {
                after.lead.resource_use.compiler = CompilerObservation::Running(Compiler {
                    name:  "sccache",
                    count: 1,
                });
            },
        }
    }

    /// The paints and the displayed change that one revision causes.
    fn expected(change: DisplayRevision) -> (usize, DisplayedValue) {
        // Live CPU and memory values draw over the painting. Only a wider
        // value column or a compiler change repaints it.
        let paints = usize::from(matches!(
            change,
            DisplayRevision::ProgressOwnership
                | DisplayRevision::MemoryWider
                | DisplayRevision::CompilerStarts
        ));
        let displayed = if matches!(
            change,
            DisplayRevision::Second
                | DisplayRevision::CpuAcrossBoundary
                | DisplayRevision::MemoryAcrossBoundary
                | DisplayRevision::HiddenCpuAcrossBoundary
                | DisplayRevision::ProgressOwnership
                | DisplayRevision::CpuShorterValue
                | DisplayRevision::CpuNarrowerValue
                | DisplayRevision::MemoryWider
                | DisplayRevision::CompilerStarts
        ) {
            DisplayedValue::Changes
        } else {
            DisplayedValue::Stays
        };
        (paints, displayed)
    }

    fn assert_display_revision(change: DisplayRevision) {
        let (before, after, expected_paints, displayed, height) = display_groups(change);
        let mut roster = super::super::Roster::default();
        let now = Instant::now();
        roster.observe(vec![before], now);
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let content = TileContent::Group(roster.visible(&[]).groups()[0].id.clone());
        let view = CommandView::long();
        let area = Rect::new(0, 0, 160, height);
        let original = Buffer::empty(area);
        let cells = Cells::from_prepared(
            tables.for_frame(&mut roster, &[], view, 80, FreeDiskSpace::Unavailable),
            &stats,
        );
        let mut first = original.clone();
        cells.draw(&mut first, &content, area, Color::Black);
        roster.observe(vec![after], now);
        let cells = Cells::from_prepared(
            tables.for_frame(&mut roster, &[], view, 80, FreeDiskSpace::Unavailable),
            &stats,
        );
        if matches!(change, DisplayRevision::RowEndPolicy) {
            cells.grids.commands.0.row_end.set(TableRowEnd::Reserved);
            cells
                .grids
                .commands
                .0
                .frame_row_end
                .set(TableRowEnd::Reserved);
        }
        let mut cells = cells;
        if matches!(change, DisplayRevision::IntrinsicCommandWidth) {
            cells.grids.commands.0.intrinsic_widths[render::COMMAND_COLUMN] += 32;
            cells.grids.commands.0.layouts.borrow_mut().clear();
        }
        // Each frame starts from the grid the painting was recorded over.
        let mut actual = original.clone();
        CELL_CONTENT_PAINTS.with(|count| count.set(0));
        cells.draw(&mut actual, &content, area, Color::Black);
        CELL_CONTENT_PAINTS.with(|count| assert_eq!(count.get(), expected_paints, "{change:?}"));
        if matches!(change, DisplayRevision::CpuShorterValue) {
            let full = |buffer: &Buffer| {
                tests::buffer_rows(buffer)
                    .iter()
                    .map(|row| row.matches("100%").count())
                    .sum::<usize>()
            };
            assert_eq!(full(&first), 2, "both rows first show 100%");
            assert_eq!(full(&actual), 1, "only the sibling still shows 100%");
        }
        let mut fresh = original;
        render::frame_draw_contents_and_values(
            &mut fresh,
            &cells.roster,
            &content,
            area,
            Color::Black,
            &cells.grids,
        );
        assert_eq!(
            actual, fresh,
            "{change:?}: retained and fresh paintings agree"
        );
        match displayed {
            DisplayedValue::Changes => {
                assert_ne!(first, fresh, "{change:?}: the displayed value changes");
            },
            DisplayedValue::Stays => {
                assert_eq!(first, fresh, "{change:?}: the displayed value stays equal");
            },
        }
    }

    fn assert_brighten_revision(ground: Color) {
        let now = Instant::now();
        let mut roster = super::super::Roster::default();
        roster.observe(
            vec![tests::cargo_group(tests::invocation(4100, &["build"]))],
            now,
        );
        roster.observe(Vec::new(), now + Duration::from_millis(100));
        roster.advance(now + Duration::from_millis(2600), Duration::from_secs(3));
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let content = TileContent::Group(roster.visible(&[]).groups()[0].id.clone());
        let (canvas, area) = match ground {
            Color::Indexed(16) => (Rect::new(0, 0, 180, 90), Rect::new(3, 5, 160, 80)),
            _ => (Rect::new(0, 0, 160, 80), Rect::new(0, 0, 160, 80)),
        };
        let original = Buffer::empty(canvas);
        let cells = Cells::from_prepared(
            tables.for_frame(
                &mut roster,
                &[],
                CommandView::long(),
                80,
                FreeDiskSpace::Unavailable,
            ),
            &stats,
        );
        let mut first = original.clone();
        cells.draw(&mut first, &content, area, ground);
        assert!(
            cells
                .grids
                .commands
                .0
                .prepared_rows
                .borrow()
                .paintings
                .is_empty(),
            "the retained fade painting needs no second row image"
        );
        let before_ink = roster.visible(&[]).groups()[0].lead.painted_ink();
        roster.advance(now + Duration::from_millis(2625), Duration::from_secs(3));
        assert_ne!(
            before_ink,
            roster.visible(&[]).groups()[0].lead.painted_ink()
        );
        let cells = Cells::from_prepared(
            tables.for_frame(
                &mut roster,
                &[],
                CommandView::long(),
                80,
                FreeDiskSpace::Unavailable,
            ),
            &stats,
        );
        let mut actual = original.clone();
        CELL_CONTENT_PAINTS.with(|count| count.set(0));
        cells.draw(&mut actual, &content, area, ground);
        CELL_CONTENT_PAINTS.with(|count| {
            assert_eq!(
                count.get(),
                0,
                "ink changes retain the painting: {ground:?}"
            );
        });
        let mut fresh = original;
        render::frame_draw_contents_and_values(
            &mut fresh,
            &cells.roster,
            &content,
            area,
            ground,
            &cells.grids,
        );
        assert_eq!(actual, fresh, "retained ink matches fresh ink: {ground:?}");
        if ground == Color::Reset {
            assert_eq!(first, fresh, "equal blended colors keep every cell");
        } else {
            assert_ne!(first, fresh, "different blended colors recolor the cells");
        }
    }

    #[test]
    fn untouched_tail_changes_reuse_fitting_cell_paint() {
        let app = tests::argument_toggle_app();
        let stats = SccacheStats::default();
        let cells = Cells::new(&app.roster, &[], CommandView::long(), &stats);
        let canvas = Rect::new(3, 5, 180, 32);
        for inner in [Rect::new(5, 7, 176, 28), Rect::new(5, 7, 176, 24)] {
            let ground = Color::Black;
            for content in [
                TileContent::Summary,
                TileContent::Group(cells.roster.groups()[0].id.clone()),
            ] {
                let original = Buffer::empty(canvas);
                let mut first = original.clone();
                cells.draw(&mut first, &content, inner, ground);
                let mut actual = original.clone();
                let tail = (inner.right() - 1, inner.bottom() - 1);
                actual[tail].set_symbol("界").set_fg(Color::Green);
                let mut fresh = actual.clone();
                render::frame_draw_contents_and_values(
                    &mut fresh,
                    &cells.roster,
                    &content,
                    inner,
                    ground,
                    &cells.grids,
                );
                assert_eq!(
                    first[tail], original[tail],
                    "the cell leaves its tail alone"
                );
                CELL_CONTENT_PAINTS.with(|count| count.set(0));
                cells.draw(&mut actual, &content, inner, ground);
                assert_eq!(actual, fresh, "tail changes preserve fresh painting");
                CELL_CONTENT_PAINTS.with(|count| {
                    assert_eq!(
                        count.get(),
                        0,
                        "untouched tail rows cannot change this painting"
                    );
                });
            }
        }
    }

    #[test]
    fn live_values_keep_the_row_background() -> Result<(), &'static str> {
        let ground = Color::Indexed(16);
        let mut lead = tests::invocation(4100, &["build"]);
        lead.resource_use.cpu = Measurement::Reading(12.1);
        lead.resource_use.memory = Measurement::Reading(BYTES_PER_GIBIBYTE);
        let before = tests::cargo_group(lead);
        let mut after = before.clone();
        after.lead.resource_use.cpu = Measurement::Reading(30.2);
        let mut roster = super::super::Roster::default();
        let now = Instant::now();
        roster.observe(vec![before], now);
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let content = TileContent::Group(roster.visible(&[]).groups()[0].id.clone());
        let view = CommandView::long();
        let area = Rect::new(3, 5, 160, 80);
        let original = Buffer::empty(Rect::new(0, 0, 180, 90));
        let cells = Cells::from_prepared(
            tables.for_frame(&mut roster, &[], view, 80, FreeDiskSpace::Unavailable),
            &stats,
        );
        let mut first = original.clone();
        cells.draw(&mut first, &content, area, ground);
        roster.observe(vec![after], now);
        let cells = Cells::from_prepared(
            tables.for_frame(&mut roster, &[], view, 80, FreeDiskSpace::Unavailable),
            &stats,
        );
        let mut actual = original;
        CELL_CONTENT_PAINTS.with(|count| count.set(0));
        let work = crate::probe::measure_paint(|| {
            cells.draw(&mut actual, &content, area, ground);
        });
        CELL_CONTENT_PAINTS.with(|count| {
            assert_eq!(count.get(), 0, "a CPU value change keeps the painting");
        });
        assert!(
            work.live_values_cpu_ns > 0,
            "retained live values record CPU time"
        );
        let rows = tests::buffer_rows(&actual);
        let (row, column) = rows
            .iter()
            .enumerate()
            .find_map(|(row, text)| {
                text.find("30%")
                    .map(|byte| (row, text[..byte].chars().count()))
            })
            .ok_or("the new CPU value is drawn")?;
        let row = u16::try_from(row).map_err(|_| "the row fits the buffer")?;
        let column = u16::try_from(column).map_err(|_| "the column fits the buffer")?;
        let row_ground = actual[(area.x, row)].bg;
        for x in area.left()..area.right() {
            assert_eq!(
                actual[(x, row)].bg,
                row_ground,
                "cell {x} takes the background of its row"
            );
        }
        assert!((column..column + 3).all(|x| actual[(x, row)].bg == row_ground));
        Ok(())
    }

    #[test]
    fn summary_memory_changes_keep_the_painting() {
        let mut app = tests::argument_toggle_app();
        let stats = SccacheStats::default();
        let mut tables = PreparedTables::default();
        let content = TileContent::Summary;
        let view = CommandView::long();
        let area = Rect::new(0, 0, 160, 80);
        let original = Buffer::empty(area);
        let mut share = |roster: &mut Roster, bytes: u64| {
            roster.observe_memory_shares(CompileMemoryShares {
                shared_compiler_cache: bytes,
                unattributed:          0,
            });
            Cells::from_prepared(
                tables.for_frame(roster, &[], view, 80, FreeDiskSpace::Unavailable),
                &stats,
            )
        };
        let cells = share(&mut app.roster, BYTES_PER_GIBIBYTE);
        let mut first = original.clone();
        cells.draw(&mut first, &content, area, Color::Black);
        let cells = share(&mut app.roster, BYTES_PER_GIBIBYTE * 3 / 2);
        let mut actual = original.clone();
        CELL_CONTENT_PAINTS.with(|count| count.set(0));
        cells.draw(&mut actual, &content, area, Color::Black);
        CELL_CONTENT_PAINTS.with(|count| {
            assert_eq!(count.get(), 0, "a summary memory change keeps the painting");
        });
        let mut fresh = original;
        render::frame_draw_contents_and_values(
            &mut fresh,
            &cells.roster,
            &content,
            area,
            Color::Black,
            &cells.grids,
        );
        assert_eq!(actual, fresh, "the replayed summary matches a fresh draw");
        assert_ne!(first, fresh, "the summary memory value changes");
    }
}
