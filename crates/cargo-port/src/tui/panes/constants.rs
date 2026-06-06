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
pub(super) const PREFIX_WT_EXPANDED: &str = "   ▼";
pub(super) const PREFIX_WT_COLLAPSED: &str = "   ▶";
pub const PREFIX_WT_FLAT: &str = "   ";
pub(super) const PREFIX_WT_GROUP_EXPANDED: &str = "       ▼";
pub(super) const PREFIX_WT_GROUP_COLLAPSED: &str = "       ▶";
pub(super) const PREFIX_WT_MEMBER_INLINE: &str = "       ";
pub(super) const PREFIX_WT_MEMBER_NAMED: &str = "           ";
pub(super) const PREFIX_WT_MEMBER_VENDORED_INLINE: &str = "           ";
pub(super) const PREFIX_WT_MEMBER_VENDORED_NAMED: &str = "               ";
pub(super) const PREFIX_WT_VENDORED: &str = "       ";

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
