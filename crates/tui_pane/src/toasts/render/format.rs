use std::time::Duration;

use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use crate::constants::ELISION;
use crate::constants::SECONDS_PER_MINUTE;
use crate::constants::TOAST_ELAPSED_MINUTE_MILLIS;
use crate::constants::TOAST_ELAPSED_SECONDS_MILLIS;

pub(super) fn fade_to_style(progress: f64) -> Style {
    let value = fade_level(progress);
    Style::default().fg(Color::Rgb(value, value, value))
}

pub(in crate::toasts) fn fade_level(progress: f64) -> u8 {
    let p = progress.clamp(0.0, 1.0);
    let curve = p * p * p;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value is mathematically clamped to [128, 255] before the cast"
    )]
    let value = 127.0f64.mul_add(-curve, 255.0) as u8;
    value
}

pub(super) fn fade_to_color(text: &str, progress: f64) -> Line<'static> {
    Line::from(Span::styled(text.to_owned(), fade_to_style(progress)))
}

pub(super) fn truncate(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out
}

/// Cuts `text` to `width` cells and ends it with `…`. The cut never leaves
/// a rule piece or a space before the `…`.
pub(super) fn truncate_with_ellipsis(text: &str, width: usize) -> String {
    let total: usize = text
        .chars()
        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum();
    if total <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = truncate(text, width - ELISION.width());
    let kept = out
        .trim_end_matches(|ch: char| ch.is_whitespace() || is_rule_piece(ch))
        .len();
    out.truncate(kept);
    out.push_str(ELISION);
    out
}

/// A box-drawing character, as a toast rule draws.
const fn is_rule_piece(ch: char) -> bool { matches!(ch, '\u{2500}'..='\u{257f}') }

pub(super) fn format_elapsed(elapsed: Duration) -> String {
    let ms = elapsed.as_millis();
    if ms >= TOAST_ELAPSED_MINUTE_MILLIS {
        let secs = elapsed.as_secs();
        format!(
            "{}m {:02}s",
            secs / SECONDS_PER_MINUTE,
            secs % SECONDS_PER_MINUTE
        )
    } else if ms >= TOAST_ELAPSED_SECONDS_MILLIS {
        format!("{}s", elapsed.as_secs())
    } else if ms >= 1 {
        format!("{ms}ms")
    } else {
        format!("{}us", elapsed.as_micros())
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_with_ellipsis;

    #[test]
    fn cut_ends_with_one_ellipsis_cell() {
        assert_eq!(truncate_with_ellipsis("cargo-liner clean", 8), "cargo-l…");
    }

    #[test]
    fn cut_never_ends_after_a_rule_piece() {
        assert_eq!(
            truncate_with_ellipsis("cargo-liner: clean ──── done", 22),
            "cargo-liner: clean…"
        );
    }

    #[test]
    fn text_that_fits_stays_whole() {
        assert_eq!(truncate_with_ellipsis("clean", 5), "clean");
    }
}
