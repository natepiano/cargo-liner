//! Non-color framework constants. Color values moved to the theme
//! system (`theme::accessors` + per-role functions re-exported at
//! crate root) so they can be swapped at runtime.

/// Bytes per kibibyte.
pub const BYTES_PER_KIB: u64 = 1024;
/// Bytes per mebibyte.
pub const BYTES_PER_MIB: u64 = 1024 * 1024;
/// Bytes per gibibyte.
pub const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// Block border costs 2 horizontal characters (left + right).
pub const BLOCK_BORDER_WIDTH: usize = 2;

/// Tick interval between render polls.
pub const FRAME_POLL_MILLIS: u64 = 16;

/// Two-space indent for popup section headers.
pub const SECTION_HEADER_INDENT: &str = "  ";
/// Four-space indent for popup section items.
pub const SECTION_ITEM_INDENT: &str = "    ";
