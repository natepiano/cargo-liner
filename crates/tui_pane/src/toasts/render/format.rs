use std::time::Duration;

use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

pub(super) fn fade_to_style(progress: f64) -> Style {
    let p = progress.clamp(0.0, 1.0);
    let curve = p * p * p;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value is mathematically clamped to [128, 255] before the cast"
    )]
    let value = 127.0f64.mul_add(-curve, 255.0) as u8;
    Style::default().fg(Color::Rgb(value, value, value))
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

pub(super) fn truncate_with_ellipsis(text: &str, width: usize) -> String {
    if text.len() <= width {
        return text.to_owned();
    }
    if width <= 1 {
        return String::new();
    }
    format!("{}...", &text[..width.saturating_sub(3)])
}

pub(super) fn format_elapsed(elapsed: Duration) -> String {
    let ms = elapsed.as_millis();
    if ms >= 60_000 {
        let secs = elapsed.as_secs();
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else if ms >= 10_000 {
        format!("{}s", elapsed.as_secs())
    } else if ms >= 1 {
        format!("{ms}ms")
    } else {
        format!("{}us", elapsed.as_micros())
    }
}
