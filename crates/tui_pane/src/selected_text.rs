use crate::ansi_text;

/// Join an inclusive, clamped range of snapshot lines with ANSI styling removed.
/// An empty snapshot produces an empty string.
#[must_use]
pub fn selected_lines_text(snapshot: &[String], anchor: usize, cursor: usize) -> String {
    let Some(last) = snapshot.len().checked_sub(1) else {
        return String::new();
    };
    let lo = anchor.min(cursor).min(last);
    let hi = anchor.max(cursor).min(last);
    snapshot[lo..=hi]
        .iter()
        .map(|line| ansi_text::strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::selected_lines_text;

    #[test]
    fn empty_reversed_and_out_of_range_selections_are_clamped() {
        assert_eq!(selected_lines_text(&[], usize::MAX, 0), "");
        let lines = vec!["first".into(), "second".into(), "third".into()];
        assert_eq!(selected_lines_text(&lines, 2, 0), "first\nsecond\nthird");
        assert_eq!(selected_lines_text(&lines, 1, usize::MAX), "second\nthird");
        assert_eq!(selected_lines_text(&lines, usize::MAX, usize::MAX), "third");
    }

    #[test]
    fn selected_lines_strip_ansi_and_preserve_blank_rows_and_placeholders() {
        let lines = vec!["\x1b[31mred\x1b[0m".into(), String::new(), "—".into()];
        assert_eq!(selected_lines_text(&lines, 0, 2), "red\n\n—");
    }
}
