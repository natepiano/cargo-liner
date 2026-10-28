//! Shell pattern bytes shared with the shim integration tests.

/// Template slot for Cargo's global options with separate values.
pub(crate) const SHIM_GLOBAL_OPTIONS_PLACEHOLDER: &str = "@CARGO_GLOBAL_VALUED_OPTIONS@";
/// Joins alternatives in the shell's option case pattern.
pub(crate) const SHIM_OPTION_PATTERN_SEPARATOR: &str = " | ";
