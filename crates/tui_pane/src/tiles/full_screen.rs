//! One tile shown on the full pane, with its motion between the grid and the full pane.
//!
//! [`FullScreenView`] owns the shown tile's identity and its motion clock. The app owns
//! the keys, the content and its own policy state `P`, such as a scroll position. Each
//! frame the app calls [`FullScreenView::resolve_opening`], [`FullScreenView::travel`],
//! [`TileGrid::draw_cell_view`](crate::TileGrid::draw_cell_view) and then
//! [`FullScreenView::settle`].

use std::time::Duration;
use std::time::Instant;

use ratatui::layout::Rect;

use super::grid::CellTravel;
use super::grid::TileContent;
use super::grid::TilePlacement;
use crate::Repaint;

/// A resolved full-screen tile: its identity, its motion endpoints and the app policy state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FullScreenCell<Id, P> {
    content:   TileContent<Id>,
    origin:    Rect,
    displayed: Rect,
    sampled:   Instant,
    policy:    P,
}

impl<Id, P> FullScreenCell<Id, P> {
    /// A tile that starts its motion from `area`, its grid allocation.
    const fn new(content: TileContent<Id>, area: Rect, sampled: Instant, policy: P) -> Self {
        Self {
            content,
            origin: area,
            displayed: area,
            sampled,
            policy,
        }
    }

    /// The tile this view shows.
    #[must_use]
    pub const fn content(&self) -> &TileContent<Id> { &self.content }

    /// The rectangle the last frame drew.
    #[must_use]
    pub const fn displayed(&self) -> Rect { self.displayed }

    /// The app policy state, such as a scroll position.
    #[must_use]
    pub const fn policy(&self) -> &P { &self.policy }

    /// The app policy state, for a key or a draw to change.
    const fn policy_mut(&mut self) -> &mut P { &mut self.policy }

    /// Start a new motion from the rectangle the last frame drew.
    const fn restart(&mut self, now: Instant) {
        self.origin = self.displayed;
        self.sampled = now;
    }
}

/// The grid, or one tile on its way to, at, or back from the full pane.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum FullScreenView<Id, P> {
    /// The grid shows every tile.
    #[default]
    Grid,
    /// Expand was asked for; the next frame resolves the focused tile.
    Opening {
        /// The motion clock reading.
        sampled: Instant,
        /// When the expand started.
        started: Instant,
    },
    /// The tile moves from its grid allocation to the full pane.
    Expanding {
        /// The shown tile.
        cell:    FullScreenCell<Id, P>,
        /// When this motion started.
        started: Instant,
    },
    /// The tile fills the pane.
    Expanded {
        /// The shown tile.
        cell: FullScreenCell<Id, P>,
    },
    /// The tile moves from the full pane back to its grid allocation.
    Collapsing {
        /// The shown tile.
        cell:    FullScreenCell<Id, P>,
        /// When this motion started.
        started: Instant,
    },
}

/// What the full-screen view draws this frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FullScreenStep<Id> {
    /// The grid shows, and no tile draws over it.
    Inactive,
    /// The collapsing tile has no grid placement now. Close the view and draw the grid.
    ContentLeft,
    /// Draw `content` at this point of its motion.
    Travel {
        /// The shown tile.
        content: TileContent<Id>,
        /// The motion for [`TileGrid::draw_cell_view`](crate::TileGrid::draw_cell_view).
        travel:  CellTravel,
    },
}

/// What a drawn frame did to the full-screen view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FullScreenSettle {
    /// The tile stays over the grid.
    Shown,
    /// The collapse ended this frame. Draw the grid again.
    Closed,
}

impl<Id, P> FullScreenView<Id, P> {
    /// Whether a tile shows, or will show after the next frame, over the grid.
    #[must_use]
    pub const fn is_open(&self) -> bool { !matches!(self, Self::Grid) }

    /// The shown tile, when one has resolved.
    #[must_use]
    pub const fn cell(&self) -> Option<&FullScreenCell<Id, P>> {
        match self {
            Self::Expanding { cell, .. }
            | Self::Expanded { cell }
            | Self::Collapsing { cell, .. } => Some(cell),
            Self::Grid | Self::Opening { .. } => None,
        }
    }

    /// The app policy state of the shown tile, when one has resolved.
    pub const fn policy_mut(&mut self) -> Option<&mut P> {
        match self {
            Self::Expanding { cell, .. }
            | Self::Expanded { cell }
            | Self::Collapsing { cell, .. } => Some(cell.policy_mut()),
            Self::Grid | Self::Opening { .. } => None,
        }
    }

    /// Read the app's poll clock. A moving view needs another frame.
    pub const fn advance(&mut self, now: Instant) -> Repaint {
        match self {
            Self::Opening { sampled, .. } => {
                *sampled = now;
                Repaint::Needed
            },
            Self::Expanding { cell, .. } | Self::Collapsing { cell, .. } => {
                cell.sampled = now;
                Repaint::Needed
            },
            Self::Grid | Self::Expanded { .. } => Repaint::NotNeeded,
        }
    }

    /// Start to show the focused tile on the full pane, or reverse a collapse.
    pub fn expand(&mut self, now: Instant) {
        *self = match std::mem::take(self) {
            Self::Grid => Self::Opening {
                sampled: now,
                started: now,
            },
            Self::Collapsing { mut cell, .. } => {
                cell.restart(now);
                Self::Expanding { cell, started: now }
            },
            state @ (Self::Opening { .. } | Self::Expanding { .. } | Self::Expanded { .. }) => {
                state
            },
        };
    }

    /// Start to move the shown tile back to its grid allocation.
    pub fn collapse(&mut self, now: Instant) {
        *self = match std::mem::take(self) {
            Self::Opening { .. } => Self::Grid,
            Self::Expanding { mut cell, .. } | Self::Expanded { mut cell } => {
                cell.restart(now);
                Self::Collapsing { cell, started: now }
            },
            state @ (Self::Grid | Self::Collapsing { .. }) => state,
        };
    }

    /// Show the grid at once, with no motion.
    pub fn close(&mut self) { *self = Self::Grid; }

    /// Record the drawn rectangle. A motion that reached its end changes state.
    pub fn settle(&mut self, displayed: Rect, repaint: Repaint) -> FullScreenSettle {
        let settled = repaint == Repaint::NotNeeded;
        let (next, settle) = match std::mem::take(self) {
            Self::Expanding { mut cell, started } => {
                cell.displayed = displayed;
                if settled {
                    (Self::Expanded { cell }, FullScreenSettle::Shown)
                } else {
                    (Self::Expanding { cell, started }, FullScreenSettle::Shown)
                }
            },
            Self::Expanded { mut cell } => {
                cell.displayed = displayed;
                (Self::Expanded { cell }, FullScreenSettle::Shown)
            },
            Self::Collapsing { .. } if settled => (Self::Grid, FullScreenSettle::Closed),
            Self::Collapsing { mut cell, started } => {
                cell.displayed = displayed;
                (Self::Collapsing { cell, started }, FullScreenSettle::Shown)
            },
            Self::Grid | Self::Opening { .. } => (Self::Grid, FullScreenSettle::Shown),
        };
        *self = next;
        settle
    }
}

impl<Id: Clone + PartialEq, P> FullScreenView<Id, P> {
    /// Resolve an opening view to the focused tile. With no focused tile the grid stays.
    pub fn resolve_opening(
        &mut self,
        placements: &[TilePlacement<Id>],
        policy: impl FnOnce() -> P,
    ) {
        let Self::Opening { sampled, started } = *self else {
            return;
        };
        *self = placements
            .iter()
            .find(|placement| placement.frame.is_focused())
            .map_or(Self::Grid, |placement| Self::Expanding {
                cell: FullScreenCell::new(
                    placement.content.clone(),
                    placement.frame.clip(),
                    sampled,
                    policy(),
                ),
                started,
            });
    }

    /// The motion to draw over the grid in `area`, whose current tiles are `placements`.
    #[must_use]
    pub fn travel(&self, area: Rect, placements: &[TilePlacement<Id>]) -> FullScreenStep<Id> {
        let (cell, started) = match self {
            Self::Grid | Self::Opening { .. } => return FullScreenStep::Inactive,
            Self::Expanding { cell, started } | Self::Collapsing { cell, started } => {
                (cell, Some(*started))
            },
            Self::Expanded { cell } => (cell, None),
        };
        let to = if matches!(self, Self::Collapsing { .. }) {
            let Some(placement) = placements
                .iter()
                .find(|placement| placement.content == cell.content)
            else {
                return FullScreenStep::ContentLeft;
            };
            placement.frame.clip()
        } else {
            area
        };
        let elapsed = started.map_or(Duration::MAX, |started| {
            cell.sampled.saturating_duration_since(started)
        });
        FullScreenStep::Travel {
            content: cell.content.clone(),
            travel:  CellTravel {
                from: cell.origin.intersection(area),
                to: to.intersection(area),
                elapsed,
            },
        }
    }
}

#[cfg(test)]
#[path = "full_screen_tests.rs"]
mod full_screen_tests;
