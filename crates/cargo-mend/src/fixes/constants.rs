// fix runner
pub(crate) const FIX_CONVERGENCE_MAX_PASSES: usize = 5;

// rustc display columns
/// Columns rustc charges a tab when it computes `SourceMap::lookup_char_pos`'s
/// `col_display`, which is the column `Finding` carries.
pub(super) const TAB_DISPLAY_WIDTH: usize = 4;

// rustc lint suggestion protocol
pub(super) const RUSTC_FIELD_VIS_REMOVE_SUGGESTION: &str =
    "remove the field's visibility annotation";
pub(super) const RUSTC_LINT_SUGGESTION_PREFIX: &str = "consider using: `";

// use-import diagnostics
pub(super) const IMPORTS_AT_TOP_MESSAGE: &str =
    "lift this `use` to the top of its enclosing module";
pub(super) const IMPORTS_AT_TOP_SUGGESTION: &str =
    "move this `use` to the top of the file or inline module";
