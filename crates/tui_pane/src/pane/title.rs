use std::borrow::Cow;

use ratatui::text::Line;
use ratatui::text::Span;

use super::state;

/// Drops the side spaces of a title that does not fit in `width` cells with
/// them, when the title fits without them. A title that uses every inner cell
/// then draws whole, as `┌Package - alpha┐`.
pub(super) fn fit_title_sides(title: Line<'_>, width: usize) -> Line<'_> {
    let first_space = title
        .spans
        .first()
        .is_some_and(|span| span.content.starts_with(' '));
    let last_space = title
        .spans
        .last()
        .is_some_and(|span| span.content.ends_with(' '));
    let full = title.width();
    let side_cells = usize::from(first_space) + usize::from(last_space);
    if full <= width || full.saturating_sub(side_cells) > width {
        return title;
    }
    let last_index = title.spans.len().saturating_sub(1);
    let spans = title
        .spans
        .into_iter()
        .enumerate()
        .map(|(index, span)| {
            let mut text: &str = &span.content;
            if index == 0 {
                text = text.strip_prefix(' ').unwrap_or(text);
            }
            if index == last_index {
                text = text.strip_suffix(' ').unwrap_or(text);
            }
            Span::styled(text.to_owned(), span.style)
        })
        .collect::<Vec<_>>();
    Line::from(spans).style(title.style)
}

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
            Self::Grouped(pane_title_groups) => pane_title_groups
                .iter()
                .map(|pane_title_group| {
                    format!(
                        "{} ({})",
                        pane_title_group.label,
                        Self::count_text(pane_title_group.len, pane_title_group.cursor)
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

/// Format `title` followed by `count` for use as a ratatui pane title.
#[must_use]
pub fn pane_title(title: &str, pane_title_count: &PaneTitleCount<'_>) -> String {
    let body = pane_title_count.body();
    if body.is_empty() {
        format!(" {title} ")
    } else {
        format!(" {title} {body} ")
    }
}

/// [`pane_title`] variant that separates `title` and `body` with `:` —
/// used by panes that prefix the body with a category label.
#[must_use]
pub fn prefixed_pane_title(title: &str, pane_title_count: &PaneTitleCount<'_>) -> String {
    let body = pane_title_count.body();
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
    fn title_without_room_for_side_spaces_drops_them() {
        let fitted = super::fit_title_sides(" Package - alpha ".into(), 15);
        assert_eq!(fitted.to_string(), "Package - alpha");
    }

    #[test]
    fn title_with_room_keeps_its_side_spaces() {
        let fitted = super::fit_title_sides(" Package - alpha ".into(), 17);
        assert_eq!(fitted.to_string(), " Package - alpha ");
    }

    #[test]
    fn title_too_wide_without_side_spaces_keeps_them() {
        let fitted = super::fit_title_sides(" Package - alpha ".into(), 14);
        assert_eq!(fitted.to_string(), " Package - alpha ");
    }

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
