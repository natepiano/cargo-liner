//! Breaking one value -- a directory, a child's name -- onto as many
//! lines as the width it is drawn in takes, so a narrow cell shows it
//! whole instead of cutting it.

/// `text` broken into lines of at most `width` cells.
///
/// A line breaks at the last place within `width` that follows a `/`
/// or comes before a space, the `/` staying at the line's end and the
/// spaces at a break dropped. A run with neither breaks where the line
/// runs out. Text that fits, and any text at a `width` of zero, comes
/// back as the one line it is.
pub(crate) fn wrapped(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut lines = Vec::new();
    let mut rest = chars.as_slice();
    while rest.len() > width {
        let (line, next) = rest.split_at(break_at(rest, width));
        lines.push(line.iter().collect::<String>().trim_end().to_string());
        rest = trimmed_start(next);
    }
    // Spaces the text ends in, dropped at the last break, leave nothing
    // to start another line with.
    if !rest.is_empty() || lines.is_empty() {
        lines.push(rest.iter().collect());
    }
    lines
}

/// Chars of `chars` the next line takes when all of them do not fit in
/// `width`: up to the last break point within `width`, or `width` when
/// there is none. A break point follows a `/`, or comes before a space
/// that ends a word.
fn break_at(chars: &[char], width: usize) -> usize {
    (1..=width)
        .rev()
        .find(|&at| {
            chars[at - 1] == '/' || (chars[at].is_whitespace() && !chars[at - 1].is_whitespace())
        })
        .unwrap_or(width)
}

/// `chars` without the whitespace it starts with.
fn trimmed_start(chars: &[char]) -> &[char] {
    let start = chars
        .iter()
        .position(|character| !character.is_whitespace())
        .unwrap_or(chars.len());
    &chars[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_that_fits_is_left_alone() {
        assert_eq!(wrapped("~/rust/handler", 14), ["~/rust/handler"]);
    }

    #[test]
    fn a_directory_breaks_after_its_last_slash_that_fits() {
        assert_eq!(
            wrapped("~/rust/tool-based-ui-geometry-material-impl", 37),
            ["~/rust/", "tool-based-ui-geometry-material-impl"]
        );
    }

    #[test]
    fn a_directory_breaks_as_many_times_as_it_takes() {
        assert_eq!(
            wrapped("~/rust/hana_catalyst/docs/hana", 12),
            ["~/rust/", "hana_catalys", "t/docs/hana"]
        );
    }

    #[test]
    fn a_name_breaks_before_a_space_and_drops_it() {
        assert_eq!(
            wrapped("Launch the Phase 1 implementation seat", 20),
            ["Launch the Phase 1", "implementation seat"]
        );
    }

    #[test]
    fn a_run_of_spaces_at_a_break_is_dropped_whole() {
        assert_eq!(wrapped("cargo   nextest", 8), ["cargo", "nextest"]);
    }

    #[test]
    fn spaces_the_text_ends_in_start_no_line_of_their_own() {
        assert_eq!(wrapped("app-server ", 10), ["app-server"]);
    }

    #[test]
    fn a_word_no_line_could_hold_breaks_where_the_line_runs_out() {
        assert_eq!(
            wrapped("tool-based-ui-arrange", 8),
            ["tool-bas", "ed-ui-ar", "range"]
        );
    }

    #[test]
    fn no_line_is_wider_than_the_width() {
        let text = "cargo nextest run -p hana_video --no-fail-fast -- permission_queue";
        for width in 1..=text.len() {
            let lines = wrapped(text, width);
            assert!(
                lines.iter().all(|line| line.chars().count() <= width),
                "a line outruns {width} cells: {lines:?}"
            );
            assert!(
                lines.iter().all(|line| !line.is_empty()),
                "an empty line at {width} cells: {lines:?}"
            );
        }
    }

    #[test]
    fn a_width_of_zero_leaves_the_text_on_one_line() {
        assert_eq!(wrapped("~/rust/handler", 0), ["~/rust/handler"]);
    }
}
