//! Patterns that select files for footprint reads.

pub(super) const UNREAD_PATH_STRING: &str = concat!(
    r#"(?s)(?<!\w)path(?:\s|/\*.*?\*/|//[^\n]*)*="#,
    r##"(?:\s|/\*.*?\*/|//[^\n]*)*(?:r#*"|"[^"\\]*\\)"##
);
