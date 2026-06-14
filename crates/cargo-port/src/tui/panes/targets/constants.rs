/// Floor on the table's data rows in degenerate heights: the Running
/// box's rendered window shrinks before the table drops below this.
pub(super) const MIN_TABLE_ROWS: u16 = 3;
/// Leaf index of the Running box; its `Placed` entry exists only while
/// anything runs.
pub(super) const RUNNING_BOX: usize = 1;
/// Ceiling on the Running box: it grows upward to at most this percent of
/// the pane's inner height, so the table keeps the rest.
pub(super) const RUNNING_CAP_PERCENT: u16 = 80;
/// Chrome rows the Running box reserves: the divider rule plus the column
/// header.
pub(super) const RUNNING_CHROME: u16 = 2;
/// Header text for the Source column — also defines the column's
/// minimum width so the header never gets truncated.
pub(super) const SOURCE_HEADER: &str = "Source";
/// Leaf index of the targets table box in the pane's tree.
pub(super) const TABLE_BOX: usize = 0;
/// Chrome rows the table box reserves: the ratatui `Table` header row.
pub(super) const TABLE_CHROME: u16 = 1;
/// Footer row the table box reserves while the Running box sits below:
/// blank when every table row is visible, the table's pager rule when it
/// scrolls.
pub(super) const TABLE_FOOTER: u16 = 1;
/// Header text for Target columns; the main table adds its leading pad.
pub(super) const TARGET_HEADER: &str = "Target";
/// Target rows render one leading space before the target name.
pub(super) const TARGET_LEADING_PAD: usize = 1;
/// Inter-column gap used by the `ratatui` table.
pub(super) const TARGET_TABLE_COLUMN_SPACING: u16 = 1;
/// Number of 1-column gaps between Target/Source/Kind.
pub(super) const TARGET_TABLE_GAP_COUNT: usize = 2;

/// Width of the CPU column: `476%` — a busy multi-threaded process can
/// exceed 100.
pub(super) const CPU_COL_WIDTH: usize = 4;
/// Width of the MEM column: `999.9 MiB`.
pub(super) const MEM_COL_WIDTH: usize = 9;
/// Width consumed by one outline depth: two leading spaces.
pub(super) const OUTLINE_DEPTH_INDENT_WIDTH: usize = 2;
/// Width consumed by an outline glyph plus its following gap.
pub(super) const OUTLINE_PARENT_PREFIX_WIDTH: usize = 2;
/// Width consumed by a one-digit child-count suffix, such as ` (9)`.
pub(super) const OUTLINE_SINGLE_DIGIT_SUFFIX_WIDTH: usize = 4;
/// Width of the PID column: Linux PIDs reach seven digits.
pub(super) const PID_COL_WIDTH: usize = 7;
/// Width of the Profile column: the widest profile label (`release`).
pub(super) const PROFILE_COL_WIDTH: usize = 7;
/// Cap on the Target column width so a single long target name can't
/// crowd out the metric columns. Overflow truncates with an ellipsis.
pub(super) const TARGET_COL_MAX: usize = 24;
