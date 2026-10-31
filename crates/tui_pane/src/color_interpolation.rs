use ratatui::style::Color;

/// Interpolate terminal colors with clamped, truncated RGB channels.
/// Named and indexed colors use the same fixed ANSI fallback palette.
#[must_use]
pub fn interpolate_terminal_color(start: Color, end: Color, fraction: f64) -> Color {
    let (sr, sg, sb) = rgb_channels(start);
    let (er, eg, eb) = rgb_channels(end);
    let lerp = |a: u8, b: u8| -> u8 {
        let af = f64::from(a);
        let bf = f64::from(b);
        truncate_channel((bf - af).mul_add(fraction, af).clamp(0.0, 255.0))
    };
    Color::Rgb(lerp(sr, er), lerp(sg, eg), lerp(sb, eb))
}

// Recover the integer part in eight comparisons. As with a float-to-u8 cast,
// NaN yields zero; callers clamp every finite value to the channel range.
fn truncate_channel(value: f64) -> u8 {
    let mut channel = 0;
    for bit in [128, 64, 32, 16, 8, 4, 2, 1] {
        let candidate = channel | bit;
        if f64::from(candidate) <= value {
            channel = candidate;
        }
    }
    channel
}

const fn rgb_channels(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset | Color::Black => (0, 0, 0),
        Color::Red => (170, 0, 0),
        Color::Green => (0, 170, 0),
        Color::Yellow => (170, 85, 0),
        Color::Blue => (0, 0, 170),
        Color::Magenta => (170, 0, 170),
        Color::Cyan => (0, 170, 170),
        Color::Gray | Color::Indexed(_) => (170, 170, 170),
        Color::DarkGray => (85, 85, 85),
        Color::LightRed => (255, 85, 85),
        Color::LightGreen => (85, 255, 85),
        Color::LightYellow => (255, 255, 85),
        Color::LightBlue => (85, 85, 255),
        Color::LightMagenta => (255, 85, 255),
        Color::LightCyan => (85, 255, 255),
        Color::White => (255, 255, 255),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::interpolate_terminal_color;
    use super::rgb_channels;
    use super::truncate_channel;

    #[test]
    fn bounded_conversion_matches_truncation_for_every_channel_pair() {
        for start in 0..=u8::MAX {
            for end in 0..=u8::MAX {
                for fraction in [
                    f64::NEG_INFINITY,
                    -2.0,
                    -0.1,
                    0.0,
                    0.25,
                    0.5,
                    0.75,
                    1.0,
                    2.0,
                    f64::INFINITY,
                    f64::NAN,
                ] {
                    let value = (f64::from(end) - f64::from(start))
                        .mul_add(fraction, f64::from(start))
                        .clamp(0.0, 255.0);
                    let expected = if value.is_nan() { 0.0 } else { value.trunc() };
                    assert_eq!(f64::from(truncate_channel(value)), expected);
                }
            }
        }
    }

    #[test]
    fn named_and_indexed_colors_keep_the_original_palette() {
        for (color, rgb) in [
            (Color::Reset, (0, 0, 0)),
            (Color::Black, (0, 0, 0)),
            (Color::Red, (170, 0, 0)),
            (Color::Green, (0, 170, 0)),
            (Color::Yellow, (170, 85, 0)),
            (Color::Blue, (0, 0, 170)),
            (Color::Magenta, (170, 0, 170)),
            (Color::Cyan, (0, 170, 170)),
            (Color::Gray, (170, 170, 170)),
            (Color::DarkGray, (85, 85, 85)),
            (Color::LightRed, (255, 85, 85)),
            (Color::LightGreen, (85, 255, 85)),
            (Color::LightYellow, (255, 255, 85)),
            (Color::LightBlue, (85, 85, 255)),
            (Color::LightMagenta, (255, 85, 255)),
            (Color::LightCyan, (85, 255, 255)),
            (Color::White, (255, 255, 255)),
            (Color::Indexed(0), (170, 170, 170)),
            (Color::Indexed(255), (170, 170, 170)),
            (Color::Rgb(1, 2, 3), (1, 2, 3)),
        ] {
            assert_eq!(rgb_channels(color), rgb);
        }
        assert_eq!(
            interpolate_terminal_color(Color::Black, Color::White, 0.5),
            Color::Rgb(127, 127, 127)
        );
        assert_eq!(
            interpolate_terminal_color(Color::Red, Color::Blue, 2.0),
            Color::Rgb(0, 0, 255)
        );
        assert_eq!(
            interpolate_terminal_color(Color::White, Color::Black, f64::NAN),
            Color::Rgb(0, 0, 0)
        );
    }
}
