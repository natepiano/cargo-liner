// description
pub(super) const ABOUT_LOWER_RESERVED_ROWS: u16 = 6;

// field dates
/// A short Git date drops this many leading year digits, as in `26-11-07`.
pub(super) const GIT_DATE_CENTURY_DIGITS: usize = 2;
/// The date pattern uses `0` for each digit.
pub(super) const GIT_DATE_PATTERN: &str = "0000-00-00";

// state messages
/// Cells between the left border and the state text of an empty pane.
pub(super) const GIT_EMPTY_TEXT_MARGIN: u16 = 1;
pub(super) const GIT_LOADING_TEXT: &str = "loading…";
pub(super) const GIT_NOT_REPOSITORY_TEXT: &str = "not a git repo";
