//! The tile grid, keyed by the agent each cell draws.
//!
//! The grid itself is [`tui_pane::TileGrid`]. Besides the summary, every
//! agent someone can talk to claims a cell of its own, named by
//! [`AgentCell`]; any other cell is an empty one opened with `+`.

/// The agent a cell draws: its machine, its process id there, and when
/// that process started, so a pid handed out again is a new cell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentCell {
    /// The machine's heading in the summary.
    pub(crate) machine: String,
    /// The agent's process id on that machine.
    pub(crate) pid:     u32,
    /// When the agent's process started, in unix seconds.
    pub(crate) started: u64,
}

/// The grid of cells, the summary first.
pub(crate) type TileGrid = tui_pane::TileGrid<AgentCell>;
/// What one cell of the grid is showing.
pub(crate) type TileContent = tui_pane::TileContent<AgentCell>;
/// What every cell of the grid is asking for.
pub(crate) type TileDemands = tui_pane::TileDemands<AgentCell>;
