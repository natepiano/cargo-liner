//! A named summary priority reserves measured content before busy tiles.

#![expect(clippy::expect_used, reason = "the summary must have a placement")]

use ratatui::layout::Rect;

use super::SummaryRowAllocation;
use super::TileContent;
use super::TileDemand;
use super::TileDemands;
use super::TileGrid;
use super::TileGrowth;
use super::draw;
use crate::TileView;

fn allocated_summary(policy: SummaryRowAllocation, width: u16, rows: usize) -> u16 {
    let area = Rect::new(0, 0, width, 50);
    let growth = TileGrowth {
        initial_rows: 8,
        ..TileGrowth::default()
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(width);
    grid.set_summary_row_allocation(policy);
    grid.set_layout(area, growth);
    grid.sync(
        &TileDemands {
            summary:       rows + usize::from(super::constants::TILE_ROWS_READOUT_HEIGHT),
            summary_width: width.saturating_sub(2),
            groups:        (1..=5).map(|id| TileDemand { id, rows: 100 }).collect(),
        },
        growth,
    );
    grid.settle_for_test();
    let placements = grid.placements(area, growth);
    let summary = placements
        .iter()
        .find(|tile| tile.content == TileContent::Summary)
        .expect("summary placement");
    draw::content_area(summary.frame.inner()).height
}

#[test]
fn accounts_summary_before_tiles_reserves_full_two_line_and_stacked_demands() {
    for (width, rows) in [(50, 12), (40, 18), (34, 26), (26, 40)] {
        let allocated = allocated_summary(SummaryRowAllocation::BeforeTiles, width, rows);
        assert!(
            usize::from(allocated) >= rows,
            "priority summary at width {width}: {allocated} content rows for {rows} lines"
        );
    }
}

#[test]
fn accounts_shared_allocation_still_shares_busy_column_rows() {
    let shared = allocated_summary(SummaryRowAllocation::Shared, 34, 26);
    let first = allocated_summary(SummaryRowAllocation::BeforeTiles, 34, 26);
    assert!(
        shared < 26,
        "fixture must exceed the summary's shared allocation: {shared}"
    );
    assert!(
        first > shared,
        "the named choice changes row ownership: shared={shared}, first={first}"
    );
}
