//! The grammar for finding rename instructions.

/// Python adds four letters to ASCII ranges with `re.IGNORECASE`.
pub(super) const CASE_INSENSITIVE_RUST_FILE: &str =
    r"(?:[A-Za-zİıſK0-9_-]+/)*[A-Za-zİıſK_][A-Za-zİıſK0-9_]*\.rs";
pub(super) const CLAUSE_END: &str = r"\s*(?:[.;]|$)";
pub(super) const CODE_NOUN: &str = concat!(
    r"(?:definitions?|items?|types?|behaviou?r|code|functions?|methods?|",
    r"impls?|tests?|helpers?|facades?|splits?)"
);
pub(super) const NO_LEAD: &str = r"no(?!\s+(?:longer|more|less|fewer|later|sooner|doubt|matter)\b)";
pub(super) const RUST_FILE: &str = r"(?:[A-Za-z0-9_-]+/)*[A-Za-z_][A-Za-z0-9_]*\.rs";
pub(super) const RUST_KEYWORDS: &str = concat!(
    "as async await break const continue crate dyn else enum extern false fn for ",
    "gen if impl in let loop match mod move mut pub ref return self Self static struct super ",
    "trait true try type unsafe use where while abstract become box do final macro override ",
    "priv typeof unsized virtual yield"
);
