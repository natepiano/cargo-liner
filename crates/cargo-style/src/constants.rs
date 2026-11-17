//! Named values of the style reader.

// blobs
/// The blobs read in parallel before their lines are written. It bounds the memory that waits
/// for stdout.
pub(crate) const BLOB_BATCH_SIZE: usize = 512;

// command arguments
/// The subcommand argument that Cargo inserts.
pub(crate) const STYLE_SUBCOMMAND: &str = "style";
/// The flag that requests the binary and protocol versions.
pub(crate) const VERSION_FLAG: &str = "--version";

// lines
/// The line end that Python reads as one boundary, though both characters are boundaries.
pub(crate) const CARRIAGE_RETURN_LINE_FEED: [char; 2] = ['\r', '\n'];

// module queries
/// The output fact name of each module site query.
pub(crate) const MODULE_SITES_FACT: &str = "module_sites";

// protocol
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
