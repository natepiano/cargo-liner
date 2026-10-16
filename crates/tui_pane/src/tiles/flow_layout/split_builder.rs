//! [`SplitBuilder`] assigns contiguous content windows before completing a split tile.

use std::num::NonZeroU16;

use super::ContentWindow;
use super::FlowContent;
use super::PieceSpan;
use super::SplitPiece;
use super::SplitTile;
use super::TileRows;
use super::framed_rows;
use super::shown_content_rows;
use crate::tiles::constants::MIN_TILE_HEIGHT;

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
pub(super) struct SplitPending {
    first_row: u16,
    remaining: NonZeroU16,
    built:     BuiltContinuations,
}

/// Every content row has an owner and at least one continuation exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SplitReady {
    continued: SplitPiece,
    more:      Vec<SplitPiece>,
}

/// The packer's only way to make a split tile: it owns the row cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SplitBuilder<Id, State = SplitPending> {
    content: FlowContent<Id>,
    head:    SplitPiece,
    state:   State,
}

impl<Id> SplitBuilder<Id> {
    /// Framed demand still awaiting a continuation.
    pub(super) fn remaining_framed(&self) -> u16 { framed_rows(self.state.remaining.get()) }

    /// Try the entire remaining column band as a head, without losing any content rows.
    pub(super) fn start(tile: TileRows<Id>, head_at: PieceSpan) -> ColumnFit<Id> {
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
    pub(super) fn continue_into(self, column: usize, band: u16) -> Continue<Id> {
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
    pub(super) fn complete(self, at: PieceSpan) -> SplitBuilder<Id, SplitReady> {
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
    pub(super) fn finish(self) -> SplitTile<Id> {
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
pub(super) enum ColumnFit<Id> {
    /// Demand fits without splitting.
    Whole(TileRows<Id>),
    /// Too little content would remain on either side of a split.
    Move(TileRows<Id>),
    /// A nonempty head leaves nonempty content for the next column.
    Split(SplitBuilder<Id>),
}

/// A continuation either leaves rows for another column or completes the tile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Continue<Id> {
    /// More columns are needed.
    More(SplitBuilder<Id>),
    /// The builder now has a nonempty, complete continuation run.
    Done(SplitBuilder<Id, SplitReady>),
}

/// One tile is either waiting to start or waiting to finish its split.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FlowingTile<Id> {
    /// No piece of this tile has been placed.
    Whole(TileRows<Id>),
    /// The builder owns the head, continuations and remaining row cursor.
    Split(SplitBuilder<Id>),
}
