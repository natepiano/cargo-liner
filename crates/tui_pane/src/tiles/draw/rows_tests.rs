use ratatui::widgets::Widget;

use super::*;
use crate::ColorMode;
use crate::TileView;
use crate::pane_background;
use crate::tiles::constants::MIN_TILE_HEIGHT;
use crate::tiles::draw::tests::*;
use crate::tiles::draw::*;

#[test]
#[expect(clippy::expect_used, reason = "test row fixtures must be valid")]
fn selected_rows_readout_checks_bounds_and_preserves_count_text() {
    let row = NonZeroUsize::new(3).expect("positive row");
    assert_eq!(RowsReadout::selected(row, 2), Err(RowExceedsTotal));
    assert_eq!(RowsReadout::selected(row, 0), Err(RowExceedsTotal));
    let selected = RowsReadout::selected(row, 12).expect("row within total");
    let area = Rect::new(0, 0, 80, 10);
    let lines = extent_readout_lines(
        area,
        CellRowsReadout::new(selected, 12),
        80,
        CellContentExtent::Measured,
    );
    assert_eq!(lines[0].to_string(), "content rows: 3 of 12  r/c: 10/80");
    assert_eq!(lines[1].to_string(), "content rows: 3 of 12");
    assert_eq!(
        rows_readout_lines(area, 12, 80)[0].to_string(),
        "content rows: 12  r/c: 10/80"
    );
}

#[test]
fn transparent_tile_readout_fades_toward_theme_without_painting_ground()
-> Result<(), Box<dyn Error>> {
    let previous_tint = crate::focused_pane_tint_enabled();
    let previous_transparency = crate::transparent_background();
    crate::set_focused_pane_tint(true);
    crate::set_transparent_background(true);
    let area = Rect::new(0, 0, 60, 10);
    let mut cells = StubCells::demanding(1, []);
    let progress = CellFadeProgress::new(128);
    cells.readout_fades = vec![(TileContent::Summary, CellReadoutFade::Fading(progress))];
    let widths = [(TileContent::Summary, area.width - 2)];
    let mut demands = cells.demands(&widths);
    add_readout_rows(&mut demands, &widths);
    for focused in [true, false] {
        let drawing = TileDrawing {
            pieces:       vec![tile_piece(
                TileContent::Summary,
                PaneFrame::new(area).with_focus(focused),
                PieceName::Shown,
            )],
            column_bands: vec![area],
        };
        let mut buffer = Buffer::empty(area);
        draw_placements(
            &mut buffer,
            area,
            &drawing,
            &TileMeasurements {
                demands: &demands,
                widths:  &widths,
            },
            TileGridContents::Shown,
            &cells,
            ColorMode::Colored,
        );
        let inner = frame_inner(area);
        let foot_y = inner.bottom() - 1;
        let label_x = (inner.left()..inner.right())
            .find(|&x| buffer[(x, foot_y)].symbol() == "c")
            .ok_or("the readout label must be present")?;
        let ink = buffer[(label_x, foot_y)].fg;
        assert_eq!(
            ink,
            blend_color(label_color(), pane_background(focused), progress.amount()),
            "a transparent tile still fades toward its theme ground"
        );
        assert_ne!(
            ink,
            label_color(),
            "the readout must not retain its raw ink"
        );
        for y in inner.top()..inner.bottom() {
            for x in inner.left()..inner.right() {
                assert_eq!(buffer[(x, y)].bg, Color::Reset, "interior at ({x}, {y})");
            }
        }
    }
    crate::set_focused_pane_tint(previous_tint);
    crate::set_transparent_background(previous_transparency);
    Ok(())
}

#[test]
fn focused_tile_tint_covers_short_rows_empty_rows_and_readout() {
    struct ShortContent {
        focused: bool,
    }

    impl TileCells<u32> for ShortContent {
        fn summary_title(&self) -> &'static str { "summary" }

        fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
            TileDemands {
                summary: 1,
                ..TileDemands::default()
            }
        }

        fn draw(&self, buffer: &mut Buffer, _: &TileContent<u32>, inner: Rect, _: Color) {
            PaneChrome::paint_tile_ground(buffer, inner, self.focused);
            Paragraph::new("short").render(inner, buffer);
        }
    }

    let previous_tint = crate::focused_pane_tint_enabled();
    let previous_transparency = crate::transparent_background();
    crate::set_focused_pane_tint(true);
    let area = Rect::new(0, 0, 60, 10);
    for transparent in [true, false] {
        crate::set_transparent_background(transparent);
        for focused in [true, false] {
            let drawing = TileDrawing {
                pieces:       vec![tile_piece(
                    TileContent::Summary,
                    PaneFrame::new(area).with_focus(focused),
                    PieceName::Shown,
                )],
                column_bands: vec![area],
            };
            let cells = ShortContent { focused };
            let widths = [(TileContent::Summary, area.width - 2)];
            let mut demands = cells.demands(&widths);
            add_readout_rows(&mut demands, &widths);
            let mut buffer = Buffer::empty(area);
            draw_placements(
                &mut buffer,
                area,
                &drawing,
                &TileMeasurements {
                    demands: &demands,
                    widths:  &widths,
                },
                TileGridContents::Shown,
                &cells,
                ColorMode::Colored,
            );
            let expected = if transparent {
                Color::Reset
            } else if focused {
                pane_background(true)
            } else {
                screen_ground().unwrap_or(Color::Reset)
            };
            let inner = frame_inner(area);
            for y in inner.top()..inner.bottom() {
                for x in inner.left()..inner.right() {
                    assert_eq!(
                        buffer[(x, y)].bg,
                        expected,
                        "interior at ({x}, {y}); focused={focused}, transparent={transparent}"
                    );
                }
            }
            assert_eq!(buffer[(1, 1)].symbol(), "s");
            assert_eq!(buffer[(inner.right() - 1, 1)].symbol(), " ");
            assert_eq!(buffer[(1, inner.bottom() - 2)].symbol(), " ");
            assert!(
                area_lines(&buffer, area)
                    .join("\n")
                    .contains("content rows: 1")
            );
        }
    }
    crate::set_focused_pane_tint(previous_tint);
    crate::set_transparent_background(previous_transparency);
}

#[test]
#[expect(clippy::expect_used, reason = "the test readout must fit its buffer")]
fn selected_rows_readout_uses_measured_content_for_fit_ink() {
    let area = Rect::new(0, 0, 80, 8);
    let mut buffer = Buffer::empty(area);
    let mut grid = TileGrid::new();
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        TileGrowth::default(),
        TileGridContents::Shown,
        &WrappedSelectedRows,
        ColorMode::Colored,
    );
    let y = area.bottom() - 2;
    let line: String = (0..area.width).map(|x| buffer[(x, y)].symbol()).collect();
    let start = line.find("content rows: 1 of 1").expect("selected readout");
    let value = start + "content rows: ".len();
    let x = u16::try_from(line[..value].chars().count()).expect("value column");
    assert_eq!(
        buffer[(x, y)].fg,
        error_color(),
        "12 content rows exceed the five-row body"
    );
}

#[test]
fn an_erased_inactive_readout_that_fits_does_not_cover_its_row() {
    let previous_transparency = crate::transparent_background();
    crate::set_transparent_background(false);
    let area = Rect::new(0, 0, 60, 10);
    let drawing = TileDrawing {
        pieces:       vec![tile_piece(
            TileContent::Group(7),
            PaneFrame::new(area),
            PieceName::Shown,
        )],
        column_bands: vec![area],
    };
    let mut buffer = draw_motion_fixture(area, &drawing);
    let inner = frame_inner(area);
    let y = inner.bottom() - TILE_ROWS_READOUT_HEIGHT;
    let background = screen_ground().unwrap_or(Color::Reset);
    assert!(buffer_line(&buffer, y).contains("content rows:"));
    assert!(row_has_coverage(
        &buffer,
        &drawing,
        background,
        inner.left()..inner.right(),
        y
    ));
    for x in inner.left()..inner.right() {
        buffer[(x, y)].set_symbol(" ").set_bg(background);
    }
    assert!(
        !row_has_coverage(
            &buffer,
            &drawing,
            background,
            inner.left()..inner.right(),
            y
        ),
        "an erased readout that fits must leave a missing row"
    );
    assert!(bare_band_row_failure(&buffer, &drawing, "erased readout", 0).is_some());
    crate::set_transparent_background(previous_transparency);
}

#[test]
fn a_fading_cells_readout_fades_with_it() {
    let previous_tint = crate::focused_pane_tint_enabled();
    let previous_transparency = crate::transparent_background();
    crate::set_focused_pane_tint(true);
    let area = Rect::new(0, 0, 200, 16);
    for transparent in [false, true] {
        crate::set_transparent_background(transparent);
        let mut cells = StubCells::demanding(1, [(7, 100)]).with_summary_foot(SummaryFoot::Text(
            Line::from(vec![
                Span::raw(FOOT_TEXT),
                Span::styled("!", Style::default().fg(success_color())),
            ])
            .style(Style::default().fg(label_color())),
        ));
        let (_, unfaded) = settled_grid_in_view(area, &cells, TileView::Cells, MIN_TILE_HEIGHT);
        for amount in [0, 64, 128, u8::MAX] {
            cells.readout_fades = vec![
                (
                    TileContent::Summary,
                    CellReadoutFade::Fading(CellFadeProgress::new(amount)),
                ),
                (
                    TileContent::Group(7),
                    CellReadoutFade::Fading(CellFadeProgress::new(u8::MAX - amount)),
                ),
            ];
            let (grid, faded) =
                settled_grid_in_view(area, &cells, TileView::Cells, MIN_TILE_HEIGHT);
            let placements = grid.placements(area, TileGrowth::default());
            assert_eq!(placements.len(), 2);
            for placement in placements {
                let inner = placement.frame.inner();
                let foot_y = inner.bottom() - TILE_ROWS_READOUT_HEIGHT;
                let progress =
                    CellFadeProgress::new(if placement.content == TileContent::Summary {
                        amount
                    } else {
                        u8::MAX - amount
                    });
                assert_eq!(
                    cells.readout_fade(&placement.content),
                    CellReadoutFade::Fading(progress)
                );
                let ground = if transparent || placement.frame.is_focused() {
                    pane_background(placement.frame.is_focused())
                } else {
                    screen_ground().unwrap_or_else(|| pane_background(false))
                };
                let mut written = 0;
                for x in inner.left()..inner.right() {
                    let before = &unfaded[(x, foot_y)];
                    let after = &faded[(x, foot_y)];
                    assert_eq!(after.symbol(), before.symbol());
                    if before.symbol() != " " {
                        written += 1;
                        let ink = if before.fg == Color::Reset {
                            text_default()
                        } else {
                            before.fg
                        };
                        assert_eq!(
                            after.fg,
                            blend_color(ink, ground, progress.amount()),
                            "{:?} at {x},{foot_y}; transparent={transparent}",
                            placement.content
                        );
                    }
                }
                assert!(written > 0);
            }
        }
    }
    crate::set_focused_pane_tint(previous_tint);
    crate::set_transparent_background(previous_transparency);
}

#[test]
fn draw_tile_cell_draws_only_the_readout_on_its_foot_row() {
    let inner = Rect::new(0, 0, 50, 4);
    let mut buffer = Buffer::empty(inner);
    draw_tile_cell(
        &mut buffer,
        &TileContent::<u32>::Summary,
        inner,
        1,
        inner.width,
        |buffer, area| Paragraph::new("body").render(area, buffer),
    );

    let readout = "content rows: 1  r/c: 4/50";
    let width = usize::from(inner.width - TILE_ROWS_RIGHT_INSET);
    assert_eq!(buffer_line(&buffer, 0), "body");
    assert_eq!(buffer_line(&buffer, 1), "");
    assert_eq!(buffer_line(&buffer, 2), "");
    assert_eq!(buffer_line(&buffer, 3), format!("{readout:>width$}"));
}

#[test]
fn the_rows_readout_is_a_whole_candidate_at_every_width() {
    let measured_at = 17;
    for width in 0..=60 {
        let inner = Rect::new(0, 0, width, 4);
        let candidates = rows_readout_lines(inner, 22, measured_at)
            .iter()
            .map(line_text)
            .collect::<Vec<_>>();
        let mut buffer = Buffer::empty(inner);
        draw_rows_readout(
            &mut buffer,
            inner,
            CellRowsReadout::count(22),
            measured_at,
            CellContentExtent::Measured,
            CellReadoutInk::Unfaded,
        );
        let row = buffer_line(&buffer, inner.bottom() - 1);
        let shown = row.trim();
        assert!(
            shown.is_empty() || candidates.iter().any(|candidate| candidate == shown),
            "width {width} drew {shown:?}; candidates={candidates:?}"
        );
    }
}

#[test]
fn a_cell_with_one_body_row_draws_its_contents_and_no_readout() {
    let inner = Rect::new(0, 0, 50, 1);
    let mut buffer = Buffer::empty(inner);
    draw_tile_cell(
        &mut buffer,
        &TileContent::<u32>::Summary,
        inner,
        1,
        inner.width,
        |buffer, area| Paragraph::new("body").render(area, buffer),
    );

    assert_eq!(buffer_line(&buffer, 0), "body");
    assert!(!buffer_line(&buffer, 0).contains(TILE_ROWS_CONTENT_LABEL));
    assert!(matches!(readout_row(inner), ReadoutRow::CellTooSmall));
}

#[test]
fn an_empty_tile_number_uses_the_only_body_row_before_the_readout() {
    let number = 42;
    for height in [TILE_ROWS_READOUT_HEIGHT, TILE_ROWS_READOUT_HEIGHT + 1] {
        let inner = Rect::new(0, 0, 80, height);
        let mut buffer = Buffer::empty(inner);
        draw_tile_cell(
            &mut buffer,
            &TileContent::<u32>::Empty(number),
            inner,
            0,
            inner.width,
            |_, _| {},
        );

        let sole_row = height == TILE_ROWS_READOUT_HEIGHT;
        assert_eq!(
            buffer_line(&buffer, height - 1).contains(TILE_ROWS_CONTENT_LABEL),
            !sole_row,
            "the readout needs a separate row"
        );
        assert!(
            buffer_line(&buffer, 0).contains(&number.to_string()),
            "the tile number keeps the first body row"
        );
    }
}

struct WrappedSelectedRows;

impl TileCells<u32> for WrappedSelectedRows {
    fn summary_title(&self) -> &'static str { "summary" }
    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary: 12,
            ..TileDemands::default()
        }
    }
    #[expect(
        clippy::expect_used,
        reason = "the test account has one selectable row"
    )]
    fn rows_readout(&self, _: &TileContent<u32>, _: Rect, _: usize) -> RowsReadout {
        RowsReadout::selected(NonZeroUsize::MIN, 1).expect("one selectable account")
    }
    fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
}
