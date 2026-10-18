//! Stable identities for production-role session tiles and each machine's
//! unassigned pool. The grid also owns its summary and manually opened empty cells.

/// The agent a cell draws: its machine, its process id there, and when
/// that process started, so a pid handed out again is a new cell.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AgentCell {
    /// The machine's heading in the summary.
    pub(crate) machine:         String,
    /// The agent's process id on that machine.
    pub(crate) pid:             u32,
    /// The process's own birth in unix seconds, apart from the session
    /// age shown; the session start for an older remote probe.
    pub(crate) process_started: u64,
}

/// A session tile or the stable pool of unassigned sessions on one machine.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TileKey {
    /// A root with a production role.
    Agent(AgentCell),
    /// All roots without a production role on this machine.
    Unassigned { machine: String },
}

/// The grid of cells, the summary first.
pub(crate) type TileGrid = tui_pane::TileGrid<TileKey>;
/// What one cell of the grid is showing.
pub(crate) type TileContent = tui_pane::TileContent<TileKey>;
/// `TileDemands` carries the summary row count and width and the group row
/// counts supplied by `TileCells::demands`.
pub(crate) type TileDemands = tui_pane::TileDemands<TileKey>;
