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

/// Rows read or written by the painter, including continuation-marker searches.
#[derive(Default)]
pub(super) struct CellPaintReach {
    pub(super) rows:     u16,
    pub(super) commands: Vec<PlacedCommandText>,
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
    pub(super) inner:       Rect,
    pub(super) buffer_area: Rect,
    pub(super) ground:      Color,
}

/// An exact destination snapshot guards replay of only the cells the painter changed.
pub(super) struct PaintedCellContents {
    pub(super) inputs:   CellPaintInputs,
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
        (area.top()..area.bottom()).all(|y| {
            let start = buffer.index_of(area.x, y);
            buffer.content[start..start + width]
                .iter()
                .all(|other| other == cell)
        })
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
                    row == &buffer.content[start..start + width]
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
        let mut changes = Vec::new();
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
        Self(changes)
    }

    pub(super) fn apply(&self, buffer: &mut Buffer) {
        for change in &self.0 {
            buffer.content[change.index].clone_from(&change.cell);
        }
    }
}

#[cfg(test)]
mod cell_paint_tests {
    use crate::render;
    use crate::render::Buffer;
    use crate::render::Cells;
    use crate::render::Color;
    use crate::render::CommandView;
    use crate::render::Rect;
    use crate::render::SccacheStats;
    use crate::render::TileCells;
    use crate::render::TileContent;
    use crate::render::tests;
    use crate::render::tests::CELL_CONTENT_PAINTS;

    #[test]
    fn untouched_tail_changes_reuse_fitting_cell_paint() {
        let app = tests::argument_toggle_app();
        let stats = SccacheStats::default();
        let cells = Cells::new(&app.roster, &[], CommandView::long(), &stats);
        let canvas = Rect::new(3, 5, 180, 32);
        let inner = Rect::new(5, 7, 176, 28);
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
