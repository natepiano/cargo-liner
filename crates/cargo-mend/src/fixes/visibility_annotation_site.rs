use super::constants::TAB_DISPLAY_WIDTH;

/// How a visibility annotation is written in source. A fixer may rewrite or
/// delete a [`Self::Bare`] annotation; a [`Self::Restricted`] one names a
/// narrower scope the author chose, so touching it would discard that choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VisibilityAnnotationForm {
    Bare,
    Restricted,
}

/// The visibility annotation at a finding's reported position, as byte offsets
/// into that file's source text. Every visibility fixer computes its edit range
/// from this instead of searching the line for `pub`, so a restricted
/// annotation is recognized rather than partially matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VisibilityAnnotationSite {
    pub form:               VisibilityAnnotationForm,
    /// Byte offset of the leading `p` of `pub`.
    pub start:              usize,
    /// Byte offset just past the annotation text.
    pub end:                usize,
    /// Byte offset just past the whitespace run separating the annotation from
    /// the item it applies to. A fix that deletes the annotation extends to
    /// here so the item is not left with a leading space.
    pub end_with_separator: usize,
}

impl VisibilityAnnotationSite {
    /// Locates the annotation a finding points at.
    ///
    /// `line` is the 1-based line `Finding` carries. `column` is the 1-based
    /// *display* column it carries — rustc's `col_display + 1`, produced by
    /// `compiler::visibility::source::build_finding`, which charges a tab
    /// [`TAB_DISPLAY_WIDTH`] columns. It is not a byte offset, so a
    /// tab-indented declaration reports a column past the byte its `pub`
    /// starts at; this converts before indexing.
    ///
    /// Returns `None` when the position is out of range, when the text there
    /// does not start with `pub`, and when a `pub(...)` form has no closing `)`
    /// on that physical line. A multiline annotation lands in that last case,
    /// and reporting no site is what stops a fixer from editing half of it.
    pub(super) fn locate(source: &str, line: usize, column: usize) -> Option<Self> {
        let line_start = line_byte_offset(source, line)?;
        let line_end = source[line_start..]
            .find('\n')
            .map_or(source.len(), |pos| line_start + pos);
        let line_text = &source[line_start..line_end];
        let column_offset = byte_offset_of_display_column(line_text, column)?;
        let annotation_text = &line_text[column_offset..];
        let (form, byte_len) = annotation_form_and_byte_len(annotation_text)?;
        let start = line_start + column_offset;
        let end = start + byte_len;
        let separator_len = annotation_text[byte_len..]
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(char::len_utf8)
            .sum::<usize>();
        Some(Self {
            form,
            start,
            end,
            end_with_separator: end + separator_len,
        })
    }
}

/// Classifies the `pub`, `pub(crate)`, `pub(super)`, `pub(self)`, or
/// `pub(in <path>)` annotation at the start of `text` and measures its byte
/// length.
fn annotation_form_and_byte_len(text: &str) -> Option<(VisibilityAnnotationForm, usize)> {
    let rest = text.strip_prefix("pub")?;
    match rest.chars().next() {
        Some('(') => {
            // `pub(in crate::foo)` may contain colons and identifiers but never
            // nested parens, so the first `)` closes the annotation.
            let close = rest.find(')')?;
            Some((
                VisibilityAnnotationForm::Restricted,
                "pub".len() + close + 1,
            ))
        },
        Some(c) if c.is_whitespace() || c == ':' => {
            Some((VisibilityAnnotationForm::Bare, "pub".len()))
        },
        None => Some((VisibilityAnnotationForm::Bare, "pub".len())),
        _ => None,
    }
}

/// Byte offset into `line_text` of the 1-based display `column`, or `None`
/// when the column falls past the end of the line or inside a character that
/// spans several display columns.
///
/// A tab is the only character rustc charges more than one column that occurs
/// in the indentation a visibility annotation sits behind, so the walk charges
/// [`TAB_DISPLAY_WIDTH`] for a tab and one column for everything else.
fn byte_offset_of_display_column(line_text: &str, column: usize) -> Option<usize> {
    let target = column.saturating_sub(1);
    let mut display_column = 0;
    for (offset, character) in line_text.char_indices() {
        if display_column >= target {
            return (display_column == target).then_some(offset);
        }
        display_column += if character == '\t' {
            TAB_DISPLAY_WIDTH
        } else {
            1
        };
    }
    (display_column == target).then_some(line_text.len())
}

/// Byte offset where 1-based `line` starts in `source`.
fn line_byte_offset(source: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    if line == 1 {
        return Some(0);
    }
    source
        .match_indices('\n')
        .nth(line - 2)
        .map(|(pos, _)| pos + 1)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::TAB_DISPLAY_WIDTH;
    use super::VisibilityAnnotationForm;
    use super::VisibilityAnnotationSite;
    use super::annotation_form_and_byte_len;

    #[test]
    fn bare_pub_is_measured_without_its_separator() {
        assert_eq!(
            annotation_form_and_byte_len("pub leaked: u32"),
            Some((VisibilityAnnotationForm::Bare, 3))
        );
    }

    #[test]
    fn pub_crate_is_restricted() {
        assert_eq!(
            annotation_form_and_byte_len("pub(crate) x"),
            Some((VisibilityAnnotationForm::Restricted, "pub(crate)".len()))
        );
    }

    #[test]
    fn pub_super_is_restricted() {
        assert_eq!(
            annotation_form_and_byte_len("pub(super) name"),
            Some((VisibilityAnnotationForm::Restricted, "pub(super)".len()))
        );
    }

    #[test]
    fn pub_in_path_is_restricted() {
        assert_eq!(
            annotation_form_and_byte_len("pub(in crate::foo::bar) y"),
            Some((
                VisibilityAnnotationForm::Restricted,
                "pub(in crate::foo::bar)".len()
            ))
        );
    }

    #[test]
    fn text_without_pub_has_no_annotation() {
        assert_eq!(annotation_form_and_byte_len("private_field: u32"), None);
    }

    #[test]
    fn unterminated_restricted_annotation_has_no_site() {
        // A `pub(\n    in crate::a\n)` annotation reaches this parser as the
        // first physical line only; half of it is not a fixable span.
        let source = "pub(\n    in crate::a\n) fn wide() {}\n";
        assert_eq!(VisibilityAnnotationSite::locate(source, 1, 1), None);
    }

    #[test]
    fn located_site_covers_annotation_and_separator() {
        let source = "mod inner;\n    pub   fn wide() {}\n";
        let site = VisibilityAnnotationSite::locate(source, 2, 5).expect("annotation at 2:5");
        assert_eq!(site.form, VisibilityAnnotationForm::Bare);
        assert_eq!(&source[site.start..site.end], "pub");
        assert_eq!(&source[site.start..site.end_with_separator], "pub   ");
    }

    #[test]
    fn tab_indented_annotation_is_located_from_its_display_column() {
        // rustc charges the tab TAB_DISPLAY_WIDTH columns, so the reported
        // column is past the byte the `pub` starts at.
        let source = "impl Wide {\n\tpub fn wide() {}\n}\n";
        let site = VisibilityAnnotationSite::locate(source, 2, TAB_DISPLAY_WIDTH + 1)
            .expect("annotation behind one tab");
        assert_eq!(site.form, VisibilityAnnotationForm::Bare);
        assert_eq!(&source[site.start..site.end], "pub");
        assert_eq!(&source[site.start..site.end_with_separator], "pub ");
    }

    #[test]
    fn column_inside_a_tab_has_no_site() {
        assert_eq!(
            VisibilityAnnotationSite::locate("\tpub fn wide() {}\n", 1, TAB_DISPLAY_WIDTH),
            None
        );
    }

    #[test]
    fn column_past_the_line_has_no_site() {
        assert_eq!(
            VisibilityAnnotationSite::locate("pub fn a() {}\n", 1, 99),
            None
        );
    }
}
