//! Constants for the binding scan, edit alignment, and presence check.

// Binding scan.
pub(super) const KEYWORDS: &[&str] = &[
    "Self", "abstract", "as", "async", "await", "become", "box", "break", "const", "continue",
    "crate", "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if",
    "impl", "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub",
    "ref", "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];
pub(super) const NAMED_ITEMS: &[&str] = &[
    "const", "enum", "fn", "mod", "static", "struct", "trait", "type", "union",
];

// Edit alignment.
pub(super) const NAMED_WIDTH_PATTERN: &str = r"[A-Za-z_][A-Za-z_0-9]*\$";

// Presence check.
pub(super) const STANDARD_EXPRESSION_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "concat",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "env",
    "eprint",
    "eprintln",
    "format",
    "format_args",
    "matches",
    "option_env",
    "panic",
    "print",
    "println",
    "stringify",
    "todo",
    "unimplemented",
    "unreachable",
    "vec",
    "write",
    "writeln",
];
