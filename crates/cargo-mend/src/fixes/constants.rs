// `FIX_CONVERGENCE_MAX_PASSES` limits fix-runner convergence passes.
pub(crate) const FIX_CONVERGENCE_MAX_PASSES: usize = 5;

// `CFG_TEST_ATTRIBUTE` supplies the `pub_use_outside_subtree` fix's test gate.
/// The gate written on a private `use` that only `#[cfg(test)]` modules need.
pub(super) const CFG_TEST_ATTRIBUTE: &str = "#[cfg(test)]";

// `TAB_DISPLAY_WIDTH` defines rustc's tab width for display columns.
/// Columns rustc charges a tab when it computes `SourceMap::lookup_char_pos`'s
/// `col_display`, which is the column `Finding` carries.
pub(super) const TAB_DISPLAY_WIDTH: usize = 4;

// `RUSTC_FIELD_VIS_REMOVE_SUGGESTION` and `RUSTC_LINT_SUGGESTION_PREFIX`
// identify rustc lint suggestions.
pub(super) const RUSTC_FIELD_VIS_REMOVE_SUGGESTION: &str =
    "remove the field's visibility annotation";
pub(super) const RUSTC_LINT_SUGGESTION_PREFIX: &str = "consider using: `";

// `IMPORTS_AT_TOP_MESSAGE` and `IMPORTS_AT_TOP_SUGGESTION`
// supply use-import diagnostic text.
pub(super) const IMPORTS_AT_TOP_MESSAGE: &str =
    "lift this `use` to the top of its enclosing module";
pub(super) const IMPORTS_AT_TOP_SUGGESTION: &str =
    "move this `use` to the top of the file or inline module";
