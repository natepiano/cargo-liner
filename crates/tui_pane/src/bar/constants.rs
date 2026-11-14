// rate labels
/// A rate below this value after rounding to tenths shows one decimal place.
pub(super) const RATE_DECIMAL_BELOW: f64 = 10.0;

// cpu reading
/// Label before the current process CPU reading.
pub(super) const CPU_LABEL: &str = "CPU: ";

// disk reads
/// Label before the current process disk read rate.
pub(super) const DISK_READ_LABEL: &str = "Reads/s: ";

/// Decimal units use one thousand bytes.
pub(super) const DISK_READ_UNIT_BASE: f64 = 1000.0;

// `SEPARATOR` configures spacing in bar regions.
/// Inter-slot separator inside one region.
pub(super) const SEPARATOR: &str = "  ";

/// Minimum number of blank cells between key hints and a status-line note.
pub(super) const STATUS_NOTE_MIN_GAP: usize = 3;
