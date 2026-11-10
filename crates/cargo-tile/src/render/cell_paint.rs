//! Replay a cell painting with its command visibility proof.

use ratatui::buffer::Cell;

use super::Buffer;
use super::CellMeasurementKey;
use super::Color;
use super::InvocationId;
use super::Rc;
use super::Rect;
use super::Text;
use super::TrackedGroup;
use super::TrackedRow;
use super::VisibleCommandText;
use super::VisibleRoster;
use crate::app::GridView;
use crate::census::invocation_cpu_accounting::CompileMemoryShares;
use crate::probe;
use crate::roster::ParentFamily;

/// Rows read or written by the painter, including continuation-marker searches.
#[derive(Default)]
pub(super) struct CellPaintReach {
    pub(super) rows:     u16,
    pub(super) commands: Vec<PlacedCommandText>,
    pub(super) header:   CellColumnHeader,
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

/// Frozen row inputs prevent a new roster revision from changing a retained painting.
struct PaintedRowSource {
    snapshot:      Rc<super::InvocationSnapshot>,
    ink:           super::RowInk,
    painted_ink:   super::RowInk,
    ended:         bool,
    family:        super::FamilyHead,
    parent_family: ParentFamily,
}

impl PaintedRowSource {
    fn read(row: &TrackedRow) -> Self {
        Self {
            snapshot:      row.snapshot(),
            ink:           row.ink(),
            painted_ink:   row.painted_ink(),
            ended:         row.is_ended(),
            family:        row.family(),
            parent_family: row.parent_family(),
        }
    }

    fn matches(&self, row: &TrackedRow) -> bool {
        self.ink == row.ink()
            && self.painted_ink == row.painted_ink()
            && self.ended == row.is_ended()
            && self.family == row.family()
            && self.parent_family == row.parent_family()
            && (Rc::ptr_eq(&self.snapshot, &row.snapshot()) || self.snapshot.process == row.process)
    }
}

/// Each cell retains the source details that its painter reads.
#[derive(PartialEq)]
enum CellSourceDetails {
    Empty,
    Group(super::AncestryWalk<super::Ancestor>),
    Summary(CompileMemoryShares),
}

/// A painting retains its row inputs and the frame's column decisions.
pub(super) struct CellPaintSource {
    details:   CellSourceDetails,
    rows:      Vec<PaintedRowSource>,
    grid:      super::FrameTableColumnGrid,
    resources: Vec<super::RowResourceReading>,
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
            super::TileContent::Summary => CellSourceDetails::Summary(roster.memory_shares()),
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
            rows:      rows.iter().map(|row| PaintedRowSource::read(row)).collect(),
            grid:      grid.clone(),
            resources: super::row_resource_readings(&rows, &grid.contributions),
        }
    }

    pub(super) fn matches(
        &self,
        roster: &VisibleRoster,
        content: &super::TileContent,
        grids: &super::FrameColumnGrids,
    ) -> bool {
        let grid = Self::grid(grids, content);
        if !self.layout_matches(grid) || self.details != Self::details(roster, content) {
            return false;
        }
        let rows = Self::rows(roster, content);
        self.rows.len() == rows.len()
            && self
                .rows
                .iter()
                .zip(&rows)
                .all(|(before, row)| before.matches(row))
            && self.resources == super::row_resource_readings(&rows, &grid.contributions)
    }

    fn layout_matches(&self, grid: &super::FrameTableColumnGrid) -> bool {
        let before = &self.grid;
        std::sync::Arc::ptr_eq(&before.theme, &grid.theme)
            && before.slots == grid.slots
            && before.separation == grid.separation
            && before.parent_content == grid.parent_content
            && before.parent_links.hidden == grid.parent_links.hidden
            && before.row_progress == grid.row_progress
            && before.intrinsic_widths == grid.intrinsic_widths
            && before.value_widths == grid.value_widths
            && before.command_title_width == grid.command_title_width
            && before.longest_command_width == grid.longest_command_width
            && before.longest_subcommand_width == grid.longest_subcommand_width
            && before.continuation_clearance == grid.continuation_clearance
            && before.table_budgets == grid.table_budgets
            && before.empty_tail_columns == grid.empty_tail_columns
            && before.state_column == grid.state_column
            && before.kind == grid.kind
            && before.view == grid.view
    }
}

/// An exact destination snapshot guards replay of only the cells the painter changed.
pub(super) struct PaintedCellContents {
    pub(super) header:   CellColumnHeader,
    pub(super) inputs:   CellPaintInputs,
    pub(super) source:   CellPaintSource,
    pub(super) before:   CellPaintSurface,
    pub(super) changes:  CellPaintPatch,
    pub(super) commands: Vec<PlacedCommandText>,
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
    pub(super) fn between(before: &CellPaintSurface, buffer: &Buffer) -> Self {
        let area = before.area();
        let width = usize::from(area.width);
        let mut changes = Vec::with_capacity(width * usize::from(area.height));
        if width > 0 {
            for (row, y) in (area.top()..area.bottom()).enumerate() {
                let start = buffer.index_of(area.x, y);
                for (offset, cell) in buffer.content[start..start + width].iter().enumerate() {
                    if before.cell(row * width + offset) != cell {
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
    use std::time::Instant;

    use crate::census::invocation_cpu_accounting::CompileMemoryShares;
    use crate::render;
    use crate::render::Buffer;
    use crate::render::Cells;
    use crate::render::Color;
    use crate::render::CommandView;
    use crate::render::FreeDiskSpace;
    use crate::render::Rect;
    use crate::render::SccacheStats;
    use crate::render::TileCells;
    use crate::render::TileContent;
    use crate::render::frame_preparation::PreparedTables;
    use crate::render::tests;
    use crate::render::tests::CELL_CONTENT_PAINTS;

    #[test]
    fn roster_revisions_keep_only_unchanged_cell_paintings() {
        for (summary, changed_command) in [(false, false), (true, false), (false, true)] {
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
            cells.draw(&mut first, &content, area, Color::Black);
            if changed_command {
                let process = tests::invocation(9999, &["test", "--all-targets"]);
                app.roster
                    .observe(vec![tests::cargo_group(process)], Instant::now());
            } else {
                app.roster.observe_memory_shares(CompileMemoryShares {
                    shared_compiler_cache: super::super::BYTES_PER_GIBIBYTE,
                    unattributed:          0,
                });
            }
            let prepared =
                tables.for_frame(&mut app.roster, &[], view, 80, FreeDiskSpace::Unavailable);
            let cells = Cells::from_prepared(prepared, &stats);
            let mut actual = original.clone();
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
            render::frame_draw_contents(
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
                render::frame_draw_contents(
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
}
