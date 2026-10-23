// `ANSI_BOLD` through `ANSI_DIM` supply ANSI terminal style codes.
pub(super) const ANSI_BOLD: &str = "1";
pub(super) const ANSI_BOLD_BLUE: &str = "1;34";
pub(super) const ANSI_BOLD_GREEN: &str = "1;32";
pub(super) const ANSI_BOLD_RED: &str = "1;31";
pub(super) const ANSI_BOLD_YELLOW: &str = "1;33";
pub(super) const ANSI_DIM: &str = "2";

// `CARGO_TERM_COLOR_ALWAYS`, `CARGO_TERM_COLOR_NEVER`, and `CLICOLOR_DISABLED_VALUE` select color
// modes.
pub(crate) const CARGO_TERM_COLOR_ALWAYS: &str = "always";
pub(crate) const CARGO_TERM_COLOR_NEVER: &str = "never";
pub(crate) const CLICOLOR_DISABLED_VALUE: &str = "0";

// `CARGO_TERM_COLOR_ENV`, `CLICOLOR_ENV`, and `CLICOLOR_FORCE_ENV` name color/terminal environment
// variables.
pub(crate) const CARGO_TERM_COLOR_ENV: &str = "CARGO_TERM_COLOR";
pub(crate) const CLICOLOR_ENV: &str = "CLICOLOR";
pub(crate) const CLICOLOR_FORCE_ENV: &str = "CLICOLOR_FORCE";

// `CARGO_MEND_FIX` through `CARGO_MEND_FIX_PUB_USE` spell user-facing cargo mend invocations.
pub(super) const CARGO_MEND_FIX: &str = "cargo mend --fix";
pub(super) const CARGO_MEND_FIX_ALL: &str = "cargo mend --fix-all";
pub(super) const CARGO_MEND_FIX_COMPILER: &str = "cargo mend --fix-compiler";
pub(super) const CARGO_MEND_FIX_PUB_USE: &str = "cargo mend --fix-pub-use";

// `DIAGNOSTICS_HELP_NAME_COLUMN_WIDTH` sets the diagnostics-help name column width.
pub(crate) const DIAGNOSTICS_HELP_NAME_COLUMN_WIDTH: usize = 40;

// `DURATION_SECONDS_PRECISION` controls duration formatting precision.
pub(super) const DURATION_SECONDS_PRECISION: usize = 2;

// `EXIT_CODE_ERROR` and `EXIT_CODE_WARNING` encode diagnostic exit statuses.
pub(crate) const EXIT_CODE_ERROR: u8 = 1;
pub(crate) const EXIT_CODE_WARNING: u8 = 2;

// `HINT_WARNING_FIXABLE_WITH_FIX` through `HINT_ERROR_FIXABLE_WITH_FIX_PUB_USE` supply
// full-sentence fix-availability hints; `concat!` cannot interpolate `const` items, so each
// invocation's literal is materialized once here.
pub(super) const HINT_WARNING_FIXABLE_WITH_FIX: &str =
    "this warning is auto-fixable with `cargo mend --fix`";
pub(super) const HINT_WARNING_FIXABLE_WITH_FIX_PUB_USE: &str =
    "this warning is auto-fixable with `cargo mend --fix-pub-use`";
pub(super) const HINT_ERROR_FIXABLE_WITH_FIX: &str =
    "this error is auto-fixable with `cargo mend --fix`";
pub(super) const HINT_ERROR_FIXABLE_WITH_FIX_PUB_USE: &str =
    "this error is auto-fixable with `cargo mend --fix-pub-use`";

// `PUB_USE_OUTSIDE_SUBTREE_HELP` explains how to resolve `pub_use_outside_subtree`.
pub(crate) const PUB_USE_OUTSIDE_SUBTREE_HELP: &str = "re-export the item from the module whose \
     subtree owns it — usually the parent `mod.rs` — or move the definition to the module that \
     owns the concept, then point callers at that path";

// `CARGO_MESSAGE_TYPE_DIAGNOSTIC` through `RUSTC_LEVEL_NOTE` identify rustc/cargo JSON protocol
// values.
pub(super) const CARGO_MESSAGE_TYPE_DIAGNOSTIC: &str = "diagnostic";
pub(super) const CARGO_REASON_BUILD_FINISHED: &str = "build-finished";
pub(super) const CARGO_REASON_COMPILER_MESSAGE: &str = "compiler-message";
pub(super) const RUSTC_LEVEL_HELP: &str = "help";
pub(super) const RUSTC_LEVEL_NOTE: &str = "note";

// `SUMMARY_LABEL` introduces the summary block.
pub(super) const SUMMARY_LABEL: &str = "summary:";
