//! Constants for the tile grid: its fixed cell numbering, the
//! defaults its sizes and timings start from, and the readout every
//! cell carries.

// grid shape
/// The cell holding the summary. Cells are numbered from one and fill
/// column by column, so the summary is always the first.
pub const TABLE_CELL: usize = 1;
/// Floor on `tiles.initial_rows`. At one there is no single-column
/// stretch at all: the grid arranges itself into a square from the
/// second cell on.
pub const MIN_INITIAL_ROWS: usize = 1;

// cell sizes
/// Rows one cell standing alone needs: a border line, a line of
/// content, and a border line. A cell with a neighbour below costs one
/// less, because the two share that line.
pub(super) const MIN_TILE_HEIGHT: u16 = 3;
/// Columns one cell standing alone needs, its two border lines
/// included. A cell with a neighbour to its right costs one less.
pub(super) const MIN_TILE_WIDTH: u16 = 8;
/// Rows a cell spends on its own border, which is what its contents
/// have to be given on top of. Two, though neighbours share one of
/// them, so a stacked cell costs less than this and the division is a
/// little conservative rather than a little short.
pub(super) const TILE_BORDER_ROWS: u16 = 2;
/// Rows of content one step of demand is worth. A cell asks for its
/// content rounded up to a whole number of these, so a column
/// re-divides on the move from a quiet cell to a busy one rather than
/// on every scan that added a row or rewrapped a command line.
pub(super) const TILE_DEMAND_STEP: usize = 3;

// cell readout
/// Kept between a cell's left border and the number it carries, so the
/// number is not flush against the line.
pub(super) const TILE_NUMBER_INDENT: &str = " ";
/// Ahead of what a cell's contents ask for, in the readout along the
/// foot of every cell.
pub const TILE_ROWS_CONTENT_LABEL: &str = "content rows: ";
/// Ahead of the cell's own size in the same readout, written as rows
/// over columns.
pub(super) const TILE_ROWS_CELL_LABEL: &str = "  r/c: ";
/// Between those two numbers.
pub(super) const TILE_ROWS_CELL_SEPARATOR: &str = "/";
/// Ahead of the width the demand was measured at, which is written only
/// where it is not the width the cell was drawn at. The two agreeing is
/// the ordinary case and says nothing; the two disagreeing is the one
/// way the counts can differ without either being wrong on its own
/// terms, and is worth the room it takes.
pub(super) const TILE_ROWS_WIDTH_LABEL: &str = " @ ";
/// Kept between that readout and the cell's right border, so it is not
/// flush against the line.
pub(super) const TILE_ROWS_RIGHT_INSET: u16 = 1;
/// Rows the readout takes: it is one line along the foot of the cell.
pub(super) const TILE_ROWS_READOUT_HEIGHT: u16 = 1;

// motion
/// Fixed-point scale a transition's progress is measured on, so the
/// animation needs no floating point.
pub(super) const PROGRESS_SCALE: u32 = 1000;
/// How long one change to the grid takes, however many single-cell
/// steps it propagates through: one step takes all of it, and a longer
/// ripple divides it up between them.
pub(super) const TILE_ANIMATION_MILLIS: u64 = 720;
/// Floor on one step of a ripple, so a long one still reads as cells
/// moving rather than flickering past.
pub(super) const MIN_STEP_MILLIS: u64 = 60;
/// How long the resize a step of the focus ring asks for takes.
///
/// Far shorter than [`TILE_ANIMATION_MILLIS`], because the two are
/// different events: a cell opening or closing happens on its own and
/// wants watching, while this one is a key the developer just pressed
/// and is already pressing again. At the full travel, holding an arrow
/// down would leave the grid still settling from the first press.
pub(super) const FOCUS_ANIMATION_MILLIS: u64 = 140;
/// Steps the grid queues before it gives up propagating and settles the
/// rest in one move. A whole test suite finishing at once would take
/// longer to walk through cell by cell than anyone would watch.
pub(super) const MAX_PENDING_STEPS: usize = 64;
