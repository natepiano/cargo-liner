//! Tiles packed top to bottom, column by column, each at its demand. A tile too tall
//! for the rows left in its column continues at the top of the next column.
//!
//! [`SplitBuilder`] assigns contiguous content windows before completing a split tile.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "draw_tile_flow reads the packer from the flow entry phase on"
    )
)]
use std::num::NonZeroU16;
use std::num::NonZeroUsize;

use super::TileContent;
use super::constants::MIN_TILE_HEIGHT;
use super::constants::TILE_BORDER_ROWS;
use super::constants::TILE_ROWS_READOUT_HEIGHT;

/// What a flowed tile draws. The summary keeps its own slot and never flows.
#[derive(Clone, Debug, Eq, PartialEq)]
enum FlowContent<Id> {
    /// One group's contents.
    Group(Id),
    /// An unclaimed cell, carrying its cell number.
    Empty(usize),
}

impl<Id> From<FlowContent<Id>> for TileContent<Id> {
    fn from(content: FlowContent<Id>) -> Self {
        match content {
            FlowContent::Group(id) => Self::Group(id),
            FlowContent::Empty(number) => Self::Empty(number),
        }
    }
}

/// One tile and the content rows its contents ask for, readout and borders excluded.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TileRows<Id> {
    content:      FlowContent<Id>,
    content_rows: u16,
}

impl<Id> TileRows<Id> {
    /// A tile measured at its full content demand.
    const fn new(content: FlowContent<Id>, content_rows: u16) -> Self {
        Self {
            content,
            content_rows,
        }
    }

    /// What the tile draws.
    const fn content(&self) -> &FlowContent<Id> { &self.content }

    /// The demand before adding the readout and borders.
    const fn content_rows(&self) -> u16 { self.content_rows }

    /// Content rows plus readout and borders, at least the minimum tile height.
    fn framed(&self) -> u16 { framed_rows(self.content_rows) }
}

/// Rows of a split tile's contents one piece shows, laid out at their full demand height.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ContentWindow {
    first_row: u16,
}

impl ContentWindow {
    /// The first content row owned by this piece.
    const fn first_row(self) -> u16 { self.first_row }
}

/// Where a piece sits in its column's band.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PieceSpan {
    column: usize,
    top:    u16,
    framed: u16,
}

impl PieceSpan {
    /// The column containing the piece.
    const fn column(&self) -> usize { self.column }

    /// The piece's top relative to the column band.
    const fn top(&self) -> u16 { self.top }

    /// The piece's height, including its readout and borders.
    const fn framed(&self) -> u16 { self.framed }

    /// The next stacked piece shares this piece's bottom border.
    const fn next_top(self) -> u16 { self.top + self.framed - 1 }
}

/// One split piece's geometry, window and owned content rows, of which there is at least one.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SplitPiece {
    at:           PieceSpan,
    window:       ContentWindow,
    content_rows: NonZeroU16,
}

impl SplitPiece {
    /// Where this piece draws.
    const fn at(&self) -> &PieceSpan { &self.at }

    /// Which part of the full-height contents this piece draws.
    const fn window(&self) -> ContentWindow { self.window }

    /// The rows owned by this piece; a clipped final piece may show fewer.
    const fn content_rows(&self) -> NonZeroU16 { self.content_rows }
}

/// A head and a nonempty run of continuations with contiguous, nonoverlapping content windows.
/// Only the split builder constructs this type, accounting for every demanded content row.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SplitTile<Id> {
    content:   FlowContent<Id>,
    head:      SplitPiece,
    continued: SplitPiece,
    more:      Vec<SplitPiece>,
}

impl<Id> SplitTile<Id> {
    /// What every piece of the tile draws.
    const fn content(&self) -> &FlowContent<Id> { &self.content }

    /// The first piece, which carries the tile's original title.
    const fn head(&self) -> &SplitPiece { &self.head }

    /// The nonempty run of pieces carrying the continued title.
    fn continuations(&self) -> impl Iterator<Item = &SplitPiece> {
        std::iter::once(&self.continued).chain(&self.more)
    }
}

/// One tile as the flow places it, in tile order.
#[derive(Clone, Debug, Eq, PartialEq)]
enum FlowPiece<Id> {
    /// Drawn whole in one column.
    Whole { tile: TileRows<Id>, at: PieceSpan },
    /// A head at the foot of one column followed by continuations at later column tops.
    Split(SplitTile<Id>),
}

/// Tiles the grid could not show even at minimum height, all trailing in cell order.
#[derive(Clone, Debug, Eq, PartialEq)]
enum UnseenTiles {
    /// Every tile has a visible piece.
    None,
    /// This many trailing tiles have no visible piece.
    Trailing(NonZeroUsize),
}

/// The packed grid.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FlowLayout<Id> {
    pieces: Vec<FlowPiece<Id>>,
    unseen: UnseenTiles,
}

impl<Id> FlowLayout<Id> {
    /// Pack measured tiles in order, sharing borders and returning spare rows on every call.
    fn pack(bands: &[u16], tiles: Vec<TileRows<Id>>) -> Self {
        let count = tiles.len();
        let mut pieces = Vec::with_capacity(count);
        let mut column = 0;
        let mut top = 0;
        let mut unseen = UnseenTiles::None;
        for (index, tile) in tiles.into_iter().enumerate() {
            let mut flowing = FlowingTile::Whole(tile);
            loop {
                let Some(&band) = bands.get(column) else {
                    if let Some(count) = NonZeroUsize::new(count - index) {
                        unseen = UnseenTiles::Trailing(count);
                    }
                    break;
                };
                let rows = band.saturating_sub(top);
                let last = !bands[column + 1..]
                    .iter()
                    .any(|&height| height >= MIN_TILE_HEIGHT);
                if rows < MIN_TILE_HEIGHT {
                    Self::fill_column_foot(&mut pieces, column, band);
                    column += 1;
                    top = 0;
                    continue;
                }
                if last {
                    let demand = match &flowing {
                        FlowingTile::Whole(tile) => tile.framed(),
                        FlowingTile::Split(builder) => builder.remaining_framed(),
                    };
                    let framed = Self::final_framed(rows, demand, count - index - 1);
                    let at = PieceSpan {
                        column,
                        top,
                        framed,
                    };
                    top = at.next_top();
                    pieces.push(match flowing {
                        FlowingTile::Whole(tile) => FlowPiece::Whole { tile, at },
                        FlowingTile::Split(builder) => {
                            FlowPiece::Split(builder.complete(at).finish())
                        },
                    });
                    break;
                }
                match flowing {
                    FlowingTile::Whole(tile) => {
                        match SplitBuilder::start(
                            tile,
                            PieceSpan {
                                column,
                                top,
                                framed: rows,
                            },
                        ) {
                            ColumnFit::Whole(tile) => {
                                let at = PieceSpan {
                                    column,
                                    top,
                                    framed: tile.framed(),
                                };
                                top = at.next_top();
                                pieces.push(FlowPiece::Whole { tile, at });
                                break;
                            },
                            ColumnFit::Move(tile) => {
                                Self::fill_column_foot(&mut pieces, column, band);
                                flowing = FlowingTile::Whole(tile);
                            },
                            ColumnFit::Split(builder) => flowing = FlowingTile::Split(builder),
                        }
                    },
                    FlowingTile::Split(builder) => match builder.continue_into(column, band) {
                        Continue::More(builder) => flowing = FlowingTile::Split(builder),
                        Continue::Done(builder) => {
                            let tile = builder.finish();
                            let at = tile.more.last().unwrap_or(&tile.continued).at;
                            top = at.next_top();
                            pieces.push(FlowPiece::Split(tile));
                            break;
                        },
                    },
                }
                column += 1;
                top = 0;
            }
            if unseen != UnseenTiles::None {
                break;
            }
        }
        if let Some(&band) = bands.get(column) {
            Self::fill_column_foot(&mut pieces, column, band);
        }
        Self { pieces, unseen }
    }

    /// Grow the last piece in a column to its foot before leaving that column.
    fn fill_column_foot(pieces: &mut [FlowPiece<Id>], column: usize, band: u16) {
        let Some(last) = pieces.last_mut() else {
            return;
        };
        let at = match last {
            FlowPiece::Whole { at, .. } => at,
            FlowPiece::Split(tile) => &mut tile.more.last_mut().unwrap_or(&mut tile.continued).at,
        };
        if at.column == column {
            at.framed = band.saturating_sub(at.top);
        }
    }

    /// Reserve shared-border minimum slots for as many following tiles as the band can hold.
    fn final_framed(rows: u16, demand: u16, tiles_after: usize) -> u16 {
        let following_capacity = (rows - MIN_TILE_HEIGHT) / (MIN_TILE_HEIGHT - 1);
        let tiles_after = u16::try_from(tiles_after).unwrap_or(u16::MAX);
        let following = following_capacity.min(tiles_after);
        let reserve = following * (MIN_TILE_HEIGHT - 1);
        demand.min(rows - reserve)
    }
}

/// Add readout and borders without overflowing terminal row coordinates.
fn framed_rows(content_rows: u16) -> u16 {
    content_rows
        .saturating_add(TILE_ROWS_READOUT_HEIGHT)
        .saturating_add(TILE_BORDER_ROWS)
        .max(MIN_TILE_HEIGHT)
}

/// Content rows shown by a framed piece; a readout gives way in a one-row interior.
const fn shown_content_rows(framed: u16) -> u16 {
    let inner = framed.saturating_sub(TILE_BORDER_ROWS);
    if inner > TILE_ROWS_READOUT_HEIGHT {
        inner - TILE_ROWS_READOUT_HEIGHT
    } else {
        inner
    }
}

/// Continuations already assigned while the builder still owns unassigned rows.
#[derive(Clone, Debug, Eq, PartialEq)]
enum BuiltContinuations {
    /// The head has not yet reached another column.
    AwaitingFirst,
    /// At least one continuation has been assigned.
    Started {
        continued: SplitPiece,
        more:      Vec<SplitPiece>,
    },
}

impl BuiltContinuations {
    /// Append one continuation, preserving a nonempty run in the completed state.
    fn append(self, piece: SplitPiece) -> SplitReady {
        match self {
            Self::AwaitingFirst => SplitReady {
                continued: piece,
                more:      Vec::new(),
            },
            Self::Started {
                continued,
                mut more,
            } => {
                more.push(piece);
                SplitReady { continued, more }
            },
        }
    }
}

/// The cursor for a split whose content still needs a continuation.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SplitPending {
    first_row: u16,
    remaining: NonZeroU16,
    built:     BuiltContinuations,
}

/// Every content row has an owner and at least one continuation exists.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SplitReady {
    continued: SplitPiece,
    more:      Vec<SplitPiece>,
}

/// The packer's only way to make a split tile: it owns the row cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SplitBuilder<Id, State = SplitPending> {
    content: FlowContent<Id>,
    head:    SplitPiece,
    state:   State,
}

impl<Id> SplitBuilder<Id> {
    /// Framed demand still awaiting a continuation.
    fn remaining_framed(&self) -> u16 { framed_rows(self.state.remaining.get()) }

    /// Try the entire remaining column band as a head, without losing any content rows.
    fn start(tile: TileRows<Id>, head_at: PieceSpan) -> ColumnFit<Id> {
        if tile.framed() <= head_at.framed {
            return ColumnFit::Whole(tile);
        }
        if head_at.framed < MIN_TILE_HEIGHT {
            return ColumnFit::Move(tile);
        }
        let shown = shown_content_rows(head_at.framed);
        let Some(remaining) = NonZeroU16::new(tile.content_rows.saturating_sub(shown)) else {
            return ColumnFit::Move(tile);
        };
        let Some(content_rows) = NonZeroU16::new(shown) else {
            return ColumnFit::Move(tile);
        };
        ColumnFit::Split(Self {
            content: tile.content,
            head:    SplitPiece {
                at: head_at,
                window: ContentWindow { first_row: 0 },
                content_rows,
            },
            state:   SplitPending {
                first_row: shown,
                remaining,
                built: BuiltContinuations::AwaitingFirst,
            },
        })
    }

    /// Open the next continuation at the column top, keeping the row cursor contiguous.
    fn continue_into(self, column: usize, band: u16) -> Continue<Id> {
        let framed = framed_rows(self.state.remaining.get());
        if framed <= band {
            return Continue::Done(self.complete(PieceSpan {
                column,
                top: 0,
                framed,
            }));
        }
        let shown = shown_content_rows(band);
        let Some(remaining) = NonZeroU16::new(self.state.remaining.get().saturating_sub(shown))
        else {
            return Continue::Done(self.complete(PieceSpan {
                column,
                top: 0,
                framed: band,
            }));
        };
        let Some(content_rows) = NonZeroU16::new(shown) else {
            return Continue::More(self);
        };
        let piece = SplitPiece {
            at: PieceSpan {
                column,
                top: 0,
                framed: band,
            },
            window: ContentWindow {
                first_row: self.state.first_row,
            },
            content_rows,
        };
        let built = self.state.built.append(piece);
        Continue::More(Self {
            content: self.content,
            head:    self.head,
            state:   SplitPending {
                first_row: self.state.first_row + shown,
                remaining,
                built: BuiltContinuations::Started {
                    continued: built.continued,
                    more:      built.more,
                },
            },
        })
    }

    /// Give the final piece every unassigned row, even when its geometry must clip them.
    fn complete(self, at: PieceSpan) -> SplitBuilder<Id, SplitReady> {
        let piece = SplitPiece {
            at,
            window: ContentWindow {
                first_row: self.state.first_row,
            },
            content_rows: self.state.remaining,
        };
        SplitBuilder {
            content: self.content,
            head:    self.head,
            state:   self.state.built.append(piece),
        }
    }
}

impl<Id> SplitBuilder<Id, SplitReady> {
    /// Finish only after every row has an owner and a continuation exists.
    fn finish(self) -> SplitTile<Id> {
        SplitTile {
            content:   self.content,
            head:      self.head,
            continued: self.state.continued,
            more:      self.state.more,
        }
    }
}

/// Whether a whole tile fits, must move, or can begin a split in this column.
#[derive(Clone, Debug, Eq, PartialEq)]
enum ColumnFit<Id> {
    /// Demand fits without splitting.
    Whole(TileRows<Id>),
    /// Too little content would remain on either side of a split.
    Move(TileRows<Id>),
    /// A nonempty head leaves nonempty content for the next column.
    Split(SplitBuilder<Id>),
}

/// A continuation either leaves rows for another column or completes the tile.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Continue<Id> {
    /// More columns are needed.
    More(SplitBuilder<Id>),
    /// The builder now has a nonempty, complete continuation run.
    Done(SplitBuilder<Id, SplitReady>),
}

/// One tile is either waiting to start or waiting to finish its split.
#[derive(Clone, Debug, Eq, PartialEq)]
enum FlowingTile<Id> {
    /// No piece of this tile has been placed.
    Whole(TileRows<Id>),
    /// The builder owns the head, continuations and remaining row cursor.
    Split(SplitBuilder<Id>),
}

#[cfg(test)]
mod tests {
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
            .map(|(index, &rows)| {
                TileRows::new(FlowContent::Group(u8::try_from(index).unwrap()), rows)
            })
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
                // Even a three-row `band` shows one content row, so `bands` gives
                // every tile enough columns to finish without last-column clipping.
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
}
