use crate::ToastSettings;

/// Interior body width available inside toast cards for the current settings.
#[must_use]
pub fn toast_body_width(settings: &ToastSettings) -> usize {
    usize::from(settings.width.get().saturating_sub(2))
}

/// Structured toast body text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToastBody {
    /// Single text body.
    Text(String),
    /// Pre-split multi-line body.
    Lines(Vec<String>),
}

impl ToastBody {
    /// Return the body as display text.
    #[must_use]
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Lines(lines) => lines.join("\n"),
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
