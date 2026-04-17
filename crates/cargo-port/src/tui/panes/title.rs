use std::borrow::Cow;

use crate::tui::pane;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct PaneTitleGroup<'a> {
    pub(in super::super) label:  Cow<'a, str>,
    pub(in super::super) len:    usize,
    pub(in super::super) cursor: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) enum PaneTitleCount<'a> {
    None,
    Single {
        len:    usize,
        cursor: Option<usize>,
    },
    Grouped(Vec<PaneTitleGroup<'a>>),
}

impl PaneTitleCount<'_> {
    fn count_text(len: usize, cursor: Option<usize>) -> String {
        if let Some(pos) = cursor
            && pos < len
        {
            pane::scroll_indicator(pos, len)
        } else {
            len.to_string()
        }
    }

    pub(in super::super) fn body(&self) -> String {
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

pub(in super::super) fn pane_title(title: &str, count: &PaneTitleCount<'_>) -> String {
    let body = count.body();
    if body.is_empty() {
        format!(" {title} ")
    } else {
        format!(" {title} {body} ")
    }
}

pub(in super::super) fn prefixed_pane_title(title: &str, count: &PaneTitleCount<'_>) -> String {
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
