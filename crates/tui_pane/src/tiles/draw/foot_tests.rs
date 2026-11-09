use super::*;
use crate::ColorMode;
use crate::tiles::draw::tests::*;
use crate::tiles::draw::*;

#[test]
fn summary_foot_is_written_only_on_summary_readout_row() {
    let area = Rect::new(0, 0, 80, 16);
    let cells = StubCells::new([7]).with_summary_foot(SummaryFoot::Text(Line::raw(FOOT_TEXT)));
    let (grid, buffer) = settled_grid(area, &cells);
    let placements = grid.placements(area, TileGrowth::default());
    assert_eq!(placements.len(), 2);
    let summary = &placements[0];
    let group = &placements[1];
    assert_eq!(summary.content, TileContent::Summary);
    assert_eq!(group.content, TileContent::Group(7));
    let summary = summary.frame.inner();
    let group = group.frame.inner();

    let foot_x = summary.x + TILE_FOOT_LEFT_INSET;
    let foot_y = summary.bottom() - TILE_ROWS_READOUT_HEIGHT;
    let foot_width = u16::try_from(FOOT_TEXT.len()).unwrap_or(u16::MAX);
    let rendered: String = (foot_x..foot_x + foot_width)
        .map(|x| buffer[(x, foot_y)].symbol())
        .collect();
    assert_eq!(rendered, FOOT_TEXT);
    assert!(
        area_lines(&buffer, group)
            .iter()
            .all(|line| !line.contains(FOOT_TEXT)),
        "the group cell has no summary foot"
    );
}

#[test]
fn summary_foot_leaves_wide_readout_against_right_inset() {
    let buffer_area = Rect::new(0, 0, 100, 8);
    let inner = Rect::new(3, 2, 80, 4);
    let mut buffer = Buffer::empty(buffer_area);
    draw_cell(
        &mut buffer,
        inner,
        1,
        inner.width,
        &SummaryFoot::Text(Line::raw(FOOT_TEXT)),
        CellReadoutInk::Unfaded,
        |_, _| {},
    );

    let y = inner.bottom() - TILE_ROWS_READOUT_HEIGHT;
    assert_eq!(
        buffer[(inner.right() - TILE_ROWS_RIGHT_INSET - 1, y)].symbol(),
        "0"
    );
    assert!(buffer_line(&buffer, y).contains(TILE_ROWS_CONTENT_LABEL));
}

#[test]
fn summary_foot_shares_with_the_short_readout_candidate() {
    let inner = Rect::new(0, 0, 30, 3);
    let mut buffer = Buffer::empty(inner);
    draw_cell(
        &mut buffer,
        inner,
        1,
        inner.width,
        &SummaryFoot::Text(Line::raw(FOOT_TEXT)),
        CellReadoutInk::Unfaded,
        |_, _| {},
    );

    let row = buffer_line(&buffer, inner.bottom() - TILE_ROWS_READOUT_HEIGHT);
    assert!(row.contains(FOOT_TEXT));
    assert!(row.contains(TILE_ROWS_CONTENT_LABEL));
    assert!(!row.contains(TILE_ROWS_CELL_LABEL));
}

#[test]
fn summary_foot_is_drawn_whole_or_absent_at_every_width() {
    let foot_width = u16::try_from(Line::raw(FOOT_TEXT).width()).unwrap_or(u16::MAX);
    let lines_fit_side_by_side = |width| {
        let inner = Rect::new(0, 0, width, 3);
        let readout_width = rows_readout_lines(inner, 1, width)
            .last()
            .map_or(u16::MAX, |line| {
                u16::try_from(line.width()).unwrap_or(u16::MAX)
            });
        foot_width
            .saturating_add(TILE_FOOT_LEFT_INSET)
            .saturating_add(TILE_FOOT_GAP)
            .saturating_add(readout_width)
            .saturating_add(TILE_ROWS_RIGHT_INSET)
            <= width
    };
    let first_side_by_side_width = (0..=u16::MAX)
        .find(|&width| lines_fit_side_by_side(width))
        .unwrap_or(u16::MAX);
    assert!(
        lines_fit_side_by_side(first_side_by_side_width),
        "the summary foot and readout fit side by side at some width"
    );

    for width in 0..=first_side_by_side_width {
        let inner = Rect::new(0, 0, width, 3);
        let y = inner.bottom() - TILE_ROWS_READOUT_HEIGHT;
        let mut foot_buffer = Buffer::empty(inner);
        draw_cell(
            &mut foot_buffer,
            inner,
            1,
            width,
            &SummaryFoot::Text(Line::raw(FOOT_TEXT)),
            CellReadoutInk::Unfaded,
            |_, _| {},
        );

        if foot_width <= width.saturating_sub(TILE_FOOT_LEFT_INSET)
            && rows_readout_height(width) > 0
        {
            let rendered: String = (TILE_FOOT_LEFT_INSET..TILE_FOOT_LEFT_INSET + foot_width)
                .map(|x| foot_buffer[(x, y)].symbol())
                .collect();
            assert_eq!(rendered, FOOT_TEXT, "summary inner width {width}");
        } else {
            let mut empty_buffer = Buffer::empty(inner);
            draw_cell(
                &mut empty_buffer,
                inner,
                1,
                width,
                &SummaryFoot::Empty,
                CellReadoutInk::Unfaded,
                |_, _| {},
            );
            for x in 0..width {
                assert_eq!(
                    foot_buffer[(x, y)],
                    empty_buffer[(x, y)],
                    "summary inner width {width}, column {x}"
                );
            }
        }
    }

    let inner = Rect::new(0, 0, first_side_by_side_width, 3);
    let mut buffer = Buffer::empty(inner);
    draw_cell(
        &mut buffer,
        inner,
        1,
        inner.width,
        &SummaryFoot::Text(Line::raw(FOOT_TEXT)),
        CellReadoutInk::Unfaded,
        |_, _| {},
    );
    let row = buffer_line(&buffer, inner.bottom() - TILE_ROWS_READOUT_HEIGHT);
    assert!(row.contains(FOOT_TEXT));
    assert!(row.contains(TILE_ROWS_CONTENT_LABEL));
}

#[test]
fn hidden_grid_draws_no_summary_foot() {
    let area = Rect::new(0, 0, 80, 8);
    let cells = StubCells::new([]).with_summary_foot(SummaryFoot::Text(Line::raw(FOOT_TEXT)));
    let mut grid = TileGrid::new();
    let mut buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        TileGrowth::default(),
        TileGridContents::Hidden,
        &cells,
        ColorMode::Colored,
    );

    assert!(
        area_lines(&buffer, area)
            .iter()
            .all(|line| !line.contains(FOOT_TEXT))
    );
}
