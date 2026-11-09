use super::Debug;
use super::GridDisplay;
use super::Rect;
use super::TileCells;
use super::TileContent;
use super::TileDemands;
use super::TileDrawing;
use super::TileGrid;
use super::TileGridContents;
use super::TileGrowth;
use super::frame_inner;
use super::has_visible_body_row;
use super::rows_readout_height;
/// The grid keeps its next layout ready while an expanded cell covers it.
pub(in crate::tiles) fn prepare_covered_grid<Id: Clone + Eq + Debug>(
    grid: &mut TileGrid<Id>,
    area: Rect,
    growth: TileGrowth,
    cells: &impl TileCells<Id>,
) {
    let prepared = prepare_grid(grid, area, growth, TileGridContents::Shown, cells);
    let frames = prepared
        .drawing
        .pieces
        .iter()
        .filter(|piece| {
            piece.placement.content == TileContent::Summary
                && has_visible_body_row(piece.placement.frame)
        })
        .map(|piece| piece.placement.frame.clip())
        .collect();
    grid.record_painted_body(prepared.drawing, TileGridContents::Shown, frames);
}

pub(super) struct PreparedGrid<Id> {
    pub(super) drawing: TileDrawing<Id>,
    pub(super) widths:  Vec<(TileContent<Id>, u16)>,
    pub(super) demands: TileDemands<Id>,
}

pub(super) fn prepare_grid<Id: Clone + Eq + Debug>(
    grid: &mut TileGrid<Id>,
    area: Rect,
    growth: TileGrowth,
    contents: TileGridContents,
    cells: &impl TileCells<Id>,
) -> PreparedGrid<Id> {
    grid.set_draw_layout(area, growth);
    cells.prepare_moving_frame(
        || grid.preparation_allocations(area, growth),
        || grid.motion_preparation_allocations(area, growth),
    );
    let selected = grid.sync_measured_demands(growth, |widths| {
        let mut demands = cells.demands(widths);
        add_readout_rows(&mut demands, widths);
        demands
    });
    let cells_widths = selected.widths;
    let cells_demands = selected.demands;
    let alone_widths = measurement_widths(grid, area, growth, GridDisplay::SummaryAlone);
    let mut alone_demands = cells.demands(&alone_widths);
    add_readout_rows(&mut alone_demands, &alone_widths);
    let progress = grid.progress();
    grid.measure_row_steps(
        area,
        growth,
        progress,
        |content, area| cells.rows_drawn(content, area),
        |content, area| cells.rows_kept(content, area),
    );
    let drawing = grid.visible_drawing(contents, progress);
    let (widths, demands) = match grid.display() {
        GridDisplay::Cells => (cells_widths, cells_demands),
        GridDisplay::SummaryAlone => (alone_widths, alone_demands),
    };
    PreparedGrid {
        drawing,
        widths,
        demands,
    }
}

/// Widths used to measure the display that will be drawn this frame.
pub(super) fn measurement_widths<Id: Clone + Eq + Debug>(
    grid: &TileGrid<Id>,
    area: Rect,
    growth: TileGrowth,
    display: GridDisplay,
) -> Vec<(TileContent<Id>, u16)> {
    match display {
        GridDisplay::Cells => grid.content_widths(area, growth),
        GridDisplay::SummaryAlone => {
            vec![(TileContent::Summary, frame_inner(area).width)]
        },
    }
}

/// Row demands and the widths used to measure them for one tile frame.
pub(in crate::tiles) struct TileMeasurements<'a, Id> {
    pub(in crate::tiles) demands: &'a TileDemands<Id>,
    pub(in crate::tiles) widths:  &'a [(TileContent<Id>, u16)],
}

impl<Id: Clone + Eq + Debug> TileMeasurements<'_, Id> {
    pub(super) fn rows(&self, content: &TileContent<Id>) -> usize {
        match content {
            TileContent::Summary => self.demands.summary,
            TileContent::Group(id) => self.demands.rows_for(id),
            TileContent::Empty(_) => 0,
        }
    }

    pub(super) fn width(&self, content: &TileContent<Id>) -> u16 {
        measured_width(self.widths, content)
    }
}

/// Add the readout's row to what every cell asks for.
///
/// Each cell gets it at the width its contents were measured at, and
/// only where that width can display the readout -- the same width
/// [`draw_tile_grid`](crate::draw_tile_grid) goes on to write in the readout.
pub(in crate::tiles) fn add_readout_rows<Id: Clone + Eq>(
    demands: &mut TileDemands<Id>,
    widths: &[(TileContent<Id>, u16)],
) {
    let summary_width = measured_width(widths, &TileContent::Summary);
    demands.summary = demands
        .summary
        .saturating_add(usize::from(rows_readout_height(summary_width)));
    for demand in &mut demands.groups {
        let width = measured_width(widths, &TileContent::Group(demand.id.clone()));
        demand.rows = demand
            .rows
            .saturating_add(usize::from(rows_readout_height(width)));
    }
}

/// The width `content` was measured at: its own entry in `widths`, or
/// the narrowest cell on screen for a group that has no cell yet, which
/// is the width it will have once the grid has opened one for it.
fn measured_width<Id: Eq>(widths: &[(TileContent<Id>, u16)], content: &TileContent<Id>) -> u16 {
    widths
        .iter()
        .find(|(measured, _)| measured == content)
        .map_or_else(
            || {
                widths
                    .iter()
                    .map(|&(_, width)| width)
                    .min()
                    .unwrap_or_default()
            },
            |&(_, width)| width,
        )
}
