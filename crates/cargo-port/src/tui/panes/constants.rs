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

use std::time::Duration;

// Git pane constants

pub(super) const FIT_TEXT_ELLIPSIS: &str = "...";
pub(super) const PULL_REQUEST_MIN_TITLE_WIDTH: usize = 8;

// Row prefix strings — single source of truth for width calc and render.

pub(super) const PREFIX_ROOT_EXPANDED: &str = "▼ ";
pub const PREFIX_ROOT_COLLAPSED: &str = "▶ ";
pub const PREFIX_ROOT_LEAF: &str = "  ";
pub(super) const PREFIX_MEMBER_INLINE: &str = "   ";
pub(super) const PREFIX_MEMBER_NAMED: &str = "       ";
pub(super) const PREFIX_MEMBER_VENDORED_INLINE: &str = "       ";
pub(super) const PREFIX_MEMBER_VENDORED_NAMED: &str = "           ";
pub(super) const PREFIX_SUBMODULE: &str = "   ";
pub(super) const PREFIX_VENDORED: &str = "   ";
pub(super) const PREFIX_GROUP_EXPANDED: &str = "   ▼";
pub(super) const PREFIX_GROUP_COLLAPSED: &str = "   ▶";
pub(super) const PREFIX_WORKTREE_EXPANDED: &str = "   ▼";
pub(super) const PREFIX_WORKTREE_COLLAPSED: &str = "   ▶";
pub const PREFIX_WORKTREE_FLAT: &str = "   ";
pub(super) const PREFIX_WORKTREE_GROUP_EXPANDED: &str = "       ▼";
pub(super) const PREFIX_WORKTREE_GROUP_COLLAPSED: &str = "       ▶";
pub(super) const PREFIX_WORKTREE_MEMBER_INLINE: &str = "       ";
pub(super) const PREFIX_WORKTREE_MEMBER_NAMED: &str = "           ";
pub(super) const PREFIX_WORKTREE_MEMBER_VENDORED_INLINE: &str = "           ";
pub(super) const PREFIX_WORKTREE_MEMBER_VENDORED_NAMED: &str = "               ";
pub(super) const PREFIX_WORKTREE_VENDORED: &str = "       ";

// tests detail rows

/// Label of the `(ignored)` annotation row in the Tests section. Shared
/// with the renderer, which dims this row's value (the count is a
/// registered-but-skipped doctest tally, not a runnable test).
pub(super) const TESTS_IGNORED_LABEL: &str = "(ignored)";
/// Label of the runnable-total row in the Tests section — intentionally
/// blank. The Languages pane shows its grand total as an unlabelled bold
/// bottom value; the Tests total matches, so the renderer keys off this
/// empty label to render the value in bold accent color.
pub(super) const TESTS_TOTAL_LABEL: &str = "";

// src tui panes ci
pub(super) const CI_BRANCH_LONG_MIN_WIDTH: usize = 16;
pub(super) const CI_BRANCH_MIN_WIDTH: usize = 6;
pub(super) const CI_COMMIT_LONG_MIN_WIDTH: usize = 22;
pub(super) const CI_COMMIT_MIN_WIDTH: usize = 7;
pub(super) const CI_JOB_LABEL_MAX_WIDTH: usize = 16;
pub(super) const CI_JOB_LABEL_MIN_WIDTH: usize = 8;
pub(super) const CI_STATUS_GAP_WIDTH: usize = 1;
pub(super) const OTHER_JOBS_HEADER: &str = "Other";

// src tui panes cpu
pub(super) const CPU_BAR_WIDTH: usize = 10;
/// Breakdown rows pinned below the cores band: System, User, Idle.
pub(super) const CPU_BREAKDOWN_ROWS: usize = 3;
pub(super) const CPU_CONTENT_WIDTH: u16 = 17;
/// GPU rows pinned below the breakdown. A single aggregate row; multi-GPU is
/// deferred (see `docs/subpane-scrolling.md` → Related, separate work), so
/// growing the pinned tail for more GPUs changes only this count.
pub(super) const CPU_GPU_ROWS: usize = 1;
pub const CPU_PANE_WIDTH: u16 = CPU_CONTENT_WIDTH + 2;
/// Pinned head rows above the scrolling cores band: the aggregate line.
pub(super) const CPU_PINNED_HEAD_ROWS: usize = 1;
/// Pixel height of every inner row except the scrolling cores band: the
/// aggregate row, the two separator rules, the three breakdown rows, and the
/// one GPU row. The cores band gets `inner.height - CPU_STATIC_INNER_HEIGHT`.
pub(super) const CPU_STATIC_INNER_HEIGHT: u16 = 7;
/// Shown in the GPU row when the OS exposes no GPU utilization (e.g. the
/// Apple `asahi` driver on Linux). Kept within `CPU_CONTENT_WIDTH` so it
/// never widens the pane.
pub(super) const GPU_UNAVAILABLE_TEXT: &str = "unavailable";

// src tui panes description
/// "No description available" placeholder rendered by the Package pane
/// when the source description is empty. Lives here so [`super::DescriptionBlock`]
/// owns both the placeholder text and the row wrapping that consumes it —
/// no caller can substitute a different placeholder and break the sync
/// invariant.
pub(super) const NO_DESCRIPTION_AVAILABLE: &str = "No description available";

// src tui panes git
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

// src tui panes lang
/// Fixed numeric column width for language stats.
pub(super) const LANG_NUM_COL: u16 = 8;

// src tui panes layout
/// Rows a bordered pane spends on its top and bottom border. Outer pane height
/// is inner content height plus this.
pub(super) const PANE_BORDER_HEIGHT: u16 = 2;

// src tui panes package
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

// src tui panes project_list
pub(super) const DISMISS_SUFFIX: &str = " [x]";
pub(super) const TITLE_ELLIPSIS: &str = "\u{2026}";

// src tui panes system
/// Cadence for the running-targets poller. Hardcoded for v1; moves to
/// config alongside `CpuConfig` once the feature stabilizes.
pub(super) const RUNNING_TARGETS_POLL_INTERVAL: Duration = Duration::from_secs(1);
