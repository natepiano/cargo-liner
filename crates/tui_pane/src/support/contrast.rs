//! Terminal color conversion and contrast calculations for tests.

use ratatui::style::Color;

use crate::constants::XTERM_ANSI_COLORS;
use crate::constants::XTERM_CUBE_LEVELS;

/// Convert a terminal color to RGB channels; treat `Color::Reset` as black.
pub(crate) fn selection_rgb(color: Color) -> (u8, u8, u8) {
    let named = match color {
        Color::Black | Color::Reset => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        Color::Rgb(red, green, blue) => return (red, green, blue),
        Color::Indexed(index) => return xterm_indexed_rgb(index),
    };
    XTERM_ANSI_COLORS[named]
}

fn xterm_indexed_rgb(index: u8) -> (u8, u8, u8) {
    if let Some(rgb) = XTERM_ANSI_COLORS.get(usize::from(index)) {
        return *rgb;
    }
    if index < 232 {
        let offset = index - 16;
        return (
            XTERM_CUBE_LEVELS[usize::from(offset / 36)],
            XTERM_CUBE_LEVELS[usize::from((offset % 36) / 6)],
            XTERM_CUBE_LEVELS[usize::from(offset % 6)],
        );
    }
    let gray = 8 + 10 * (index - 232);
    (gray, gray, gray)
}

/// Calculate selection contrast with the palette test arithmetic order.
#[cfg(feature = "contrast-test")]
#[must_use]
pub fn covered_contrast_ratio(foreground: Color, background: Color) -> f64 {
    let ink = <[u8; 3]>::from(selection_rgb(foreground)).map(f64::from);
    let ground = <[u8; 3]>::from(selection_rgb(background)).map(f64::from);
    let covered = std::array::from_fn(|index| {
        f64::mul_add(ground[index], 47.0 / 255.0, ink[index] * (208.0 / 255.0))
    });
    let luminance = |channels: [f64; 3]| {
        let [red, green, blue] = channels.map(|channel| {
            let channel = channel / 255.0;
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126f64.mul_add(red, 0.7152f64.mul_add(green, 0.0722 * blue))
    };
    let foreground = luminance(covered);
    let background = luminance(ground);
    (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
}

/// Calculate drawn-cell contrast with the rendered test arithmetic order.
#[cfg(test)]
pub(crate) fn drawn_contrast_ratio(ink: [u8; 3], ground: [u8; 3]) -> f64 {
    let ink = ink.map(f64::from);
    let ground = ground.map(f64::from);
    let covered =
        std::array::from_fn(|index| f64::mul_add(ground[index], 47.0, ink[index] * 208.0) / 255.0);
    let first = luminance(covered);
    let second = luminance(ground);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

#[cfg(test)]
fn luminance(channels: [f64; 3]) -> f64 {
    let linear = channels.map(|channel| {
        let value = channel / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    f64::mul_add(
        linear[2],
        0.0722,
        f64::mul_add(linear[1], 0.7152, linear[0] * 0.2126),
    )
}
