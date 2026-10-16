//! Tiles packed top to bottom, column by column, each at its demand. A tile too tall
//! for the rows left in its column continues at the top of the next column.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "draw_tile_flow reads the packer from the flow entry phase on"
    )
)]
mod split_builder;
#[cfg(test)]
mod tests;

use std::num::NonZeroU16;
use std::num::NonZeroUsize;

use self::split_builder::ColumnFit;
use self::split_builder::Continue;
use self::split_builder::FlowingTile;
use self::split_builder::SplitBuilder;
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
                    fill_column_foot(&mut pieces, column, band);
                    column += 1;
                    top = 0;
                    continue;
                }
                if last {
                    let demand = match &flowing {
                        FlowingTile::Whole(tile) => tile.framed(),
                        FlowingTile::Split(builder) => builder.remaining_framed(),
                    };
                    let framed = final_framed(rows, demand, count - index - 1);
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
                                fill_column_foot(&mut pieces, column, band);
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
            fill_column_foot(&mut pieces, column, band);
        }
        Self { pieces, unseen }
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

/// Grow the last piece in a column to its foot before leaving that column.
fn fill_column_foot<Id>(pieces: &mut [FlowPiece<Id>], column: usize, band: u16) {
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
