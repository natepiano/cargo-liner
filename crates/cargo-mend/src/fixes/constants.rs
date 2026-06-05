// fix runner
pub(crate) const FIX_ALL_MAX_PASSES: usize = 5;

// rustc lint suggestion protocol
pub(crate) const RUSTC_FIELD_VIS_REMOVE_SUGGESTION: &str =
    "remove the field's visibility annotation";
pub(crate) const RUSTC_LINT_SUGGESTION_PREFIX: &str = "consider using: `";

// use-import diagnostics
pub(super) const IMPORTS_AT_TOP_MESSAGE: &str =
    "lift this `use` to the top of its enclosing module";
pub(super) const IMPORTS_AT_TOP_SUGGESTION: &str =
    "move this `use` to the top of the file or inline module";
