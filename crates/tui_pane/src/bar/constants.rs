// cpu reading
/// A status-line CPU reading below this percent shows one decimal place.
pub(super) const CPU_DECIMAL_BELOW_PERCENT: f64 = 10.0;

// `SEPARATOR` configures spacing in bar regions.
/// Inter-slot separator inside one region.
pub(super) const SEPARATOR: &str = "  ";

/// Minimum number of blank cells between key hints and a status-line note.
pub(super) const STATUS_NOTE_MIN_GAP: usize = 3;
