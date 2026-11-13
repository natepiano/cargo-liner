use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::ColorMode;
use crate::TileView;
use crate::tiles;
use crate::tiles::SummaryRowAllocation;
use crate::tiles::TileAction;
use crate::tiles::TileCells;
use crate::tiles::TileColumnOrder;
use crate::tiles::TileContent;
use crate::tiles::TileDemand;
use crate::tiles::TileDemands;
use crate::tiles::TileGrid;
use crate::tiles::TileGridContents;
use crate::tiles::TileGrowth;
use crate::tiles::grid::Slot;

struct TreeCells(Vec<(u32, usize)>);

impl TileCells<u32> for TreeCells {
    fn summary_title(&self) -> &'static str { "summary" }

    fn column_order(&self) -> TileColumnOrder<u32> {
        TileColumnOrder::LongestWithShortest(self.0.clone())
    }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       1,
            summary_width: 0,
            groups:        self
                .0
                .iter()
                .map(|&(id, rows)| TileDemand { id, rows })
                .collect(),
        }
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
        if let TileContent::Group(id) = content {
            let rows = self
                .0
                .iter()
                .find(|&&(key, _)| key == *id)
                .map_or(0, |&(_, n)| n);
            for y in inner.y
                ..inner.bottom().min(
                    inner
                        .y
                        .saturating_add(u16::try_from(rows).unwrap_or(u16::MAX)),
                )
            {
                buffer[(inner.x, y)].set_symbol("x");
            }
        }
    }
}

fn ranked_grid(rows: &[usize], columns: u16) -> (TileGrid<u32>, Rect, TileGrowth, TreeCells) {
    let area = Rect::new(0, 0, columns * 60, 72);
    let growth = TileGrowth {
        initial_rows: 3,
        ..TileGrowth::default()
    };
    let cells = TreeCells(
        rows.iter()
            .enumerate()
            .map(|(i, &rows)| (u32::try_from(i).unwrap_or(u32::MAX), rows))
            .collect(),
    );
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(60);
    grid.set_layout(area, growth);
    paint(&mut grid, area, growth, &cells);
    grid.settle_for_test();
    paint(&mut grid, area, growth, &cells);
    grid.settle_for_test();
    (grid, area, growth, cells)
}

fn paint(
    grid: &mut TileGrid<u32>,
    area: Rect,
    growth: TileGrowth,
    cells: &impl TileCells<u32>,
) -> Buffer {
    let mut buffer = Buffer::empty(area);
    tiles::draw_tile_grid(
        &mut buffer,
        grid,
        area,
        growth,
        TileGridContents::Shown,
        cells,
        ColorMode::Colored,
    );
    buffer
}

fn columns(grid: &TileGrid<u32>, area: Rect, growth: TileGrowth) -> Vec<Vec<u32>> {
    let mut placed = grid
        .placements(area, growth)
        .into_iter()
        .filter_map(|p| match p.content {
            TileContent::Group(id) => Some((p.frame.rect().x, p.frame.rect().y, id)),
            _ => None,
        })
        .collect::<Vec<_>>();
    placed.sort_unstable();
    let mut groups: Vec<(u16, Vec<u32>)> = Vec::new();
    for (x, _, id) in placed {
        if let Some((last_x, ids)) = groups.last_mut()
            && *last_x == x
        {
            ids.push(id);
        } else {
            groups.push((x, vec![id]));
        }
    }
    groups.into_iter().map(|(_, ids)| ids).collect()
}

#[test]
fn ranked_columns_pair_the_natedev_long_and_short_trees() {
    let (grid, area, growth, _) = ranked_grid(&[42, 42, 11, 3], 2);
    assert_eq!(columns(&grid, area, growth), vec![vec![0, 3], vec![1, 2]]);
}

#[test]
fn ranked_columns_pair_six_trees_across_three_columns() {
    let (grid, area, growth, _) = ranked_grid(&[42, 35, 28, 11, 7, 3], 3);
    assert_eq!(
        columns(&grid, area, growth),
        vec![vec![0, 5], vec![1, 4], vec![2, 3]]
    );
}

struct VisibleTreeCells {
    full:         TreeCells,
    visible:      TreeCells,
    summary_rows: usize,
}

impl TileCells<u32> for VisibleTreeCells {
    fn summary_title(&self) -> &'static str { "summary" }
    fn column_order(&self) -> TileColumnOrder<u32> { self.full.column_order() }
    fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary: self.summary_rows,
            ..self.visible.demands(widths)
        }
    }
    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, ground: Color) {
        self.visible.draw(buffer, content, inner, ground);
    }
}

#[test]
fn ranked_columns_keep_ties_and_unchanged_ranks_stable() {
    let (mut grid, area, growth, mut cells) = ranked_grid(&[10, 9, 8, 7, 6, 5], 2);
    let before = columns(&grid, area, growth);
    cells.0[0].1 = 11;
    paint(&mut grid, area, growth, &cells);
    grid.settle_for_test();
    assert_eq!(columns(&grid, area, growth), before);
    let (mut grid, area, growth, mut cells) = ranked_grid(&[9, 10, 8, 7], 2);
    let before = columns(&grid, area, growth);
    cells.0[0].1 = 10;
    paint(&mut grid, area, growth, &cells);
    grid.settle_for_test();
    assert_eq!(
        columns(&grid, area, growth),
        before,
        "a tie retains the previous ranking"
    );

    let area = Rect::new(0, 0, 120, 40);
    let mut cells = VisibleTreeCells {
        full:         TreeCells(vec![(0, 42), (1, 42), (2, 11), (3, 3)]),
        visible:      TreeCells(vec![(0, 42), (1, 42), (2, 11), (3, 3)]),
        summary_rows: 25,
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(60);
    for (visible_rows, expected_depth, expected_columns) in [
        (42, 2, vec![vec![0], vec![1, 2, 3]]),
        (41, 2, vec![vec![0], vec![1, 2, 3]]),
        (1, 1, vec![vec![0, 3], vec![1, 2]]),
    ] {
        cells.visible.0[0].1 = visible_rows;
        for _ in 0..2 {
            paint(&mut grid, area, growth, &cells);
            grid.settle_for_test();
        }
        assert_eq!(grid.depth, expected_depth, "visible rows: {visible_rows}");
        assert_eq!(columns(&grid, area, growth), expected_columns);
    }
}

struct SourceCells<'a, C>(&'a C);

impl<C: TileCells<u32>> TileCells<u32> for SourceCells<'_, C> {
    fn summary_title(&self) -> &'static str { "summary" }
    fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        self.0.demands(widths)
    }
    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, ground: Color) {
        self.0.draw(buffer, content, inner, ground);
    }
}

#[test]
fn ranked_columns_draw_more_long_tree_rows_and_keep_the_summary_slot() {
    let (mut ranked, area, growth, cells) = ranked_grid(&[42, 42, 11, 3], 2);
    let mut source = TileGrid::new();
    source.set_view(TileView::Cells);
    source.set_min_tile_width(60);
    source.set_layout(area, growth);
    let mut before = Buffer::empty(area);
    let source_cells = SourceCells(&cells);
    for _ in 0..2 {
        tiles::draw_tile_grid(
            &mut before,
            &mut source,
            area,
            growth,
            TileGridContents::Shown,
            &source_cells,
            ColorMode::Colored,
        );
        source.settle_for_test();
    }
    let after = paint(&mut ranked, area, growth, &cells);
    assert_eq!(
        columns(&source, area, growth),
        vec![vec![0, 1], vec![2, 3]],
        "the default trait method keeps source order"
    );
    for id in [0, 1] {
        let drawn = |grid: &TileGrid<u32>, buffer: &Buffer| {
            grid.placements(area, growth)
                .into_iter()
                .find(|p| p.content == TileContent::Group(id))
                .map_or(0, |p| {
                    let rect = p.frame.inner();
                    rect.positions()
                        .filter(|&p| buffer[p].symbol() == "x")
                        .count()
                })
        };
        assert!(
            drawn(&ranked, &after) > drawn(&source, &before),
            "long tree {id} gains rows"
        );
    }
    let summary = |grid: &TileGrid<u32>| {
        grid.placements(area, growth)
            .into_iter()
            .find(|p| p.content == TileContent::Summary)
            .map(|p| (p.frame.rect().x, p.frame.rect().y))
    };
    assert_eq!(summary(&source), summary(&ranked));
}

struct SpanningCells(TreeCells);

impl TileCells<u32> for SpanningCells {
    fn summary_title(&self) -> &'static str { "summary" }
    fn column_order(&self) -> TileColumnOrder<u32> { self.0.column_order() }
    fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary: 8,
            summary_width: 120,
            ..self.0.demands(widths)
        }
    }
    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, ground: Color) {
        self.0.draw(buffer, content, inner, ground);
    }
}

#[test]
fn ranked_columns_retain_the_widened_summary_shape_and_a_user_empty_cell() {
    let cells = SpanningCells(TreeCells(vec![(0, 42), (1, 35), (2, 20), (3, 8), (4, 3)]));
    let area = Rect::new(0, 0, 120, 72);
    let growth = TileGrowth {
        initial_rows: 3,
        widen_summary: true,
        ..TileGrowth::default()
    };
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(60);
    grid.set_summary_row_allocation(SummaryRowAllocation::BeforeTiles);
    for _ in 0..2 {
        paint(&mut grid, area, growth, &SourceCells(&cells));
        grid.settle_for_test();
    }
    grid.apply(TileAction::Add, growth);
    grid.settle_for_test();
    paint(&mut grid, area, growth, &SourceCells(&cells));
    grid.settle_for_test();
    let summary = |grid: &TileGrid<u32>| {
        grid.placements(area, growth)
            .into_iter()
            .find(|p| p.content == TileContent::Summary)
            .map(|p| p.frame.rect())
    };
    let before_summary = summary(&grid);
    let before_depth = grid.depth;
    let before_empty: Vec<_> = grid
        .slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| matches!(slot, Slot::Empty(_)))
        .map(|(i, s)| (i, s.clone()))
        .collect();
    let before_capacities: Vec<_> = columns(&grid, area, growth).iter().map(Vec::len).collect();
    assert_eq!(before_empty.len(), 1);
    assert!(before_summary.is_some_and(|rect| rect.width == area.width));
    paint(&mut grid, area, growth, &cells);
    grid.settle_for_test();
    paint(&mut grid, area, growth, &cells);
    grid.settle_for_test();
    assert_eq!(summary(&grid), before_summary);
    assert_eq!(grid.depth, before_depth);
    assert_eq!(
        columns(&grid, area, growth)
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        before_capacities
    );
    for (index, empty) in before_empty {
        assert_eq!(grid.slots[index], empty);
    }
}

#[test]
fn ranked_columns_keep_two_left_and_three_right_slots_after_membership_changes() {
    let (mut grid, area, growth, mut cells) = ranked_grid(&[42, 42, 11, 3], 2);
    cells.0[0].1 = 2;
    paint(&mut grid, area, growth, &cells);
    assert!(
        grid.has_pending_motion_for_test(),
        "rank changes use the existing motion"
    );
    grid.settle_for_test();
    assert_eq!(columns(&grid, area, growth), vec![vec![0, 1], vec![2, 3]]);
    cells.0.push((4, 60));
    paint(&mut grid, area, growth, &cells);
    grid.settle_for_test();
    paint(&mut grid, area, growth, &cells);
    grid.settle_for_test();
    assert_eq!(
        columns(&grid, area, growth),
        vec![vec![3, 4], vec![0, 1, 2]],
        "the left column has two detail slots and the right column has three"
    );
    let narrow = Rect::new(0, 0, 60, 72);
    paint(&mut grid, narrow, growth, &cells);
    grid.settle_for_test();
    paint(&mut grid, narrow, growth, &cells);
    grid.settle_for_test();
    assert_eq!(columns(&grid, narrow, growth), vec![vec![0, 1, 2, 3, 4]]);
}
