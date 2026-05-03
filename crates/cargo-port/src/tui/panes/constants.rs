//! Panes-rendering constants.
//!
//! Per `~/rust/nate_style/rust/no-magic-values.md`: directory
//! modules own their constants in their own `constants.rs`, not
//! the parent's. These are panes-rendering-specific (tree-row
//! prefixes), so they live here, not in `tui::constants`.
//!
//! Visibility: `pub(super)` — visible to `panes/*` siblings only.
//! No `pub use` re-export at `panes/mod.rs`. The column-fit-width
//! math in `panes::widths` is the only consumer outside the
//! renderer in `panes::project_list`.

// Row prefix strings — single source of truth for width calc and render.

pub(super) const PREFIX_ROOT_EXPANDED: &str = "▼";
pub const PREFIX_ROOT_COLLAPSED: &str = "▶";
pub const PREFIX_ROOT_LEAF: &str = " ";
pub(super) const PREFIX_MEMBER_INLINE: &str = "   ";
pub(super) const PREFIX_MEMBER_NAMED: &str = "       ";
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
pub(super) const PREFIX_WT_VENDORED: &str = "       ";
