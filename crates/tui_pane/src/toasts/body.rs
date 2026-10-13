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
}

/// Where a styled toast run permits a line break.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunBreaks {
    /// Break between words, or inside words wider than a line.
    AtSpaces,
    /// Keep the run together on one line. A run too wide for the line,
    /// counting short text glued after it, is cut and marked with `…`.
    /// Explicit newlines still start a new line.
    Never,
}

/// Toast text with its drawing style and line-break policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToastRun {
    /// Text drawn by this run.
    pub text:   String,
    /// Style applied over the toast's body style.
    pub style:  Style,
    /// Permitted line breaks inside this run.
    pub breaks: RunBreaks,
}

impl ToastRun {
    /// Build styled toast text with an explicit line-break policy.
    #[must_use]
    pub fn new(text: impl Into<String>, style: Style, breaks: RunBreaks) -> Self {
        Self {
            text: text.into(),
            style,
            breaks,
        }
    }
}

/// Body content retained through measuring and drawing a toast.
#[derive(Clone, Debug)]
pub(super) enum ToastRenderBody {
    Text(ToastBody),
    StyledRuns {
        body: ToastBody,
        runs: Vec<ToastRun>,
    },
}

impl From<ToastBody> for ToastRenderBody {
    fn from(body: ToastBody) -> Self { Self::Text(body) }
}

impl ToastRenderBody {
    pub(super) fn styled(runs: Vec<ToastRun>) -> Self {
        let text = runs.iter().map(|run| run.text.as_str()).collect::<String>();
        Self::StyledRuns {
            body: ToastBody::from(text),
            runs,
        }
    }

    pub(super) const fn public_body(&self) -> &ToastBody {
        match self {
            Self::Text(body) | Self::StyledRuns { body, .. } => body,
        }
    }

    pub(super) fn as_text(&self) -> String { self.public_body().as_text() }

    pub(super) fn wrapped_line_count(&self, width: usize) -> usize {
        let width = u16::try_from(width.max(1)).unwrap_or(u16::MAX);
        self.wrapped_lines(width, Style::default()).len().max(1)
    }

    pub(super) fn wrapped_lines(&self, width: u16, style: Style) -> Vec<Line<'static>> {
        match self {
            Self::Text(body) => body
                .as_text()
                .split('\n')
                .enumerate()
                .flat_map(|(index, line)| {
                    let line_style = match body {
                        ToastBody::Colored { colors, .. } => {
                            colors.get(index).map_or(style, |color| style.fg(*color))
                        },
                        ToastBody::Text(_) | ToastBody::Lines(_) => style,
                    };
                    wrap::wrapped_preserving_whitespace(
                        vec![Span::styled(line.to_owned(), line_style)],
                        width,
                    )
                    .lines
                })
                .collect(),
            Self::StyledRuns { runs, .. } => wrap::wrapped_runs(runs, width, style).lines,
        }
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
pub fn toast_body_width(settings: &ToastSettings) -> usize { card_body_width(settings.width.get()) }

pub(super) fn card_body_width(card_width: u16) -> usize {
    usize::from(card_width).saturating_sub(
        BLOCK_BORDER_WIDTH
            .saturating_add(usize::from(TOAST_BODY_HORIZONTAL_PADDING).saturating_mul(2)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nonbreaking_run_moves_whole_to_the_next_line() {
        let highlight = Style::default().fg(Color::Cyan);
        let body = ToastRenderBody::styled(vec![
            ToastRun::new(
                "To stop reporting, run ",
                Style::default(),
                RunBreaks::AtSpaces,
            ),
            ToastRun::new("cargo tile uninstall", highlight, RunBreaks::Never),
            ToastRun::new(".", Style::default(), RunBreaks::AtSpaces),
        ]);
        let lines = body.wrapped_lines(32, Style::default());
        let text: Vec<String> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect();
        assert_eq!(text, ["To stop reporting, run", "cargo tile uninstall."]);
        assert!(
            lines[1]
                .spans
                .iter()
                .any(|span| span.content == "cargo tile uninstall" && span.style == highlight)
        );
    }
}
