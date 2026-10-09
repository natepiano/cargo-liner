use std::collections::VecDeque;

use ratatui::buffer::CellWidth;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::ToastSettings;

/// Structured toast body text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToastBody {
    /// Single text body.
    Text(String),
    /// Pre-split multi-line body.
    Lines(Vec<String>),
    /// Multi-line body with a foreground color per line, rendered with each
    /// line in its own color (the plain-text path is recovered via
    /// [`as_text`](Self::as_text), so width and truncation are unaffected).
    Colored {
        lines:  Vec<String>,
        colors: Vec<Color>,
    },
}

impl ToastBody {
    /// Return the body as display text.
    #[must_use]
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Lines(lines) | Self::Colored { lines, .. } => lines.join("\n"),
        }
    }

    /// The per-line foreground colors, when this body carries them.
    pub(super) fn line_colors(&self) -> Option<Vec<Color>> {
        match self {
            Self::Colored { colors, .. } => Some(colors.clone()),
            Self::Text(_) | Self::Lines(_) => None,
        }
    }

    pub(super) fn wrapped_line_count(&self, width: usize) -> usize {
        drawn_line_count(&self.as_text(), width)
    }
}

pub(super) fn drawn_line_count(text: &str, width: usize) -> usize {
    let width = u16::try_from(width.max(1)).unwrap_or(u16::MAX);
    text.lines()
        .map(|line| drawn_input_line_count(line, width))
        .sum::<usize>()
        .max(1)
}

fn drawn_input_line_count(line: &str, width: u16) -> usize {
    let span = Span::raw(line);
    let mut line_count = 0usize;
    let mut line_symbols = 0usize;
    let mut line_width = 0u16;
    let mut word_symbols = 0usize;
    let mut word_width = 0u16;
    let mut whitespace_width = 0u16;
    let mut whitespace = VecDeque::new();
    let mut previous_was_text = false;

    for grapheme in span.styled_graphemes(Style::default()) {
        let is_whitespace = grapheme.is_whitespace();
        let symbol_width = grapheme.symbol.cell_width();
        if symbol_width > width {
            continue;
        }

        let word_ended = previous_was_text && is_whitespace;
        let segment_overflows = line_symbols == 0
            && word_width
                .saturating_add(whitespace_width)
                .saturating_add(symbol_width)
                > width;
        if word_ended || segment_overflows {
            line_symbols = line_symbols.saturating_add(whitespace.len());
            line_width = line_width.saturating_add(whitespace_width);
            line_symbols = line_symbols.saturating_add(word_symbols);
            line_width = line_width.saturating_add(word_width);
            whitespace.clear();
            whitespace_width = 0;
            word_symbols = 0;
            word_width = 0;
        }

        let line_is_full = line_width >= width;
        let word_overflows = symbol_width > 0
            && line_width
                .saturating_add(whitespace_width)
                .saturating_add(word_width)
                >= width;
        if line_is_full || word_overflows {
            let mut remaining = width.saturating_sub(line_width);
            line_count = line_count.saturating_add(1);
            line_symbols = 0;
            line_width = 0;

            while let Some(&pending_width) = whitespace.front() {
                if pending_width > remaining {
                    break;
                }
                whitespace_width = whitespace_width.saturating_sub(pending_width);
                remaining = remaining.saturating_sub(pending_width);
                let _ = whitespace.pop_front();
            }
            if is_whitespace && whitespace.is_empty() {
                continue;
            }
        }

        if is_whitespace {
            whitespace_width = whitespace_width.saturating_add(symbol_width);
            whitespace.push_back(symbol_width);
        } else {
            word_width = word_width.saturating_add(symbol_width);
            word_symbols = word_symbols.saturating_add(1);
        }
        previous_was_text = !is_whitespace;
    }

    line_symbols = line_symbols
        .saturating_add(whitespace.len())
        .saturating_add(word_symbols);
    if line_symbols > 0 {
        line_count = line_count.saturating_add(1);
    }
    line_count.max(1)
}

impl From<String> for ToastBody {
    fn from(value: String) -> Self {
        if value.contains('\n') {
            Self::Lines(value.lines().map(ToOwned::to_owned).collect())
        } else {
            Self::Text(value)
        }
    }
}

impl From<&str> for ToastBody {
    fn from(value: &str) -> Self { Self::from(value.to_owned()) }
}

/// Interior body width available inside toast cards for the current settings.
#[must_use]
pub fn toast_body_width(settings: &ToastSettings) -> usize {
    usize::from(settings.width.get().saturating_sub(2))
}
