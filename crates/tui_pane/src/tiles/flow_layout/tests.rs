#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]

use std::num::NonZeroUsize;

use ratatui::layout::Rect;

use super::*;
use crate::frame_inner;
use crate::tiles::constants::MIN_TILE_HEIGHT;
use crate::tiles::draw;

/// Give each requested tile a distinct group in cell order.
fn tiles(rows: &[u16]) -> Vec<TileRows<u8>> {
    rows.iter()
        .enumerate()
        .map(|(index, &rows)| TileRows::new(FlowContent::Group(u8::try_from(index).unwrap()), rows))
        .collect()
}

/// Read a placement without constructing packer-owned geometry.
const fn span(at: &PieceSpan) -> (usize, u16, u16) { (at.column(), at.top(), at.framed()) }

#[test]
fn flow_natedev_continues_after_row_twenty_eight() {
    let layout = FlowLayout::pack(&[32, 32], tiles(&[40, 2]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    assert_eq!(layout.pieces.len(), 2);
    let FlowPiece::Split(tile) = &layout.pieces[0] else {
        panic!("the 43-row tile must cross the first 32-row band");
    };
    assert_eq!(tile.content(), &FlowContent::Group(0));
    assert_eq!(span(tile.head().at()), (0, 0, 32));
    assert_eq!(tile.head().window().first_row(), 0);
    assert_eq!(tile.head().content_rows().get(), 29);

    let continued: Vec<_> = tile.continuations().collect();
    assert_eq!(continued.len(), 1);
    assert_eq!(span(continued[0].at()), (1, 0, 14));
    assert_eq!(continued[0].window().first_row(), 29);
    assert_eq!(continued[0].content_rows().get(), 11);

    let FlowPiece::Whole { tile, at } = &layout.pieces[1] else {
        panic!("the following tile fits in the continuation's column");
    };
    assert_eq!(tile.content(), &FlowContent::Group(1));
    assert_eq!(span(at), (1, 13, 19));
}

#[test]
fn flow_hana_crosses_three_columns_without_repeating_rows() {
    let layout = FlowLayout::pack(&[40, 40, 40], tiles(&[90, 1]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    let FlowPiece::Split(tile) = &layout.pieces[0] else {
        panic!("90 content rows need three 40-row bands");
    };
    let pieces: Vec<_> = std::iter::once(tile.head())
        .chain(tile.continuations())
        .collect();
    assert_eq!(pieces.len(), 3);
    let expected = [(0, 0, 40, 0, 37), (1, 0, 40, 37, 37), (2, 0, 19, 74, 16)];
    for (piece, (column, top, framed, first, rows)) in pieces.iter().zip(expected) {
        assert_eq!(span(piece.at()), (column, top, framed));
        assert_eq!(piece.window().first_row(), first);
        assert_eq!(piece.content_rows().get(), rows);
    }
    assert_eq!(
        pieces
            .iter()
            .map(|piece| piece.content_rows().get())
            .sum::<u16>(),
        90
    );
}

#[test]
fn flow_split_head_uses_the_rows_left_after_a_whole_tile() {
    let layout = FlowLayout::pack(&[16, 20], tiles(&[2, 20, 1]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    assert_eq!(layout.pieces.len(), 3);
    let FlowPiece::Whole { at, .. } = &layout.pieces[0] else {
        panic!("the first tile fits whole");
    };
    assert_eq!(span(at), (0, 0, 5));
    let FlowPiece::Split(tile) = &layout.pieces[1] else {
        panic!("the next tile must split at the first tile's shared border");
    };
    assert_eq!(tile.content(), &FlowContent::Group(1));
    assert_eq!(span(tile.head().at()), (0, 4, 12));
    assert_eq!(tile.head().window().first_row(), 0);
    assert_eq!(tile.head().content_rows().get(), 9);
    let continued: Vec<_> = tile.continuations().collect();
    assert_eq!(continued.len(), 1);
    assert_eq!(span(continued[0].at()), (1, 0, 14));
    assert_eq!(continued[0].window().first_row(), 9);
    assert_eq!(continued[0].content_rows().get(), 11);
    let FlowPiece::Whole { at, .. } = &layout.pieces[2] else {
        panic!("the third tile fits after the continuation");
    };
    assert_eq!(span(at), (1, 13, 7));
}

#[test]
fn flow_short_tiles_return_rows_on_every_call() {
    let grown = FlowLayout::pack(&[40], tiles(&[20, 2, 2]));
    let short = FlowLayout::pack(&[40], tiles(&[2, 2, 2]));

    let FlowPiece::Whole { at: grown_at, .. } = &grown.pieces[1] else {
        panic!("one band cannot split a tile");
    };
    assert_eq!(grown_at.top(), 22);
    assert_eq!(short.unseen, UnseenTiles::None);
    assert_eq!(short.pieces.len(), 3);
    for (piece, expected) in short.pieces.iter().zip([(0, 0, 5), (0, 4, 5), (0, 8, 32)]) {
        let FlowPiece::Whole { tile, at } = piece else {
            panic!("short tiles fit whole");
        };
        assert_eq!(tile.content_rows(), 2);
        assert_eq!(tile.framed(), 5);
        assert_eq!(span(at), expected);
    }
}

#[test]
fn flow_two_rows_left_move_the_next_tile_whole() {
    let layout = FlowLayout::pack(&[6, 12], tiles(&[2, 4]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    assert_eq!(layout.pieces.len(), 2);
    for (piece, expected) in layout.pieces.iter().zip([(0, 0, 6), (1, 0, 12)]) {
        let FlowPiece::Whole { at, .. } = piece else {
            panic!("two leftover rows cannot open a split head");
        };
        assert_eq!(span(at), expected);
    }
}

#[test]
fn flow_head_with_no_remaining_content_moves_whole() {
    let layout = FlowLayout::pack(&[6, 12], tiles(&[1, 1]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    assert_eq!(layout.pieces.len(), 2);
    for (piece, expected) in layout.pieces.iter().zip([(0, 0, 6), (1, 0, 12)]) {
        let FlowPiece::Whole { at, .. } = piece else {
            panic!("a three-row head would show everything and leave no continuation");
        };
        assert_eq!(span(at), expected);
    }
}

#[test]
fn flow_last_column_reserves_every_tile_floor_and_clips_the_last() {
    let layout = FlowLayout::pack(&[12], tiles(&[2, 2, 20]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    assert_eq!(layout.pieces.len(), 3);
    for (piece, expected) in layout.pieces.iter().zip([(0, 0, 5), (0, 4, 5), (0, 8, 4)]) {
        let FlowPiece::Whole { at, .. } = piece else {
            panic!("the last column clips without opening a continuation");
        };
        assert_eq!(span(at), expected);
        assert!(at.framed() >= MIN_TILE_HEIGHT);
    }
    let FlowPiece::Whole { tile, at } = &layout.pieces[2] else {
        panic!("the last placed tile must remain whole");
    };
    assert!(at.framed() < tile.framed());
}

#[test]
fn flow_large_demands_cannot_consume_later_tiles_floors() {
    let layout = FlowLayout::pack(&[9], tiles(&[20, 20, 20]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    assert_eq!(layout.pieces.len(), 3);
    let mut top = 0;
    for (index, piece) in layout.pieces.iter().enumerate() {
        let FlowPiece::Whole { tile, at } = piece else {
            panic!("one column cannot split a tile");
        };
        assert_eq!(
            tile.content(),
            &FlowContent::Group(u8::try_from(index).unwrap())
        );
        assert_eq!(at.column(), 0);
        assert_eq!(at.top(), top);
        assert!(at.framed() >= MIN_TILE_HEIGHT);
        top += at.framed() - 1;
    }
    assert_eq!(top + 1, 9);
}

#[test]
fn flow_reports_the_exact_trailing_unseen_count() {
    let layout = FlowLayout::pack(&[8], tiles(&[0, 0, 20, 2, 2]));

    assert_eq!(
        layout.unseen,
        UnseenTiles::Trailing(NonZeroUsize::new(2).unwrap())
    );
    assert_eq!(layout.pieces.len(), 3);
    for (index, (piece, expected)) in layout
        .pieces
        .iter()
        .zip([(0, 0, 3), (0, 2, 3), (0, 4, 4)])
        .enumerate()
    {
        let FlowPiece::Whole { tile, at } = piece else {
            panic!("a single band must not split tiles");
        };
        assert_eq!(
            tile.content(),
            &FlowContent::Group(u8::try_from(index).unwrap())
        );
        assert_eq!(span(at), expected);
    }
    let FlowPiece::Whole { tile, at } = &layout.pieces[2] else {
        panic!("the last placed tile must remain whole");
    };
    assert!(at.framed() < tile.framed());
}

#[test]
fn flow_split_windows_are_contiguous_across_content_and_band_sizes() {
    for content_rows in 1..=150 {
        for band in 3..=40 {
            // Even a three-row band shows one content row, so this gives every
            // tile enough columns to finish without last-column clipping.
            let bands = vec![band; usize::from(content_rows) + 2];
            let layout = FlowLayout::pack(&bands, tiles(&[content_rows]));
            assert_eq!(layout.unseen, UnseenTiles::None);
            assert_eq!(layout.pieces.len(), 1);
            let FlowPiece::Split(tile) = &layout.pieces[0] else {
                assert!(
                    content_rows <= shown_content_rows(band),
                    "rows={content_rows}, band={band}: a tile needing a continuation stayed whole"
                );
                continue;
            };
            let mut next_row = 0;
            let first_column = tile.head().at().column();
            assert!(tile.continuations().count() >= 1);
            for (index, piece) in std::iter::once(tile.head())
                .chain(tile.continuations())
                .enumerate()
            {
                assert_eq!(
                    piece.window().first_row(),
                    next_row,
                    "rows={content_rows}, band={band}"
                );
                assert_eq!(piece.at().column(), first_column + index);
                assert_eq!(piece.at().top(), 0);
                assert!(piece.at().framed() <= band);
                assert!(piece.content_rows().get() <= shown_content_rows(piece.at().framed()));
                next_row += piece.content_rows().get();
            }
            assert_eq!(next_row, content_rows, "rows={content_rows}, band={band}");
        }
    }
}

#[test]
fn flow_shown_rows_match_the_renderer_including_readout_fallback() {
    for framed in 3..=12 {
        let actual = draw::content_area(frame_inner(Rect::new(0, 0, 40, framed))).height;
        assert_eq!(shown_content_rows(framed), actual, "framed={framed}");
    }
}

#[test]
fn flow_single_band_keeps_fitting_tiles_whole_and_in_order() {
    let layout = FlowLayout::pack(&[20], tiles(&[4, 1, 0]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    assert_eq!(layout.pieces.len(), 3);
    for (index, (piece, expected)) in layout
        .pieces
        .iter()
        .zip([(0, 0, 7), (0, 6, 4), (0, 9, 11)])
        .enumerate()
    {
        let FlowPiece::Whole { tile, at } = piece else {
            panic!("a single band must keep every tile whole");
        };
        assert_eq!(
            tile.content(),
            &FlowContent::Group(u8::try_from(index).unwrap())
        );
        assert_eq!(span(at), expected);
    }
}

#[test]
fn flow_empty_slots_keep_their_cell_identity() {
    let layout: FlowLayout<u8> =
        FlowLayout::pack(&[12], vec![TileRows::new(FlowContent::Empty(7), 2)]);

    assert_eq!(layout.unseen, UnseenTiles::None);
    let [FlowPiece::Whole { tile, at }] = layout.pieces.as_slice() else {
        panic!("the empty slot must still occupy a tile");
    };
    assert_eq!(tile.content(), &FlowContent::Empty(7));
    assert_eq!(span(at), (0, 0, 12));
    assert_eq!(
        TileContent::from(tile.content().clone()),
        TileContent::Empty(7)
    );
}

#[test]
fn flow_without_columns_reports_every_tile_unseen() {
    let layout = FlowLayout::pack(&[], tiles(&[2, 40, 0]));

    assert_eq!(layout.pieces, []);
    assert_eq!(
        layout.unseen,
        UnseenTiles::Trailing(NonZeroUsize::new(3).unwrap())
    );
}

#[test]
fn flow_no_tiles_need_no_unseen_readout() {
    let layout = FlowLayout::<u8>::pack(&[32, 40], Vec::new());

    assert_eq!(layout.pieces, []);
    assert_eq!(layout.unseen, UnseenTiles::None);
}

#[test]
fn flow_clipped_final_continuation_owns_all_remaining_content() {
    let layout = FlowLayout::pack(&[32, 16], tiles(&[40, 2, 2]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    assert_eq!(layout.pieces.len(), 3);
    let FlowPiece::Split(tile) = &layout.pieces[0] else {
        panic!("the first tile must continue into the last column");
    };
    assert_eq!(span(tile.head().at()), (0, 0, 32));
    assert_eq!(tile.head().content_rows().get(), 29);
    let continued: Vec<_> = tile.continuations().collect();
    assert_eq!(continued.len(), 1);
    let last = continued[0];
    assert_eq!(span(last.at()), (1, 0, 12));
    assert_eq!(last.window().first_row(), 29);
    assert_eq!(last.content_rows().get(), 11);
    assert!(shown_content_rows(last.at().framed()) < last.content_rows().get());
    assert_eq!(
        tile.head().content_rows().get() + last.content_rows().get(),
        40
    );
    for (piece, expected) in layout.pieces[1..].iter().zip([(1, 11, 3), (1, 13, 3)]) {
        let FlowPiece::Whole { at, .. } = piece else {
            panic!("later tiles must retain their floor after the clipped continuation");
        };
        assert_eq!(span(at), expected);
    }
}

#[test]
fn flow_clipped_continuation_reserves_floors_before_reporting_unseen_tiles() {
    let layout = FlowLayout::pack(&[32, 8], tiles(&[40, 0, 0, 0]));

    assert_eq!(
        layout.unseen,
        UnseenTiles::Trailing(NonZeroUsize::new(1).unwrap())
    );
    assert_eq!(layout.pieces.len(), 3);
    let FlowPiece::Split(tile) = &layout.pieces[0] else {
        panic!("the partially visible first tile must keep its split windows");
    };
    let continued: Vec<_> = tile.continuations().collect();
    assert_eq!(continued.len(), 1);
    assert_eq!(span(continued[0].at()), (1, 0, 4));
    assert_eq!(continued[0].window().first_row(), 29);
    assert_eq!(continued[0].content_rows().get(), 11);
    for (index, (piece, expected)) in layout.pieces[1..]
        .iter()
        .zip([(1, 3, 3), (1, 5, 3)])
        .enumerate()
    {
        let FlowPiece::Whole { tile, at } = piece else {
            panic!("every later floor that fits must stay visible");
        };
        assert_eq!(
            tile.content(),
            &FlowContent::Group(u8::try_from(index + 1).unwrap())
        );
        assert_eq!(span(at), expected);
    }
}

#[test]
fn flow_zero_and_undersized_bands_cannot_start_tiles() {
    let layout = FlowLayout::pack(&[0, 1, 2, 12], tiles(&[2]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    let [FlowPiece::Whole { tile, at }] = layout.pieces.as_slice() else {
        panic!("the tile must move whole to the first usable band");
    };
    assert_eq!(tile.content(), &FlowContent::Group(0));
    assert_eq!(span(at), (3, 0, 12));
}

#[test]
fn flow_only_undersized_bands_leave_all_tiles_unseen() {
    let layout = FlowLayout::pack(&[2, 0, 1], tiles(&[1, 2]));

    assert_eq!(layout.pieces, []);
    assert_eq!(
        layout.unseen,
        UnseenTiles::Trailing(NonZeroUsize::new(2).unwrap())
    );
}

#[test]
fn flow_continuations_skip_bands_that_cannot_hold_a_piece() {
    let layout = FlowLayout::pack(&[8, 0, 1, 2, 20], tiles(&[12, 1]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    assert_eq!(layout.pieces.len(), 2);
    let FlowPiece::Split(tile) = &layout.pieces[0] else {
        panic!("the tall tile needs a usable continuation band");
    };
    assert_eq!(span(tile.head().at()), (0, 0, 8));
    assert_eq!(tile.head().content_rows().get(), 5);
    let continued: Vec<_> = tile.continuations().collect();
    assert_eq!(continued.len(), 1);
    assert_eq!(span(continued[0].at()), (4, 0, 10));
    assert_eq!(continued[0].window().first_row(), 5);
    assert_eq!(continued[0].content_rows().get(), 7);
    let FlowPiece::Whole { at, .. } = &layout.pieces[1] else {
        panic!("the next tile fits after the continuation");
    };
    assert_eq!(span(at), (4, 9, 11));
}

#[test]
fn flow_undersized_trailing_bands_do_not_force_an_unfinishable_split() {
    let layout = FlowLayout::pack(&[8, 0, 2], tiles(&[20]));

    assert_eq!(layout.unseen, UnseenTiles::None);
    let [FlowPiece::Whole { tile, at }] = layout.pieces.as_slice() else {
        panic!("the last usable band must clip the tile whole");
    };
    assert_eq!(tile.content_rows(), 20);
    assert_eq!(span(at), (0, 0, 8));
}
