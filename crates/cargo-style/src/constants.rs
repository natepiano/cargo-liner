//! Named values of the style reader.

use std::time::Duration;

// blobs
/// The blobs read in parallel before their lines are written. It bounds the memory that waits
/// for stdout.
pub(crate) const BLOB_BATCH_SIZE: usize = 512;

// cache timing
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const WAL_RETRY_INTERVAL: Duration = Duration::from_millis(10);

// command arguments
pub(crate) const FINDING_RENAME_POLICY_FLAG: &str = "--rename-policy";
pub(crate) const FINDING_RULES_FLAG: &str = "--rules";
/// The subcommand argument that Cargo inserts.
pub(crate) const STYLE_SUBCOMMAND: &str = "style";
/// The flag that requests the binary and protocol versions.
pub(crate) const VERSION_FLAG: &str = "--version";

// enclosing items
pub(crate) const ITEMS: &[&str] = &[
    "fn",
    "struct",
    "enum",
    "union",
    "trait",
    "impl",
    "mod",
    "type",
    "const",
    "static",
    "macro_rules",
    "use",
    "extern",
];

// ledger
pub(crate) const LEDGER_SCHEMA_VERSION: i64 = 47;

// lines
/// The line end that Python reads as one boundary, though both characters are boundaries.
pub(crate) const CARRIAGE_RETURN_LINE_FEED: [char; 2] = ['\r', '\n'];

// module queries
/// The output fact name of each module site query.
pub(crate) const MODULE_SITES_FACT: &str = "module_sites";

// protocol
pub(crate) const FINDING_REPLY_SCHEMA: u32 = 4;
/// The version of the request and reply shapes.
pub(crate) const PROTOCOL_VERSION: u32 = 1;

// python token reader
/// The Python `PUNCTUATION` alternatives of two characters, in pattern order.
pub(crate) const PUNCTUATION_PAIRS: [[char; 2]; 9] = [
    [':', ':'],
    ['-', '>'],
    ['=', '>'],
    ['=', '='],
    ['!', '='],
    ['<', '='],
    ['>', '='],
    ['&', '&'],
    ['|', '|'],
];
/// The Python `PUNCTUATION` class of one character.
pub(crate) const PUNCTUATION_SINGLES: &str = "{}()[];:,.<>!&|+*/%=-?#@'$^~";
/// The token value that the Python reader gives a block comment with no end.
pub(crate) const UNTERMINATED_COMMENT: &str = "unterminated comment";

// rule components
pub(crate) const UNCOVERED: &str = "#uncovered";

// tip inputs
pub(crate) const IGNORE_FILES: [&str; 3] =
    [".claude/config/berth.toml", "rustfmt.toml", ".rustfmt.toml"];
