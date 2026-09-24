//! Constants for `favorites.toml`: the keys each row is written with,
//! the diagnostic spelling for a key a row leaves out, and the lock and
//! atomic-write timing and file names.

use std::time::Duration;

/// TOML key for a favorite's pixel block width.
pub(super) const FAVORITE_BLOCK_COLUMNS_KEY: &str = "block_columns";
/// TOML key for a favorite's travel direction.
pub(super) const FAVORITE_DIRECTION_KEY: &str = "direction";
/// TOML key for a favorite's text drift behavior.
pub(super) const FAVORITE_DRIFT_KEY: &str = "drift";
/// TOML key for a favorite's cell fill behavior.
pub(super) const FAVORITE_FILL_KEY: &str = "fill";
/// TOML key for a favorite's band fraying behavior.
pub(super) const FAVORITE_FRAYING_KEY: &str = "fraying";
/// TOML key for a favorite's stable UUID.
pub(super) const FAVORITE_ID_KEY: &str = "id";
/// Diagnostic spelling used when a favorite omits a required key.
pub(super) const FAVORITE_MISSING_VALUE: &str = "<missing>";
/// TOML key for a favorite's attract mode.
pub(super) const FAVORITE_MODE_KEY: &str = "mode";
/// TOML key for a favorite's pixel resolution behavior.
pub(super) const FAVORITE_RESOLVE_KEY: &str = "resolve";
/// TOML key for a favorite's save timestamp.
pub(super) const FAVORITE_SAVED_KEY: &str = "saved";
/// TOML key for a favorite's travel speed.
pub(super) const FAVORITE_SPEED_KEY: &str = "speed";
/// TOML key for a favorite's text speed spread.
pub(super) const FAVORITE_SPREAD_KEY: &str = "spread";
/// TOML key for a favorite's band-tail speed.
pub(super) const FAVORITE_TAIL_SPEED_KEY: &str = "tail_speed";
/// TOML key for a favorite's pixel-wave width.
pub(super) const FAVORITE_WAVE_PERCENT_KEY: &str = "wave_percent";
/// TOML key for a favorite's band width.
pub(super) const FAVORITE_WIDTH_KEY: &str = "width";
/// Top-level array of favorite tables.
pub(super) const FAVORITES_ARRAY_KEY: &str = "favorite";
/// Attract-screen favorites stored beside the app configuration.
pub(super) const FAVORITES_FILENAME: &str = "favorites.toml";
/// Number of brief retries while another process owns the favorites lock.
pub(super) const FAVORITES_LOCK_RETRY_ATTEMPTS: usize = 10;
/// Delay between attempts to acquire the favorites lock.
pub(super) const FAVORITES_LOCK_RETRY_DELAY: Duration = Duration::from_millis(10);
/// Suffix appended to the favorites path for its sibling lock file.
pub(super) const FAVORITES_LOCK_SUFFIX: &str = ".lock";
/// Suffix appended to the favorites path for its atomic-write file.
pub(super) const FAVORITES_TEMP_SUFFIX: &str = ".tmp";
