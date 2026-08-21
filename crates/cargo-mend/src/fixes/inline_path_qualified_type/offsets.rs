use proc_macro2::LineColumn;

pub(super) fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            offsets.push(idx + 1);
        }
    }
    offsets
}

pub(super) fn offset(text: &str, line_offsets: &[usize], position: LineColumn) -> usize {
    let line_start = line_offsets
        .get(position.line.saturating_sub(1))
        .copied()
        .unwrap_or(0);
    // `proc_macro2::LineColumn::column` is a 0-based count of UTF-8 *characters*
    // from the start of the line, not bytes. Walk char_indices to convert to a
    // byte offset so multi-byte characters (em-dashes, accented letters, etc.)
    // earlier on the same line don't shift the replacement window.
    let line_text = text.get(line_start..).unwrap_or("");
    let byte_in_line = line_text
        .char_indices()
        .nth(position.column)
        .map_or(line_text.len(), |(byte_idx, _)| byte_idx);
    line_start + byte_in_line
}
