//! Keywords for qualified rename checks.

pub(super) const ITEM_KEYWORDS: &[&str] = &[
    "const", "enum", "fn", "mod", "static", "struct", "trait", "type", "union",
];
pub(super) const NAME_MACROS: &[&str] = &["concat_idents", "include", "paste"];
pub(super) const PARENT_KEYWORDS: &[&str] = &["enum", "mod", "struct", "trait", "union"];
pub(super) const STANDARD_DERIVES: &[&str] = &[
    "Clone",
    "Copy",
    "Debug",
    "Default",
    "Eq",
    "Hash",
    "Ord",
    "PartialEq",
    "PartialOrd",
];
