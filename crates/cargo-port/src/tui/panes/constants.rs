//! Panes-rendering constants.
//!
//! Per `~/rust/nate_style/rust/no-magic-values.md`: directory
//! modules own their constants in their own `constants.rs`, not
//! the parent's. These are panes-rendering-specific (tree-row
//! prefixes and Tests detail-row labels), so they live here, not
//! in `tui::constants`.
//!
//! Visibility: `pub(super)` — visible to `panes/*` siblings only.
//! No `pub use` re-export at `panes/mod.rs`. The row prefixes are
//! consumed by the renderer in `panes::project_list` and the
//! column-fit-width math in `panes::widths`; the Tests row labels
//! are shared by `panes::pane_data` (row building) and
//! `panes::package` (rendering).

// `FIT_TEXT_ELLIPSIS` and `PULL_REQUEST_MIN_TITLE_WIDTH` bound Git pane text fitting.

use std::ops::Range;

use ratatui::style::Style;

use crate::project::MemberGroup;
use crate::project::WorktreeHealth;
use crate::tui::columns;
use crate::tui::columns::RowLifecycle;

pub(super) const FIT_TEXT_ELLIPSIS: &str = "...";
pub(super) const PULL_REQUEST_MIN_TITLE_WIDTH: usize = 8;

// Tree prefixes such as `PREFIX_ROOT_EXPANDED` and `TreeRowExpansion::prefix` provide the single
// source of text for width calculation and rendering.

const DELETED_DISK_TEXT: &str = "0.0";

/// Disk-cell suffix on a `WorktreeHealth::Broken` row; measured by
/// `panes::widths` and drawn by `panes::project_list`.
pub(in crate::tui) const BROKEN_WORKTREE_DISK_SUFFIX: &str = " [broken]";

/// Every Disk-cell presentation. Rendering and fitting read the same text,
/// suffix and suffix style, so adding a presentation also defines its width.
pub(super) enum DiskCell {
    Size(String),
    Broken(String),
    DeleteMarked,
}

impl DiskCell {
    pub(super) fn for_row(size: String, lifecycle: RowLifecycle, health: WorktreeHealth) -> Self {
        match (lifecycle, health) {
            (RowLifecycle::Deleted, WorktreeHealth::Normal | WorktreeHealth::Broken) => {
                Self::DeleteMarked
            },
            (RowLifecycle::Present, WorktreeHealth::Normal) => Self::Size(size),
            (RowLifecycle::Present, WorktreeHealth::Broken) => Self::Broken(size),
        }
    }

    pub(super) fn text(&self) -> &str {
        match self {
            Self::Size(size) | Self::Broken(size) => size,
            Self::DeleteMarked => DELETED_DISK_TEXT,
        }
    }

    pub(super) const fn suffix(&self) -> Option<&'static str> {
        match self {
            Self::Size(_) => None,
            Self::Broken(_) => Some(BROKEN_WORKTREE_DISK_SUFFIX),
            Self::DeleteMarked => Some(DISMISS_SUFFIX),
        }
    }

    pub(super) fn suffix_style(&self) -> Option<Style> {
        match self {
            Self::Size(_) => None,
            Self::Broken(_) => Some(
                Style::default()
                    .fg(tui_pane::on_error_color())
                    .bg(tui_pane::error_color()),
            ),
            Self::DeleteMarked => Some(Style::default().fg(tui_pane::label_color())),
        }
    }

    pub(super) fn width(&self) -> usize {
        columns::display_width(self.text()) + self.suffix().map_or(0, columns::display_width)
    }
}

/// Project-list label shared by vendored rows and their fitted widths.
pub(super) fn vendored_row_label(package_name: &str) -> String { format!("{package_name} (v)") }

pub(super) const PREFIX_ROOT_EXPANDED: &str = "▼ ";
pub(in crate::tui) const PREFIX_ROOT_COLLAPSED: &str = "▶ ";
pub(in crate::tui) const PREFIX_ROOT_LEAF: &str = "  ";
pub(super) const TREE_PREFIX_BLANK: &str = "  ";
pub(super) const TREE_PREFIX_CONTINUATION: &str = "│ ";
/// Complete leaf ending, also exposed to the existing width tests.
pub(in crate::tui) const PREFIX_WORKTREE_FLAT: &str = "└── ";

/// Expansion state of a non-root row's tree prefix.
#[derive(Clone, Copy)]
pub(super) enum TreeRowExpansion {
    Expanded,
    Collapsed,
    Leaf,
}

impl TreeRowExpansion {
    /// The complete branch and expansion marker; fit measures read this
    /// same text rather than maintaining a second set of prefix strings.
    pub(super) const fn prefix(self, has_later_sibling: bool) -> &'static str {
        match (self, has_later_sibling) {
            (Self::Expanded, true) => "├─▼ ",
            (Self::Expanded, false) => "└─▼ ",
            (Self::Collapsed, true) => "├─▶ ",
            (Self::Collapsed, false) => "└─▶ ",
            (Self::Leaf, true) => "├── ",
            (Self::Leaf, false) => PREFIX_WORKTREE_FLAT,
        }
    }
}

/// Non-root row depth, counting the ancestors between its root and ending.
/// Both the fit pass and the renderer use this depth for ancestor segments.
#[derive(Clone, Copy)]
pub(super) struct TreeRowDepth(usize);

impl TreeRowDepth {
    pub(super) const fn root_child() -> Self { Self(0) }

    pub(super) const fn child(self) -> Self { Self(self.0 + 1) }

    pub(super) const fn member_of(self, group: &MemberGroup) -> Self {
        if group.is_named() { self.child() } else { self }
    }

    /// A non-root lineage includes its root and the row itself.
    pub(super) const fn from_lineage_length(length: usize) -> Self { Self(length - 2) }

    /// Indices of the lineage segments between the root and the row.
    pub(super) const fn ancestor_indices(self) -> Range<usize> { 1..self.0 + 1 }
}

/// Reserve the widest drawn ending and ancestor segments, so fitting is
/// independent of expansion state and sibling order.
pub(super) fn tree_prefix_width(depth: TreeRowDepth) -> usize {
    let dw = columns::display_width;
    let ending_width = [
        TreeRowExpansion::Expanded,
        TreeRowExpansion::Collapsed,
        TreeRowExpansion::Leaf,
    ]
    .into_iter()
    .flat_map(|state| [state.prefix(false), state.prefix(true)])
    .map(dw)
    .max()
    .unwrap_or_default();
    depth.0 * dw(TREE_PREFIX_BLANK).max(dw(TREE_PREFIX_CONTINUATION)) + ending_width
}

// `TESTS_IGNORED_LABEL` and `TESTS_TOTAL_LABEL` identify Tests detail rows.

/// Label of the `(ignored)` annotation row in the Tests section. Shared
/// with the renderer, which dims this row's value (the count is a
/// registered-but-skipped doctest tally, not a runnable test).
pub(super) const TESTS_IGNORED_LABEL: &str = "(ignored)";
/// Label of the runnable-total row in the Tests section — intentionally
/// blank. The Languages pane shows its grand total as an unlabelled bold
/// bottom value; the Tests total matches, so the renderer keys off this
/// empty label to render the value in bold accent color.
pub(super) const TESTS_TOTAL_LABEL: &str = "";

// `CI_BRANCH_*`, `CI_COMMIT_*` and `CI_JOB_LABEL_*` bound CI pane columns; `OTHER_JOBS_HEADER`
// labels remaining jobs.
pub(super) const CI_BRANCH_LONG_MIN_WIDTH: usize = 16;
pub(super) const CI_BRANCH_MIN_WIDTH: usize = 6;
pub(super) const CI_COMMIT_LONG_MIN_WIDTH: usize = 22;
pub(super) const CI_COMMIT_MIN_WIDTH: usize = 7;
pub(super) const CI_JOB_LABEL_MAX_WIDTH: usize = 16;
pub(super) const CI_JOB_LABEL_MIN_WIDTH: usize = 8;
pub(super) const CI_STATUS_GAP_WIDTH: usize = 1;
pub(super) const OTHER_JOBS_HEADER: &str = "Other";

// `CPU_BAR_WIDTH` and the `CPU_*_ROWS` constants size CPU pane bars and pinned rows.
pub(super) const CPU_BAR_WIDTH: usize = 10;
/// Breakdown rows pinned below the cores band: System, User, Idle.
pub(super) const CPU_BREAKDOWN_ROWS: usize = 3;
pub(super) const CPU_CONTENT_WIDTH: u16 = 17;
/// GPU rows pinned below the breakdown. macOS exposes the full set (core
/// count, device, renderer, tiler); other platforms expose only the single
/// device row.
#[cfg(target_os = "macos")]
pub(super) const CPU_GPU_ROWS: usize = 4;
#[cfg(not(target_os = "macos"))]
pub(super) const CPU_GPU_ROWS: usize = 1;
pub(in crate::tui) const CPU_PANE_WIDTH: u16 = CPU_CONTENT_WIDTH + 2;
/// Pinned head rows above the scrolling cores band: the `Cores` count row and
/// the `Device` utilization row.
pub(super) const CPU_PINNED_HEAD_ROWS: usize = 2;
/// Pixel height of every inner row except the scrolling cores band: the two
/// pinned head rows, the two separator rules, the three breakdown rows, and
/// the GPU rows. The cores band gets `inner.height - CPU_STATIC_INNER_HEIGHT`.
#[cfg(target_os = "macos")]
pub(super) const CPU_STATIC_INNER_HEIGHT: u16 = 11;
#[cfg(not(target_os = "macos"))]
pub(super) const CPU_STATIC_INNER_HEIGHT: u16 = 8;
/// Shown in GPU rows when the OS exposes no matching value. Kept within
/// `CPU_CONTENT_WIDTH` so it never widens the pane. macOS packs four narrow
/// rows so it uses the compact text; the single-row platforms keep the
/// original spelled-out word.
#[cfg(target_os = "macos")]
pub(super) const GPU_UNAVAILABLE_TEXT: &str = "n/a";
#[cfg(not(target_os = "macos"))]
pub(super) const GPU_UNAVAILABLE_TEXT: &str = "unavailable";

// `DescriptionBlock` uses `NO_DESCRIPTION_AVAILABLE` for empty descriptions.
/// "No description available" placeholder rendered by the Package pane
/// when the source description is empty. Lives here so [`super::DescriptionBlock`]
/// owns both the placeholder text and the row wrapping that consumes it —
/// no caller can substitute a different placeholder and break the sync
/// invariant.
pub(super) const NO_DESCRIPTION_AVAILABLE: &str = "No description available";

// `BRANCH_HEADER` and the remote, pull-request and worktree column constants label Git pane tables.
pub(super) const BRANCH_HEADER: &str = "Branch";
/// Floor for the flexible columns (Remotes URL, Worktrees Name). When the
/// pane is too narrow they truncate with an ellipsis down to this width,
/// then stop — past here there's nothing useful left to shrink, so the
/// line is allowed to clip at the pane edge rather than squeeze further.
pub(super) const MIN_FLEX_COL: usize = 8;
pub(super) const PULL_REQUEST_BRANCH_HEADER: &str = "Branch";
pub(super) const PULL_REQUEST_NUMBER_HEADER: &str = "#";
pub(super) const PULL_REQUEST_STATUS_HEADER: &str = "Status";
pub(super) const PULL_REQUEST_TITLE_HEADER: &str = "Title";
/// The icon column pads to this display width. Emoji render as 2 cells on
/// most terminals; we append a trailing space for separation, giving 3.
pub(super) const REMOTE_ICON_COL: usize = 3;
pub(super) const REMOTES_NAME_HEADER: &str = "Remote";
pub(super) const REMOTES_URL_HEADER: &str = "URL";
pub(super) const SYNC_HEADER: &str = "Sync";
pub(super) const TRACKED_HEADER: &str = "Tracked";
pub(super) const WORKTREES_NAME_HEADER: &str = "Name";

// `LANG_NUM_COL` sizes the Languages pane numeric column.
/// Fixed numeric column width for language stats.
pub(super) const LANG_NUM_COL: u16 = 8;

// `PANE_BORDER_HEIGHT` contributes border rows to pane layout.
/// Rows a bordered pane spends on its top and bottom border. Outer pane height
/// is inner content height plus this.
pub(super) const PANE_BORDER_HEIGHT: u16 = 2;

// The package pane uses `DESCRIPTION_BOX` and `METADATA_BOX` for tree leaves and the following
// title and width constants for its statistics column.
/// Title of the crates.io sub-section rule in the stats column.
pub(super) const CRATES_IO_TITLE: &str = "crates.io";
/// Leaf index of the description box in the package tree.
pub(super) const DESCRIPTION_BOX: usize = 0;
/// Leaf index of the metadata column in the package tree.
pub(super) const METADATA_BOX: usize = 1;
/// Floor on the metadata column's width when the stats column is present.
pub(super) const MIN_METADATA_WIDTH: u16 = 20;
/// Floor on the stats-column label field, so a project with only short
/// labels keeps the same column width. The widest default Structure labels
/// (`proc-macro` / `submodules`) are 10 wide.
pub(super) const MIN_STATS_LABEL_WIDTH: u16 = 10;
pub(super) const STATS_TITLE: &str = "Structure";
/// Title of the Tests sub-section rule in the stats column.
pub(super) const TESTS_TITLE: &str = "Tests";

// `DISMISS_SUFFIX` marks dismissible project-list rows; `TITLE_ELLIPSIS` marks truncated titles.
pub(in crate::tui) const DISMISS_SUFFIX: &str = " [x]";
pub(super) const TITLE_ELLIPSIS: &str = "\u{2026}";
