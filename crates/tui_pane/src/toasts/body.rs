use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

use crate::BLOCK_BORDER_WIDTH;
use crate::ToastSettings;
use crate::constants::TOAST_BODY_HORIZONTAL_PADDING;
use crate::wrap;

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
        let width = u16::try_from(width.max(1)).unwrap_or(u16::MAX);
        wrapped_body_lines(&self.as_text(), width, |_, line| {
            Line::from(Span::styled(line.to_owned(), Style::default()))
        })
        .len()
        .max(1)
    }
}

pub(super) fn wrapped_body_lines(
    body: &str,
    width: u16,
    mut styled_line: impl FnMut(usize, &str) -> Line<'static>,
) -> Vec<Line<'static>> {
    body.split('\n')
        .enumerate()
        .flat_map(|(index, line)| {
            wrap::wrapped_preserving_whitespace(styled_line(index, line).spans, width).lines
        })
        .collect()
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
pub fn toast_body_width(settings: &ToastSettings) -> usize { card_body_width(settings.width.get()) }

pub(super) fn card_body_width(card_width: u16) -> usize {
    usize::from(card_width).saturating_sub(
        BLOCK_BORDER_WIDTH
            .saturating_add(usize::from(TOAST_BODY_HORIZONTAL_PADDING).saturating_mul(2)),
    )
}
