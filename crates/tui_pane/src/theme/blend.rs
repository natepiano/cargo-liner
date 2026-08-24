//! Colour arithmetic over ratatui's [`Color`].
//!
//! A terminal cell holds three opaque bytes and has nowhere to put a
//! fourth, so a colour that must stand part way between two others is
//! worked out here and only its result is written. [`blend_color`] is
//! what the render layer calls.
//!
//! Most of the work is [`channels`]: a [`Theme`](super::Theme) is
//! written in whatever [`Color`] reads best, and the named ANSI
//! colours carry no channels of their own. They are read against
//! [`ANSI_BASE_PALETTE`], so a theme naming `Color::Cyan` blends the
//! same way as one naming `Color::Rgb(0, 205, 205)`.

use ratatui::style::Color;

use super::constants::ANSI_BASE_PALETTE;
use super::constants::ANSI_CUBE_BASE;
use super::constants::ANSI_CUBE_LEVELS;
use super::constants::ANSI_GRAYSCALE_BASE;
use super::constants::ANSI_GRAYSCALE_START;
use super::constants::ANSI_GRAYSCALE_STEP;
use super::constants::COLOR_DISTANCE_GREEN_WEIGHT;
use super::constants::COLOR_DISTANCE_SCALE;
use super::constants::COLOR_DISTANCE_SIDE_WEIGHT;

/// `color` carried `alpha` of the way toward `toward`: zero leaves
/// `color` where it is, [`u8::MAX`] arrives at `toward`.
///
/// This is the composite an alpha channel would have done, and the
/// scale is the one an alpha channel uses. A colour with no channels
/// to read -- [`Color::Reset`], which stands for whatever the terminal
/// profile calls its default -- is handed back untouched at either
/// end, because there is nothing to work a mixture out against.
#[must_use]
pub fn blend_color(color: Color, toward: Color, alpha: u8) -> Color {
    let (Some(from), Some(to)) = (channels(color), channels(toward)) else {
        return color;
    };
    Color::Rgb(
        mix(from.0, to.0, alpha),
        mix(from.1, to.1, alpha),
        mix(from.2, to.2, alpha),
    )
}

/// How far apart two colours look, on the 0 to 764 scale black against
/// white sits at the top of. [`None`] where either of them has no
/// channels to read.
///
/// Straight channel distance answers a different question: it weights
/// the three equally, and the eye does not. This is the redmean
/// approximation, which weights green heaviest throughout and slides
/// the rest from blue toward red as the pair gets redder -- the
/// cheapest correction that tracks perception at all, and enough to
/// ask whether two colours would be read as the same one.
///
/// Integer throughout: the weights are held in a fixed point and
/// divided back out under the root, which lands on the same answer the
/// float formula gives.
#[must_use]
pub fn color_distance(color: Color, other: Color) -> Option<u16> {
    let (Some(from), Some(to)) = (channels(color), channels(other)) else {
        return None;
    };
    let mean = u32::midpoint(u32::from(from.0), u32::from(to.0));
    let red = COLOR_DISTANCE_SIDE_WEIGHT.saturating_add(mean);
    let blue = COLOR_DISTANCE_SIDE_WEIGHT.saturating_add(u32::from(u8::MAX) - mean);
    let weighted = red * squared(from.0, to.0)
        + COLOR_DISTANCE_GREEN_WEIGHT * squared(from.1, to.1)
        + blue * squared(from.2, to.2);
    u16::try_from((weighted / COLOR_DISTANCE_SCALE).isqrt()).ok()
}

/// The gap between two channels, squared, wide enough to hold the
/// weight [`color_distance`] multiplies it by.
fn squared(from: u8, to: u8) -> u32 {
    let difference = u32::from(from.abs_diff(to));
    difference * difference
}

/// One channel of `from` carried `alpha` of the way to `to`.
fn mix(from: u8, to: u8, alpha: u8) -> u8 {
    let whole = u32::from(u8::MAX);
    let rest = u32::from(u8::MAX.saturating_sub(alpha));
    let mixed = (u32::from(from) * rest + u32::from(to) * u32::from(alpha)) / whole;
    u8::try_from(mixed).unwrap_or(u8::MAX)
}

/// What `color` amounts to in red, green and blue, or `None` for
/// [`Color::Reset`] -- the one variant that names no colour at all.
fn channels(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Rgb(red, green, blue) => Some((red, green, blue)),
        Color::Indexed(index) => indexed_channels(index),
        Color::Reset => None,
        named => indexed_channels(ansi_index(named)?),
    }
}

/// Where a named [`Color`] sits in the 256-colour palette.
const fn ansi_index(color: Color) -> Option<u8> {
    // The dictionary for the enum: ANSI numbers the sixteen colours,
    // and ratatui names them.
    Some(match color {
        Color::Black => 0,
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
        _ => return None,
    })
}

/// What a 256-colour palette entry amounts to in red, green and blue.
///
/// Three ranges: the sixteen ANSI colours, then the 6x6x6 colour cube,
/// then the grayscale ramp closing the palette.
fn indexed_channels(index: u8) -> Option<(u8, u8, u8)> {
    if let Some(&channels) = ANSI_BASE_PALETTE.get(usize::from(index)) {
        return Some(channels);
    }
    if index >= ANSI_GRAYSCALE_BASE {
        let step = index
            .saturating_sub(ANSI_GRAYSCALE_BASE)
            .saturating_mul(ANSI_GRAYSCALE_STEP);
        let level = ANSI_GRAYSCALE_START.saturating_add(step);
        return Some((level, level, level));
    }
    let cube = usize::from(index.checked_sub(ANSI_CUBE_BASE)?);
    let side = ANSI_CUBE_LEVELS.len();
    Some((
        *ANSI_CUBE_LEVELS.get(cube / (side * side) % side)?,
        *ANSI_CUBE_LEVELS.get(cube / side % side)?,
        *ANSI_CUBE_LEVELS.get(cube % side)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where [`Color::Cyan`] sits in the sixteen, which is the entry a
    /// blend against it reads.
    const CYAN_PALETTE_INDEX: usize = 6;

    /// The widest pair there is has to reach the top of the scale, or
    /// a threshold written against the scale means nothing.
    #[test]
    fn black_and_white_stand_the_whole_scale_apart() {
        assert_eq!(
            color_distance(Color::Rgb(0, 0, 0), Color::Rgb(255, 255, 255)),
            Some(764)
        );
    }

    #[test]
    fn a_colour_stands_no_distance_from_itself() {
        let color = Color::Rgb(0x2A, 0xA1, 0x98);

        assert_eq!(color_distance(color, color), Some(0));
    }

    /// Green carries the heaviest weight throughout, so the same gap
    /// on green reads wider than it does on red or blue.
    #[test]
    fn green_counts_for_more_than_the_channels_either_side_of_it() {
        let gap = 60;
        let green = color_distance(Color::Rgb(0, 0, 0), Color::Rgb(0, gap, 0));

        assert!(green > color_distance(Color::Rgb(0, 0, 0), Color::Rgb(gap, 0, 0)));
        assert!(green > color_distance(Color::Rgb(0, 0, 0), Color::Rgb(0, 0, gap)));
    }

    /// [`Color::Reset`] names whatever the terminal profile calls its
    /// default, which is a colour this side of the wire cannot read.
    #[test]
    fn a_colour_with_no_channels_is_no_distance_from_anything() {
        assert_eq!(color_distance(Color::Reset, Color::Rgb(0, 0, 0)), None);
        assert_eq!(color_distance(Color::Rgb(0, 0, 0), Color::Reset), None);
    }

    #[test]
    fn a_blend_of_nothing_leaves_the_colour_where_it_is() {
        let color = Color::Rgb(10, 20, 30);

        assert_eq!(blend_color(color, Color::Rgb(200, 200, 200), 0), color);
    }

    #[test]
    fn a_whole_blend_arrives_at_the_colour_it_was_carried_toward() {
        let toward = Color::Rgb(200, 210, 220);

        assert_eq!(blend_color(Color::Rgb(0, 0, 0), toward, u8::MAX), toward);
    }

    #[test]
    fn a_half_blend_stands_between_the_two() {
        let half = u8::MAX / 2;

        assert_eq!(
            blend_color(Color::Rgb(0, 0, 0), Color::Rgb(200, 100, 50), half),
            Color::Rgb(99, 49, 24)
        );
    }

    /// A theme written in named colours has to fade like any other, so
    /// the names are read against the palette rather than refused.
    #[test]
    fn a_named_colour_blends_by_its_palette_entry() {
        let (red, green, blue) = ANSI_BASE_PALETTE[CYAN_PALETTE_INDEX];

        assert_eq!(
            blend_color(Color::Cyan, Color::Black, 0),
            Color::Rgb(red, green, blue)
        );
    }

    /// Nothing to work a mixture out against, at either end.
    #[test]
    fn the_terminal_default_is_left_alone() {
        assert_eq!(
            blend_color(Color::Reset, Color::Black, u8::MAX),
            Color::Reset
        );
        assert_eq!(blend_color(Color::Red, Color::Reset, u8::MAX), Color::Red);
    }

    #[test]
    fn the_colour_cube_reads_as_its_three_levels() {
        // The cube runs from its own first entry to the one below the
        // grayscale ramp, and each of those corners is a single level
        // standing on all three channels at once.
        let last = ANSI_GRAYSCALE_BASE - 1;
        let opens = ANSI_CUBE_LEVELS[0];
        let closes = ANSI_CUBE_LEVELS[ANSI_CUBE_LEVELS.len() - 1];

        assert_eq!(
            blend_color(Color::Indexed(ANSI_CUBE_BASE), Color::Black, 0),
            Color::Rgb(opens, opens, opens)
        );
        assert_eq!(
            blend_color(Color::Indexed(last), Color::Black, 0),
            Color::Rgb(closes, closes, closes)
        );
    }

    #[test]
    fn the_grayscale_ramp_reads_as_one_level_on_every_channel() {
        assert_eq!(
            blend_color(Color::Indexed(ANSI_GRAYSCALE_BASE), Color::Black, 0),
            Color::Rgb(
                ANSI_GRAYSCALE_START,
                ANSI_GRAYSCALE_START,
                ANSI_GRAYSCALE_START
            )
        );
    }
}
