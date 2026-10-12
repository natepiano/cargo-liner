//! Constants for the tile grid: its fixed cell numbering, the
//! defaults its sizes and timings start from, and the readout every
//! cell carries.

#[cfg(test)]
use ratatui::layout::Rect;

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
/// Unused content rows the summary keeps before `auto` returns from
/// the summary alone to command cells.
pub(super) const AUTO_VIEW_RETURN_MARGIN: u16 = 3;
/// Divisor that makes half of one summary position the largest shortfall
/// the summary may take from its column mates without adding a position.
pub(super) const SUMMARY_SLOT_SHORTFALL_DIVISOR: u16 = 2;

// cell readout
/// Kept between the summary foot and the rows readout.
pub(super) const TILE_FOOT_GAP: u16 = 2;
/// Kept between the summary foot and the cell's left border.
pub(super) const TILE_FOOT_LEFT_INSET: u16 = 1;
/// Kept between a cell's left border and the number it carries, so the
/// number is not flush against the line.
pub(super) const TILE_NUMBER_INDENT: &str = " ";
/// Ahead of the cell's own size in the same readout, written as rows
/// over columns.
pub(super) const TILE_ROWS_CELL_LABEL: &str = "  r/c: ";
/// Between those two numbers.
pub(super) const TILE_ROWS_CELL_SEPARATOR: &str = "/";
/// Ahead of what a cell's contents ask for, in the readout along the
/// foot of every cell.
pub const TILE_ROWS_CONTENT_LABEL: &str = "content rows: ";
/// Rows the readout takes: it is one line along the foot of the cell.
pub(super) const TILE_ROWS_READOUT_HEIGHT: u16 = 1;
/// Kept between that readout and the cell's right border, so it is not
/// flush against the line.
pub(super) const TILE_ROWS_RIGHT_INSET: u16 = 1;
/// Ahead of the width the demand was measured at, which is written only
/// where it is not the width the cell was drawn at. The two agreeing is
/// the ordinary case and says nothing; the two disagreeing is the one
/// way the counts can differ without either being wrong on its own
/// terms, and is worth the room it takes.
pub(super) const TILE_ROWS_WIDTH_LABEL: &str = " @ ";

// motion
/// Fixed-point scale a transition's progress is measured on, so the
/// animation needs no floating point.
pub(super) const PROGRESS_SCALE: u32 = 1000;
/// One row toward the destination from the last whole or interpolated height.
pub(super) const MOVING_HEIGHT_PROBE_STEP: u16 = 1;
/// Command count in the column-foot render regressions.
#[cfg(test)]
pub(super) const COLUMN_FOOT_TEST_GROUP_COUNT: u32 = 5;
/// Summary demand in the column-foot render regressions.
#[cfg(test)]
pub(super) const COLUMN_FOOT_TEST_SUMMARY_ROWS: u16 = 18;
/// Summary demand in the column-foot key regression.
#[cfg(test)]
pub(super) const COLUMN_FOOT_TEST_KEY_SUMMARY_ROWS: usize = 13;
/// App-level framed-height floor in the column-foot render regressions.
#[cfg(test)]
pub(super) const COLUMN_FOOT_TEST_MIN_HEIGHT: u16 = 6;
/// Smallest complete command body in the column-foot startup regression.
#[cfg(test)]
pub(super) const COLUMN_FOOT_TEST_BODY_STEP: u16 = 3;
/// Grid area below the status line in a 200 by 50 terminal.
#[cfg(test)]
pub(super) const COLUMN_FOOT_TEST_AREA: Rect = Rect::new(0, 0, 200, 49);
/// Content demands in the one-column spare-row motion regression.
#[cfg(test)]
pub(super) const LENDING_TEST_DEMANDS: [u16; 7] = [7, 1, 10, 1, 1, 9, 1];
/// Painted content rows in the one-column spare-row motion regression.
#[cfg(test)]
pub(super) const LENDING_TEST_DRAWN: [u16; 7] = [7, 1, 3, 1, 1, 9, 1];
/// Command whose incomplete body step is skipped by the height search.
#[cfg(test)]
pub(super) const LENDING_TEST_WHOLE_GROUP: u32 = 2;
/// Command below the whole piece whose content is still being cut.
#[cfg(test)]
pub(super) const LENDING_TEST_CUT_GROUP: u32 = 5;
/// Content allocation that ends inside an incomplete command step.
#[cfg(test)]
pub(super) const LENDING_TEST_GAP_HEIGHT: u16 = 4;
/// Keep the spare-row regression in one column through depth changes.
#[cfg(test)]
pub(super) const LENDING_TEST_INITIAL_ROWS: usize = 12;
/// The whole command's framed heights at the removal's two endpoints.
#[cfg(test)]
pub(super) const LENDING_TEST_ENDPOINT_HEIGHTS: [u16; 2] = [8, 9];
/// Title used by the spare-row motion fixture.
#[cfg(test)]
pub(super) const LENDING_TEST_SUMMARY_TITLE: &str = "summary";
/// Intervals sampled across one key in the visible-column-motion regression.
#[cfg(test)]
pub(super) const SLIDING_TEST_INTERVALS: u32 = 61;
/// Keep additions and removals in one column in the slide regression.
#[cfg(test)]
pub(super) const SLIDING_TEST_INITIAL_ROWS: usize = 12;
/// Summary and command rows in the one-column slide regression.
#[cfg(test)]
pub(super) const SLIDING_TEST_ROWS: [u16; 6] = [13, 1, 10, 1, 1, 9];
/// Geometry changes required before an opening or closing key finishes.
#[cfg(test)]
pub(super) const SLIDING_TEST_MIN_CHANGES: usize = 3;
/// Summary and command demands in the paired-entry motion regression.
#[cfg(test)]
pub(super) const SUMMARY_STEP_TEST_ROWS: [u16; 4] = [9, 8, 8, 9];
/// Rows in each complete summary entry.
#[cfg(test)]
pub(super) const SUMMARY_STEP_TEST_ENTRY_ROWS: u16 = 2;
/// Header rows above the summary entries.
#[cfg(test)]
pub(super) const SUMMARY_STEP_TEST_HEADER_ROWS: u16 = 1;
/// Command allocation with one optional ancestry row above its table.
#[cfg(test)]
pub(super) const SUMMARY_STEP_TEST_OPTIONAL_HEIGHTS: [u16; 3] = [5, 4, 5];
/// Piece count reached by adding empty cells to the paired-entry column.
#[cfg(test)]
pub(super) const SUMMARY_STEP_TEST_PIECES: usize = 9;
/// Project counts before and after a summary grows during motion.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_ENTRIES: [u16; 2] = [3, 4];
/// Rows in a summary project with its separator present.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_PROJECT_ROWS: u16 = 3;
/// Complete rows in each command beside the changing summary.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_COMMAND_ROWS: u16 = 9;
/// Commands present before a key opens another cell.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_GROUPS: u32 = 3;
/// Empty cells opened before the removal regression.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_EMPTY_CELLS: usize = 2;
/// Early, late and final moving-frame content changes.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_PROGRESS: [u32; 3] = [250, 750, 983];
/// Painted row marker in the content-change buffer assertions.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_ROW: &str = "x";
/// Column spans before a project arrives during a slot opening.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_SPARE_SPANS: [u16; 4] = [12, 12, 12, 12];
/// Filled source spans with room for two empty cells and three cut commands.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_SOURCE_SPANS: [u16; 6] = [12, 10, 10, 6, 5, 5];
/// Positions per column in the content-driven column-change regression.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_COLUMN_ROWS: usize = 4;
/// Project count whose growth changes the summary's column layout.
#[cfg(test)]
pub(super) const CONTENT_CHANGE_TEST_COLUMN_ENTRIES: u16 = 12;
/// Framed donor and recipient heights in the invalid settled allocation.
#[cfg(test)]
pub(super) const SETTLED_SPARE_TEST_HEIGHTS: [u16; 2] = [14, 10];
/// Column spans when a whole summary releases an entry beside cut commands.
#[cfg(test)]
pub(super) const DEPARTED_ENTRY_TEST_SPANS: [u16; 7] = [16, 5, 5, 5, 5, 7, 5];
/// Column spans with an empty departing cell above its height floor.
#[cfg(test)]
pub(super) const EMPTY_DONOR_TEST_SPANS: [u16; 5] = [11, 10, 10, 10, 7];
/// Summary and command spans before a compact entry leaves during an opening.
#[cfg(test)]
pub(super) const SUMMARY_DONOR_TEST_SPANS: [u16; 5] = [16, 8, 8, 8, 8];
/// Painted- and retained-row calls made by the seven-cell moving-frame regression.
#[cfg(test)]
pub(super) const SEVEN_CELL_MOVING_ROW_PROBE_COUNT: usize = 29;
/// Painted-row calls made by one changed seven-cell settled frame.
#[cfg(test)]
pub(super) const SEVEN_CELL_SETTLED_DRAWN_PROBE_COUNT: usize = 8;
/// Retained-row calls made by one changed seven-cell settled frame.
#[cfg(test)]
pub(super) const SEVEN_CELL_SETTLED_KEPT_PROBE_COUNT: usize = 7;
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

/// Shared spans of the resident-empty column before an opening and departure.
#[cfg(test)]
pub(super) const RESIDENT_EMPTY_TEST_SPANS: [u16; 8] = [11, 5, 5, 5, 5, 7, 5, 5];
/// Shared spans of the summary and command before another command opens.
#[cfg(test)]
pub(super) const COMMAND_OPENING_TEST_SPANS: [u16; 2] = [24, 24];
/// Shared spans before a floor-height empty cell departs.
#[cfg(test)]
pub(super) const EMPTY_DEPARTURE_TEST_SPANS: [u16; 5] = [11, 5, 10, 11, 11];

/// Source spans of a command lending two rows before the resident trace anchor.
#[cfg(test)]
pub(super) const RESIDENT_EMPTY_TEST_START_SPANS: [u16; 8] = [11, 5, 7, 5, 5, 5, 5, 5];
