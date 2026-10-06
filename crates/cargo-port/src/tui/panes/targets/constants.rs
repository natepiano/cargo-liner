/// Header text for the Source column — also defines the column's
/// minimum width so the header never gets truncated.
pub(super) const SOURCE_HEADER: &str = "Source";
/// Leaf index of the targets table box in the pane's tree.
pub(super) const TABLE_BOX: usize = 0;
/// Header text for Target columns; the main table adds its leading pad.
pub(super) const TARGET_HEADER: &str = "Target";
/// Target rows render one leading space before the target name.
pub(super) const TARGET_LEADING_PAD: usize = 1;
/// Inter-column gap used by the `ratatui` table.
pub(super) const TARGET_TABLE_COLUMN_SPACING: u16 = 1;
/// Number of 1-column gaps between Target/Source/Kind.
pub(super) const TARGET_TABLE_GAP_COUNT: usize = 2;
