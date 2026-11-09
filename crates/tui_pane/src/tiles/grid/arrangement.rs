#[cfg(test)]
#[path = "arrangement_tests.rs"]
mod tests;

use super::CellRowDemand;
use super::ColumnPlan;
use super::Rect;
use super::TABLE_CELL;
use super::TileContent;
use super::TileDemands;
use super::TileGrowth;
use super::TileSettings;
use super::column_layout;
/// A cell after the summary.
///
/// The identity here is what makes the motion work. A cell is animated
/// from where its slot stood to where the same slot stands now, so a
/// command finishing in the middle of the grid draws every cell after it
/// travelling one place forward, rather than the contents jumping
/// between cells that stayed put.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Slot<Id> {
    /// The tile drawing one command's rows.
    Group(Id),
    /// A cell opened with `+` and not yet claimed. The number is only
    /// an identity -- it is never shown, and no two empties share one --
    /// so an empty cell can be told from its neighbour while the grid
    /// closes up around them.
    Empty(u64),
}

/// The cell holding focus.
///
/// Focus is held by identity rather than by cell number for the same
/// reason the cells are: the command a developer is watching keeps the
/// ring as the grid closes up around it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Focus<Id> {
    /// The summary, where focus starts and where it falls back to.
    Summary,
    /// One of the cells after it.
    Cell(Slot<Id>),
}

/// One step of the focus ring, named for the arrow that asks for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Direction {
    /// Toward the column on the left.
    Left,
    /// Toward the column on the right.
    Right,
    /// Toward the cell above, within one column.
    Up,
    /// Toward the cell below, within one column.
    Down,
}

/// Where the focus ring is located within the current arrangement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FocusLocation {
    /// The ring sits on this one-based cell number, including the summary.
    Cell(usize),
    /// The focused slot is outside the arrangement while it moves.
    #[default]
    Departing,
}

/// Which row receives spare height first within one column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ColumnFocus {
    /// This zero-based row belongs to the focused cell.
    Row(usize),
    /// The focus ring sits outside this column.
    Outside,
}

/// Whether one demand sync belongs to the frame being drawn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DemandSync {
    /// A scan independent of the current render.
    Independent,
    /// Demands measured at the widths this render will draw.
    CurrentRender,
}

/// Whether the current measurement changes the requested rows or tile identities.
pub(super) enum RowDemandChange {
    Unchanged,
    Changed,
}

impl RowDemandChange {
    pub(super) fn between<Id: PartialEq>(
        previous: &TileDemands<Id>,
        current: &TileDemands<Id>,
    ) -> Self {
        if previous.summary == current.summary && previous.groups == current.groups {
            Self::Unchanged
        } else {
            Self::Changed
        }
    }
}

/// What one arrangement is drawn at: the rows every cell is holding
/// and which of them has the focus ring.
///
/// `DemandSync::Independent` retains `HeldCellLayout::rows` until another cell needs room.
/// `DemandSync::CurrentRender` with `RowDemandChange::Changed` replaces those row demands before
/// layout. `HeldCellLayout` owns this policy so [`shares`](super::allocation::shares) stays a pure
/// function of its input.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct HeldCellLayout {
    /// Row demands each cell holds, in cell order and the summary's first.
    pub(super) rows:          Vec<CellRowDemand>,
    /// Which cell holds the ring, or whether that slot is outside the layout.
    pub(super) focused:       FocusLocation,
    /// Columns the summary asks to reach across, its own included; see
    /// [`summary_span`](super::layout::summary_span). Held with the rows so the summary widening
    /// or narrowing is a change the grid travels through like any other.
    pub(super) summary_span:  usize,
    /// Positions occupied by the summary, including its first.
    pub(super) summary_depth: usize,
}

impl HeldCellLayout {
    /// Whether changing from `other` needs the grid's resize motion.
    pub(super) fn needs_motion_from(&self, other: &Self) -> bool {
        self.focused != other.focused
            || self.summary_span != other.summary_span
            || self.summary_depth != other.summary_depth
            || self.rows.len() != other.rows.len()
            || self
                .rows
                .iter()
                .zip(&other.rows)
                .any(|(current, held)| current.stepped_framed() != held.stepped_framed())
    }
}

/// What each slot draws and the cell it sits at, the summary first.
pub(super) fn cells<Id: Clone>(slots: &[Slot<Id>]) -> Vec<(TileContent<Id>, usize)> {
    let mut out = vec![(TileContent::Summary, TABLE_CELL)];
    out.extend(slots.iter().enumerate().map(|(position, slot)| {
        let index = position + TABLE_CELL + 1;
        (content_of(slot, index), index)
    }));
    out
}

/// The cell `slot` sits at, or `None` when this arrangement has no such
/// slot.
pub(super) fn cell_of<Id: Eq>(slots: &[Slot<Id>], slot: &Slot<Id>) -> Option<usize> {
    slots
        .iter()
        .position(|held| held == slot)
        .map(|position| position + TABLE_CELL + 1)
}

/// What a slot draws once it knows the cell it landed at.
pub(super) fn content_of<Id: Clone>(slot: &Slot<Id>, index: usize) -> TileContent<Id> {
    match slot {
        Slot::Group(id) => TileContent::Group(id.clone()),
        Slot::Empty(_) => TileContent::Empty(index),
    }
}

/// Whether `taken` is a command landing in the empty cell `held` stood
/// for, which is the one way a slot changes identity without moving.
pub(super) const fn claims<Id>(held: &Slot<Id>, taken: &Slot<Id>) -> bool {
    matches!((held, taken), (Slot::Empty(_), Slot::Group(_)))
}

/// Every slot either arrangement holds, in the order each end gives it.
pub(super) fn union<Id: Clone + Eq>(from: &[Slot<Id>], to: &[Slot<Id>]) -> Vec<Slot<Id>> {
    let mut out = to.to_vec();
    for (position, slot) in from.iter().enumerate() {
        if to.contains(slot) {
            continue;
        }
        let next_survivor = from[position.saturating_add(1)..]
            .iter()
            .find(|following| to.contains(following));
        let insert_at = next_survivor
            .and_then(|following| out.iter().position(|placed| placed == following))
            .unwrap_or(out.len());
        out.insert(insert_at, slot.clone());
    }
    out
}

/// How many of `ids` can stand where they already do: the longest run
/// from the front of the order that `arrangement` already holds in that
/// same order.
///
/// Everything past that run is what closes. Counted from the front
/// rather than as the largest run that happens to be in order anywhere,
/// because a cell only ever comes back in *behind* the cells still
/// standing -- so the ones that stay have to be the front of the order
/// rather than a stretch out of its middle.
pub(super) fn kept<Id: Eq>(arrangement: &[Slot<Id>], ids: &[Id]) -> usize {
    let mut cursor = 0_usize;
    let mut count = 0_usize;
    for id in ids {
        let Some(offset) = arrangement
            .iter()
            .skip(cursor)
            .position(|slot| matches!(slot, Slot::Group(held) if held == id))
        else {
            break;
        };
        cursor = cursor.saturating_add(offset).saturating_add(1);
        count = count.saturating_add(1);
    }
    count
}

/// The column and row a one-based cell index falls in.
pub(super) fn position(widths: &[usize], index: usize) -> Option<(usize, usize)> {
    let mut seen = 0;
    for (column, &height) in widths.iter().enumerate() {
        if index <= seen + height {
            return Some((column, index.checked_sub(seen + 1)?));
        }
        seen += height;
    }
    None
}

/// The column `slot` occupies in one queued arrangement.
fn queued_slot_column<Id: Eq>(
    slots: &[Slot<Id>],
    depth: usize,
    slot: &Slot<Id>,
    growth: TileGrowth,
    area: Rect,
    settings: &TileSettings,
) -> Option<usize> {
    let cell = slots
        .iter()
        .position(|held| held == slot)?
        .saturating_add(TABLE_CELL + 1);
    let ColumnPlan::Columns(layout) =
        column_layout(slots.len() + TABLE_CELL, depth, growth, area, settings)
    else {
        return None;
    };
    let laid_out = cell.saturating_add(layout.columns.summary_depth.saturating_sub(1));
    position(&layout.columns.widths, laid_out).map(|(column, _)| column)
}

/// Cell order and summary depth at one step of a queued movement.
pub(super) struct QueuedArrangement<'a, Id> {
    pub(super) slots:         &'a [Slot<Id>],
    pub(super) summary_depth: usize,
}

/// Whether every surviving cell uses one of its two endpoint columns.
pub(super) fn queued_cells_keep_endpoint_columns<Id: Eq>(
    start: &QueuedArrangement<'_, Id>,
    finish: &QueuedArrangement<'_, Id>,
    candidate: &QueuedArrangement<'_, Id>,
    growth: TileGrowth,
    area: Rect,
    settings: &TileSettings,
) -> bool {
    start
        .slots
        .iter()
        .filter(|slot| finish.slots.contains(slot) && candidate.slots.contains(slot))
        .all(|slot| {
            let start_column = queued_slot_column(
                start.slots,
                start.summary_depth,
                slot,
                growth,
                area,
                settings,
            );
            let final_column = queued_slot_column(
                finish.slots,
                finish.summary_depth,
                slot,
                growth,
                area,
                settings,
            );
            let candidate_column = queued_slot_column(
                candidate.slots,
                candidate.summary_depth,
                slot,
                growth,
                area,
                settings,
            );
            candidate_column == start_column || candidate_column == final_column
        })
}
