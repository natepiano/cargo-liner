use super::ColorMode;
use crate::reporting::constants::ANSI_BOLD_BLUE;
use crate::reporting::constants::ANSI_DIM;

pub(super) fn dim(text: &str, color_mode: ColorMode) -> String { paint(text, ANSI_DIM, color_mode) }

pub(super) fn blue_bold(text: &str, color_mode: ColorMode) -> String {
    paint(text, ANSI_BOLD_BLUE, color_mode)
}

pub(super) fn paint(text: &str, code: &str, color_mode: ColorMode) -> String {
    match color_mode {
        ColorMode::Enabled => format!("\x1b[{code}m{text}\x1b[0m"),
        ColorMode::Disabled => text.to_string(),
    }
}
