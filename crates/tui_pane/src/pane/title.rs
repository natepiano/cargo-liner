use std::borrow::Cow;

use super::state;

/// One labelled group of items inside a [`PaneTitleCount::Grouped`]
/// title — e.g. `Binary (1 of 1)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneTitleGroup<'a> {
    /// Group label shown before the count.
    pub label:  Cow<'a, str>,
    /// Total items in the group.
    pub len:    usize,
    /// Optional cursor position within the group.
    pub cursor: Option<usize>,
}

/// Trailing count rendered in a pane title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneTitleCount<'a> {
    /// No count.
    None,
    /// One total with an optional cursor — renders as `(N)` or
    /// `(M of N)`.
    Single {
        /// Total item count.
        len:    usize,
        /// Optional cursor position.
        cursor: Option<usize>,
    },
    /// One labelled count per group — renders as
    /// `Label1 (N1), Label2 (N2)`.
    Grouped(Vec<PaneTitleGroup<'a>>),
}

impl PaneTitleCount<'_> {
    fn count_text(len: usize, cursor: Option<usize>) -> String {
        if let Some(pos) = cursor
            && pos < len
        {
            state::scroll_indicator(pos, len)
        } else {
            len.to_string()
        }
    }

    /// Render this count as the body of a pane title.
    #[must_use]
    pub fn body(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Single { len, cursor } => format!("({})", Self::count_text(*len, *cursor)),
            Self::Grouped(groups) => groups
                .iter()
                .map(|group| {
                    format!(
                        "{} ({})",
                        group.label,
                        Self::count_text(group.len, group.cursor)
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

/// Format `title` followed by `count` for use as a ratatui pane title.
#[must_use]
pub fn pane_title(title: &str, count: &PaneTitleCount<'_>) -> String {
    let body = count.body();
    if body.is_empty() {
        format!(" {title} ")
    } else {
        format!(" {title} {body} ")
    }
}

/// [`pane_title`] variant that separates `title` and `body` with `:` —
/// used by panes that prefix the body with a category label.
#[must_use]
pub fn prefixed_pane_title(title: &str, count: &PaneTitleCount<'_>) -> String {
    let body = count.body();
    if body.is_empty() {
        format!(" {title} ")
    } else {
        format!(" {title}: {body} ")
    }
}

#[cfg(test)]
mod tests {
    use super::PaneTitleCount;
    use super::PaneTitleGroup;
    use super::pane_title;
    use super::prefixed_pane_title;

    #[test]
    fn single_title_count_formats_cursor_position() {
        assert_eq!(
            pane_title(
                "Languages",
                &PaneTitleCount::Single {
                    len:    4,
                    cursor: Some(1),
                }
            ),
            " Languages (2 of 4) "
        );
    }

    #[test]
    fn single_title_count_ignores_out_of_range_cursor() {
        assert_eq!(
            pane_title(
                "Lint Runs",
                &PaneTitleCount::Single {
                    len:    3,
                    cursor: Some(9),
                }
            ),
            " Lint Runs (3) "
        );
    }

    #[test]
    fn grouped_title_count_formats_each_group() {
        assert_eq!(
            prefixed_pane_title(
                "Targets",
                &PaneTitleCount::Grouped(vec![
                    PaneTitleGroup {
                        label:  "Binary".into(),
                        len:    1,
                        cursor: Some(0),
                    },
                    PaneTitleGroup {
                        label:  "Examples".into(),
                        len:    3,
                        cursor: None,
                    },
                ])
            ),
            " Targets: Binary (1 of 1), Examples (3) "
        );
    }
}
