//! The grammar for finding rename instructions.

/// Python adds four letters to ASCII ranges with `re.IGNORECASE`.
pub(super) const CASE_INSENSITIVE_RUST_FILE: &str =
    r"(?:[A-Za-zİıſK0-9_-]+/)*[A-Za-zİıſK_][A-Za-zİıſK0-9_]*\.rs";

pub(super) const FILE_RENAME: &str = r"\ARename ({file}) to ({file})";
pub(super) const SECOND_FILE_PAIR: &str = r"{file}\s+to\s+{file}";
pub(super) const NEGATED_SUBJECT: &str =
    r"\A\s*(?:nothing|none|no(?:\s+[\w-]+){0,3}?\s+{noun})\s*\z";
pub(super) const NEGATED_OBJECT: &str = r"\A\s+no(?:\s+[\w-]+){0,3}?\s+{noun}{end}";
pub(super) const SUBJECT_END: &str = r"\A(?:\s+(?:into|out|from|to|between|across|within)\b|{end})";
pub(super) const RUST_FILE: &str = r"(?:[A-Za-z0-9_-]+/)*[A-Za-z_][A-Za-z0-9_]*\.rs";
pub(super) const CODE_NOUN: &str = r"(?:definitions?|items?|types?|behaviou?r|code|functions?|methods?|impls?|tests?|helpers?|facades?|splits?)";
pub(super) const CLAUSE_END: &str = r"\s*(?:[.;]|$)";

pub(super) const MOVEMENT: &str = r"mov(?:ing|ed|es|e)|split(?:ting|s)?|merg(?:ing|ed|es|e)";
pub(super) const CLAUSE: &str = r"[.;:,()]|\b(?:and|but|then)\b";
pub(super) const NEGATED_VERB: &str = r"(?:\b(?:does|do|did|will|shall|must|should|may|can|could|would)\s+not|n't|\bcannot|\bnever|\bwithout)\s*$";
pub(super) const NEGATED_NOUN: &str = r"\A\s*no(?:\s+[\w-]+){0,3}\s*\z";
pub(super) const NOUN_END: &str = r"\A\s+(?:is|are|was|were)\b";
pub(super) const POSITIVE_NO: &str =
    r"\A\s*no\s+(?:longer|more|less|fewer|later|sooner|doubt|matter)\b";
