//! Named values of the `sites` command.

// allow attributes
pub(super) const BOILERPLATE_LINTS: &[(&str, &str)] = &[
    ("clippy::expect_used", ".expect("),
    ("clippy::unwrap_used", ".unwrap("),
    ("clippy::panic", "panic!"),
];

// command
pub(crate) const SITES_SUBCOMMAND: &str = "sites";

// protocol
pub(super) const SITES_REPLY_SCHEMA: u32 = 1;

// site identity
/// Lowercase hexadecimal digits, indexed by a 4-bit value.
pub(super) const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

// display text
/// Python keeps the first 200 characters of a site's stripped display text.
pub(super) const DISPLAY_LIMIT: usize = 200;

// exclusion interpreter
/// The variable that names the first Python interpreter that `scripts/lib/py` tries.
pub(super) const CLAUDE_PYTHON: &str = "CLAUDE_PYTHON";
pub(super) const MATCH_EXCLUSIONS: &str = r"
import json
import re
import sys
pattern, lines = json.load(sys.stdin)
try:
    matcher = re.compile(pattern)
except re.error as error:
    print(error, file=sys.stderr)
    sys.exit(1)
json.dump([matcher.search(line) is not None for line in lines], sys.stdout)
";
/// The `python3` paths that `scripts/lib/py` tries after the per-user profile, in order.
pub(super) const PACKAGE_PYTHONS: [&str; 2] =
    ["/opt/homebrew/bin/python3", "/usr/local/bin/python3"];
/// The folder that holds each user's profile; `scripts/lib/py` tries `<folder>/$USER/bin/python3`.
pub(super) const PER_USER_PROFILES: &str = "/etc/profiles/per-user";
pub(super) const PROFILE_PYTHON: &str = "bin/python3";
/// The command name that `scripts/lib/py` looks up on `PATH`.
pub(super) const PYTHON_COMMAND: &str = "python3";
/// Exits 0 only on Python 3.12 or newer, the floor that `scripts/lib/py` sets.
pub(super) const PYTHON_VERSION_CHECK: &str =
    "import sys; sys.exit(0 if sys.version_info >= (3, 12) else 1)";
pub(super) const SYSTEM_PYTHON: &str = "/run/current-system/sw/bin/python3";

// literals
pub(super) const FIXED_SPELLINGS: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "mod.rs",
    "lib.rs",
    "main.rs",
    "crate",
    "super",
    "self",
    "src",
];
pub(super) const FORMAT_MACROS: &str = concat!(
    "format|print|println|eprint|eprintln|write|writeln|panic|assert|assert_eq|assert_ne|",
    "debug_assert|debug_assert_eq|debug_assert_ne|todo|unimplemented|unreachable|matches|vec",
);
pub(super) const IDENTITY_NUMERICS: &[&str] = &["0", "1", "0.0", "1.0"];
pub(super) const INCLUDE_MACROS: &str = "include_str|include_bytes|embedded_asset|env|concat";
pub(super) const LITERAL_EXEMPTIONS: [&str; 3] = ["examples", "benches", "build.rs"];
pub(super) const MIN_NUMERIC_LITERAL_DIGITS: usize = 2;
pub(super) const MIN_STRING_LITERAL_CHARACTERS: usize = 2;

// observer guards
pub(super) const OBSERVER_BODY_WINDOW: usize = 6;

// source mask
/// The last character position that can close an escaped char literal.
pub(super) const ESCAPED_CHAR_LIMIT: usize = 12;

// test spans
pub(super) const CFG_TEST: &str = "#[cfg(test)]";

// type sites
pub(super) const UNIT_SUFFIXES: &[&str] = &[
    "bytes", "deg", "degrees", "hz", "micros", "millis", "ms", "nanos", "px", "rad", "radians",
    "seconds", "secs",
];
pub(super) const VISIBILITY_PATTERN: &str = r"(?:pub(?:\([^)]*\))?\s+)?";
