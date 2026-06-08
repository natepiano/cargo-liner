use std::time::Duration;

use super::ColorMode;
use super::color;
use crate::reporting::constants::ANSI_BOLD_GREEN;

pub(crate) fn render_timing(
    total: Duration,
    check: Duration,
    mend: Duration,
    color_mode: ColorMode,
) -> String {
    format!(
        "    {} in {:.2}s (check: {:.2}s, mend: {:.2}s)",
        color::paint("Finished", ANSI_BOLD_GREEN, color_mode),
        total.as_secs_f64(),
        check.as_secs_f64(),
        mend.as_secs_f64(),
    )
}
