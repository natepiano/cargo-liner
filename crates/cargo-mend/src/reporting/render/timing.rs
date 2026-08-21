use std::time::Duration;

use super::ColorMode;
use super::color;
use crate::reporting::constants::ANSI_BOLD_GREEN;
use crate::reporting::constants::DURATION_SECONDS_PRECISION;

pub(crate) fn render_timing(
    total: Duration,
    check: Duration,
    mend: Duration,
    color_mode: ColorMode,
) -> String {
    format!(
        "    {} in {:.precision$}s (check: {:.precision$}s, mend: {:.precision$}s)",
        color::paint("Finished", ANSI_BOLD_GREEN, color_mode),
        total.as_secs_f64(),
        check.as_secs_f64(),
        mend.as_secs_f64(),
        precision = DURATION_SECONDS_PRECISION,
    )
}
