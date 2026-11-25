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
pub(crate) const COVERAGE_SUBCOMMAND: &str = "coverage";
/// The flag that names the user's answers on decision files for the declaration reading.
pub(crate) const FINDING_APPROVALS_FLAG: &str = "--approvals";
/// The flag that selects a base commit for finding footprint searches.
pub(crate) const FINDING_BASE_FLAG: &str = "--base";
pub(crate) const FINDING_RENAME_POLICY_FLAG: &str = "--rename-policy";
pub(crate) const FINDING_RULES_FLAG: &str = "--rules";
/// The subcommand argument that Cargo inserts.
pub(crate) const STYLE_SUBCOMMAND: &str = "style";
/// The flag that requests the binary and protocol versions.
pub(crate) const VERSION_FLAG: &str = "--version";

// declaration reading
/// The keywords that a type finding's item head can start with.
pub(crate) const ITEM_HEAD_KEYWORDS: [&str; 9] = [
    "fn", "struct", "enum", "union", "trait", "type", "const", "static", "mod",
];
/// The keywords of a module item that can own an impl or trait member.
pub(crate) const OWNER_KEYWORDS: [&str; 5] = ["struct", "enum", "union", "trait", "type"];

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

// import resolution
pub(crate) const IMPORT_EXPANSION_LIMIT: usize = 32;

// ledger
pub(crate) const LEDGER_SCHEMA_VERSIONS: [i64; 2] = [47, 51];

// lines
/// The line end that Python reads as one boundary, though both characters are boundaries.
pub(crate) const CARRIAGE_RETURN_LINE_FEED: [char; 2] = ['\r', '\n'];
/// The minimum sequence length for Python `SequenceMatcher` to omit popular lines.
pub(crate) const AUTOJUNK_MIN_LINES: usize = 200;
/// The divisor that Python `SequenceMatcher` uses to find popular lines.
pub(crate) const AUTOJUNK_POPULARITY_DIVISOR: usize = 100;

// module queries
/// The output fact name of each module site query.
pub(crate) const MODULE_SITES_FACT: &str = "module_sites";

// protocol
pub(crate) const FINDING_PROOF_REPLY_SCHEMA: u32 = 7;
pub(crate) const FINDING_REPLY_SCHEMA: u32 = 5;
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
