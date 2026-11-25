//! The grammar for file rename instructions.

/// Python folds these letters into ASCII ranges with `re.IGNORECASE`.
pub(super) const CASE_INSENSITIVE_RUST_FILE: &str =
    r"(?:[A-Za-zİıſK0-9_-]+/)*[A-Za-zİıſK_][A-Za-zİıſK0-9_]*\.rs";

pub(super) const RUST_FILE: &str = r"(?:[A-Za-z0-9_-]+/)*[A-Za-z_][A-Za-z0-9_]*\.rs";
pub(super) const RENAME_FILE_PAIR: &str = r"(?P<old_quote>`?)(?P<old>{file})\k<old_quote>\s+to\s+(?P<new_quote>`?)(?P<new>{file})\k<new_quote>(?=\z|\s|[;,:)]|\.(?:\s|\z))";
pub(super) const RENAME_VERB: &str = r"rename\s+(?:(?:the\s+)?(?:compan[iİı]on\s+)?f[iİı]le\s+)?";
pub(super) const FILE_INSTRUCTION_VERB: &str = r"\A(?:rename|move|relocate)\s+";
pub(super) const SENTENCE_SEPARATOR: &str = r"\s*[.;]\s+";
pub(super) const PAIR_SEPARATOR: &str = r"(?:\s*,\s*(?:(?:and|plus)\s+)?|\s+(?:and|plus)\s+)";
pub(super) const RENAME_DESCRIPTION: &str = r"\A\s+\(the type [iİı]t holds\)";
pub(super) const END_OF_RENAME: &str = r"\A\s*\.?\z";
pub(super) const REFERENCE_NAME: &str = r"[A-Za-zİıſK_][A-Za-zİıſK0-9_]*";
pub(super) const REFERENCE_TARGET: &str = r"(?:the\s+|all\s+)?(?:module declarat[iİı]ons?|mod(?: l[iİı]nes?)?|pub use(?: l[iİı]nes?)?|[iİı]mports?|path attr[iİı]butes?|documentat[iİı]on references?|`{file}`|{file}|(?:{name}::)+{name}\s+paths?)(?=\z|\s|[;,]|\.(?:\s|\z))";
pub(super) const REFERENCE_SEPARATOR: &str = r"(?:\s*,\s*(?:and\s+)?|\s+and\s+)";

/// Python's space class includes these four separators in addition to Unicode whitespace.
pub(super) const PYTHON_SPACE: &str = r"[\s\x1c-\x1f]";
