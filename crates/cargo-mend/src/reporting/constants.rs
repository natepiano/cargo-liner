// ansi codes
pub(super) const ANSI_BOLD: &str = "1";
pub(super) const ANSI_BOLD_BLUE: &str = "1;34";
pub(super) const ANSI_BOLD_GREEN: &str = "1;32";
pub(super) const ANSI_BOLD_RED: &str = "1;31";
pub(super) const ANSI_BOLD_YELLOW: &str = "1;33";
pub(super) const ANSI_DIM: &str = "2";

// color mode
pub(crate) const CARGO_TERM_COLOR_ALWAYS: &str = "always";
pub(crate) const CARGO_TERM_COLOR_NEVER: &str = "never";
pub(crate) const CLICOLOR_DISABLED_VALUE: &str = "0";

// color/terminal environment variables
pub(crate) const CARGO_TERM_COLOR_ENV: &str = "CARGO_TERM_COLOR";
pub(crate) const CLICOLOR_ENV: &str = "CLICOLOR";
pub(crate) const CLICOLOR_FORCE_ENV: &str = "CLICOLOR_FORCE";

// cargo mend user-facing invocations
pub(super) const CARGO_MEND_FIX: &str = "cargo mend --fix";
pub(super) const CARGO_MEND_FIX_ALL: &str = "cargo mend --fix-all";
pub(super) const CARGO_MEND_FIX_COMPILER: &str = "cargo mend --fix-compiler";
pub(super) const CARGO_MEND_FIX_PUB_USE: &str = "cargo mend --fix-pub-use";

// diagnostics help
pub(crate) const DIAGNOSTICS_HELP_NAME_COLUMN_WIDTH: usize = 40;

// exit codes
pub(crate) const EXIT_CODE_ERROR: u8 = 1;
pub(crate) const EXIT_CODE_WARNING: u8 = 2;

// fix-availability hint strings (full sentence; `concat!` cannot interpolate
// `const` items so the literal is materialized once here per invocation)
pub(super) const HINT_FIXABLE_WITH_FIX: &str =
    "this warning is auto-fixable with `cargo mend --fix`";
pub(super) const HINT_FIXABLE_WITH_FIX_PUB_USE: &str =
    "this warning is auto-fixable with `cargo mend --fix-pub-use`";

// rustc/cargo json protocol
pub(super) const CARGO_MESSAGE_TYPE_DIAGNOSTIC: &str = "diagnostic";
pub(super) const CARGO_REASON_BUILD_FINISHED: &str = "build-finished";
pub(super) const CARGO_REASON_COMPILER_MESSAGE: &str = "compiler-message";
pub(super) const RUSTC_LEVEL_HELP: &str = "help";
pub(super) const RUSTC_LEVEL_NOTE: &str = "note";

// summary block
pub(super) const SUMMARY_LABEL: &str = "summary:";
