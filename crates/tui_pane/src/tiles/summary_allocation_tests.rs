//! A named summary priority reserves measured content before busy tiles.

#![expect(clippy::expect_used, reason = "the summary must have a placement")]

use ratatui::layout::Rect;

use super::SummaryDemand;
use super::SummaryRowAllocation;
use super::TileContent;
use super::TileDemand;
use super::TileDemands;
use super::TileGrid;
use super::TileGrowth;
use super::constants::TILE_ROWS_READOUT_HEIGHT;
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
    grid.sync_test_cells(
        &TileDemands {
            summary:       SummaryDemand::new(rows + usize::from(TILE_ROWS_READOUT_HEIGHT), []),
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

fn assert_narrow_summary_width(width: u16) {
    let area = Rect::new(0, 0, width, 50);
    let growth = TileGrowth {
        initial_rows: 8,
        ..TileGrowth::default()
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(28);
    grid.set_summary_row_allocation(SummaryRowAllocation::BeforeTiles);
    grid.set_layout(area, growth);
    grid.sync_test_cells(
        &TileDemands {
            summary:       SummaryDemand::new(18 + usize::from(TILE_ROWS_READOUT_HEIGHT), []),
            summary_width: 30,
            groups:        (1..=20).map(|id| TileDemand { id, rows: 8 }).collect(),
        },
        growth,
    );
    grid.settle_for_test();
    let placements = grid.placements(area, growth);
    let tiles: Vec<_> = placements
        .iter()
        .filter(|tile| matches!(tile.content, TileContent::Group(_)))
        .collect();
    assert!(
        !tiles.is_empty(),
        "width={width}: a core-width tile fits, so cells view must stay active"
    );
    for tile in &tiles {
        assert!(
            tile.frame.clip().width >= 28,
            "width={width}: {}/{}, narrow frame {:?}",
            tiles.len(),
            20,
            tile.frame.clip()
        );
    }
    let summary = placements
        .iter()
        .find(|tile| tile.content == TileContent::Summary)
        .expect("priority summary");
    let content = draw::content_area(summary.frame.inner());
    assert!(
        content.width >= 30 && content.height >= 18,
        "width={width}: fitted accounts need 30x18 content, got {content:?}; tiles={}/20",
        tiles.len()
    );
}

#[test]
fn narrow_summary_width_and_height_at_200_leave_a_core_width_tile_in_cells_view() {
    assert_narrow_summary_width(200);
}

#[test]
fn narrow_summary_width_and_height_at_80_leave_a_core_width_tile_in_cells_view() {
    assert_narrow_summary_width(80);
}

#[test]
fn narrow_summary_width_and_height_at_50_leave_a_core_width_tile_in_cells_view() {
    assert_narrow_summary_width(50);
}

#[test]
fn narrow_summary_width_and_height_at_40_leave_a_core_width_tile_in_cells_view() {
    assert_narrow_summary_width(40);
}

#[test]
fn narrow_summary_width_and_height_at_34_leave_a_core_width_tile_in_cells_view() {
    assert_narrow_summary_width(34);
}

#[test]
fn narrow_resize_replans_existing_cells_instead_of_hiding_every_agent_tile() {
    let growth = TileGrowth {
        initial_rows: 8,
        ..TileGrowth::default()
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(28);
    grid.set_summary_row_allocation(SummaryRowAllocation::BeforeTiles);
    let demands = TileDemands {
        summary:       SummaryDemand::new(18 + usize::from(TILE_ROWS_READOUT_HEIGHT), []),
        summary_width: 30,
        groups:        (1..=20).map(|id| TileDemand { id, rows: 8 }).collect(),
    };
    grid.set_layout(Rect::new(0, 0, 200, 50), growth);
    grid.sync_test_cells(&demands, growth);
    grid.settle_for_test();
    assert!(
        grid.placements(Rect::new(0, 0, 200, 50), growth).len() > 1,
        "wide fixture starts in cells view"
    );
    for width in [80, 50, 40, 34] {
        let area = Rect::new(0, 0, width, 50);
        grid.set_layout(area, growth);
        grid.sync_test_cells(&demands, growth);
        grid.settle_for_test();
        let placements = grid.placements(area, growth);
        assert!(
            placements
                .iter()
                .any(|tile| matches!(tile.content, TileContent::Group(_))),
            "resize to {width} must replan columns, not replace the cells view with only the summary"
        );
        for tile in placements
            .iter()
            .filter(|tile| matches!(tile.content, TileContent::Group(_)))
        {
            assert!(
                tile.frame.clip().width >= 28,
                "whole tile after resize to {width}: {:?}",
                tile.frame.clip()
            );
        }
        let summary = placements
            .iter()
            .find(|tile| tile.content == TileContent::Summary)
            .expect("resized summary");
        let content = draw::content_area(summary.frame.inner());
        assert!(
            content.width >= 30 && content.height >= 18,
            "resize to {width} keeps the measured summary: {content:?}"
        );
    }
}
