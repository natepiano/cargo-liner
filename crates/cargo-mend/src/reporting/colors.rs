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
