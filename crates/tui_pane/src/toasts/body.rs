use ratatui::style::Color;

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
        let width = width.max(1);
        self.as_text()
            .lines()
            .map(|line| (line.chars().count().max(1).saturating_sub(1) / width) + 1)
            .sum::<usize>()
            .max(1)
    }
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
