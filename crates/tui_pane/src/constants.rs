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

/// Seconds in one minute.
pub(crate) const SECONDS_PER_MINUTE: u64 = 60;

/// Default entrance and exit duration for toast animations.
pub(crate) const TOAST_ANIMATION_MILLIS: u64 = 150;
/// Elapsed time threshold where toast labels switch to minute formatting.
pub(crate) const TOAST_ELAPSED_MINUTE_MILLIS: u128 = 60_000;
/// Elapsed time threshold where toast labels switch from milliseconds to seconds.
pub(crate) const TOAST_ELAPSED_SECONDS_MILLIS: u128 = 10_000;

/// Two-space indent for popup section headers.
pub const SECTION_HEADER_INDENT: &str = "  ";
/// Four-space indent for popup section items.
pub const SECTION_ITEM_INDENT: &str = "    ";
