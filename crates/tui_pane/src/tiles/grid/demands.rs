use super::Debug;
/// One group's claim on its column.
///
/// `Id` is the app's identity for the group a cell draws. The grid only
/// clones and compares it, so any stable key the app already holds will
/// do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileDemand<Id> {
    /// The group's identity, the same key [`TileContent::Group`](crate::TileContent::Group)
    /// carries.
    pub id:   Id,
    /// Rows the group's cell would draw given all the room it wants,
    /// which the app measures at the width
    /// [`TileCells::demands`](crate::TileCells::demands) is handed for
    /// the cell.
    pub rows: usize,
}

/// What every cell of the grid is asking for, as one scan left it.
///
/// Kept apart from the arrangement on purpose. The arrangement says
/// which slot sits at which cell and is what the motion is keyed on;
/// this says how much of its column each of them takes, and the two
/// change on different events -- a command starting or finishing moves
/// cells, while a command merely running more invocations than before
/// only re-divides the column it is already in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileDemands<Id> {
    /// Rows the summary cell would draw given all the room it wants.
    pub summary:       usize,
    /// Preferred content width of the summary under
    /// [`TileGrowth::widen_summary`](crate::TileGrowth::widen_summary). Zero requests only its
    /// own column. Measured layouts retain the selected allocation after a preferred width
    /// change when every cached candidate still requests the same rows.
    /// Changed row requests, terminal geometry or tile membership select a new allocation.
    pub summary_width: u16,
    /// Every group that gets a cell, in cell order.
    pub groups:        Vec<TileDemand<Id>>,
}

impl<Id> Default for TileDemands<Id> {
    fn default() -> Self {
        Self {
            summary:       0,
            summary_width: 0,
            groups:        Vec::new(),
        }
    }
}

impl<Id: Clone + Eq + Debug> TileDemands<Id> {
    /// Rows the cell drawing `id` is asking for, and none when this
    /// scan no longer carries that group.
    pub(in crate::tiles) fn rows_for(&self, id: &Id) -> usize {
        self.groups
            .iter()
            .find(|demand| &demand.id == id)
            .map_or(0, |demand| demand.rows)
    }

    /// The identity of every group that gets a cell, in order.
    pub(super) fn ids(&self) -> Vec<Id> {
        self.groups.iter().map(|demand| demand.id.clone()).collect()
    }

    /// Whether this scan still carries `id`, which is what tells a cell
    /// closing to make way for the order apart from one whose command
    /// has finished.
    pub(super) fn holds(&self, id: &Id) -> bool {
        self.groups.iter().any(|demand| &demand.id == id)
    }
}
